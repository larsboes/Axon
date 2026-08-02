//! Public, zero-personal-data config for the Trips store.
//!
//! Resolution:
//!   1. `$AXON_TRIPS_DATABASE_URL`
//!   2. values from `$AXON_PERSONAL_ROOT/config/postgres.env`
//!   3. a localhost development fallback

use crate::axon_config::{expand_tilde, postgres_conn_from_shared_env, resolve_port};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
pub struct ObsidianConfig {
    pub root: PathBuf,
    pub trips_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub port: u16,
    pub obsidian: Option<ObsidianConfig>,
}

#[derive(Debug, Deserialize)]
struct TripsFileConfig {
    obsidian: Option<TripsFileObsidian>,
}

#[derive(Debug, Deserialize)]
struct TripsFileObsidian {
    root: String,
    #[serde(default = "default_trips_dir")]
    trips_dir: String,
}

fn default_trips_dir() -> String {
    "Atlas/Events".into()
}

fn obsidian_from_personal_config() -> Option<ObsidianConfig> {
    let overlay = std::env::var("AXON_PERSONAL_ROOT").ok()?;
    let path = expand_tilde(&overlay).join("config").join("trips.json");
    let body = std::fs::read_to_string(path).ok()?;
    let config: TripsFileConfig = serde_json::from_str(&body).ok()?;
    let obsidian = config.obsidian?;
    Some(ObsidianConfig {
        root: expand_tilde(&obsidian.root),
        trips_dir: PathBuf::from(obsidian.trips_dir),
    })
}

impl Config {
    pub fn load() -> Self {
        let database_url = std::env::var("AXON_TRIPS_DATABASE_URL")
            .ok()
            .or_else(postgres_conn_from_shared_env)
            .unwrap_or_else(|| {
                "host=127.0.0.1 port=5432 user=axon password=axon dbname=axon".into()
            });
        let port = resolve_port(None, None, 8086);
        let obsidian = match std::env::var("AXON_TRIPS_OBSIDIAN_ROOT") {
            Ok(root) => Some(ObsidianConfig {
                root: expand_tilde(&root),
                trips_dir: PathBuf::from(
                    std::env::var("AXON_TRIPS_OBSIDIAN_DIR")
                        .unwrap_or_else(|_| default_trips_dir()),
                ),
            }),
            Err(_) => obsidian_from_personal_config(),
        };
        Self {
            database_url,
            port,
            obsidian,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn obsidian_default_is_atlas_events() {
        assert_eq!(default_trips_dir(), "Atlas/Events");
    }
}
