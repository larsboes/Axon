//! punctuality-server — the contract other capabilities read this through.
//!
//! `capabilities/transit` needs these numbers and must not reach into this crate to get
//! them (README.md#schemas-and-dependency-direction): a capability depends on another's HTTP surface and schema,
//! never its code. This is that surface.
//!
//! Axum + tokio here, sync `postgres` underneath, same split transit-server already
//! makes: serving needs an async runtime, the queries do not, so blocking work goes
//! through `spawn_blocking` rather than dragging an async driver into the library.

use std::sync::{Arc, Mutex};

use axum::{
    extract::{Query, State},
    response::Json,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tower_http::cors::CorsLayer;

use punctuality::config::Config;
use punctuality::store::{StatRow, Store};

/// One stop to look up. `hour` and `weekend` are the caller's, because only the caller
/// knows which of a leg's two timestamps it is asking about — departure at the origin
/// or arrival at the destination.
/// What this capability answers, served as data beside `/health`.
/// Required query parameters are named in the summary: a path alone cannot tell
/// a caller what it must send, and learning that from a 400 is the thing this
/// endpoint exists to avoid.
const ROUTES: &[route_manifest::Route] = &[
    r("GET", "/health", "Liveness."),
    r("GET", "/routes", "This manifest."),
    r("POST", "/lookup", "Punctuality for a connection."),
    r("GET", "/stations", "Station search. Requires a query."),
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
    Json(route_manifest::manifest("punctuality", ROUTES))
}

#[derive(Debug, Deserialize)]
struct StopQuery {
    eva: String,
    train_type: String,
    hour: u8,
    #[serde(default)]
    weekend: bool,
}

#[derive(Debug, Deserialize)]
struct LookupBody {
    stops: Vec<StopQuery>,
}

/// What one stop's history says. `null` in the response array where nothing is known —
/// distinct from a zero-risk answer, which is why this is an Option at the array level
/// and not a row of zeroes.
#[derive(Debug, Serialize)]
struct StopStats {
    eva: String,
    station_name: Option<String>,
    train_type: String,
    hour: i16,
    weekend: bool,
    n: i64,
    mean_delay: f32,
    p50: i16,
    p90: i16,
    share_late_6: f32,
    cancel_rate: f32,
}

impl From<StatRow> for StopStats {
    fn from(r: StatRow) -> Self {
        Self {
            eva: r.eva,
            station_name: r.station_name,
            train_type: r.train_type,
            hour: r.hour,
            weekend: r.weekend,
            n: r.n,
            mean_delay: r.mean_delay,
            p50: r.p50,
            p90: r.p90,
            share_late_6: r.share_late_6,
            cancel_rate: r.cancel_rate,
        }
    }
}

/// Cells thinner than this are reported as unknown rather than as a statistic. A 100%
/// late rate over four observations is noise wearing a percentage sign, and a consumer
/// cannot tell the difference once it is a float in a JSON field.
const MIN_SAMPLE: i64 = 30;

#[derive(Clone)]
struct AppState {
    store: Arc<Mutex<Store>>,
}

type ApiResult = Result<Json<Value>, (axum::http::StatusCode, String)>;

fn fail(e: impl std::fmt::Display) -> (axum::http::StatusCode, String) {
    // Every capability server answers a failure as {"error": "..."} -- the dashboard's
    // client unwraps exactly that field. transit's /api/split still returns a bare
    // sentence and is the odd one out; this one is not going to be the second.
    (
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        json!({ "error": e.to_string() }).to_string(),
    )
}

async fn handle_health(State(state): State<AppState>) -> ApiResult {
    let store = state.store;
    let covered = tokio::task::spawn_blocking(move || {
        store
            .lock()
            .map_err(|e| e.to_string())
            .and_then(|mut s| s.coverage().map_err(|e| e.to_string()))
    })
    .await
    .map_err(fail)?
    .map_err(fail)?;

    Ok(Json(json!({
        "ok": covered.is_some(),
        "capability": "punctuality",
        // A server that answers but has never ingested is up and useless. Saying which
        // window it holds is the difference between "no data for that train" meaning
        // "punctual" and meaning "nobody has run ingest".
        "coverage": covered.map(|(from, to, cells)| json!({
            "from_month": from, "to_month": to, "cells": cells
        })),
    })))
}

async fn handle_lookup(State(state): State<AppState>, Json(body): Json<LookupBody>) -> ApiResult {
    if body.stops.len() > 200 {
        return Err((
            axum::http::StatusCode::BAD_REQUEST,
            json!({ "error": "at most 200 stops per lookup" }).to_string(),
        ));
    }
    let store = state.store;
    let stats = tokio::task::spawn_blocking(move || -> Result<Vec<Option<StopStats>>, String> {
        let mut store = store.lock().map_err(|e| e.to_string())?;
        body.stops
            .iter()
            .map(|s| {
                store
                    .stop_stats(&s.eva, &s.train_type, s.hour as i16, s.weekend, MIN_SAMPLE)
                    .map(|opt| opt.map(StopStats::from))
                    .map_err(|e| e.to_string())
            })
            .collect()
    })
    .await
    .map_err(fail)?
    .map_err(fail)?;

    Ok(Json(json!({ "stats": stats, "min_sample": MIN_SAMPLE })))
}

#[derive(Deserialize)]
struct StationQuery {
    eva: Option<String>,
    q: Option<String>,
    train_type: Option<String>,
}

async fn handle_station(State(state): State<AppState>, Query(p): Query<StationQuery>) -> ApiResult {
    let store = state.store;
    let value = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let mut store = store.lock().map_err(|e| e.to_string())?;
        if let Some(q) = p.q {
            let hits = store.find_stations(&q).map_err(|e| e.to_string())?;
            return Ok(json!(hits
                .into_iter()
                .map(|(eva, name)| json!({ "eva": eva, "station_name": name }))
                .collect::<Vec<_>>()));
        }
        let eva = p.eva.ok_or("pass eva=<code> or q=<name fragment>")?;
        let rows = store
            .station_stats(&eva, p.train_type.as_deref(), MIN_SAMPLE)
            .map_err(|e| e.to_string())?;
        Ok(json!(rows
            .into_iter()
            .map(StopStats::from)
            .collect::<Vec<_>>()))
    })
    .await
    .map_err(fail)?
    .map_err(fail)?;

    Ok(Json(value))
}

#[tokio::main]
async fn main() {
    let cfg = Config::load();
    // The sync `postgres` client drives its connection with its own internal runtime,
    // so constructing it on an async runtime thread panics with "cannot start a runtime
    // from within a runtime". spawn_blocking moves it off. transit-server does the same
    // dance for reqwest::blocking -- same cause, different library.
    // The error is stringified inside the closure because Box<dyn Error> is not Send
    // and so cannot cross the spawn_blocking boundary.
    let url = cfg.database_url.clone();
    let store =
        match tokio::task::spawn_blocking(move || Store::open(&url).map_err(|e| e.to_string()))
            .await
        {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => {
                eprintln!("punctuality-server: cannot reach the database: {e}");
                std::process::exit(1);
            }
            Err(e) => {
                eprintln!("punctuality-server: store construction panicked: {e}");
                std::process::exit(1);
            }
        };
    let state = AppState {
        store: Arc::new(Mutex::new(store)),
    };

    // AXON_PORT is what service-runner.sh exports from the manifest, so the port lives
    // in one file rather than two -- resolution itself lives in axon_server.
    let port = axon_server::resolve_port(None, None, 8085);

    let app = Router::new()
        .route("/routes", get(routes))
        .route("/health", get(handle_health))
        .route("/lookup", post(handle_lookup))
        .route("/stations", get(handle_station))
        .layer(CorsLayer::permissive())
        .with_state(state);

    // 127.0.0.1: this reads a database of public data, but it is still a local service
    // with no auth, and binding it to the LAN would be a decision nobody made. That
    // rationale is now axon_server::serve_local's policy (this crate's bind-failure
    // behavior became the shared default there, too).
    axon_server::serve_local("punctuality-server", port, app).await;
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
