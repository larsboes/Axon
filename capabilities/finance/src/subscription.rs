//! What a subscription actually is, and the arithmetic over it.
//!
//! Every tool in this space stores a subscription's price as one mutable number.
//! That number answers "what am I paying now" and nothing else. It cannot say what
//! this has cost since it started, and it cannot notice that a provider raised the
//! price, because the moment the new figure is written the old one is gone.
//!
//! So a subscription here is a contract carrying two append-only histories. A price
//! change appends a [`PricePoint`]; a pause appends a [`StateChange`]. Nothing is
//! ever edited in place, which makes both "what did this cost me last year" and
//! "when did I pause this, and why" answerable from the record itself.
//!
//! Everything in this module is pure. The store persists these types and the server
//! serialises them, but the arithmetic that decides what a month costs lives here,
//! where it is testable without a database.
//!
//! ## Two representation choices
//!
//! **Money is integer cents.** A monthly burn is a sum of divisions, since a yearly
//! plan contributes a twelfth, and binary floating point turns that into figures
//! that disagree with the bank by a cent for reasons nobody can reconstruct later.
//!
//! **Dates are ISO-8601 strings.** They sort lexicographically, which is the only
//! operation this module performs on them, and `trips` already set the precedent. A
//! date library would be a dependency bought for `<=`.

use serde::{Deserialize, Serialize};

/// How often a price recurs. The monthly equivalent of each is the whole reason
/// this is an enum rather than a free-text field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BillingCycle {
    Weekly,
    Monthly,
    Quarterly,
    Yearly,
    /// A one-time charge. Contributes nothing to a recurring burn, but belongs in
    /// the history so cost-since-inception stays honest.
    OneOff,
}

impl BillingCycle {
    /// What one month of this cycle costs, in cents, rounded half away from zero.
    ///
    /// Weekly is 52/12 rather than 4, because four weeks is eleven months of the
    /// year and the error runs in the direction of under-reporting what you spend.
    pub fn monthly_cents(self, amount_cents: i64) -> i64 {
        match self {
            BillingCycle::Weekly => div_round(amount_cents * 52, 12),
            BillingCycle::Monthly => amount_cents,
            BillingCycle::Quarterly => div_round(amount_cents, 3),
            BillingCycle::Yearly => div_round(amount_cents, 12),
            BillingCycle::OneOff => 0,
        }
    }
}

/// Integer division rounding half away from zero, so a burn total does not drift
/// down by a cent per subscription per month the way truncation would.
fn div_round(numerator: i64, denominator: i64) -> i64 {
    let sign = if (numerator < 0) != (denominator < 0) {
        -1
    } else {
        1
    };
    let (n, d) = (numerator.abs(), denominator.abs());
    sign * ((n * 2 + d) / (d * 2))
}

/// Where a subscription is in its life. No ordering is implied: `paused` can return
/// to `active`, and that round trip is exactly what the history is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Considering,
    Trial,
    Active,
    Paused,
    Cancelled,
}

impl State {
    /// Whether money is leaving the account in this state.
    ///
    /// `Trial` counts as billing even when the current price is zero, because the
    /// price series says what it costs and the state says whether you are on the
    /// hook. A free trial with a scheduled price point is the case that makes
    /// collapsing these into one field wrong.
    pub fn is_billing(self) -> bool {
        matches!(self, State::Active | State::Trial)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            State::Considering => "considering",
            State::Trial => "trial",
            State::Active => "active",
            State::Paused => "paused",
            State::Cancelled => "cancelled",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "considering" => Some(State::Considering),
            "trial" => Some(State::Trial),
            "active" => Some(State::Active),
            "paused" => Some(State::Paused),
            "cancelled" | "canceled" => Some(State::Cancelled),
            _ => None,
        }
    }
}

/// One entry in the price history. Appended, never edited.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PricePoint {
    /// ISO-8601 date this price took effect.
    pub valid_from: String,
    pub amount_cents: i64,
    pub currency: String,
    pub cycle: BillingCycle,
    /// Why it changed. "initial" for the first, then whatever the provider did.
    #[serde(default)]
    pub reason: String,
}

/// One entry in the state history. Appended, never edited.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateChange {
    /// ISO-8601 date this state took effect.
    pub effective: String,
    pub state: State,
    #[serde(default)]
    pub note: String,
}

/// A subscription and its two histories.
///
/// The fields here are the machine's: identity, and the series. What the human
/// wrote about it, meaning why they pay and whether it is worth it, stays in the
/// vault note and is deliberately absent. `value_rating` is the one borrowed field,
/// read from frontmatter so cut candidates can be ranked, and never written back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Subscription {
    pub id: String,
    pub name: String,
    /// Vault-relative path of the owning note. The import identity, so importing
    /// twice cannot produce two rows.
    pub source_path: String,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub value_rating: Option<i16>,
    #[serde(default)]
    pub prices: Vec<PricePoint>,
    #[serde(default)]
    pub states: Vec<StateChange>,
}

impl Subscription {
    /// The price in force on `date`, which is the latest point not in the future.
    ///
    /// `None` before the first price point rather than a zero: a subscription whose
    /// history has not started costs an unknown amount, and reporting nothing is
    /// honest where reporting zero is a figure someone might trust.
    pub fn price_at(&self, date: &str) -> Option<&PricePoint> {
        self.prices
            .iter()
            .filter(|p| p.valid_from.as_str() <= date)
            .max_by(|a, b| a.valid_from.cmp(&b.valid_from))
    }

    /// The state in force on `date`. Absent history reads as `Considering`: a note
    /// exists, so somebody is thinking about it, and nothing is being billed.
    pub fn state_at(&self, date: &str) -> State {
        self.states
            .iter()
            .filter(|s| s.effective.as_str() <= date)
            .max_by(|a, b| a.effective.cmp(&b.effective))
            .map(|s| s.state)
            .unwrap_or(State::Considering)
    }

    /// What this contributes to the monthly burn on `date`. Zero unless it is
    /// actually billing.
    pub fn monthly_cents_at(&self, date: &str) -> i64 {
        if !self.state_at(date).is_billing() {
            return 0;
        }
        self.price_at(date)
            .map(|p| p.cycle.monthly_cents(p.amount_cents))
            .unwrap_or(0)
    }

    /// How far the monthly price moved between two dates, in cents. Positive is an
    /// increase. This is the drift signal a single mutable number cannot produce.
    pub fn price_drift_cents(&self, from: &str, to: &str) -> Option<i64> {
        let before = self.price_at(from)?;
        let after = self.price_at(to)?;
        Some(
            after.cycle.monthly_cents(after.amount_cents)
                - before.cycle.monthly_cents(before.amount_cents),
        )
    }
}

/// Monthly and annual burn across a set of subscriptions on a given date.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Burn {
    pub monthly_cents: i64,
    pub annual_cents: i64,
    /// How many subscriptions were actually billing. A burn of zero across nine
    /// subscriptions and a burn of zero across none are different situations.
    pub billing_count: usize,
}

/// Total burn on `date`, computed from each subscription's series rather than from
/// any stored total. There is no cached figure to go stale.
pub fn burn_at(subscriptions: &[Subscription], date: &str) -> Burn {
    let mut monthly = 0i64;
    let mut count = 0usize;
    for sub in subscriptions {
        monthly += sub.monthly_cents_at(date);
        if sub.state_at(date).is_billing() {
            count += 1;
        }
    }
    Burn {
        monthly_cents: monthly,
        annual_cents: monthly * 12,
        billing_count: count,
    }
}

/// Cents as a decimal string with two places, for display and for the vault block.
pub fn cents_to_decimal(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let abs = cents.abs();
    format!("{sign}{}.{:02}", abs / 100, abs % 100)
}

/// Parse `"12.34"`, `"12"`, or `"12,34"` into cents.
///
/// The comma is not politeness. These figures come out of German subscription
/// notes, where a human types `9,99` without thinking about it, and silently
/// reading that as `9` would be a wrong number written confidently.
pub fn decimal_to_cents(raw: &str) -> Option<i64> {
    let cleaned: String = raw
        .trim()
        .replace(',', ".")
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
        .collect();
    if cleaned.is_empty() {
        return None;
    }
    let negative = cleaned.starts_with('-');
    let digits = cleaned.trim_start_matches('-');
    let mut parts = digits.splitn(2, '.');
    let whole: i64 = parts.next()?.parse().ok()?;
    let cents = match parts.next() {
        None => 0,
        Some(frac) => {
            let mut f: String = frac.chars().filter(|c| c.is_ascii_digit()).collect();
            f.truncate(2);
            while f.len() < 2 {
                f.push('0');
            }
            f.parse().ok()?
        }
    };
    let total = whole * 100 + cents;
    Some(if negative { -total } else { total })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn price(from: &str, cents: i64, cycle: BillingCycle) -> PricePoint {
        PricePoint {
            valid_from: from.into(),
            amount_cents: cents,
            currency: "EUR".into(),
            cycle,
            reason: String::new(),
        }
    }

    fn state(on: &str, state: State) -> StateChange {
        StateChange {
            effective: on.into(),
            state,
            note: String::new(),
        }
    }

    /// The worked example from the spec: 20 EUR a month, raised to 100 in October.
    /// Two rows, and the answer depends on when you ask.
    fn two_price_points() -> Subscription {
        Subscription {
            id: "sub_example".into(),
            name: "Example".into(),
            source_path: "Subscriptions/Example.md".into(),
            category: Some("productivity".into()),
            value_rating: Some(5),
            prices: vec![
                price("2026-02-01", 2000, BillingCycle::Monthly),
                price("2026-10-01", 10_000, BillingCycle::Monthly),
            ],
            states: vec![state("2026-02-01", State::Active)],
        }
    }

    #[test]
    fn the_price_in_force_depends_on_when_you_ask() {
        let sub = two_price_points();
        assert_eq!(sub.price_at("2026-08-08").unwrap().amount_cents, 2000);
        assert_eq!(sub.price_at("2026-10-01").unwrap().amount_cents, 10_000);
        assert_eq!(sub.price_at("2026-12-31").unwrap().amount_cents, 10_000);
    }

    #[test]
    fn a_date_before_the_first_price_point_reports_nothing_rather_than_zero() {
        assert!(two_price_points().price_at("2026-01-01").is_none());
        assert_eq!(two_price_points().monthly_cents_at("2026-01-01"), 0);
    }

    #[test]
    fn the_drift_between_two_dates_is_what_the_provider_did() {
        let drift = two_price_points()
            .price_drift_cents("2026-08-08", "2026-10-01")
            .unwrap();
        assert_eq!(drift, 8000, "20.00 to 100.00 is 80.00 of drift");
    }

    #[test]
    fn a_paused_subscription_contributes_nothing_and_resuming_brings_it_back() {
        let mut sub = two_price_points();
        sub.states.push(state("2026-06-01", State::Paused));
        assert_eq!(sub.state_at("2026-08-08"), State::Paused);
        assert_eq!(sub.monthly_cents_at("2026-08-08"), 0);

        sub.states.push(state("2026-09-01", State::Active));
        assert_eq!(sub.monthly_cents_at("2026-09-02"), 2000);
    }

    #[test]
    fn a_trial_is_billing_even_while_it_costs_nothing() {
        let sub = Subscription {
            prices: vec![
                price("2026-01-01", 0, BillingCycle::Monthly),
                price("2026-02-01", 999, BillingCycle::Monthly),
            ],
            states: vec![state("2026-01-01", State::Trial)],
            ..two_price_points()
        };
        assert!(sub.state_at("2026-01-15").is_billing());
        assert_eq!(sub.monthly_cents_at("2026-01-15"), 0);
        assert_eq!(sub.monthly_cents_at("2026-02-15"), 999);
    }

    #[test]
    fn cancelling_does_not_erase_what_it_cost() {
        let mut sub = two_price_points();
        sub.states.push(state("2026-11-01", State::Cancelled));
        assert_eq!(sub.monthly_cents_at("2026-12-01"), 0);
        assert_eq!(sub.price_at("2026-12-01").unwrap().amount_cents, 10_000);
    }

    #[test]
    fn yearly_and_quarterly_convert_to_a_monthly_equivalent() {
        assert_eq!(BillingCycle::Yearly.monthly_cents(12_000), 1000);
        assert_eq!(BillingCycle::Quarterly.monthly_cents(3000), 1000);
        assert_eq!(BillingCycle::Monthly.monthly_cents(1000), 1000);
        assert_eq!(BillingCycle::OneOff.monthly_cents(50_000), 0);
    }

    #[test]
    fn weekly_is_fifty_two_over_twelve_not_four_weeks_a_month() {
        // 10.00 a week is 43.33 a month, not 40.00. Four-week months would
        // under-report by about 8%, and the error compounds across the year.
        assert_eq!(BillingCycle::Weekly.monthly_cents(1000), 4333);
    }

    #[test]
    fn conversion_rounds_rather_than_truncating() {
        // 100.00 a year is 8.333... a month. Truncation loses a cent per
        // subscription per month and the burn drifts below the bank.
        assert_eq!(BillingCycle::Yearly.monthly_cents(10_000), 833);
        assert_eq!(BillingCycle::Yearly.monthly_cents(10_100), 842);
        assert_eq!(BillingCycle::Quarterly.monthly_cents(1000), 333);
    }

    #[test]
    fn burn_sums_only_what_is_actually_billing() {
        let active = two_price_points();

        // Paused *after* the seeded active state. Dating it earlier leaves the
        // later Active in force, which is correct and was how the first draft of
        // this test managed to assert the wrong thing.
        let mut paused = two_price_points();
        paused.id = "sub_paused".into();
        paused.states.push(state("2026-03-01", State::Paused));

        let considering = Subscription {
            id: "sub_considering".into(),
            prices: vec![price("2026-01-01", 50_000, BillingCycle::Monthly)],
            states: vec![],
            ..two_price_points()
        };

        let burn = burn_at(&[active, paused, considering], "2026-08-08");
        assert_eq!(burn.monthly_cents, 2000);
        assert_eq!(burn.annual_cents, 24_000);
        assert_eq!(burn.billing_count, 1);
    }

    #[test]
    fn burn_before_a_price_increase_reports_the_old_figure() {
        let subs = vec![two_price_points()];
        assert_eq!(burn_at(&subs, "2026-09-30").monthly_cents, 2000);
        assert_eq!(burn_at(&subs, "2026-10-01").monthly_cents, 10_000);
    }

    #[test]
    fn an_empty_set_and_a_fully_paused_set_are_distinguishable() {
        let mut paused = two_price_points();
        paused.states.push(state("2026-03-01", State::Paused));
        assert_eq!(burn_at(&[], "2026-08-08").billing_count, 0);
        assert_eq!(burn_at(&[paused], "2026-08-08").billing_count, 0);
        assert_eq!(
            burn_at(&[two_price_points()], "2026-08-08").billing_count,
            1
        );
    }

    #[test]
    fn a_german_decimal_comma_parses_rather_than_truncating_to_euros() {
        assert_eq!(decimal_to_cents("9,99"), Some(999));
        assert_eq!(decimal_to_cents("9.99"), Some(999));
        assert_eq!(decimal_to_cents("20"), Some(2000));
        assert_eq!(decimal_to_cents("100.5"), Some(10_050));
        assert_eq!(decimal_to_cents("12,34 EUR"), Some(1234));
        assert_eq!(decimal_to_cents(""), None);
        assert_eq!(decimal_to_cents("free"), None);
    }

    #[test]
    fn cents_render_with_two_places() {
        assert_eq!(cents_to_decimal(999), "9.99");
        assert_eq!(cents_to_decimal(2000), "20.00");
        assert_eq!(cents_to_decimal(5), "0.05");
        assert_eq!(cents_to_decimal(-250), "-2.50");
    }

    #[test]
    fn state_parsing_accepts_both_spellings_of_cancelled() {
        assert_eq!(State::parse("cancelled"), Some(State::Cancelled));
        assert_eq!(State::parse("Canceled"), Some(State::Cancelled));
        assert_eq!(State::parse(" Active "), Some(State::Active));
        assert_eq!(State::parse("nonsense"), None);
    }
}
