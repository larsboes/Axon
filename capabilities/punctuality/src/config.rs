//! Where the database is and where the raw cache lives. Same resolution order as
//! `scouting::config` and `transit::config`, deliberately: a fourth capability inventing
//! a fourth way to find the shared Postgres would be a fourth thing to fix when it moves.

use axon_config::{expand_tilde, overlay_data_dir, postgres_conn_from_shared_env};
use std::path::PathBuf;

pub struct Config {
    /// Postgres connection string for the shared instance (`capabilities/postgres`).
    /// Resolution: `AXON_PUNCTUALITY_DATABASE_URL` > built from
    /// `<overlay>/config/postgres.env` > a localhost dev-default guess.
    pub database_url: String,
    /// Where downloaded monthly parquet lands. A cache, not state: everything in it is
    /// re-downloadable, so it is deliberately outside the backup set.
    pub raw_dir: PathBuf,
}

impl Config {
    pub fn load() -> Self {
        let database_url = std::env::var("AXON_PUNCTUALITY_DATABASE_URL")
            .ok()
            .or_else(postgres_conn_from_shared_env)
            .unwrap_or_else(|| {
                "host=127.0.0.1 port=5432 user=postgres dbname=postgres".to_string()
            });

        let raw_dir = std::env::var("AXON_PUNCTUALITY_RAW_DIR")
            .ok()
            .map(|p| expand_tilde(&p))
            .or_else(|| overlay_data_dir("punctuality").map(|d| d.join("raw")))
            .unwrap_or_else(|| PathBuf::from("data/punctuality/raw"));

        Self {
            database_url,
            raw_dir,
        }
    }
}

/// Masks the password for any display purpose. Nothing in this crate prints a
/// connection string without going through here — a DSN reaches a terminal, a log, or
/// an error message eventually, and the one that reached a transcript on 2026-07-28
/// did it through a library's own exception text. The implementation (including the
/// rfind-'@' case its tests pin) moved to `axon_config::redact_dsn`; the wrapper keeps
/// this crate's callers and its documented invariant intact.
pub fn redact(url: &str) -> String {
    axon_config::redact_dsn(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_password_never_survives_redaction() {
        let masked = redact("postgresql://axon:KwOyQ+/zynCo@127.0.0.1:5432/axon");
        assert_eq!(masked, "postgresql://axon:***@127.0.0.1:5432/axon");
        assert!(!masked.contains("KwOyQ"));
    }

    #[test]
    fn a_url_without_credentials_is_left_alone() {
        assert_eq!(
            redact("postgresql://127.0.0.1:5432/axon"),
            "postgresql://127.0.0.1:5432/axon"
        );
        assert_eq!(redact("not a url"), "not a url");
    }

    #[test]
    fn a_password_containing_an_at_sign_still_masks() {
        // find('@') takes the first one, which is inside the password here. The result
        // must still not leak the tail of it.
        let masked = redact("postgresql://axon:p@ss@127.0.0.1:5432/axon");
        assert!(!masked.contains("ss@127"), "got {masked}");
    }
}
