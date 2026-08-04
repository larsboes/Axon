use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
    routing::get,
    Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tower_http::cors::CorsLayer;

use tasks::config::Config;
use tasks::store::{NewTask, Store, TaskPatch};

/// What this capability answers, served as data beside `/health`.
const ROUTES: &[route_manifest::Route] = &[
    r("GET", "/health", "Liveness."),
    r("GET", "/routes", "This manifest."),
    r("GET", "/api/tasks", "Every task. Optional status filter: open, done, dropped."),
    r("POST", "/api/tasks", "Create a task, or return the one this source already owns."),
    r("GET", "/api/tasks/:id", "One task."),
    r("PATCH", "/api/tasks/:id", "Patch a task's title, status, due date or note."),
    r("GET", "/api/counts", "Open and overdue counts, for a badge."),
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
    Json(route_manifest::manifest("tasks", ROUTES))
}

#[derive(Clone)]
struct AppState {
    database_url: Arc<String>,
}

type ApiResponse = (StatusCode, Json<Value>);

/// Open the store and do one piece of work with it, off the async runtime.
///
/// `postgres` is the sync client, and it drives its own `block_on` internally —
/// calling it straight from a handler panics the worker with "cannot start a
/// runtime from within a runtime", which reaches the caller as an empty reply
/// rather than an error. Every store touch goes through here so that cannot be
/// forgotten one handler at a time.
async fn with_store<T, F>(state: &AppState, work: F) -> Result<T, ApiResponse>
where
    T: Send + 'static,
    F: FnOnce(Store) -> Result<T, Box<dyn std::error::Error>> + Send + 'static,
{
    let database_url = state.database_url.clone();
    // The error becomes a String inside the closure: `Box<dyn Error>` is not
    // `Send`, so it cannot cross back out of the blocking pool.
    let joined = tokio::task::spawn_blocking(move || {
        Store::open(&database_url)
            .and_then(work)
            .map_err(|error| error.to_string())
    })
    .await;
    match joined {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(bad_request(error)),
        Err(_) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        )),
    }
}

fn bad_request(error: impl std::fmt::Display) -> ApiResponse {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": error.to_string() })),
    )
}

fn not_found() -> ApiResponse {
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "error": "no task with that id" })),
    )
}

fn ok<T: serde::Serialize>(status: StatusCode, value: T) -> ApiResponse {
    match serde_json::to_value(value) {
        Ok(value) => (status, Json(value)),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error.to_string() })),
        ),
    }
}

async fn health() -> Json<Value> {
    Json(json!({ "service": "tasks", "status": "ok", "version": "0.0.0" }))
}

#[derive(Debug, Deserialize)]
struct ListQuery {
    status: Option<String>,
}

async fn list_tasks(State(state): State<AppState>, Query(query): Query<ListQuery>) -> ApiResponse {
    match with_store(&state, move |store| store.list(query.status.as_deref())).await {
        Ok(tasks) => ok(StatusCode::OK, json!({ "tasks": tasks })),
        Err(response) => response,
    }
}

/// Create, or hand back the task this source already owns.
///
/// `200` rather than `201` on the second call, and the body says which: a
/// caller pressing promote twice is doing something reasonable, and an error
/// would push it into checking-then-creating, which races.
async fn create_task(State(state): State<AppState>, Json(body): Json<NewTask>) -> ApiResponse {
    match with_store(&state, move |store| store.create(&body)).await {
        Ok((task, true)) => ok(StatusCode::CREATED, json!({ "task": task, "created": true })),
        Ok((task, false)) => ok(StatusCode::OK, json!({ "task": task, "created": false })),
        Err(response) => response,
    }
}

async fn get_task(State(state): State<AppState>, Path(id): Path<String>) -> ApiResponse {
    match with_store(&state, move |store| store.get(&id)).await {
        Ok(Some(task)) => ok(StatusCode::OK, json!({ "task": task })),
        Ok(None) => not_found(),
        Err(response) => response,
    }
}

async fn patch_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<TaskPatch>,
) -> ApiResponse {
    match with_store(&state, move |store| store.patch(&id, &body)).await {
        Ok(Some(task)) => ok(StatusCode::OK, json!({ "task": task })),
        Ok(None) => not_found(),
        Err(response) => response,
    }
}

async fn counts(State(state): State<AppState>) -> ApiResponse {
    match with_store(&state, |store| store.counts()).await {
        Ok((open, overdue)) => ok(StatusCode::OK, json!({ "open": open, "overdue": overdue })),
        Err(response) => response,
    }
}

#[path = "../../../libs/axon-server/src/lib.rs"]
#[allow(dead_code)]
mod axon_server;

#[tokio::main]
async fn main() {
    let config = Config::load();
    let state = AppState {
        database_url: Arc::new(config.database_url),
    };
    let app = Router::new()
        .route("/routes", get(routes))
        .route("/health", get(health))
        .route("/api/tasks", get(list_tasks).post(create_task))
        .route("/api/tasks/:id", get(get_task).patch(patch_task))
        .route("/api/counts", get(counts))
        .layer(CorsLayer::permissive())
        .with_state(state);
    axon_server::serve_local("tasks-server", config.port, app).await;
}

// The self-describing surface, on the same include terms as the other libs.
#[path = "../../../libs/route-manifest/src/lib.rs"]
#[allow(dead_code)]
mod route_manifest;

#[cfg(test)]
mod route_manifest_tests {
    /// A stale manifest is worse than none, because it gets believed. This
    /// reads the router's own source, so adding a `.route()` without a summary
    /// fails here rather than shipping a surface that lies about itself.
    #[test]
    fn the_manifest_covers_every_served_route() {
        let missing =
            super::route_manifest::undeclared_routes(include_str!("server.rs"), super::ROUTES);
        assert!(missing.is_empty(), "served but undocumented: {missing:?}");
    }
}
