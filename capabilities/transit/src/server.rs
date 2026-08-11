use std::sync::Arc;

use axum::{
    extract::{Query, State},
    response::Json,
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tower_http::cors::CorsLayer;

use transit::config::Config;
use transit::hafas::HafasClient;
use transit::store::TransitStore;

/// What this capability answers, served as data beside `/health`.
/// Required query parameters are named in the summary: a path alone cannot tell
/// a caller what it must send, and learning that from a 400 is the thing this
/// endpoint exists to avoid.
const ROUTES: &[route_manifest::Route] = &[
    r("GET", "/health", "Liveness."),
    r("GET", "/routes", "This manifest."),
    r(
        "GET",
        "/api/health",
        "Liveness under the API prefix. Same handler as /health.",
    ),
    r("GET", "/api/suggest", "Station suggestions for a query."),
    r(
        "GET",
        "/api/search",
        "Fare search between two stations on a date.",
    ),
    r("GET", "/api/split", "Split-ticket options for a search."),
    r(
        "GET",
        "/api/trips",
        "Saved trip searches with their legs. Optional session_id filters to one \
         `transit plan` session; optional limit (default 100, max 500) bounds the read, \
         and the reply's count/returned/truncated say what was left behind.",
    ),
    r(
        "POST",
        "/api/tickets/extract",
        "Parse a rail ticket confirmation. Body is the raw file bytes; file_name (query) \
         picks the reader by extension (pdf, eml, txt, html). Returns the parse for review \
         and stores nothing: the parser is not fit to run unattended, see the README.",
    ),
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

async fn routes() -> Json<Value> {
    Json(route_manifest::manifest("transit", ROUTES))
}

#[derive(Deserialize)]
struct SuggestQuery {
    q: String,
}

#[derive(Deserialize)]
struct RouteQuery {
    from: String,
    to: String,
    time: String,
}

/// Every capability server answers a failure as `{"error": "..."}` -- the dashboard's
/// client unwraps exactly that field, and without it a reader saw the raw JSON of a
/// failure, or worse, a bare sentence that is not JSON at all. These handlers returned
/// `e.to_string()` and were the one surface in the repo still breaking that contract.
fn fail(
    status: axum::http::StatusCode,
    message: impl std::fmt::Display,
) -> (axum::http::StatusCode, String) {
    (status, json!({ "error": message.to_string() }).to_string())
}

fn hafas_fail(e: transit::hafas::HafasError) -> (axum::http::StatusCode, String) {
    // "no cheaper split exists" is a result. Answering 500 made the absence of a bargain
    // look like a broken server, and the dashboard had to render it as one.
    let status = match e {
        transit::hafas::HafasError::NoSplitFound => axum::http::StatusCode::NOT_FOUND,
        _ => axum::http::StatusCode::INTERNAL_SERVER_ERROR,
    };
    fail(status, e)
}

#[derive(Clone)]
struct AppState {
    hafas_client: Arc<HafasClient>,
    config: Arc<Config>,
}

async fn handle_suggest(
    State(state): State<AppState>,
    Query(params): Query<SuggestQuery>,
) -> Result<Json<Value>, (axum::http::StatusCode, String)> {
    let client = state.hafas_client;
    match tokio::task::spawn_blocking(move || client.suggest_stations(&params.q)).await {
        Ok(Ok(stations)) => Ok(Json(serde_json::to_value(stations).unwrap_or_default())),
        Ok(Err(e)) => Err(hafas_fail(e)),
        Err(e) => Err(fail(axum::http::StatusCode::INTERNAL_SERVER_ERROR, e)),
    }
}

async fn handle_search(
    State(state): State<AppState>,
    Query(params): Query<RouteQuery>,
) -> Result<Json<Value>, (axum::http::StatusCode, String)> {
    let client = state.hafas_client;
    // Enrichment runs inside the same blocking task as the search: it is a blocking HTTP
    // call, and punctuality::enrich leaves the journeys untouched when the statistics
    // service is unreachable, so a search never fails for want of a delay figure.
    match tokio::task::spawn_blocking(move || {
        client
            .search_connections(&params.from, &params.to, &params.time)
            .map(|mut journeys| {
                transit::punctuality::enrich(&mut journeys);
                journeys
            })
    })
    .await
    {
        Ok(Ok(journeys)) => Ok(Json(serde_json::to_value(journeys).unwrap_or_default())),
        Ok(Err(e)) => Err(hafas_fail(e)),
        Err(e) => Err(fail(axum::http::StatusCode::INTERNAL_SERVER_ERROR, e)),
    }
}

async fn handle_split(
    State(state): State<AppState>,
    Query(params): Query<RouteQuery>,
) -> Result<Json<Value>, (axum::http::StatusCode, String)> {
    let client = state.hafas_client;
    match tokio::task::spawn_blocking(move || {
        client
            .search_split_tickets(&params.from, &params.to, &params.time)
            .map(|mut result| {
                transit::punctuality::enrich_split(&mut result);
                result
            })
    })
    .await
    {
        Ok(Ok(result)) => Ok(Json(serde_json::to_value(result).unwrap_or_default())),
        Ok(Err(e)) => Err(hafas_fail(e)),
        Err(e) => Err(fail(axum::http::StatusCode::INTERNAL_SERVER_ERROR, e)),
    }
}

async fn handle_health() -> Json<Value> {
    // Same class of bug as the other handlers: TransitStore::open() drives
    // the sync `postgres` crate's own internal blocking runtime bootstrap --
    // calling it directly inside an async axum handler panics ("cannot
    // start a runtime from within a runtime"). spawn_blocking, same as
    // handle_list_trips.
    let pg_status = tokio::task::spawn_blocking(|| {
        let cfg = Config::load();
        if TransitStore::open(&cfg.database_url).is_ok() {
            "ok".to_string()
        } else {
            "offline".to_string()
        }
    })
    .await
    .unwrap_or_else(|_| "offline".to_string());

    Json(json!({
        "status": "ok",
        "service": "transit",
        "version": env!("CARGO_PKG_VERSION"),
        "postgres": pg_status,
    }))
}

#[derive(Deserialize)]
struct ExtractQuery {
    /// Only the extension is read, to choose the reader. A ticket's own filename
    /// is a personal fact and is echoed back rather than stored anywhere.
    file_name: String,
}

/// Parses a ticket confirmation and returns the parse. Stores nothing.
///
/// `transit import <file>` has done this since the port, printed the JSON and
/// forgotten it, so the parser had no caller but a human at a terminal. This is
/// the same function behind HTTP.
///
/// It deliberately does not write a booking record. `extractor.rs` emits one leg
/// per train number, all sharing origin, destination and times, assigns dates
/// positionally, takes the first price match rather than the total, and falls
/// back to `<year>-01-01` when no date parses. Behind a human reviewing every
/// field that is fine. Behind a scanner it writes wrong itineraries confidently,
/// which is the failure this endpoint must not enable.
async fn handle_extract_ticket(
    Query(params): Query<ExtractQuery>,
    body: axum::body::Bytes,
) -> Result<Json<Value>, (axum::http::StatusCode, String)> {
    if body.is_empty() {
        return Err(fail(
            axum::http::StatusCode::BAD_REQUEST,
            "request body is the ticket file's bytes, and it is empty",
        ));
    }
    match transit::extractor::extract_from_bytes(&body, &params.file_name) {
        Ok(ticket) => Ok(Json(serde_json::to_value(ticket).unwrap_or_default())),
        // A file this parser cannot read is the caller's input, not a server
        // fault: an image, or a format with no reader. 400 tells it to send
        // something else rather than to retry the same bytes.
        Err(e) => Err(fail(axum::http::StatusCode::BAD_REQUEST, e)),
    }
}

/// One stored trip with its legs, as JSON.
///
/// Written out here rather than derived, because `TripRow`/`TripLegRow` are the
/// store's row shapes and deriving `Serialize` on them would make every column
/// rename a breaking API change. The CLI's `session_summary` deliberately serves
/// a flattened version of the same rows: a human choosing between fares wants
/// price and duration, while the caller of this endpoint wants the legs it would
/// otherwise have to query Postgres directly to see.
fn trip_json(t: &transit::store::TripRow, legs: &[transit::store::TripLegRow]) -> Value {
    json!({
        "trip_id": t.id,
        "status": t.status,
        "origin_eva": t.origin_eva,
        "destination_eva": t.destination_eva,
        "trigger_reason": t.trigger_reason,
        "total_duration_minutes": t.total_duration_minutes,
        "total_price": t.total_price,
        "created_at": t.created_at,
        "session_id": t.session_id,
        // When the fare was last seen, so a reader can tell a ten-week-old price
        // from a fresh one. Null means unknown, never recent.
        "priced_at": t.priced_at,
        "legs": legs.iter().map(|l| json!({
            "origin_eva": l.origin_eva,
            "origin_name": l.origin_name,
            "destination_eva": l.destination_eva,
            "destination_name": l.destination_name,
            "departure_time": l.departure_time,
            "arrival_time": l.arrival_time,
            "train_name": l.train_name,
            "train_number": l.train_number,
            "train_category": l.train_category,
            "platform": l.platform,
            "is_regional": l.is_regional,
        })).collect::<Vec<Value>>(),
    })
}

/// A bounded read says what it left behind. `count` is every trip matching the
/// filter, `returned` is how many came back, and `truncated` says whether those
/// two disagree -- borrowed from knowledge-graph's `/api/graph/unit`, for the
/// same reason: a capped answer that looked complete would read as the whole set.
#[derive(Deserialize)]
struct TripsQuery {
    session_id: Option<String>,
    limit: Option<i64>,
}

/// How many trips one unfiltered read returns before it starts saying `truncated`.
const TRIPS_DEFAULT_LIMIT: i64 = 100;
/// The ceiling a caller can raise `limit` to. A trip carries its full leg set, so
/// an unbounded read is a response size nobody asked for.
const TRIPS_MAX_LIMIT: i64 = 500;

async fn handle_list_trips(
    State(state): State<AppState>,
    Query(params): Query<TripsQuery>,
) -> Result<Json<Value>, (axum::http::StatusCode, String)> {
    let db_url = state.config.database_url.clone();
    // Clamped rather than rejected: a caller asking for more than the ceiling wants
    // as much as it can get, and `truncated` already tells it what it did not get.
    let limit = params
        .limit
        .unwrap_or(TRIPS_DEFAULT_LIMIT)
        .clamp(1, TRIPS_MAX_LIMIT);
    match tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let store = TransitStore::open(&db_url).map_err(|e| e.to_string())?;
        let session_id = params.session_id.as_deref();
        let count = store.count_trips(session_id).map_err(|e| e.to_string())?;
        let trips = store
            .list_trips(session_id, Some(limit))
            .map_err(|e| e.to_string())?;
        let rendered: Vec<Value> = trips.iter().map(|(t, legs)| trip_json(t, legs)).collect();
        Ok(json!({
            "count": count,
            "returned": rendered.len(),
            "truncated": count > rendered.len() as i64,
            "session_id": params.session_id,
            "trips": rendered,
        }))
    })
    .await
    {
        Ok(Ok(val)) => Ok(Json(val)),
        Ok(Err(e)) => Err(fail(axum::http::StatusCode::INTERNAL_SERVER_ERROR, e)),
        Err(e) => Err(fail(axum::http::StatusCode::INTERNAL_SERVER_ERROR, e)),
    }
}

// No /discover or /pulse/* proxy routes here — see
// dashboard/README.md. transit-server
// serves transit's own API only; aggregating scouting/pulse behind one origin
// is the dashboard's concern (or a dedicated gateway), not something transit
// hardcodes another capability's port to do. Re-add a proxy only when the
// dashboard names a concrete need it can't solve with a multi-target dev
// proxy / reverse proxy on its own side.

#[tokio::main]
async fn main() {
    let config = Arc::new(Config::load());
    // HafasClient wraps reqwest::blocking::Client, which spins up its own
    // background tokio runtime internally. Constructing it directly inside
    // #[tokio::main]'s async context panics on drop ("cannot drop a runtime
    // in a context where blocking is not allowed") -- spawn_blocking moves
    // the construction off the async runtime thread.
    let hafas_client = Arc::new(
        tokio::task::spawn_blocking(HafasClient::new)
            .await
            .expect("hafas client construction panicked"),
    );

    let state = AppState {
        hafas_client,
        config,
    };

    let app = Router::new()
        .route("/routes", get(routes))
        .route("/health", get(handle_health))
        .route("/api/health", get(handle_health))
        .route("/api/suggest", get(handle_suggest))
        .route("/api/search", get(handle_search))
        .route("/api/split", get(handle_split))
        .route("/api/trips", get(handle_list_trips))
        .route("/api/tickets/extract", post(handle_extract_ticket))
        .layer(CorsLayer::permissive())
        .with_state(state);

    // Port contract and loopback bind live in axon_server; the old 0.0.0.0 bind
    // here was never a documented decision and is retired with it.
    let port = axon_server::resolve_port(Some("TRANSIT_PORT"), None, 3000);
    axon_server::serve_local("transit-server", port, app).await;
}

#[cfg(test)]
mod route_manifest_tests {
    /// A stale manifest is worse than none, because it gets believed. This reads
    /// the router's own source, so adding a `.route()` without a summary fails
    /// here rather than shipping a surface that lies about itself.
    #[test]
    fn the_manifest_covers_every_served_route() {
        let missing = route_manifest::undeclared_routes(include_str!("server.rs"), super::ROUTES);
        assert!(missing.is_empty(), "served but undocumented: {missing:?}");
    }
}
