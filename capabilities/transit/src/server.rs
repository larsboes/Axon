use std::sync::Arc;

use axum::{
    extract::{Query, State},
    response::Json,
    routing::get,
    Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tower_http::cors::CorsLayer;

use transit::config::Config;
use transit::hafas::HafasClient;
use transit::store::TransitStore;

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
fn fail(status: axum::http::StatusCode, message: impl std::fmt::Display) -> (axum::http::StatusCode, String) {
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

async fn handle_list_trips(
    State(state): State<AppState>,
) -> Result<Json<Value>, (axum::http::StatusCode, String)> {
    let db_url = state.config.database_url.clone();
    match tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let store = TransitStore::open(&db_url).map_err(|e| e.to_string())?;
        // TransitStore doesn't have a list_all_trips method yet.
        // Return count for now.
        let count = store.count().map_err(|e| e.to_string())?;
        Ok(json!({ "count": count, "trips": [] }))
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

#[path = "../../../libs/axon-server/src/lib.rs"]
#[allow(dead_code)]
mod axon_server;

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
        .route("/health", get(handle_health))
        .route("/api/health", get(handle_health))
        .route("/api/suggest", get(handle_suggest))
        .route("/api/search", get(handle_search))
        .route("/api/split", get(handle_split))
        .route("/api/trips", get(handle_list_trips))
        .layer(CorsLayer::permissive())
        .with_state(state);

    // Port contract and loopback bind live in axon_server; the old 0.0.0.0 bind
    // here was never a documented decision and is retired with it.
    let port = axon_server::resolve_port(Some("TRANSIT_PORT"), None, 3000);
    axon_server::serve_local("transit-server", port, app).await;
}
