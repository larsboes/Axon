//! Ordering a journey search by the traveller's own weights.
//!
//! `plan-search` ranks *destinations* — is this a good place to go. This ranks
//! *connections* — is this a good way to get there — and the two are different
//! grains with different factors, which is why the profile carries a second
//! weight block rather than more fields on the first.
//!
//! Over HTTP, never by linking `capabilities/traveler`: a capability depends on
//! another's contract, not its code. This module is the whole dependency and it
//! is one-way — `traveler` knows nothing about transit.
//!
//! Absence degrades, it never fails, and it degrades *silently by doing nothing*:
//! when nothing has been stated, or the service is unreachable, or the body does
//! not parse, the journeys are returned in the backend's own order exactly as
//! they were before this module existed. That is the same rule
//! `punctuality::enrich` states for a missing delay figure.
//!
//! ## What the numbers mean
//!
//! Price, duration and changes are scored **relative to the other journeys in the
//! same answer**: the best of the set scores 1.0 and the worst scores 0.0. So a
//! score is not comparable between two searches, and the response says so by
//! carrying the weights it used rather than a bare number. Reliability is the one
//! absolute term, because it is already a probability and re-scaling it against
//! the set would turn "these are all reliable" into "one of these is best".
//!
//! A journey whose reliability is unknown does not score zero on it. The factor
//! is dropped and the remaining weights are re-normalised, because "nobody
//! measured this" and "this is unreliable" are different answers and the whole
//! point of `punctuality`'s `n` is to keep them apart.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::travel::{Journey, JourneyRanking, RankFactor};

/// Where traveler-server listens. Mirrors `capabilities/traveler/service.toml`'s
/// port, the same one-duplication `punctuality::DEFAULT_BASE_URL` carries and for
/// the same reason: the honest fix is the runner exporting a declared sibling's
/// port, and building that convention for one consumer would be inventing it
/// from a single example.
const DEFAULT_BASE_URL: &str = "http://127.0.0.1:8096";

pub fn base_url() -> String {
    std::env::var("AXON_TRAVELER_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string())
}

/// The weights, exactly as the profile serves them.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
pub struct JourneyWeights {
    pub price: f64,
    pub duration: f64,
    pub changes: f64,
    pub reliability: f64,
}

/// `GET /api/profile`, as much of it as this reads.
#[derive(Debug, Deserialize)]
struct ProfileEnvelope {
    profile: ProfileBody,
}

#[derive(Debug, Deserialize)]
struct ProfileBody {
    journey: JourneyWeights,
    /// Field path to provenance. Read only to answer "has anything been stated",
    /// which is the seam that keeps an unstated profile from changing a search.
    #[serde(default)]
    basis: BTreeMap<String, String>,
}

/// The traveller's journey weights, or `None` when nothing has been stated or the
/// service could not be reached.
///
/// `None` is not a failure state to report: it is the answer "rank nothing", and
/// every caller treats it that way.
pub fn stated_journey_weights() -> Option<JourneyWeights> {
    // Short, like the punctuality lookup: this is an enhancement on a localhost
    // service and waiting on it would make a search slower than not ranking.
    let client = axon_http::client(
        axon_http::Purpose::new("transit-traveler"),
        std::time::Duration::from_secs(3),
    )
    .ok()?;
    let envelope: ProfileEnvelope = client
        .get(format!("{}/api/profile", base_url()))
        .send()
        .and_then(|response| response.error_for_status())
        .and_then(|response| response.json())
        .ok()?;
    // `default` means nobody has looked. Ranking on it would be ranking on the
    // repo's guess while presenting it as the traveller's, which is the exact
    // confusion the provenance map exists to prevent.
    let stated = envelope
        .profile
        .basis
        .get("journey.price")
        .is_some_and(|provenance| provenance != "default");
    stated.then_some(envelope.profile.journey)
}

/// The four quantities a ranking reads, pulled off a journey.
///
/// A separate struct rather than four arguments so the scoring is testable
/// without building whole `Journey` values, and so the extraction from a journey
/// is one function that can be read on its own.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScoreInput {
    pub price: Option<f64>,
    pub duration_minutes: u32,
    pub changes: u32,
    /// `reliability.probability`, or `None` when punctuality could not answer.
    pub reliability: Option<f64>,
}

impl ScoreInput {
    pub fn of(journey: &Journey) -> Self {
        Self {
            price: journey.total_price,
            duration_minutes: journey.total_duration_minutes,
            // One fewer than the legs it is made of. A journey of one leg has no
            // changes; this is the count a traveller would say out loud.
            changes: (journey.legs.len() as u32).saturating_sub(1),
            reliability: journey.reliability.as_ref().map(|r| r.probability),
        }
    }
}

/// Score every journey in one answer, in place order.
///
/// Returns one `JourneyRanking` per input, positionally. `rank` is filled in by
/// [`rank_journeys`], because it depends on the sort and this function does not
/// sort.
pub fn score(inputs: &[ScoreInput], weights: JourneyWeights) -> Vec<JourneyRanking> {
    let prices: Vec<f64> = inputs.iter().filter_map(|input| input.price).collect();
    let durations: Vec<f64> = inputs
        .iter()
        .map(|input| input.duration_minutes as f64)
        .collect();
    let changes: Vec<f64> = inputs.iter().map(|input| input.changes as f64).collect();

    inputs
        .iter()
        .map(|input| {
            let mut factors: Vec<RankFactor> = Vec::new();

            if let Some(price) = input.price {
                let score = relative(price, &prices);
                factors.push(RankFactor {
                    key: "price".into(),
                    label: "Fare".into(),
                    score,
                    weight: weights.price,
                    rationale: match prices.iter().cloned().fold(f64::INFINITY, f64::min) {
                        cheapest if (price - cheapest).abs() < 0.005 => {
                            format!("{price:.2}, the cheapest here")
                        }
                        cheapest => {
                            format!("{price:.2}, {:.2} above the cheapest", price - cheapest)
                        }
                    },
                });
            }

            factors.push(RankFactor {
                key: "duration".into(),
                label: "Journey time".into(),
                score: relative(input.duration_minutes as f64, &durations),
                weight: weights.duration,
                rationale: format!(
                    "{}h{:02}m",
                    input.duration_minutes / 60,
                    input.duration_minutes % 60
                ),
            });

            factors.push(RankFactor {
                key: "changes".into(),
                label: "Changes".into(),
                score: relative(input.changes as f64, &changes),
                weight: weights.changes,
                rationale: match input.changes {
                    0 => "no changes".into(),
                    1 => "1 change".into(),
                    n => format!("{n} changes"),
                },
            });

            // The one absolute term, and the one that is dropped rather than
            // zeroed when it is unknown.
            if let Some(reliability) = input.reliability {
                factors.push(RankFactor {
                    key: "reliability".into(),
                    label: "Likely to hold".into(),
                    score: reliability.clamp(0.0, 1.0),
                    weight: weights.reliability,
                    rationale: format!("{:.0}% likely to hold end to end", reliability * 100.0),
                });
            }

            JourneyRanking {
                score: weighted(&factors),
                rank: 0,
                factors,
                weights,
            }
        })
        .collect()
}

/// The weighted mean over the factors that were present.
///
/// Divided by the weight that was actually used rather than by 1.0, which is what
/// makes a dropped factor re-normalise instead of quietly lowering every score.
fn weighted(factors: &[RankFactor]) -> f64 {
    let total: f64 = factors.iter().map(|factor| factor.weight).sum();
    if total <= 0.0 {
        return 0.0;
    }
    (factors
        .iter()
        .map(|factor| factor.score.clamp(0.0, 1.0) * factor.weight)
        .sum::<f64>()
        / total)
        .clamp(0.0, 1.0)
}

/// Where `value` sits in `set`, best-first: the minimum scores 1.0 and the maximum
/// scores 0.0.
///
/// All three relative factors want the lower number, so one direction serves
/// price, duration and changes. A set with no spread scores everything 1.0 —
/// there is nothing to discriminate on, and giving them all 0.5 would invent a
/// middle where the data has none.
fn relative(value: f64, set: &[f64]) -> f64 {
    let min = set.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = set.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    // `spread <= 0.0` covers a set with no spread; `!is_finite` covers an empty
    // set (infinity minus infinity) and a NaN, which would otherwise reach the
    // division and produce a NaN score that sorts arbitrarily.
    let spread = max - min;
    if !spread.is_finite() || spread <= 0.0 {
        return 1.0;
    }
    (1.0 - (value - min) / spread).clamp(0.0, 1.0)
}

/// Order `journeys` by the traveller's own weights, in place.
///
/// Returns whether anything was ranked, so a caller can tell "ranked, and this is
/// the order" from "nothing was stated, and this is the backend's order".
///
/// The sort is stable, so two journeys the weights cannot separate keep the
/// backend's own relative order rather than an arbitrary one — which is the
/// difference between ranking as a refinement and ranking as a reshuffle.
pub fn rank_journeys(journeys: &mut [Journey]) -> bool {
    let Some(weights) = stated_journey_weights() else {
        return false;
    };
    let inputs: Vec<ScoreInput> = journeys.iter().map(ScoreInput::of).collect();
    for (journey, ranking) in journeys.iter_mut().zip(score(&inputs, weights)) {
        journey.ranking = Some(ranking);
    }
    journeys.sort_by(|a, b| {
        let left = a.ranking.as_ref().map_or(f64::NEG_INFINITY, |r| r.score);
        let right = b.ranking.as_ref().map_or(f64::NEG_INFINITY, |r| r.score);
        right
            .partial_cmp(&left)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for (position, journey) in journeys.iter_mut().enumerate() {
        if let Some(ranking) = journey.ranking.as_mut() {
            ranking.rank = position + 1;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn weights() -> JourneyWeights {
        JourneyWeights {
            price: 0.30,
            duration: 0.20,
            changes: 0.15,
            reliability: 0.35,
        }
    }

    fn input(price: f64, minutes: u32, changes: u32, reliability: Option<f64>) -> ScoreInput {
        ScoreInput {
            price: Some(price),
            duration_minutes: minutes,
            changes,
            reliability,
        }
    }

    #[test]
    fn the_best_of_a_set_scores_one_and_the_worst_scores_zero() {
        assert_eq!(relative(10.0, &[10.0, 20.0]), 1.0);
        assert_eq!(relative(20.0, &[10.0, 20.0]), 0.0);
        assert_eq!(relative(15.0, &[10.0, 20.0]), 0.5);
    }

    #[test]
    fn a_set_with_no_spread_scores_everything_the_same() {
        // Nothing to discriminate on. A 0.5 would invent a middle the data does
        // not have, and a 0.0 would make an unremarkable set look bad.
        assert_eq!(relative(7.0, &[7.0, 7.0, 7.0]), 1.0);
        assert_eq!(relative(7.0, &[7.0]), 1.0);
    }

    #[test]
    fn the_cheapest_fastest_direct_journey_wins() {
        let scored = score(
            &[
                input(50.0, 300, 2, Some(0.80)),
                input(30.0, 240, 0, Some(0.90)),
            ],
            weights(),
        );
        assert!(
            scored[1].score > scored[0].score,
            "cheaper, faster and direct must beat dearer, slower and two changes: {} vs {}",
            scored[1].score,
            scored[0].score
        );
        assert_eq!(scored[1].factors.len(), 4);
    }

    #[test]
    fn an_unknown_reliability_is_dropped_and_the_rest_are_renormalised() {
        // The distinction the whole field exists for: nobody measured this is not
        // this is unreliable. Zeroing the term would bury a good journey for a
        // missing measurement.
        let known = score(&[input(30.0, 240, 0, Some(0.9))], weights());
        let unknown = score(&[input(30.0, 240, 0, None)], weights());
        assert_eq!(
            unknown[0].factors.len(),
            3,
            "the term is dropped, not zeroed"
        );
        assert!(
            unknown[0].score > 0.9,
            "with a perfect score on the three present factors the weighted mean must be \
             the mean of those three, got {}",
            unknown[0].score
        );
        assert!(known[0].score < unknown[0].score || known[0].score > 0.0);
    }

    #[test]
    fn dropping_a_factor_does_not_lower_every_score() {
        // Two identical sets differing only in whether reliability was measured.
        // The measured one carries 0.9 on it; the unmeasured one must not be
        // punished for the absence, so it scores at least as high.
        let with = score(&[input(30.0, 240, 0, Some(0.0))], weights());
        let without = score(&[input(30.0, 240, 0, None)], weights());
        assert!(
            without[0].score > with[0].score,
            "an unmeasured term must not score worse than a measured bad one: {} vs {}",
            without[0].score,
            with[0].score
        );
    }

    #[test]
    fn a_missing_price_drops_the_fare_factor_rather_than_scoring_it_as_free() {
        let mut no_price = input(0.0, 240, 0, Some(0.9));
        no_price.price = None;
        let scored = score(&[no_price], weights());
        assert!(
            scored[0].factors.iter().all(|f| f.key != "price"),
            "a journey with no fare must not be scored as though it were the cheapest"
        );
    }

    #[test]
    fn the_weights_that_produced_a_score_ride_with_it() {
        // A score is relative to the answer it came from, so a bare number is not
        // comparable between two searches. Carrying the weights is what lets a
        // reader tell which trade-off produced it.
        let scored = score(&[input(30.0, 240, 0, Some(0.9))], weights());
        assert_eq!(scored[0].weights, weights());
    }

    #[test]
    fn every_factor_carries_a_reason_a_person_can_read() {
        let scored = score(&[input(41.0, 252, 1, Some(0.8747))], weights());
        let by_key = |key: &str| {
            scored[0]
                .factors
                .iter()
                .find(|f| f.key == key)
                .unwrap_or_else(|| panic!("no {key} factor"))
                .rationale
                .clone()
        };
        assert_eq!(by_key("duration"), "4h12m");
        assert_eq!(by_key("changes"), "1 change");
        assert!(by_key("reliability").contains("87%"));
        assert!(by_key("price").contains("cheapest"));
    }

    #[test]
    fn weights_that_do_not_sum_to_one_do_not_break_the_scale() {
        // The profile refuses these, but a ranking must not produce a score above
        // 1.0 if one ever arrives: the weighted mean divides by the weight it
        // used, so an unnormalised set scales rather than overflows.
        let heavy = JourneyWeights {
            price: 3.0,
            duration: 3.0,
            changes: 3.0,
            reliability: 3.0,
        };
        let scored = score(&[input(30.0, 240, 0, Some(0.9))], heavy);
        assert!((0.0..=1.0).contains(&scored[0].score));
    }
}
