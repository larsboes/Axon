//! The inbound gate: one shared-secret check for every capability server.
//!
//! ## Why this is here and not in each capability
//!
//! Until this module existed, exactly one of twelve Rust capabilities
//! authenticated an inbound request — comms, whose `src/server/auth.rs` carried
//! a constant-time Bearer / `X-Axon-Token` check on its mutating routes. The
//! other eleven relied entirely on the loopback bind, including axon-status,
//! which serves `POST /api/axon-status/capabilities/:name/start|stop`: process
//! control. "Reachable from the phone" and "unauthenticated process control"
//! cannot both be true, so the check moved to the crate all twelve already
//! route their startup through. A second copy of this check is the drift the
//! repo's third principle forbids; comms now calls this one.
//!
//! ## The contract
//!
//! | Configured token | `/health`, `/ready`, `OPTIONS` | Every other route | Reach beyond loopback |
//! |---|---|---|---|
//! | yes | served | `401` without a matching token | permitted |
//! | no | served | served (or `403`, see below) | **refused at bind** |
//!
//! `Reach::AllInterfaces` without a token has no representation:
//! [`crate::bind_addr_for`] is the only constructor of a non-loopback
//! `SocketAddr` in this crate and it returns `Err` for that pairing, while
//! doctor's "Server bind policy" gate fails any capability source that builds
//! its own listener (README.md, "What actually enforces this").
//!
//! ## Token sourcing: shared, not per-capability
//!
//! One token for the whole deployment, because the thing it gates is one thing:
//! whether an inbound request reached this machine legitimately. Twelve tokens
//! would be twelve secrets for one boundary and twelve injections in every
//! client that fans out across capabilities — the dashboard's Vite proxy and
//! axon-status' `/routes` aggregation both do exactly that.
//!
//! The value is referenced, never inlined, following the pattern comms
//! established for `api_secret_file`: `<overlay>/config/deployment.env`
//! declares `AXON_INBOUND_TOKEN_FILE=<path>` and the token is the contents of
//! that private file (`schemas/deployment.env.example`). A path is not a
//! secret, which is why the reference may live in a tracked-shape file while
//! the value may not.
//!
//! A capability may still supply its own token to [`InboundAuth::resolve`] and
//! it wins — comms' pre-existing `api_secret_file` is the one caller that does.
//! A deployment converges the two by pointing both references at one file.

use std::path::Path;

use axum::extract::{Request, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde_json::json;

/// Paths that answer before the gate. Liveness and readiness are what a proxy,
/// the runner and axon-status poll to find out whether a process is alive at
/// all; behind a token they would report a healthy capability as down, and the
/// answer carries nothing an unauthenticated caller could not learn by
/// observing that the port accepts a connection.
const EXEMPT_PATHS: &[&str] = &["/health", "/ready"];

/// The resolved inbound gate for one server.
///
/// Cloned per request by axum's `State`, so the token is a `String` rather than
/// a borrow. Never `Debug`-printed with its value — see the manual impl below.
#[derive(Clone, Default)]
pub struct InboundAuth {
    token: Option<String>,
    /// `true` when the absence of a token must close the non-exempt routes
    /// rather than leave them open. See [`InboundAuth::refuse_without_token`].
    refuse_without_token: bool,
}

/// Redacts the token. A capability that logs its own config must not turn this
/// value into a line in the runner's captured stderr.
impl std::fmt::Debug for InboundAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InboundAuth")
            .field("token", &self.token.as_ref().map(|_| "<redacted>"))
            .field("refuse_without_token", &self.refuse_without_token)
            .finish()
    }
}

impl InboundAuth {
    /// The deployment-wide token, or none.
    ///
    /// Reads `AXON_INBOUND_TOKEN_FILE` from `<overlay>/config/deployment.env`
    /// and then that file. Every step is allowed to be absent: an overlay that
    /// has not declared a token yields `None`, which is the loopback-only
    /// deployment that predates this gate.
    pub fn from_deployment() -> Self {
        Self::resolve(None)
    }

    /// A capability-supplied token wins over the deployment-wide one.
    ///
    /// Precedence, not merging, and deliberately not the conflict error
    /// `axon_config::resolve_home_timezone` raises: two timezones are a
    /// mistake, whereas two tokens are a deployment mid-rotation or a
    /// capability whose clients predate the shared file. Both are legitimate.
    pub fn resolve(capability_token: Option<String>) -> Self {
        let token = capability_token
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .or_else(deployment_token);
        Self {
            token,
            refuse_without_token: false,
        }
    }

    /// No I/O: the token exactly as given. For tests and for a capability that
    /// has already resolved its own value.
    pub fn with_token(token: Option<String>) -> Self {
        Self {
            token: token
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty()),
            refuse_without_token: false,
        }
    }

    /// Answer `403` on the non-exempt routes when no token is configured,
    /// instead of serving them.
    ///
    /// For a capability whose routes must not run for an unauthenticated caller
    /// even on loopback. comms is the reason this exists: `POST /ingest` fetches
    /// an attacker-chosen URL, and a page open in the operator's own browser is
    /// already inside the loopback boundary, so `127.0.0.1` is not what contains
    /// that route — the token is.
    pub fn refuse_without_token(mut self) -> Self {
        self.refuse_without_token = true;
        self
    }

    /// Whether a token was resolved. `false` is what confines a server to
    /// loopback ([`crate::bind_addr_for`]).
    pub fn is_configured(&self) -> bool {
        self.token.is_some()
    }

    /// `Bearer <token>`, for a process that calls a sibling capability through
    /// this same gate — axon-status polling `/routes` is the only one today.
    /// `None` when no token is configured, which is also when no sibling
    /// requires one.
    pub fn bearer_header(&self) -> Option<String> {
        self.token.as_ref().map(|t| format!("Bearer {t}"))
    }

    /// Whether this gate rejects anything at all. A gate with no token and no
    /// refusal is not layered onto the router, so an unconfigured deployment
    /// pays nothing per request.
    fn gates_anything(&self) -> bool {
        self.token.is_some() || self.refuse_without_token
    }

    /// `Some(rejection)` when this request must not reach a handler.
    ///
    /// Split out of the middleware so the policy is testable without a socket.
    fn reject(&self, method: &Method, path: &str, headers: &HeaderMap) -> Option<Response> {
        // CORS preflight carries no Authorization header by construction — the
        // browser strips it — so gating OPTIONS would reject every cross-origin
        // request from the dashboard before the CORS layer inside this one ever
        // answered. Nothing is disclosed: the real request still needs the
        // token, and axum answers an unrouted method with 405, not a handler.
        if method == Method::OPTIONS || EXEMPT_PATHS.contains(&path) {
            return None;
        }
        let Some(expected) = self.token.as_deref() else {
            if self.refuse_without_token {
                return Some(
                    (
                        StatusCode::FORBIDDEN,
                        Json(json!({
                            "error": "no inbound token is configured — these routes are disabled. Declare AXON_INBOUND_TOKEN_FILE in <overlay>/config/deployment.env."
                        })),
                    )
                        .into_response(),
                );
            }
            return None;
        };
        match presented_token(headers) {
            Some(t) if constant_time_eq(t.as_bytes(), expected.as_bytes()) => None,
            _ => Some(
                (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({ "error": "invalid or missing authentication token" })),
                )
                    .into_response(),
            ),
        }
    }
}

/// Wraps `router` in the inbound gate.
///
/// Returns the router untouched when the gate would reject nothing, so a
/// deployment with no token keeps exactly the request path it had before this
/// module existed.
///
/// Applied outermost, above whatever CORS layer the capability built: a request
/// that fails the token check must not consume a handler, and the exemption for
/// `OPTIONS` in [`InboundAuth::reject`] is what keeps preflight working from
/// underneath.
pub fn authenticated(router: Router, auth: InboundAuth) -> Router {
    if !auth.gates_anything() {
        return router;
    }
    router.layer(axum::middleware::from_fn_with_state(auth, gate))
}

async fn gate(State(auth): State<InboundAuth>, request: Request, next: Next) -> Response {
    match auth.reject(request.method(), request.uri().path(), request.headers()) {
        Some(rejection) => rejection,
        None => next.run(request).await,
    }
}

/// `Authorization: Bearer <token>` first, then `X-Axon-Token: <token>`.
///
/// Two header forms because two kinds of client call these ports: HTTP tooling
/// and proxies that already speak `Authorization`, and the browser extension /
/// `curl` callers for which a dedicated header is one fewer thing to get wrong.
fn presented_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .or_else(|| headers.get("x-axon-token").and_then(|v| v.to_str().ok()))
}

/// Compares every byte regardless of where the first difference is.
///
/// A short-circuiting `==` leaks the length of the matching prefix through
/// response time, which turns guessing a token from an exhaustive search into a
/// per-character one. The length check ahead of it leaks only the length, which
/// an attacker who can send a token already knows how to measure another way.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

/// `AXON_INBOUND_TOKEN_FILE` from `<overlay>/config/deployment.env`, then that
/// file's contents.
fn deployment_token() -> Option<String> {
    let body = std::fs::read_to_string(axon_config::overlay_config("deployment.env")?).ok()?;
    let reference = body.lines().find_map(|l| {
        l.strip_prefix("AXON_INBOUND_TOKEN_FILE=")
            .map(str::trim)
            .filter(|v| !v.is_empty())
    })?;
    token_from_file(&axon_config::expand_tilde(reference))
}

/// Reads a token out of a private file: the trimmed contents, or `auth.api_key`
/// when the file is JSON.
///
/// The JSON form is not decoration — comms' `api_secret_file` has always been
/// allowed to point at an existing settings file rather than a bare token, and
/// this is that reader, moved up so Rust has one implementation of it. The
/// third implementation, `tokenFromBody` in `dashboard/vite/comms-proxy-auth.ts`,
/// cannot share code across the language boundary and says so at its own site.
///
/// `None` for absent, unreadable or empty: a token that failed to load must
/// never be mistaken for a token that matched.
pub fn token_from_file(path: &Path) -> Option<String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(
                axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                value.parse().unwrap(),
            );
        }
        map
    }

    fn status(auth: &InboundAuth, method: Method, path: &str, sent: &[(&str, &str)]) -> u16 {
        match auth.reject(&method, path, &headers(sent)) {
            Some(response) => response.status().as_u16(),
            None => 200,
        }
    }

    #[test]
    fn a_configured_token_gates_every_route_except_health_and_ready() {
        let auth = InboundAuth::with_token(Some("s3cret".into()));
        assert_eq!(status(&auth, Method::GET, "/health", &[]), 200);
        assert_eq!(status(&auth, Method::GET, "/ready", &[]), 200);
        assert_eq!(status(&auth, Method::GET, "/routes", &[]), 401);
        assert_eq!(status(&auth, Method::GET, "/feed", &[]), 401);
        assert_eq!(
            status(
                &auth,
                Method::POST,
                "/api/axon-status/capabilities/comms/start",
                &[]
            ),
            401
        );
    }

    #[test]
    fn both_header_forms_are_accepted_and_a_wrong_value_in_either_is_not() {
        let auth = InboundAuth::with_token(Some("s3cret".into()));
        for (name, good, bad) in [
            ("authorization", "Bearer s3cret", "Bearer wrong"),
            ("x-axon-token", "s3cret", "wrong"),
        ] {
            assert_eq!(status(&auth, Method::GET, "/feed", &[(name, good)]), 200);
            assert_eq!(status(&auth, Method::GET, "/feed", &[(name, bad)]), 401);
        }
    }

    /// The browser strips `Authorization` from a preflight, so gating OPTIONS
    /// would break every cross-origin call the dashboard makes.
    #[test]
    fn cors_preflight_passes_through_to_the_cors_layer_underneath() {
        let auth = InboundAuth::with_token(Some("s3cret".into()));
        assert_eq!(status(&auth, Method::OPTIONS, "/ingest", &[]), 200);
    }

    #[test]
    fn without_a_token_a_server_serves_as_it_did_before_this_gate() {
        let auth = InboundAuth::with_token(None);
        assert_eq!(status(&auth, Method::GET, "/feed", &[]), 200);
        assert!(!auth.is_configured());
    }

    /// comms' contract: an absent secret closes the route rather than opening it.
    #[test]
    fn refuse_without_token_closes_the_non_exempt_routes_instead_of_opening_them() {
        let auth = InboundAuth::with_token(None).refuse_without_token();
        assert_eq!(status(&auth, Method::POST, "/ingest", &[]), 403);
        assert_eq!(status(&auth, Method::GET, "/health", &[]), 200);
    }

    /// An empty string is what an unset `api_secret_file` resolves to, and it
    /// must not become a token that any empty header would match.
    #[test]
    fn an_empty_token_counts_as_no_token() {
        let auth = InboundAuth::with_token(Some("  ".into())).refuse_without_token();
        assert!(!auth.is_configured());
        assert_eq!(status(&auth, Method::POST, "/ingest", &[]), 403);
    }

    #[test]
    fn the_comparison_reads_every_byte_and_rejects_a_matching_prefix() {
        assert!(constant_time_eq(b"s3cret", b"s3cret"));
        assert!(!constant_time_eq(b"s3cret", b"s3crev"));
        assert!(!constant_time_eq(b"s3cret", b"s3cre"));
        assert!(!constant_time_eq(b"s3cret", b"s3cretx"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn a_token_file_is_read_raw_or_as_the_api_key_of_a_json_settings_file() {
        let dir = std::env::temp_dir().join(format!(
            "axon-server-token-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let raw = dir.join("raw");
        std::fs::write(&raw, "  s3cret\n").unwrap();
        assert_eq!(token_from_file(&raw).as_deref(), Some("s3cret"));

        let settings = dir.join("settings.json");
        std::fs::write(&settings, r#"{"auth":{"api_key":"s3cret"}}"#).unwrap();
        assert_eq!(token_from_file(&settings).as_deref(), Some("s3cret"));

        let empty = dir.join("empty");
        std::fs::write(&empty, "\n \n").unwrap();
        assert_eq!(token_from_file(&empty), None);

        assert_eq!(token_from_file(&dir.join("absent")), None);

        let _ = std::fs::remove_dir_all(dir);
    }

    /// The value is a live credential; a capability that dumps its config must
    /// not put it in the runner's stderr.
    #[test]
    fn debug_never_prints_the_token() {
        let rendered = format!("{:?}", InboundAuth::with_token(Some("s3cret".into())));
        assert!(!rendered.contains("s3cret"), "got: {rendered}");
    }
}
