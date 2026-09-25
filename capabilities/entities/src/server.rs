//! entities HTTP surface (port 8097), and the `sync-obsidian` / `sync-google` commands.
//!
//! Same shape as `capabilities/traveler`: blocking store work in `spawn_blocking`, `/ready`
//! proves the database, `/routes` serves the manifest the tests check against this file.
//! No CORS: this serves C2, and the shared origin guard refuses a foreign browser origin.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};

use entities::config::Config;
use entities::model::{located_on, FieldDef};
use entities::places::{self, Resolved};
use entities::store::{check_kind, check_new_fact, EntitiesStore, NewFact, Patch, StoreError};

const ROUTES: &[route_manifest::Route] = &[
    route_manifest::get("GET", "/health", "Liveness."),
    route_manifest::get(
        "GET",
        "/ready",
        "Readiness: liveness plus a reachable database.",
    ),
    route_manifest::get("GET", "/routes", "This manifest."),
    route_manifest::get(
        "GET",
        "/api/fields",
        "The field registry: built-in and declared fields, per kind. Optional ?kind=.",
    ),
    route_manifest::Route {
        method: "POST",
        path: "/api/fields",
        summary: "Declare a field for a kind: { kind, key (snake_case), label, field_type, \
                  options (enum only), data_class }. An existing key is refused with 400.",
        request_schema: Some(route_manifest::schema_of::<FieldDef>),
    },
    route_manifest::get(
        "GET",
        "/api/entities",
        "Entities with their values and dated facts, name order. Optional ?kind= and ?q= \
         (name contains, case-insensitive).",
    ),
    route_manifest::Route {
        method: "POST",
        path: "/api/entities",
        summary: "Create an entity: { kind, name, note_ref?, values?, source? }. Every value is \
                  checked against the kind's fields; an unknown key is refused.",
        request_schema: Some(route_manifest::schema_of::<CreateRequest>),
    },
    route_manifest::get(
        "GET",
        "/api/entities/:id",
        "One entity with its values and facts.",
    ),
    route_manifest::Route {
        method: "PATCH",
        path: "/api/entities/:id",
        summary: "Change an entity: { name?, note_ref?, values?, source?, expected_revision? }. \
                  A value of null clears the field. With expected_revision, a concurrent write \
                  is refused with 409 and the current revision.",
        request_schema: Some(route_manifest::schema_of::<PatchRequest>),
    },
    route_manifest::get(
        "DELETE",
        "/api/entities/:id",
        "Delete an entity with its values and facts.",
    ),
    route_manifest::Route {
        method: "POST",
        path: "/api/entities/:id/facts",
        summary: "Add a dated place fact: { predicate: home_base|away, place, valid_from?, \
                  valid_to?, note?, source? }. An away period needs both dates. The place text \
                  (never the name) goes to places for a coordinate; the fact is stored even when \
                  none is found, and `geocode` in the reply says what happened.",
        request_schema: Some(route_manifest::schema_of::<FactRequest>),
    },
    route_manifest::get(
        "DELETE",
        "/api/entities/:id/facts/:fact_id",
        "Delete one dated fact.",
    ),
    route_manifest::get(
        "GET",
        "/api/located",
        "Where each person is on ?day=YYYY-MM-DD (default today): an away period covering it, \
         else the home base that holds. With coordinates and the sleeping option, for the \
         dashboard's per-leg join. C2: names; local callers only.",
    ),
];

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CreateRequest {
    kind: String,
    name: String,
    #[serde(default)]
    note_ref: Option<String>,
    #[serde(default)]
    values: BTreeMap<String, Value>,
    #[serde(default)]
    source: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PatchRequest {
    #[serde(default)]
    name: Option<String>,
    /// Present and null clears the link; absent leaves it.
    #[serde(default, deserialize_with = "present")]
    note_ref: Option<Option<String>>,
    #[serde(default)]
    values: BTreeMap<String, Value>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    expected_revision: Option<u32>,
}

/// Distinguishes `"note_ref": null` (clear) from an absent key (leave).
fn present<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(d).map(Some)
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct FactRequest {
    predicate: String,
    place: String,
    #[serde(default)]
    valid_from: Option<String>,
    #[serde(default)]
    valid_to: Option<String>,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    source: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ListQuery {
    kind: Option<String>,
    q: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DayQuery {
    day: Option<String>,
}

struct AppState {
    store: EntitiesStore,
    places_url: String,
}

struct ApiError(StatusCode, Value);

impl From<StoreError> for ApiError {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::Invalid(m) => Self(StatusCode::BAD_REQUEST, json!({ "error": m })),
            StoreError::NotFound(m) => Self(StatusCode::NOT_FOUND, json!({ "error": m })),
            StoreError::Stale { current_revision } => Self(
                StatusCode::CONFLICT,
                json!({
                    "error": "the entity changed since you read it",
                    "code": "stale_entity",
                    "current_revision": current_revision,
                }),
            ),
            StoreError::Db(m) => Self(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": m })),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(self.1)).into_response()
    }
}

type Reply = Result<Json<Value>, ApiError>;

/// Runs store work off the async runtime.
async fn blocking<T: Send + 'static>(
    state: &Arc<AppState>,
    work: impl FnOnce(&AppState) -> Result<T, StoreError> + Send + 'static,
) -> Result<T, ApiError> {
    let state = state.clone();
    tokio::task::spawn_blocking(move || work(&state))
        .await
        .map_err(|e| {
            ApiError(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({ "error": e.to_string() }),
            )
        })?
        .map_err(ApiError::from)
}

fn to_json<T: serde::Serialize>(value: T) -> Json<Value> {
    Json(serde_json::to_value(value).unwrap_or(Value::Null))
}

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok", "service": "entities" }))
}

async fn ready(State(state): State<Arc<AppState>>) -> Reply {
    blocking(&state, |s| s.store.ping()).await?;
    Ok(Json(json!({ "status": "ready", "service": "entities" })))
}

async fn routes() -> Json<Value> {
    Json(route_manifest::manifest("entities", ROUTES))
}

async fn list_fields(State(state): State<Arc<AppState>>, Query(q): Query<ListQuery>) -> Reply {
    let fields = blocking(&state, move |s| s.store.fields(q.kind.as_deref())).await?;
    Ok(Json(json!({ "fields": fields })))
}

async fn declare_field(State(state): State<Arc<AppState>>, Json(def): Json<FieldDef>) -> Reply {
    blocking(&state, move |s| s.store.declare_field(def))
        .await
        .map(to_json)
}

async fn list_entities(State(state): State<Arc<AppState>>, Query(q): Query<ListQuery>) -> Reply {
    let entities = blocking(&state, move |s| {
        if let Some(kind) = &q.kind {
            check_kind(kind)?;
        }
        s.store.list(q.kind.as_deref(), q.q.as_deref())
    })
    .await?;
    Ok(Json(json!({ "entities": entities })))
}

async fn create_entity(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateRequest>,
) -> Response {
    let source = body.source.unwrap_or_else(|| "operator".into());
    match blocking(&state, move |s| {
        s.store.create(
            &body.kind,
            &body.name,
            body.note_ref.as_deref(),
            &body.values,
            &source,
        )
    })
    .await
    {
        Ok(entity) => (StatusCode::CREATED, to_json(entity)).into_response(),
        Err(error) => error.into_response(),
    }
}

async fn get_entity(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Reply {
    blocking(&state, move |s| {
        s.store
            .get(&id)?
            .ok_or(StoreError::NotFound(format!("no entity {id}")))
    })
    .await
    .map(to_json)
}

async fn patch_entity(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<PatchRequest>,
) -> Reply {
    let patch = Patch {
        name: body.name,
        note_ref: body.note_ref,
        values: body.values,
        source: body.source.unwrap_or_else(|| "operator".into()),
        expected_revision: body.expected_revision,
    };
    blocking(&state, move |s| s.store.patch(&id, &patch))
        .await
        .map(to_json)
}

async fn delete_entity(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match blocking(&state, move |s| s.store.delete(&id)).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => {
            ApiError(StatusCode::NOT_FOUND, json!({ "error": "no such entity" })).into_response()
        }
        Err(error) => error.into_response(),
    }
}

async fn add_fact(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<FactRequest>,
) -> Response {
    let result = blocking(&state, move |s| {
        // Checked before the geocode, so a refused fact sends nothing to places.
        if s.store.get(&id)?.is_none() {
            return Err(StoreError::NotFound(format!("no entity {id}")));
        }
        let mut new = NewFact {
            predicate: body.predicate,
            place: body.place,
            valid_from: body.valid_from.filter(|d| !d.trim().is_empty()),
            valid_to: body.valid_to.filter(|d| !d.trim().is_empty()),
            note: body.note,
            source: body.source.unwrap_or_else(|| "operator".into()),
            ..NewFact::default()
        };
        check_new_fact(&new)?;
        let geocode = match places::resolve(&s.places_url, new.place.trim()) {
            Resolved::At {
                latitude,
                longitude,
                name,
            } => {
                new.latitude = Some(latitude);
                new.longitude = Some(longitude);
                json!({ "status": "found", "name": name })
            }
            Resolved::NotFound => json!({ "status": "not_found" }),
            Resolved::Unavailable(reason) => json!({ "status": "unavailable", "reason": reason }),
        };
        let fact = s.store.add_fact(&id, &new)?;
        Ok(json!({ "fact": fact, "geocode": geocode }))
    })
    .await;
    match result {
        Ok(body) => (StatusCode::CREATED, Json(body)).into_response(),
        Err(error) => error.into_response(),
    }
}

async fn delete_fact(
    State(state): State<Arc<AppState>>,
    Path((id, fact_id)): Path<(String, String)>,
) -> Response {
    match blocking(&state, move |s| s.store.delete_fact(&id, &fact_id)).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => {
            ApiError(StatusCode::NOT_FOUND, json!({ "error": "no such fact" })).into_response()
        }
        Err(error) => error.into_response(),
    }
}

async fn located(State(state): State<Arc<AppState>>, Query(q): Query<DayQuery>) -> Reply {
    let day = q.day.unwrap_or_else(civil_date::today);
    if !entities::model::is_day(&day) {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            json!({ "error": format!("{day:?} is not a YYYY-MM-DD date") }),
        ));
    }
    let people = blocking(&state, |s| s.store.list(Some("person"), None)).await?;
    let located: Vec<Value> = people
        .iter()
        .filter_map(|person| {
            let fact = located_on(&person.facts, &day)?;
            Some(json!({
                "entity_id": person.id,
                "name": person.name,
                "predicate": fact.predicate,
                "place": fact.place,
                "latitude": fact.latitude,
                "longitude": fact.longitude,
                "sleeping_option": person.values.get("sleeping_option").map(|v| &v.value),
                "sleeping_note": person.values.get("sleeping_note").map(|v| &v.value),
            }))
        })
        .collect();
    Ok(Json(json!({ "day": day, "located": located })))
}

fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/routes", get(routes))
        .route("/api/fields", get(list_fields).post(declare_field))
        .route("/api/entities", get(list_entities).post(create_entity))
        .route(
            "/api/entities/:id",
            get(get_entity).patch(patch_entity).delete(delete_entity),
        )
        .route("/api/entities/:id/facts", post(add_fact))
        .route("/api/entities/:id/facts/:fact_id", delete(delete_fact))
        .route("/api/located", get(located))
        // Below every route: `layer` wraps only what is registered before it
        // (libs/axon-server/src/origin.rs).
        .layer(middleware::from_fn_with_state(
            "entities",
            axon_server::origin::refuse_foreign_origins,
        ))
        .with_state(state)
}

/// Runs one adapter's inbound sync and prints what it did.
fn run_sync(
    config: &Config,
    store: &EntitiesStore,
    system: &str,
    dry_run: bool,
) -> Result<(), String> {
    let (records, managed): (Vec<entities::sync::Incoming>, &[&str]) = match system {
        "obsidian" => {
            let vault_url =
                std::env::var("AXON_VAULT_URL").unwrap_or_else(|_| "http://127.0.0.1:8094".into());
            (
                entities::obsidian::fetch(&vault_url)?,
                entities::obsidian::MANAGED,
            )
        }
        "google" => {
            let people = entities::google::fetch()?;
            let skipped = people.len();
            let records: Vec<_> = people.iter().filter_map(entities::google::record).collect();
            if skipped > records.len() {
                println!(
                    "sync-google: {} contacts without a name skipped",
                    skipped - records.len()
                );
            }
            (records, entities::google::MANAGED)
        }
        other => return Err(format!("unknown system {other:?}")),
    };
    let report = entities::sync::apply(
        store,
        &config.places_url,
        system,
        managed,
        &records,
        dry_run,
    )
    .map_err(|e| e.to_string())?;
    println!(
        "sync-{system}{}: {} records, {} created, {} linked by name, {} updated, {} unchanged, {} home bases set",
        if dry_run { " (dry run)" } else { "" },
        report.records,
        report.created,
        report.linked_by_name,
        report.updated,
        report.unchanged,
        report.homes_set,
    );
    for refused in &report.refused {
        println!("  refused {refused}");
    }
    Ok(())
}

pub async fn serve(config: Config, store: EntitiesStore) {
    let state = Arc::new(AppState {
        store,
        places_url: config.places_url.clone(),
    });
    axon_server::serve_local("entities", config.port, router(state)).await;
}

fn main() {
    let config = Config::load();
    let store = match EntitiesStore::open(&config.database_path) {
        Ok(store) => store,
        Err(error) => {
            eprintln!("entities: cannot open store: {error}");
            std::process::exit(1);
        }
    };
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some(command @ ("sync-obsidian" | "import-obsidian" | "sync-google")) => {
            let system = if command == "sync-google" {
                "google"
            } else {
                "obsidian"
            };
            let dry_run = args.iter().any(|a| a == "--dry-run");
            if let Err(error) = run_sync(&config, &store, system, dry_run) {
                eprintln!("entities: {error}");
                std::process::exit(1);
            }
        }
        None => tokio::runtime::Runtime::new()
            .expect("tokio runtime could not start")
            .block_on(serve(config, store)),
        Some(other) => {
            eprintln!(
                "usage: entities-server [sync-obsidian|sync-google [--dry-run]] (got {other:?})"
            );
            std::process::exit(64);
        }
    }
}

#[cfg(test)]
mod route_manifest_tests {
    use super::*;

    #[test]
    fn the_manifest_covers_every_served_route() {
        let missing = route_manifest::undeclared_routes(include_str!("server.rs"), ROUTES);
        assert!(missing.is_empty(), "undeclared routes: {missing:?}");
    }

    #[test]
    fn every_write_route_declares_its_request_schema() {
        let missing = route_manifest::bodies_without_schemas(ROUTES);
        assert!(missing.is_empty(), "bodies with no schema: {missing:?}");
    }
}

#[cfg(test)]
mod http_tests {
    use super::*;

    async fn scratch_server(name: &str) -> (String, std::path::PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("axon-entities-http-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = EntitiesStore::open(&dir.join("axon.db")).unwrap();
        // Port 9 answers nothing, so a geocode is "unavailable" and no request leaves.
        let state = Arc::new(AppState {
            store,
            places_url: "http://127.0.0.1:9".into(),
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, router(state)).await;
        });
        (format!("http://{addr}"), dir)
    }

    #[tokio::test]
    async fn a_person_with_an_away_period_is_located_there_on_its_days() {
        let (base, dir) = scratch_server("located").await;
        let client = reqwest::Client::new();
        let ron: Value = client
            .post(format!("{base}/api/entities"))
            .json(
                &json!({ "kind": "person", "name": "Ron", "values": { "sleeping_option": "ask" } }),
            )
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let id = ron["id"].as_str().unwrap();
        for body in [
            json!({ "predicate": "home_base", "place": "Bonn" }),
            json!({ "predicate": "away", "place": "Lisbon", "valid_from": "2026-10-04", "valid_to": "2026-10-18" }),
        ] {
            let added = client
                .post(format!("{base}/api/entities/{id}/facts"))
                .json(&body)
                .send()
                .await
                .unwrap();
            assert_eq!(added.status(), 201);
            let reply: Value = added.json().await.unwrap();
            assert_eq!(
                reply["geocode"]["status"],
                json!("unavailable"),
                "stored without a coordinate"
            );
        }
        let on = |day: &'static str| {
            let url = format!("{base}/api/located?day={day}");
            async move {
                reqwest::get(url)
                    .await
                    .unwrap()
                    .json::<Value>()
                    .await
                    .unwrap()
            }
        };
        assert_eq!(
            on("2026-10-10").await["located"][0]["place"],
            json!("Lisbon")
        );
        let after = on("2026-10-20").await;
        assert_eq!(after["located"][0]["place"], json!("Bonn"));
        assert_eq!(after["located"][0]["sleeping_option"], json!("ask"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_stale_patch_is_a_409_and_a_foreign_origin_is_refused() {
        let (base, dir) = scratch_server("stale").await;
        let client = reqwest::Client::new();
        let ron: Value = client
            .post(format!("{base}/api/entities"))
            .json(&json!({ "kind": "person", "name": "Ron" }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let id = ron["id"].as_str().unwrap();
        let stale = client
            .patch(format!("{base}/api/entities/{id}"))
            .json(&json!({ "name": "Ronald", "expected_revision": 7 }))
            .send()
            .await
            .unwrap();
        assert_eq!(stale.status(), 409);
        assert_eq!(
            stale.json::<Value>().await.unwrap()["current_revision"],
            json!(1)
        );

        let foreign = client
            .get(format!("{base}/api/entities"))
            .header("Origin", "https://evil.example")
            .send()
            .await
            .unwrap();
        assert_eq!(foreign.status(), 403);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_refused_fact_is_a_400_naming_the_rule() {
        let (base, dir) = scratch_server("refused").await;
        let client = reqwest::Client::new();
        let ron: Value = client
            .post(format!("{base}/api/entities"))
            .json(&json!({ "kind": "person", "name": "Ron" }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let id = ron["id"].as_str().unwrap();
        let refused = client
            .post(format!("{base}/api/entities/{id}/facts"))
            .json(&json!({ "predicate": "away", "place": "Lisbon", "valid_from": "2026-10-04" }))
            .send()
            .await
            .unwrap();
        assert_eq!(refused.status(), 400);
        let body: Value = refused.json().await.unwrap();
        assert!(body["error"]
            .as_str()
            .unwrap()
            .contains("both valid_from and valid_to"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
