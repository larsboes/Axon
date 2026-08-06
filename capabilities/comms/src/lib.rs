//! Axon doctrine: this crate is public. No personal value (sender names, email
//! addresses, personal classification rules) lives here -- those come from the
//! private overlay at runtime (`$AXON_PERSONAL_ROOT/config/comms.json` +
//! `comms.env`). Mirrors `capabilities/scouting`'s split exactly.
//!
//! Gmail sweeps are read-only. The only Gmail writes are explicit authenticated
//! archive or move-to-Trash actions for a stored proposal. Permanent deletion,
//! sending, and arbitrary label changes are outside this capability.

// Shared config helpers, compiled in by #[path] include rather than a cargo
// dependency: rules_rust's splicer flattens listed manifests and breaks
// ../../libs path deps, and folding a small shared shape in as a module is
// this repo's stated preference anyway (libs/axon-config/README.md).
#[path = "../../../libs/axon-config/src/lib.rs"]
#[allow(dead_code)]
pub(crate) mod axon_config;

// Shared model-role resolution. Comms owns Feed behavior; libs/inference owns
// which backend and model perform embedding or summarization on this machine.
#[path = "../../../libs/inference/src/lib.rs"]
#[allow(dead_code)]
pub(crate) mod inference;

// The `content-item-v1` reader contract, on the same include terms as calendar
// uses it. It owns data classification for every capability, mail included:
// this module previously carried its own byte-identical copy of `DataClass` and
// `processing_policy`, which is exactly the drift the shared lib exists to stop.
#[path = "../../../libs/content-item/src/lib.rs"]
#[allow(dead_code)]
pub mod content_item;

// The adaptive digest engine. It used to be `media::summarize` -- one prompt,
// one token ceiling, reachable only from the feed ingest path. Three sources
// want the same artifact, so the ladder, the prompt and the Mermaid gate moved
// out where calendar can include them on the same terms.
#[path = "../../../libs/summarize/src/lib.rs"]
#[allow(dead_code)]
pub mod summarize;

pub mod cloud_derivative;
pub mod cloud_dispatch;
pub mod config;
pub mod digest;
pub mod evaluation;
pub mod extraction;
pub mod extraction_eval;
pub mod google;
pub mod intake;
pub mod local_gate;
pub mod media;
pub mod normalize;
pub mod provenance;
pub mod quality;
pub mod relevance;
pub mod rules;
pub mod sources;
pub mod store;
pub mod travel;
pub mod vault_links;

use thiserror::Error;

/// One error type shared across the network-facing modules (google, media).
/// `store` stays on `Box<dyn std::error::Error>` to mirror scouting's store.rs
/// pattern verbatim.
#[derive(Debug, Error)]
pub enum CommsError {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("config error: {0}")]
    Config(String),
    #[error("auth error: {0}")]
    Auth(String),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, CommsError>;
