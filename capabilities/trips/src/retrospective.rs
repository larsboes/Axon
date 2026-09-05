//! The per-plan retrospective and the weight it feeds forward.
//!
//! Two grains, two names, and this file owns the second one.
//! `POST /api/plans/:id/outcome` records how one booked connection went against
//! the option it was chosen under — per-stage measurement. This is per-plan
//! judgement: three fields, ruled and closed by PRD §8.2 (cost, again?,
//! change?), written when a trip is over.
//!
//! What the summary publishes and what it deliberately does not: a factor **per
//! destination only**. The companion half was designed and cut. `trips` ends its
//! router with `CorsLayer::permissive()` and refuses no origin anywhere, while
//! `places` — which owns the companion register — layers `refuse_foreign_origins`
//! on its whole router exactly because that register is C2. A route keyed by a
//! traveler name, carrying a score and a basis of plan ids that resolve to
//! destinations and date ranges, is person + place + date range readable
//! cross-origin, and the shipped invariant for this same data (places ISA
//! PLC-12) is falsified by "a person name in its output". `by_destination` adds
//! no exposure, because `GET /api/plans` already serves `destinations` over the
//! same permissive layer. The three preconditions for bringing the companion
//! half back are written down in `capabilities/trips/README.md`.

use serde::{Deserialize, Serialize};

use crate::store::{normalize_place_name, Retrospective, TripPlan};

/// The published formula, served in the response so a consumer never has to read
/// this file to know what it is multiplying by.
pub const FORMULA: &str =
    "factor = 1 + 0.25 * mean_again * n/(n+2); again scores yes=+1, maybe=0, no=-1";

/// The contract a consumer is held to. Principle 5, and calendar's own
/// soft-verdict precedent: a ranked output explains and never filters.
pub const CONTRACT: &str =
    "multiply a candidate's rank by factor and show basis; never filter on it";

/// Why `by_companion` is a sentence rather than data. Served at the wire so the
/// omission is discovered here rather than by finding an absent key.
pub const COMPANION_NOTE: &str = "not served — see capabilities/trips/README.md";

/// One destination while it accumulates: the normalized key, one `again` score
/// per recorded trip, the plan ids that produced them, and the overruns from the
/// plans that carried both a budget and a cost. Named because it is built in one
/// place and read positionally in another, which is exactly what clippy's
/// `type_complexity` asks to be given a name.
type Group = (String, Vec<f64>, Vec<String>, Vec<i64>);

const WEIGHT: f64 = 0.25;
/// The confidence term's denominator offset: one bad trip moves a destination
/// about 8%, four move it about 17%, and nothing ever moves it enough to hide a
/// candidate. That bound is the point — see `BOUNDS`.
const CONFIDENCE_OFFSET: f64 = 2.0;
pub const BOUNDS: (f64, f64) = (0.75, 1.25);

/// The closed vocabulary. One of the two reasons the retrospective is a table
/// and not a JSON payload: a `CHECK` constraint can spell this and a payload
/// cannot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Again {
    Yes,
    No,
    Maybe,
}

impl Again {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Yes => "yes",
            Self::No => "no",
            Self::Maybe => "maybe",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "yes" => Some(Self::Yes),
            "no" => Some(Self::No),
            "maybe" => Some(Self::Maybe),
            _ => None,
        }
    }

    /// +1 / 0 / −1. The only place the words become numbers.
    pub fn score(self) -> f64 {
        match self {
            Self::Yes => 1.0,
            Self::Maybe => 0.0,
            Self::No => -1.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DestinationFactor {
    pub key: String,
    pub n: usize,
    pub mean_again: f64,
    pub factor: f64,
    /// The median overrun against `budget_cents`, in basis points, over the
    /// plans in this group that carry both a budget and a recorded cost. `null`
    /// when none do — never 0, which would read as "on budget".
    pub median_overrun_bp: Option<i64>,
    pub basis: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Summary {
    pub formula: &'static str,
    pub bounds: (f64, f64),
    pub contract: &'static str,
    pub by_destination: Vec<DestinationFactor>,
    pub by_companion: &'static str,
}

/// `1 + 0.25 · mean · n/(n+2)`, clamped to `BOUNDS`.
///
/// Cost overrun deliberately does not fold in. One number mixing "was it worth
/// it" with "was it over budget" cannot be explained, so the overrun rides
/// beside the factor instead.
pub fn factor(mean_again: f64, n: usize) -> f64 {
    let confidence = n as f64 / (n as f64 + CONFIDENCE_OFFSET);
    let raw = 1.0 + WEIGHT * mean_again * confidence;
    raw.clamp(BOUNDS.0, BOUNDS.1)
}

fn median(mut values: Vec<i64>) -> Option<i64> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    let middle = values.len() / 2;
    Some(if values.len() % 2 == 1 {
        values[middle]
    } else {
        (values[middle - 1] + values[middle]) / 2
    })
}

/// Group the recorded retrospectives by destination and publish one factor each.
///
/// A group with no rows is ABSENT rather than 1.0: "no evidence" and "evidence
/// that says neutral" are different answers and a consumer must be able to tell
/// them apart.
pub fn summary(rows: &[Retrospective], plans: &[TripPlan]) -> Summary {
    let mut groups: Vec<Group> = Vec::new();
    for row in rows {
        let Some(plan) = plans.iter().find(|plan| plan.id == row.plan_id) else {
            continue;
        };
        let Some(again) = Again::parse(&row.again) else {
            continue;
        };
        // Basis points against the intent, when both halves exist. A plan with
        // no budget contributes a score and no overrun, rather than a zero.
        let overrun = plan
            .budget_cents
            .filter(|budget| *budget > 0)
            .zip(row.cost_cents)
            .map(|(budget, cost)| (cost - budget) * 10_000 / budget);
        for destination in &plan.destinations {
            let key = normalize_place_name(&destination.name);
            if key.is_empty() {
                continue;
            }
            match groups.iter_mut().find(|(existing, ..)| *existing == key) {
                Some((_, scores, basis, overruns)) => {
                    scores.push(again.score());
                    if !basis.contains(&plan.id) {
                        basis.push(plan.id.clone());
                    }
                    overruns.extend(overrun);
                }
                None => groups.push((
                    key,
                    vec![again.score()],
                    vec![plan.id.clone()],
                    overrun.into_iter().collect(),
                )),
            }
        }
    }
    groups.sort_by(|a, b| a.0.cmp(&b.0));

    Summary {
        formula: FORMULA,
        bounds: BOUNDS,
        contract: CONTRACT,
        by_destination: groups
            .into_iter()
            .map(|(key, scores, basis, overruns)| {
                let n = scores.len();
                let mean_again = scores.iter().sum::<f64>() / n as f64;
                DestinationFactor {
                    key,
                    n,
                    mean_again,
                    factor: factor(mean_again, n),
                    median_overrun_bp: median(overruns),
                    basis,
                }
            })
            .collect(),
        by_companion: COMPANION_NOTE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{PlaceKind, PlaceRef};

    fn place(name: &str) -> PlaceRef {
        PlaceRef {
            id: format!("place:{name}"),
            name: name.into(),
            kind: PlaceKind::City,
            address: None,
            latitude: None,
            longitude: None,
        }
    }

    fn plan(id: &str, destination: &str, travelers: &[&str], budget: Option<i64>) -> TripPlan {
        TripPlan {
            id: id.into(),
            title: format!("Trip {id}"),
            origin: place("Origin"),
            destinations: vec![place(destination)],
            date_start: "2026-01-01".into(),
            date_end: "2026-01-07".into(),
            interests: String::new(),
            status: "saved".into(),
            travelers: travelers.iter().map(|t| (*t).to_string()).collect(),
            transport_modes: Vec::new(),
            stages: Vec::new(),
            cover_image_url: None,
            source: None,
            created_at: "0".into(),
            updated_at: "0".into(),
            budget_cents: budget,
            currency: budget.map(|_| "EUR".into()),
        }
    }

    fn retrospective(plan_id: &str, again: &str, cost: Option<i64>) -> Retrospective {
        Retrospective {
            plan_id: plan_id.into(),
            cost_cents: cost,
            currency: Some("EUR".into()),
            again: again.into(),
            change_note: String::new(),
            filled_at: "0".into(),
        }
    }

    #[test]
    fn the_summary_factor_is_bounded_and_moves_with_the_score() {
        // One "no": 1 + 0.25 * -1 * 1/3 = 0.916666…
        assert!((factor(-1.0, 1) - 0.916_666_666).abs() < 1e-6);
        // Four "no": 1 + 0.25 * -1 * 4/6 = 0.833333…
        assert!((factor(-1.0, 4) - 0.833_333_333).abs() < 1e-6);
        // All "yes" moves up and never past the ceiling.
        assert!(factor(1.0, 4) > 1.0);
        for n in [1_usize, 2, 5, 50, 5_000] {
            for mean in [-1.0, -0.5, 0.0, 0.5, 1.0] {
                let value = factor(mean, n);
                assert!(
                    value > BOUNDS.0 && value < BOUNDS.1,
                    "factor({mean}, {n}) = {value} left the bounds"
                );
            }
        }
    }

    #[test]
    fn a_destination_with_no_rows_is_absent_rather_than_neutral() {
        let plans = vec![
            plan("p1", "Lisbon", &[], None),
            plan("p2", "Porto", &[], None),
        ];
        let summary = summary(&[retrospective("p1", "no", None)], &plans);
        assert_eq!(summary.by_destination.len(), 1);
        assert_eq!(summary.by_destination[0].key, "lisbon");
        assert_eq!(summary.by_destination[0].n, 1);
        assert!(summary.by_destination[0].factor < 1.0);
        assert_eq!(summary.by_destination[0].median_overrun_bp, None);
    }

    #[test]
    fn an_overrun_is_basis_points_against_the_plans_own_budget() {
        let plans = vec![
            plan("p1", "Lisbon", &[], Some(100_000)),
            plan("p2", "Lisbon", &[], Some(100_000)),
        ];
        let rows = vec![
            retrospective("p1", "yes", Some(120_000)), // +20%
            retrospective("p2", "yes", Some(140_000)), // +40%
        ];
        let summary = summary(&rows, &plans);
        assert_eq!(summary.by_destination[0].median_overrun_bp, Some(3_000));
        assert_eq!(summary.by_destination[0].basis, vec!["p1", "p2"]);
    }

    /// The assertion the privacy ruling earns. Two plans whose `travelers` are
    /// set, and no name appears anywhere in the serialized body — the same
    /// falsifier shape as places ISA PLC-12.
    #[test]
    fn the_summary_response_carries_no_traveler_name() {
        let plans = vec![
            plan("p1", "Lisbon", &["Synthetic Companion"], None),
            plan("p2", "Porto", &["Second Synthetic Companion"], None),
        ];
        let rows = vec![
            retrospective("p1", "yes", None),
            retrospective("p2", "maybe", None),
        ];
        let body = serde_json::to_string(&summary(&rows, &plans)).unwrap();
        assert!(
            !body.contains("Synthetic"),
            "a traveler name reached the wire: {body}"
        );
        assert!(
            body.contains("not served"),
            "the omission is named at the wire"
        );
        assert!(body.contains("lisbon") && body.contains("porto"));
    }

    #[test]
    fn again_is_one_of_three_words() {
        assert_eq!(Again::parse("yes"), Some(Again::Yes));
        assert_eq!(Again::parse("no"), Some(Again::No));
        assert_eq!(Again::parse("maybe"), Some(Again::Maybe));
        assert_eq!(Again::parse("sure"), None);
        assert_eq!(Again::parse("Yes"), None, "the vocabulary is exact");
    }
}
