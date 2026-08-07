//! Public, zero-personal-data config for the Tasks store.
//!
//! Resolution:
//!   1. `$AXON_TASKS_DATABASE_URL`
//!   2. values from `$AXON_PERSONAL_ROOT/config/postgres.env`
//!   3. a localhost development fallback

use axon_config::{postgres_conn_from_shared_env, resolve_port};

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub port: u16,
}

impl Config {
    pub fn load() -> Self {
        let database_url = std::env::var("AXON_TASKS_DATABASE_URL")
            .ok()
            .or_else(postgres_conn_from_shared_env)
            .unwrap_or_else(|| {
                "host=127.0.0.1 port=5432 user=axon password=axon dbname=axon".into()
            });
        Self {
            database_url,
            port: resolve_port(None, None, 8089),
        }
    }
}
