//! Deterministic, inspectable evaluation for Feed items.
//!
//! The model supplies summaries and TELOS embeddings; it does not invent the
//! final rank. This module turns stored facts into explicit factors with fixed
//! weights. Revisions make the result cacheable: unchanged content under the
//! same TELOS context and evaluator revision is never evaluated again.

use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::media;
use crate::relevance::{InterestProfile, RelevanceMatch};
use crate::store::FeedItem;
use crate::travel::{self, TravelContext};

/// v5 grades content evidence by `content_status` instead of counting field
/// presence, so every stored evaluation restales and is recomputed.
pub const EVALUATOR_REVISION: &str = "feed-evaluator-v5-english";

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EvaluationFactorContext {
    pub kind: String,
    pub id: String,
    pub label: String,
    pub date_start: Option<String>,
    pub date_end: Option<String>,
    pub matched_terms: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EvaluationFactor {
    pub key: String,
    pub label: String,
    /// Normalized value in the closed interval 0..=1.
    pub score: f64,
    /// Share of the overall score. All factors for this revision sum to 1.
    pub weight: f64,
    pub rationale: String,
    pub context: Option<EvaluationFactorContext>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FeedEvaluation {
    pub feed_id: String,
    pub overall_score: f64,
    pub explanation: String,
    pub mode: String,
    pub item_revision: String,
    pub context_revision: String,
    pub evaluator_revision: String,
    pub evaluated_at: String,
    pub factors: Vec<EvaluationFactor>,
}

pub fn item_revision(item: &FeedItem) -> String {
    revision_hash(&[
        &item.stream,
        &item.kind,
        &item.url,
        item.title.as_deref().unwrap_or_default(),
        item.author.as_deref().unwrap_or_default(),
        item.summary.as_deref().unwrap_or_default(),
        item.transcript.as_deref().unwrap_or_default(),
    ])
}

/// What decides whether an item must be RE-EMBEDDED.
///
/// The half of the old `context_revision` that is genuinely about the vector
/// space: the lens texts and the two producers. Split out because
/// `relevance_refresh_handler` retains items by `is_current` and hands the
/// retained set straight to `score_items`, so every term in `context_revision`
/// was an embedding trigger — a changed travel snapshot, or a retrained
/// feedback model, would re-embed the whole window for a factor weighted 0.10.
///
/// The last completed full sweep records this in the `relevance-pass` receipt's
/// cursor, so a pass can tell "the vector space moved" from "the ranking inputs
/// moved" without a new column and without an ALTER.
pub fn relevance_revision(
    profiles: &[InterestProfile],
    embedding_producer: Option<&str>,
    reranking_producer: Option<&str>,
) -> String {
    let mut revisions = profiles
        .iter()
        .map(|profile| format!("{}:{}", profile.key, profile.fingerprint))
        .collect::<Vec<_>>();
    revisions.push(format!(
        "embedding:{}",
        embedding_producer.unwrap_or("lexical")
    ));
    revisions.push(format!(
        "reranking:{}",
        reranking_producer.unwrap_or("semantic")
    ));
    revisions.sort();
    revision_hash(&revisions.iter().map(String::as_str).collect::<Vec<_>>())
}

/// What decides whether an item must be RE-EVALUATED, from stored matches.
pub fn context_revision(
    profiles: &[InterestProfile],
    embedding_producer: Option<&str>,
    reranking_producer: Option<&str>,
    travel_revision: &str,
) -> String {
    let mut revisions = profiles
        .iter()
        .map(|profile| format!("{}:{}", profile.key, profile.fingerprint))
        .collect::<Vec<_>>();
    // A provider/model change changes the vector space even when the source
    // notes do not. Including it here makes the persisted ledger self-heal on
    // the next normal refresh instead of requiring an undocumented force run.
    revisions.push(format!(
        "embedding:{}",
        embedding_producer.unwrap_or("lexical")
    ));
    revisions.push(format!(
        "reranking:{}",
        reranking_producer.unwrap_or("semantic")
    ));
    revisions.push(format!("travel:{travel_revision}"));
    revisions.sort();
    revision_hash(&revisions.iter().map(String::as_str).collect::<Vec<_>>())
}

/// Whether a stored evaluation may be left alone.
///
/// `semantic_available` is the fourth condition, and it is not folded into a
/// revision on purpose: making the answering mode part of the hash would mint a
/// second revision per outcome and thrash between them. A row that was written
/// `lexical` while an embedding role is reachable is stale by definition — 525
/// such rows were all written in one pass on 2026-08-30 and have read as
/// current ever since. Passing `false` (no reachable role) keeps them current,
/// so the drain happens over ordinary passes with no force flag.
pub fn is_current(
    stored: Option<&FeedEvaluation>,
    item_revision: &str,
    context_revision: &str,
    semantic_available: bool,
) -> bool {
    stored.is_some_and(|evaluation| {
        let mode_is_final =
            !(semantic_available && matches!(evaluation.mode.as_str(), "lexical" | "unscored"));
        mode_is_final
            && evaluation.item_revision == item_revision
            && evaluation.context_revision == context_revision
            && evaluation.evaluator_revision == EVALUATOR_REVISION
    })
}

/// The share the learned feedback factor takes when it is active.
///
/// Taste, not measurement, which is why it is a named constant reported by
/// `GET /feed/evaluation/status`: 15 points of 100 re-orders inside a band and
/// can never outvote the 45-point TELOS factor.
pub const FEEDBACK_WEIGHT: f64 = 0.15;

/// One factor's contribution before the weights are normalised.
///
/// `weight` is the factor's share when it can be computed and `0.0` when it
/// cannot. The rule that makes both states legal: **a factor that cannot be
/// computed carries weight 0 and the remaining factors scale so the sum stays
/// 1.0.** That is what lets a class refusal, or an inert learned factor, leave
/// the arithmetic whole instead of dumping the item to the bottom of its band.
fn normalise_weights(factors: &mut [EvaluationFactor]) {
    let total = factors.iter().map(|factor| factor.weight).sum::<f64>();
    if total <= 0.0 {
        return;
    }
    for factor in factors.iter_mut() {
        factor.weight /= total;
    }
}

pub fn evaluate(
    item: &FeedItem,
    strongest_match: Option<&RelevanceMatch>,
    context_revision: &str,
    travel_contexts: &[TravelContext],
    refused_class: bool,
) -> FeedEvaluation {
    let interest_score = strongest_match
        .map(|matched| matched.score.clamp(0.0, 1.0))
        .unwrap_or(0.0);
    let interest_rationale = if refused_class {
        // Stored as a row rather than as an absence. An item with no
        // evaluation is indistinguishable from one nobody has got to yet; a
        // refusal says who refused and why, and it survives the next pass.
        "Not scored: c3 is never read by a model".to_string()
    } else {
        strongest_match
            .map(|matched| {
                format!(
                    "{} with {:.0}% alignment ({})",
                    matched.profile_label,
                    interest_score * 100.0,
                    match matched.mode.as_str() {
                        "reranked" => "reranked",
                        "semantic" => "semantic",
                        _ => "lexical",
                    }
                )
            })
            .unwrap_or_else(|| "No configured TELOS lens is available".into())
    };

    let age = age_days(&item.day);
    let freshness_score = freshness_score(age);
    let freshness_rationale = match age {
        Some(0) => "Captured today".to_string(),
        Some(1) => "Captured yesterday".to_string(),
        Some(days) => format!("Captured {days} days ago"),
        None => "Capture date cannot be evaluated".to_string(),
    };

    let (evidence_score, evidence_rationale) = evidence_score(item);
    let travel_signal = travel::score_item(item, travel_contexts);
    let mut factors = vec![
        EvaluationFactor {
            key: "interest".into(),
            label: "Interest fit".into(),
            score: if refused_class { 0.0 } else { interest_score },
            weight: if refused_class { 0.0 } else { 0.45 },
            rationale: interest_rationale,
            context: None,
        },
        EvaluationFactor {
            key: "travel".into(),
            label: "Travel relevance".into(),
            score: travel_signal.score,
            weight: 0.25,
            rationale: travel_signal.rationale,
            context: travel_signal
                .context
                .map(|context| EvaluationFactorContext {
                    kind: "trip".into(),
                    id: context.id,
                    label: context.label,
                    date_start: Some(context.date_start),
                    date_end: Some(context.date_end),
                    matched_terms: context.matched_terms,
                }),
        },
        EvaluationFactor {
            key: "freshness".into(),
            label: "Freshness".into(),
            score: freshness_score,
            weight: 0.20,
            rationale: freshness_rationale,
            context: None,
        },
        EvaluationFactor {
            key: "evidence".into(),
            label: "Content evidence".into(),
            score: evidence_score,
            weight: 0.10,
            rationale: evidence_rationale,
            context: None,
        },
    ];
    normalise_weights(&mut factors);
    let overall_score = factors
        .iter()
        .map(|factor| factor.score * factor.weight)
        .sum::<f64>()
        .clamp(0.0, 1.0);
    // Zero-weight factors are skipped: a refused interest factor scoring 0.0
    // counts for nothing in the score, so reporting it as the largest deduction
    // would be the explanation contradicting the arithmetic.
    let counted = factors
        .iter()
        .filter(|factor| factor.weight > 0.0)
        .collect::<Vec<_>>();
    let strongest = counted
        .iter()
        .max_by(|left, right| left.score.partial_cmp(&right.score).unwrap())
        .expect("the evaluator always has at least one weighted factor");
    let weakest = counted
        .iter()
        .min_by(|left, right| left.score.partial_cmp(&right.score).unwrap())
        .expect("the evaluator always has at least one weighted factor");
    let explanation = format!(
        "Strongest signal: {} ({:.0}%). Largest deduction: {} ({:.0}%).",
        strongest.label,
        strongest.score * 100.0,
        weakest.label,
        weakest.score * 100.0
    );

    FeedEvaluation {
        feed_id: item.id.clone(),
        overall_score,
        explanation,
        mode: match strongest_match {
            // A refusal is `unscored` whatever a stale match once said.
            Some(matched) if !refused_class => matched.mode.clone(),
            _ => "unscored".into(),
        },
        item_revision: item_revision(item),
        context_revision: context_revision.to_string(),
        evaluator_revision: EVALUATOR_REVISION.into(),
        evaluated_at: String::new(),
        factors,
    }
}

/// Evidence is graded, not counted.
///
/// Presence alone scored a stored consent wall exactly like a stored paper,
/// because both put a non-empty string in `transcript`. The pipeline already
/// classifies the difference — `content_status` is derived from the
/// *normalized* body — so the source-text signal takes the share its status
/// earns. The other three stay binary: a title either exists or it does not.
fn evidence_score(item: &FeedItem) -> (f64, String) {
    let (text_earned, text_label) = source_text_evidence(item);
    let signals = [
        (
            "title",
            binary(item.title.as_deref().is_some_and(non_empty)),
            0.20,
        ),
        (
            "author",
            binary(item.author.as_deref().is_some_and(non_empty)),
            0.15,
        ),
        (
            "summary",
            binary(item.summary.as_deref().is_some_and(non_empty)),
            0.30,
        ),
        (text_label, text_earned, 0.35),
    ];
    let score = signals
        .iter()
        .map(|(_, earned, weight)| earned * weight)
        .sum::<f64>();
    let present = signals
        .iter()
        .filter(|(_, earned, _)| *earned > 0.0)
        .map(|(label, _, _)| *label)
        .collect::<Vec<_>>();
    let missing = signals
        .iter()
        .filter(|(_, earned, _)| *earned == 0.0)
        .map(|(label, _, _)| *label)
        .collect::<Vec<_>>();
    let rationale = match (present.is_empty(), missing.is_empty()) {
        // `present` carries the graded label, so this arm cannot claim full
        // source text when the item only stored a card.
        (_, true) => format!("Available: {}", present.join(", ")),
        (true, _) => format!("No usable content yet; missing: {}", missing.join(", ")),
        _ => format!(
            "Available: {}; missing: {}",
            present.join(", "),
            missing.join(", ")
        ),
    };
    (score, rationale)
}

/// What the stored body is worth as a share of its weight, and how to name it
/// in the rationale. `unknown` is the legacy rows written before extraction
/// classified itself: grade those by the same threshold the classifier uses
/// rather than assuming the best case for them.
fn source_text_evidence(item: &FeedItem) -> (f64, &'static str) {
    let Some(text) = item.transcript.as_deref().filter(|t| non_empty(t)) else {
        return (0.0, "source text");
    };
    match item.content_status.as_str() {
        "full" => (1.0, "source text"),
        "thin" => (THIN_TEXT_SHARE, "thin source text"),
        "none" => (0.0, "source text"),
        _ if text.chars().count() >= media::CONTENT_FULL_THRESHOLD => (1.0, "source text"),
        _ => (THIN_TEXT_SHARE, "thin source text"),
    }
}

/// A card, an abstract or a page that normalized down to a stub is real
/// evidence, just not the article. It keeps well under half its weight so a
/// full body always outranks one on this factor.
const THIN_TEXT_SHARE: f64 = 0.4;

fn binary(present: bool) -> f64 {
    if present {
        1.0
    } else {
        0.0
    }
}

fn non_empty(value: &str) -> bool {
    !value.trim().is_empty()
}

fn freshness_score(age: Option<i64>) -> f64 {
    match age {
        None => 0.0,
        Some(days) if days <= 0 => 1.0,
        Some(days) if days <= 7 => interpolate(days, 0, 7, 1.0, 0.90),
        Some(days) if days <= 30 => interpolate(days, 7, 30, 0.90, 0.65),
        Some(days) if days <= 90 => interpolate(days, 30, 90, 0.65, 0.35),
        Some(days) if days <= 365 => interpolate(days, 90, 365, 0.35, 0.10),
        Some(_) => 0.05,
    }
}

fn interpolate(value: i64, start: i64, end: i64, high: f64, low: f64) -> f64 {
    let progress = (value - start) as f64 / (end - start) as f64;
    high + (low - high) * progress
}

fn age_days(day: &str) -> Option<i64> {
    let mut parts = day.split('-');
    let year = parts.next()?.parse::<i64>().ok()?;
    let month = parts.next()?.parse::<i64>().ok()?;
    let date = parts.next()?.parse::<i64>().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&date) {
        return None;
    }
    let item_days = days_from_civil(year, month, date);
    let now_days = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs() as i64 / 86_400;
    Some((now_days - item_days).max(0))
}

/// Gregorian civil date to days since 1970-01-01 (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let adjusted_year = year - i64::from(month <= 2);
    let era = if adjusted_year >= 0 {
        adjusted_year
    } else {
        adjusted_year - 399
    } / 400;
    let year_of_era = adjusted_year - era * 400;
    let adjusted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * adjusted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn revision_hash(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item() -> FeedItem {
        let mut item = FeedItem::new("https://example.com/item", "news", "article");
        item.title = Some("Local-first systems".into());
        item.summary = Some("A bounded summary".into());
        item.transcript = Some("Full source".into());
        item.day = "2026-07-29".into();
        item
    }

    #[test]
    fn revisions_change_only_with_relevant_inputs() {
        let first = item();
        let mut second = first.clone();
        second.status = "keeper".into();
        assert_eq!(item_revision(&first), item_revision(&second));
        second.summary = Some("Changed summary".into());
        assert_ne!(item_revision(&first), item_revision(&second));
    }

    #[test]
    fn context_revision_includes_embedding_vector_space() {
        let profiles = Vec::new();
        assert_ne!(
            context_revision(&profiles, Some("ollama:nomic-embed-text"), None, "travel"),
            context_revision(
                &profiles,
                Some("omlx:multilingual-embedding"),
                None,
                "travel"
            )
        );
    }

    #[test]
    fn context_revision_includes_travel_snapshot() {
        assert_ne!(
            context_revision(&[], None, None, "travel-one"),
            context_revision(&[], None, None, "travel-two")
        );
    }

    #[test]
    fn context_revision_includes_reranking_model() {
        assert_ne!(
            context_revision(&[], Some("omlx:e5"), Some("omlx:reranker-a"), "travel"),
            context_revision(&[], Some("omlx:e5"), Some("omlx:reranker-b"), "travel")
        );
    }

    #[test]
    fn score_is_weighted_and_bounded() {
        let item = item();
        let matched = RelevanceMatch {
            profile_key: "p".into(),
            profile_label: "Local AI".into(),
            score: 0.8,
            rationale: "match".into(),
            mode: "semantic".into(),
            profile_revision: "r".into(),
        };
        let evaluation = evaluate(&item, Some(&matched), "context", &[], false);
        assert_eq!(evaluation.factors.len(), 4);
        assert!((0.0..=1.0).contains(&evaluation.overall_score));
        assert!(
            (evaluation
                .factors
                .iter()
                .map(|factor| factor.weight)
                .sum::<f64>()
                - 1.0)
                .abs()
                < 1e-9
        );
        assert_eq!(evaluation.mode, "semantic");
        assert_eq!(
            evaluation
                .factors
                .iter()
                .map(|factor| factor.label.as_str())
                .collect::<Vec<_>>(),
            vec![
                "Interest fit",
                "Travel relevance",
                "Freshness",
                "Content evidence"
            ]
        );
        assert!(evaluation.explanation.starts_with("Strongest signal:"));
    }

    #[test]
    fn content_evidence_grades_the_body_it_actually_stored() {
        let mut full = item();
        full.transcript = Some("a".repeat(media::CONTENT_FULL_THRESHOLD));
        full.content_status = "full".into();

        // Same fields populated, same field count -- a consent wall that
        // normalized down to a stub. Counting presence scored these alike.
        let mut thin = full.clone();
        thin.transcript = Some("Accept all cookies".into());
        thin.content_status = "thin".into();

        let mut none = full.clone();
        none.transcript = None;
        none.content_status = "none".into();

        let score = |item: &FeedItem| evidence_score(item).0;
        assert!(
            score(&full) > score(&thin),
            "a full body must outrank a stub: {} vs {}",
            score(&full),
            score(&thin)
        );
        assert!(
            score(&thin) > score(&none),
            "a stub is still more than nothing"
        );

        assert!(
            evidence_score(&thin).1.contains("thin source text"),
            "the rationale must name what it graded: {}",
            evidence_score(&thin).1
        );

        // A legacy row predating classification is graded by the same
        // threshold the classifier uses, not assumed to be a full body.
        let mut legacy = thin.clone();
        legacy.content_status = "unknown".into();
        assert_eq!(score(&legacy), score(&thin));
    }

    fn stored(mode: &str) -> FeedEvaluation {
        FeedEvaluation {
            feed_id: "id".into(),
            overall_score: 0.5,
            explanation: String::new(),
            mode: mode.into(),
            item_revision: "item".into(),
            context_revision: "context".into(),
            evaluator_revision: EVALUATOR_REVISION.into(),
            evaluated_at: String::new(),
            factors: Vec::new(),
        }
    }

    #[test]
    fn a_lexical_row_is_stale_while_the_embedding_role_answers() {
        // Every revision matches. The only thing that differs is whether an
        // embedding role is answering right now -- which is what drains the 525
        // rows written lexical in one pass, with no force flag and no endpoint.
        let lexical = stored("lexical");
        assert!(!is_current(Some(&lexical), "item", "context", true));
        assert!(is_current(Some(&lexical), "item", "context", false));

        let unscored = stored("unscored");
        assert!(!is_current(Some(&unscored), "item", "context", true));

        let semantic = stored("semantic");
        assert!(is_current(Some(&semantic), "item", "context", true));
        assert!(!is_current(Some(&semantic), "moved", "context", true));
    }

    #[test]
    fn relevance_revision_ignores_the_travel_snapshot() {
        // The whole point of the split: a trip that starts or ends must not
        // re-embed 372 items for a factor weighted 0.10.
        assert_eq!(
            relevance_revision(&[], Some("ollama:bge-m3"), None),
            relevance_revision(&[], Some("ollama:bge-m3"), None)
        );
        assert_ne!(
            context_revision(&[], Some("ollama:bge-m3"), None, "trip-one"),
            context_revision(&[], Some("ollama:bge-m3"), None, "trip-two")
        );
        assert_ne!(
            relevance_revision(&[], Some("ollama:bge-m3"), None),
            relevance_revision(&[], Some("omlx:e5"), None)
        );
    }

    #[test]
    fn a_refused_factor_carries_weight_zero_and_the_rest_rescale() {
        let item = item();
        let matched = RelevanceMatch {
            profile_key: "p".into(),
            profile_label: "Local AI".into(),
            score: 0.8,
            rationale: "match".into(),
            mode: "semantic".into(),
            profile_revision: "r".into(),
        };
        let refused = evaluate(&item, Some(&matched), "context", &[], true);
        let interest = refused
            .factors
            .iter()
            .find(|factor| factor.key == "interest")
            .expect("the refusal is a row, not an absence");
        assert_eq!(interest.weight, 0.0);
        assert!(interest.rationale.contains("never read by a model"));
        assert_eq!(refused.mode, "unscored");
        assert!(
            (refused
                .factors
                .iter()
                .map(|factor| factor.weight)
                .sum::<f64>()
                - 1.0)
                .abs()
                < 1e-9,
            "the weights still sum to one with a factor at zero"
        );
        // The explanation must not report a factor that counts for nothing.
        assert!(!refused.explanation.contains("Interest fit"));
    }

    #[test]
    fn civil_date_epoch_is_stable() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 1, 1), 10_957);
    }
}
