//! Axon doctrine: this crate is public. No personal value (sender names, email
//! addresses, personal classification rules) lives here -- those come from the
//! private overlay at runtime (`$AXON_PERSONAL_ROOT/config/comms.json` +
//! `comms.env`). Mirrors `capabilities/scouting`'s split exactly.
//!
//! Gmail sweeps are read-only. The only Gmail writes are explicit authenticated
//! archive or move-to-Trash actions for a stored proposal. Permanent deletion,
//! sending, and arbitrary label changes are outside this capability.

pub use axon_summarize as summarize;
pub use content_item;

pub mod capacity;
pub mod cloud_derivative;
pub mod cloud_dispatch;
pub mod cloud_run;
pub mod config;
pub mod digest;
pub mod evaluation;
pub mod extraction;
pub mod extraction_eval;
pub mod google;
pub mod grounding;
pub mod intake;
pub mod local_gate;
pub mod media;
pub mod normalize;
pub mod people_registry;
pub mod projection;
pub mod provenance;
pub mod quality;
pub mod quiet;
pub mod redaction_eval;
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
