//! places HTTP surface (README "HTTP surface", port 8093). Same shape as
//! finance's server: blocking store work in `spawn_blocking`, `/ready` proves
//! the database, and `GET /routes` serves the manifest the coverage test below
//! checks against this file's own source.
//!
//! One deliberate departure from the sibling servers: no permissive CORS.
//! They guard no C2 table; this one serves the companion register (README D4),
//! so browser cross-origin access is refused instead — see `origin_allowed`.

use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    extract::{Path, Query, Request, State},
    http::{header, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Json, Response},
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use serde_json::{json, Value};

use places::config::Config;
use places::geocode::{GeocodeQuery, Geocoder, StructuredQuery};
use places::store::{PlacesStore, Review};
use places::{layers, today};

const ROUTES: &[route_manifest::Route] = &[
    r("GET", "/health", "Liveness."),
    r(
        "GET",
        "/ready",
        "Readiness: liveness plus a reachable database.",
    ),
    r("GET", "/routes", "This manifest."),
    r(
        "GET",
        "/api/places",
        "List/search the place registry. Optional ?q= substring and ?kind= venue|city|station|address|region.",
    ),
    r(
        "POST",
        "/api/geocode",
        "Cached forward geocode. Body: { query } or { structured: { street, postalcode, city, country } }. Place text only; a repeated query never leaves the host.",
    ),
    r(
        "GET",
        "/api/layers/spend",
        "Venue features, city aggregates and a ranked summary over location-linked finance transactions. GeoJSON, cents, EUR implied.",
    ),
    r(
        "GET",
        "/api/layers/travel",
        "Trip destinations with past/upcoming phase, transit legs as LineStrings, station points and spend-presence evidence. GeoJSON.",
    ),
    r(
        "GET",
        "/api/layers/people",
        "Confirmed, currently-valid companion-register rows. GeoJSON.",
    ),
    r(
        "GET",
        "/api/unplaced",
        "Expense transactions with no place link, grouped by exact description, ranked by total. Cents, EUR implied, capped at 200 groups.",
    ),
    r(
        "POST",
        "/api/unplaced/assign",
        "Link every unlinked transaction whose description matches exactly to one place. Body: { description, place_id | geocode_query, precision: venue|city }. A city-kind place is linked at city precision whatever was requested (D1). Writes source=manual links.",
    ),
    r(
        "GET",
        "/api/people/proposals",
        "Proposed register rows awaiting human review.",
    ),
    r(
        "POST",
        "/api/people/proposals/:id/confirm",
        "Confirm one register proposal. The only path that produces state=confirmed.",
    ),
    r(
        "POST",
        "/api/people/proposals/:id/dismiss",
        "Dismiss one register proposal.",
    ),
];

const fn r(
    method: &'static str,
    path: &'static str,
    summary: &'static str,
) -> route_manifest::Route {
    route_manifest::get(method, path, summary)
}

async fn routes() -> Json<Value> {
    Json(route_manifest::manifest("places", ROUTES))
}

#[derive(Clone)]
struct AppState {
    database_path: Arc<PathBuf>,
}

type ApiResponse = (StatusCode, Json<Value>);

fn respond(status: StatusCode, value: Value) -> ApiResponse {
    (status, Json(value))
}

fn failed(error: String) -> ApiResponse {
    respond(
        StatusCode::INTERNAL_SERVER_ERROR,
        json!({ "ok": false, "capability": "places", "error": error }),
    )
}

async fn health() -> Json<Value> {
    Json(json!({ "ok": true, "capability": "places" }))
}

async fn ready(State(state): State<AppState>) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        PlacesStore::open(&database_path)
            .and_then(|store| store.ping())
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(())) => respond(
            StatusCode::OK,
            json!({ "ok": true, "capability": "places" }),
        ),
        Ok(Err(error)) => respond(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({ "ok": false, "capability": "places", "error": error }),
        ),
        Err(_) => respond(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({ "ok": false, "capability": "places", "error": "readiness check failed" }),
        ),
    }
}

#[derive(Debug, Deserialize)]
struct PlacesQuery {
    q: Option<String>,
    kind: Option<String>,
}

async fn list_places(
    State(state): State<AppState>,
    Query(query): Query<PlacesQuery>,
) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        PlacesStore::open(&database_path)
            .and_then(|store| store.search_places(query.q.as_deref(), query.kind.as_deref()))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(found)) => {
            let rendered: Vec<Value> = found
                .iter()
                .map(|place| {
                    json!({
                        "id": place.id,
                        "name": place.name,
                        "kind": place.kind,
                        "city": place.city,
                        "country_code": place.country_code,
                        "latitude": place.latitude,
                        "longitude": place.longitude,
                        "source": place.source,
                        "external_ref": place.external_ref,
                    })
                })
                .collect();
            respond(StatusCode::OK, json!({ "places": rendered }))
        }
        Ok(Err(error)) => failed(error),
        Err(_) => failed("task panicked".into()),
    }
}

#[derive(Debug, Deserialize)]
struct GeocodeRequest {
    query: Option<String>,
    structured: Option<StructuredQuery>,
}

async fn geocode(
    State(state): State<AppState>,
    Json(request): Json<GeocodeRequest>,
) -> ApiResponse {
    let query = match (request.query, request.structured) {
        (Some(free), None) => GeocodeQuery::Free(free),
        (None, Some(structured)) => GeocodeQuery::Structured(structured),
        _ => {
            return respond(
                StatusCode::BAD_REQUEST,
                json!({ "error": "send exactly one of query or structured" }),
            )
        }
    };
    // Emptiness is checked here, for both variants, so a blank query is the
    // client's 400 and never surfaces as the geocoder's own error via 500.
    if query.is_empty() {
        return respond(
            StatusCode::BAD_REQUEST,
            json!({ "error": "geocode query must not be empty" }),
        );
    }
    let database_path = state.database_path.clone();
    let now = today();
    match tokio::task::spawn_blocking(move || {
        let store = PlacesStore::open(&database_path).map_err(|error| error.to_string())?;
        let geocoder = Geocoder::new(&store);
        geocoder
            .geocode(&query, None, &now)
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(outcome)) => {
            let place = outcome.place.map(|place| {
                json!({
                    "place_id": place.id,
                    "name": place.name,
                    // The registry kind, so a client can refuse venue
                    // precision for a city-kind result (README D1) — the
                    // dashboard's "Pin venue" guard reads exactly this.
                    "kind": place.kind,
                    "latitude": place.latitude,
                    "longitude": place.longitude,
                    "city": place.city,
                    "country_code": place.country_code,
                })
            });
            respond(
                StatusCode::OK,
                json!({
                    "status": if outcome.found { "ok" } else { "not_found" },
                    "cached": outcome.cached,
                    "place": place,
                }),
            )
        }
        Ok(Err(error)) => failed(error),
        Err(_) => failed("task panicked".into()),
    }
}

async fn spend_layer(State(state): State<AppState>) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        PlacesStore::open(&database_path)
            .and_then(|store| layers::spend_layer(&store))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(body)) => respond(StatusCode::OK, body),
        Ok(Err(error)) => failed(error),
        Err(_) => failed("task panicked".into()),
    }
}

async fn travel_layer(State(state): State<AppState>) -> ApiResponse {
    let database_path = state.database_path.clone();
    let now = today();
    match tokio::task::spawn_blocking(move || {
        PlacesStore::open(&database_path)
            .and_then(|store| layers::travel_layer(&store, &now))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(body)) => respond(StatusCode::OK, body),
        Ok(Err(error)) => failed(error),
        Err(_) => failed("task panicked".into()),
    }
}

async fn people_layer(State(state): State<AppState>) -> ApiResponse {
    let database_path = state.database_path.clone();
    let now = today();
    match tokio::task::spawn_blocking(move || {
        PlacesStore::open(&database_path)
            .and_then(|store| layers::people_layer(&store, &now))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(body)) => respond(StatusCode::OK, body),
        Ok(Err(error)) => failed(error),
        Err(_) => failed("task panicked".into()),
    }
}

async fn unplaced(State(state): State<AppState>) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        PlacesStore::open(&database_path)
            .and_then(|store| layers::unplaced_groups(&store))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(body)) => respond(StatusCode::OK, body),
        Ok(Err(error)) => failed(error),
        Err(_) => failed("task panicked".into()),
    }
}

async fn assign_unplaced(
    State(state): State<AppState>,
    Json(request): Json<layers::AssignUnplaced>,
) -> ApiResponse {
    // Bad bodies are the client's 400 before any store work, mirroring the
    // geocode handler's emptiness rule.
    if request.description.trim().is_empty() {
        return respond(
            StatusCode::BAD_REQUEST,
            json!({ "error": "description must not be empty" }),
        );
    }
    let by_place_id = match (&request.place_id, &request.geocode_query) {
        (Some(_), None) => true,
        (None, Some(query)) if !query.trim().is_empty() => false,
        (None, Some(_)) => {
            return respond(
                StatusCode::BAD_REQUEST,
                json!({ "error": "geocode_query must not be empty" }),
            )
        }
        _ => {
            return respond(
                StatusCode::BAD_REQUEST,
                json!({ "error": "send exactly one of place_id or geocode_query" }),
            )
        }
    };
    if !matches!(request.precision.as_str(), "venue" | "city") {
        return respond(
            StatusCode::BAD_REQUEST,
            json!({ "error": "precision must be venue or city" }),
        );
    }
    let database_path = state.database_path.clone();
    let now = today();
    match tokio::task::spawn_blocking(move || {
        let store = PlacesStore::open(&database_path).map_err(|error| error.to_string())?;
        let geocoder = Geocoder::new(&store);
        layers::assign_unplaced(&store, &geocoder, "finance", &request, &now)
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(Some(body))) => respond(StatusCode::OK, body),
        Ok(Ok(None)) => respond(
            StatusCode::NOT_FOUND,
            json!({
                "error": if by_place_id {
                    "no place with that id"
                } else {
                    "the geocode query resolved to no place"
                }
            }),
        ),
        Ok(Err(error)) => failed(error),
        Err(_) => failed("task panicked".into()),
    }
}

async fn list_proposals(State(state): State<AppState>) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let store = PlacesStore::open(&database_path).map_err(|error| error.to_string())?;
        let rows = store
            .person_places_in_state("proposed")
            .map_err(|error| error.to_string())?;
        let mut proposals = Vec::with_capacity(rows.len());
        for row in rows {
            let place = store
                .place(&row.place_id)
                .map_err(|error| error.to_string())?;
            proposals.push(json!({
                "id": row.id,
                "person": row.person,
                "place_name": place.as_ref().map(|p| p.name.clone()).unwrap_or_default(),
                "city": place.as_ref().and_then(|p| p.city.clone()),
                "latitude": place.as_ref().and_then(|p| p.latitude),
                "longitude": place.as_ref().and_then(|p| p.longitude),
                "date_start": row.date_start,
                "date_end": row.date_end,
                "confidence_bp": i64::from(row.confidence_bp),
                "source": row.source,
                "state": "proposed",
            }));
        }
        Ok(json!({ "proposals": proposals }))
    })
    .await
    {
        Ok(Ok(body)) => respond(StatusCode::OK, body),
        Ok(Err(error)) => failed(error),
        Err(_) => failed("task panicked".into()),
    }
}

/// The explicit human review path — with `dismiss` below, the ONLY code that
/// can move a register row to `confirmed` (README D4, ISA PLC-7).
async fn confirm_proposal(State(state): State<AppState>, Path(id): Path<String>) -> ApiResponse {
    review(state, id, Review::Confirmed).await
}

async fn dismiss_proposal(State(state): State<AppState>, Path(id): Path<String>) -> ApiResponse {
    review(state, id, Review::Dismissed).await
}

async fn review(state: AppState, id: String, decision: Review) -> ApiResponse {
    let database_path = state.database_path.clone();
    let now = today();
    match tokio::task::spawn_blocking(move || {
        PlacesStore::open(&database_path)
            .and_then(|store| store.review_person_place(&id, decision, &now))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(true)) => respond(
            StatusCode::OK,
            json!({ "ok": true, "state": decision.as_str() }),
        ),
        Ok(Ok(false)) => respond(
            StatusCode::NOT_FOUND,
            json!({ "error": "no register row with that id" }),
        ),
        Ok(Err(error)) => failed(error),
        Err(_) => failed("task panicked".into()),
    }
}

/// Is this browser origin allowed to talk to places at all?
///
/// The register is C2 (README D4), so places refuses cross-origin browser
/// access instead of inheriting the siblings' permissive CORS. Refusing the
/// request — not merely omitting CORS headers — is what also stops a hostile
/// page's "simple" cross-site POST to the confirm route, which a browser sends
/// before it ever reads a response header (ISA PLC-7).
///
/// A request with no `Origin` header is not a browser cross-origin call
/// (curl, the vite proxy's own health probes, same-origin GETs) and passes.
/// With one, the allowed set mirrors how the dashboard itself is reached
/// (`dashboard/vite.config.ts`, `allowedHosts`): the loopback dev origin, or a
/// tailnet name.
///
/// The tailnet check is a bare `.ts.net` suffix by default, because the
/// machine's MagicDNS name is a house fact and this repo is public (the same
/// trade-off vite.config.ts records). Known gap: the suffix also admits
/// Tailscale Funnel sites — public pages on other people's tailnets, which do
/// NOT authenticate at this tailnet's layer. Set
/// `AXON_PLACES_ALLOWED_ORIGIN_HOSTS` (comma-separated exact hosts, from the
/// overlay) to replace the suffix with the deployment's own names and close
/// that gap without naming the machine in public code.
fn origin_allowed(origin: Option<&str>) -> bool {
    let allowed_hosts = std::env::var("AXON_PLACES_ALLOWED_ORIGIN_HOSTS").ok();
    origin_allowed_by(origin, allowed_hosts.as_deref())
}

fn origin_allowed_by(origin: Option<&str>, allowed_hosts: Option<&str>) -> bool {
    let Some(origin) = origin else { return true };
    let Some(rest) = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
    else {
        return false; // "null", file://, extensions — nothing places serves
    };
    let authority = rest.split('/').next().unwrap_or(rest);
    let host = authority
        .rsplit_once(':')
        .map_or(authority, |(host, port)| {
            if port.chars().all(|c| c.is_ascii_digit()) {
                host
            } else {
                authority // no port; the colon was IPv6's
            }
        });
    if matches!(host, "localhost" | "127.0.0.1" | "[::1]") {
        return true;
    }
    match allowed_hosts.map(str::trim).filter(|list| !list.is_empty()) {
        Some(list) => list
            .split(',')
            .map(str::trim)
            .filter(|allowed| !allowed.is_empty())
            .any(|allowed| allowed == host),
        None => host.ends_with(".ts.net"),
    }
}

async fn refuse_foreign_origins(request: Request, next: Next) -> Response {
    let origin = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok());
    if !origin_allowed(origin) {
        return respond(
            StatusCode::FORBIDDEN,
            json!({ "error": "cross-origin access to places is not allowed" }),
        )
        .into_response();
    }
    next.run(request).await
}

pub async fn serve() {
    let config = Config::load();
    let state = AppState {
        database_path: Arc::new(config.database_path),
    };
    let app = Router::new()
        .route("/routes", get(routes))
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/api/places", get(list_places))
        .route("/api/geocode", post(geocode))
        .route("/api/layers/spend", get(spend_layer))
        .route("/api/layers/travel", get(travel_layer))
        .route("/api/layers/people", get(people_layer))
        .route("/api/unplaced", get(unplaced))
        .route("/api/unplaced/assign", post(assign_unplaced))
        .route("/api/people/proposals", get(list_proposals))
        .route("/api/people/proposals/:id/confirm", post(confirm_proposal))
        .route("/api/people/proposals/:id/dismiss", post(dismiss_proposal))
        .layer(middleware::from_fn(refuse_foreign_origins))
        .with_state(state);
    axon_server::serve_local("places-server", config.port, app).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The manifest is data a caller reads to learn the surface; a served route
    /// missing from it is invisible. `undeclared_routes` reads this file's own
    /// source, so adding a `.route()` without a manifest entry fails here
    /// (ISA PLC-1, `libs/route-manifest/README.md`).
    #[test]
    fn the_manifest_covers_every_served_route() {
        assert!(
            route_manifest::undeclared_routes(include_str!("server.rs"), ROUTES).is_empty(),
            "a served route is missing from the manifest"
        );
    }

    #[test]
    fn only_the_two_review_decisions_exist() {
        assert_eq!(Review::Confirmed.as_str(), "confirmed");
        assert_eq!(Review::Dismissed.as_str(), "dismissed");
    }

    /// The C2 guard (README D4): non-browser callers and the origins the
    /// dashboard itself is served from pass; every other web origin is refused,
    /// so a hostile page can neither read the register nor drive the confirm
    /// route cross-site. Exercised through `origin_allowed_by` so the tests
    /// never touch process env (the explicit-parameter pattern geocode's
    /// postgres_tests use for URLs).
    #[test]
    fn foreign_browser_origins_are_refused() {
        // No Origin header: curl, the runner, same-origin GETs.
        assert!(origin_allowed_by(None, None));
        // The dashboard's own origins (dashboard/vite.config.ts allowedHosts).
        assert!(origin_allowed_by(Some("http://localhost:47117"), None));
        assert!(origin_allowed_by(Some("http://127.0.0.1:47117"), None));
        assert!(origin_allowed_by(Some("http://[::1]:47117"), None));
        assert!(origin_allowed_by(Some("http://localhost"), None));
        assert!(origin_allowed_by(Some("https://mac.tailnet.ts.net"), None));
        // Everyone else.
        assert!(!origin_allowed_by(Some("https://evil.example"), None));
        assert!(!origin_allowed_by(Some("https://evilts.net"), None));
        assert!(!origin_allowed_by(
            Some("https://mac.ts.net.evil.example"),
            None
        ));
        assert!(!origin_allowed_by(
            Some("http://localhost.evil.example"),
            None
        ));
        assert!(!origin_allowed_by(Some("null"), None));
        assert!(!origin_allowed_by(Some("file:///tmp/page.html"), None));
    }

    /// With AXON_PLACES_ALLOWED_ORIGIN_HOSTS set, only the named tailnet hosts
    /// pass: a Funnel page on someone else's tailnet no longer does, which is
    /// the gap the bare `.ts.net` suffix leaves open.
    #[test]
    fn an_explicit_host_list_replaces_the_tailnet_suffix() {
        let list = Some("mac.tailnet.ts.net, phone.tailnet.ts.net");
        assert!(origin_allowed_by(Some("https://mac.tailnet.ts.net"), list));
        assert!(origin_allowed_by(
            Some("https://phone.tailnet.ts.net"),
            list
        ));
        assert!(!origin_allowed_by(
            Some("https://evil.other-tailnet.ts.net"),
            list
        ));
        // Loopback stays allowed whatever the list says.
        assert!(origin_allowed_by(Some("http://localhost:47117"), list));
        // A blank value means unset, not "allow nothing".
        assert!(origin_allowed_by(
            Some("https://mac.tailnet.ts.net"),
            Some("  ")
        ));
    }
}
