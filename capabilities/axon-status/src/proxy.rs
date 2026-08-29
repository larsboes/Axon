//! The shell's front door: capability routes, then the built dashboard bundle.
//!
//! # Why this is here rather than in the dashboard
//!
//! The dashboard is an `adapter-static` SPA whose every API call is a *relative* fetch
//! (`dashboard/src/lib/api.ts` `request()`), so something has to map `/comms/...` onto
//! `127.0.0.1:8083`. Until now that something was Vite's dev-server proxy, which exists only
//! when `command === "serve"` (`dashboard/vite.config.ts`) -- `preview` declares none. So the
//! built bundle has never had a working API path in any mode, and "serve the bundle statically"
//! was not a swap: it needed this file first.
//!
//! Supervising a dev server to get a reverse proxy cost 95.5 MB across node, esbuild and bun,
//! against 24.9 MB for all three Rust servers combined. axon-status already had to stay up, it
//! already resolves the registry (`status/registry.rs`), and it already links a reqwest client.
//!
//! # Routing, and why `fallback` rather than a layer
//!
//! Proxy matching runs as the router's fallback, so axon-status' own routes always win. That is
//! not a stylistic choice. `transit` declares `proxy_extra = ["/api"]`, and a bare `/api` prefix
//! evaluated *before* routing would swallow `/api/axon-status/health` and send this surface's own
//! health to transit. As a fallback it cannot: the real route matches first, and only genuinely
//! unrouted paths are offered to the proxy.
//!
//! The rules mirror the Vite proxy exactly, because the shell is the same shell:
//!
//! - `/<name>` reaches the capability with the prefix stripped, so a capability's own contract
//!   never has to know it is proxied. `proxy_api_only` narrows the mount to `/<name>/api`.
//! - `proxy_extra` prefixes pass through *unstripped* -- transit's `/api`, scouting's `/discover`
//!   are paths that predate the uniform rule.
//! - Spine and portless entries are skipped, as they are there.
//!
//! One rule Vite never needed: **this surface must not proxy to itself.** `/axon-status` resolves
//! to the port this process is listening on, and forwarding to it would re-enter this fallback and
//! loop until the connection pool gave out. Excluded by port, not by name, so a rename cannot
//! reintroduce it.

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use std::sync::Arc;
use tower::ServiceExt;
use tower_http::services::{ServeDir, ServeFile};

/// One prefix and where it goes. `strip` is the difference between the uniform rule and the
/// grandfathered `proxy_extra` paths, and it is the only difference.
#[derive(Clone, Debug)]
pub(crate) struct Route {
    pub(crate) prefix: String,
    pub(crate) target: String,
    /// What to remove from the front of the path, which is NOT the same as `prefix`.
    ///
    /// `proxy_api_only` mounts a capability at `/<name>/api` while still stripping only
    /// `/<name>`, so `/finance/api/finance/health` reaches finance as `/api/finance/health`.
    /// Conflating the two sent it `/finance/health` and got a 404 — caught by exercising the
    /// route rather than by reading it, because both readings look right in prose.
    ///
    /// `None` for a `proxy_extra` prefix, which passes through untouched.
    pub(crate) strip: Option<String>,
    /// Comms is the only capability whose proxy adds a credential, and the flag is set from the
    /// registry entry rather than re-derived at request time. An earlier draft of this file
    /// decided it by comparing the target against a helper that returned an empty string, and
    /// `ends_with("")` is true for every string — which would have put the comms bearer token on
    /// every request to every capability. Carried as data on the route so there is one place it
    /// can be true.
    pub(crate) inject_comms_auth: bool,
}

#[derive(Clone)]
pub(crate) struct Proxy {
    routes: Arc<Vec<Route>>,
    /// Comms authenticates every route except `/health` and `/ready`
    /// (`libs/axon-server/src/auth.rs`). Resolved once at startup with
    /// `axon_server::auth::token_from_file`, which is the same reader the server itself uses --
    /// so this is not a fourth implementation of the token shape, it is the first Rust consumer
    /// of the one that already existed. `None` stays fail-closed: comms answers 401 and the page
    /// says so, which is the honest outcome of an unconfigured credential.
    comms_authorization: Option<HeaderValue>,
    client: reqwest::Client,
    ui_dir: String,
}

/// Hop-by-hop headers, which belong to one connection and must not be forwarded across another
/// (RFC 9110 §7.6.1). Leaving `connection` or `transfer-encoding` on a proxied response is how a
/// client is told to expect framing the proxy has already undone.
const HOP_BY_HOP: [&str; 8] = [
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

fn is_hop_by_hop(name: &HeaderName) -> bool {
    HOP_BY_HOP.iter().any(|h| name.as_str().eq_ignore_ascii_case(h))
}

/// Does `path` sit under `prefix` as a *segment*, rather than merely starting with its letters?
///
/// `/comms` must match `/comms` and `/comms/api/feed` and must not match `/commsomething`. A bare
/// `starts_with` says yes to the third, which would route one capability's traffic to another and
/// report it as a 404 from the wrong server.
fn matches(path: &str, prefix: &str) -> bool {
    path == prefix || (path.starts_with(prefix) && path[prefix.len()..].starts_with('/'))
}

impl Proxy {
    pub(crate) fn new(services: &[crate::status::Service], own_port: &str, ui_dir: String) -> Self {
        let mut routes: Vec<Route> = Vec::new();
        for svc in services {
            // Same two exclusions as the Vite proxy: the spine is the shell itself, and a
            // capability with no port has no HTTP surface to reach.
            if svc.scope == "spine" || svc.port.is_empty() {
                continue;
            }
            // The exclusion Vite did not need. Compared by port so a rename cannot smuggle a
            // self-route back in.
            // Compared on BOTH, because either alone has a hole: a port override would let the
            // name back in, and a rename would let the port back in.
            if svc.port == own_port || svc.name == "axon-status" {
                continue;
            }
            let target = format!("http://127.0.0.1:{}", svc.port);
            let mount = if svc.proxy_api_only == "true" {
                format!("/{}/api", svc.name)
            } else {
                format!("/{}", svc.name)
            };
            let comms = svc.name == "comms";
            routes.push(Route {
                prefix: mount,
                target: target.clone(),
                strip: Some(format!("/{}", svc.name)),
                inject_comms_auth: comms,
            });
            for extra in &svc.proxy_extra {
                routes.push(Route {
                    prefix: extra.clone(),
                    target: target.clone(),
                    strip: None,
                    inject_comms_auth: comms,
                });
            }
        }
        // Longest prefix first, so `/trips/api` is considered before a hypothetical `/trips`.
        routes.sort_by(|a, b| b.prefix.len().cmp(&a.prefix.len()));

        Self {
            routes: Arc::new(routes),
            comms_authorization: comms_authorization(),
            client: reqwest::Client::new(),
            ui_dir,
        }
    }

    fn route_for(&self, path: &str) -> Option<&Route> {
        self.routes.iter().find(|r| matches(path, &r.prefix))
    }

    pub(crate) fn route_count(&self) -> usize {
        self.routes.len()
    }
}

/// The comms credential, read from the same config the comms server reads.
fn comms_authorization() -> Option<HeaderValue> {
    // The same three candidates comms itself resolves, in the same order
    // (`capabilities/comms/src/config.rs`). Re-derived rather than imported because axon-status
    // does not depend on comms and must not start doing so to read one path.
    let path = if let Ok(p) = std::env::var("AXON_COMMS_CONFIG") {
        axon_config::expand_tilde(&p)
    } else if let Some(p) = axon_config::overlay_config("comms.json") {
        p
    } else {
        std::path::PathBuf::from("capabilities/comms/comms.config.json")
    };
    let token = axon_server::token_from_file(&path)?;
    HeaderValue::from_str(&format!("Bearer {token}")).ok()
}

/// Router fallback: proxy when a prefix claims the path, otherwise serve the SPA.
pub(crate) async fn fallback(State(proxy): State<Proxy>, req: Request) -> Response {
    let path = req.uri().path().to_string();
    match proxy.route_for(&path) {
        Some(route) => forward(&proxy, route.clone(), req).await,
        // Not a capability path, so it is a page. `adapter-static` emits one entry point and
        // routes client-side, which makes an unknown path a route rather than a 404.
        //
        // The trap, since it cost a misread: an API path for a capability with NO route falls here
        // and answers 200 with `text/html`. `GET /foundation-models/health` looked like a passing
        // health check and was the shell's index page. The routing table is resolved once at
        // startup, so a capability enabled afterwards has no route until this process restarts —
        // which is the documented behaviour above, not a bug, but it fails as a plausible success.
        // When a capability path answers HTML, restart axon-status before believing anything else.
        None => serve_ui(&proxy.ui_dir, req).await,
    }
}

async fn serve_ui(dir: &str, req: Request) -> Response {
    let index = format!("{dir}/index.html");
    match ServeDir::new(dir).fallback(ServeFile::new(index)).oneshot(req).await {
        Ok(res) => res.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("static: {e}")).into_response(),
    }
}

async fn forward(proxy: &Proxy, route: Route, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let path = parts.uri.path();
    let rest = match &route.strip {
        Some(p) if path.starts_with(p.as_str()) => {
            let stripped = &path[p.len()..];
            if stripped.is_empty() { "/" } else { stripped }
        }
        _ => path,
    };
    let query = parts.uri.query().map(|q| format!("?{q}")).unwrap_or_default();
    let url = format!("{}{}{}", route.target, rest, query);

    // Buffered rather than streamed. Every request through here is a dashboard API call, the
    // largest of which is a CSV import measured in kilobytes, and buffering keeps the failure
    // mode legible. The RESPONSE is streamed below, because that side has soundscape's
    // `/api/soundscape/stream` on it and a buffered SSE response is a hung page.
    let bytes = match axum::body::to_bytes(body, 8 * 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => return (StatusCode::PAYLOAD_TOO_LARGE, "request body too large").into_response(),
    };

    let mut headers = HeaderMap::new();
    for (name, value) in parts.headers.iter() {
        if is_hop_by_hop(name) || name == axum::http::header::HOST {
            continue;
        }
        headers.insert(name.clone(), value.clone());
    }
    if route.inject_comms_auth {
        if let Some(auth) = &proxy.comms_authorization {
            headers.insert(axum::http::header::AUTHORIZATION, auth.clone());
        }
    }

    let upstream = proxy
        .client
        .request(parts.method.clone(), &url)
        .headers(headers)
        .body(bytes)
        .send()
        .await;

    let upstream = match upstream {
        Ok(r) => r,
        // A capability that is not running is the normal case on a machine where most things are
        // on demand, so this says which one and stays a 502 rather than a panic.
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                format!("{} is not answering: {e}", route.target),
            )
                .into_response()
        }
    };

    let status = upstream.status();
    let mut out = Response::builder().status(status);
    for (name, value) in upstream.headers().iter() {
        if is_hop_by_hop(name) {
            continue;
        }
        out = out.header(name, value);
    }
    let stream = upstream.bytes_stream();
    out.body(Body::from_stream(stream))
        .unwrap_or_else(|e| (StatusCode::BAD_GATEWAY, format!("upstream: {e}")).into_response())
}


#[cfg(test)]
mod tests {
    use super::*;

    fn svc(name: &str, port: &str, api_only: bool, extra: &[&str]) -> crate::status::Service {
        serde_json::from_value(serde_json::json!({
            "name": name, "kind": "process", "scope": "capability", "port": port,
            "health_path": "/health", "panel_port": "", "panel_path": "", "autostart": "",
            "proxy_api_only": if api_only { "true" } else { "" },
            "proxy_extra": extra,
        }))
        .expect("registry fixture")
    }

    fn table(services: &[crate::status::Service]) -> Vec<Route> {
        Proxy::new(services, "8082", "dist".into()).routes.as_ref().clone()
    }

    /// The rule that broke in the first draft. `proxy_api_only` moves the MOUNT to
    /// `/<name>/api` while still stripping only `/<name>` — so finance is reached at
    /// `/finance/api/subscriptions` and receives `/api/subscriptions`. Stripping the mount
    /// instead sent it `/subscriptions` and produced a 404 that looked like a finance bug.
    #[test]
    fn api_only_mounts_deeper_than_it_strips() {
        let t = table(&[svc("finance", "8090", true, &[])]);
        let r = &t[0];
        assert_eq!(r.prefix, "/finance/api");
        assert_eq!(r.strip.as_deref(), Some("/finance"));
    }

    /// A `proxy_extra` prefix is a path that predates the uniform rule, so it passes through
    /// with nothing removed.
    #[test]
    fn extras_pass_through_unstripped() {
        let t = table(&[svc("scouting", "8084", false, &["/discover"])]);
        let extra = t.iter().find(|r| r.prefix == "/discover").expect("extra route");
        assert!(extra.strip.is_none());
    }

    /// A prefix must match on a segment boundary. `starts_with` alone routes `/punctualityXX`
    /// into punctuality, which answers 404 for a path the shell meant as a page.
    #[test]
    fn prefixes_match_on_segment_boundaries() {
        assert!(matches("/comms", "/comms"));
        assert!(matches("/comms/api/feed", "/comms"));
        assert!(!matches("/commsomething", "/comms"));
        assert!(!matches("/com", "/comms"));
    }

    /// Forwarding to our own port re-enters this fallback and loops. Excluded on name and port
    /// both, because either alone has a hole: a port override lets the name back in, a rename
    /// lets the port back in.
    #[test]
    fn never_proxies_to_itself() {
        let t = table(&[svc("axon-status", "8082", false, &[]), svc("comms", "8083", false, &[])]);
        assert!(t.iter().all(|r| r.prefix != "/axon-status"));
        assert!(t.iter().any(|r| r.prefix == "/comms"));
    }

    /// The spine is this process, and a capability with no port has no HTTP surface. Same two
    /// exclusions the Vite proxy made.
    #[test]
    fn skips_the_spine_and_the_portless() {
        let mut spine = svc("dashboard", "47117", false, &[]);
        spine.scope = "spine".into();
        let t = table(&[spine, svc("host-watch", "", false, &[])]);
        assert!(t.is_empty());
    }

    /// Only comms gets a credential. An earlier draft decided this by comparing the target
    /// against a helper returning "", and `ends_with("")` is true for everything — which would
    /// have put the comms bearer token on every request to every capability.
    #[test]
    fn only_comms_carries_the_credential() {
        let t = table(&[svc("comms", "8083", false, &[]), svc("places", "8093", false, &[])]);
        for r in &t {
            assert_eq!(r.inject_comms_auth, r.prefix.starts_with("/comms"), "{}", r.prefix);
        }
    }

    /// Longest first, so a mount is considered before any shorter prefix that also matches.
    #[test]
    fn longest_prefix_wins() {
        let t = table(&[svc("trips", "8086", true, &["/t"])]);
        assert!(t[0].prefix.len() >= t[t.len() - 1].prefix.len());
    }
}
