//! TELOS relevance for the general Feed.
//!
//! Comms owns this concern: it ranks any observation against explicitly
//! configured focus lenses without turning Feed into a Scouting view. Profiles
//! and items always pass through one resolved `embedding` role in one batch.
//! When that role is absent or unavailable, both sides use the same
//! deterministic lexical vector space and the stored mode says `lexical`.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::config::RelevanceConfig;
use crate::inference::{ResolvedRole, TextRole};
use crate::store::FeedItem;

// The selected multilingual E5 model accepts 512 tokens. A conservative
// character cap avoids sending and tokenizing long transcripts that the model
// would discard anyway, while leaving room for multilingual token variation.
const DOCUMENT_CAP: usize = 1_800;
const PROFILE_CAP: usize = 1_800;
const LEXICAL_DIMENSIONS: usize = 512;

#[derive(Debug, Clone)]
pub struct InterestProfile {
    pub key: String,
    pub label: String,
    pub focus: String,
    pub text: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone)]
pub struct RelevanceMatch {
    pub profile_key: String,
    pub profile_label: String,
    pub score: f64,
    pub rationale: String,
    pub mode: String,
    pub profile_revision: String,
}

#[derive(Debug, Clone)]
pub struct ScoredFeedItem {
    pub feed_id: String,
    pub matches: Vec<RelevanceMatch>,
}

pub fn embedding_provider_label(role: Option<&ResolvedRole>) -> &'static str {
    role.map(ResolvedRole::provider_label)
        .unwrap_or("No embedding role configured")
}

pub fn embedding_backend_configured(role: Option<&ResolvedRole>) -> bool {
    role.is_some_and(|role| {
        !role.backend.base_url.trim().is_empty() && !role.model.trim().is_empty()
    })
}

pub fn load_profiles(config: &RelevanceConfig) -> Vec<InterestProfile> {
    let mut files = Vec::new();
    for configured in &config.profile_paths {
        let path = PathBuf::from(configured);
        if path.is_file() {
            files.push(path);
        } else if path.is_dir() {
            let mut entries = fs::read_dir(&path)
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|entry| entry.extension().and_then(|ext| ext.to_str()) == Some("md"))
                .collect::<Vec<_>>();
            entries.sort();
            files.extend(entries);
        }
    }

    files
        .into_iter()
        .filter_map(|path| parse_profile(&path))
        .collect()
}

fn parse_profile(path: &Path) -> Option<InterestProfile> {
    let label = path.file_stem()?.to_str()?.trim().to_string();
    if label.eq_ignore_ascii_case("focus") || label.eq_ignore_ascii_case("readme") {
        return None;
    }
    let body = fs::read_to_string(path).ok()?;
    let summary = frontmatter_value(&body, "summary").unwrap_or_default();
    let current_focus = frontmatter_value(&body, "current_focus").unwrap_or_default();
    let affinity = frontmatter_value(&body, "category_affinity").unwrap_or_default();
    let focus = [summary.as_str(), current_focus.as_str(), affinity.as_str()]
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    let text = profile_embedding_text(&label, &summary, &current_focus, &affinity, &body);
    let fingerprint = sha256_hex(&text);
    Some(InterestProfile {
        key: sha256_hex(&path.to_string_lossy()),
        label,
        focus,
        text,
        fingerprint,
    })
}

fn profile_embedding_text(
    label: &str,
    summary: &str,
    current_focus: &str,
    affinity: &str,
    body: &str,
) -> String {
    let text = frontmatter_value(body, "relevance_query")
        .filter(|query| !query.trim().is_empty())
        .unwrap_or_else(|| format!("{label}\n{summary}\n{current_focus}\n{affinity}\n{body}"));
    cap_chars(&text, PROFILE_CAP)
}

fn frontmatter_value(body: &str, key: &str) -> Option<String> {
    let mut lines = body.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    for line in lines {
        let line = line.trim();
        if line == "---" {
            break;
        }
        if let Some(value) = line.strip_prefix(&format!("{key}:")) {
            return Some(value.trim().trim_matches('"').to_string());
        }
    }
    None
}

pub fn score_items(
    items: &[FeedItem],
    profiles: &[InterestProfile],
    role: Option<&ResolvedRole>,
) -> Vec<ScoredFeedItem> {
    if profiles.is_empty() {
        return items
            .iter()
            .map(|item| ScoredFeedItem {
                feed_id: item.id.clone(),
                matches: Vec::new(),
            })
            .collect();
    }

    let profile_documents = profiles
        .iter()
        .map(|profile| profile.text.clone())
        .collect::<Vec<_>>();
    let item_documents = items.iter().map(item_document).collect::<Vec<_>>();
    let semantic_inputs = profiles
        .iter()
        .map(|profile| (profile.text.clone(), TextRole::Query))
        .chain(
            item_documents
                .iter()
                .cloned()
                .map(|document| (document, TextRole::Document)),
        )
        .collect::<Vec<_>>();
    let lexical_documents = profile_documents
        .iter()
        .cloned()
        .chain(item_documents.iter().cloned())
        .collect::<Vec<_>>();
    let semantic = embed(&semantic_inputs, role);
    let (vectors, mode) = match semantic {
        Some(vectors) if vectors.len() == semantic_inputs.len() => (vectors, "semantic"),
        _ => (
            lexical_documents
                .iter()
                .map(|document| lexical_vector(document))
                .collect(),
            "lexical",
        ),
    };

    let profile_vectors = &vectors[..profiles.len()];
    let item_vectors = &vectors[profiles.len()..];
    items
        .iter()
        .zip(item_vectors)
        .map(|(item, item_vector)| {
            let mut matches = profiles
                .iter()
                .zip(profile_vectors)
                .map(|(profile, profile_vector)| {
                    let score = cosine(profile_vector, item_vector);
                    let method = if mode == "semantic" {
                        "Semantische Nähe"
                    } else {
                        "Lexikalische Nähe"
                    };
                    let rationale = if profile.focus.is_empty() {
                        format!("{method} zur TELOS-Linse {}", profile.label)
                    } else {
                        format!("{method} zu {} · {}", profile.label, profile.focus)
                    };
                    RelevanceMatch {
                        profile_key: profile.key.clone(),
                        profile_label: profile.label.clone(),
                        score,
                        rationale,
                        mode: mode.to_string(),
                        profile_revision: profile.fingerprint.clone(),
                    }
                })
                .collect::<Vec<_>>();
            matches.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            matches.truncate(3);
            ScoredFeedItem {
                feed_id: item.id.clone(),
                matches,
            }
        })
        .collect()
}

/// Cheap provider-specific readiness probe. Listing installed models avoids
/// running an embedding just to paint a status indicator in the dashboard.
pub fn embedding_backend_reachable(role: Option<&ResolvedRole>) -> bool {
    role.is_some_and(ResolvedRole::model_reachable)
}

fn item_document(item: &FeedItem) -> String {
    // Once a summary exists it is the distilled relevance input. Re-appending
    // the full transcript duplicates its information and wastes the small
    // embedding model's fixed context window.
    let content = item
        .summary
        .as_deref()
        .or(item.transcript.as_deref())
        .unwrap_or_default();
    cap_chars(
        &[
            item.title.as_deref().unwrap_or_default(),
            item.author.as_deref().unwrap_or_default(),
            content,
        ]
        .join("\n"),
        DOCUMENT_CAP,
    )
}

fn embed(
    inputs: &[(String, TextRole)],
    role: Option<&ResolvedRole>,
) -> Option<Vec<Vec<f64>>> {
    role?
        .embed_mixed(inputs)
        .ok()
        .map(|vectors| {
            vectors
                .into_iter()
                .map(|vector| vector.into_iter().map(f64::from).collect())
                .collect()
        })
}

fn lexical_vector(text: &str) -> Vec<f64> {
    let mut vector = vec![0.0; LEXICAL_DIMENSIONS];
    for token in tokens(text) {
        let mut hasher = DefaultHasher::new();
        token.hash(&mut hasher);
        let hash = hasher.finish();
        let index = (hash as usize) % LEXICAL_DIMENSIONS;
        let sign = if hash & 1 == 0 { 1.0 } else { -1.0 };
        vector[index] += sign;
    }
    normalize(vector)
}

fn tokens(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| token.chars().count() >= 3)
        .map(str::to_string)
        .collect()
}

fn normalize(mut vector: Vec<f64>) -> Vec<f64> {
    let norm = vector.iter().map(|value| value * value).sum::<f64>().sqrt();
    if norm > 0.0 {
        for value in &mut vector {
            *value /= norm;
        }
    }
    vector
}

fn cosine(a: &[f64], b: &[f64]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    a.iter().zip(b).map(|(left, right)| left * right).sum()
}

fn cap_chars(text: &str, cap: usize) -> String {
    text.chars().take(cap).collect()
}

fn sha256_hex(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_fields_are_read() {
        let body = "---\nsummary: \"Technical systems\"\ncurrent_focus: AI, RAG\n---\n# Lens";
        assert_eq!(
            frontmatter_value(body, "current_focus").as_deref(),
            Some("AI, RAG")
        );
    }

    #[test]
    fn explicit_relevance_query_excludes_note_scaffolding() {
        let body = "---\nsummary: Technical systems\ncurrent_focus: AI, RAG\nrelevance_query: LLM agents, retrieval, and software engineering\n---\n[[Private Link]]";
        assert_eq!(
            profile_embedding_text(
                "Polymath",
                "Technical systems",
                "AI, RAG",
                "conference",
                body
            ),
            "LLM agents, retrieval, and software engineering"
        );
    }

    #[test]
    fn lexical_fallback_uses_one_space_for_both_sides() {
        let profile = lexical_vector("AI systems architecture and open source");
        let close = lexical_vector("open source AI system architecture");
        let far = lexical_vector("cooking pottery relationship");
        assert!(cosine(&profile, &close) > cosine(&profile, &far));
    }

    #[test]
    fn item_text_is_bounded() {
        let mut item = FeedItem::new("https://example.com", "news", "article");
        item.transcript = Some("a".repeat(DOCUMENT_CAP + 100));
        assert_eq!(item_document(&item).chars().count(), DOCUMENT_CAP);
    }

    #[test]
    fn summary_replaces_transcript_for_embedding_input() {
        let mut item = FeedItem::new("https://example.com", "news", "article");
        item.summary = Some("distilled".into());
        item.transcript = Some("long raw transcript".into());
        let document = item_document(&item);
        assert!(document.contains("distilled"));
        assert!(!document.contains("long raw transcript"));
    }

    #[test]
    fn a_missing_embedding_role_is_an_explicit_lexical_fallback() {
        assert!(!embedding_backend_configured(None));
        assert_eq!(embedding_provider_label(None), "No embedding role configured");
    }
}
