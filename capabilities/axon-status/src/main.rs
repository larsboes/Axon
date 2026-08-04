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

#[path = "../../../libs/axon-server/src/lib.rs"]
#[allow(dead_code)]
mod axon_server;

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
    panel_port: String,
    panel_path: String,
    autostart: String,
    /// Seconds a panel may go unread before it is stopped. Empty means never — a
    /// capability opts into being reaped, it is not opted in by having a panel.
    #[serde(default)]
    idle_timeout: String,
    #[serde(default)]
    requires: Vec<String>,
}

impl Service {
    /// `None` when the manifest declares nothing to poll — a capability without a
    /// health surface is reported as unknown rather than silently "down".
    fn health_url(&self) -> Option<String> {
        if self.port.is_empty() || self.health_path.is_empty() {
            return None;
        }
        Some(format!("http://127.0.0.1:{}{}", self.port, self.health_path))
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
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
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
];

/// Shorthand so the table above reads as a table.
const fn r(method: &'static str, path: &'static str, summary: &'static str) -> route_manifest::Route {
    route_manifest::Route { method, path, summary }
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
    let services: Vec<Service> =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("registry is not valid JSON: {e}"))?;

    if let Some(key) = key {
        if let Ok(mut cache) = REGISTRY_CACHE.get_or_init(|| Mutex::new(None)).lock() {
            *cache = Some((key, services.clone()));
        }
    }
    Ok(services)
}

async fn is_up(client: &reqwest::Client, svc: &Service) -> bool {
    let Some(url) = svc.health_url() else {
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
        let up = match service.health_url() {
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
        let Some(url) = svc.health_url() else { continue };
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
        let up = match service.health_url() {
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
            Json(json!({ "error": format!("'{name}' is not an enabled capability on this machine") })),
        ));
    };

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
    (StatusCode::BAD_GATEWAY, Json(json!({ "error": msg.into() })))
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
    res.json::<Value>().await.ok()?.get("idle_seconds")?.as_u64()
}

/// Stop panels nobody is looking at.
///
/// `idle-stop`, not `stop`: `stop` sets a maintenance hold so a tool can work on a
/// capability's data undisturbed, and an unread page is not a maintenance window. A
/// hold here would make the panel un-startable by anything except the dashboard's
/// resume button until it expired.
async fn reap_idle_panels(client: &reqwest::Client, root: &std::path::Path) {
    let Ok(services) = registry().await else { return };
    for svc in services {
        let Some(timeout) = svc.idle_timeout_secs() else { continue };
        if !is_up(client, &svc).await {
            continue;
        }
        let Some(idle) = panel_idle_seconds(client, &svc).await else { continue };
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
                println!("reaped {} after {idle}s idle (timeout {timeout}s)", svc.name)
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
        );

    // This process can start and stop the machine's capabilities, so it answers to
    // this machine only (axon_server binds loopback) -- and deliberately carries no
    // CORS layer, unlike the data-serving siblings: a permissive header here would
    // let any website's JS drive start/stop from the operator's own browser.
    axon_server::serve_local("axon-status", port, app).await;
}

// The self-describing surface, on the same include terms as the other libs.
#[path = "../../../libs/route-manifest/src/lib.rs"]
#[allow(dead_code)]
mod route_manifest;

#[cfg(test)]
mod route_manifest_tests {
    /// A stale manifest is worse than none, because it gets believed. This reads
    /// the router's own source, so adding a `.route()` without a summary fails
    /// here rather than shipping a surface that lies about itself.
    #[test]
    fn the_manifest_covers_every_served_route() {
        let missing =
            super::route_manifest::undeclared_routes(include_str!("main.rs"), super::ROUTES);
        assert!(missing.is_empty(), "served but undocumented: {missing:?}");
    }
}
