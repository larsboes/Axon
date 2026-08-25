//! Config resolution. Axon doctrine: this crate is public, so no personal value
//! lives here — everything personal comes from the private overlay at runtime.
//!
//! The database URL resolves the way the root `ISA.md` decision of 2026-08-20
//! requires: `axon_config::database_url_override("places")` first (that is the
//! variable `tools/demo-up` exports), then the shared overlay `postgres.env`,
//! then the localhost dev-default guess.

use axon_config::{database_url_override, postgres_conn_from_shared_env, resolve_port};

pub struct Config {
    pub database_url: String,
    pub port: u16,
}

impl Config {
    pub fn load() -> Self {
        let database_url = database_url_override("places")
            .or_else(postgres_conn_from_shared_env)
            .unwrap_or_else(|| {
                "host=127.0.0.1 port=5432 user=axon password=axon dbname=axon".into()
            });
        // 8093 mirrors service.toml here; AXON_PORT (the runner's contract) wins.
        // Not 8091 (capabilities/foundation-models/service.toml owns it) and not
        // 8092 (the private overlay's interior capability owns it — a repo-only
        // port sweep misses overlay service.tomls) — the scouting/vaultwarden
        // 8080 collision class, libs/axon-config.
        let port = resolve_port(None, None, 8093);
        Self { database_url, port }
    }
}
