use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
    routing::{get, post, put},
    Router,
};
use serde_json::{json, Value};
use tower_http::cors::CorsLayer;

use calendar::config::Config;
use calendar::correlate::{self, Candidate};
use calendar::date;
use calendar::google_sync::{self, HttpCalendarApi, Settings};
use calendar::model::{
    NewContext, NewEntry, NewRhythm, UpdateContext, UpdateEntry, UpdateRhythm,
};
use calendar::store::CalendarStore;

#[derive(Clone)]
struct AppState {
    database_url: Arc<String>,
    config: Arc<Config>,
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
        "capability": "calendar"
    }))
}

#[derive(serde::Deserialize)]
struct EntriesQuery {
    from: String,
    to: String,
    /// Optional CSV kind filter: ?kind=busy,event
    kind: Option<String>,
}

/// Google drafts are source-owned imports still at `possible`, not entries of
/// a made-up `draft` kind. The dedicated endpoint keeps that meaning inside
/// Calendar rather than duplicating it in the dashboard.
#[derive(serde::Deserialize)]
struct GoogleDraftsQuery {
    from: String,
    to: String,
}

#[derive(serde::Deserialize)]
struct ProposalsQuery {
    from: String,
    to: String,
}

async fn list_entries(
    State(state): State<AppState>,
    Query(query): Query<EntriesQuery>,
) -> ApiResponse {
    let kinds: Vec<String> = query
        .kind
        .unwrap_or_default()
        .split(',')
        .map(|kind| kind.trim().to_string())
        .filter(|kind| !kind.is_empty())
        .collect();
    let from = query.from;
    let to = query.to;
    let database_url = state.database_url.clone();
    match tokio::task::spawn_blocking(move || {
        CalendarStore::open(&database_url)
            .and_then(|store| store.list_entries(&from, &to, &kinds))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(entries)) => response(StatusCode::OK, entries),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn list_google_drafts(
    State(state): State<AppState>,
    Query(query): Query<GoogleDraftsQuery>,
) -> ApiResponse {
    let database_url = state.database_url.clone();
    match tokio::task::spawn_blocking(move || {
        CalendarStore::open(&database_url)
            .and_then(|store| store.list_google_drafts(&query.from, &query.to))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(entries)) => response(StatusCode::OK, entries),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn list_external_proposals(
    State(state): State<AppState>,
    Query(query): Query<ProposalsQuery>,
) -> ApiResponse {
    let database_url = state.database_url.clone();
    match tokio::task::spawn_blocking(move || {
        CalendarStore::open(&database_url)
            .and_then(|store| store.list_external_proposals(&query.from, &query.to))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(entries)) => response(StatusCode::OK, entries),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn create_entry(State(state): State<AppState>, Json(input): Json<NewEntry>) -> ApiResponse {
    let database_url = state.database_url.clone();
    match tokio::task::spawn_blocking(move || {
        CalendarStore::open(&database_url)
            .and_then(|store| store.create_entry(&input))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(entry)) => response(StatusCode::CREATED, entry),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn upsert_external_entry(
    State(state): State<AppState>,
    Json(input): Json<NewEntry>,
) -> ApiResponse {
    let database_url = state.database_url.clone();
    match tokio::task::spawn_blocking(move || {
        CalendarStore::open(&database_url)
            .and_then(|store| store.upsert_external_entry(&input))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(entry)) => response(StatusCode::OK, entry),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn get_entry(State(state): State<AppState>, Path(id): Path<String>) -> ApiResponse {
    let database_url = state.database_url.clone();
    match tokio::task::spawn_blocking(move || {
        CalendarStore::open(&database_url)
            .and_then(|store| store.get_entry(&id))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(Some(entry))) => response(StatusCode::OK, entry),
        Ok(Ok(None)) => response(StatusCode::NOT_FOUND, json!({ "error": "entry not found" })),
        Ok(Err(error)) => response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn update_entry(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(input): Json<UpdateEntry>,
) -> ApiResponse {
    let database_url = state.database_url.clone();
    match tokio::task::spawn_blocking(move || {
        CalendarStore::open(&database_url)
            .and_then(|store| store.update_entry(&id, &input))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(Some(entry))) => response(StatusCode::OK, entry),
        Ok(Ok(None)) => response(StatusCode::NOT_FOUND, json!({ "error": "entry not found" })),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn delete_entry(State(state): State<AppState>, Path(id): Path<String>) -> ApiResponse {
    let database_url = state.database_url.clone();
    match tokio::task::spawn_blocking(move || {
        CalendarStore::open(&database_url)
            .and_then(|store| store.delete_entry(&id))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(true)) => response(StatusCode::OK, json!({ "deleted": true })),
        Ok(Ok(false)) => response(StatusCode::NOT_FOUND, json!({ "error": "entry not found" })),
        Ok(Err(error)) => response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

#[derive(serde::Deserialize)]
struct ContextsQuery {
    from: String,
    to: String,
}

async fn list_contexts(
    State(state): State<AppState>,
    Query(query): Query<ContextsQuery>,
) -> ApiResponse {
    let database_url = state.database_url.clone();
    match tokio::task::spawn_blocking(move || {
        CalendarStore::open(&database_url)
            .and_then(|store| store.list_contexts(&query.from, &query.to))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(contexts)) => response(StatusCode::OK, contexts),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn create_context(
    State(state): State<AppState>,
    Json(input): Json<NewContext>,
) -> ApiResponse {
    let database_url = state.database_url.clone();
    match tokio::task::spawn_blocking(move || {
        CalendarStore::open(&database_url)
            .and_then(|store| store.create_context(&input))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(context)) => response(StatusCode::CREATED, context),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn update_context(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(input): Json<UpdateContext>,
) -> ApiResponse {
    let database_url = state.database_url.clone();
    match tokio::task::spawn_blocking(move || {
        CalendarStore::open(&database_url)
            .and_then(|store| store.update_context(&id, &input))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(Some(context))) => response(StatusCode::OK, context),
        Ok(Ok(None)) => response(
            StatusCode::NOT_FOUND,
            json!({ "error": "context not found" }),
        ),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn delete_context(State(state): State<AppState>, Path(id): Path<String>) -> ApiResponse {
    let database_url = state.database_url.clone();
    match tokio::task::spawn_blocking(move || {
        CalendarStore::open(&database_url)
            .and_then(|store| store.delete_context(&id))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(true)) => response(StatusCode::OK, json!({ "deleted": true })),
        Ok(Ok(false)) => response(
            StatusCode::NOT_FOUND,
            json!({ "error": "context not found" }),
        ),
        Ok(Err(error)) => response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn list_rhythms(State(state): State<AppState>) -> ApiResponse {
    let database_url = state.database_url.clone();
    match tokio::task::spawn_blocking(move || {
        CalendarStore::open(&database_url)
            .and_then(|store| store.list_rhythms())
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(rhythms)) => response(StatusCode::OK, rhythms),
        Ok(Err(error)) => response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn create_rhythm(State(state): State<AppState>, Json(input): Json<NewRhythm>) -> ApiResponse {
    let database_url = state.database_url.clone();
    match tokio::task::spawn_blocking(move || {
        CalendarStore::open(&database_url)
            .and_then(|store| store.create_rhythm(&input))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok((rhythm, created))) => response(
            StatusCode::CREATED,
            json!({ "rhythm": rhythm, "instances_created": created }),
        ),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn get_rhythm(State(state): State<AppState>, Path(id): Path<String>) -> ApiResponse {
    let database_url = state.database_url.clone();
    match tokio::task::spawn_blocking(move || {
        CalendarStore::open(&database_url)
            .and_then(|store| store.get_rhythm(&id))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(Some(rhythm))) => response(StatusCode::OK, rhythm),
        Ok(Ok(None)) => response(
            StatusCode::NOT_FOUND,
            json!({ "error": "rhythm not found" }),
        ),
        Ok(Err(error)) => response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn update_rhythm(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(input): Json<UpdateRhythm>,
) -> ApiResponse {
    let database_url = state.database_url.clone();
    match tokio::task::spawn_blocking(move || {
        CalendarStore::open(&database_url)
            .and_then(|store| store.update_rhythm(&id, &input))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(Some((rhythm, affected)))) => response(
            StatusCode::OK,
            json!({ "rhythm": rhythm, "future_instances_affected": affected }),
        ),
        Ok(Ok(None)) => response(
            StatusCode::NOT_FOUND,
            json!({ "error": "rhythm not found" }),
        ),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

#[derive(serde::Deserialize)]
struct DeleteRhythmQuery {
    delete_instances: Option<bool>,
}

async fn delete_rhythm(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<DeleteRhythmQuery>,
) -> ApiResponse {
    let delete_instances = query.delete_instances.unwrap_or(false);
    let database_url = state.database_url.clone();
    match tokio::task::spawn_blocking(move || {
        CalendarStore::open(&database_url)
            .and_then(|store| store.delete_rhythm(&id, delete_instances))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(true)) => response(StatusCode::OK, json!({ "deleted": true })),
        Ok(Ok(false)) => response(
            StatusCode::NOT_FOUND,
            json!({ "error": "rhythm not found" }),
        ),
        Ok(Err(error)) => response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn materialize_rhythm(State(state): State<AppState>, Path(id): Path<String>) -> ApiResponse {
    let database_url = state.database_url.clone();
    match tokio::task::spawn_blocking(move || {
        CalendarStore::open(&database_url)
            .and_then(|store| store.materialize_rhythm(&id))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(Some(created))) => response(StatusCode::OK, json!({ "instances_created": created })),
        Ok(Ok(None)) => response(
            StatusCode::NOT_FOUND,
            json!({ "error": "rhythm not found" }),
        ),
        Ok(Err(error)) => response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

// ---- correlation (Phase C) ------------------------------------------------

#[derive(serde::Deserialize)]
struct VerdictsRequest {
    candidates: Vec<Candidate>,
}

/// Soft feasibility verdicts for a batch of dated candidates. Batched because
/// Feed's Entdecken view asks about a screenful of opportunities at once, and
/// one span-covering read beats one query per row.
async fn candidate_verdicts(
    State(state): State<AppState>,
    Json(input): Json<VerdictsRequest>,
) -> ApiResponse {
    let database_url = state.database_url.clone();
    match tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let candidates = input.candidates;
        let window = correlate::query_window(&candidates)?;
        let entries = match window {
            Some((from, to)) => CalendarStore::open(&database_url)
                .and_then(|store| store.list_entries(&from, &to, &[]))
                .map_err(|error| error.to_string())?,
            // No candidates, no query — an empty ask is not an error.
            None => Vec::new(),
        };
        let verdicts = correlate::verdicts_for(&candidates, &entries)?;
        Ok(json!({ "verdicts": verdicts }))
    })
    .await
    {
        Ok(Ok(body)) => response(StatusCode::OK, body),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

#[derive(serde::Deserialize)]
struct MaterializeBody {
    /// The entries this trip is made of. Explicit rather than "the draft at
    /// place X": drafts are recomputed per request, so naming one by position
    /// would race any edit made between reading and confirming.
    entry_ids: Vec<String>,
    #[serde(default)]
    title: Option<String>,
}

/// Turns a set of entries into a `trips.plan`.
///
/// Calendar posts to trips' public HTTP API and never touches its store. The
/// ledger it does own records which entries already became a plan, so asking
/// twice returns the plan that exists instead of making a second one.
async fn materialize_trip(
    State(state): State<AppState>,
    Json(body): Json<MaterializeBody>,
) -> ApiResponse {
    let database_url = state.database_url.clone();
    let config = state.config.clone();
    match tokio::task::spawn_blocking(move || -> Result<Value, String> {
        if body.entry_ids.is_empty() {
            return Err("entry_ids is required".into());
        }
        let store = CalendarStore::open(&database_url).map_err(|e| e.to_string())?;

        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .map_err(|e| format!("client build: {e}"))?;
        let base = config.trips_base_url.trim_end_matches('/').to_string();

        // Already a trip? Only if trips still has it. The ledger records what
        // calendar did, not what trips kept, so a plan deleted over there would
        // otherwise leave the entry permanently refusing to become one again
        // and pointing at something that no longer exists.
        for entry_id in &body.entry_ids {
            let Some(plan_id) = store.trip_plan_for(entry_id).map_err(|e| e.to_string())? else {
                continue;
            };
            let probe = client
                .get(format!("{base}/api/plans/{plan_id}"))
                .send()
                .map_err(|e| {
                    // Unreachable is not the same as gone. Forgetting the row
                    // here would turn a trips outage into a duplicate plan, so
                    // this fails loudly instead.
                    format!("cannot ask trips whether {plan_id} still exists ({e})")
                })?;
            if probe.status().is_success() {
                return Ok(json!({
                    "plan_id": plan_id,
                    "created": false,
                    "reason": format!("{entry_id} already belongs to {plan_id}"),
                }));
            }
            if probe.status() == reqwest::StatusCode::NOT_FOUND {
                let forgotten = store
                    .forget_trip_materialization(&plan_id)
                    .map_err(|e| e.to_string())?;
                eprintln!(
                    "  trips: {plan_id} is gone, forgetting {forgotten} stale ledger row(s)"
                );
                continue;
            }
            return Err(format!(
                "trips answered {} for {plan_id}, which is neither yes nor no",
                probe.status()
            ));
        }

        let mut entries = Vec::new();
        for entry_id in &body.entry_ids {
            let entry = store
                .get_entry(entry_id)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("no entry {entry_id}"))?;
            entries.push(entry);
        }

        let drafts = correlate::cluster_trips(&entries, i64::MAX, None)?;
        let draft = drafts
            .drafts
            .first()
            .ok_or("none of those entries can be placed, so there is nothing to travel to")?;
        if drafts.drafts.len() > 1 {
            return Err(format!(
                "those entries are in {} different places; one trip goes to one place",
                drafts.drafts.len()
            ));
        }

        // trips' date_end is INCLUSIVE; every end in calendar is exclusive.
        // Handing ends_before straight over would add a day to every trip, and
        // it would look like a rounding quirk rather than a unit mismatch.
        let date_end = date::parse_date(&draft.ends_before)
            .map(|day| date::format_date(day - 1))
            .ok_or("unreadable draft end")?;

        let title = body
            .title
            .clone()
            .unwrap_or_else(|| format!("{} — {}", draft.place, draft.starts_on));
        let payload = json!({
            "title": title,
            "origin": { "id": "", "name": config.home_city.clone().unwrap_or_default() },
            "destinations": [{ "id": "", "name": draft.place }],
            "date_start": draft.starts_on,
            "date_end": date_end,
            "interests": draft.titles.join(" · "),
        });

        let url = format!("{base}/api/plans");
        let response = client
            .post(&url)
            .json(&payload)
            .send()
            .map_err(|e| format!("POST {url}: {e}"))?;
        let status = response.status();
        let plan: Value = response
            .json()
            .map_err(|e| format!("trips returned something unreadable: {e}"))?;
        if !status.is_success() {
            return Err(format!("trips refused the plan ({status}): {plan}"));
        }
        let plan_id = plan
            .get("id")
            .and_then(|id| id.as_str())
            .ok_or("trips accepted the plan but returned no id")?
            .to_string();

        // Only now, after trips confirmed it exists.
        store
            .record_trip_materialization(&body.entry_ids, &plan_id)
            .map_err(|e| e.to_string())?;

        Ok(json!({ "plan_id": plan_id, "created": true, "plan": plan }))
    })
    .await
    {
        Ok(Ok(body)) => response(StatusCode::OK, body),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

#[derive(serde::Deserialize)]
struct TripDraftsQuery {
    from: String,
    to: String,
    /// How far apart two things in the same place can be and still be one
    /// journey. Defaults to the five days the issue's own example uses.
    max_gap_days: Option<i64>,
    /// The place that is never a trip. Falls back to `home_city` in the
    /// capability config, and passing neither clusters everything including
    /// where you live, which is visible rather than silently wrong.
    home: Option<String>,
}

/// Which events belong to one journey.
///
/// Recomputed per request rather than stored: a draft is a function of the
/// entries, and every one of them can move. Materialising a draft into a real
/// `trips.plan` is an explicit act elsewhere, which is what keeps this cheap
/// enough to recompute and keeps calendar out of trips' domain.
async fn trip_drafts(
    State(state): State<AppState>,
    Query(query): Query<TripDraftsQuery>,
) -> ApiResponse {
    let database_url = state.database_url.clone();
    let config_home = state.config.home_city.clone();
    match tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let max_gap_days = query.max_gap_days.unwrap_or(5);
        if max_gap_days < 0 {
            return Err("max_gap_days cannot be negative".into());
        }
        let home = query.home.clone().or(config_home);
        let entries = CalendarStore::open(&database_url)
            .and_then(|store| store.list_entries(&query.from, &query.to, &[]))
            .map_err(|error| error.to_string())?;
        let drafts = correlate::cluster_trips(&entries, max_gap_days, home.as_deref())?;
        Ok(json!({
            "from": query.from,
            "to": query.to,
            "max_gap_days": max_gap_days,
            "home": home,
            "drafts": drafts.drafts,
            "unclustered": drafts.unclustered,
        }))
    })
    .await
    {
        Ok(Ok(body)) => response(StatusCode::OK, body),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

#[derive(serde::Deserialize)]
struct WindowsQuery {
    from: String,
    to: String,
    /// Shortest run worth returning; defaults to a single day.
    min_days: Option<usize>,
}

/// The runs of days travel is possible in — what a fare search should be
/// constrained to. Calendar computes availability and stops there; handing
/// these days to `transit plan --dates` is the caller's move, so neither
/// capability learns the other's domain (see the README's why-block).
async fn windows(State(state): State<AppState>, Query(query): Query<WindowsQuery>) -> ApiResponse {
    let database_url = state.database_url.clone();
    match tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let from_day = date::parse_date(&query.from).ok_or("from must be YYYY-MM-DD")?;
        let to_day = date::parse_date(&query.to).ok_or("to must be YYYY-MM-DD")?;
        if to_day <= from_day {
            return Err("to must be after from (the window end is exclusive)".into());
        }
        let min_days = query.min_days.unwrap_or(1);
        let entries = CalendarStore::open(&database_url)
            .and_then(|store| store.list_entries(&query.from, &query.to, &[]))
            .map_err(|error| error.to_string())?;
        let windows = correlate::feasible_windows(from_day, to_day, &entries, min_days)?;
        Ok(json!({
            "from": query.from,
            "to": query.to,
            "min_days": min_days,
            "windows": windows,
        }))
    })
    .await
    {
        Ok(Ok(body)) => response(StatusCode::OK, body),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

// ---- google sync (Phase E) ------------------------------------------------

#[derive(serde::Deserialize, Default)]
struct SyncRequest {
    /// Reports what would happen and writes nothing. Not the default: a run
    /// the operator asked for should do the thing.
    #[serde(default)]
    dry_run: bool,
}

/// A deliberate, bounded provider slice for the import-review UI. Dates are
/// date-only and form an exclusive `[from, to)` window, like entry queries.
#[derive(serde::Deserialize)]
struct GoogleImportPreviewRequest {
    from: String,
    to: String,
}

#[derive(serde::Deserialize)]
struct GoogleSelectedImportRequest {
    from: String,
    to: String,
    selected: Vec<google_sync::SelectedGoogleEvent>,
}

/// Resolves the settings both runs need, turning a missing home timezone or
/// calendar id into a 400 that names the config key rather than a 500.
fn google_settings(state: &AppState) -> Result<Settings, ApiResponse> {
    Settings::resolve(&state.config)
        .map_err(|error| response(StatusCode::BAD_REQUEST, json!({ "error": error })))
}

/// Pulls the configured Google calendar in as drafts.
///
/// Blocking work — the Google client and the store are both synchronous — so
/// it runs on the blocking pool like every other handler here. Missing
/// credentials surface as a 400 naming the file and key, never as an empty
/// success.
async fn google_import(
    State(state): State<AppState>,
    Json(input): Json<SyncRequest>,
) -> ApiResponse {
    let settings = match google_settings(&state) {
        Ok(settings) => settings,
        Err(error) => return error,
    };
    let database_url = state.database_url.clone();
    match tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let store = CalendarStore::open(&database_url).map_err(|error| error.to_string())?;
        let env_path = settings.google.env_path();
        let api = HttpCalendarApi::new(&env_path);
        let report = google_sync::import(&store, &api, &settings, input.dry_run)?;
        serde_json::to_value(report).map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(body)) => response(StatusCode::OK, body),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

/// Read-only candidate review. This intentionally does not reuse the broad
/// unattended-import window: an operator should first see a small, chosen
/// time range and any likely duplicates.
async fn google_import_preview(
    State(state): State<AppState>,
    Json(input): Json<GoogleImportPreviewRequest>,
) -> ApiResponse {
    let settings = match google_settings(&state) {
        Ok(settings) => settings,
        Err(error) => return error,
    };
    let database_url = state.database_url.clone();
    match tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let store = CalendarStore::open(&database_url).map_err(|error| error.to_string())?;
        let env_path = settings.google.env_path();
        let api = HttpCalendarApi::new(&env_path);
        let preview = google_sync::preview(&store, &api, &settings, &input.from, &input.to)?;
        serde_json::to_value(preview).map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(body)) => response(StatusCode::OK, body),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

/// Commits only explicit, still-current selections from a prior preview. The
/// Google event revisions are checked before any Axon entry is written.
async fn google_import_selected(
    State(state): State<AppState>,
    Json(input): Json<GoogleSelectedImportRequest>,
) -> ApiResponse {
    let settings = match google_settings(&state) {
        Ok(settings) => settings,
        Err(error) => return error,
    };
    let database_url = state.database_url.clone();
    match tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let store = CalendarStore::open(&database_url).map_err(|error| error.to_string())?;
        let env_path = settings.google.env_path();
        let api = HttpCalendarApi::new(&env_path);
        let report = google_sync::import_selected(
            &store,
            &api,
            &settings,
            &input.from,
            &input.to,
            &input.selected,
        )?;
        serde_json::to_value(report).map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(body)) => response(StatusCode::OK, body),
        // A stale candidate is an expected review result, not a server error.
        Ok(Err(error)) if error.contains("review again") => {
            response(StatusCode::CONFLICT, json!({ "error": error }))
        }
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

/// Pushes the opted-in entries, and only those.
async fn google_export(
    State(state): State<AppState>,
    Json(input): Json<SyncRequest>,
) -> ApiResponse {
    let settings = match google_settings(&state) {
        Ok(settings) => settings,
        Err(error) => return error,
    };
    let database_url = state.database_url.clone();
    match tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let store = CalendarStore::open(&database_url).map_err(|error| error.to_string())?;
        let env_path = settings.google.env_path();
        let api = HttpCalendarApi::new(&env_path);
        let report = google_sync::export(&store, &api, &settings, input.dry_run)?;
        serde_json::to_value(report).map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(body)) => response(StatusCode::OK, body),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn list_export_optins(State(state): State<AppState>) -> ApiResponse {
    let database_url = state.database_url.clone();
    match tokio::task::spawn_blocking(move || {
        CalendarStore::open(&database_url)
            .and_then(|store| store.list_export_optins())
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(optins)) => response(StatusCode::OK, optins),
        Ok(Err(error)) => response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

#[derive(serde::Deserialize, Default)]
struct OptInRequest {
    /// Which Google calendar this entry belongs on. Defaults to the configured
    /// one; recorded on the ledger row so a later config change cannot
    /// relocate an event that has already been pushed.
    #[serde(default)]
    google_calendar_id: Option<String>,
}

/// Opts one entry in to export. Nothing exports until this is called, and
/// `store::opt_in_export` refuses the entries that must never be pushed (an
/// imported Google event, a generated rhythm instance).
async fn opt_in_export(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Option<Json<OptInRequest>>,
) -> ApiResponse {
    let requested = body.map(|Json(input)| input).unwrap_or_default();
    let calendar_id = match requested
        .google_calendar_id
        .or_else(|| state.config.google.calendar_id.clone())
    {
        Some(calendar_id) => calendar_id,
        None => {
            return response(
                StatusCode::BAD_REQUEST,
                json!({
                    "error": "no google_calendar_id given and none configured — set google.calendar_id in the overlay's calendar.json or pass it in the body"
                }),
            )
        }
    };
    let database_url = state.database_url.clone();
    match tokio::task::spawn_blocking(move || {
        CalendarStore::open(&database_url)
            .and_then(|store| store.opt_in_export(&id, &calendar_id))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(optin)) => response(StatusCode::OK, optin),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

/// Opts an entry back out. The Google event it already created is deliberately
/// left alone: deleting someone's calendar entry as a side effect of a toggle
/// is not a decision this capability makes.
async fn opt_out_export(State(state): State<AppState>, Path(id): Path<String>) -> ApiResponse {
    let database_url = state.database_url.clone();
    match tokio::task::spawn_blocking(move || {
        CalendarStore::open(&database_url)
            .and_then(|store| store.opt_out_export(&id))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(true)) => response(StatusCode::OK, json!({ "opted_out": true })),
        Ok(Ok(false)) => response(
            StatusCode::NOT_FOUND,
            json!({ "error": "entry is not opted in to export" }),
        ),
        Ok(Err(error)) => response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

#[path = "../../../libs/axon-server/src/lib.rs"]
#[allow(dead_code)]
mod axon_server;

#[tokio::main]
async fn main() {
    let config = Config::load();
    let port = config.port;
    let state = AppState {
        database_url: Arc::new(config.database_url.clone()),
        config: Arc::new(config),
    };
    let app = Router::new()
        .route("/health", get(health))
        .route("/api/entries", get(list_entries).post(create_entry))
        .route("/api/google/drafts", get(list_google_drafts))
        .route("/api/proposals", get(list_external_proposals))
        .route("/api/entries/external", put(upsert_external_entry))
        .route(
            "/api/entries/:id",
            get(get_entry).patch(update_entry).delete(delete_entry),
        )
        .route("/api/contexts", get(list_contexts).post(create_context))
        .route(
            "/api/contexts/:id",
            axum::routing::patch(update_context).delete(delete_context),
        )
        .route("/api/rhythms", get(list_rhythms).post(create_rhythm))
        .route(
            "/api/rhythms/:id",
            get(get_rhythm).patch(update_rhythm).delete(delete_rhythm),
        )
        .route("/api/rhythms/:id/materialize", post(materialize_rhythm))
        .route("/api/verdicts", post(candidate_verdicts))
        .route("/api/windows", get(windows))
        .route("/api/trip-drafts", get(trip_drafts))
        .route("/api/trip-drafts/materialize", post(materialize_trip))
        .route("/api/google/import", post(google_import))
        .route("/api/google/import-preview", post(google_import_preview))
        .route("/api/google/import-selected", post(google_import_selected))
        .route("/api/google/export", post(google_export))
        .route("/api/google/exports", get(list_export_optins))
        .route(
            "/api/entries/:id/google-export",
            put(opt_in_export).delete(opt_out_export),
        )
        .layer(CorsLayer::permissive())
        .with_state(state);
    axon_server::serve_local("calendar-server", port, app).await;
}
