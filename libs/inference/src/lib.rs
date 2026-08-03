//! One home for "which model answers this, on this machine".
//!
//! Before this existed there were four shapes for the same fact: comms'
//! `SummarizerConfig` and `RelevanceConfig`, scouting's `EmbedConfig`,
//! `libs/ai-client`'s `RouterConfig`, plus `tools/graphify.sh`'s env var. Each
//! knew a base URL and a model name, and each had to be edited by hand to move
//! a machine between runtimes. `systems.toml` already stated the rule this
//! module implements, in the oMLX entry: *"Referenced by id, not by URL,
//! because host/port/model differ per machine."*
//!
//! ## The shape
//!
//! Two levels, and callers only ever touch the second.
//!
//! * A **backend** is a server: an API shape, a base URL, optionally a file to
//!   read a bearer key out of. Declared once.
//! * A **role** is a job: `embedding`, `summarization`. It names a backend, the
//!   model on it, and that model's input conventions.
//!
//! A capability asks for a role. It never learns whether it just talked to
//! oMLX or Ollama, which is the whole point: oMLX needs Metal and cannot exist
//! on the family Pi, Ollama runs anywhere, and moving between them must be a
//! config edit rather than a code change.
//!
//! ## Two things portability actually requires
//!
//! **Models are not interchangeable.** `multilingual-e5-*` wants `query: ` and
//! `passage: ` role prefixes; `nomic-embed-text` wants `search_query: ` and
//! `search_document: `. Sending the wrong ones costs retrieval quality and
//! raises no error at all, so the prefixes belong to the role, beside the
//! model, and travel with it.
//!
//! **Cached vectors belong to the model that produced them.** A cache keyed on
//! the profile alone silently serves e5 vectors to a nomic run after a backend
//! switch — every score wrong, nothing logged. [`ResolvedRole::cache_key`]
//! exists so a cache can name its producer, and consumers are expected to use
//! it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The wire shape a backend speaks. Both are embedding-capable; the request
/// and response bodies differ, nothing else does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Api {
    /// `POST {base_url}/embeddings`, bearer auth, `data[].embedding` back.
    /// oMLX, LM Studio, Ollama's own `/v1` shim, and every hosted provider.
    OpenAi,
    /// `POST {base_url}/api/embed`, no auth, `embeddings` back. Ollama native.
    Ollama,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Backend {
    pub api: Api,
    /// For `OpenAi` this includes the version segment, e.g.
    /// `http://127.0.0.1:8000/v1`. For `Ollama` it is the bare origin.
    pub base_url: String,
    /// A file to read a bearer key from. If it parses as JSON, `.auth.api_key`
    /// is used, which is what lets this point straight at
    /// `~/.omlx/settings.json`; otherwise the trimmed contents are the key.
    /// Never a key value inline — secrets are references in this repo.
    #[serde(default)]
    pub api_key_file: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Role {
    pub backend: String,
    pub model: String,
    /// Prefixed onto the *query* side of a retrieval pair. Empty for models
    /// that take none.
    #[serde(default)]
    pub query_prefix: String,
    /// Prefixed onto the *document* side.
    #[serde(default)]
    pub document_prefix: String,
}

/// Which side of a retrieval pair a text is, so the right prefix goes on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextRole {
    Query,
    Document,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InferenceConfig {
    #[serde(default)]
    pub backends: HashMap<String, Backend>,
    #[serde(default)]
    pub roles: HashMap<String, Role>,
}

/// A role with its backend already looked up. This is what callers hold.
#[derive(Debug, Clone)]
pub struct ResolvedRole {
    pub backend_name: String,
    pub backend: Backend,
    pub model: String,
    pub query_prefix: String,
    pub document_prefix: String,
}

/// Overrides the backend for every role, for one machine.
///
/// `service-runner.sh` exports this from `machine.toml`'s `[inference] backend`,
/// the same path `[capability.<name>] port` already takes to reach a process.
/// A machine that cannot run the configured backend says so once, in the file
/// that already holds machine-local facts, and no capability config changes.
pub const BACKEND_OVERRIDE_ENV: &str = "AXON_INFERENCE_BACKEND";

/// Points at the config file directly. Mainly for tests and one-off runs.
pub const CONFIG_PATH_ENV: &str = "AXON_INFERENCE_CONFIG";

impl InferenceConfig {
    /// Reads the overlay's `inference.json`, or returns an empty config when
    /// there is none. Absent config is not an error: every consumer is
    /// expected to degrade to something that still works offline (scouting
    /// falls back to hash embedding), so a machine with no inference set up at
    /// all keeps running rather than failing at startup.
    pub fn load(overlay_config: impl Fn(&str) -> Option<PathBuf>) -> Self {
        let path = match std::env::var(CONFIG_PATH_ENV) {
            Ok(explicit) if !explicit.trim().is_empty() => PathBuf::from(explicit),
            _ => match overlay_config("inference.json") {
                Some(path) => path,
                None => return Self::default(),
            },
        };
        Self::from_path(&path)
    }

    pub fn from_path(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::from_str(&text).unwrap_or_else(|error| {
                eprintln!(
                    "  inference: {} is not readable as config ({error}) — continuing without it",
                    path.display()
                );
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn from_str(text: &str) -> Result<Self, String> {
        serde_json::from_str(text).map_err(|error| error.to_string())
    }

    /// Looks a role up and resolves its backend, applying the machine
    /// override. `None` means this machine has no way to do that job, which is
    /// a normal state a caller degrades from, not a crash.
    pub fn role(&self, name: &str) -> Option<ResolvedRole> {
        let role = self.roles.get(name)?;
        let backend_name = match std::env::var(BACKEND_OVERRIDE_ENV) {
            Ok(over) if !over.trim().is_empty() => over.trim().to_string(),
            _ => role.backend.clone(),
        };
        let backend = self.backends.get(&backend_name).cloned().or_else(|| {
            eprintln!(
                "  inference: role '{name}' wants backend '{backend_name}', which is not declared"
            );
            None
        })?;
        Some(ResolvedRole {
            backend_name,
            backend,
            model: role.model.clone(),
            query_prefix: role.query_prefix.clone(),
            document_prefix: role.document_prefix.clone(),
        })
    }
}

impl ResolvedRole {
    /// Identifies the thing that produced a vector, so a cache can refuse to
    /// serve it to a different one. Cheap to store beside cached data and the
    /// only defence against a backend switch silently reusing incompatible
    /// vectors.
    pub fn cache_key(&self) -> String {
        format!("{}:{}", self.backend_name, self.model)
    }

    fn embedding_endpoint(&self) -> String {
        let base = self.backend.base_url.trim_end_matches('/');
        match self.backend.api {
            Api::OpenAi => format!("{base}/embeddings"),
            Api::Ollama => format!("{base}/api/embed"),
        }
    }

    /// OpenAI-compatible chat-completions endpoint for generative roles.
    /// Ollama exposes the same wire shape under its `/v1` compatibility path.
    pub fn chat_completions_endpoint(&self) -> String {
        let base = self.backend.base_url.trim_end_matches('/');
        match self.backend.api {
            Api::OpenAi => format!("{base}/chat/completions"),
            Api::Ollama => format!("{base}/v1/chat/completions"),
        }
    }

    pub fn provider_label(&self) -> &'static str {
        match self.backend.api {
            Api::OpenAi => "OpenAI-compatible local endpoint",
            Api::Ollama => "Ollama-compatible local endpoint",
        }
    }

    pub fn bearer_key(&self) -> Option<String> {
        api_key_from_file(self.backend.api_key_file.as_deref())
    }

    /// Cheap readiness probe that confirms the role's selected model is listed
    /// without loading it or running inference.
    pub fn model_reachable(&self) -> bool {
        if self.backend.base_url.trim().is_empty() || self.model.trim().is_empty() {
            return false;
        }
        let client = match reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(3))
            .build()
        {
            Ok(client) => client,
            Err(_) => return false,
        };
        let base = self.backend.base_url.trim_end_matches('/');
        let endpoint = match self.backend.api {
            Api::OpenAi => format!("{base}/models"),
            Api::Ollama => format!("{base}/api/tags"),
        };
        let mut request = client.get(endpoint);
        if let Some(key) = self.bearer_key() {
            request = request.bearer_auth(key);
        }
        let body = match request.send() {
            Ok(response) if response.status().is_success() => {
                match response.json::<serde_json::Value>() {
                    Ok(body) => body,
                    Err(_) => return false,
                }
            }
            _ => return false,
        };
        let models = match self.backend.api {
            Api::OpenAi => body.get("data").and_then(|models| models.as_array()),
            Api::Ollama => body.get("models").and_then(|models| models.as_array()),
        };
        models.is_some_and(|models| {
            models.iter().any(|model| {
                let installed = match self.backend.api {
                    Api::OpenAi => model.get("id"),
                    Api::Ollama => model.get("name").or_else(|| model.get("model")),
                }
                .and_then(|value| value.as_str())
                .unwrap_or_default();
                installed == self.model
                    || installed
                        .strip_suffix(":latest")
                        .is_some_and(|name| name == self.model)
            })
        })
    }

    fn prefix(&self, text_role: TextRole) -> &str {
        match text_role {
            TextRole::Query => &self.query_prefix,
            TextRole::Document => &self.document_prefix,
        }
    }

    pub fn request_body(&self, texts: &[String], text_role: TextRole) -> serde_json::Value {
        let inputs = texts
            .iter()
            .cloned()
            .map(|text| (text, text_role))
            .collect::<Vec<_>>();
        self.request_body_mixed(&inputs)
    }

    pub fn request_body_mixed(&self, inputs: &[(String, TextRole)]) -> serde_json::Value {
        let input: Vec<String> = inputs
            .iter()
            .map(|(text, role)| format!("{}{text}", self.prefix(*role)))
            .collect();
        let mut payload = serde_json::json!({ "model": self.model, "input": input });
        if self.backend.api == Api::Ollama {
            // Ollama otherwise keeps the model resident for five minutes.
            // These calls are cached and bursty, so unloading immediately
            // trades a future cold start for handing unified memory back to
            // the desktop. Lifted from comms, which learned it first.
            payload["keep_alive"] = serde_json::json!(0);
        }
        payload
    }

    /// Embeds a batch. `Err` carries a reason worth printing; callers are
    /// expected to degrade rather than abort.
    pub fn embed(&self, texts: &[String], text_role: TextRole) -> Result<Vec<Vec<f32>>, String> {
        let inputs = texts
            .iter()
            .cloned()
            .map(|text| (text, text_role))
            .collect::<Vec<_>>();
        self.embed_mixed(&inputs)
    }

    /// Embeds query and document inputs in one batch while applying each
    /// input's declared role prefix. Retrieval consumers use this to keep both
    /// sides in one request and one vector space.
    pub fn embed_mixed(&self, inputs: &[(String, TextRole)]) -> Result<Vec<Vec<f32>>, String> {
        if inputs.is_empty() {
            return Ok(Vec::new());
        }
        let endpoint = self.embedding_endpoint();
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|error| format!("client build: {error}"))?;

        let mut request = client
            .post(&endpoint)
            .json(&self.request_body_mixed(inputs));
        if let Some(key) = self.bearer_key() {
            request = request.bearer_auth(key);
        }

        let response = request
            .send()
            .map_err(|error| format!("POST {endpoint}: {error}"))?;
        if !response.status().is_success() {
            return Err(format!("{endpoint} returned HTTP {}", response.status()));
        }

        let vectors = match self.backend.api {
            Api::Ollama => {
                response
                    .json::<OllamaEmbedResponse>()
                    .map_err(|error| format!("parse ollama response: {error}"))?
                    .embeddings
            }
            Api::OpenAi => {
                let mut body = response
                    .json::<OpenAiEmbedResponse>()
                    .map_err(|error| format!("parse openai response: {error}"))?;
                body.data.sort_by_key(|datum| datum.index);
                body.data.into_iter().map(|datum| datum.embedding).collect()
            }
        };

        if vectors.len() != inputs.len() {
            return Err(format!(
                "expected {} embeddings, got {}",
                inputs.len(),
                vectors.len()
            ));
        }
        Ok(vectors)
    }
}

#[derive(Deserialize)]
struct OllamaEmbedResponse {
    embeddings: Vec<Vec<f32>>,
}

#[derive(Deserialize)]
struct OpenAiEmbedResponse {
    data: Vec<OpenAiEmbedDatum>,
}

#[derive(Deserialize)]
struct OpenAiEmbedDatum {
    index: usize,
    embedding: Vec<f32>,
}

/// Reads a bearer key out of a file. JSON content yields `.auth.api_key` so
/// this can point straight at `~/.omlx/settings.json`; anything else is taken
/// as the trimmed key itself. Same contract comms established.
pub fn api_key_from_file(path: Option<&str>) -> Option<String> {
    let raw = path?;
    let expanded = if let Some(rest) = raw.strip_prefix("~/") {
        PathBuf::from(std::env::var("HOME").ok()?).join(rest)
    } else {
        PathBuf::from(raw)
    };
    let content = std::fs::read_to_string(expanded).ok()?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(trimmed) {
        return json
            .get("auth")
            .and_then(|auth| auth.get("api_key"))
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .map(str::to_string);
    }
    Some(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "backends": {
        "omlx":   { "api": "openai", "base_url": "http://127.0.0.1:8000/v1",
                    "api_key_file": "~/.omlx/settings.json" },
        "ollama": { "api": "ollama", "base_url": "http://127.0.0.1:11434" }
      },
      "roles": {
        "embedding": { "backend": "omlx", "model": "multilingual-e5-base-mlx",
                       "query_prefix": "query: ", "document_prefix": "passage: " }
      }
    }"#;

    fn config() -> InferenceConfig {
        InferenceConfig::from_str(SAMPLE).expect("sample parses")
    }

    fn write_temp_key(name: &str, content: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("axon-inference-key-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("key");
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn a_role_resolves_to_its_backend() {
        let role = config().role("embedding").expect("declared");
        assert_eq!(role.backend_name, "omlx");
        assert_eq!(role.backend.api, Api::OpenAi);
        assert_eq!(role.model, "multilingual-e5-base-mlx");
    }

    #[test]
    fn an_undeclared_role_is_none_not_a_panic() {
        assert!(config().role("summarization").is_none());
    }

    #[test]
    fn the_endpoint_follows_the_api_shape_not_the_backend_name() {
        let mut cfg = config();
        let openai = cfg.role("embedding").unwrap();
        assert_eq!(
            openai.embedding_endpoint(),
            "http://127.0.0.1:8000/v1/embeddings"
        );

        cfg.roles.get_mut("embedding").unwrap().backend = "ollama".into();
        let ollama = cfg.role("embedding").unwrap();
        assert_eq!(
            ollama.embedding_endpoint(),
            "http://127.0.0.1:11434/api/embed"
        );
    }

    #[test]
    fn the_role_prefix_reaches_the_request_body() {
        let role = config().role("embedding").unwrap();
        let query = role.request_body(&["Kanutour".into()], TextRole::Query);
        assert_eq!(query["input"][0], "query: Kanutour");

        let document = role.request_body(&["Kanutour".into()], TextRole::Document);
        assert_eq!(document["input"][0], "passage: Kanutour");
    }

    #[test]
    fn mixed_embedding_inputs_keep_query_and_document_roles() {
        let role = config().role("embedding").unwrap();
        let body = role.request_body_mixed(&[
            ("lens".into(), TextRole::Query),
            ("item".into(), TextRole::Document),
        ]);
        assert_eq!(body["input"][0], "query: lens");
        assert_eq!(body["input"][1], "passage: item");
    }

    #[test]
    fn chat_endpoint_follows_the_backend_api_shape() {
        let mut cfg = config();
        assert_eq!(
            cfg.role("embedding").unwrap().chat_completions_endpoint(),
            "http://127.0.0.1:8000/v1/chat/completions"
        );
        cfg.roles.get_mut("embedding").unwrap().backend = "ollama".into();
        assert_eq!(
            cfg.role("embedding").unwrap().chat_completions_endpoint(),
            "http://127.0.0.1:11434/v1/chat/completions"
        );
    }

    #[test]
    fn bearer_key_reads_json_and_raw_secret_references() {
        let json = write_temp_key(
            "json",
            "{\"auth\": {\"api_key\": \"sk-json\"}, \"other\": 1}",
        );
        let raw = write_temp_key("raw", "  sk-raw\n");
        assert_eq!(api_key_from_file(json.to_str()), Some("sk-json".into()));
        assert_eq!(api_key_from_file(raw.to_str()), Some("sk-raw".into()));
        let _ = std::fs::remove_dir_all(json.parent().unwrap());
        let _ = std::fs::remove_dir_all(raw.parent().unwrap());
    }

    #[test]
    fn bearer_key_rejects_empty_or_wrong_json_shape() {
        let empty = write_temp_key("empty", "  \n");
        let wrong = write_temp_key("wrong-json", "{\"auth\": {}}");
        assert_eq!(api_key_from_file(empty.to_str()), None);
        assert_eq!(api_key_from_file(wrong.to_str()), None);
        let _ = std::fs::remove_dir_all(empty.parent().unwrap());
        let _ = std::fs::remove_dir_all(wrong.parent().unwrap());
    }

    /// Ollama keeps a model resident for five minutes unless told otherwise.
    #[test]
    fn the_ollama_shape_asks_for_an_immediate_unload() {
        let mut cfg = config();
        cfg.roles.get_mut("embedding").unwrap().backend = "ollama".into();
        let body = cfg
            .role("embedding")
            .unwrap()
            .request_body(&["x".into()], TextRole::Query);
        assert_eq!(body["keep_alive"], 0);
    }

    /// The claim that makes a backend switch safe rather than silently wrong.
    #[test]
    fn the_cache_key_changes_when_the_producing_model_changes() {
        let mut cfg = config();
        let omlx = cfg.role("embedding").unwrap().cache_key();

        cfg.roles.get_mut("embedding").unwrap().backend = "ollama".into();
        cfg.roles.get_mut("embedding").unwrap().model = "nomic-embed-text".into();
        let ollama = cfg.role("embedding").unwrap().cache_key();

        assert_ne!(omlx, ollama, "a cache keyed on this would swap silently");
        assert_eq!(omlx, "omlx:multilingual-e5-base-mlx");
    }

    #[test]
    fn a_missing_config_file_is_an_empty_config_not_a_failure() {
        let cfg = InferenceConfig::from_path(Path::new("/nonexistent/inference.json"));
        assert!(cfg.roles.is_empty() && cfg.backends.is_empty());
    }

    #[test]
    fn a_role_pointing_at_an_undeclared_backend_resolves_to_none() {
        let mut cfg = config();
        cfg.roles.get_mut("embedding").unwrap().backend = "vllm".into();
        assert!(cfg.role("embedding").is_none());
    }
}
