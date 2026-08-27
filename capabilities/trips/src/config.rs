//! Public, zero-personal-data config for the Trips store.
//!
//! The store path comes from `axon_config::database_path` — `AXON_DB_PATH`, else
//! `<overlay>/data/axon/axon.db`. One file for every capability (PRD Q45), so
//! there is no per-capability database to resolve any more.

use axon_config::{database_path, expand_tilde, resolve_port};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
pub struct ObsidianConfig {
    pub root: PathBuf,
    pub trips_dir: PathBuf,
}

/// A city where sleeping costs nothing and staying is wanted -- a friend, family.
/// The pivot-routing search (PRD F4) enumerates itineraries THROUGH these, which
/// is the one thing no commercial engine can offer: it does not know where you
/// are welcome. Personal by nature, so it lives in the overlay's trips.json,
/// never in this repository.
#[derive(Debug, Clone, Deserialize)]
pub struct PivotConfig {
    pub name: String,
    pub iata: String,
    /// How many nights staying there is welcome, which becomes the offset range
    /// for the onward leg. Defaults to 2.
    #[serde(default = "default_pivot_nights")]
    pub max_nights: u8,
}

fn default_pivot_nights() -> u8 {
    2
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct TravelPrefs {
    /// Where searches start when the caller names no origin.
    pub home_airport: Option<String>,
    #[serde(default)]
    pub pivots: Vec<PivotConfig>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub database_path: PathBuf,
    pub port: u16,
    pub obsidian: Option<ObsidianConfig>,
    pub travel: TravelPrefs,
}

#[derive(Debug, Deserialize)]
struct TripsFileConfig {
    obsidian: Option<TripsFileObsidian>,
    #[serde(default)]
    travel: Option<TravelPrefs>,
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

fn file_config() -> Option<TripsFileConfig> {
    let overlay = std::env::var("AXON_PERSONAL_ROOT").ok()?;
    let path = expand_tilde(&overlay).join("config").join("trips.json");
    let body = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&body).ok()
}

fn obsidian_from_personal_config() -> Option<ObsidianConfig> {
    let obsidian = file_config()?.obsidian?;
    Some(ObsidianConfig {
        root: expand_tilde(&obsidian.root),
        trips_dir: PathBuf::from(obsidian.trips_dir),
    })
}

impl Config {
    pub fn load() -> Self {
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
        let travel = file_config().and_then(|c| c.travel).unwrap_or_default();
        Self {
            database_path: database_path(),
            port,
            obsidian,
            travel,
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
