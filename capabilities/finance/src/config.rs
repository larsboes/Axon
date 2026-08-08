//! Public, zero-personal-data config for the finance store.
//!
//! Nothing here names a vault, a bank, an institution or a figure. The overlay
//! supplies all of it, which is what keeps this capability publishable while the
//! data it operates on is the most private in the system.
//!
//! Resolution, in order:
//!   1. `$AXON_FINANCE_DATABASE_URL`
//!   2. values from `$AXON_PERSONAL_ROOT/config/postgres.env`
//!   3. a localhost development fallback

use axon_config::{expand_tilde, postgres_conn_from_shared_env, resolve_port};
use serde::Deserialize;
use std::path::PathBuf;

/// Where the subscription notes live. The journal root is deliberately absent:
/// phase 3 adds it, and declaring a config key before anything reads it is how a
/// manifest starts lying.
#[derive(Debug, Clone, Deserialize)]
pub struct ObsidianConfig {
    pub root: PathBuf,
    pub subscriptions_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub port: u16,
    pub obsidian: Option<ObsidianConfig>,
}

#[derive(Debug, Deserialize)]
struct FinanceFileConfig {
    obsidian: Option<FinanceFileObsidian>,
}

#[derive(Debug, Deserialize)]
struct FinanceFileObsidian {
    root: String,
    #[serde(default = "default_subscriptions_dir")]
    subscriptions_dir: String,
}

fn default_subscriptions_dir() -> String {
    "Atlas/Finance/Subscriptions".into()
}

fn obsidian_from_personal_config() -> Option<ObsidianConfig> {
    let overlay = std::env::var("AXON_PERSONAL_ROOT").ok()?;
    let path = expand_tilde(&overlay).join("config").join("finance.json");
    let body = std::fs::read_to_string(path).ok()?;
    let config: FinanceFileConfig = serde_json::from_str(&body).ok()?;
    let obsidian = config.obsidian?;
    Some(ObsidianConfig {
        root: expand_tilde(&obsidian.root),
        subscriptions_dir: PathBuf::from(obsidian.subscriptions_dir),
    })
}

impl Config {
    pub fn load() -> Self {
        let database_url = std::env::var("AXON_FINANCE_DATABASE_URL")
            .ok()
            .or_else(postgres_conn_from_shared_env)
            .unwrap_or_else(|| {
                "host=127.0.0.1 port=5432 user=axon password=axon dbname=axon".into()
            });
        let port = resolve_port(None, None, 8090);
        let obsidian = match std::env::var("AXON_FINANCE_OBSIDIAN_ROOT") {
            Ok(root) => Some(ObsidianConfig {
                root: expand_tilde(&root),
                subscriptions_dir: PathBuf::from(
                    std::env::var("AXON_FINANCE_OBSIDIAN_DIR")
                        .unwrap_or_else(|_| default_subscriptions_dir()),
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
    fn the_default_directory_matches_where_the_notes_already_live() {
        assert_eq!(default_subscriptions_dir(), "Atlas/Finance/Subscriptions");
    }
}
