//! Shared config resolution for Axon's Rust capabilities. Public crate, Axon
//! doctrine: no personal value lives here — everything personal comes from the
//! private overlay at runtime via `AXON_PERSONAL_ROOT`.
//!
//! Why a shared crate now: transit/config.rs once argued "duplicated rather than
//! forcing a shared crate for ~15 lines across two standalone crates" — right at
//! two copies. By five capabilities the repo held six copies of `expand_tilde`
//! and five shared-Postgres DSN builders in two *diverging* forms, one of which
//! (the `postgresql://user:password@…` URL form) comms had already documented as
//! an auth trap: the instance's real password is base64 and can contain `/`,
//! `+`, `=`, which URL userinfo silently mangles. The keyword/value form below
//! has no such escaping trap, and `postgres::Client::connect` accepts both
//! identically. One builder, the safe form, everywhere.

use std::path::PathBuf;

/// `~/foo` → `$HOME/foo`; absolute and relative paths pass through unchanged.
pub fn expand_tilde(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(p)
}

/// The private overlay's root (`AXON_PERSONAL_ROOT`, exported by
/// `tools/lib/paths.sh` / the shell), tilde-expanded. `None` when unset —
/// callers degrade to their dev fallback, never guess a location.
pub fn overlay_root() -> Option<PathBuf> {
    std::env::var("AXON_PERSONAL_ROOT")
        .ok()
        .map(|p| expand_tilde(&p))
}

/// `<overlay>/config/<name>` — where a capability's instance config lives.
pub fn overlay_config(name: &str) -> Option<PathBuf> {
    overlay_root().map(|r| r.join("config").join(name))
}

/// `<overlay>/data/<capability>` — where a capability persists its data.
pub fn overlay_data_dir(capability: &str) -> Option<PathBuf> {
    overlay_root().map(|r| r.join("data").join(capability))
}

/// Builds a connection string for the shared local instance
/// (`capabilities/postgres`) from the same plain `KEY=value` file
/// `tools/service-runner.sh` reads for the container itself
/// (`<overlay>/config/postgres.env`, written by `tools/setup-secret.sh`).
///
/// libpq keyword/value form on purpose, never the URL form: the real
/// `POSTGRES_PASSWORD` is base64 and can contain `/`, `+`, `=`, which the URL
/// userinfo form silently mangles (`/` truncates the password → auth failure,
/// verified against the local instance). keyword/value has no escaping trap
/// for these characters and `postgres::Client::connect` accepts it identically.
pub fn postgres_conn_from_shared_env() -> Option<String> {
    let body = std::fs::read_to_string(overlay_config("postgres.env")?).ok()?;
    let get = |key: &str| -> Option<String> {
        body.lines().find_map(|l| {
            l.strip_prefix(&format!("{key}="))
                .map(|v| v.trim().to_string())
        })
    };
    let user = get("POSTGRES_USER")?;
    let password = get("POSTGRES_PASSWORD")?;
    let db = get("POSTGRES_DB")?;
    Some(format!(
        "host=127.0.0.1 port=5432 user={user} password={password} dbname={db}"
    ))
}

/// The runner's port contract, one implementation: `AXON_PORT` (exported by
/// `tools/service-runner.sh` from the manifest plus any machine-local
/// `[capability.<name>]` override) always wins; a capability-specific escape
/// hatch (e.g. `TRANSIT_PORT`) applies when running outside the runner; then
/// the capability's own config file value; then its shipped default. A server
/// that skips this resolution binds its default while the proxy and health
/// poll target the override — the scouting/vaultwarden 8080 collision class.
pub fn resolve_port(fallback_env: Option<&str>, file_port: Option<u16>, default: u16) -> u16 {
    let parse = |v: String| v.parse::<u16>().ok();
    std::env::var("AXON_PORT")
        .ok()
        .and_then(parse)
        .or_else(|| {
            fallback_env
                .and_then(|k| std::env::var(k).ok())
                .and_then(parse)
        })
        .or(file_port)
        .unwrap_or(default)
}

/// Masks the password in a connection string for any display purpose — a DSN
/// reaches a terminal, a log, or an error message eventually. Handles both
/// forms this repo uses: libpq keyword/value (`password=…`) and URL userinfo.
/// The URL branch uses `rfind('@')`: an unencoded password can itself contain
/// `@`, and the LAST `@` is the host separator — taking the first would leak
/// the tail of such a password.
pub fn redact_dsn(url: &str) -> String {
    if let Some(idx) = url.find("password=") {
        let start = idx + "password=".len();
        let end = url[start..]
            .find(' ')
            .map(|i| start + i)
            .unwrap_or(url.len());
        return format!("{}***{}", &url[..start], &url[end..]);
    }
    match (url.find("://"), url.rfind('@')) {
        (Some(scheme), Some(at)) if at > scheme => {
            let after = scheme + 3;
            let user = url[after..at].split(':').next().unwrap_or("");
            format!("{}{}:***{}", &url[..after], user, &url[at..])
        }
        _ => url.to_string(),
    }
}

// Gated on the standalone-tests feature, not bare cfg(test): these tests mutate
// process-global env (HOME, AXON_PERSONAL_ROOT, AXON_PORT). Standalone that is
// safe; compiled into a consumer via the #[path] include they would race the
// consumer's own parallel tests reading those variables (transit's store tests
// caught exactly that). Consumers never set the feature, so they skip this mod.
#[cfg(all(test, feature = "standalone-tests"))]
mod tests {
    use super::*;

    /// One lock for every env-touching test: cargo runs tests as parallel threads
    /// of one process, and HOME / AXON_PERSONAL_ROOT / AXON_PORT are process-global.
    /// EnvGuard restores values, but only a lock stops two tests interleaving.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Restores an env var on drop. Rust runs a crate's tests as threads of ONE
    /// process, so a bare `set_var`/`remove_var` leaks into sibling tests.
    struct EnvGuard(&'static str, Option<String>);

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self(key, previous)
        }
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
    fn expand_tilde_uses_home() {
        let _env = env_lock();
        let _home = EnvGuard::set("HOME", "/tmp/fake-home");
        assert_eq!(expand_tilde("~/foo"), PathBuf::from("/tmp/fake-home/foo"));
        assert_eq!(expand_tilde("/abs/path"), PathBuf::from("/abs/path"));
        assert_eq!(expand_tilde("rel/path"), PathBuf::from("rel/path"));
    }

    #[test]
    fn overlay_paths_derive_from_the_env_root() {
        let _env = env_lock();
        let _o = EnvGuard::set("AXON_PERSONAL_ROOT", "/tmp/fake-overlay");
        assert_eq!(
            overlay_config("postgres.env").unwrap(),
            PathBuf::from("/tmp/fake-overlay/config/postgres.env")
        );
        assert_eq!(
            overlay_data_dir("scouting").unwrap(),
            PathBuf::from("/tmp/fake-overlay/data/scouting")
        );
    }

    #[test]
    fn overlay_absent_means_none_not_a_guess() {
        let _env = env_lock();
        let _o = EnvGuard::take("AXON_PERSONAL_ROOT");
        assert!(overlay_root().is_none());
        assert!(postgres_conn_from_shared_env().is_none());
    }

    #[test]
    fn shared_env_builds_keyword_value_form() {
        let _env = env_lock();
        let dir = std::env::temp_dir().join(format!("axon-config-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("config")).unwrap();
        std::fs::write(
            dir.join("config/postgres.env"),
            "POSTGRES_USER=axon\nPOSTGRES_PASSWORD=KwOyQ+/z=\nPOSTGRES_DB=axon\n",
        )
        .unwrap();
        let _o = EnvGuard::set("AXON_PERSONAL_ROOT", dir.to_str().unwrap());
        assert_eq!(
            postgres_conn_from_shared_env().unwrap(),
            "host=127.0.0.1 port=5432 user=axon password=KwOyQ+/z= dbname=axon"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn port_resolution_order_is_axon_port_then_fallback_then_file_then_default() {
        let _env = env_lock();
        let _a = EnvGuard::take("AXON_PORT");
        let _f = EnvGuard::take("AXON_CONFIG_TEST_PORT");
        assert_eq!(
            resolve_port(Some("AXON_CONFIG_TEST_PORT"), Some(9000), 8000),
            9000
        );
        assert_eq!(
            resolve_port(Some("AXON_CONFIG_TEST_PORT"), None, 8000),
            8000
        );
        let _f2 = EnvGuard::set("AXON_CONFIG_TEST_PORT", "9100");
        assert_eq!(
            resolve_port(Some("AXON_CONFIG_TEST_PORT"), Some(9000), 8000),
            9100
        );
        let _a2 = EnvGuard::set("AXON_PORT", "9200");
        assert_eq!(
            resolve_port(Some("AXON_CONFIG_TEST_PORT"), Some(9000), 8000),
            9200
        );
    }

    #[test]
    fn redact_handles_both_dsn_forms() {
        assert_eq!(
            redact_dsn("host=127.0.0.1 port=5432 user=axon password=s3cr3t dbname=axon"),
            "host=127.0.0.1 port=5432 user=axon password=*** dbname=axon"
        );
        assert_eq!(
            redact_dsn("postgresql://axon:s3cr3t@127.0.0.1:5432/axon"),
            "postgresql://axon:***@127.0.0.1:5432/axon"
        );
    }

    #[test]
    fn a_password_containing_an_at_sign_still_masks() {
        let masked = redact_dsn("postgresql://axon:p@ss@127.0.0.1:5432/axon");
        assert!(!masked.contains("ss@127"), "got {masked}");
    }

    #[test]
    fn a_dsn_without_credentials_is_left_alone() {
        assert_eq!(
            redact_dsn("postgresql://127.0.0.1:5432/axon"),
            "postgresql://127.0.0.1:5432/axon"
        );
        assert_eq!(redact_dsn("not a url"), "not a url");
    }
}
