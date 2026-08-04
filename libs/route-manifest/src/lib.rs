//! What a capability answers, as data it serves about itself.
//!
//! Every capability exposes `GET /routes` beside `/health`. `/health` is the
//! one path all of them already share regardless of their other conventions,
//! and it is reachable whether or not the manifest proxies API paths only — so
//! it is the right neighbour for the endpoint that says what else is here.
//!
//! ## Why this exists
//!
//! Axon's HTTP surface grew five conventions across seven capabilities: `/api/…`
//! behind a proxy, bare paths, self-prefixed `/api/<name>/…`, and transit
//! serving both `/health` and `/api/health`. Renaming all of that is churn with
//! a large blast radius; **not being able to find out what exists** was the
//! part that actually cost time. This fixes that half directly, and leaves the
//! naming free to converge later without a flag day.
//!
//! ## Drift is the whole risk
//!
//! A hand-written list of endpoints is wrong the first time someone adds a
//! route and forgets it — and a stale manifest is worse than none, because it
//! is believed. `undeclared_routes` reads the server's own source at compile
//! time via `include_str!` and reports anything the router serves that the
//! manifest does not mention, so the test fails instead of the manifest lying.
//!
//! ## Dependency rule
//!
//! Compiled into consumers by `#[path]` include (see `libs/axon-config/README.md`),
//! so it may only use crates every consumer already has: `serde` and `serde_json`.

use serde::Serialize;
use serde_json::{json, Value};

/// One endpoint, described for whoever is trying to use it rather than for
/// whoever wrote it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Route {
    pub method: &'static str,
    pub path: &'static str,
    /// One line, in the imperative: what calling this gets you. Query
    /// parameters belong here when they are required — a path alone does not
    /// tell a caller that `from` and `to` are mandatory.
    pub summary: &'static str,
}

/// The body `GET /routes` returns.
pub fn manifest(capability: &str, routes: &[Route]) -> Value {
    json!({
        "capability": capability,
        "routes": routes,
    })
}

/// Paths the router serves that the manifest does not declare.
///
/// `source` is the server's own text, passed in by the caller as
/// `include_str!("server.rs")` — this lib cannot reach the consumer's files, and
/// having the caller name its own source keeps the check honest about what it
/// actually read.
///
/// Deliberately one-directional. A manifest entry with no matching `.route()`
/// is not reported, because a capability may legitimately describe a path its
/// router mounts indirectly; a *served* path that nobody documented is the
/// failure that leaves a caller guessing.
pub fn undeclared_routes(source: &str, routes: &[Route]) -> Vec<String> {
    let mut missing: Vec<String> = Vec::new();
    for served in served_paths(source) {
        let declared = routes.iter().any(|route| route.path == served);
        if !declared && !missing.iter().any(|path| path == &served) {
            missing.push(served);
        }
    }
    missing.sort();
    missing
}

/// Every path literal passed to `.route(` in the given source.
///
/// Text matching rather than parsing, and that is a deliberate trade: it cannot
/// see a route built from a runtime string, and it says so here rather than
/// pretending to be exhaustive. Every router in this repo passes a literal.
fn served_paths(source: &str) -> Vec<String> {
    const MARKER: &str = ".route(\"";
    let mut paths = Vec::new();
    let mut rest = source;
    while let Some(start) = rest.find(MARKER) {
        rest = &rest[start + MARKER.len()..];
        match rest.find('"') {
            Some(end) => {
                paths.push(rest[..end].to_string());
                rest = &rest[end..];
            }
            // An unterminated literal cannot happen in source that compiles;
            // stopping is still better than looping.
            None => break,
        }
    }
    paths
}

// Gated on the standalone-tests feature, not bare cfg(test), matching
// libs/axon-config and libs/axon-server: this file is compiled into every
// consumer by #[path] include, and a lib's own suite has no business running
// inside each consumer's test binary. //libs/route-manifest:route_manifest_test
// sets the feature and runs them.
#[cfg(all(test, feature = "standalone-tests"))]
mod tests {
    use super::*;

    const ROUTES: &[Route] = &[
        Route {
            method: "GET",
            path: "/health",
            summary: "Liveness.",
        },
        Route {
            method: "GET",
            path: "/api/things",
            summary: "Every thing.",
        },
    ];

    #[test]
    fn a_served_path_missing_from_the_manifest_is_reported() {
        let source = r#"
            Router::new()
                .route("/health", get(health))
                .route("/api/things", get(list))
                .route("/api/things/:id", get(one))
        "#;
        assert_eq!(undeclared_routes(source, ROUTES), vec!["/api/things/:id"]);
    }

    #[test]
    fn a_fully_declared_router_reports_nothing() {
        let source = r#".route("/health", get(h)).route("/api/things", post(c))"#;
        assert!(undeclared_routes(source, ROUTES).is_empty());
    }

    /// The same path mounted twice (GET and POST chained on one `.route`, or
    /// two builders) must not be reported twice.
    #[test]
    fn a_repeated_path_is_reported_once() {
        let source = r#".route("/a", get(x)).route("/a", post(y)).route("/b", get(z))"#;
        let missing = undeclared_routes(source, ROUTES);
        assert_eq!(missing, vec!["/a", "/b"]);
    }

    #[test]
    fn the_manifest_body_names_the_capability_and_its_routes() {
        let body = manifest("calendar", ROUTES);
        assert_eq!(body["capability"], "calendar");
        assert_eq!(body["routes"].as_array().unwrap().len(), 2);
        assert_eq!(body["routes"][0]["method"], "GET");
        assert_eq!(body["routes"][1]["path"], "/api/things");
        assert_eq!(body["routes"][1]["summary"], "Every thing.");
    }

    #[test]
    fn source_with_no_routes_at_all_is_not_an_error() {
        assert!(undeclared_routes("fn main() {}", ROUTES).is_empty());
        assert!(served_paths("").is_empty());
    }
}
