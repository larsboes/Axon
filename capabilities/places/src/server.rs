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
    r(
        "GET",
        "/api/places/:id/climate",
        "Twelve months of climate normals for one registered place, folded from ten complete calendar years. Optional ?from=YYYY-MM-DD&to=YYYY-MM-DD marks the months a plan window covers. Empty months with fetched_at null means the place has none yet — run `places-server climate fetch`.",
    ),
    r(
        "GET",
        "/api/climate",
        "Climate normals for up to 8 places at once. Exactly one of ?place_ids=<id>,<id> or ?at=<lat>,<lon>;<lat>,<lon> (semicolon between pairs, comma inside a pair — NOT a repeated key). Optional ?from=&to=. One result per requested key, in request order.",
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

// ─── Climate normals (README D5, ISA F4) ─────────────────────────────────────

/// Optional plan window. Only the month numbers it covers are used, so a
/// request cannot narrow the normals themselves — a normal is the whole month
/// or it is nothing.
#[derive(Debug, Deserialize)]
struct ClimateWindow {
    from: Option<String>,
    to: Option<String>,
}

/// The month numbers a `from`..`to` window touches, capped at twelve. A window
/// longer than a year covers every month, which is the honest answer rather than
/// an error.
fn window_months(from: Option<&str>, to: Option<&str>) -> Vec<u32> {
    let index = |value: &str| -> Option<i64> {
        let year: i64 = value.get(..4)?.parse().ok()?;
        let month: i64 = value.get(5..7)?.parse().ok()?;
        (1..=12).contains(&month).then_some(year * 12 + month - 1)
    };
    let (Some(first), Some(last)) = (from.and_then(index), to.and_then(index)) else {
        return Vec::new();
    };
    if last < first {
        return Vec::new();
    }
    (first..=last.min(first + 11))
        .map(|slot| (slot.rem_euclid(12) + 1) as u32)
        .collect()
}

/// One place's stored normals as the wire carries them. `months: []` with
/// `fetched_at: null` is the never-fetched state, which the UI turns into "run
/// the fetch verb" rather than into an empty grid.
fn climate_body(store: &PlacesStore, place_id: &str, in_window: &[u32]) -> Result<Value, String> {
    let months = store.climate_get(place_id).map_err(|e| e.to_string())?;
    let meta = store.climate_meta(place_id).map_err(|e| e.to_string())?;
    let best = places::climate::best_months(&months);
    let rendered: Vec<Value> = months
        .iter()
        .map(|month| {
            json!({
                "month": month.month,
                "t_max_mean": month.t_max_mean,
                "t_min_mean": month.t_min_mean,
                "rain_days_mean": month.rain_days_mean,
                "precipitation_mm_mean": month.precipitation_mm_mean,
                "daylight_hours_mean": month.daylight_hours_mean,
                "sunshine_hours_mean": month.sunshine_hours_mean,
                "days_observed": month.days_observed,
                "best_month": best.contains(&month.month),
                "in_window": in_window.contains(&month.month),
            })
        })
        .collect();
    Ok(json!({
        "source": meta.as_ref().map(|meta| meta.source.clone()),
        "period": meta.as_ref().map(|meta| json!({
            "start": meta.period_start,
            "end": meta.period_end,
            "years": meta.years_covered,
        })),
        "fetched_at": meta.as_ref().map(|meta| meta.fetched_at.clone()),
        "months": rendered,
    }))
}

fn place_summary(place: &places::store::Place, distance_km: Option<f64>) -> Value {
    json!({
        "id": place.id,
        "name": place.name,
        "kind": place.kind,
        "latitude": place.latitude,
        "longitude": place.longitude,
        "distance_km": distance_km,
    })
}

async fn place_climate(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(window): Query<ClimateWindow>,
) -> ApiResponse {
    let database_path = state.database_path.clone();
    let in_window = window_months(window.from.as_deref(), window.to.as_deref());
    match tokio::task::spawn_blocking(move || -> Result<Option<Value>, String> {
        let store = PlacesStore::open(&database_path).map_err(|e| e.to_string())?;
        let Some(place) = store.place(&id).map_err(|e| e.to_string())? else {
            return Ok(None);
        };
        let mut body = climate_body(&store, &place.id, &in_window)?;
        let object = body.as_object_mut().expect("climate_body builds an object");
        object.insert("place".into(), place_summary(&place, None));
        object.insert(
            "best_months_rule".into(),
            json!(places::climate::BEST_MONTHS_RULE),
        );
        object.insert("attribution".into(), json!(places::climate::ATTRIBUTION));
        Ok(Some(body))
    })
    .await
    {
        Ok(Ok(Some(body))) => respond(StatusCode::OK, body),
        Ok(Ok(None)) => respond(StatusCode::NOT_FOUND, json!({ "error": "no such place" })),
        Ok(Err(error)) => failed(error),
        Err(_) => failed("task panicked".into()),
    }
}

#[derive(Debug, Deserialize)]
struct ClimateBatch {
    place_ids: Option<String>,
    /// Semicolon between pairs, comma inside a pair. NOT a repeated `at=` key:
    /// axum's `Query` deserializes through serde_urlencoded, which cannot fill a
    /// sequence from repeated keys, so `?at=..&at=..` would answer 400 for every
    /// multi-destination request. Both delimiters are safe because every value
    /// here is a number, and comma-splitting is this file's own precedent.
    at: Option<String>,
    from: Option<String>,
    to: Option<String>,
}

/// At most eight keys per request. A bound rather than a limit anybody will hit:
/// a plan has a handful of destinations, and an unbounded batch would turn one
/// request into an unbounded scan of the registry per key.
const MAX_CLIMATE_KEYS: usize = 8;

async fn climate(State(state): State<AppState>, Query(query): Query<ClimateBatch>) -> ApiResponse {
    let selectors = (query.place_ids.as_deref(), query.at.as_deref());
    let keys: Vec<String> = match selectors {
        (Some(ids), None) => ids.split(',').map(|id| id.trim().to_string()).collect(),
        (None, Some(at)) => at.split(';').map(|pair| pair.trim().to_string()).collect(),
        _ => {
            return respond(
                StatusCode::BAD_REQUEST,
                json!({ "error": "send exactly one of place_ids=<id>,<id> or at=<lat>,<lon>;<lat>,<lon>" }),
            )
        }
    };
    let keys: Vec<String> = keys.into_iter().filter(|key| !key.is_empty()).collect();
    if keys.is_empty() || keys.len() > MAX_CLIMATE_KEYS {
        return respond(
            StatusCode::BAD_REQUEST,
            json!({ "error": format!("send 1 to {MAX_CLIMATE_KEYS} keys") }),
        );
    }
    let by_id = query.place_ids.is_some();
    let database_path = state.database_path.clone();
    let in_window = window_months(query.from.as_deref(), query.to.as_deref());

    match tokio::task::spawn_blocking(move || -> Result<Vec<Value>, String> {
        let store = PlacesStore::open(&database_path).map_err(|e| e.to_string())?;
        // Loaded once for the whole batch, not once per key.
        let registry = if by_id {
            Vec::new()
        } else {
            store.places_with_climate().map_err(|e| e.to_string())?
        };
        let mut results = Vec::with_capacity(keys.len());
        for key in &keys {
            // Err carries the reason this key has no place, and every reason is
            // about what the caller actually sent: an unparsable pair is told so
            // rather than being told the registry is empty near a coordinate it
            // never sent.
            let resolution: Result<(&str, places::store::Place, Option<f64>), String> = if by_id {
                match store.place(key).map_err(|e| e.to_string())? {
                    Some(place) => Ok(("id", place, None)),
                    None => Err("no place with that id".to_string()),
                }
            } else {
                match parse_pair(key) {
                    None => Err("not a lat,lon pair".to_string()),
                    Some((latitude, longitude)) => {
                        match places::climate::resolve_at(&registry, latitude, longitude) {
                            places::climate::Resolution::Registry { place, distance_km } => {
                                Ok(("registry", place, Some(distance_km)))
                            }
                            places::climate::Resolution::Nearest { place, distance_km } => {
                                Ok(("nearest", place, Some(distance_km)))
                            }
                            // The refusal is built where the radius is, so the
                            // sentence and the constant cannot drift.
                            places::climate::Resolution::Unmatched { reason } => Err(reason),
                        }
                    }
                }
            };
            let entry = match resolution {
                Ok((resolved_by, place, distance_km)) => {
                    let mut body = climate_body(&store, &place.id, &in_window)?;
                    let object = body.as_object_mut().expect("an object");
                    object.insert("key".into(), json!(key));
                    object.insert("resolved_by".into(), json!(resolved_by));
                    object.insert("matched_place".into(), place_summary(&place, distance_km));
                    object.insert("reason".into(), Value::Null);
                    body
                }
                Err(reason) => json!({
                    "key": key,
                    "resolved_by": Value::Null,
                    "matched_place": Value::Null,
                    "reason": reason,
                    "source": Value::Null,
                    "period": Value::Null,
                    "fetched_at": Value::Null,
                    "months": [],
                }),
            };
            results.push(entry);
        }
        Ok(results)
    })
    .await
    {
        Ok(Ok(results)) => respond(
            StatusCode::OK,
            json!({
                "best_months_rule": places::climate::BEST_MONTHS_RULE,
                "attribution": places::climate::ATTRIBUTION,
                "results": results,
            }),
        ),
        Ok(Err(error)) => failed(error),
        Err(_) => failed("task panicked".into()),
    }
}

fn parse_pair(pair: &str) -> Option<(f64, f64)> {
    let (latitude, longitude) = pair.split_once(',')?;
    Some((
        latitude.trim().parse().ok()?,
        longitude.trim().parse().ok()?,
    ))
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
        .route("/api/places/:id/climate", get(place_climate))
        .route("/api/climate", get(climate))
        .layer(middleware::from_fn(refuse_foreign_origins))
        .with_state(state);
    axon_server::serve_local("places-server", config.port, app).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch database file this process owns. `store::db_tests` has the same
    /// helper, but it is `#[cfg(test)]` inside the library and this file is the
    /// binary, so it cannot be reached from here.
    fn scratch_database(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("places-server-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a writable temp directory");
        let path = dir.join(format!("{name}.db"));
        for tail in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{tail}", path.display()));
        }
        path
    }

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

    /// The defect this test exists for: axum 0.7's `Query` deserializes through
    /// serde_urlencoded, which cannot fill a `Vec` from a repeated key, so a
    /// `?at=..&at=..` extractor would have answered 400 for every
    /// multi-destination request. A pure resolver test cannot fail on a
    /// query-string shape, so the assertion is made here, through the extractor.
    #[tokio::test]
    async fn two_coordinates_in_one_at_parameter_answer_two_results_in_request_order() {
        let path = scratch_database("climate-handler");
        let store = PlacesStore::open(&path).unwrap();
        for (id, name, latitude, longitude) in [
            ("place_first", "First", 52.52, 13.40),
            ("place_second", "Second", 41.90, 12.50),
        ] {
            store
                .upsert_place(
                    &places::store::Place {
                        id: id.into(),
                        name: name.into(),
                        kind: "city".into(),
                        address: None,
                        city: None,
                        country_code: None,
                        latitude: Some(latitude),
                        longitude: Some(longitude),
                        source: "test".into(),
                        external_ref: Some(format!("test:{id}")),
                    },
                    "2026-09-05",
                )
                .unwrap();
        }

        let state = AppState {
            database_path: Arc::new(path),
        };
        let query = ClimateBatch {
            place_ids: None,
            at: Some("52.52,13.40;41.90,12.50".into()),
            from: None,
            to: None,
        };
        let (status, Json(body)) = climate(State(state), Query(query)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let results = body["results"].as_array().expect("results is an array");
        assert_eq!(results.len(), 2, "one result per requested key");
        assert_eq!(results[0]["key"], "52.52,13.40", "request order is kept");
        assert_eq!(results[1]["key"], "41.90,12.50");
        assert_eq!(results[0]["resolved_by"], "registry");
        assert_eq!(results[0]["matched_place"]["id"], "place_first");
        assert_eq!(results[1]["matched_place"]["id"], "place_second");
        // Registered but never fetched: an honest empty, with the stamp null so
        // the UI says "run the fetch verb" instead of drawing an empty grid.
        assert_eq!(results[0]["fetched_at"], Value::Null);
        assert_eq!(results[0]["months"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn a_climate_batch_needs_exactly_one_selector() {
        let state = AppState {
            database_path: Arc::new(scratch_database("climate-selector")),
        };
        for (place_ids, at) in [
            (None, None),
            (Some("place_a".to_string()), Some("52.5,13.4".to_string())),
        ] {
            let (status, Json(body)) = climate(
                State(state.clone()),
                Query(ClimateBatch {
                    place_ids,
                    at,
                    from: None,
                    to: None,
                }),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert!(
                body["error"].as_str().unwrap_or_default().contains("at="),
                "the 400 names both forms: {body}"
            );
        }
    }

    /// A key that never parsed is not a key with no match nearby. Telling a
    /// caller "no registered place with normals within 60 km" about a typo sends
    /// it looking for the wrong bug.
    #[tokio::test]
    async fn a_malformed_pair_is_told_it_is_malformed() {
        let state = AppState {
            database_path: Arc::new(scratch_database("climate-malformed")),
        };
        let (status, Json(body)) = climate(
            State(state),
            Query(ClimateBatch {
                place_ids: None,
                at: Some("abc;52.52,13.40".into()),
                from: None,
                to: None,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["results"][0]["key"], "abc");
        assert_eq!(body["results"][0]["reason"], "not a lat,lon pair");
        // The second key parses, and answers on its own terms.
        assert_eq!(body["results"][1]["key"], "52.52,13.40");
        assert!(
            body["results"][1]["reason"]
                .as_str()
                .unwrap_or_default()
                .contains("60 km"),
            "an empty registry still answers with the distance sentence: {body}"
        );
    }

    #[test]
    fn a_plan_window_marks_the_months_it_covers() {
        assert_eq!(
            window_months(Some("2026-06-10"), Some("2026-08-02")),
            vec![6, 7, 8]
        );
        // Across a year boundary.
        assert_eq!(
            window_months(Some("2026-12-20"), Some("2027-01-04")),
            vec![12, 1]
        );
        // Longer than a year: every month, not an error.
        assert_eq!(
            window_months(Some("2026-03-01"), Some("2030-03-01")).len(),
            12
        );
        // No window is no marks, never all of them.
        assert!(window_months(None, Some("2026-08-02")).is_empty());
        assert!(window_months(Some("2026-08-02"), Some("2026-06-10")).is_empty());
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
    /// db_tests use for URLs).
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
