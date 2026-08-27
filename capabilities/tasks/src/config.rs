//! Public, zero-personal-data config for the Tasks store.
//!
//! Resolution: `axon_config::database_path` — `AXON_DB_PATH`, else
//! `<overlay>/data/axon/axon.db`. One file for every capability (PRD Q45), so
//! there is no per-capability database to resolve any more.

use std::path::PathBuf;

use axon_config::{database_path, resolve_port};

#[derive(Debug, Clone)]
pub struct Config {
    pub database_path: PathBuf,
    pub port: u16,
}

impl Config {
    pub fn load() -> Self {
        Self {
            database_path: database_path(),
            port: resolve_port(None, None, 8089),
        }
    }
}
