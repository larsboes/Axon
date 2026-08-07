//! Public config. No personal value lives here -- it comes from the private
//! overlay at runtime. Resolution order mirrors scouting's config.rs:
//!   1. `$AXON_COMMS_CONFIG` (explicit override, full path to a JSON file)
//!   2. `$AXON_PERSONAL_ROOT/config/comms.json` (the overlay)
//!   3. `capabilities/comms/comms.config.json` next to this source
//!      (local, gitignored -- dev fallback)
//!
//! See `comms.config.example.json` for the shape. Every field is optional;
//! the tool runs zero-config against a local Postgres and no rules.

use serde::Deserialize;
use std::path::PathBuf;

use crate::rules::Rule;
use axon_inference::{InferenceConfig, ResolvedRole};

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct RelevanceConfig {
    /// Exact Markdown files or non-recursive directories containing TELOS
    /// focus lenses. Personal paths belong in the private overlay.
    pub profile_paths: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct QualityFlagConfig {
    /// Passing corpus fixtures retain 39.7–89.6% of their total raw text.
    /// Round outward so the observed fixtures remain inside the review band.
    pub minimum_total_retention_percent: f64,
    pub maximum_total_retention_percent: f64,
    /// Every passing corpus fixture leaks zero judged boilerplate.
    pub maximum_boilerplate_leakage_percent: f64,
    /// The summary retry ledger caps at three; warn on the attempt before it parks.
    pub summary_attempt_warning: i32,
}

impl Default for QualityFlagConfig {
    fn default() -> Self {
        Self {
            minimum_total_retention_percent: 39.0,
            maximum_total_retention_percent: 90.0,
            maximum_boilerplate_leakage_percent: 0.0,
            summary_attempt_warning: 2,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct TravelContextConfig {
    /// Trips owns the plan data. Comms reads its existing HTTP contract and
    /// retains only the last bounded context snapshot used for Feed ranking.
    pub enabled: bool,
    pub base_url: String,
    pub max_plans: usize,
    pub timeout_ms: u64,
}

impl Default for TravelContextConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            base_url: "http://127.0.0.1:8086".into(),
            max_plans: 20,
            timeout_ms: 2_000,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct CalendarContextConfig {
    /// Calendar owns its entries. Comms reads one over Calendar's existing
    /// `content-item-v1` route to digest it — the same bounded
    /// cross-capability read it already does against Trips, rather than
    /// reaching into a second capability's database schema.
    pub base_url: String,
    pub timeout_ms: u64,
}

impl Default for CalendarContextConfig {
    fn default() -> Self {
        Self {
            base_url: "http://127.0.0.1:8087".into(),
            timeout_ms: 2_000,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct VaultLinkSourceConfig {
    /// Stable provenance id, for example `scratchpad-to-read`.
    pub id: String,
    /// Exact Markdown file. Comms never walks the vault recursively.
    pub path: String,
    /// Optional exact Markdown heading text. Only links below that heading and
    /// before the next heading of the same or higher level are candidates.
    /// A missing heading yields zero candidates, never a whole-file fallback.
    pub heading: Option<String>,
    pub enabled: bool,
}

impl Default for VaultLinkSourceConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            path: String::new(),
            heading: None,
            enabled: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct FeedSourceConfig {
    /// Stable provenance id shown in the Feed.
    pub id: String,
    /// `github-trending` or `arxiv`.
    pub adapter: String,
    pub enabled: bool,
    /// arXiv search_query. Empty for GitHub Trending.
    pub query: Option<String>,
    /// Optional GitHub Trending language slug, for example `rust`.
    pub language: Option<String>,
    /// GitHub Trending window: daily, weekly or monthly.
    pub since: Option<String>,
    /// Hard per-run bound. Clamped again by the adapter.
    pub limit: usize,
}

impl Default for FeedSourceConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            adapter: String::new(),
            enabled: true,
            query: None,
            language: None,
            since: None,
            limit: 10,
        }
    }
}

fn default_feed_sources() -> Vec<FeedSourceConfig> {
    vec![
        FeedSourceConfig {
            id: "github-trending-daily".into(),
            adapter: "github-trending".into(),
            enabled: true,
            query: None,
            language: None,
            since: Some("daily".into()),
            limit: 12,
        },
        FeedSourceConfig {
            id: "arxiv-ai-recent".into(),
            adapter: "arxiv".into(),
            enabled: true,
            query: Some("cat:cs.AI OR cat:cs.LG OR cat:cs.CL".into()),
            language: None,
            since: None,
            limit: 12,
        },
    ]
}

#[derive(Debug, Clone, Deserialize, Default)]
struct FileConfig {
    database_url: Option<String>,
    google_env_path: Option<String>,
    port: Option<u16>,
    api_secret_file: Option<String>,
    dashboard_origin: Option<String>,
    enrichment_drain_minutes: Option<u64>,
    digest_drain_minutes: Option<u64>,
    gmail_maintenance_minutes: Option<u64>,
    inbox_sweep_minutes: Option<u64>,
    inbox_sweep_max_threads: Option<usize>,
    inbox_sweep_quiet_hours: Option<String>,
    #[serde(default)]
    rules: Vec<Rule>,
    keeper_export_dir: Option<String>,
    relevance: Option<RelevanceConfig>,
    travel_context: Option<TravelContextConfig>,
    calendar_context: Option<CalendarContextConfig>,
    #[serde(default)]
    vault_link_sources: Vec<VaultLinkSourceConfig>,
    feed_sources: Option<Vec<FeedSourceConfig>>,
    quality_flags: Option<QualityFlagConfig>,
}

#[derive(Debug, Clone)]
pub struct Config {
    /// Postgres connection string. Resolution: explicit `database_url` >
    /// built from `axon-overlay/config/postgres.env` (the shared instance,
    /// `capabilities/postgres`) > a localhost dev-default guess.
    pub database_url: String,
    /// Path to the `KEY=value` env file holding GOOGLE_CLIENT_ID /
    /// GOOGLE_CLIENT_SECRET / GOOGLE_REFRESH_TOKEN. Default
    /// `$AXON_PERSONAL_ROOT/config/comms.env`.
    pub google_env_path: PathBuf,
    /// HTTP port for `comms-server`.
    pub port: u16,
    /// Resolved shared secret token for mutating HTTP endpoints (`POST /ingest` etc).
    pub api_secret: Option<String>,
    /// Allowed CORS origin for cross-origin requests (defaults to `http://127.0.0.1:47117`).
    pub dashboard_origin: String,
    /// How often comms-server drains the summary backlog, in minutes. `0`
    /// disables the pass entirely, which is the setting for a machine that runs
    /// `comms summarize --pending` from somewhere else. Without a drain, items
    /// ingested while the inference server was unreachable stay empty forever —
    /// that is how 36 of 39 items ended up without a summary (#74).
    pub enrichment_drain_minutes: u64,
    /// How often comms-server retries digests that failed retryably, in
    /// minutes. `0` disables the pass, leaving digests manual-press-only.
    ///
    /// Separate from `enrichment_drain_minutes` because the two write different
    /// things — that one fills `feed_items.summary`, this one fills
    /// `content_digests` — and a machine may reasonably want one without the
    /// other. Bounded by the same ledger: three attempts, then the row rests.
    pub digest_drain_minutes: u64,
    /// Retry durable Gmail actions and reconcile labels on this interval. `0`
    /// disables automatic maintenance; manual reconciliation remains available.
    pub gmail_maintenance_minutes: u64,
    /// Pull new inbox proposals on this interval. **`0`, disabled, is the
    /// default and the shipped value.** An unattended job that reads a mailbox
    /// is opt-in per machine, never something a fresh clone starts doing.
    pub inbox_sweep_minutes: u64,
    /// Newest-N threads per scheduled pass. A bound, not a cursor: a cursor
    /// that advances every run walks backwards through the whole mailbox over
    /// time, which is the unbounded rescan this schedule exists to avoid.
    /// Re-reading the newest page is free because upserts key on thread id.
    pub inbox_sweep_max_threads: usize,
    /// `"22-7"` — local hours `[start, end)` in which the schedule holds off.
    /// `None` means no quiet window. Manual sweeps ignore it: the point is to
    /// stop unattended traffic, not to lock the operator out of their own tool.
    pub inbox_sweep_quiet_hours: Option<(u32, u32)>,
    /// Shared, machine-resolved inference roles. Backend URLs, models and key
    /// references never belong to Comms configuration.
    pub inference: InferenceConfig,
    /// Config-driven classification rules (first match wins, before built-in
    /// heuristics). Empty = built-in heuristics only.
    pub rules: Vec<Rule>,
    /// Optional directory to export distilled keeper notes into on `comms keep`.
    pub keeper_export_dir: Option<PathBuf>,
    pub relevance: RelevanceConfig,
    pub travel_context: TravelContextConfig,
    pub calendar_context: CalendarContextConfig,
    /// Explicit Markdown link sources. This is intentionally not a vault root:
    /// Scratchpad and other notes can contain credentials and admin URLs.
    pub vault_link_sources: Vec<VaultLinkSourceConfig>,
    /// General awareness sources. `None` in a file uses the public, non-personal
    /// defaults; an explicit empty array disables them all.
    pub feed_sources: Vec<FeedSourceConfig>,
    pub quality_flags: QualityFlagConfig,
}

// One implementation, in libs/axon-config, re-exported under the name this
// module's call sites already use. comms was the last capability still carrying
// its own copies of these helpers.
pub(crate) use axon_config::expand_tilde;

/// Resolve an API-key reference without ever storing or logging its value.
/// JSON files use `.auth.api_key` (the oMLX settings shape); non-JSON files use
/// their trimmed contents. Parsed JSON without that field deliberately has no
/// raw-content fallback.
pub(crate) fn api_key_from_file(path: Option<&str>) -> Option<String> {
    let path = expand_tilde(path?);
    let content = std::fs::read_to_string(path).ok()?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(trimmed) {
        return json
            .get("auth")
            .and_then(|auth| auth.get("api_key"))
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .map(str::to_string);
    }
    Some(trimmed.to_string())
}

/// `"22-7"` -> `(22, 7)`. Returns `None` for anything it does not fully
/// understand, and `None` means "no quiet window" — a misconfigured string
/// must not silently become a window that suppresses every run instead.
fn parse_quiet_hours(value: &str) -> Option<(u32, u32)> {
    let (start, end) = value.trim().split_once('-')?;
    let start: u32 = start.trim().parse().ok()?;
    let end: u32 = end.trim().parse().ok()?;
    (start < 24 && end < 24 && start != end).then_some((start, end))
}

fn config_path() -> PathBuf {
    if let Ok(p) = std::env::var("AXON_COMMS_CONFIG") {
        return expand_tilde(&p);
    }
    if let Some(p) = axon_config::overlay_config("comms.json") {
        return p;
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("comms.config.json")
}

/// Masks the password in a connection string for safe display. Never print
/// `Config::database_url` directly -- it's a live credential.
///
/// Kept as a named wrapper so this crate's call sites stay stable; the
/// implementation is `axon_config::redact_dsn`. The copy that used to live here
/// took the FIRST `@` when redacting the URL form, which printed the tail of any
/// password containing an `@`. The shared one uses `rfind` and has a test
/// pinning that case.
pub fn redact_database_url(url: &str) -> String {
    axon_config::redact_dsn(url)
}

fn default_google_env_path() -> PathBuf {
    axon_config::overlay_config("comms.env").unwrap_or_else(|| PathBuf::from("comms.env"))
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

impl Config {
    pub fn load() -> Self {
        let file = load_file_config();

        // Fallback is keyword/value form, not the `postgresql://` URL this used to
        // build: the URL userinfo form mangles a base64 password containing `/`, and
        // the sibling capabilities all fall back the same way.
        let database_url = file.database_url.unwrap_or_else(|| {
            axon_config::postgres_conn_from_shared_env().unwrap_or_else(|| {
                "host=127.0.0.1 port=5432 user=axon password=axon dbname=axon".into()
            })
        });

        let google_env_path = file
            .google_env_path
            .map(|p| expand_tilde(&p))
            .unwrap_or_else(default_google_env_path);

        // The runner's port contract, resolved in one place for every capability.
        // No capability-specific escape-hatch env var here: comms never had one.
        let port = axon_config::resolve_port(None, file.port, 8083);
        let api_secret = api_key_from_file(file.api_secret_file.as_deref());
        let dashboard_origin = file
            .dashboard_origin
            .unwrap_or_else(|| "http://127.0.0.1:47117".into());
        let enrichment_drain_minutes = file.enrichment_drain_minutes.unwrap_or(15);
        let digest_drain_minutes = file.digest_drain_minutes.unwrap_or(15);
        let gmail_maintenance_minutes = file.gmail_maintenance_minutes.unwrap_or(15);
        let inbox_sweep_minutes = file.inbox_sweep_minutes.unwrap_or(0);
        let inbox_sweep_max_threads = file.inbox_sweep_max_threads.unwrap_or(25).clamp(1, 100);
        let inbox_sweep_quiet_hours = file
            .inbox_sweep_quiet_hours
            .as_deref()
            .and_then(parse_quiet_hours);
        let inference = InferenceConfig::load(axon_config::overlay_config);
        let keeper_export_dir = file.keeper_export_dir.map(|p| expand_tilde(&p));
        let mut relevance = file.relevance.unwrap_or_default();
        relevance.profile_paths = relevance
            .profile_paths
            .into_iter()
            .map(|path| expand_tilde(&path).to_string_lossy().into_owned())
            .collect();
        let vault_link_sources = file
            .vault_link_sources
            .into_iter()
            .map(|mut source| {
                source.path = expand_tilde(&source.path).to_string_lossy().into_owned();
                source
            })
            .collect();
        let feed_sources = file.feed_sources.unwrap_or_else(default_feed_sources);
        let travel_context = file.travel_context.unwrap_or_default();
        let calendar_context = file.calendar_context.unwrap_or_default();
        let quality_flags = file.quality_flags.unwrap_or_default();

        Self {
            database_url,
            google_env_path,
            port,
            api_secret,
            dashboard_origin,
            enrichment_drain_minutes,
            digest_drain_minutes,
            gmail_maintenance_minutes,
            inbox_sweep_minutes,
            inbox_sweep_max_threads,
            inbox_sweep_quiet_hours,
            inference,
            rules: file.rules,
            keeper_export_dir,
            relevance,
            travel_context,
            calendar_context,
            vault_link_sources,
            feed_sources,
            quality_flags,
        }
    }

    pub fn embedding_role(&self) -> Option<ResolvedRole> {
        self.inference.role("embedding")
    }

    pub fn reranking_role(&self) -> Option<ResolvedRole> {
        self.inference.role("reranking")
    }

    pub fn summarization_role(&self) -> Option<ResolvedRole> {
        self.inference.role("summarization")
    }

    /// A smaller, faster local model for the cheap rungs, if this machine has
    /// one configured.
    ///
    /// Optional by design: a machine with only `summarization` behaves exactly
    /// as it did before this existed. Where it earns its place is a host with a
    /// second local model that is quick but small — Apple's on-device model is
    /// 4,096 tokens and answers a Brief digest in about two seconds against
    /// twelve to twenty for the 26B — because moving the short rungs onto it
    /// takes them off the shared GPU budget entirely.
    ///
    /// The role must declare `max_input_tokens`. Without it there is no way to
    /// tell whether a source fits, and guessing is how you get a context error
    /// instead of a digest, so an undeclared window means the role is skipped.
    pub fn light_summarization_role(&self) -> Option<ResolvedRole> {
        self.inference
            .role("summarization_light")
            .filter(|role| role.max_input_tokens.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_tilde_uses_home() {
        std::env::set_var("HOME", "/tmp/fake-home");
        assert_eq!(expand_tilde("~/foo"), PathBuf::from("/tmp/fake-home/foo"));
        assert_eq!(expand_tilde("/abs/path"), PathBuf::from("/abs/path"));
    }

    #[test]
    fn quiet_hours_parse_or_decline() {
        assert_eq!(parse_quiet_hours("22-7"), Some((22, 7)));
        assert_eq!(parse_quiet_hours(" 0 - 6 "), Some((0, 6)));
        // Anything unparseable means no window, never an all-day one — the
        // failure mode of the opposite choice is a schedule that silently
        // never runs and looks identical to a broken connector.
        for bad in ["", "22", "22-24", "9-9", "night", "22-7-3"] {
            assert_eq!(parse_quiet_hours(bad), None, "{bad} should not parse");
        }
    }

    #[test]
    fn redact_database_url_hides_password_only() {
        assert_eq!(
            redact_database_url("postgresql://axon:s3cr3t@127.0.0.1:5432/axon"),
            "postgresql://axon:***@127.0.0.1:5432/axon"
        );
        assert_eq!(redact_database_url("not-a-url"), "not-a-url");
    }

    #[test]
    fn redact_database_url_hides_keyword_value_password() {
        // The env-derived form. Password with base64 special chars must not leak.
        assert_eq!(
            redact_database_url("host=127.0.0.1 port=5432 user=axon password=Kw+/z= dbname=axon"),
            "host=127.0.0.1 port=5432 user=axon password=*** dbname=axon"
        );
        // password= as the trailing token (no space after) is also redacted.
        assert_eq!(
            redact_database_url("host=h user=u password=secret"),
            "host=h user=u password=***"
        );
    }

    #[test]
    fn relevance_config_only_owns_profile_paths() {
        let relevance = RelevanceConfig::default();
        assert!(relevance.profile_paths.is_empty());
    }

    /// Restores an env var on drop. Rust runs a crate's tests as threads of ONE
    /// process, so `remove_var` here is not local to this test: unrestored, it
    /// left every later store test resolving the fallback connection string
    /// instead of the overlay's real one, and eight of them failed against a
    /// perfectly healthy Postgres.
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
        let _config = EnvGuard::take("AXON_COMMS_CONFIG");
        let _overlay = EnvGuard::take("AXON_PERSONAL_ROOT");
        let cfg = Config::load();
        assert_eq!(cfg.port, 8083);
        assert!(cfg.rules.is_empty());
        assert!(cfg.inference.roles.is_empty());
        assert!(cfg.relevance.profile_paths.is_empty());
        assert!(cfg.vault_link_sources.is_empty());
        assert_eq!(cfg.feed_sources.len(), 2);
        assert_eq!(cfg.quality_flags.minimum_total_retention_percent, 39.0);
        assert_eq!(cfg.quality_flags.maximum_total_retention_percent, 90.0);
        assert_eq!(cfg.quality_flags.maximum_boilerplate_leakage_percent, 0.0);
        assert_eq!(cfg.quality_flags.summary_attempt_warning, 2);
    }
}
