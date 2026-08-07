//! Public, zero-personal-data config for the calendar store and its Google
//! sync. Every personal value — the home timezone, which Google calendar,
//! where the credential file lives — comes from the private overlay at
//! runtime, never from this repo.
//!
//! Resolution for the connection and port:
//!   1. `$AXON_CALENDAR_DATABASE_URL`
//!   2. values from `$AXON_PERSONAL_ROOT/config/postgres.env`
//!   3. a localhost development fallback
//!
//! Resolution for everything else (a JSON file, mirroring comms/scouting):
//!   1. `$AXON_CALENDAR_CONFIG` (explicit override, full path)
//!   2. `$AXON_PERSONAL_ROOT/config/calendar.json` (the overlay)
//!   3. `capabilities/calendar/calendar.config.json` (local, gitignored)
//!
//! There is no file at all in the common case: Phases A–D need none, and
//! `Config::load` returns working defaults for everything except the two
//! values Phase E refuses to guess (see `GoogleConfig`).

use std::path::PathBuf;

use serde::Deserialize;

use axon_config::{expand_tilde, overlay_config, postgres_conn_from_shared_env, resolve_port};

/// Where the Google credential and the calendar to sync are named.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct GoogleConfig {
    /// `KEY=value` file holding GOOGLE_CLIENT_ID / GOOGLE_CLIENT_SECRET /
    /// GOOGLE_REFRESH_TOKEN. Default `$AXON_PERSONAL_ROOT/config/calendar.env`.
    ///
    /// Pointing this at comms' `comms.env` is supported and is the cheap path:
    /// `capabilities/comms/auth/get-refresh-token.ts` already requests
    /// `calendar.events` alongside its Gmail scope, so that grant covers this
    /// capability too. Its own default is a separate file so the two can be
    /// revoked independently.
    pub env_path: Option<String>,
    /// Which Google calendar to sync. `primary` is the account's own calendar;
    /// a secondary calendar is addressed by its long `…@group.calendar.google.com`
    /// id. No default on purpose — a wrong guess would import a stranger's
    /// calendar or export into one.
    pub calendar_id: Option<String>,
    /// Import window, relative to today. Past events are of little use to an
    /// availability layer, so the default reaches back only far enough to
    /// catch an event that moved earlier.
    pub import_days_back: i64,
    pub import_days_ahead: i64,
    /// Hard bound on how many events one import will page through.
    pub max_events: usize,
}

impl Default for GoogleConfig {
    fn default() -> Self {
        Self {
            env_path: None,
            calendar_id: None,
            import_days_back: 7,
            import_days_ahead: 120,
            max_events: 1_000,
        }
    }
}

impl GoogleConfig {
    /// The credential file's resolved path. Falls back to the overlay's
    /// `calendar.env`, and to a bare relative name when there is no overlay —
    /// which then fails loudly at read time naming that path.
    pub fn env_path(&self) -> PathBuf {
        self.env_path
            .as_deref()
            .map(expand_tilde)
            .or_else(|| overlay_config("calendar.env"))
            .unwrap_or_else(|| PathBuf::from("calendar.env"))
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
struct FileConfig {
    database_url: Option<String>,
    port: Option<u16>,
    home_timezone: Option<String>,
    home_city: Option<String>,
    trips_base_url: Option<String>,
    google: Option<GoogleConfig>,
    #[serde(default)]
    markdown_sources: Vec<crate::markdown_import::MarkdownSource>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub port: u16,
    /// The operator's home timezone. Everything stored is naive wall time in
    /// this zone (README § Time model), so Phase E cannot convert a Google
    /// offset without it.
    ///
    /// `None` by default and never guessed: a wrong zone writes every imported
    /// event an hour or more off, silently and plausibly. Import and export
    /// both refuse until it is set. Phases A–D never read it.
    pub home_timezone: Option<String>,
    /// Where the operator lives, so Phase D can tell a trip from an evening
    /// out. Optional on purpose: absent means every place clusters, including
    /// this one, which is wrong in a way you can see rather than a filter that
    /// quietly eats things.
    pub home_city: Option<String>,
    /// Where trips answers. Calendar posts a plan to its public API and never
    /// reaches into its store, so this is a URL and not a database handle.
    pub trips_base_url: String,
    pub google: GoogleConfig,
    /// Declared markdown event sources. Empty by default and empty in the
    /// public template: a note store is something an operator points calendar
    /// at, never something it goes looking for. `~/` is expanded here so the
    /// importer only ever sees a real path.
    pub markdown_sources: Vec<crate::markdown_import::MarkdownSource>,
}

impl Config {
    /// One declared markdown source by id, enabled ones only. A disabled source
    /// answers the same as an unknown one: the operator turned it off, and a
    /// scan that ran anyway would be ignoring that.
    pub fn markdown_source(&self, id: &str) -> Option<&crate::markdown_import::MarkdownSource> {
        self.markdown_sources
            .iter()
            .find(|source| source.enabled && source.id == id)
    }
}

/// The JSON config file this capability would read. Public so a "you have not
/// configured X" error can name the exact path the operator has to create,
/// rather than describing one.
pub fn config_path() -> PathBuf {
    if let Ok(path) = std::env::var("AXON_CALENDAR_CONFIG") {
        return expand_tilde(&path);
    }
    if let Some(path) = overlay_config("calendar.json") {
        return path;
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("calendar.config.json")
}

fn load_file_config() -> FileConfig {
    let path = config_path();
    if !path.is_file() {
        return FileConfig::default();
    }
    match std::fs::read_to_string(&path) {
        Ok(body) => serde_json::from_str(&body).unwrap_or_else(|error| {
            eprintln!("warning: could not parse {path:?}: {error} — using defaults");
            FileConfig::default()
        }),
        Err(_) => FileConfig::default(),
    }
}

impl Config {
    pub fn load() -> Self {
        let file = load_file_config();
        let database_url = std::env::var("AXON_CALENDAR_DATABASE_URL")
            .ok()
            .or(file.database_url)
            .or_else(postgres_conn_from_shared_env)
            .unwrap_or_else(|| {
                "host=127.0.0.1 port=5432 user=axon password=axon dbname=axon".into()
            });
        Self {
            database_url,
            port: resolve_port(Some("AXON_CALENDAR_PORT"), file.port, 8087),
            // Deployment declaration first, capability override second — one
            // implementation in axon_config so calendar and scouting cannot drift.
            // A conflict resolves to None deliberately: the caller's own
            // refuse-to-guess error then fires, which is the fail-closed direction
            // for a value that silently shifts every stored wall time when wrong.
            home_timezone: axon_config::resolve_home_timezone(
                file.home_timezone.as_deref(),
                "calendar.json",
            )
            .unwrap_or_else(|conflict| {
                eprintln!("warning: {conflict}");
                None
            }),
            home_city: file.home_city.filter(|city| !city.trim().is_empty()),
            trips_base_url: file
                .trips_base_url
                .filter(|url| !url.trim().is_empty())
                .unwrap_or_else(|| "http://127.0.0.1:8086".to_string()),
            google: file.google.unwrap_or_default(),
            markdown_sources: file
                .markdown_sources
                .into_iter()
                .map(|mut source| {
                    source.path = expand_tilde(&source.path).to_string_lossy().into_owned();
                    source
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Restores an env var on drop. Rust runs a crate's tests as threads of
    /// one process, so an unrestored `remove_var` leaks into every later test
    /// — the trap comms' config tests documented after it cost them eight
    /// failures against a healthy database.
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
                Some(value) => std::env::set_var(self.0, value),
                None => std::env::remove_var(self.0),
            }
        }
    }

    #[test]
    fn google_defaults_name_nothing_personal() {
        let google = GoogleConfig::default();
        assert!(google.calendar_id.is_none(), "no calendar is guessed");
        assert!(google.env_path.is_none());
        assert_eq!(google.import_days_ahead, 120);
        assert_eq!(google.import_days_back, 7);
    }

    #[test]
    fn a_configured_env_path_wins_over_the_overlay_default() {
        let google = GoogleConfig {
            env_path: Some("/etc/axon/creds.env".into()),
            ..Default::default()
        };
        assert_eq!(google.env_path(), PathBuf::from("/etc/axon/creds.env"));
    }

    #[test]
    fn the_home_timezone_has_no_default() {
        let _config = EnvGuard::take("AXON_CALENDAR_CONFIG");
        let _overlay = EnvGuard::take("AXON_PERSONAL_ROOT");
        let config = Config::load();
        assert!(
            config.home_timezone.is_none(),
            "guessing a zone writes every import silently off by an hour"
        );
        assert_eq!(config.port, 8087);
    }

    #[test]
    fn a_file_supplies_the_personal_values() {
        let dir = std::env::temp_dir().join(format!("calendar-config-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("calendar.json");
        std::fs::write(
            &path,
            r#"{"home_timezone":"Europe/Berlin","google":{"calendar_id":"primary","import_days_ahead":30}}"#,
        )
        .unwrap();

        let previous = std::env::var("AXON_CALENDAR_CONFIG").ok();
        std::env::set_var("AXON_CALENDAR_CONFIG", &path);
        let config = Config::load();
        match previous {
            Some(value) => std::env::set_var("AXON_CALENDAR_CONFIG", value),
            None => std::env::remove_var("AXON_CALENDAR_CONFIG"),
        }
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(config.home_timezone.as_deref(), Some("Europe/Berlin"));
        assert_eq!(config.google.calendar_id.as_deref(), Some("primary"));
        assert_eq!(config.google.import_days_ahead, 30);
        assert_eq!(
            config.google.import_days_back, 7,
            "an unspecified field keeps its default"
        );
    }
}
