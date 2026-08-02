//! TELOS relevance for the general Feed.
//!
//! Comms owns this concern: it ranks any observation against explicitly
//! configured focus lenses without turning Feed into a Scouting view. Profiles
//! and items always pass through the same embedding backend in one batch. oMLX
//! is the preferred shared local server through its OpenAI-compatible endpoint;
//! Ollama remains a compatibility provider. When the selected endpoint is
//! unavailable, both sides use the same deterministic lexical vector space and
//! the stored mode says `lexical`.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::config::RelevanceConfig;
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

#[derive(Debug, Deserialize)]
struct OllamaEmbedResponse {
    embeddings: Vec<Vec<f64>>,
}

#[derive(Debug, Deserialize)]
struct OpenAiEmbedDatum {
    index: usize,
    embedding: Vec<f64>,
}

#[derive(Debug, Deserialize)]
struct OpenAiEmbedResponse {
    data: Vec<OpenAiEmbedDatum>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EmbeddingProvider {
    Ollama,
    OpenAi,
}

fn embedding_provider(config: &RelevanceConfig) -> Option<EmbeddingProvider> {
    match config.provider.trim().to_ascii_lowercase().as_str() {
        "ollama" => Some(EmbeddingProvider::Ollama),
        "openai" | "omlx" => Some(EmbeddingProvider::OpenAi),
        _ => None,
    }
}

pub fn embedding_provider_label(config: &RelevanceConfig) -> &'static str {
    match embedding_provider(config) {
        Some(EmbeddingProvider::OpenAi) => "OpenAI-compatible local endpoint",
        Some(EmbeddingProvider::Ollama) => "Ollama-compatible local endpoint",
        None => "Unknown embedding endpoint",
    }
}

pub fn embedding_backend_configured(config: &RelevanceConfig) -> bool {
    embedding_provider(config).is_some()
        && !config.base_url.trim().is_empty()
        && !config.model.trim().is_empty()
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
    config: &RelevanceConfig,
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
    let all_documents = profiles
        .iter()
        .map(|profile| prefixed_document(&config.query_prefix, &profile.text))
        .chain(
            item_documents
                .iter()
                .map(|document| prefixed_document(&config.document_prefix, document)),
        )
        .collect::<Vec<_>>();
    let lexical_documents = profile_documents
        .iter()
        .cloned()
        .chain(item_documents.iter().cloned())
        .collect::<Vec<_>>();
    let semantic = embed(&all_documents, config);
    let (vectors, mode) = match semantic {
        Some(vectors) if vectors.len() == all_documents.len() => (vectors, "semantic"),
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
pub fn embedding_backend_reachable(config: &RelevanceConfig) -> bool {
    if !embedding_backend_configured(config) {
        return false;
    }
    let provider = match embedding_provider(config) {
        Some(provider) => provider,
        None => return false,
    };
    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
    {
        Ok(client) => client,
        Err(_) => return false,
    };
    let endpoint = match provider {
        EmbeddingProvider::Ollama => {
            format!("{}/api/tags", config.base_url.trim_end_matches('/'))
        }
        EmbeddingProvider::OpenAi => {
            format!("{}/models", config.base_url.trim_end_matches('/'))
        }
    };
    let mut request = client.get(endpoint);
    if provider == EmbeddingProvider::OpenAi {
        if let Some(key) = crate::config::api_key_from_file(config.api_key_file.as_deref()) {
            request = request.bearer_auth(key);
        }
    }
    let response = match request.send() {
        Ok(response) if response.status().is_success() => response,
        _ => return false,
    };
    let body = match response.json::<serde_json::Value>() {
        Ok(body) => body,
        Err(_) => return false,
    };
    let models = match provider {
        EmbeddingProvider::Ollama => body.get("models").and_then(|models| models.as_array()),
        EmbeddingProvider::OpenAi => body.get("data").and_then(|models| models.as_array()),
    };
    models.is_some_and(|models| {
        models.iter().any(|model| {
            let installed = match provider {
                EmbeddingProvider::Ollama => model
                    .get("name")
                    .or_else(|| model.get("model"))
                    .and_then(|value| value.as_str())
                    .unwrap_or_default(),
                EmbeddingProvider::OpenAi => model
                    .get("id")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default(),
            };
            installed == config.model
                || installed
                    .strip_suffix(":latest")
                    .is_some_and(|name| name == config.model)
        })
    })
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

fn prefixed_document(prefix: &str, document: &str) -> String {
    format!("{}{document}", prefix.trim_start())
}

fn embedding_request(
    documents: &[String],
    config: &RelevanceConfig,
) -> Option<(EmbeddingProvider, String, serde_json::Value)> {
    let provider = embedding_provider(config)?;
    let endpoint = match provider {
        EmbeddingProvider::Ollama => {
            format!("{}/api/embed", config.base_url.trim_end_matches('/'))
        }
        EmbeddingProvider::OpenAi => {
            format!("{}/embeddings", config.base_url.trim_end_matches('/'))
        }
    };
    let mut payload = serde_json::json!({
        "model": config.model,
        "input": documents,
    });
    if provider == EmbeddingProvider::Ollama {
        // Ollama otherwise keeps the model resident for five minutes. Feed
        // enrichment is revision-cached and bursty, so immediate unload trades
        // a future cold start for returning unified memory to the desktop.
        payload["keep_alive"] = serde_json::json!(0);
    }
    Some((provider, endpoint, payload))
}

fn embed(documents: &[String], config: &RelevanceConfig) -> Option<Vec<Vec<f64>>> {
    if !embedding_backend_configured(config) {
        return None;
    }
    let (provider, endpoint, payload) = embedding_request(documents, config)?;
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(45))
        .build()
        .ok()?;
    let mut request = client.post(endpoint).json(&payload);
    if provider == EmbeddingProvider::OpenAi {
        if let Some(key) = crate::config::api_key_from_file(config.api_key_file.as_deref()) {
            request = request.bearer_auth(key);
        }
    }
    let response = request.send().ok()?;
    if !response.status().is_success() {
        return None;
    }
    match provider {
        EmbeddingProvider::Ollama => response
            .json::<OllamaEmbedResponse>()
            .ok()
            .map(|body| body.embeddings),
        EmbeddingProvider::OpenAi => {
            let mut body = response.json::<OpenAiEmbedResponse>().ok()?;
            body.data.sort_by_key(|datum| datum.index);
            Some(body.data.into_iter().map(|datum| datum.embedding).collect())
        }
    }
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
    fn semantic_roles_are_configurable() {
        assert_eq!(prefixed_document("query: ", "local AI"), "query: local AI");
        assert_eq!(prefixed_document("", "local AI"), "local AI");
    }

    #[test]
    fn provider_selection_is_explicit_and_backwards_compatible() {
        let ollama = RelevanceConfig::default();
        assert_eq!(embedding_provider(&ollama), Some(EmbeddingProvider::Ollama));

        let omlx = RelevanceConfig {
            provider: "openai".into(),
            base_url: "http://127.0.0.1:8000/v1".into(),
            model: "embedding-model".into(),
            query_prefix: "query: ".into(),
            document_prefix: "passage: ".into(),
            api_key_file: Some("~/.omlx/settings.json".into()),
            ..Default::default()
        };
        assert_eq!(embedding_provider(&omlx), Some(EmbeddingProvider::OpenAi));
        assert!(embedding_backend_configured(&omlx));

        let unknown = RelevanceConfig {
            provider: "guess".into(),
            ..Default::default()
        };
        assert_eq!(embedding_provider(&unknown), None);
        assert!(!embedding_backend_configured(&unknown));
    }

    #[test]
    fn provider_requests_use_the_right_endpoint_and_unload_policy() {
        let documents = vec!["one".into(), "two".into()];
        let (_, ollama_endpoint, ollama_payload) =
            embedding_request(&documents, &RelevanceConfig::default()).unwrap();
        assert_eq!(ollama_endpoint, "http://127.0.0.1:11434/api/embed");
        assert_eq!(ollama_payload["keep_alive"], 0);

        let omlx = RelevanceConfig {
            provider: "openai".into(),
            base_url: "http://127.0.0.1:8000/v1/".into(),
            model: "embedding-model".into(),
            ..Default::default()
        };
        let (_, omlx_endpoint, omlx_payload) = embedding_request(&documents, &omlx).unwrap();
        assert_eq!(omlx_endpoint, "http://127.0.0.1:8000/v1/embeddings");
        assert!(omlx_payload.get("keep_alive").is_none());
        assert_eq!(omlx_payload["input"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn openai_embeddings_are_restored_to_input_order() {
        let mut response: OpenAiEmbedResponse = serde_json::from_value(serde_json::json!({
            "data": [
                {"index": 1, "embedding": [0.0, 1.0]},
                {"index": 0, "embedding": [1.0, 0.0]}
            ]
        }))
        .unwrap();
        response.data.sort_by_key(|datum| datum.index);
        let vectors = response
            .data
            .into_iter()
            .map(|datum| datum.embedding)
            .collect::<Vec<_>>();
        assert_eq!(vectors, vec![vec![1.0, 0.0], vec![0.0, 1.0]]);
    }
}
