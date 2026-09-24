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
pub const ALLOWED_PATHS: &[&str] = &["/interior/", "/axon-status/api/axon-status/health"];

/// The request headers the bridge forwards. Every other name is dropped.
pub const FORWARDED_HEADERS: &[&str] = &["content-type", "if-match"];

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

#[tauri::command]
pub async fn mac_request<R: Runtime>(
    app: AppHandle<R>,
    request: MacRequest,
) -> Result<MacResponse, String> {
    let settings = read_settings(&app)?;
    let Some(base) = settings.base_url.filter(|b| !b.trim().is_empty()) else {
        return Err(format!(
            "{NOT_CONFIGURED}: set the Mac address in Settings, Mac connection"
        ));
    };
    let base = validate_base(&base, allow_loopback())?;
    let url = target_url(&base, &request.path)?;
    let method = validate_method(&request.method)?;
    let headers = filter_headers(&request.headers)?;

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(TIMEOUT)
        .build()
        .map_err(|e| format!("mac-bridge: {e}"))?;
    let mut builder = client.request(method, url);
    for (name, value) in headers {
        builder = builder.header(name, value);
    }
    if let Some(body) = request.body {
        builder = builder.body(body);
    }
    let response = builder
        .send()
        .await
        .map_err(|e| format!("mac-bridge: the Mac did not answer: {e}"))?;
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
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
        assert!(validate_path("/interior/api/items").is_ok());
        assert!(validate_path("/interior/api/items?room=kitchen&limit=5").is_ok());
        assert!(validate_path("/axon-status/api/axon-status/health").is_ok());
    }

    #[test]
    fn refuses_non_allow_listed_prefixes() {
        for bad in [
            "/finance/api/accounts",
            "/interior",
            "/interior/",
            "/interiorx/api",
            "/axon-status/api/axon-status/healthz",
            "/axon-status/api/axon-status/services",
            "/",
        ] {
            assert!(validate_path(bad).is_err(), "{bad} must be refused");
        }
    }

    #[test]
    fn refuses_traversal_and_absolute_urls() {
        for bad in [
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
