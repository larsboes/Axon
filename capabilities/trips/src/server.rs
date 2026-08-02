use std::sync::Arc;

use axum::{
    extract::{Path, State},
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
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
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

async fn delete_plan(State(state): State<AppState>, Path(id): Path<String>) -> ApiResponse {
    let database_url = state.database_url.clone();
    match tokio::task::spawn_blocking(move || {
        TripsStore::open(&database_url)
            .and_then(|store| store.delete_plan(&id))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(true)) => response(StatusCode::NO_CONTENT, Value::Null),
        Ok(Ok(false)) => response(
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
        .ok_or_else(|| "Zielort fehlt".to_owned())?;
    let date_start = candidate
        .date_start
        .ok_or_else(|| "Startdatum fehlt".to_owned())?;
    let date_end = candidate
        .date_end
        .ok_or_else(|| "Enddatum fehlt".to_owned())?;
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

#[path = "../../../libs/axon-server/src/lib.rs"]
#[allow(dead_code)]
mod axon_server;

#[tokio::main]
async fn main() {
    let config = Config::load();
    let state = AppState {
        database_url: Arc::new(config.database_url),
        obsidian: config.obsidian,
    };
    let app = Router::new()
        .route("/health", get(health))
        .route("/api/plans", get(list_plans).post(create_plan))
        .route(
            "/api/plans/:id",
            get(get_plan).patch(update_plan).delete(delete_plan),
        )
        .route("/api/plans/:id/items", post(add_item))
        .route("/api/plans/:plan_id/items/:item_id", delete(delete_item))
        .route("/api/import/obsidian/scan", get(scan_obsidian))
        .route("/api/import/obsidian/all", post(import_all_obsidian))
        .route("/api/import/obsidian", post(import_obsidian))
        .layer(CorsLayer::permissive())
        .with_state(state);
    // Loopback via axon_server; the old 0.0.0.0 bind here was never a
    // documented decision and is retired with it.
    axon_server::serve_local("trips-server", config.port, app).await;
}
