//! Axon doctrine: this crate is public. No personal value (a home/default
//! station pair) lives here -- that comes from the private overlay at
//! runtime, if set at all. Mirrors `capabilities/scouting/src/config.rs`'s
//! `Config::load()` resolution exactly:
//!   1. `$AXON_TRANSIT_CONFIG` (explicit override, full path to a JSON file)
//!   2. `$AXON_PERSONAL_ROOT/config/transit.json` (the overlay; exported by
//!      `tools/lib/paths.sh` / `~/.zshrc`)
//!   3. `capabilities/transit/transit.config.json` next to this crate's
//!      source (local, gitignored -- dev fallback)
//! See `transit.config.example.json` for the shape.
//!
//! CLI args always win over whatever this module resolves -- `Config::load()`
//! only supplies defaults; `main.rs` overrides individual fields from
//! `--flag` values where a flag exists for that field.
//!
//! Unlike scouting, there is deliberately no baked-in station-pair default
//! anywhere (the original had `8000044`/`8098160` -- real Bonn/Berlin EVA
//! codes -- hardcoded as CLI argument defaults). `default_from_eva`/
//! `default_to_eva`/`default_time` exist purely as an opt-in convenience: set
//! your own home route in the overlay if you want `transit search`/`split`
//! runnable with no flags; leave them unset and the CLI requires `--from`/
//! `--to` explicitly, erroring with a clear message rather than silently
//! defaulting to someone else's stations.

use axon_config::{expand_tilde, postgres_conn_from_shared_env};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, Default)]
struct FileConfig {
    default_from_eva: Option<String>,
    default_to_eva: Option<String>,
    default_time: Option<String>,
    database_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub default_from_eva: Option<String>,
    pub default_to_eva: Option<String>,
    pub default_time: Option<String>,
    /// Postgres connection string for `store::TransitStore` (own `transit`
    /// schema on the shared local instance -- see `capabilities/postgres`),
    /// libpq keyword/value form via `axon_config::postgres_conn_from_shared_env`.
    /// Resolution: explicit override here, else built from
    /// `axon-overlay/config/postgres.env`, else a localhost dev-default guess.
    pub database_url: String,
}

fn config_path() -> PathBuf {
    if let Ok(p) = std::env::var("AXON_TRANSIT_CONFIG") {
        return expand_tilde(&p);
    }
    if let Ok(overlay) = std::env::var("AXON_PERSONAL_ROOT") {
        return expand_tilde(&overlay).join("config").join("transit.json");
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("transit.config.json")
}

fn load_file_config() -> FileConfig {
    let path = config_path();
    if !path.is_file() {
        return FileConfig::default();
    }
    match std::fs::read_to_string(&path) {
        Ok(body) => serde_json::from_str(&body).unwrap_or_else(|e| {
            eprintln!("warning: could not parse {path:?}: {e} -- using defaults");
            FileConfig::default()
        }),
        Err(_) => FileConfig::default(),
    }
}

/// The duplicated-helper era ended with `libs/axon-config`: the "duplicated
/// rather than forcing a shared crate for ~15 lines" call this file used to
/// make was right at two copies and wrong at six -- see that crate's README.
/// Redaction stays exported under its old name so callers don't churn.
pub fn redact_database_url(url: &str) -> String {
    axon_config::redact_dsn(url)
}

impl Config {
    pub fn load() -> Self {
        let file = load_file_config();
        let database_url = file.database_url.unwrap_or_else(|| {
            postgres_conn_from_shared_env()
                .unwrap_or_else(|| "host=127.0.0.1 port=5432 user=axon password=axon dbname=axon".into())
        });
        Self {
            default_from_eva: file.default_from_eva,
            default_to_eva: file.default_to_eva,
            default_time: file.default_time,
            database_url,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Restores an env var on drop. Rust runs a crate's tests as threads of ONE
    /// process, so `remove_var` here is not local to this test: unrestored, it
    /// left every later store test resolving the fallback connection string
    /// instead of the overlay's real one, and they failed against a perfectly
    /// healthy Postgres.
    struct EnvGuard(&'static str, Option<String>);

    impl EnvGuard {
        fn take(key: &'static str) -> Self {
            let previous = std::env::var(key).ok();
            std::env::remove_var(key);
            Self(key, previous)
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match self.1.take() {
                Some(v) => std::env::set_var(self.0, v),
                None => std::env::remove_var(self.0),
            }
        }
    }

    #[test]
    fn load_with_no_env_and_no_file_uses_defaults() {
        let _config = EnvGuard::take("AXON_TRANSIT_CONFIG");
        let _overlay = EnvGuard::take("AXON_PERSONAL_ROOT");
        let cfg = Config::load();
        assert!(cfg.default_from_eva.is_none());
        assert!(cfg.default_to_eva.is_none());
    }

    #[test]
    fn redact_database_url_hides_password_only() {
        assert_eq!(
            redact_database_url("postgresql://axon:s3cr3t@127.0.0.1:5432/axon"),
            "postgresql://axon:***@127.0.0.1:5432/axon"
        );
    }
}
