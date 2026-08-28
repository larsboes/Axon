//! Reading subscription notes out of the vault, and writing derived figures back.
//!
//! The boundary this file implements, and the reason it is worth reading closely:
//! the vault owns why the principal pays for something, whether it is worth it, and
//! what the alternatives are. This capability owns the price series, the state
//! series, and anything computed from them. Neither writes the other's fields.
//!
//! Reading is one direction of that. A note's frontmatter seeds a subscription: its
//! current cost becomes the *first* price point, its status becomes the *first*
//! state change. From then on the series is authoritative and the frontmatter is
//! not re-read for those fields, because a series cannot be reconstructed from the
//! single mutable number it replaced.
//!
//! Writing is the other. Derived figures go into a marked region via
//! `markdown_root::region`, which guarantees bytes outside the markers survive and
//! refuses to overwrite a region a human edited. This module never opens a file for
//! writing outside that path.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use markdown_root::{frontmatter, region, MarkdownRoot, RegionOutcome, RegionSpec};

use crate::subscription::{
    cents_to_decimal, decimal_to_cents, BillingCycle, PricePoint, State, StateChange, Subscription,
};

/// The region marker owner. Stable forever: changing it orphans every region
/// already written into the vault and the next write appends a second one.
pub const REGION_OWNER: &str = "finance";

/// Bumped when the rendered block's shape changes, so a later generator can tell
/// its own old output from a shape it no longer produces.
///
/// 2 (2026-08-28): the price and state series joined the current-state callout, per
/// PRD Q47. The bump does not force a rewrite by itself — the region's hash does that,
/// because every v1 body differs from its v2 replacement.
pub const REGION_VERSION: u32 = 2;

/// A note found by the scanner, before anything is persisted.
#[derive(Debug, Clone, PartialEq)]
pub struct ScannedNote {
    /// Vault-relative, slash-separated. The import identity.
    pub source_path: String,
    pub absolute: PathBuf,
    pub name: String,
    pub fields: HashMap<String, String>,
}

#[derive(Debug)]
pub enum ScanError {
    Root(markdown_root::RootError),
    Read { path: PathBuf, detail: String },
    Frontmatter { path: PathBuf, detail: String },
}

impl std::fmt::Display for ScanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScanError::Root(e) => write!(f, "{e}"),
            ScanError::Read { path, detail } => {
                write!(f, "cannot read {}: {detail}", path.display())
            }
            ScanError::Frontmatter { path, detail } => {
                write!(f, "frontmatter in {}: {detail}", path.display())
            }
        }
    }
}

impl std::error::Error for ScanError {}

/// Every markdown note directly inside the configured subscriptions directory.
///
/// Read-only, and bounded by `MarkdownRoot`, so a misconfigured directory cannot
/// walk out of the declared vault. A note that fails to parse is an error naming
/// the file rather than a silently skipped subscription: a subscription that
/// vanishes from a burn total because its frontmatter had a typo is exactly the
/// kind of quietly-wrong figure this whole capability exists to avoid.
pub fn scan(root: &MarkdownRoot, dir: &Path) -> Result<Vec<ScannedNote>, ScanError> {
    let pattern = format!("{}/*.md", dir.to_string_lossy().trim_end_matches('/'));
    let files = root.markdown_files(&pattern).map_err(ScanError::Root)?;

    let mut out = Vec::with_capacity(files.len());
    for file in files {
        let body = std::fs::read_to_string(&file).map_err(|e| ScanError::Read {
            path: file.clone(),
            detail: e.to_string(),
        })?;
        let fields = frontmatter(&body).map_err(|detail| ScanError::Frontmatter {
            path: file.clone(),
            detail,
        })?;
        let source_path = root
            .relative_id(&file)
            .unwrap_or_else(|| file.to_string_lossy().into_owned());
        let name = file
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| source_path.clone());
        out.push(ScannedNote {
            source_path,
            absolute: file,
            name,
            fields,
        });
    }
    Ok(out)
}

/// Map a note's frontmatter onto the shape of a subscription.
///
/// Only ever used to *seed*. Re-running it against a note whose series has since
/// moved on would throw that series away, so the store imports by path and leaves
/// an existing subscription's history alone.
///
/// The vault's own vocabulary is honoured rather than replaced. New notes can use
/// `cost` plus an ISO currency code; `cost_eur` remains a compatible shorthand for
/// the existing notes. A `start_date` seeds the first price point's date; without
/// one the caller's `today` is used, which is wrong-but-visible rather than invented.
pub fn seed_from_note(note: &ScannedNote, today: &str) -> Subscription {
    let f = &note.fields;

    let cycle = f
        .get("billing_cycle")
        .map(|c| match c.trim().to_ascii_lowercase().as_str() {
            "weekly" => BillingCycle::Weekly,
            "quarterly" => BillingCycle::Quarterly,
            "yearly" | "annual" | "annually" => BillingCycle::Yearly,
            "once" | "one_off" | "one-off" => BillingCycle::OneOff,
            _ => BillingCycle::Monthly,
        })
        .unwrap_or(BillingCycle::Monthly);

    let start = f
        .get("start_date")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or(today)
        .to_string();

    let currency = f
        .get("currency")
        .map(|value| value.trim().to_ascii_uppercase())
        .filter(|value| value.len() == 3)
        .unwrap_or_else(|| "EUR".into());
    let prices = f
        .get("cost")
        .or_else(|| f.get("cost_eur"))
        .and_then(|raw| decimal_to_cents(raw))
        .map(|amount_cents| {
            vec![PricePoint {
                valid_from: start.clone(),
                amount_cents,
                currency,
                cycle,
                plan: f
                    .get("plan")
                    .map(|p| p.trim().to_string())
                    .filter(|p| !p.is_empty()),
                reason: "seeded from the vault note".into(),
            }]
        })
        .unwrap_or_default();

    let states = f
        .get("status")
        .and_then(|s| State::parse(s))
        .map(|state| {
            vec![StateChange {
                effective: start,
                state,
                note: "seeded from the vault note".into(),
            }]
        })
        .unwrap_or_default();

    Subscription {
        id: String::new(), // assigned by the store
        name: note.name.clone(),
        source_path: note.source_path.clone(),
        category: f.get("category").map(|c| c.trim().to_string()),
        value_rating: f.get("value_rating").and_then(|v| v.trim().parse().ok()),
        prices,
        states,
    }
}

/// The body of the derived block, as it appears between the markers.
///
/// Every line here is computed. Nothing a human typed is reproduced, because a copy
/// of somebody's prose inside a machine-owned region is a second writable home for
/// it, which is the doubling the whole boundary exists to prevent.
///
/// Two parts, and they answer different questions. The callout is the **current
/// state**: what it costs now, what happens next, whether it has drifted. The two
/// tables below it are the **series** — every price point and every state change, as
/// rows. Version 1 rendered only the callout.
///
/// The series are here because PRD Q47 (2026-08-27) counted `finance_price_points` and
/// `finance_state_changes` among the 512 irreplaceable rows in the store and made
/// projecting them a rule: a price is observed once, on the day it changed, and a
/// series cannot be recomputed from anything. A current-state summary is not a copy of
/// them. This region is the one existing machine→vault writer, so the safety copy goes
/// where the writer already is rather than into a second file.
pub fn render_block(sub: &Subscription, today: &str) -> String {
    let mut out = String::new();
    out.push_str("> [!info] Derived by Axon — do not edit inside this block\n");

    match sub.price_at(today) {
        Some(p) => {
            out.push_str(&format!(
                "> **Current price:** {} {} / {}\n",
                cents_to_decimal(p.amount_cents),
                p.currency,
                cycle_word(p.cycle)
            ));
            out.push_str(&format!(
                "> **Monthly equivalent:** {} {}\n",
                cents_to_decimal(p.cycle.monthly_cents(p.amount_cents)),
                p.currency
            ));
            if let Some(plan) = &p.plan {
                out.push_str(&format!("> **Plan:** {plan}\n"));
            }
        }
        None => out.push_str("> **Current price:** not recorded yet\n"),
    }

    out.push_str(&format!("> **State:** {}\n", sub.state_at(today).as_str()));

    // Drift only when something has actually drifted. The first version keyed off
    // `prices.len() > 1`, which rendered "drift: down 0.00" for a subscription
    // whose second price point is still in the future — a line that says a price
    // moved when none has, on the one surface meant to notice exactly that.
    if let Some(first) = sub.prices.first() {
        match sub.price_drift_cents(&first.valid_from, today) {
            Some(drift) if drift != 0 => {
                out.push_str(&format!(
                    "> **Price drift since {}:** {} {} {}\n",
                    first.valid_from,
                    if drift > 0 { "up" } else { "down" },
                    cents_to_decimal(drift.abs()),
                    first.currency
                ));
            }
            _ => {}
        }
    }

    // A price point dated ahead of today is the increase you want warning about
    // before it bills, which is the whole reason the series is dated rather than
    // overwritten.
    if let Some(next) = sub
        .prices
        .iter()
        .filter(|p| p.valid_from.as_str() > today)
        .min_by(|a, b| a.valid_from.cmp(&b.valid_from))
    {
        out.push_str(&format!(
            "> **Scheduled:** {}{} {} / {} from {}\n",
            next.plan
                .as_deref()
                .map(|p| format!("{p}, "))
                .unwrap_or_default(),
            cents_to_decimal(next.amount_cents),
            next.currency,
            cycle_word(next.cycle),
            next.valid_from
        ));
    }

    // The count line that used to sit here is gone. The table below holds every price
    // point, so a count beside it is the same fact written twice, and the second copy
    // is the one that goes wrong.

    out.push_str("\n#### Price series\n\n");
    if sub.prices.is_empty() {
        out.push_str("None recorded.\n");
    } else {
        out.push_str("| From | Amount | Cycle | Plan | Reason |\n|---|---|---|---|---|\n");
        for price in &sub.prices {
            out.push_str(&format!(
                "| {} | {} {} | {} | {} | {} |\n",
                price.valid_from,
                cents_to_decimal(price.amount_cents),
                price.currency,
                cycle_word(price.cycle),
                cell(price.plan.as_deref().unwrap_or("")),
                cell(&price.reason),
            ));
        }
    }

    out.push_str("\n#### State series\n\n");
    if sub.states.is_empty() {
        out.push_str("None recorded.\n");
    } else {
        out.push_str("| From | State | Note |\n|---|---|---|\n");
        for change in &sub.states {
            out.push_str(&format!(
                "| {} | {} | {} |\n",
                change.effective,
                change.state.as_str(),
                cell(&change.note),
            ));
        }
    }

    out
}

/// One table cell.
///
/// A `|` in a reason ends the row early and shifts every column after it, which turns
/// a safety copy into a wrong one silently. A newline does the same to the whole table.
/// Both are escaped rather than stripped: the text is the record.
fn cell(text: &str) -> String {
    let flattened = text.replace(['\n', '\r'], " ");
    let escaped = flattened.replace('|', "\\|");
    let trimmed = escaped.trim();
    if trimmed.is_empty() {
        "—".to_string()
    } else {
        trimmed.to_string()
    }
}

fn cycle_word(cycle: BillingCycle) -> &'static str {
    match cycle {
        BillingCycle::Weekly => "week",
        BillingCycle::Monthly => "month",
        BillingCycle::Quarterly => "quarter",
        BillingCycle::Yearly => "year",
        BillingCycle::OneOff => "one-off",
    }
}

/// What a writeback attempt did to one note.
#[derive(Debug, Clone, PartialEq)]
pub enum WriteBack {
    Created,
    Updated,
    /// Already correct. The file was not opened for writing at all, so the vault's
    /// git history stays free of no-op commits.
    Unchanged,
    /// A human edited inside the region. Nothing was written, and both revisions
    /// come back so the caller can show them rather than pick one.
    Conflict {
        theirs: String,
        ours: String,
    },
}

/// Regenerate one note's derived block.
///
/// The write happens only on `Created` and `Updated`. Every other outcome, conflict
/// included, leaves the file exactly as it was found.
pub fn write_block(
    path: &Path,
    sub: &Subscription,
    today: &str,
) -> Result<WriteBack, Box<dyn std::error::Error>> {
    let original = std::fs::read_to_string(path)?;
    let spec = RegionSpec::new(REGION_OWNER, REGION_VERSION);
    let (updated, outcome) = region::apply(&original, &spec, &render_block(sub, today))?;

    match outcome {
        RegionOutcome::Created => {
            std::fs::write(path, updated)?;
            Ok(WriteBack::Created)
        }
        RegionOutcome::Updated => {
            std::fs::write(path, updated)?;
            Ok(WriteBack::Updated)
        }
        RegionOutcome::Unchanged => Ok(WriteBack::Unchanged),
        RegionOutcome::Conflict { theirs, ours } => Ok(WriteBack::Conflict { theirs, ours }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(fields: &[(&str, &str)], name: &str) -> ScannedNote {
        ScannedNote {
            source_path: format!("Atlas/Finance/Subscriptions/{name}.md"),
            absolute: PathBuf::from(format!("/nowhere/{name}.md")),
            name: name.to_string(),
            fields: fields
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    #[test]
    fn a_notes_cost_becomes_the_first_price_point_not_a_mutable_field() {
        let sub = seed_from_note(
            &note(
                &[
                    ("cost_eur", "20"),
                    ("billing_cycle", "monthly"),
                    ("status", "active"),
                    ("start_date", "2026-02-01"),
                    ("value_rating", "5"),
                    ("category", "productivity"),
                ],
                "Example",
            ),
            "2026-08-08",
        );

        assert_eq!(sub.prices.len(), 1);
        assert_eq!(sub.prices[0].amount_cents, 2000);
        assert_eq!(sub.prices[0].valid_from, "2026-02-01");
        assert_eq!(sub.states[0].state, State::Active);
        assert_eq!(sub.value_rating, Some(5));
        assert_eq!(sub.category.as_deref(), Some("productivity"));
    }

    #[test]
    fn generic_cost_and_currency_support_non_euro_notes() {
        let note = note(
            &[
                ("cost", "12.50"),
                ("currency", "usd"),
                ("billing_cycle", "monthly"),
                ("status", "active"),
            ],
            "Synthetic USD plan",
        );
        let subscription = seed_from_note(&note, "2026-08-09");
        assert_eq!(subscription.prices[0].amount_cents, 1_250);
        assert_eq!(subscription.prices[0].currency, "USD");
    }

    #[test]
    fn a_yearly_note_seeds_a_yearly_cycle() {
        let sub = seed_from_note(
            &note(&[("cost_eur", "120"), ("billing_cycle", "yearly")], "Drive"),
            "2026-08-08",
        );
        assert_eq!(sub.prices[0].cycle, BillingCycle::Yearly);
        assert_eq!(
            sub.monthly_cents_at("2026-08-08"),
            0,
            "no status means not billing"
        );
    }

    #[test]
    fn a_note_with_no_cost_seeds_no_price_rather_than_a_zero() {
        let sub = seed_from_note(&note(&[("status", "considering")], "Maybe"), "2026-08-08");
        assert!(sub.prices.is_empty());
        assert_eq!(sub.state_at("2026-08-08"), State::Considering);
    }

    #[test]
    fn a_missing_start_date_falls_back_to_today_rather_than_inventing_one() {
        let sub = seed_from_note(&note(&[("cost_eur", "9,99")], "Thing"), "2026-08-08");
        assert_eq!(sub.prices[0].valid_from, "2026-08-08");
        assert_eq!(sub.prices[0].amount_cents, 999);
    }

    #[test]
    fn the_rendered_block_is_entirely_computed() {
        let sub = Subscription {
            id: "s1".into(),
            name: "Example".into(),
            source_path: "Subscriptions/Example.md".into(),
            category: None,
            value_rating: None,
            prices: vec![
                PricePoint {
                    valid_from: "2026-02-01".into(),
                    amount_cents: 2000,
                    currency: "EUR".into(),
                    cycle: BillingCycle::Monthly,
                    plan: None,
                    reason: String::new(),
                },
                PricePoint {
                    valid_from: "2026-07-01".into(),
                    amount_cents: 2500,
                    currency: "EUR".into(),
                    cycle: BillingCycle::Monthly,
                    plan: None,
                    reason: "provider raised it".into(),
                },
            ],
            states: vec![StateChange {
                effective: "2026-02-01".into(),
                state: State::Active,
                note: String::new(),
            }],
        };

        let block = render_block(&sub, "2026-08-08");
        assert!(block.contains("**Current price:** 25.00 EUR / month"));
        assert!(block.contains("**Monthly equivalent:** 25.00 EUR"));
        assert!(block.contains("**State:** active"));
        assert!(block.contains("**Price drift since 2026-02-01:** up 5.00 EUR"));
        // The count line this used to assert is gone: the series table below carries
        // every price point, and a count beside it is one fact written twice.
        assert!(block.contains("| 2026-02-01 | 20.00 EUR | month | — | — |"));
        assert!(block.contains("| 2026-07-01 | 25.00 EUR | month | — | provider raised it |"));
        assert!(block.contains("| 2026-02-01 | active | — |"));
    }

    /// The reason the series is in the region at all (PRD Q47): these rows exist
    /// nowhere else, so every one of them has to survive the render. A current-state
    /// summary is not a copy of a series.
    #[test]
    fn every_price_point_and_state_change_reaches_the_block() {
        let sub = Subscription {
            id: "s1".into(),
            name: "Example".into(),
            source_path: "x.md".into(),
            category: None,
            value_rating: None,
            prices: (0..5)
                .map(|i| PricePoint {
                    valid_from: format!("2026-0{}-01", i + 1),
                    amount_cents: 1000 + i * 100,
                    currency: "EUR".into(),
                    cycle: BillingCycle::Monthly,
                    plan: Some(format!("tier-{i}")),
                    reason: format!("step {i}"),
                })
                .collect(),
            states: (0..3)
                .map(|i| StateChange {
                    effective: format!("2026-0{}-01", i + 1),
                    state: State::Active,
                    note: format!("note {i}"),
                })
                .collect(),
        };
        let block = render_block(&sub, "2026-08-08");
        for i in 0..5 {
            assert!(block.contains(&format!("step {i}")), "price point {i} lost");
            assert!(block.contains(&format!("tier-{i}")), "plan {i} lost");
        }
        for i in 0..3 {
            assert!(
                block.contains(&format!("note {i}")),
                "state change {i} lost"
            );
        }
    }

    /// A pipe in a reason ends the row early and shifts every column after it, which
    /// is how a safety copy becomes a wrong one without anything failing.
    #[test]
    fn a_pipe_in_a_reason_is_escaped_rather_than_breaking_the_row() {
        let sub = Subscription {
            id: "s1".into(),
            name: "Example".into(),
            source_path: "x.md".into(),
            category: None,
            value_rating: None,
            prices: vec![PricePoint {
                valid_from: "2026-02-01".into(),
                amount_cents: 2000,
                currency: "EUR".into(),
                cycle: BillingCycle::Monthly,
                plan: None,
                reason: "moved Pro | Max\nafter the mail".into(),
            }],
            states: vec![],
        };
        let block = render_block(&sub, "2026-08-08");
        assert!(block.contains("moved Pro \\| Max after the mail"));
        let row = block
            .lines()
            .find(|l| l.contains("2026-02-01") && l.starts_with('|'))
            .expect("a price row");
        assert_eq!(
            row.matches("| ").count() - row.matches("\\| ").count(),
            5,
            "five cells, however many pipes the text held: {row}"
        );
    }

    #[test]
    fn a_subscription_with_no_price_says_so_rather_than_rendering_zero() {
        let sub = Subscription {
            id: "s1".into(),
            name: "Example".into(),
            source_path: "x.md".into(),
            category: None,
            value_rating: None,
            prices: vec![],
            states: vec![],
        };
        let block = render_block(&sub, "2026-08-08");
        assert!(block.contains("not recorded yet"));
        assert!(!block.contains("0.00"));
    }

    #[test]
    fn a_future_price_point_is_announced_rather_than_reported_as_drift() {
        // The case the fixture run caught: a second price point dated ahead of
        // today. Nothing has drifted, and saying so would be a false alarm on the
        // one surface built to raise real ones.
        let sub = Subscription {
            id: "s1".into(),
            name: "Example".into(),
            source_path: "x.md".into(),
            category: None,
            value_rating: None,
            prices: vec![
                PricePoint {
                    valid_from: "2026-08-08".into(),
                    amount_cents: 2000,
                    currency: "EUR".into(),
                    cycle: BillingCycle::Monthly,
                    plan: None,
                    reason: String::new(),
                },
                PricePoint {
                    valid_from: "2026-10-01".into(),
                    amount_cents: 10_000,
                    currency: "EUR".into(),
                    cycle: BillingCycle::Monthly,
                    plan: Some("Max".into()),
                    reason: "upgrade".into(),
                },
            ],
            states: vec![StateChange {
                effective: "2026-08-08".into(),
                state: State::Active,
                note: String::new(),
            }],
        };

        let block = render_block(&sub, "2026-08-08");
        assert!(!block.contains("drift"), "nothing has drifted yet");
        // The plan rides along, so the line answers "to what" as well as "to how much".
        assert!(block.contains("**Scheduled:** Max, 100.00 EUR / month from 2026-10-01"));
        assert!(block.contains("**Current price:** 20.00 EUR / month"));

        // Once it lands, it is drift and there is nothing left to schedule.
        let after = render_block(&sub, "2026-10-02");
        assert!(after.contains("**Price drift since 2026-08-08:** up 80.00 EUR"));
        assert!(!after.contains("Scheduled"));
    }

    #[test]
    fn a_single_price_point_renders_no_drift_line() {
        let sub = seed_from_note(
            &note(
                &[
                    ("cost_eur", "20"),
                    ("status", "active"),
                    ("start_date", "2026-02-01"),
                ],
                "Example",
            ),
            "2026-08-08",
        );
        let block = render_block(&sub, "2026-08-08");
        assert!(
            !block.contains("drift"),
            "no drift to report from one point"
        );
    }
}
