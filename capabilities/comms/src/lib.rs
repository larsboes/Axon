//! Axon doctrine: this crate is public. No personal value (sender names, email
//! addresses, personal classification rules) lives here -- those come from the
//! private overlay at runtime (`$AXON_PERSONAL_ROOT/config/comms.json` +
//! `comms.env`). Mirrors `capabilities/scouting`'s split exactly.
//!
//! Phase 0 + media-v1: Gmail is strictly READ-ONLY here. The only Google
//! endpoints this crate calls are OAuth token refresh and the two read GETs on
//! `users/me/threads` (list + metadata). There is no modify/trash/delete/
//! labels/send call anywhere -- not behind a flag.

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

pub mod config;
pub mod evaluation;
pub mod extraction_eval;
pub mod google;
pub mod media;
pub mod normalize;
pub mod provenance;
pub mod quality;
pub mod relevance;
pub mod rules;
pub mod store;
pub mod sources;
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
