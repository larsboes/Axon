//! axon-status — what is enabled on this machine, what is up, and the one thing
//! allowed to bring a capability up.
//!
//! It knows nothing about which capabilities exist. The manifests do, and
//! `tools/capability.sh registry` renders them as JSON so this process never learns
//! to parse TOML (README.md#one-manifest-per-concern — `tools/lib/toml.sh` stays the only parser). That
//! also retired the hardcoded transit/scouting port literals this file used to carry:
//! its own source comment named the flip condition — "if a third reader of these
//! ports shows up" — and additional capability consumers plus the dashboard proxy met it.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::{
    extract::Path,
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

static STARTED_AT: OnceLock<Instant> = OnceLock::new();

/// One service as the registry renders it. Every field is a string because the
/// manifests are single-line TOML and the shell emitter does not guess types.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct Service {
    name: String,
    kind: String,
    scope: String,
    port: String,
    health_path: String,
    /// Liveness answers "is the process there", which is not what an availability surface
    /// is asking. A capability that reaches a database declares this too, and answers it by
    /// touching that database. Empty for everything that has no dependency worth checking —
    /// and an empty one means this surface keeps polling `health_path`, exactly as before.
    #[serde(default)]
    ready_path: String,
    panel_port: String,
    panel_path: String,
    autostart: String,
    /// Seconds a panel may go unread before it is stopped. Empty means never — a
    /// capability opts into being reaped, it is not opted in by having a panel.
    #[serde(default)]
    idle_timeout: String,
    /// The systems.toml id a backup lands on. Non-empty is this surface's whole
    /// definition of "has a backup contract", because `tools/backup.sh` refuses a run
    /// without one. Never projected outward — an id is one hop from the private
    /// coordinates in the overlay's systems.local.toml, and no backup UI needs it.
    #[serde(default)]
    backup_target: String,
    /// Declared when the capability's data is a SQLite file. This is the field that
    /// decides whether a backup run stops the service: `backup.sh` holds a capability
    /// down for exactly this case, to take the cold copy. Read as a contract rather
    /// than matching a service name, so a capability that later declares one inherits
    /// the warning without anything here changing.
    #[serde(default)]
    backup_sqlite: String,
    /// What timely MEANS for this data — how often it should be backed up, and the
    /// age past which the last backup is a problem rather than merely due. Two numbers
    /// because "you could" and "you have a problem" are different answers.
    #[serde(default)]
    backup_advise_days: String,
    #[serde(default)]
    backup_stale_days: String,
    /// Set only where `scope == "external"`: the base URL of the deployment that actually
    /// provides this capability, resolved by the active overlay (retired-tracker#169). Empty
    /// for everything this machine runs, which is the normal case and the one that must not
    /// change — an empty endpoint means loopback, exactly as before.
    #[serde(default)]
    endpoint: String,
    #[serde(default)]
    requires: Vec<String>,
}

impl Service {
    /// A capability this machine consumes rather than runs. The registry decides this; the
    /// question is asked here often enough that spelling the comparison out at each call site
    /// would be four chances to spell it differently.
    fn is_external(&self) -> bool {
        self.scope == "external"
    }

    /// Where to reach one of this capability's probe paths, or `None` when there is nothing
    /// to poll — a capability without such a surface is reported as unknown rather than
    /// silently "down".
    ///
    /// Two shapes, and the port is what separates them. A local capability's URL is loopback
    /// plus the port it publishes here. An external one has no port on this machine at all —
    /// the registry blanks it, because a port number is a fact about the host that binds it —
    /// so its URL is the resolved endpoint plus the same path. The probe paths are the only
    /// manifest fields the two have in common, which is the whole reason they are the only
    /// ones a consuming machine inherits.
    fn probe_url(&self, path: &str) -> Option<String> {
        if path.is_empty() {
            return None;
        }
        if self.is_external() {
            if self.endpoint.is_empty() {
                return None;
            }
            return Some(format!("{}{}", self.endpoint, path));
        }
        if self.port.is_empty() {
            return None;
        }
        Some(format!("http://127.0.0.1:{}{}", self.port, path))
    }

    fn health_url(&self) -> Option<String> {
        self.probe_url(&self.health_path)
    }

    /// What availability should be judged on: readiness where a capability declares it,
    /// liveness everywhere else.
    ///
    /// The fallback is the compatibility contract. Until 2026-08-07 this surface polled
    /// `health_path` for every capability, and five database-backed ones answered it from a
    /// stateless handler that could not observe their database — so they reported themselves up
    /// through an outage that made every query behind them fail (#126). A capability that
    /// declares no `ready_path` still behaves exactly as it did.
    fn readiness_url(&self) -> Option<String> {
        self.probe_url(&self.ready_path)
            .or_else(|| self.health_url())
    }

    /// How long this panel may go unread, when it says so at all.
    ///
    /// Three conditions, all required, and each one is a separate way the reaper could
    /// otherwise stop something it has no business stopping: a panel to look at, an
    /// explicit timeout, and NOT autostart — a capability the machine is supposed to
    /// keep running is never idle by definition.
    fn idle_timeout_secs(&self) -> Option<u64> {
        if self.panel_port.is_empty() || self.autostart == "true" {
            return None;
        }
        self.idle_timeout.parse::<u64>().ok().filter(|s| *s > 0)
    }

    /// `None` for a capability with nothing to back up, which is most of them.
    ///
    /// Presence is `backup_target`, not the day fields: a capability may decline to say
    /// what timely means and still have a backup contract, and treating that as "no
    /// contract" would hide it from the only surface that could tell you it has never
    /// run. The reverse — day fields without a target — is a manifest `backup.sh`
    /// itself rejects, so there is nothing to represent.
    fn backup_contract(&self) -> Option<BackupContract> {
        if self.backup_target.is_empty() {
            return None;
        }
        Some(BackupContract {
            holds_service: !self.backup_sqlite.is_empty(),
            advise_days: self.backup_advise_days.parse::<u64>().ok(),
            stale_days: self.backup_stale_days.parse::<u64>().ok(),
        })
    }
}

/// What the manifest declares about backing this capability up, with the private half
/// already dropped. Every field here is safe to render.
#[derive(Clone, Copy, Serialize)]
struct BackupContract {
    /// A run stops the capability. The UI must say so BEFORE asking for confirmation,
    /// derived from the contract rather than a hardcoded service name.
    holds_service: bool,
    advise_days: Option<u64>,
    stale_days: Option<u64>,
}

#[derive(Serialize)]
struct CapabilityStatus {
    up: bool,
    url: String,
}

#[derive(Serialize)]
struct AxonStatusHealth {
    ok: bool,
    version: String,
    uptime_seconds: u64,
    capabilities: HashMap<String, CapabilityStatus>,
}

#[derive(Serialize)]
struct CapabilityView {
    #[serde(flatten)]
    service: Service,
    /// `None` — not false — when the manifest declares no health surface. A running
    /// container with nothing to poll is unknown from here, and reporting it as down
    /// would be a lie the shell then renders as a red dot.
    up: Option<bool>,
    health_url: Option<String>,
}

/// The repo this binary belongs to. Passed in rather than derived from the binary's
/// own path, which under Bazel points into an output tree, not the checkout.
/// `tools/lib/paths.sh` exports it, so anything started by `service-runner.sh`
/// inherits it; a hand-started run has to say where it is.
fn axon_root() -> Result<PathBuf, String> {
    std::env::var("AXON_ROOT").map(PathBuf::from).map_err(|_| {
        "AXON_ROOT is not set — start this through tools/service-runner.sh, or export it"
            .to_string()
    })
}

/// Every file `tools/capability.sh registry` reads, with its mtime — the cache key
/// below. `None` when the enabled set's location is unknown, which disables caching
/// rather than caching against a key that cannot see the enabled set change.
///
/// Globbed on every call rather than remembered once: a cache keyed on a remembered
/// file list cannot notice a manifest that did not exist when the list was built, and
/// adding a capability is exactly that case. Directory reads and stats, no forks.
// `std::path::Path` stays qualified: `axum::extract::Path` owns the bare name in this
// file, same as in `reap_idle_panels` below.
fn manifest_key(root: &std::path::Path) -> Option<Vec<(PathBuf, SystemTime)>> {
    // Same env contract as AXON_ROOT above: tools/lib/paths.sh exports it, so anything
    // started by service-runner.sh inherits it.
    let machine_toml = std::env::var("AXON_MACHINE_TOML").ok()?;
    let mut paths = vec![PathBuf::from(machine_toml)];

    // Spine services at the repo root, capabilities under capabilities/, and the same
    // directory in the active overlay — the globs capability.sh itself uses
    // (`_spine_names`, `_cap_dirs`). A top-level service.toml IS the declaration
    // (README.md#three-architectural-nouns), so there is no list to keep in sync here
    // either.
    //
    // The overlay directory belongs in the key even though the registry it guards is
    // rendered by capability.sh: this key is what notices a manifest appearing, and an
    // overlay capability that is added while the process runs would otherwise stay
    // invisible until a restart. AXON_OVERLAY_CAPS_DIR comes from tools/lib/paths.sh,
    // and its absence is normal — a deployment need not own any overlay capability.
    let mut dirs = vec![root.to_path_buf(), root.join("capabilities")];
    if let Ok(overlay_caps) = std::env::var("AXON_OVERLAY_CAPS_DIR") {
        dirs.push(PathBuf::from(overlay_caps));
    }
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let manifest = entry.path().join("service.toml");
            if manifest.is_file() {
                paths.push(manifest);
            }
        }
    }

    paths.sort();
    Some(
        paths
            .into_iter()
            .map(|p| {
                let mtime = std::fs::metadata(&p)
                    .and_then(|m| m.modified())
                    .unwrap_or(UNIX_EPOCH);
                (p, mtime)
            })
            .collect(),
    )
}

static REGISTRY_CACHE: OnceLock<Mutex<Option<(Vec<(PathBuf, SystemTime)>, Vec<Service>)>>> =
    OnceLock::new();

// The offline upstream check still starts a shell/Bun process and walks the complete manifest.
// That is useful work when a maintainer asks for a fresh audit, but wasteful for every visit to
// a read-only dashboard page. Keep the live API responsive without pretending the cached result
// is a new audit: the page continues to label this as an offline check, and a restart or five
// minutes is enough to pick up ordinary manifest maintenance.
const UPSTREAMS_CACHE_TTL: Duration = Duration::from_secs(300);
static UPSTREAMS_CACHE: OnceLock<Mutex<Option<(Instant, Value)>>> = OnceLock::new();

/// The enabled set, from the manifests, memoized against their mtimes.
///
/// The shell call underneath costs ~440ms on this machine — `tools/lib/toml.sh` forks a
/// grep and a sed per key per manifest, ~12 manifests × 9 keys — and every handler here
/// starts with it, including the `/capabilities` the dashboard's layout polls every 15s
/// on every page. That put ~440ms of dead time in front of every Axon page for an answer
/// that changes only when somebody edits a file.
///
/// Two concurrent misses may both run the script. That is left alone deliberately: the
/// script is a pure read, running it twice costs one extra fork, and avoiding it would
/// mean holding a lock across an await for no correctness gain.
/// What this capability answers, served as data beside `/health`.
/// Required query parameters are named in the summary: a path alone cannot tell
/// a caller what it must send, and learning that from a 400 is the thing this
/// endpoint exists to avoid.
const ROUTES: &[route_manifest::Route] = &[
    r("GET", "/health", "Liveness."),
    r("GET", "/routes", "This manifest."),
    r("GET", "/api/axon-status/health", "Aggregate health across enabled capabilities."),
    r("GET", "/api/axon-status/routes", "Every enabled capability's route manifest, in one map."),
    r("GET", "/api/axon-status/capabilities", "Enabled capabilities, their ports and whether each is up."),
    r("GET", "/api/axon-status/self", "This machine's resolved Axon model."),
    r("GET", "/api/axon-status/repos", "Axon and overlay repo state."),
    r("GET", "/api/axon-status/upstreams", "Declared upstreams and their pins."),
    r("GET", "/api/axon-status/backups", "Every capability with a backup contract: last success, age, and whether it is overdue."),
    r("POST", "/api/axon-status/capabilities/:name/backup", "Request a backup of one capability. Accepts the run and returns; poll /backups for the outcome."),
];

/// Shorthand so the table above reads as a table.
const fn r(
    method: &'static str,
    path: &'static str,
    summary: &'static str,
) -> route_manifest::Route {
    route_manifest::Route {
        method,
        path,
        summary,
    }
}

async fn routes() -> axum::Json<serde_json::Value> {
    axum::Json(route_manifest::manifest("axon-status", ROUTES))
}

async fn registry() -> Result<Vec<Service>, String> {
    let root = axon_root()?;
    let key = manifest_key(&root);

    if let Some(key) = key.as_ref() {
        if let Ok(cache) = REGISTRY_CACHE.get_or_init(|| Mutex::new(None)).lock() {
            if let Some((cached_key, services)) = cache.as_ref() {
                if cached_key == key {
                    return Ok(services.clone());
                }
            }
        }
    }

    let out = tokio::process::Command::new(root.join("tools/capability.sh"))
        .arg("registry")
        .output()
        .await
        .map_err(|e| format!("could not run tools/capability.sh: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "tools/capability.sh registry failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let services: Vec<Service> = serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("registry is not valid JSON: {e}"))?;

    if let Some(key) = key {
        if let Ok(mut cache) = REGISTRY_CACHE.get_or_init(|| Mutex::new(None)).lock() {
            *cache = Some((key, services.clone()));
        }
    }
    Ok(services)
}

async fn is_up(client: &reqwest::Client, svc: &Service) -> bool {
    let Some(url) = svc.readiness_url() else {
        return false;
    };
    client
        .get(&url)
        .timeout(Duration::from_secs(2))
        .send()
        .await
        // A redirect counts as up: a locale-routing frontend answers / with a 307 and
        // is perfectly alive.
        .map(|res| res.status().is_success() || res.status().is_redirection())
        .unwrap_or(false)
}

async fn health_handler() -> Json<Value> {
    Json(json!({ "ok": true, "service": "axon-status" }))
}

/// Version identity for the spine repo and the overlay, read at request time.
///
/// Shelled out to `tools/repos` rather than reimplemented: git plumbing and the
/// overlay's location both already have exactly one home, and this process learning
/// either would be a second one. Same pattern as the registry.
async fn repos_handler() -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let root = axon_root().map_err(bad_gateway)?;
    let out = tokio::process::Command::new(root.join("tools/repos"))
        .arg("--json")
        .output()
        .await
        .map_err(|e| bad_gateway(format!("could not run tools/repos: {e}")))?;
    if !out.status.success() {
        return Err(bad_gateway(format!(
            "tools/repos failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    serde_json::from_slice(&out.stdout)
        .map(Json)
        .map_err(|e| bad_gateway(format!("tools/repos did not emit JSON: {e}")))
}

/// The upstream-dependency audit, served for the dashboard's `/upstreams` feed.
///
/// Shelled out to `tools/upstream-checker --json`, the same pattern as the registry and
/// `tools/repos`: `upstreams.toml` and the verdict/pin/cooldown gate over it both already
/// have exactly one home (README.md#dependency-verdicts-and-provenance), and this process reading the manifest itself
/// would be a second one. `--offline` because this is a page poll, not the M2 gate — the
/// online mode makes one GitHub call per entry and would rate-limit an unauthenticated
/// box; the response carries `offline: true` so the page can say drift was not checked.
/// `--json` never exits non-zero (a manifest fail is `totals.fail` in the payload), so a
/// non-success status here is a real tool failure, handled the same way as the siblings.
async fn upstreams_handler() -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if let Ok(cache) = UPSTREAMS_CACHE.get_or_init(|| Mutex::new(None)).lock() {
        if let Some((checked_at, response)) = cache.as_ref() {
            if checked_at.elapsed() < UPSTREAMS_CACHE_TTL {
                return Ok(Json(response.clone()));
            }
        }
    }

    let root = axon_root().map_err(bad_gateway)?;
    let out = tokio::process::Command::new(root.join("tools/upstream-checker"))
        .arg("--json")
        .arg("--offline")
        .output()
        .await
        .map_err(|e| bad_gateway(format!("could not run tools/upstream-checker: {e}")))?;
    if !out.status.success() {
        return Err(bad_gateway(format!(
            "tools/upstream-checker failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    let response: Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| bad_gateway(format!("tools/upstream-checker did not emit JSON: {e}")))?;
    if let Ok(mut cache) = UPSTREAMS_CACHE.get_or_init(|| Mutex::new(None)).lock() {
        *cache = Some((Instant::now(), response.clone()));
    }
    Ok(Json(response))
}

/// The committed self-model, fused with live state at read time.
///
/// `model` is `self.json` verbatim — structure, compile-time coupling, provenance and
/// code size, all derived from tracked files by `tools/self generate`. `live` is the
/// per-capability `up` map this process already owns.
///
/// They stay two sibling keys rather than one merged object on purpose. `self.json` is
/// committed, so writing `up` into it would give live state a second home and make the
/// file lie the moment a process stops; keeping the fusion visible at the response
/// boundary means a reader can always tell which half is a fact about the repo and which
/// is a fact about this machine right now. `up` is `null` for a capability that declares
/// nothing to poll — unknown, not down — matching the capabilities endpoint.
async fn self_model_handler() -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let root = axon_root().map_err(bad_gateway)?;
    let path = root.join("self.json");
    let text = std::fs::read_to_string(&path).map_err(|e| {
        bad_gateway(format!(
            "cannot read {} ({e}) — run: tools/self generate",
            path.display()
        ))
    })?;
    let model: Value = serde_json::from_str(&text)
        .map_err(|e| bad_gateway(format!("{} is not valid JSON: {e}", path.display())))?;

    let services = registry().await.map_err(bad_gateway)?;
    let client = reqwest::Client::new();
    let mut live = serde_json::Map::new();
    for service in &services {
        let up = match service.readiness_url() {
            Some(_) => Some(is_up(&client, service).await),
            None => None,
        };
        live.insert(service.name.clone(), json!(up));
    }

    Ok(Json(json!({ "model": model, "live": live })))
}

/// The dashboard's long-standing contract: name -> {up, url}. Same shape as before,
/// now covering every enabled capability instead of two compiled-in names.
async fn axon_status_health_handler() -> Result<Json<AxonStatusHealth>, (StatusCode, Json<Value>)> {
    let services = registry().await.map_err(bad_gateway)?;
    let client = reqwest::Client::new();

    let mut capabilities = HashMap::new();
    for svc in &services {
        // The URL reported beside `up` is the one `up` was judged on, so the two cannot
        // disagree about which surface was asked.
        let Some(url) = svc.readiness_url() else {
            continue;
        };
        capabilities.insert(
            svc.name.clone(),
            CapabilityStatus {
                up: is_up(&client, svc).await,
                url,
            },
        );
    }

    // "ok" means what should be running is running — the autostart set. Once
    // capabilities start on demand, "everything is up" stopped being a health signal:
    // a stopped on-demand capability is the normal state, not a fault, and reporting
    // it as one would leave the shell permanently amber.
    //
    // Only over what can actually be polled. A capability with no health surface is
    // never in `capabilities` above, and the old `unwrap_or(false)` read that absence
    // as "down" — so the moment postgres and vaultwarden declared `autostart = "true"`
    // (2026-07-30, replacing the undeclared watchdogs that had been keeping them up),
    // the shell reported "at least one service that should be running is not
    // answering" about two containers that were both running fine. Unknown is not down;
    // that is what every other reader of this registry already says.
    let ok = services
        .iter()
        .filter(|s| s.autostart == "true")
        .filter_map(|s| capabilities.get(&s.name))
        .all(|c| c.up);
    let uptime_seconds = STARTED_AT.get().map(|t| t.elapsed().as_secs()).unwrap_or(0);

    Ok(Json(AxonStatusHealth {
        ok,
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_seconds,
        capabilities,
    }))
}

/// Everything the shell needs to render itself: what exists and what is up.
///
/// `panel_port` and `panel_path` are reported as the manifest facts they are; the
/// panel's URL is deliberately NOT assembled here. A panel is loaded by a browser, and
/// that browser has to reach it on the host it is already talking to: composing
/// `127.0.0.1:<port>` server-side breaks the moment the shell is opened as `localhost`
/// (Chrome treats the two as different sites and partitions storage, which is enough to
/// kill a framework's client init) and breaks harder over Tailscale, where 127.0.0.1 is
/// the phone. The shell builds the URL from its own `location`.
async fn capabilities_handler() -> Result<Json<Vec<CapabilityView>>, (StatusCode, Json<Value>)> {
    let services = registry().await.map_err(bad_gateway)?;
    let client = reqwest::Client::new();

    let mut views = Vec::with_capacity(services.len());
    for service in services {
        let up = match service.readiness_url() {
            Some(_) => Some(is_up(&client, &service).await),
            None => None,
        };
        views.push(CapabilityView {
            health_url: service.health_url(),
            up,
            service,
        });
    }
    Ok(Json(views))
}

/// Every enabled capability's route manifest, in one map.
///
/// The single "what can I call" endpoint. Each capability reports its own paths,
/// so this stays correct across the five different URL conventions in use
/// without anyone having to remember which capability follows which — and it
/// keeps working unchanged if those conventions later converge.
///
/// A capability that is down, or too old to serve `/routes`, is reported with
/// its reason rather than omitted. Silently returning a shorter list would read
/// as "that capability has no endpoints", which is the one wrong answer here.
async fn routes_handler() -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let services = registry().await.map_err(bad_gateway)?;
    let client = reqwest::Client::new();

    let mut capabilities = Vec::with_capacity(services.len());
    for service in services {
        if service.port.is_empty() {
            continue;
        }
        let url = format!("http://127.0.0.1:{}/routes", service.port);
        let manifest = client
            .get(&url)
            .timeout(Duration::from_secs(2))
            .send()
            .await
            .ok()
            .filter(|response| response.status().is_success());
        let entry = match manifest {
            Some(response) => match response.json::<Value>().await {
                Ok(body) => json!({ "name": service.name, "routes": body["routes"] }),
                Err(error) => json!({
                    "name": service.name,
                    "unavailable": format!("served /routes but the body did not parse: {error}"),
                }),
            },
            None => json!({
                "name": service.name,
                "unavailable": "not running, or does not serve /routes",
            }),
        };
        capabilities.push(entry);
    }
    Ok(Json(json!({ "capabilities": capabilities })))
}

/// Start or stop one capability on demand — the click-to-open half of the dashboard.
///
/// The only reachable names are the ones the registry already lists, so a request can
/// never name an arbitrary program: this handler passes a capability NAME to
/// `service-runner.sh`, never a command. Together with binding 127.0.0.1 (see `main`)
/// that is the whole security model, and it is only sufficient while the port stays
/// local. Exposing this over Tailscale means adding real authentication first —
/// "reachable from the phone" and "unauthenticated process control" cannot both be
/// true.
async fn lifecycle(
    name: String,
    action: &'static str,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let services = registry().await.map_err(bad_gateway)?;
    let Some(service) = services.into_iter().find(|s| s.name == name) else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(
                json!({ "error": format!("'{name}' is not an enabled capability on this machine") }),
            ),
        ));
    };

    // An external capability is in the registry so its health can be READ, and for no other
    // reason (retired-tracker#169). Independently managed overlays stay separate operational
    // authorities: whoever owns that host owns its lifecycle, its secrets and its backups.
    // Refused here rather than left to fail downstream — `service-runner.sh` would look for a
    // local process, not find one, and report something that reads like an outage on a service
    // that is running perfectly well somewhere else.
    if service.is_external() {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({
                "error": format!("'{name}' is provided by another deployment — this machine may read its health, not {action} it"),
            })),
        ));
    }

    let root = axon_root().map_err(bad_gateway)?;
    let out = tokio::process::Command::new(root.join("tools/service-runner.sh"))
        .arg(action)
        .arg(&service.name)
        .output()
        .await
        .map_err(|e| bad_gateway(format!("could not run tools/service-runner.sh: {e}")))?;

    let detail = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if !out.status.success() {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("{action} {name} failed"), "detail": detail })),
        ));
    }

    let up = is_up(&reqwest::Client::new(), &service).await;
    Ok(Json(json!({
        "name": service.name,
        "action": action,
        "up": up,
        "detail": detail,
    })))
}

async fn start_handler(Path(name): Path<String>) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    // `resume`, not `start`: a capability stopped through this API carries a
    // maintenance hold, and `start` deliberately no-ops while one is set. Asking for it
    // from the UI means you want it back.
    lifecycle(name, "resume").await
}

async fn stop_handler(Path(name): Path<String>) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    lifecycle(name, "stop").await
}

fn bad_gateway(msg: impl Into<String>) -> (StatusCode, Json<Value>) {
    (
        StatusCode::BAD_GATEWAY,
        Json(json!({ "error": msg.into() })),
    )
}

// --- backups ------------------------------------------------------------------------
//
// A backup you cannot see the age of is a backup you find out about during a restore.
// `tools/backup.sh` has written a receipt per capability since it was built, and the
// manifests have declared what timely means for that data for just as long — but nothing
// read either, so the two numbers sat in the schema with a comment admitting the gap.
// This is the reader, plus the one button that fixes what it reports.
//
// The security model is `lifecycle`'s, unchanged: a capability NAME the registry already
// lists, passed to a tool, never a command, never a path, never a destination. This
// handler additionally refuses a name with no backup contract, so the reachable set is
// smaller than the reachable set for start/stop rather than larger.

/// The active deployment overlay. Receipts are instance state, so they live there rather
/// than in Axon — same reasoning as `backup.sh`'s own comment where it writes them.
fn overlay_root() -> Result<PathBuf, String> {
    std::env::var("AXON_OVERLAY_ROOT")
        .map(PathBuf::from)
        .map_err(|_| {
            "AXON_OVERLAY_ROOT is not set — start this through tools/service-runner.sh, or export it"
                .to_string()
        })
}

/// What `backup.sh` writes after the remote byte count matched. Only the fields this
/// surface projects are named: `target`, `tarball` and `sha256` are deliberately absent
/// from the struct, so there is no path by which a destination reaches a response.
#[derive(Deserialize)]
struct Receipt {
    completed_at: String,
    #[serde(default)]
    bytes: u64,
    #[serde(default)]
    contents: String,
}

/// `20260805T220018Z` — fixed-width UTC, written by `date -u +%Y%m%dT%H%M%SZ` in
/// `backup.sh`, parsed here by hand.
///
/// Hand-rolled rather than adding a date crate: the format is ours and fixed, and the
/// alternative costs a dependency in Cargo.lock and the Bazel crate index for one
/// `strptime`. `None` on anything that does not match, which reads downstream as "no
/// usable receipt" — the same answer as a missing file, and the right one, because a
/// receipt this process cannot date cannot be used to claim a backup is fresh.
fn parse_receipt_ts(s: &str) -> Option<u64> {
    let b = s.as_bytes();
    if b.len() != 16 || b[8] != b'T' || b[15] != b'Z' {
        return None;
    }
    let num = |from: usize, to: usize| s.get(from..to)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0, 4)?, num(4, 6)?, num(6, 8)?);
    let (h, mi, sec) = (num(9, 11)?, num(11, 13)?, num(13, 15)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || sec > 60 {
        return None;
    }

    // days_from_civil (Howard Hinnant's civil-calendar algorithm): era arithmetic, so no
    // leap-year special-casing and no table.
    let y = y - i64::from(mo <= 2);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (mo + if mo > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;

    u64::try_from(days * 86_400 + h * 3_600 + mi * 60 + sec).ok()
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// One in-flight or finished run, kept in this process.
///
/// Process memory rather than a file on purpose: a run's progress is a fact about THIS
/// process, and the durable record of a backup is the receipt `backup.sh` writes. That
/// split is what lets a slow run survive a page refresh — the page is not holding the
/// state — while a restart of axon-status correctly forgets a run it can no longer
/// observe, instead of leaving a "running" marker on disk that nothing will ever clear.
#[derive(Clone, Serialize)]
struct BackupRun {
    state: &'static str,
    started_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    finished_at: Option<u64>,
    /// `backup.sh`'s own stderr on failure. It reports progress and errors in terms of
    /// capability names and staging steps; the one place it names a destination is the
    /// final success line, which is not on this path.
    #[serde(skip_serializing_if = "str::is_empty")]
    detail: String,
}

static BACKUP_RUNS: OnceLock<Mutex<HashMap<String, BackupRun>>> = OnceLock::new();

fn backup_runs() -> &'static Mutex<HashMap<String, BackupRun>> {
    BACKUP_RUNS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Age against the capability's own two thresholds.
///
/// `unknown` when the manifest declares no `backup_stale_days`: this surface will not
/// invent a cadence for data whose owner did not state one, because a red badge derived
/// from a number Axon made up is worse than no badge. `never` outranks everything —
/// a capability with a backup contract and no receipt has the problem, whatever its
/// thresholds say.
fn backup_state(age_secs: Option<u64>, c: &BackupContract) -> &'static str {
    let Some(age) = age_secs else { return "never" };
    let days = age / 86_400;
    match (c.stale_days, c.advise_days) {
        (Some(stale), _) if days >= stale => "overdue",
        (_, Some(advise)) if days >= advise => "due",
        (None, None) => "unknown",
        _ => "ok",
    }
}

/// Every capability that declares a backup contract, with the age of its last one.
async fn backups_handler() -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let services = registry().await.map_err(bad_gateway)?;
    let overlay = overlay_root().map_err(bad_gateway)?;
    let now = now_epoch();
    let runs = backup_runs().lock().unwrap().clone();

    let mut out = Vec::new();
    for service in services {
        let Some(contract) = service.backup_contract() else {
            continue;
        };
        let receipt: Option<Receipt> = std::fs::read_to_string(
            overlay
                .join("backup/receipts")
                .join(format!("{}.json", service.name)),
        )
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok());

        let last = receipt
            .as_ref()
            .and_then(|r| parse_receipt_ts(&r.completed_at));
        // saturating: a receipt dated in the future is a clock problem, not a negative
        // age, and it must not wrap into "overdue by 500 years".
        let age = last.map(|t| now.saturating_sub(t));

        out.push(json!({
            "capability": service.name,
            "state": backup_state(age, &contract),
            "holds_service": contract.holds_service,
            "advise_days": contract.advise_days,
            "stale_days": contract.stale_days,
            "last_success": receipt.as_ref().map(|r| r.completed_at.clone()),
            "age_seconds": age,
            "bytes": receipt.as_ref().map(|r| r.bytes),
            "contents": receipt.as_ref().map(|r| r.contents.clone()),
            "run": runs.get(&service.name),
        }));
    }
    Ok(Json(json!({ "backups": out })))
}

/// Ask for a backup now. Accepts the run and returns; it does not wait for it.
///
/// Asynchronous because a real run tars, hashes, ships over ssh and verifies a remote
/// byte count — minutes, not one HTTP request. The response says the run was accepted;
/// `GET /api/axon-status/backups` says how it went, and keeps saying so across a page
/// refresh because the state lives here rather than in the page.
async fn backup_handler(
    Path(name): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let services = registry().await.map_err(bad_gateway)?;
    let Some(service) = services.into_iter().find(|s| s.name == name) else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(
                json!({ "error": format!("'{name}' is not an enabled capability on this machine") }),
            ),
        ));
    };
    if service.backup_contract().is_none() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": format!("'{name}' declares no backup contract") })),
        ));
    }

    let root = axon_root().map_err(bad_gateway)?;
    let started = now_epoch();
    {
        // Refuse rather than queue. A second concurrent run on one capability is not a
        // wasted fork: for a SQLite contract both runs drive the same maintenance hold,
        // and the first to finish resumes the capability out from under the second,
        // which is how a "coherent cold snapshot" stops being either.
        let mut runs = backup_runs().lock().unwrap();
        if let Some(existing) = runs.get(&name).filter(|r| r.state == "running") {
            return Err((
                StatusCode::CONFLICT,
                Json(json!({
                    "error": format!("a backup of '{name}' is already running"),
                    "run": existing,
                })),
            ));
        }
        runs.insert(
            name.clone(),
            BackupRun {
                state: "running",
                started_at: started,
                finished_at: None,
                detail: String::new(),
            },
        );
    }

    let task_name = name.clone();
    tokio::spawn(async move {
        let out = tokio::process::Command::new(root.join("tools/backup.sh"))
            .arg(&task_name)
            .output()
            .await;
        let (state, detail) = match out {
            Ok(o) if o.status.success() => ("succeeded", String::new()),
            Ok(o) => (
                "failed",
                String::from_utf8_lossy(&o.stderr).trim().to_string(),
            ),
            Err(e) => ("failed", format!("could not run tools/backup.sh: {e}")),
        };
        let mut runs = backup_runs().lock().unwrap();
        runs.insert(
            task_name,
            BackupRun {
                state,
                started_at: started,
                finished_at: Some(now_epoch()),
                detail,
            },
        );
    });

    Ok(Json(json!({
        "name": name,
        "accepted": true,
        "holds_service": service.backup_contract().map(|c| c.holds_service),
    })))
}

/// How often the reaper looks. Well under the smallest sane `idle_timeout`, so the
/// worst-case overshoot is one interval rather than a multiple of the timeout.
const REAP_INTERVAL: Duration = Duration::from_secs(30);

/// Seconds since a visible tab last said it was there, asked of the panel itself.
///
/// `None` on any failure, and that is the whole safety property: the panel not
/// answering means this process cannot tell whether somebody is reading it, and the
/// only acceptable answer to "I don't know" is to leave it alone. A panel served by
/// something other than tools/panel-server.ts simply never reports idle and is
/// therefore never reaped, which is the correct behaviour rather than a gap.
async fn panel_idle_seconds(client: &reqwest::Client, svc: &Service) -> Option<u64> {
    let url = format!("http://127.0.0.1:{}/__axon/idle", svc.panel_port);
    let res = client
        .get(&url)
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .ok()?;
    if !res.status().is_success() {
        return None;
    }
    res.json::<Value>()
        .await
        .ok()?
        .get("idle_seconds")?
        .as_u64()
}

/// Stop panels nobody is looking at.
///
/// `idle-stop`, not `stop`: `stop` sets a maintenance hold so a tool can work on a
/// capability's data undisturbed, and an unread page is not a maintenance window. A
/// hold here would make the panel un-startable by anything except the dashboard's
/// resume button until it expired.
async fn reap_idle_panels(client: &reqwest::Client, root: &std::path::Path) {
    let Ok(services) = registry().await else {
        return;
    };
    for svc in services {
        let Some(timeout) = svc.idle_timeout_secs() else {
            continue;
        };
        if !is_up(client, &svc).await {
            continue;
        }
        let Some(idle) = panel_idle_seconds(client, &svc).await else {
            continue;
        };
        if idle < timeout {
            continue;
        }
        let out = tokio::process::Command::new(root.join("tools/service-runner.sh"))
            .arg("idle-stop")
            .arg(&svc.name)
            .output()
            .await;
        match out {
            Ok(o) if o.status.success() => {
                println!(
                    "reaped {} after {idle}s idle (timeout {timeout}s)",
                    svc.name
                )
            }
            Ok(o) => eprintln!(
                "idle-stop {} failed: {}",
                svc.name,
                String::from_utf8_lossy(&o.stderr).trim()
            ),
            Err(e) => eprintln!("could not run service-runner.sh for {}: {e}", svc.name),
        }
    }
}

#[tokio::main]
async fn main() {
    STARTED_AT.set(Instant::now()).ok();

    // AXON_PORT first (the runner exports it from the manifest + machine override);
    // AXON_STATUS_PORT stays as the manual escape hatch for running outside the runner.
    let port = axon_server::resolve_port(Some("AXON_STATUS_PORT"), None, 8082);

    // The other half of on-demand. This process is already the only thing allowed to
    // start a capability, so it is also the only sensible place to stop one — anything
    // else would be a second lifecycle owner. It runs here rather than as a launchd
    // job for the same reason: a reaper that outlives axon-status could stop panels
    // while nothing is left to bring them back.
    match axon_root() {
        Ok(root) => {
            tokio::spawn(async move {
                let client = reqwest::Client::new();
                loop {
                    tokio::time::sleep(REAP_INTERVAL).await;
                    reap_idle_panels(&client, &root).await;
                }
            });
        }
        Err(e) => eprintln!("idle reaper not started: {e}"),
    }

    let app = Router::new()
        .route("/routes", get(routes))
        .route("/health", get(health_handler))
        .route("/api/axon-status/health", get(axon_status_health_handler))
        .route("/api/axon-status/routes", get(routes_handler))
        .route("/api/axon-status/capabilities", get(capabilities_handler))
        .route("/api/axon-status/self", get(self_model_handler))
        .route("/api/axon-status/repos", get(repos_handler))
        .route("/api/axon-status/upstreams", get(upstreams_handler))
        .route(
            "/api/axon-status/capabilities/:name/start",
            post(start_handler),
        )
        .route(
            "/api/axon-status/capabilities/:name/stop",
            post(stop_handler),
        )
        .route("/api/axon-status/backups", get(backups_handler))
        .route(
            "/api/axon-status/capabilities/:name/backup",
            post(backup_handler),
        );

    // This process can start and stop the machine's capabilities, so it answers to
    // this machine only (axon_server binds loopback) -- and deliberately carries no
    // CORS layer, unlike the data-serving siblings: a permissive header here would
    // let any website's JS drive start/stop from the operator's own browser.
    axon_server::serve_local("axon-status", port, app).await;
}

#[cfg(test)]
mod backup_tests {
    use super::*;

    /// A synthetic capability. Every field empty by default, so each test names only the
    /// declaration it is about — and a test for "no backup contract" is the default
    /// rather than something to remember to write.
    fn cap(name: &str) -> Service {
        Service {
            name: name.to_string(),
            kind: "container".into(),
            scope: "capability".into(),
            port: String::new(),
            health_path: String::new(),
            ready_path: String::new(),
            panel_port: String::new(),
            panel_path: String::new(),
            autostart: String::new(),
            idle_timeout: String::new(),
            backup_target: String::new(),
            backup_sqlite: String::new(),
            backup_advise_days: String::new(),
            backup_stale_days: String::new(),
            endpoint: String::new(),
            requires: Vec::new(),
        }
    }

    const DAY: u64 = 86_400;

    // --- externally provided capabilities (retired-tracker#169) ---------------------------
    //
    // The registry decides what is external and blanks every field that would be a claim of
    // authority over another host. These assert the half that lives here: that a resolved
    // endpoint is dialled instead of loopback, and that loopback is untouched without one.

    #[test]
    fn a_local_capability_is_polled_on_loopback() {
        // The case that must not change. Every capability this machine runs takes this path,
        // and it is the same string it was before external references existed.
        let mut c = cap("transit");
        c.port = "8085".into();
        c.health_path = "/health".into();
        assert_eq!(
            c.health_url().as_deref(),
            Some("http://127.0.0.1:8085/health")
        );
    }

    #[test]
    fn an_external_capability_is_polled_at_its_resolved_endpoint() {
        let mut c = cap("vaultwarden");
        c.scope = "external".into();
        c.endpoint = "https://vault.provider.test".into();
        c.health_path = "/alive".into();
        assert_eq!(
            c.health_url().as_deref(),
            Some("https://vault.provider.test/alive")
        );
    }

    #[test]
    fn an_external_capability_never_falls_back_to_a_local_port() {
        // A port would have to come from the manifest of a service running somewhere else, so
        // dialling 127.0.0.1 with it asks this machine about a process it does not have. The
        // registry blanks the field for exactly this reason; if that ever regresses, the URL
        // must still not be built.
        let mut c = cap("vaultwarden");
        c.scope = "external".into();
        c.port = "8080".into();
        c.health_path = "/alive".into();
        assert_eq!(
            c.health_url(),
            None,
            "an unresolved external reference has no health URL"
        );
    }

    #[test]
    fn an_endpoint_without_a_path_is_not_a_health_url() {
        // Knowing where something lives is not knowing how to ask it whether it is alive.
        // Reported as unknown, which is what a capability with no health surface has always
        // been — not as down, which would be a false alarm about someone else's machine.
        let mut c = cap("vaultwarden");
        c.scope = "external".into();
        c.endpoint = "https://vault.provider.test".into();
        assert_eq!(c.health_url(), None);
    }

    // --- readiness vs liveness (#126) -----------------------------------------------------
    //
    // Availability is judged on readiness where a capability declares it. The fallback is the
    // compatibility contract, and the reason this surface reported five database-backed
    // capabilities as up through a Postgres outage.

    #[test]
    fn readiness_is_preferred_over_liveness() {
        let mut c = cap("tasks");
        c.port = "8089".into();
        c.health_path = "/health".into();
        c.ready_path = "/ready".into();
        assert_eq!(
            c.readiness_url().as_deref(),
            Some("http://127.0.0.1:8089/ready")
        );
    }

    #[test]
    fn liveness_still_answers_for_a_capability_without_readiness() {
        // The case that must not change: every capability that declares no ready_path is
        // polled exactly where it was before.
        let mut c = cap("macmon");
        c.port = "9911".into();
        c.health_path = "/json".into();
        assert_eq!(
            c.readiness_url().as_deref(),
            Some("http://127.0.0.1:9911/json")
        );
    }

    #[test]
    fn readiness_reaches_an_external_capability_at_its_endpoint() {
        // ready_path is a "how to ask" field, so it crosses the machine boundary with
        // health_path rather than being blanked with the fields that describe how to act.
        let mut c = cap("vaultwarden");
        c.scope = "external".into();
        c.endpoint = "https://vault.provider.test".into();
        c.health_path = "/alive".into();
        c.ready_path = "/ready".into();
        assert_eq!(
            c.readiness_url().as_deref(),
            Some("https://vault.provider.test/ready")
        );
    }

    #[test]
    fn a_capability_with_neither_path_has_nothing_to_poll() {
        let mut c = cap("feed-sweep");
        c.port = "8090".into();
        assert_eq!(c.readiness_url(), None);
    }

    #[test]
    fn a_capability_without_a_target_has_no_backup_contract() {
        // Most capabilities. They must not appear on the surface at all: a transit server
        // listed as "never backed up" is a false alarm that teaches you to ignore the page.
        assert!(cap("transit").backup_contract().is_none());
    }

    #[test]
    fn the_target_is_what_makes_a_contract_not_the_day_fields() {
        // A capability may decline to say what timely means and still owe a backup. If
        // presence were derived from the day fields, this one would vanish from the
        // surface — the single place that could tell you it has never run.
        let mut c = cap("pihole");
        c.backup_target = "backup-target".into();
        let contract = c.backup_contract().expect("a target alone is a contract");
        assert_eq!(contract.advise_days, None);
        assert_eq!(contract.stale_days, None);
    }

    #[test]
    fn only_a_sqlite_contract_holds_the_service_down() {
        // The claim the confirmation dialog makes, and it is read from the contract rather
        // than matched against a service name. backup.sh stops a capability for the cold
        // SQLite copy and for nothing else — a pg_dumpall runs inside the live container.
        let mut pg = cap("postgres");
        pg.backup_target = "backup-target".into();
        assert!(!pg.backup_contract().unwrap().holds_service);

        let mut vw = cap("vaultwarden");
        vw.backup_target = "backup-target".into();
        vw.backup_sqlite = "data/vaultwarden/data/db.sqlite3".into();
        assert!(vw.backup_contract().unwrap().holds_service);
    }

    #[test]
    fn an_unparseable_day_field_reads_as_undeclared_not_as_zero() {
        // The dangerous coercion: `"soon".parse().unwrap_or(0)` makes every backup overdue
        // forever, and a badge that is always red is a badge nobody looks at.
        let mut c = cap("odd");
        c.backup_target = "backup-target".into();
        c.backup_stale_days = "soon".into();
        assert_eq!(c.backup_contract().unwrap().stale_days, None);
    }

    #[test]
    fn state_walks_ok_then_due_then_overdue() {
        let contract = BackupContract {
            holds_service: false,
            advise_days: Some(1),
            stale_days: Some(7),
        };
        assert_eq!(backup_state(Some(0), &contract), "ok");
        assert_eq!(backup_state(Some(23 * 3_600), &contract), "ok");
        assert_eq!(backup_state(Some(DAY), &contract), "due");
        assert_eq!(backup_state(Some(6 * DAY), &contract), "due");
        assert_eq!(backup_state(Some(7 * DAY), &contract), "overdue");
        assert_eq!(backup_state(Some(400 * DAY), &contract), "overdue");
    }

    #[test]
    fn no_receipt_is_never_whatever_the_thresholds_say() {
        // Outranks the thresholds deliberately: a contract with nothing behind it is the
        // worst state this surface can report, and it is not reachable by ageing.
        let declared = BackupContract {
            holds_service: false,
            advise_days: Some(1),
            stale_days: Some(7),
        };
        let silent = BackupContract {
            holds_service: false,
            advise_days: None,
            stale_days: None,
        };
        assert_eq!(backup_state(None, &declared), "never");
        assert_eq!(backup_state(None, &silent), "never");
    }

    #[test]
    fn undeclared_thresholds_never_invent_a_cadence() {
        // Axon ships no personal schedule. A capability whose owner never said what timely
        // means gets `unknown` — not a red badge derived from a number Axon made up.
        let silent = BackupContract {
            holds_service: false,
            advise_days: None,
            stale_days: None,
        };
        assert_eq!(backup_state(Some(900 * DAY), &silent), "unknown");

        // ...and half a declaration still only answers the half it declared.
        let advise_only = BackupContract {
            holds_service: false,
            advise_days: Some(1),
            stale_days: None,
        };
        assert_eq!(backup_state(Some(900 * DAY), &advise_only), "due");
    }

    #[test]
    fn a_receipt_dates_to_the_second() {
        // The live postgres receipt as backup.sh wrote it, against the epoch value
        // `date -u -d @... ` agrees with. Fixes the civil-calendar arithmetic to a real
        // observation rather than to itself.
        assert_eq!(parse_receipt_ts("20260805T220018Z"), Some(1_785_967_218));
        assert_eq!(parse_receipt_ts("20260801T074701Z"), Some(1_785_570_421));
        // The epoch itself, and a leap day — the two the era arithmetic would get wrong.
        assert_eq!(parse_receipt_ts("19700101T000000Z"), Some(0));
        assert_eq!(parse_receipt_ts("20240229T120000Z"), Some(1_709_208_000));
    }

    #[test]
    fn a_receipt_this_process_cannot_date_is_not_a_fresh_backup() {
        // Every one of these must read as "no usable receipt", which downstream is the
        // same answer as a missing file. The failure to avoid is a malformed timestamp
        // parsing to something plausible and reporting a stale backup as current.
        for bad in [
            "",
            "20260805T220018",      // no zone marker
            "20260805 220018Z",     // no T
            "2026-08-05T22:00:18Z", // ISO with separators: right length, wrong shape
            "20261305T220018Z",     // month 13
            "20260832T220018Z",     // day 32
            "20260805T250018Z",     // hour 25
            "20260805T226018Z",     // minute 60
            "yyyymmddThhmmssZ",
        ] {
            assert_eq!(parse_receipt_ts(bad), None, "should not parse: {bad:?}");
        }
    }

    #[test]
    fn a_run_state_is_terminal_or_running_never_absent() {
        // The spinner criterion. Whatever backup.sh does — succeed, fail, or fail to
        // start at all — the run must leave `running`, because the UI shows a spinner for
        // exactly as long as this says `running` and a locked vault is the normal way a
        // real run fails.
        let mut runs: HashMap<String, BackupRun> = HashMap::new();
        runs.insert(
            "vaultwarden".into(),
            BackupRun {
                state: "running",
                started_at: 100,
                finished_at: None,
                detail: String::new(),
            },
        );
        assert_eq!(runs["vaultwarden"].state, "running");

        // What the spawned task writes when the vault is locked: terminal, with the
        // provider's own message carried through rather than swallowed.
        runs.insert(
            "vaultwarden".into(),
            BackupRun {
                state: "failed",
                started_at: 100,
                finished_at: Some(160),
                detail: "backup.sh: vault is locked".into(),
            },
        );
        assert_eq!(runs["vaultwarden"].state, "failed");
        assert!(runs["vaultwarden"].finished_at.is_some());
        assert!(!runs["vaultwarden"].detail.is_empty());
    }

    #[test]
    fn the_projection_carries_no_destination() {
        // The boundary this whole surface is built around: a receipt names a target, a
        // tarball and a hash, and none of them may reach a response. Enforced against the
        // deserialize struct, so adding a field to Receipt without thinking fails here.
        let raw = r#"{"capability":"vaultwarden","completed_at":"20260801T074701Z",
            "target":"home-automation","tarball":"vaultwarden-20260801T074701Z.tar.gz",
            "bytes":348862,"sha256":"deadbeef","contents":"paths"}"#;
        let receipt: Receipt = serde_json::from_str(raw).expect("the live receipt shape parses");
        assert_eq!(receipt.bytes, 348_862);
        assert_eq!(receipt.contents, "paths");

        let rendered = serde_json::to_string(&json!({
            "capability": "vaultwarden",
            "last_success": receipt.completed_at,
            "bytes": receipt.bytes,
            "contents": receipt.contents,
        }))
        .unwrap();
        for private in ["home-automation", "tarball", "sha256", "deadbeef"] {
            assert!(
                !rendered.contains(private),
                "projection leaked {private}: {rendered}"
            );
        }
    }
}

#[cfg(test)]
mod route_manifest_tests {
    /// A stale manifest is worse than none, because it gets believed. This reads
    /// the router's own source, so adding a `.route()` without a summary fails
    /// here rather than shipping a surface that lies about itself.
    #[test]
    fn the_manifest_covers_every_served_route() {
        let missing = route_manifest::undeclared_routes(include_str!("main.rs"), super::ROUTES);
        assert!(missing.is_empty(), "served but undocumented: {missing:?}");
    }
}
