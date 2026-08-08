use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tower_http::cors::CorsLayer;

use finance::config::{Config, ObsidianConfig};
use finance::obsidian::{self, WriteBack};
use finance::store::FinanceStore;
use finance::subscription::{burn_at, cents_to_decimal, PricePoint, StateChange};

/// What this capability answers, served as data beside `/health`. Query parameters
/// are named in the summary: a path alone cannot tell a caller what to send, and
/// learning it from a 400 is what this endpoint exists to prevent.
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
        "/api/subscriptions",
        "Every subscription with its full price and state history.",
    ),
    r(
        "GET",
        "/api/subscriptions/burn",
        "Monthly and annual burn, computed from the price series. Optional ?at=YYYY-MM-DD, default today.",
    ),
    r(
        "POST",
        "/api/subscriptions/:id/price",
        "Append a price point. Body: valid_from, amount_cents, currency, cycle, reason.",
    ),
    r(
        "POST",
        "/api/subscriptions/:id/state",
        "Append a state change. Body: effective, state, note.",
    ),
    r(
        "GET",
        "/api/import/obsidian/scan",
        "Vault subscription notes that could be imported. Read-only.",
    ),
    r(
        "POST",
        "/api/import/obsidian",
        "Import every scanned vault subscription note. Idempotent by vault path.",
    ),
    r(
        "POST",
        "/api/writeback",
        "Regenerate the derived block in each note. Conflicts are reported, never resolved.",
    ),
];

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
    Json(route_manifest::manifest("finance", ROUTES))
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

fn failed(error: String) -> ApiResponse {
    response(
        StatusCode::INTERNAL_SERVER_ERROR,
        json!({ "ok": false, "capability": "finance", "error": error }),
    )
}

/// No vault configured is a 409 rather than a 500: nothing is broken, the operator
/// has not pointed this capability at anything yet, and a stack trace would suggest
/// otherwise.
fn no_vault() -> ApiResponse {
    response(
        StatusCode::CONFLICT,
        json!({
            "ok": false,
            "capability": "finance",
            "error": "no vault configured; set the overlay's config/finance.json or AXON_FINANCE_OBSIDIAN_ROOT"
        }),
    )
}

async fn health() -> Json<Value> {
    Json(json!({ "ok": true, "capability": "finance" }))
}

/// Readiness: whether this capability can actually serve, which liveness cannot
/// answer. `health` is a literal, so during a Postgres outage it would report up
/// while every query behind it failed (#126).
async fn ready(State(state): State<AppState>) -> ApiResponse {
    let database_url = state.database_url.clone();
    match tokio::task::spawn_blocking(move || {
        FinanceStore::open(&database_url)
            .and_then(|store| store.ping())
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(())) => response(
            StatusCode::OK,
            json!({ "ok": true, "capability": "finance" }),
        ),
        // 503, not 500: the request was fine, the dependency is not, and a caller
        // that retries should be told to come back rather than to fix its input.
        Ok(Err(error)) => response(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({ "ok": false, "capability": "finance", "error": error }),
        ),
        Err(_) => response(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({ "ok": false, "capability": "finance", "error": "readiness check failed" }),
        ),
    }
}

async fn list_subscriptions(State(state): State<AppState>) -> ApiResponse {
    let database_url = state.database_url.clone();
    match tokio::task::spawn_blocking(move || {
        FinanceStore::open(&database_url)
            .and_then(|store| store.list())
            .map_err(|e| e.to_string())
    })
    .await
    {
        Ok(Ok(subs)) => response(StatusCode::OK, subs),
        Ok(Err(e)) => failed(e),
        Err(_) => failed("task panicked".into()),
    }
}

#[derive(Debug, Deserialize)]
struct AtQuery {
    at: Option<String>,
}

/// Burn on a date, computed from each subscription's series.
///
/// There is no stored total to return, by design: a cached figure is a second
/// source of truth that goes stale the moment a price point lands.
async fn burn(State(state): State<AppState>, Query(query): Query<AtQuery>) -> ApiResponse {
    let database_url = state.database_url.clone();
    let at = query.at.unwrap_or_else(today);
    let at_for_body = at.clone();
    match tokio::task::spawn_blocking(move || {
        FinanceStore::open(&database_url)
            .and_then(|store| store.list())
            .map_err(|e| e.to_string())
    })
    .await
    {
        Ok(Ok(subs)) => {
            let burn = burn_at(&subs, &at);
            response(
                StatusCode::OK,
                json!({
                    "at": at_for_body,
                    "monthly_cents": burn.monthly_cents,
                    "annual_cents": burn.annual_cents,
                    "monthly": cents_to_decimal(burn.monthly_cents),
                    "annual": cents_to_decimal(burn.annual_cents),
                    "billing_count": burn.billing_count,
                    "total_count": subs.len(),
                }),
            )
        }
        Ok(Err(e)) => failed(e),
        Err(_) => failed("task panicked".into()),
    }
}

async fn append_price(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(price): Json<PricePoint>,
) -> ApiResponse {
    let database_url = state.database_url.clone();
    let now = today();
    match tokio::task::spawn_blocking(move || {
        FinanceStore::open(&database_url)
            .and_then(|store| store.append_price(&id, &price, &now).map(|_| id.clone()))
            .map_err(|e| e.to_string())
    })
    .await
    {
        Ok(Ok(id)) => response(StatusCode::CREATED, json!({ "ok": true, "id": id })),
        Ok(Err(e)) => failed(e),
        Err(_) => failed("task panicked".into()),
    }
}

async fn append_state(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(change): Json<StateChange>,
) -> ApiResponse {
    let database_url = state.database_url.clone();
    let now = today();
    match tokio::task::spawn_blocking(move || {
        FinanceStore::open(&database_url)
            .and_then(|store| store.append_state(&id, &change, &now).map(|_| id.clone()))
            .map_err(|e| e.to_string())
    })
    .await
    {
        Ok(Ok(id)) => response(StatusCode::CREATED, json!({ "ok": true, "id": id })),
        Ok(Err(e)) => failed(e),
        Err(_) => failed("task panicked".into()),
    }
}

async fn scan_vault(State(state): State<AppState>) -> ApiResponse {
    let Some(vault) = state.obsidian.clone() else {
        return no_vault();
    };
    match tokio::task::spawn_blocking(move || scan_notes(&vault)).await {
        Ok(Ok(notes)) => response(
            StatusCode::OK,
            json!({
                "count": notes.len(),
                "notes": notes.iter().map(|n| json!({
                    "name": n.name,
                    "source_path": n.source_path,
                })).collect::<Vec<_>>(),
            }),
        ),
        Ok(Err(e)) => failed(e),
        Err(_) => failed("task panicked".into()),
    }
}

async fn import_vault(State(state): State<AppState>) -> ApiResponse {
    let Some(vault) = state.obsidian.clone() else {
        return no_vault();
    };
    let database_url = state.database_url.clone();
    let now = today();
    match tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let notes = scan_notes(&vault)?;
        let store = FinanceStore::open(&database_url).map_err(|e| e.to_string())?;
        let (mut created, mut existing) = (0usize, 0usize);
        for note in &notes {
            let (_, is_new) = store.import_note(note, &now).map_err(|e| e.to_string())?;
            if is_new {
                created += 1;
            } else {
                existing += 1;
            }
        }
        Ok(json!({ "ok": true, "created": created, "already_present": existing }))
    })
    .await
    {
        Ok(Ok(body)) => response(StatusCode::OK, body),
        Ok(Err(e)) => failed(e),
        Err(_) => failed("task panicked".into()),
    }
}

/// Regenerate every note's derived block.
///
/// A conflict is reported and counted, never resolved. The response names each
/// conflicting note so the operator can look at it, which is the entire difference
/// between this and a machine that overwrites what somebody wrote.
async fn writeback(State(state): State<AppState>) -> ApiResponse {
    let Some(vault) = state.obsidian.clone() else {
        return no_vault();
    };
    let database_url = state.database_url.clone();
    let now = today();
    match tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let notes = scan_notes(&vault)?;
        let store = FinanceStore::open(&database_url).map_err(|e| e.to_string())?;
        let subs = store.list().map_err(|e| e.to_string())?;

        let (mut written, mut unchanged) = (0usize, 0usize);
        let mut conflicts: Vec<String> = Vec::new();
        let mut unimported: Vec<String> = Vec::new();

        for note in &notes {
            let Some(sub) = subs.iter().find(|s| s.source_path == note.source_path) else {
                unimported.push(note.source_path.clone());
                continue;
            };
            match obsidian::write_block(&note.absolute, sub, &now).map_err(|e| e.to_string())? {
                WriteBack::Created | WriteBack::Updated => written += 1,
                WriteBack::Unchanged => unchanged += 1,
                WriteBack::Conflict { .. } => conflicts.push(note.source_path.clone()),
            }
        }

        Ok(json!({
            "ok": conflicts.is_empty(),
            "written": written,
            "unchanged": unchanged,
            "conflicts": conflicts,
            "not_imported": unimported,
        }))
    })
    .await
    {
        Ok(Ok(body)) => response(StatusCode::OK, body),
        Ok(Err(e)) => failed(e),
        Err(_) => failed("task panicked".into()),
    }
}

fn scan_notes(vault: &ObsidianConfig) -> Result<Vec<finance::ScannedNote>, String> {
    let root =
        markdown_root::MarkdownRoot::declare(vault.root.clone()).map_err(|e| e.to_string())?;
    obsidian::scan(&root, &vault.subscriptions_dir).map_err(|e| e.to_string())
}

/// Today as an ISO date, from the wall clock, with no date dependency.
///
/// Days since the Unix epoch converted through the civil-from-days algorithm
/// (Howard Hinnant's, public domain). It is UTC: a subscription's billing date is
/// not precise to the hour, and a timezone database would be a dependency bought
/// for a boundary case that does not exist here.
fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    civil_from_days(secs / 86_400)
}

fn civil_from_days(days: i64) -> String {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    format!("{year:04}-{m:02}-{d:02}")
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
        .route("/api/subscriptions", get(list_subscriptions))
        .route("/api/subscriptions/burn", get(burn))
        .route("/api/subscriptions/:id/price", post(append_price))
        .route("/api/subscriptions/:id/state", post(append_state))
        .route("/api/import/obsidian/scan", get(scan_vault))
        .route("/api/import/obsidian", post(import_vault))
        .route("/api/writeback", post(writeback))
        .layer(CorsLayer::permissive())
        .with_state(state);
    axon_server::serve_local("finance-server", config.port, app).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_date_conversion_matches_known_days() {
        assert_eq!(civil_from_days(0), "1970-01-01");
        assert_eq!(civil_from_days(19_723), "2024-01-01");
        // 2024 was a leap year; the day after 02-28 is 02-29, not 03-01.
        assert_eq!(civil_from_days(19_782), "2024-02-29");
        assert_eq!(civil_from_days(20_673), "2026-08-08");
    }

    #[test]
    fn today_is_an_iso_date() {
        let t = today();
        assert_eq!(t.len(), 10);
        assert_eq!(t.chars().filter(|c| *c == '-').count(), 2);
    }

    #[test]
    fn every_route_the_router_serves_is_in_the_manifest() {
        // The manifest is data a caller reads to learn the surface. A route missing
        // from it is invisible, which is worse than one that does not exist.
        for path in [
            "/health",
            "/ready",
            "/routes",
            "/api/subscriptions",
            "/api/subscriptions/burn",
            "/api/subscriptions/:id/price",
            "/api/subscriptions/:id/state",
            "/api/import/obsidian/scan",
            "/api/import/obsidian",
            "/api/writeback",
        ] {
            assert!(
                ROUTES.iter().any(|r| r.path == path),
                "{path} is served but undeclared"
            );
        }
    }
}
