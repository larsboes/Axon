//! The native bridge from the app to the Mac's Axon services.
//!
//! ## Why the request leaves from Rust and not from the WebView
//!
//! Inside the Tauri bundle a relative `fetch('/interior/api/items')` resolves
//! against the app's own origin and reaches nothing. An absolute WebView fetch
//! to the Mac would carry `Origin: tauri://localhost`, which
//! `libs/axon-server/src/origin.rs` (`origin_allowed_by`) refuses. Admitting
//! that origin would admit every Tauri app on the phone, not this one.
//!
//! A request made here sends no `Origin` header, so origin.rs passes it the way
//! it passes curl. `tailscale serve` still injects the caller's tailnet
//! identity, so the tailnet gate in `libs/axon-server/src/tailnet.rs` applies
//! without a shared secret in the app.
//!
//! ## Why it is not a general proxy
//!
//! The WebView can call this command with any argument. So the base URL, the
//! path, the method and the headers are each checked against a closed set
//! before anything leaves the device:
//!
//! - base URL: `https://<name>.ts.net[:port]`, or loopback on desktop only;
//! - path: relative, under [`ALLOWED_PATHS`], no `..`, no scheme, no `//`;
//! - headers: [`FORWARDED_HEADERS`] only, values without control characters;
//! - redirects: never followed.
//!
//! ## Two commands, one rule set
//!
//! [`mac_request`] answers text, which every JSON API needs. [`mac_request_bytes`]
//! answers raw bytes for pictures and the RoomPlan USDZ, which do not survive a
//! UTF-8 string. It is GET only, refuses an answer larger than
//! [`MAX_RESPONSE_BYTES`], and sends its answer as a raw IPC payload
//! ([`tauri::ipc::Response`]), not as base64 inside JSON. The frame is
//! described at [`frame_bytes`].
//!
//! The base URL is a house fact and this repository is public. It lives in the
//! app-private settings file ([`SETTINGS_FILE`]), never in code or config.
//!
//! ## Offline
//!
//! Both commands go through `crate::sync` (sync step 2, PRD §10 A5): a C1 answer is kept,
//! served from the device when the Mac cannot be reached, and an item edit made offline is
//! queued. [`ReqwestTransport`] is the only code here that touches the network.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use reqwest::Url;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager, Runtime, State};

use crate::sync::{self, LocalStore, Outgoing, Reply, SendError, SyncStatus, Transport};

/// The Mac paths the app may reach. An entry that ends in `/` admits every
/// path below it. Any other entry admits that exact path, optionally with a
/// query string.
///
/// ## Why this set, and why it adds no exposure
///
/// The set is the mount table of the shell's proxy, restricted to the
/// capabilities the dashboard calls. `capabilities/axon-status/src/proxy.rs`
/// (`Proxy::new`) mounts every registry entry with a port at `/<name>`, or at
/// `/<name>/api` where its `service.toml` sets `proxy_api_only = "true"`
/// (calendar, finance, interior, traveler, trips), and passes each
/// `proxy_extra` prefix through unstripped (transit's `/api`, which is how
/// `transit` in `dashboard/src/lib/api.ts` calls it). `/axon-status/` is the
/// shell's own API, which `strip_self_prefix` in the same file serves.
///
/// PRD Q94 puts that shell behind `tailscale serve`, so a phone browser on the
/// tailnet already reaches every path below. The bridge reaches the same
/// server with the same tailnet identity and no more headers than a browser
/// sends (see [`FORWARDED_HEADERS`]). So this list gives the app what the phone
/// browser has, and nothing that the browser does not have.
///
/// Token-guarded routes: comms is the one capability that sets
/// `refuse_without_token` (`capabilities/comms/src/server/main.rs`,
/// `inbound_auth`), and a tailnet identity never satisfies it
/// (`libs/axon-server/src/auth.rs`). The bridge sends no token. But the shell
/// adds comms' bearer token to every request it forwards to `/comms`
/// (`inject_comms_auth` in `proxy.rs`), so `POST /comms/ingest` works through
/// the bridge exactly as it works from the phone browser. The server refuses
/// it only if the shell has no token configured. This is Q94's exposure, not a
/// new one.
///
/// `/foundation-models/` is the Mac rung of the app's model ladder
/// (`src/lib/intelligence/`), admitted 2026-09-25 so a phone without Apple
/// Intelligence still reaches a model. It is the same exposure as the other
/// mounts: the operator's tailnet identity plus this device's signed headers.
///
/// Not listed: capabilities the shell proxies but the dashboard does not call
/// (punctuality, soundscape) and scouting's `/discover`.
pub const ALLOWED_PATHS: &[&str] = &[
    "/axon-status/",
    "/calendar/api/",
    "/comms/",
    "/devices/api/",
    "/entities/api/",
    "/finance/api/",
    "/foundation-models/",
    "/interior/api/",
    "/knowledge-graph/",
    "/macmon/",
    "/places/",
    "/scouting/",
    "/transit/",
    "/api/",
    "/traveler/api/",
    "/trips/api/",
    "/vault/",
];

/// The request headers the bridge forwards. Every other name is dropped.
pub const FORWARDED_HEADERS: &[&str] = &["accept", "content-type", "if-match"];

/// The largest answer [`mac_request_bytes`] accepts. A larger answer is refused,
/// not truncated. A RoomPlan USDZ of one flat is a few MB (not measured for
/// every flat); 64 MiB leaves room and still bounds the phone's memory.
pub const MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;

/// The methods a capability API uses.
const ALLOWED_METHODS: &[&str] = &["GET", "POST", "PUT", "PATCH", "DELETE"];

/// The node connection settings file, in the app data directory.
pub const SETTINGS_FILE: &str = "axon-node.json";
/// The pre-node-neutral settings file. It remains readable so existing installs keep working.
const LEGACY_SETTINGS_FILE: &str = "mac-bridge.json";
pub const NODE_PROTOCOL_VERSION: &str = "axon-node/v1";

/// A request that is not answered in this time fails.
const TIMEOUT: Duration = Duration::from_secs(30);

/// A connection that is not open in this time fails, and an offline read falls back to the
/// device's copy. Shorter than [`TIMEOUT`] so the fallback does not wait 30 s. A tailnet
/// connection through a DERP relay usually opens in well under this (not measured here).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Settings {
    pub protocol_version: String,
    /// Stable identity of this canonical Axon node. It survives URL changes and migration.
    #[serde(default = "default_node_id")]
    pub node_id: String,
    /// For example `https://<name>.ts.net`. `None` until the operator sets it.
    pub canonical_base_url: Option<String>,
}

fn default_node_id() -> String {
    "node_mac".to_string()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            protocol_version: NODE_PROTOCOL_VERSION.to_string(),
            node_id: default_node_id(),
            canonical_base_url: None,
        }
    }
}

#[derive(Debug, Deserialize)]
struct LegacySettings {
    base_url: Option<String>,
}

fn settings_from_legacy(legacy: LegacySettings) -> Settings {
    Settings {
        canonical_base_url: legacy.base_url,
        ..Settings::default()
    }
}

fn parse_settings(text: &str) -> Result<Settings, String> {
    let settings: Settings = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if settings.protocol_version != NODE_PROTOCOL_VERSION {
        return Err(format!(
            "unsupported protocol version {:?}; expected {NODE_PROTOCOL_VERSION}",
            settings.protocol_version
        ));
    }
    Ok(settings)
}

#[derive(Debug, Deserialize)]
pub struct MacRequest {
    pub method: String,
    pub path: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
}

#[derive(Debug, Serialize)]
pub struct MacResponse {
    pub status: u16,
    pub content_type: Option<String>,
    pub body: String,
    /// True when the Mac was not reached and this is the device's copy (`crate::sync`).
    pub stale: bool,
    /// For a stale answer: when the copy was fetched, unix milliseconds.
    pub fetched_at: Option<i64>,
}

impl From<MacRequest> for Outgoing {
    fn from(r: MacRequest) -> Self {
        Outgoing {
            method: r.method,
            path: r.path,
            headers: r.headers,
            body: r.body,
        }
    }
}

/// The prefix the TypeScript side matches to tell "no base URL" from a
/// network failure. Keep it in sync with `dashboard/src/lib/mac-bridge.ts`.
pub const NOT_CONFIGURED: &str = "axon-node: not configured";

/// Checks a base URL and returns it normalized to `scheme://host[:port]`.
///
/// `allow_loopback` is true only on desktop: on the phone, loopback is the
/// phone itself, and plain http to it has no use.
pub fn validate_base(base: &str, allow_loopback: bool) -> Result<Url, String> {
    let trimmed = base.trim().trim_end_matches('/');
    let url = Url::parse(trimmed).map_err(|e| format!("base URL is not a URL: {e}"))?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err("base URL must not contain a user name or password".into());
    }
    if url.query().is_some() || url.fragment().is_some() || !matches!(url.path(), "" | "/") {
        return Err("base URL must be only scheme, host and port".into());
    }
    // `host_str` writes IPv6 in brackets, so `[::1]` compares as written.
    let Some(host) = url.host_str().map(str::to_ascii_lowercase) else {
        return Err("base URL has no host".into());
    };
    let loopback = matches!(host.as_str(), "localhost" | "127.0.0.1" | "[::1]");
    let tailnet = host
        .strip_suffix(".ts.net")
        .is_some_and(|name| !name.is_empty() && !name.starts_with('.') && !name.ends_with('.'));
    match url.scheme() {
        "https" if tailnet => {}
        "http" | "https" if loopback && allow_loopback => {}
        "http" | "https" if loopback => {
            return Err("loopback base URL is allowed only in the desktop app".into())
        }
        "http" if tailnet => return Err("a tailnet base URL must use https".into()),
        _ => {
            return Err(
                "base URL must be https://<name>.ts.net or, on the Mac, http://127.0.0.1:<port>"
                    .into(),
            )
        }
    }
    Ok(url)
}

/// Checks a request path against [`ALLOWED_PATHS`] and the shape rules.
pub fn validate_path(path: &str) -> Result<(), String> {
    if !path.starts_with('/') || path.starts_with("//") {
        return Err("path must be relative to the Mac and start with a single /".into());
    }
    if path.contains("://") || path.contains('\\') || path.contains('#') {
        return Err("path must not contain a scheme, a backslash or a fragment".into());
    }
    if path.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err("path must not contain whitespace or control characters".into());
    }
    let (route, _query) = path.split_once('?').unwrap_or((path, ""));
    if route.contains("//") {
        return Err("path must not contain //".into());
    }
    let lower = route.to_ascii_lowercase();
    if route
        .split('/')
        .any(|segment| segment == ".." || segment == ".")
        || lower.contains("%2e")
        || lower.contains("%2f")
        || lower.contains("%5c")
    {
        return Err("path must not contain dot segments or encoded separators".into());
    }
    let allowed = ALLOWED_PATHS.iter().any(|entry| {
        if entry.ends_with('/') {
            route.starts_with(entry) && route.len() > entry.len()
        } else {
            route == *entry
        }
    });
    if !allowed {
        return Err(format!(
            "path {route} is not on the app's allow-list ({})",
            ALLOWED_PATHS.join(", ")
        ));
    }
    Ok(())
}

/// Joins a checked base and a checked path, and confirms the join did not
/// move the request to another origin or path.
pub fn target_url(base: &Url, path: &str) -> Result<Url, String> {
    validate_path(path)?;
    let origin = base.origin().ascii_serialization();
    let url = Url::parse(&format!("{origin}{path}")).map_err(|e| format!("bad path: {e}"))?;
    if url.origin() != base.origin() {
        return Err("path changed the request origin".into());
    }
    let route = path.split_once('?').map_or(path, |(route, _)| route);
    if url.path() != route {
        return Err("path was normalized to a different path".into());
    }
    Ok(url)
}

/// Keeps the forwarded headers and refuses a value that could split a header.
pub fn filter_headers(headers: &[(String, String)]) -> Result<Vec<(String, String)>, String> {
    let mut kept = Vec::new();
    for (name, value) in headers {
        let name = name.trim().to_ascii_lowercase();
        if !FORWARDED_HEADERS.contains(&name.as_str()) {
            continue;
        }
        if value.chars().any(|c| c.is_control()) {
            return Err(format!("header {name} contains a control character"));
        }
        kept.push((name, value.clone()));
    }
    Ok(kept)
}

pub fn validate_method(method: &str) -> Result<reqwest::Method, String> {
    let upper = method.trim().to_ascii_uppercase();
    if !ALLOWED_METHODS.contains(&upper.as_str()) {
        return Err(format!("method {method} is not allowed"));
    }
    reqwest::Method::from_bytes(upper.as_bytes()).map_err(|e| e.to_string())
}

fn settings_dir<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map_err(|e| format!("app data directory is not available: {e}"))
}

fn settings_path<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    Ok(settings_dir(app)?.join(SETTINGS_FILE))
}

fn read_settings<R: Runtime>(app: &AppHandle<R>) -> Result<Settings, String> {
    let dir = settings_dir(app)?;
    let path = dir.join(SETTINGS_FILE);
    match std::fs::read_to_string(&path) {
        Ok(text) => parse_settings(&text).map_err(|e| format!("{SETTINGS_FILE}: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let legacy_path = dir.join(LEGACY_SETTINGS_FILE);
            match std::fs::read_to_string(&legacy_path) {
                Ok(text) => serde_json::from_str::<LegacySettings>(&text)
                    .map(settings_from_legacy)
                    .map_err(|e| format!("{LEGACY_SETTINGS_FILE}: {e}")),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
                Err(e) => Err(format!("{LEGACY_SETTINGS_FILE}: {e}")),
            }
        }
        Err(e) => Err(format!("{SETTINGS_FILE}: {e}")),
    }
}

fn allow_loopback() -> bool {
    cfg!(desktop)
}

#[tauri::command]
pub async fn connection_settings_get<R: Runtime>(app: AppHandle<R>) -> Result<Settings, String> {
    read_settings(&app)
}

#[cfg(mobile)]
pub fn connection_settings<R: Runtime>(app: &AppHandle<R>) -> Result<Settings, String> {
    read_settings(app)
}

/// Stores the canonical node URL. An empty value clears it.
#[tauri::command]
pub async fn connection_settings_set<R: Runtime>(
    app: AppHandle<R>,
    canonical_base_url: String,
) -> Result<Settings, String> {
    let settings = if canonical_base_url.trim().is_empty() {
        Settings::default()
    } else {
        let url = validate_base(&canonical_base_url, allow_loopback())?;
        Settings {
            canonical_base_url: Some(url.origin().ascii_serialization()),
            ..Settings::default()
        }
    };
    let path = settings_path(&app)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let text = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| format!("{SETTINGS_FILE}: {e}"))?;
    Ok(settings)
}

/// The Mac as a [`Transport`]. Every request is checked here, whichever command sent it, so
/// every command applies the same rules.
trait RequestSigner: Send + Sync {
    fn device_id(&self) -> &str;
    fn sign(&self, message: &[u8]) -> Result<Vec<u8>, String>;
}

#[cfg(mobile)]
struct NativeRequestSigner<R: Runtime> {
    app: AppHandle<R>,
    device_id: String,
}

#[cfg(mobile)]
impl<R: Runtime> RequestSigner for NativeRequestSigner<R> {
    fn device_id(&self) -> &str {
        &self.device_id
    }

    fn sign(&self, message: &[u8]) -> Result<Vec<u8>, String> {
        self.app
            .state::<tauri_plugin_device_identity::DeviceIdentity<R>>()
            .sign(message)
            .map_err(|error| error.to_string())
    }
}

pub struct ReqwestTransport {
    base: Url,
    client: reqwest::Client,
    signer: Option<Arc<dyn RequestSigner>>,
}

impl ReqwestTransport {
    pub fn new(base: Url) -> Result<Self, String> {
        Self::with_signer(base, None)
    }

    fn with_signer(base: Url, signer: Option<Arc<dyn RequestSigner>>) -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(TIMEOUT)
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .map_err(|e| format!("mac-bridge: {e}"))?;
        Ok(Self {
            base,
            client,
            signer,
        })
    }
}

/// Whether an error from `send()` means the Mac was not reached.
///
/// Decided by what the error is NOT, because the flags are unreliable per platform:
/// measured on an iPhone in airplane mode (2026-09-25), the DNS failure ("nodename nor
/// servname provided, or not known") came back without `is_connect()`, so the page showed
/// the error instead of the local copy. Any error that arrives before a response exists means
/// the Mac did not answer, except a request we built wrongly (`is_builder`) and a redirect,
/// which the client refuses by policy. Those are our faults, and the offline path must not hide
/// them.
fn is_unreachable(builder: bool, redirect: bool) -> bool {
    !builder && !redirect
}

fn classify(error: reqwest::Error, what: &str) -> SendError {
    let message = format!("mac-bridge: {what}: {error}");
    if is_unreachable(error.is_builder(), error.is_redirect()) {
        SendError::Unreachable(message)
    } else {
        SendError::Failed(message)
    }
}

fn signed_path(path: &str) -> &str {
    if path == "/api" || path.starts_with("/api/") {
        return path;
    }
    // The shell mounts capabilities at /<name>/..., then strips that first segment before
    // forwarding. Sign the upstream target, not the shell mount.
    path[1..].find('/').map_or("/", |index| &path[index + 1..])
}

fn body_digest(body: &[u8]) -> String {
    Sha256::digest(body)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn signed_request(
    signer: &dyn RequestSigner,
    method: &str,
    path: &str,
    body: &[u8],
) -> Result<[(String, String); 4], String> {
    let timestamp = now_ms() / 1000;
    let mut nonce_bytes = [0u8; 16];
    getrandom::fill(&mut nonce_bytes).map_err(|error| format!("secure random source: {error}"))?;
    let nonce = hex(&nonce_bytes);
    let message = format!(
        "axon-device-auth/v1\n{}\n{}\n{}\n{}\n{}\n{}\n",
        signer.device_id(),
        timestamp,
        nonce,
        method.to_ascii_uppercase(),
        signed_path(path),
        body_digest(body),
    );
    let signature = hex(&signer.sign(message.as_bytes())?);
    Ok([
        ("x-axon-device-id".into(), signer.device_id().into()),
        ("x-axon-timestamp".into(), timestamp.to_string()),
        ("x-axon-nonce".into(), nonce),
        ("x-axon-signature".into(), signature),
    ])
}

impl Transport for ReqwestTransport {
    async fn send(&self, request: &Outgoing, max_bytes: usize) -> Result<Reply, SendError> {
        let url = target_url(&self.base, &request.path).map_err(SendError::Failed)?;
        let method = validate_method(&request.method).map_err(SendError::Failed)?;
        let mut headers = filter_headers(&request.headers).map_err(SendError::Failed)?;
        let body = request.body.as_deref().unwrap_or("").as_bytes();
        if let Some(signer) = &self.signer {
            headers.extend(
                signed_request(signer.as_ref(), method.as_str(), &request.path, body)
                    .map_err(SendError::Failed)?,
            );
        }
        let mut builder = self.client.request(method, url);
        for (name, value) in headers {
            builder = builder.header(name, value);
        }
        if request.body.is_some() {
            builder = builder.body(body.to_vec());
        }
        let mut response = builder
            .send()
            .await
            .map_err(|e| classify(e, "the Mac did not answer"))?;
        check_declared_length(response.content_length(), max_bytes).map_err(SendError::Failed)?;
        let status = response.status().as_u16();
        let content_type = content_type_of(&response);
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| classify(e, "reading the answer failed"))?
        {
            append_capped(&mut body, &chunk, max_bytes).map_err(SendError::Failed)?;
        }
        Ok(Reply {
            status,
            content_type,
            body,
        })
    }
}

/// The transport to the configured canonical node, or the "not configured" error.
pub fn transport<R: Runtime>(app: &AppHandle<R>) -> Result<ReqwestTransport, String> {
    let settings = read_settings(app)?;
    let Some(base) = settings.canonical_base_url.filter(|b| !b.trim().is_empty()) else {
        return Err(format!(
            "{NOT_CONFIGURED}: set the Axon node address in Settings, Axon connection"
        ));
    };
    let base = validate_base(&base, allow_loopback())?;
    #[cfg(mobile)]
    {
        let identity = app
            .state::<tauri_plugin_device_identity::DeviceIdentity<R>>()
            .get()
            .map_err(|error| error.to_string())?;
        return ReqwestTransport::with_signer(
            base,
            Some(Arc::new(NativeRequestSigner {
                app: app.clone(),
                device_id: identity.id,
            })),
        );
    }
    #[cfg(not(mobile))]
    ReqwestTransport::new(base)
}

fn content_type_of(response: &reqwest::Response) -> Option<String> {
    response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

/// The app's local store, opened once at start. `store` is `None` when the database could not
/// be opened; the bridge then works as it did before step 2, and `error` says why.
pub struct Local {
    pub store: Option<LocalStore>,
    pub error: Option<String>,
}

impl Local {
    pub fn open<R: Runtime>(app: &AppHandle<R>) -> Self {
        let opened = app
            .path()
            .app_data_dir()
            .map_err(|e| format!("app data directory is not available: {e}"))
            .and_then(|dir| LocalStore::open(&dir.join(sync::DB_FILE)));
        match opened {
            Ok(store) => Self {
                store: Some(store),
                error: None,
            },
            Err(error) => {
                log::error!("{error}");
                Self {
                    store: None,
                    error: Some(error),
                }
            }
        }
    }

    fn store(&self) -> Result<&LocalStore, String> {
        self.store.as_ref().ok_or_else(|| {
            format!(
                "local store: {}",
                self.error.as_deref().unwrap_or("not open")
            )
        })
    }
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

#[tauri::command]
pub async fn mac_request<R: Runtime>(
    app: AppHandle<R>,
    local: State<'_, Local>,
    request: MacRequest,
) -> Result<MacResponse, String> {
    let t = transport(&app)?;
    match &local.store {
        Some(store) => sync::request_text(store, &t, request.into(), now_ms()).await,
        None => sync::plain_text(&t, request.into()).await,
    }
}

/// Refuses an answer whose declared length is above `cap`.
pub fn check_declared_length(length: Option<u64>, cap: usize) -> Result<(), String> {
    match length {
        Some(n) if n > cap as u64 => Err(format!(
            "mac-bridge: the answer is {n} bytes, above the limit of {cap} bytes"
        )),
        _ => Ok(()),
    }
}

/// Adds one chunk to `body`, or refuses when the total would pass `cap`. The
/// declared length can be absent or wrong, so the count is what decides.
pub fn append_capped(body: &mut Vec<u8>, chunk: &[u8], cap: usize) -> Result<(), String> {
    if body.len() + chunk.len() > cap {
        return Err(format!(
            "mac-bridge: the answer is above the limit of {cap} bytes"
        ));
    }
    body.extend_from_slice(chunk);
    Ok(())
}

/// The raw answer of [`mac_request_bytes`]:
///
/// | bytes      | content                                   |
/// |------------|-------------------------------------------|
/// | 0..2       | HTTP status, u16 big-endian               |
/// | 2..4       | content-type length `n`, u16 big-endian   |
/// | 4..4+n     | content-type, UTF-8 (empty when absent)   |
/// | 4+n..      | the body, unchanged                       |
///
/// `dashboard/src/lib/mac-bridge.ts` (`decodeBytesFrame`) reads the same frame.
pub fn frame_bytes(status: u16, content_type: Option<&str>, body: &[u8]) -> Vec<u8> {
    let ct = content_type.unwrap_or("").as_bytes();
    // A content type longer than u16::MAX is not a content type; drop it.
    let ct = if ct.len() > usize::from(u16::MAX) {
        &[][..]
    } else {
        ct
    };
    let mut out = Vec::with_capacity(4 + ct.len() + body.len());
    out.extend_from_slice(&status.to_be_bytes());
    out.extend_from_slice(&(ct.len() as u16).to_be_bytes());
    out.extend_from_slice(ct);
    out.extend_from_slice(body);
    out
}

/// Fetches one path as bytes, for pictures and the RoomPlan USDZ. GET only,
/// same path, header and base rules as [`mac_request`]. An answer from the device's copy
/// carries status [`sync::STALE_BYTES_STATUS`] (203).
#[tauri::command]
pub async fn mac_request_bytes<R: Runtime>(
    app: AppHandle<R>,
    local: State<'_, Local>,
    path: String,
    headers: Option<Vec<(String, String)>>,
) -> Result<tauri::ipc::Response, String> {
    let t = transport(&app)?;
    let request = Outgoing {
        method: "GET".into(),
        path,
        headers: headers.unwrap_or_default(),
        body: None,
    };
    let reply = match &local.store {
        Some(store) => {
            sync::request_bytes(store, &t, request, MAX_RESPONSE_BYTES, now_ms())
                .await?
                .reply
        }
        None => t
            .send(&request, MAX_RESPONSE_BYTES)
            .await
            .map_err(SendError::message)?,
    };
    Ok(tauri::ipc::Response::new(frame_bytes(
        reply.status,
        reply.content_type.as_deref(),
        &reply.body,
    )))
}

fn status_of(local: &Local) -> Result<SyncStatus, String> {
    match &local.store {
        Some(store) => store.status(),
        None => Ok(SyncStatus {
            offline: false,
            offline_since: None,
            showing_from: None,
            pending: 0,
            conflicts: 0,
            failed: 0,
            store_error: local.error.clone(),
        }),
    }
}

/// Counts for the sync status line, and whether the Mac answered the last request.
#[tauri::command]
pub async fn sync_status(local: State<'_, Local>) -> Result<SyncStatus, String> {
    status_of(&local)
}

/// Every queued edit, oldest first, for the conflicts view.
#[tauri::command]
pub async fn sync_entries(local: State<'_, Local>) -> Result<Vec<sync::OutboxEntry>, String> {
    local.store()?.entries()
}

/// Sends pending edits now ("Sync now", app start, foreground). Without a canonical node address it
/// sends nothing and answers the status.
#[tauri::command]
pub async fn sync_flush<R: Runtime>(
    app: AppHandle<R>,
    local: State<'_, Local>,
) -> Result<SyncStatus, String> {
    let store = local.store()?;
    if let Ok(t) = transport(&app) {
        let now = now_ms();
        #[cfg(mobile)]
        {
            let settings = read_settings(&app)?;
            let identity = app
                .state::<tauri_plugin_device_identity::DeviceIdentity<R>>()
                .get()
                .map_err(|error| error.to_string())?;
            sync::flush_protocol(
                store,
                &t,
                now,
                &format!("node_{}", identity.id),
                &settings.node_id,
                &identity.id,
            )
            .await;
        }
        #[cfg(not(mobile))]
        {
            sync::flush(store, &t, now).await;
        }
    }
    status_of(&local)
}

/// Resolves one entry. `keep_mine` re-queues it against the Mac's current revision and sends
/// it; `discard` drops it and the Mac's value stands.
#[tauri::command]
pub async fn sync_resolve<R: Runtime>(
    app: AppHandle<R>,
    local: State<'_, Local>,
    id: i64,
    action: String,
) -> Result<SyncStatus, String> {
    let store = local.store()?;
    match action.as_str() {
        "keep_mine" => {
            store.keep_mine(id, now_ms())?;
            if let Ok(t) = transport(&app) {
                let now = now_ms();
                #[cfg(mobile)]
                {
                    let settings = read_settings(&app)?;
                    let identity = app
                        .state::<tauri_plugin_device_identity::DeviceIdentity<R>>()
                        .get()
                        .map_err(|error| error.to_string())?;
                    sync::flush_protocol(
                        store,
                        &t,
                        now,
                        &format!("node_{}", identity.id),
                        &settings.node_id,
                        &identity.id,
                    )
                    .await;
                }
                #[cfg(not(mobile))]
                {
                    sync::flush(store, &t, now).await;
                }
            }
        }
        "discard" => store.discard(id)?,
        other => return Err(format!("unknown action `{other}` (keep_mine or discard)")),
    }
    status_of(&local)
}

#[cfg(test)]
mod tests {

    #[test]
    fn only_our_own_request_errors_count_as_failures_not_as_offline() {
        // DNS, connect, TLS, timeout, reset: all arrive without a response, so all are offline.
        assert!(is_unreachable(false, false));
        assert!(!is_unreachable(true, false));
        assert!(!is_unreachable(false, true));
    }

    use super::*;

    fn base(url: &str) -> Url {
        validate_base(url, true).expect("valid base")
    }

    #[test]
    fn signed_path_matches_the_upstream_capability_path() {
        assert_eq!(signed_path("/devices/api/devices/me"), "/api/devices/me");
        assert_eq!(
            signed_path("/interior/api/items/item-1?room=kitchen"),
            "/api/items/item-1?room=kitchen"
        );
        assert_eq!(
            signed_path("/api/suggest?q=Berlin"),
            "/api/suggest?q=Berlin"
        );
        assert_eq!(signed_path("/devices"), "/");
    }

    #[test]
    fn settings_use_the_node_protocol_and_legacy_settings_map_forward() {
        let current = Settings {
            canonical_base_url: Some("https://node.example.ts.net".into()),
            ..Settings::default()
        };
        let encoded = serde_json::to_string(&current).unwrap();
        assert!(encoded.contains("axon-node/v1"));
        assert!(encoded.contains("canonical_base_url"));
        assert!(encoded.contains("node_id"));
        assert!(parse_settings(&encoded).is_ok());
        assert!(parse_settings(
            r#"{\"protocol_version\":\"axon-node/v0\",\"canonical_base_url\":null}"#
        )
        .is_err());

        let legacy = settings_from_legacy(LegacySettings {
            base_url: Some("https://mac.example.ts.net".into()),
        });
        assert_eq!(legacy.protocol_version, NODE_PROTOCOL_VERSION);
        assert_eq!(
            legacy.canonical_base_url.as_deref(),
            Some("https://mac.example.ts.net")
        );
    }

    #[test]
    fn accepts_tailnet_https_with_and_without_port() {
        assert!(validate_base("https://mac.example-tailnet.ts.net", false).is_ok());
        assert!(validate_base("https://mac.example-tailnet.ts.net/", false).is_ok());
        assert!(validate_base("https://mac.example-tailnet.ts.net:8443", false).is_ok());
    }

    #[test]
    fn accepts_loopback_only_on_desktop() {
        assert!(validate_base("http://127.0.0.1:8082", true).is_ok());
        assert!(validate_base("http://localhost:8082", true).is_ok());
        assert!(validate_base("http://[::1]:8082", true).is_ok());
        assert!(validate_base("http://127.0.0.1:8082", false).is_err());
    }

    #[test]
    fn refuses_http_to_non_loopback() {
        assert!(validate_base("http://mac.example-tailnet.ts.net", true).is_err());
        assert!(validate_base("http://192.168.1.10:8082", true).is_err());
        assert!(validate_base("http://100.64.0.1", true).is_err());
    }

    #[test]
    fn refuses_other_hosts() {
        for bad in [
            "https://example.com",
            "https://ts.net",
            "https://.ts.net",
            "https://evil.ts.net.example.com",
            "https://evilts.net",
            "https://100.64.0.1",
            "ftp://mac.example-tailnet.ts.net",
            "tauri://localhost",
            "https://user:pw@mac.example-tailnet.ts.net",
            "https://mac.example-tailnet.ts.net/interior",
            "https://mac.example-tailnet.ts.net?x=1",
            "not a url",
        ] {
            assert!(validate_base(bad, true).is_err(), "{bad} must be refused");
        }
    }

    #[test]
    fn accepts_allow_listed_paths() {
        for good in [
            "/interior/api/items",
            "/interior/api/items?room=kitchen&limit=5",
            "/interior/api/media/photos/lamp.jpg",
            "/interior/api/roomplan/asset",
            "/axon-status/api/axon-status/health",
            "/axon-status/api/axon-status/start/interior",
            "/axon-status/api/axon-status/capabilities",
            "/calendar/api/entries",
            "/comms/feed?limit=5",
            "/comms/ingest",
            "/devices/api/devices",
            "/finance/api/accounts",
            "/foundation-models/health",
            "/foundation-models/v1/chat/completions",
            "/knowledge-graph/api/graph/unit/x",
            "/macmon/json",
            "/places/api/places",
            "/scouting/opportunities",
            "/transit/health",
            "/api/suggest?q=Berlin",
            "/traveler/api/profile",
            "/trips/api/plans",
            "/vault/api/tasks",
        ] {
            assert!(validate_path(good).is_ok(), "{good} must be admitted");
        }
    }

    #[test]
    fn refuses_non_allow_listed_prefixes() {
        for bad in [
            // Shell-proxied, but the dashboard does not call them.
            "/punctuality/api/x",
            "/soundscape/api/soundscape/stream",
            "/discover/x",
            // The api-only mounts admit nothing outside `/<name>/api/`.
            "/interior",
            "/interior/",
            "/interior/health",
            "/calendar/health",
            "/finance/",
            "/trips/api",
            // A prefix is a segment, not a string prefix.
            "/interiorx/api",
            "/commsx/feed",
            "/apix/suggest",
            // A bare prefix admits nothing: there must be a path below it.
            "/comms/",
            "/devices/",
            "/api/",
            "/",
            "/feed/library",
        ] {
            assert!(validate_path(bad).is_err(), "{bad} must be refused");
        }
    }

    #[test]
    fn every_allow_list_entry_is_a_segment_prefix() {
        for entry in ALLOWED_PATHS {
            assert!(entry.starts_with('/') && entry.ends_with('/'), "{entry}");
            assert!(!entry.contains(".."), "{entry}");
        }
    }

    #[test]
    fn byte_cap_refuses_a_declared_length_above_the_cap() {
        assert!(check_declared_length(None, 10).is_ok());
        assert!(check_declared_length(Some(10), 10).is_ok());
        assert!(check_declared_length(Some(11), 10).is_err());
        assert!(
            check_declared_length(Some(MAX_RESPONSE_BYTES as u64 + 1), MAX_RESPONSE_BYTES).is_err()
        );
        assert_eq!(MAX_RESPONSE_BYTES, 64 * 1024 * 1024);
    }

    #[test]
    fn byte_cap_counts_chunks_and_refuses_the_one_that_passes_it() {
        let mut body = Vec::new();
        assert!(append_capped(&mut body, &[1; 6], 10).is_ok());
        assert!(append_capped(&mut body, &[2; 4], 10).is_ok());
        assert_eq!(body.len(), 10);
        assert!(append_capped(&mut body, &[3], 10).is_err());
        assert_eq!(body.len(), 10, "a refused chunk is not kept");
    }

    #[test]
    fn frame_carries_status_type_and_raw_body() {
        let body = [0u8, 159, 146, 150, 255];
        let framed = frame_bytes(200, Some("image/jpeg"), &body);
        assert_eq!(&framed[0..2], &200u16.to_be_bytes());
        assert_eq!(&framed[2..4], &10u16.to_be_bytes());
        assert_eq!(&framed[4..14], b"image/jpeg");
        assert_eq!(&framed[14..], &body);

        let bare = frame_bytes(404, None, b"");
        assert_eq!(bare, vec![1, 148, 0, 0]);
    }

    #[test]
    fn refuses_traversal_and_absolute_urls() {
        for bad in [
            "/interior/api/../../finance/api/accounts",
            "/interior/../finance/api/accounts",
            "/interior/api/..",
            "/interior/./api",
            "/interior/%2e%2e/finance",
            "/interior/%2E%2E/finance",
            "/interior/api%2f..%2ffinance",
            "/interior\\..\\finance",
            "//evil.example/interior/api",
            "/interior//api",
            "https://evil.example/interior/api",
            "interior/api/items",
            "/interior/api?next=https://evil.example",
            "/interior/api#x",
            "",
        ] {
            assert!(validate_path(bad).is_err(), "{bad:?} must be refused");
        }
    }

    #[test]
    fn target_url_stays_on_the_base_origin() {
        let b = base("https://mac.example-tailnet.ts.net:8443");
        let url = target_url(&b, "/interior/api/items?x=1").unwrap();
        assert_eq!(
            url.as_str(),
            "https://mac.example-tailnet.ts.net:8443/interior/api/items?x=1"
        );
        assert!(target_url(&b, "@evil.example/interior/").is_err());
        assert!(target_url(&b, "/interior/api/items@evil.example")
            .is_ok_and(|u| u.host_str() == Some("mac.example-tailnet.ts.net")));
    }

    #[test]
    fn refuses_header_smuggling() {
        let smuggled = vec![(
            "Content-Type".to_string(),
            "application/json\r\nTailscale-User-Login: attacker@evil.example".to_string(),
        )];
        assert!(filter_headers(&smuggled).is_err());
        let nul = vec![("If-Match".to_string(), "\"abc\"\0".to_string())];
        assert!(filter_headers(&nul).is_err());
    }

    #[test]
    fn forwards_only_safe_headers() {
        let headers = vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            ("If-Match".to_string(), "\"rev-1\"".to_string()),
            ("Accept".to_string(), "image/*".to_string()),
            ("Origin".to_string(), "tauri://localhost".to_string()),
            (
                "Tailscale-User-Login".to_string(),
                "attacker@evil.example".to_string(),
            ),
            ("Authorization".to_string(), "Bearer x".to_string()),
            ("Host".to_string(), "evil.example".to_string()),
        ];
        let kept = filter_headers(&headers).unwrap();
        assert_eq!(
            kept,
            vec![
                ("content-type".to_string(), "application/json".to_string()),
                ("if-match".to_string(), "\"rev-1\"".to_string()),
                ("accept".to_string(), "image/*".to_string()),
            ]
        );
    }

    #[test]
    fn methods_are_a_closed_set() {
        assert!(validate_method("get").is_ok());
        assert!(validate_method("PATCH").is_ok());
        assert!(validate_method("CONNECT").is_err());
        assert!(validate_method("TRACE").is_err());
        assert!(validate_method("GET\r\nX: y").is_err());
    }
}
