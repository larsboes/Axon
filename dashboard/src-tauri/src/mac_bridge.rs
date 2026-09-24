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

use std::path::PathBuf;
use std::time::Duration;

use reqwest::Url;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime};

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
/// Not listed: capabilities the shell proxies but the dashboard does not call
/// (foundation-models, punctuality, soundscape) and scouting's `/discover`.
pub const ALLOWED_PATHS: &[&str] = &[
    "/axon-status/",
    "/calendar/api/",
    "/comms/",
    "/finance/api/",
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

/// The settings file, in the app data directory.
pub const SETTINGS_FILE: &str = "mac-bridge.json";

/// A request that is not answered in this time fails.
const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Settings {
    /// For example `https://<name>.ts.net`. `None` until the operator sets it.
    pub base_url: Option<String>,
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
}

/// The prefix the TypeScript side matches to tell "no base URL" from a
/// network failure. Keep it in sync with `dashboard/src/lib/mac-bridge.ts`.
pub const NOT_CONFIGURED: &str = "mac-bridge: not configured";

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

fn settings_path<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app data directory is not available: {e}"))?;
    Ok(dir.join(SETTINGS_FILE))
}

fn read_settings<R: Runtime>(app: &AppHandle<R>) -> Result<Settings, String> {
    let path = settings_path(app)?;
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| format!("{SETTINGS_FILE}: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
        Err(e) => Err(format!("{SETTINGS_FILE}: {e}")),
    }
}

fn allow_loopback() -> bool {
    cfg!(desktop)
}

#[tauri::command]
pub async fn mac_settings_get<R: Runtime>(app: AppHandle<R>) -> Result<Settings, String> {
    read_settings(&app)
}

/// Stores the base URL. An empty value clears it.
#[tauri::command]
pub async fn mac_settings_set<R: Runtime>(
    app: AppHandle<R>,
    base_url: String,
) -> Result<Settings, String> {
    let settings = if base_url.trim().is_empty() {
        Settings::default()
    } else {
        let url = validate_base(&base_url, allow_loopback())?;
        Settings {
            base_url: Some(url.origin().ascii_serialization()),
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

/// Checks one request and builds it. Every command goes through here, so
/// every command applies the same rules.
fn build_request<R: Runtime>(
    app: &AppHandle<R>,
    method: &str,
    path: &str,
    headers: &[(String, String)],
) -> Result<reqwest::RequestBuilder, String> {
    let settings = read_settings(app)?;
    let Some(base) = settings.base_url.filter(|b| !b.trim().is_empty()) else {
        return Err(format!(
            "{NOT_CONFIGURED}: set the Mac address in Settings, Mac connection"
        ));
    };
    let base = validate_base(&base, allow_loopback())?;
    let url = target_url(&base, path)?;
    let method = validate_method(method)?;
    let headers = filter_headers(headers)?;

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(TIMEOUT)
        .build()
        .map_err(|e| format!("mac-bridge: {e}"))?;
    let mut builder = client.request(method, url);
    for (name, value) in headers {
        builder = builder.header(name, value);
    }
    Ok(builder)
}

fn content_type_of(response: &reqwest::Response) -> Option<String> {
    response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

#[tauri::command]
pub async fn mac_request<R: Runtime>(
    app: AppHandle<R>,
    request: MacRequest,
) -> Result<MacResponse, String> {
    let mut builder = build_request(&app, &request.method, &request.path, &request.headers)?;
    if let Some(body) = request.body {
        builder = builder.body(body);
    }
    let response = builder
        .send()
        .await
        .map_err(|e| format!("mac-bridge: the Mac did not answer: {e}"))?;
    let status = response.status().as_u16();
    let content_type = content_type_of(&response);
    let body = response
        .text()
        .await
        .map_err(|e| format!("mac-bridge: reading the answer failed: {e}"))?;
    Ok(MacResponse {
        status,
        content_type,
        body,
    })
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
/// same path, header and base rules as [`mac_request`].
#[tauri::command]
pub async fn mac_request_bytes<R: Runtime>(
    app: AppHandle<R>,
    path: String,
    headers: Option<Vec<(String, String)>>,
) -> Result<tauri::ipc::Response, String> {
    let headers = headers.unwrap_or_default();
    let builder = build_request(&app, "GET", &path, &headers)?;
    let mut response = builder
        .send()
        .await
        .map_err(|e| format!("mac-bridge: the Mac did not answer: {e}"))?;
    check_declared_length(response.content_length(), MAX_RESPONSE_BYTES)?;
    let status = response.status().as_u16();
    let content_type = content_type_of(&response);
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| format!("mac-bridge: reading the answer failed: {e}"))?
    {
        append_capped(&mut body, &chunk, MAX_RESPONSE_BYTES)?;
    }
    Ok(tauri::ipc::Response::new(frame_bytes(
        status,
        content_type.as_deref(),
        &body,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base(url: &str) -> Url {
        validate_base(url, true).expect("valid base")
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
            "/finance/api/accounts",
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
            "/foundation-models/health",
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
