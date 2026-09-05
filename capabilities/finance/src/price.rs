//! Market prices: what was observed, by whom, and what every attempt did.
//!
//! Two rules govern every provider in this file, and both were bought with a
//! measurement rather than reasoned from taste.
//!
//! **A response is validated by the shape of its body, never by its status
//! code.** Measured 2026-09-05: `GET https://stooq.com/q/d/l/?s=spy.us&i=d`
//! answers **HTTP 200** with a 796-byte HTML body carrying a JavaScript
//! proof-of-work loop. A client that trusts the 200 parses HTML as CSV and
//! writes garbage into an append-only table nothing can correct. So a CSV body
//! whose first line is not the expected header is a `refused` fetch with a named
//! detail, whatever the status was. `upstreams.toml` carries the reject rows for
//! stooq and tradegate so the measurement outlives the memory of it.
//!
//! **A per-instrument failure is a recorded row, never a fatal run.** Every
//! attempt writes a `finance_price_fetches` row with a status, modelled on
//! `capabilities/places/src/store.rs`'s geocode cache. One instrument refusing
//! must not cost the other twelve their prices.
//!
//! A fetched price is **never** written back into the reviewed holdings
//! snapshot. `investment::validate_source_snapshot` recomputes the content hash
//! over `latest_unit_price`, so a quote written there would make the file refuse
//! itself on the next read.
//!
//! These tables are NOT `finance_price_points`, which is Axon's own subscription
//! pricing history (PRD §9.2 dogfooding). Market data and what Axon pays for a
//! streaming service are two different series that happen to share a word.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::Serialize;

use crate::clock;
use crate::config::InstrumentProfile;
use crate::investment::{parse_decimal, Quantity};
use crate::store::FinanceStore;

/// Every provider name that may reach `finance_prices.source`.
///
/// This const is the enumeration the column deliberately does not carry as a
/// CHECK: the provider set grows, SQLite cannot alter a CHECK, and this crate
/// has no table-rebuild path at all (`store.rs`'s migration doc comment).
/// `every_registered_provider_name_round_trips` is the test that replaces it.
pub const PROVIDERS: &[&str] = &["broker", "yahoo", "ecb"];

/// What the FX providers publish rates against.
pub const FX_BASE: &str = "EUR";

/// One observed price. An observation, never a correction: two sources may hold
/// one instrument-day and the reader prefers the newest `fetched_at` and reports
/// which source it used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PriceObservation {
    pub instrument: String,
    pub observed_on: String,
    pub price: Quantity,
    pub currency: String,
    pub source: String,
    pub fetched_at: String,
}

/// One published FX reference rate, in quote units per one base unit, stored as
/// published. Never inverted at write time: a division is where an exact decimal
/// stops being exact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FxObservation {
    pub base: String,
    pub quote: String,
    pub observed_on: String,
    pub rate: Quantity,
    pub source: String,
    pub fetched_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FetchStatus {
    /// Rows were written.
    Ok,
    /// The provider answered correctly and had nothing to give.
    Empty,
    /// The provider answered something that is not the contract: a gate page, a
    /// body that is not CSV, an instrument with no configured ticker.
    Refused,
    /// The request itself failed: no route, a timeout, a torn body.
    Error,
}

impl FetchStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Empty => "empty",
            Self::Refused => "refused",
            Self::Error => "error",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "ok" => Some(Self::Ok),
            "empty" => Some(Self::Empty),
            "refused" => Some(Self::Refused),
            "error" => Some(Self::Error),
            _ => None,
        }
    }
}

/// One attempt against one target, successful or not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FetchAttempt {
    pub provider: String,
    pub target: String,
    pub requested_on: String,
    pub status: FetchStatus,
    /// A bounded reason -- an HTTP status, a note that the body was not CSV.
    /// Never a response body: a provider page can contain anything and this
    /// table is inside the backup contract.
    pub detail: String,
    pub rows_written: i64,
    pub fetched_at: String,
}

/// What one provider produced for one target.
pub struct ProviderResult {
    pub prices: Vec<PriceObservation>,
    pub rates: Vec<FxObservation>,
    pub status: FetchStatus,
    pub detail: String,
}

impl ProviderResult {
    pub fn refused(detail: impl Into<String>) -> Self {
        Self {
            prices: Vec::new(),
            rates: Vec::new(),
            status: FetchStatus::Refused,
            detail: detail.into(),
        }
    }

    pub fn error(detail: impl Into<String>) -> Self {
        Self {
            prices: Vec::new(),
            rates: Vec::new(),
            status: FetchStatus::Error,
            detail: detail.into(),
        }
    }
}

/// What a provider needs to know to fetch. Assembled by the caller so a provider
/// opens no store and reads no config file of its own.
pub struct FetchContext<'a> {
    pub instruments: &'a [InstrumentProfile],
    /// The reviewed broker prices, by instrument. The `broker` provider's whole
    /// input; the networked providers ignore it.
    pub broker_prices: &'a BTreeMap<String, (Quantity, String, String)>,
    pub as_of: &'a str,
    pub fetched_at: &'a str,
    /// How many days of history a networked provider should ask for.
    pub history_days: u32,
}

/// A source of prices.
///
/// `targets` and `fetch` are separate so orchestration can write one
/// `finance_price_fetches` row per target even when the provider itself never
/// runs -- an instrument with no configured ticker is a recorded `refused`, not
/// a silence.
pub trait PriceProvider {
    fn name(&self) -> &'static str;

    /// The targets this provider would attempt, in order. A target is opaque to
    /// the caller and meaningful to the provider: an instrument symbol, a
    /// currency pair.
    fn targets(&self, context: &FetchContext<'_>) -> Vec<String>;

    /// Fetch one target. Must never panic and must never abort the run: a
    /// failure is a `ProviderResult` with a status.
    fn fetch(&self, target: &str, context: &FetchContext<'_>) -> ProviderResult;
}

// ---------------------------------------------------------------------------
// broker
// ---------------------------------------------------------------------------

/// Replays the reviewed activity prices already in `finance_holding_projection`.
///
/// No network, so it always works, and it is what makes the Investments tab
/// useful on a machine with no internet. It writes one observation per priced
/// instrument per run, which is also its limit: a history built this way grows
/// one point per fetch, so it is a floor under the price series and never a
/// substitute for a market source.
pub struct BrokerProvider;

impl PriceProvider for BrokerProvider {
    fn name(&self) -> &'static str {
        "broker"
    }

    fn targets(&self, context: &FetchContext<'_>) -> Vec<String> {
        context.broker_prices.keys().cloned().collect()
    }

    fn fetch(&self, target: &str, context: &FetchContext<'_>) -> ProviderResult {
        let Some((price, currency, observed_on)) = context.broker_prices.get(target) else {
            return ProviderResult::refused("instrument is not in the reviewed projection");
        };
        ProviderResult {
            prices: vec![PriceObservation {
                instrument: target.to_string(),
                observed_on: observed_on.clone(),
                price: price.clone(),
                currency: currency.clone(),
                source: "broker".into(),
                fetched_at: context.fetched_at.to_string(),
            }],
            rates: Vec::new(),
            status: FetchStatus::Ok,
            detail: String::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// The shape rule, shared by every networked provider
// ---------------------------------------------------------------------------

/// The header a CSV body must start with, or the body is not the contract.
///
/// Named rather than inlined because it is the rule the Stooq measurement bought:
/// a 200 with an HTML gate page fails here and nowhere else.
pub fn csv_header_matches(body: &str, required: &[&str]) -> bool {
    let Some(first) = body.lines().next() else {
        return false;
    };
    let columns: Vec<&str> = first
        .split(',')
        .map(|column| column.trim().trim_matches('"'))
        .collect();
    required
        .iter()
        .all(|name| columns.iter().any(|column| column == name))
}

/// A body that is HTML rather than the data contract, whatever the status said.
pub fn looks_like_html(body: &str) -> bool {
    let head = body.trim_start().get(..512).unwrap_or(body.trim_start());
    let lowered = head.to_ascii_lowercase();
    lowered.starts_with("<!doctype")
        || lowered.starts_with("<html")
        || lowered.contains("<script")
        || lowered.contains("<noscript")
}

/// One shared blocking client. Built by the caller inside `spawn_blocking`,
/// never on the async runtime: a blocking reqwest client driven from a Tokio
/// worker panics at run time rather than failing to compile.
pub fn blocking_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .cookie_store(true)
        .user_agent("axon-finance/1.0 (personal use; https://github.com/larsboes/Axon)")
        .build()
        .map_err(|error| format!("client could not be built: {error}"))
}

// ---------------------------------------------------------------------------
// Orchestration
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FetchRun {
    pub provider: String,
    pub attempted: usize,
    pub written: i64,
    pub refused: usize,
    pub errored: usize,
    pub attempts: Vec<FetchAttempt>,
}

impl FetchRun {
    /// True when at least one target produced a row. The CLI's exit status: a
    /// run in which every target refused is a failure worth a non-zero exit,
    /// while one refusal among twelve successes is not.
    pub fn succeeded(&self) -> bool {
        self.written > 0
    }
}

/// Run one provider over every target it names, writing a fetch row per attempt.
///
/// `dry_run` writes nothing at all -- not the prices and not the attempt rows --
/// because a recorded attempt that never happened is a lie in the one table that
/// exists to answer why an instrument is stale.
pub fn run_provider(
    store: &FinanceStore,
    provider: &dyn PriceProvider,
    context: &FetchContext<'_>,
    dry_run: bool,
) -> Result<FetchRun, String> {
    let mut run = FetchRun {
        provider: provider.name().to_string(),
        attempted: 0,
        written: 0,
        refused: 0,
        errored: 0,
        attempts: Vec::new(),
    };
    for target in provider.targets(context) {
        run.attempted += 1;
        let result = provider.fetch(&target, context);
        let mut written = 0i64;
        if !dry_run {
            for observation in &result.prices {
                match store.append_market_price(observation) {
                    Ok(true) => written += 1,
                    Ok(false) => {}
                    Err(error) => return Err(error.to_string()),
                }
            }
            for rate in &result.rates {
                match store.append_fx_rate(rate) {
                    Ok(true) => written += 1,
                    Ok(false) => {}
                    Err(error) => return Err(error.to_string()),
                }
            }
        } else {
            written = (result.prices.len() + result.rates.len()) as i64;
        }
        let status = match result.status {
            // A provider that answered correctly and produced nothing new is
            // `empty`, not `ok`: an idempotent re-fetch and a source with no
            // data are different facts.
            FetchStatus::Ok if result.prices.is_empty() && result.rates.is_empty() => {
                FetchStatus::Empty
            }
            other => other,
        };
        match status {
            FetchStatus::Refused => run.refused += 1,
            FetchStatus::Error => run.errored += 1,
            _ => {}
        }
        run.written += written;
        let attempt = FetchAttempt {
            provider: provider.name().to_string(),
            target: target.clone(),
            requested_on: context.as_of.to_string(),
            status,
            detail: truncate_detail(&result.detail),
            rows_written: written,
            fetched_at: context.fetched_at.to_string(),
        };
        if !dry_run {
            store
                .record_fetch(&attempt)
                .map_err(|error| error.to_string())?;
        }
        run.attempts.push(attempt);
    }
    Ok(run)
}

/// A reason a human reads, bounded so no provider can grow this column.
fn truncate_detail(detail: &str) -> String {
    let cleaned: String = detail
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.chars().count() <= 200 {
        return cleaned.to_string();
    }
    cleaned.chars().take(197).collect::<String>() + "..."
}

/// Build the `broker_prices` map a [`FetchContext`] needs from the reviewed
/// snapshot. The observation date is the snapshot's own review date, so a broker
/// point never claims to have been observed later than the file it came from.
pub fn broker_prices_from_snapshot(
    snapshot: &crate::investment::ReviewedHoldingsSnapshot,
) -> BTreeMap<String, (Quantity, String, String)> {
    let mut map = BTreeMap::new();
    for holding in &snapshot.holdings {
        let Some(price) = holding.latest_unit_price.clone() else {
            continue;
        };
        let observed_on = holding_review_date(snapshot, &holding.instrument);
        map.insert(
            holding.instrument.clone(),
            (price, holding.currency.clone(), observed_on),
        );
    }
    map
}

fn holding_review_date(
    snapshot: &crate::investment::ReviewedHoldingsSnapshot,
    _instrument: &str,
) -> String {
    // The snapshot's own review date. A per-source date would be more precise,
    // but `finance_holding_projection` does not record which source a holding
    // came from, and inventing that link here would be a provenance claim this
    // module cannot back.
    if clock::valid_iso_date(&snapshot.reviewed_at) {
        snapshot.reviewed_at.clone()
    } else {
        snapshot
            .reviewed_at
            .get(..10)
            .filter(|head| clock::valid_iso_date(head))
            .map(str::to_string)
            .unwrap_or_else(clock::today)
    }
}

/// Parse a provider decimal through the reader the CSV import already uses, which
/// handles both decimal marks, grouping separators and a trailing minus.
pub fn parse_provider_decimal(value: &str) -> Result<Quantity, String> {
    parse_decimal(value.trim(), '.', "price").map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The body Stooq served on 2026-09-05, trimmed to its shape. Served with
    /// HTTP 200. It is a test fixture because the 200 is the trap: a client that
    /// checks the status parses this as CSV.
    const STOOQ_PROOF_OF_WORK_BODY: &str = concat!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\"></head><body>\n",
        "<script>async function s(){for(let n=0;;n++){",
        "const h=await crypto.subtle.digest(\"SHA-256\",new TextEncoder().encode(n));",
        "if(new Uint8Array(h)[0]===0&&new Uint8Array(h)[1]===0)return n}}</script>\n",
        "<noscript>Please enable JavaScript to continue.</noscript>\n",
        "</body></html>\n"
    );

    #[test]
    fn a_proof_of_work_gate_is_not_a_csv() {
        // The measurement, not the taste: this body arrived with HTTP 200.
        assert!(looks_like_html(STOOQ_PROOF_OF_WORK_BODY));
        assert!(!csv_header_matches(
            STOOQ_PROOF_OF_WORK_BODY,
            &["Date", "Close"]
        ));
        assert!(csv_header_matches(
            "Date,Open,High,Low,Close,Volume\n2026-09-04,1,2,0,1.5,10\n",
            &["Date", "Close"]
        ));
    }

    #[test]
    fn every_registered_provider_name_is_a_distinct_lowercase_token() {
        for name in PROVIDERS {
            assert!(!name.is_empty());
            assert!(name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()));
        }
        let mut sorted = PROVIDERS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), PROVIDERS.len());
    }

    #[test]
    fn a_price_never_becomes_a_float() {
        let parsed = parse_provider_decimal("128.42").expect("a decimal");
        assert_eq!(parsed.mantissa, 12_842);
        assert_eq!(parsed.scale, 2);
    }

    #[test]
    fn a_detail_is_bounded_and_carries_no_control_characters() {
        let detail = truncate_detail(&format!("a\nb{}", "x".repeat(500)));
        assert_eq!(detail.chars().count(), 200);
        assert!(!detail.contains('\n'));
    }

    #[test]
    fn a_broker_target_missing_from_the_projection_is_refused_not_skipped() {
        let broker = BrokerProvider;
        let empty = BTreeMap::new();
        let context = FetchContext {
            instruments: &[],
            broker_prices: &empty,
            as_of: "2026-09-05",
            fetched_at: "2026-09-05T00:00:00Z",
            history_days: 400,
        };
        let result = broker.fetch("SYNTH", &context);
        assert_eq!(result.status, FetchStatus::Refused);
        assert!(result.prices.is_empty());
    }
}
