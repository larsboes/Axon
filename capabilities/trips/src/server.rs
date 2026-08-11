use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
    routing::{delete, get, post},
    Router,
};
use serde::Serialize;
use serde_json::{json, Value};
use tower_http::cors::CorsLayer;

use trips::config::{Config, ObsidianConfig};
use trips::obsidian::{read_trip_note, scan_trip_notes, ObsidianTripCandidate};
use trips::store::{
    CreatePlan, CreatePlanItem, PlaceRef, PlanSource, TripPlan, TripsStore, UpdatePlan,
};

/// What this capability answers, served as data beside `/health`.
/// Required query parameters are named in the summary: a path alone cannot tell
/// a caller what it must send, and learning that from a 400 is the thing this
/// endpoint exists to avoid.
const ROUTES: &[route_manifest::Route] = &[
    r("GET", "/health", "Liveness."),
    r(
        "GET",
        "/ready",
        "Readiness: liveness plus a reachable database.",
    ),
    r("GET", "/routes", "This manifest."),
    r("GET", "/api/plans", "Every trip plan."),
    r("POST", "/api/plans", "Create a trip plan."),
    r(
        "GET",
        "/api/plans/:id",
        "One trip plan with its stages and items.",
    ),
    r(
        "PATCH",
        "/api/plans/:id",
        "Patch a trip plan. Optional expected_updated_at (body) makes the write conditional: \
         a mismatch is 409 with code stale_plan instead of overwriting another writer. \
         budget_cents + currency record what the trip is meant to cost.",
    ),
    r(
        "DELETE",
        "/api/plans/:id",
        "Delete a trip plan. Optional expected_updated_at (query) makes it conditional, \
         409 with code stale_plan on a mismatch.",
    ),
    r("POST", "/api/plans/:id/items", "Add an item to a plan."),
    r(
        "PATCH",
        "/api/plans/:plan_id/items/:item_id",
        "Move an item to a day. Body is {day: \"YYYY-MM-DD\"} or {day: null} to unset.",
    ),
    r(
        "POST",
        "/api/plans/:id/outcome",
        "Record how a stage actually went. Requires stage_id; every other field is kept as \
         observed. Refused when the stage has no selected_option_id, because there is then \
         nothing to compare an actual against.",
    ),
    r(
        "GET",
        "/api/places",
        "Every place across all plans, with visit counts and merge_candidates: distinct \
         place ids whose names normalise to the same string. Computed on read.",
    ),
    r(
        "DELETE",
        "/api/plans/:plan_id/items/:item_id",
        "Remove an item from a plan.",
    ),
    r(
        "GET",
        "/api/import/obsidian/scan",
        "Vault trip notes that could be imported. Read-only.",
    ),
    r(
        "POST",
        "/api/import/obsidian",
        "Import one vault trip note.",
    ),
    r(
        "POST",
        "/api/import/obsidian/all",
        "Import every scanned vault trip note.",
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
    Json(route_manifest::manifest("trips", ROUTES))
}

#[derive(Clone)]
struct AppState {
    database_url: Arc<String>,
    obsidian: Option<ObsidianConfig>,
}

type ApiResponse = (StatusCode, Json<Value>);

fn response<T: serde::Serialize>(status: StatusCode, value: T) -> ApiResponse {
    (
        status,
        Json(
            serde_json::to_value(value)
                .unwrap_or_else(|_| json!({ "error": "serialization failed" })),
        ),
    )
}

async fn health() -> Json<Value> {
    Json(json!({
        "ok": true,
        "capability": "trips"
    }))
}

/// Readiness: whether this capability can actually serve, which liveness does not answer.
///
/// `health` is a literal and cannot observe the database, so during a Postgres outage this
/// capability reported itself up while every query behind it failed (#126). Availability is
/// judged here instead.
async fn ready(State(state): State<AppState>) -> ApiResponse {
    let database_url = state.database_url.clone();
    match tokio::task::spawn_blocking(move || {
        TripsStore::open(&database_url)
            .and_then(|store| store.ping())
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(())) => response(StatusCode::OK, json!({ "ok": true, "capability": "trips" })),
        // 503, not 500: the request was fine, the dependency is not, and a caller that retries
        // should be told to come back rather than to fix its input.
        Ok(Err(error)) => response(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({ "ok": false, "capability": "trips", "error": error }),
        ),
        Err(_) => response(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({ "ok": false, "capability": "trips", "error": "readiness check failed" }),
        ),
    }
}

async fn list_plans(State(state): State<AppState>) -> ApiResponse {
    let database_url = state.database_url.clone();
    match tokio::task::spawn_blocking(move || {
        TripsStore::open(&database_url)
            .and_then(|store| store.list_plans())
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(plans)) => response(StatusCode::OK, plans),
        Ok(Err(error)) => response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn create_plan(State(state): State<AppState>, Json(input): Json<CreatePlan>) -> ApiResponse {
    let database_url = state.database_url.clone();
    match tokio::task::spawn_blocking(move || {
        TripsStore::open(&database_url)
            .and_then(|store| store.create_plan(&input))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(plan)) => response(StatusCode::CREATED, plan),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn update_plan(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(input): Json<UpdatePlan>,
) -> ApiResponse {
    let database_url = state.database_url.clone();
    match tokio::task::spawn_blocking(move || {
        TripsStore::open(&database_url)
            .and_then(|store| store.update_plan(&id, &input))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(Some(plan))) => response(StatusCode::OK, plan),
        Ok(Ok(None)) => response(
            StatusCode::NOT_FOUND,
            json!({ "error": "trip plan not found" }),
        ),
        Ok(Err(error)) => write_conflict_or_bad_request(error),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

/// A stale revision is 409, not 400: the request was well-formed and the caller
/// should re-read and retry, which is a different instruction from "fix your
/// input". `code` is there so a caller can branch without parsing prose.
fn write_conflict_or_bad_request(error: String) -> ApiResponse {
    if error.starts_with("stale_plan:") {
        return response(
            StatusCode::CONFLICT,
            json!({ "error": error, "code": "stale_plan" }),
        );
    }
    response(StatusCode::BAD_REQUEST, json!({ "error": error }))
}

#[derive(serde::Deserialize)]
struct SetDay {
    /// Explicit null clears the day; omitting the field is the same request with
    /// no instruction in it, so it is rejected rather than guessed at.
    day: Option<String>,
}

async fn set_item_day(
    State(state): State<AppState>,
    Path((plan_id, item_id)): Path<(String, String)>,
    Json(input): Json<SetDay>,
) -> ApiResponse {
    let database_url = state.database_url.clone();
    match tokio::task::spawn_blocking(move || {
        TripsStore::open(&database_url)
            .and_then(|store| store.set_item_day(&plan_id, &item_id, input.day.as_deref()))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(Some(item))) => response(StatusCode::OK, item),
        Ok(Ok(None)) => response(StatusCode::NOT_FOUND, json!({ "error": "item not found" })),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

#[derive(serde::Deserialize)]
struct OutcomeBody {
    stage_id: String,
    /// Whatever was actually observed. Left open on purpose: nobody has filled
    /// one of these in yet, so fixing a shape now would be designing an analysis
    /// before there is anything to analyse. The fields that matter get promoted
    /// to a declared variant once two real trips show which ones they are.
    #[serde(flatten)]
    outcome: serde_json::Map<String, Value>,
}

async fn record_outcome(
    State(state): State<AppState>,
    Path(plan_id): Path<String>,
    Json(body): Json<OutcomeBody>,
) -> ApiResponse {
    let database_url = state.database_url.clone();
    match tokio::task::spawn_blocking(move || {
        TripsStore::open(&database_url)
            .and_then(|store| {
                store.record_outcome(&plan_id, &body.stage_id, &Value::Object(body.outcome))
            })
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(item)) => response(StatusCode::CREATED, item),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn list_places(State(state): State<AppState>) -> ApiResponse {
    let database_url = state.database_url.clone();
    match tokio::task::spawn_blocking(move || {
        TripsStore::open(&database_url)
            .and_then(|store| store.list_places())
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(places)) => response(StatusCode::OK, places),
        Ok(Err(error)) => response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn get_plan(State(state): State<AppState>, Path(id): Path<String>) -> ApiResponse {
    let database_url = state.database_url.clone();
    match tokio::task::spawn_blocking(move || {
        TripsStore::open(&database_url)
            .and_then(|store| store.get_plan(&id))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(Some(plan))) => response(StatusCode::OK, plan),
        Ok(Ok(None)) => response(
            StatusCode::NOT_FOUND,
            json!({ "error": "trip plan not found" }),
        ),
        Ok(Err(error)) => response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

#[derive(serde::Deserialize)]
struct DeleteQuery {
    /// Same guard as PATCH, and in the same commit deliberately: a stale delete
    /// is worse than a stale patch, because there is nothing left to re-apply.
    expected_updated_at: Option<String>,
}

async fn delete_plan(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<DeleteQuery>,
) -> ApiResponse {
    let database_url = state.database_url.clone();
    match tokio::task::spawn_blocking(move || {
        let store = TripsStore::open(&database_url).map_err(|error| error.to_string())?;
        if let Some(expected) = params.expected_updated_at.as_deref() {
            match store.get_plan(&id).map_err(|error| error.to_string())? {
                None => return Ok(false),
                Some(details) if details.plan.updated_at != expected => {
                    return Err(format!(
                        "stale_plan: expected_updated_at {expected} but the plan is at {}; \
                         re-read it before deleting",
                        details.plan.updated_at
                    ));
                }
                Some(_) => {}
            }
        }
        store.delete_plan(&id).map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(true)) => response(StatusCode::NO_CONTENT, Value::Null),
        Ok(Ok(false)) => response(
            StatusCode::NOT_FOUND,
            json!({ "error": "trip plan not found" }),
        ),
        Ok(Err(error)) => write_conflict_or_bad_request(error),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn add_item(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(input): Json<CreatePlanItem>,
) -> ApiResponse {
    let database_url = state.database_url.clone();
    match tokio::task::spawn_blocking(move || {
        TripsStore::open(&database_url)
            .and_then(|store| store.add_item(&id, &input))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(item)) => response(StatusCode::CREATED, item),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn delete_item(
    State(state): State<AppState>,
    Path((plan_id, item_id)): Path<(String, String)>,
) -> ApiResponse {
    let database_url = state.database_url.clone();
    match tokio::task::spawn_blocking(move || {
        TripsStore::open(&database_url)
            .and_then(|store| store.delete_item(&plan_id, &item_id))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(true)) => response(StatusCode::NO_CONTENT, Value::Null),
        Ok(Ok(false)) => response(
            StatusCode::NOT_FOUND,
            json!({ "error": "itinerary item not found" }),
        ),
        Ok(Err(error)) => response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn scan_obsidian(State(state): State<AppState>) -> ApiResponse {
    let Some(obsidian) = state.obsidian.clone() else {
        return response(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({ "error": "Obsidian import is not configured for this machine" }),
        );
    };
    let database_url = state.database_url.clone();
    match tokio::task::spawn_blocking(move || {
        let store = TripsStore::open(&database_url).map_err(|error| error.to_string())?;
        let mut candidates = scan_trip_notes(&obsidian.root, &obsidian.trips_dir)
            .map_err(|error| error.to_string())?;
        for candidate in &mut candidates {
            candidate.imported_plan_id = store
                .find_plan_by_source("obsidian", &candidate.reference)
                .map_err(|error| error.to_string())?
                .map(|plan| plan.id);
        }
        Ok::<_, String>(candidates)
    })
    .await
    {
        Ok(Ok(candidates)) => response(StatusCode::OK, candidates),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

#[derive(serde::Deserialize)]
struct ImportObsidianTrip {
    reference: String,
    origin: PlaceRef,
}

enum ImportOutcome {
    Imported(TripPlan),
    Existing(TripPlan),
}

impl ImportOutcome {
    fn plan(self) -> TripPlan {
        match self {
            Self::Imported(plan) | Self::Existing(plan) => plan,
        }
    }
}

#[derive(Serialize)]
struct SkippedObsidianTrip {
    reference: String,
    title: String,
    issues: Vec<String>,
}

#[derive(Serialize)]
struct ImportAllObsidianResult {
    imported: Vec<TripPlan>,
    existing: Vec<TripPlan>,
    skipped: Vec<SkippedObsidianTrip>,
}

fn import_obsidian_candidate(
    store: &TripsStore,
    candidate: ObsidianTripCandidate,
    origin: PlaceRef,
) -> Result<ImportOutcome, String> {
    if let Some(existing) = store
        .find_plan_by_source("obsidian", &candidate.reference)
        .map_err(|error| error.to_string())?
    {
        return Ok(ImportOutcome::Existing(existing));
    }
    if !candidate.issues.is_empty() {
        return Err(candidate.issues.join(" · "));
    }
    let destination = candidate
        .destination
        .ok_or_else(|| "Destination is missing".to_owned())?;
    let date_start = candidate
        .date_start
        .ok_or_else(|| "Start date is missing".to_owned())?;
    let date_end = candidate
        .date_end
        .ok_or_else(|| "End date is missing".to_owned())?;
    let cover_image_url = candidate
        .cover
        .as_ref()
        .filter(|value| value.starts_with("https://"))
        .cloned();
    let plan = store
        .create_plan(&CreatePlan {
            title: candidate.title.clone(),
            origin,
            destinations: vec![destination],
            date_start,
            date_end,
            interests: candidate.summary.clone(),
            travelers: candidate.travelers,
            transport_modes: candidate.transport_modes,
            stages: Vec::new(),
            cover_image_url,
            source: Some(PlanSource {
                kind: "obsidian".into(),
                reference: candidate.reference.clone(),
            }),
        })
        .map_err(|error| error.to_string())?;
    store
        .add_item(
            &plan.id,
            &CreatePlanItem {
                item_type: "note".into(),
                day: None,
                external_id: candidate.reference.clone(),
                title: "Obsidian-Reisenotiz".into(),
                payload: json!({
                    "vault_path": candidate.reference,
                    "summary": candidate.summary,
                    "status": candidate.status,
                    "cover": candidate.cover,
                }),
            },
        )
        .map_err(|error| error.to_string())?;
    let imported = store
        .get_plan(&plan.id)
        .map_err(|error| error.to_string())?
        .map(|details| details.plan)
        .ok_or_else(|| "imported trip could not be reloaded".to_owned())?;
    Ok(ImportOutcome::Imported(imported))
}

async fn import_obsidian(
    State(state): State<AppState>,
    Json(input): Json<ImportObsidianTrip>,
) -> ApiResponse {
    let Some(obsidian) = state.obsidian.clone() else {
        return response(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({ "error": "Obsidian import is not configured for this machine" }),
        );
    };
    let database_url = state.database_url.clone();
    match tokio::task::spawn_blocking(move || {
        let store = TripsStore::open(&database_url).map_err(|error| error.to_string())?;
        let candidate =
            read_trip_note(&obsidian.root, &input.reference).map_err(|error| error.to_string())?;
        import_obsidian_candidate(&store, candidate, input.origin).map(ImportOutcome::plan)
    })
    .await
    {
        Ok(Ok(plan)) => response(StatusCode::CREATED, plan),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn import_all_obsidian(
    State(state): State<AppState>,
    Json(input): Json<ImportAllObsidianTrips>,
) -> ApiResponse {
    let Some(obsidian) = state.obsidian.clone() else {
        return response(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({ "error": "Obsidian import is not configured for this machine" }),
        );
    };
    let database_url = state.database_url.clone();
    match tokio::task::spawn_blocking(move || {
        let store = TripsStore::open(&database_url).map_err(|error| error.to_string())?;
        let candidates = scan_trip_notes(&obsidian.root, &obsidian.trips_dir)
            .map_err(|error| error.to_string())?;
        let mut result = ImportAllObsidianResult {
            imported: Vec::new(),
            existing: Vec::new(),
            skipped: Vec::new(),
        };
        for candidate in candidates {
            let reference = candidate.reference.clone();
            let title = candidate.title.clone();
            match import_obsidian_candidate(&store, candidate, input.origin.clone()) {
                Ok(ImportOutcome::Imported(plan)) => result.imported.push(plan),
                Ok(ImportOutcome::Existing(plan)) => result.existing.push(plan),
                Err(error) => result.skipped.push(SkippedObsidianTrip {
                    reference,
                    title,
                    issues: vec![error],
                }),
            }
        }
        Ok::<_, String>(result)
    })
    .await
    {
        Ok(Ok(result)) => response(StatusCode::OK, result),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

#[derive(serde::Deserialize)]
struct ImportAllObsidianTrips {
    origin: PlaceRef,
}

#[tokio::main]
async fn main() {
    let config = Config::load();
    let state = AppState {
        database_url: Arc::new(config.database_url),
        obsidian: config.obsidian,
    };
    let app = Router::new()
        .route("/routes", get(routes))
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/api/plans", get(list_plans).post(create_plan))
        .route(
            "/api/plans/:id",
            get(get_plan).patch(update_plan).delete(delete_plan),
        )
        .route("/api/plans/:id/items", post(add_item))
        .route(
            "/api/plans/:plan_id/items/:item_id",
            delete(delete_item).patch(set_item_day),
        )
        .route("/api/places", get(list_places))
        .route("/api/plans/:id/outcome", post(record_outcome))
        .route("/api/import/obsidian/scan", get(scan_obsidian))
        .route("/api/import/obsidian/all", post(import_all_obsidian))
        .route("/api/import/obsidian", post(import_obsidian))
        .layer(CorsLayer::permissive())
        .with_state(state);
    // Loopback via axon_server; the old 0.0.0.0 bind here was never a
    // documented decision and is retired with it.
    axon_server::serve_local("trips-server", config.port, app).await;
}

#[cfg(test)]
mod readiness_tests {
    use super::*;

    /// The contract the dashboard depends on: an unreachable database is reported as
    /// unavailable rather than as a healthy service (#126). Before the split, the only
    /// surface axon-status polled was `health`, which is a literal and answers 200 here.
    #[tokio::test]
    async fn readiness_fails_when_the_database_is_unreachable() {
        // Port 1 is reserved and nothing listens there — the stopped-container case.
        let state = AppState {
            database_url: Arc::new(
                "host=127.0.0.1 port=1 user=axon password=axon dbname=axon".to_string(),
            ),
            obsidian: None,
        };

        let (status, Json(body)) = ready(State(state)).await;
        assert_eq!(
            status,
            StatusCode::SERVICE_UNAVAILABLE,
            "an unreachable database was reported as ready: {body}"
        );
        assert_eq!(body["ok"], false);

        // The control: liveness is deliberately unaffected, because the process is fine.
        let Json(live) = health().await;
        assert_eq!(live["ok"], true, "liveness must not depend on the database");
    }
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
