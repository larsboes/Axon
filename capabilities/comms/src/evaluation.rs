//! Deterministic, inspectable evaluation for Feed items.
//!
//! The model supplies summaries and TELOS embeddings; it does not invent the
//! final rank. This module turns stored facts into explicit factors with fixed
//! weights. Revisions make the result cacheable: unchanged content under the
//! same TELOS context and evaluator revision is never evaluated again.

use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::relevance::{InterestProfile, RelevanceMatch};
use crate::store::FeedItem;
use crate::travel::{self, TravelContext};

pub const EVALUATOR_REVISION: &str = "feed-evaluator-v2-travel";

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

pub fn context_revision(
    profiles: &[InterestProfile],
    embedding_producer: Option<&str>,
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
    revisions.push(format!("travel:{travel_revision}"));
    revisions.sort();
    revision_hash(&revisions.iter().map(String::as_str).collect::<Vec<_>>())
}

pub fn is_current(
    stored: Option<&FeedEvaluation>,
    item_revision: &str,
    context_revision: &str,
) -> bool {
    stored.is_some_and(|evaluation| {
        evaluation.item_revision == item_revision
            && evaluation.context_revision == context_revision
            && evaluation.evaluator_revision == EVALUATOR_REVISION
    })
}

pub fn evaluate(
    item: &FeedItem,
    strongest_match: Option<&RelevanceMatch>,
    context_revision: &str,
    travel_contexts: &[TravelContext],
) -> FeedEvaluation {
    let interest_score = strongest_match
        .map(|matched| matched.score.clamp(0.0, 1.0))
        .unwrap_or(0.0);
    let interest_rationale = strongest_match
        .map(|matched| {
            format!(
                "{} mit {:.0}% Übereinstimmung ({})",
                matched.profile_label,
                interest_score * 100.0,
                if matched.mode == "semantic" {
                    "semantisch"
                } else {
                    "lexikal"
                }
            )
        })
        .unwrap_or_else(|| "Keine konfigurierte TELOS-Linse verfügbar".into());

    let age = age_days(&item.day);
    let freshness_score = freshness_score(age);
    let freshness_rationale = match age {
        Some(0) => "Heute erfasst".to_string(),
        Some(1) => "Gestern erfasst".to_string(),
        Some(days) => format!("Vor {days} Tagen erfasst"),
        None => "Erfassungsdatum nicht auswertbar".to_string(),
    };

    let (evidence_score, evidence_rationale) = evidence_score(item);
    let travel_signal = travel::score_item(item, travel_contexts);
    let factors = vec![
        EvaluationFactor {
            key: "interest".into(),
            label: "Interessen-Fit".into(),
            score: interest_score,
            weight: 0.45,
            rationale: interest_rationale,
            context: None,
        },
        EvaluationFactor {
            key: "travel".into(),
            label: "Reisebezug".into(),
            score: travel_signal.score,
            weight: 0.25,
            rationale: travel_signal.rationale,
            context: travel_signal.context.map(|context| EvaluationFactorContext {
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
            label: "Aktualität".into(),
            score: freshness_score,
            weight: 0.20,
            rationale: freshness_rationale,
            context: None,
        },
        EvaluationFactor {
            key: "evidence".into(),
            label: "Inhaltsbasis".into(),
            score: evidence_score,
            weight: 0.10,
            rationale: evidence_rationale,
            context: None,
        },
    ];
    let overall_score = factors
        .iter()
        .map(|factor| factor.score * factor.weight)
        .sum::<f64>()
        .clamp(0.0, 1.0);
    let strongest = factors
        .iter()
        .max_by(|left, right| left.score.partial_cmp(&right.score).unwrap())
        .expect("the evaluator always has factors");
    let weakest = factors
        .iter()
        .min_by(|left, right| left.score.partial_cmp(&right.score).unwrap())
        .expect("the evaluator always has factors");
    let explanation = format!(
        "Stärkstes Signal: {} ({:.0}%). Größter Abschlag: {} ({:.0}%).",
        strongest.label,
        strongest.score * 100.0,
        weakest.label,
        weakest.score * 100.0
    );

    FeedEvaluation {
        feed_id: item.id.clone(),
        overall_score,
        explanation,
        mode: strongest_match
            .map(|matched| matched.mode.clone())
            .unwrap_or_else(|| "unscored".into()),
        item_revision: item_revision(item),
        context_revision: context_revision.to_string(),
        evaluator_revision: EVALUATOR_REVISION.into(),
        evaluated_at: String::new(),
        factors,
    }
}

fn evidence_score(item: &FeedItem) -> (f64, String) {
    let signals = [
        ("Titel", item.title.as_deref().is_some_and(non_empty), 0.20),
        ("Autor", item.author.as_deref().is_some_and(non_empty), 0.15),
        (
            "Kurzfassung",
            item.summary.as_deref().is_some_and(non_empty),
            0.30,
        ),
        (
            "Quelltext",
            item.transcript.as_deref().is_some_and(non_empty),
            0.35,
        ),
    ];
    let score = signals
        .iter()
        .filter(|(_, present, _)| *present)
        .map(|(_, _, weight)| weight)
        .sum::<f64>();
    let present = signals
        .iter()
        .filter(|(_, present, _)| *present)
        .map(|(label, _, _)| *label)
        .collect::<Vec<_>>();
    let missing = signals
        .iter()
        .filter(|(_, present, _)| !*present)
        .map(|(label, _, _)| *label)
        .collect::<Vec<_>>();
    let rationale = match (present.is_empty(), missing.is_empty()) {
        (_, true) => "Titel, Autor, Kurzfassung und Quelltext vorhanden".into(),
        (true, _) => format!("Noch keine auswertbaren Inhalte; fehlt: {}", missing.join(", ")),
        _ => format!(
            "Vorhanden: {}; fehlt: {}",
            present.join(", "),
            missing.join(", ")
        ),
    };
    (score, rationale)
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
    let now_days = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_secs() as i64
        / 86_400;
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
            context_revision(&profiles, Some("ollama:nomic-embed-text"), "travel"),
            context_revision(&profiles, Some("omlx:multilingual-embedding"), "travel")
        );
    }

    #[test]
    fn context_revision_includes_travel_snapshot() {
        assert_ne!(
            context_revision(&[], None, "travel-one"),
            context_revision(&[], None, "travel-two")
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
        let evaluation = evaluate(&item, Some(&matched), "context", &[]);
        assert_eq!(evaluation.factors.len(), 4);
        assert!((0.0..=1.0).contains(&evaluation.overall_score));
        assert!((evaluation.factors.iter().map(|factor| factor.weight).sum::<f64>() - 1.0).abs() < 1e-9);
        assert_eq!(evaluation.mode, "semantic");
    }

    #[test]
    fn civil_date_epoch_is_stable() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 1, 1), 10_957);
    }
}
