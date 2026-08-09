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

use finance::accounting::AccountingEngine;
use finance::analytics::{self, AnalyticsFilter, BudgetTarget};
use finance::config::{Config, CsvMappingProfile, InvestmentCsvMappingProfile, ObsidianConfig};
use finance::import::{self, CandidateState, CsvMapping};
use finance::investment::{self, HoldingsCoverage, InvestmentCsvMapping};
use finance::obsidian::{self, WriteBack};
use finance::store::FinanceStore;
use finance::subscription::{burn_at, cents_to_decimal, PricePoint, StateChange};
use finance::HledgerEngine;

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
        "Idempotently append a price point. Body: valid_from, amount_cents, currency, cycle, optional plan, required reason. Response: created.",
    ),
    r(
        "POST",
        "/api/subscriptions/:id/state",
        "Idempotently append a state change. Body: effective, state, note. Response: created.",
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
    r(
        "POST",
        "/api/import/csv/preview",
        "Validate and summarize a CSV export without staging candidates or returning transaction details.",
    ),
    r(
        "POST",
        "/api/import/csv",
        "Recompute and stage an unchanged CSV preview as review candidates. Raw rows are not retained.",
    ),
    r(
        "GET",
        "/api/import/csv/mappings",
        "Named CSV mapping profiles loaded from the private overlay.",
    ),
    r(
        "GET",
        "/api/import/investments/mappings",
        "Named investment CSV mapping profiles loaded from the private overlay.",
    ),
    r(
        "POST",
        "/api/import/investments/preview",
        "Reconstruct holdings from signed investment activity without persisting source rows or writing the journal.",
    ),
    r(
        "POST",
        "/api/import/investments/confirm",
        "Recompute and confirm an unchanged holdings preview for one source in the configured private collection.",
    ),
    r(
        "GET",
        "/api/import/candidates",
        "List normalized transaction candidates and their review state.",
    ),
    r(
        "POST",
        "/api/import/candidates/:id/review",
        "Confirm or reject one candidate. Confirmation requires an account and is the only journal write path.",
    ),
    r(
        "GET",
        "/api/ledger/check",
        "Check the configured journal through the accounting-engine boundary.",
    ),
    r(
        "POST",
        "/api/ledger/rebuild",
        "Atomically rebuild the disposable transaction projection from the canonical journal.",
    ),
    r(
        "GET",
        "/api/dashboard",
        "One reconciled projection for KPIs, trend, budget, transactions, filters and Sankey links.",
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
    journal: Option<std::path::PathBuf>,
    budgets: Arc<Vec<BudgetTarget>>,
    csv_mappings: Arc<Vec<CsvMappingProfile>>,
    investment_csv_mappings: Arc<Vec<InvestmentCsvMappingProfile>>,
    investment_snapshot: Option<std::path::PathBuf>,
    journal_write: Arc<std::sync::Mutex<()>>,
    projection_write: Arc<std::sync::Mutex<()>>,
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

fn no_journal() -> ApiResponse {
    response(
        StatusCode::CONFLICT,
        json!({
            "ok": false,
            "capability": "finance",
            "error": "no journal configured; set the overlay's config/finance.json or AXON_FINANCE_JOURNAL"
        }),
    )
}

#[derive(Debug, Deserialize)]
struct CsvImportRequest {
    content: String,
    mapping: CsvMapping,
    expected_preview_id: String,
}

#[derive(Debug, Deserialize)]
struct CsvPreviewRequest {
    content: String,
    mapping: CsvMapping,
}

async fn preview_csv(Json(request): Json<CsvPreviewRequest>) -> ApiResponse {
    match import::preview_csv(request.content.as_bytes(), &request.mapping) {
        Ok(preview) => response(StatusCode::OK, preview),
        Err(error) => response(
            StatusCode::BAD_REQUEST,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn import_csv(
    State(state): State<AppState>,
    Json(request): Json<CsvImportRequest>,
) -> ApiResponse {
    let prepared = match import::prepare_csv(request.content.as_bytes(), &request.mapping) {
        Ok(prepared) => prepared,
        Err(error) => {
            return response(
                StatusCode::BAD_REQUEST,
                json!({ "error": error.to_string() }),
            )
        }
    };
    if prepared.preview.preview_id != request.expected_preview_id {
        return response(
            StatusCode::CONFLICT,
            json!({ "error": "CSV preview changed before staging" }),
        );
    }
    let preview = prepared.preview;
    let candidates = prepared.candidates;
    let database_url = state.database_url.clone();
    let now = today();
    match tokio::task::spawn_blocking(move || {
        FinanceStore::open(&database_url)
            .and_then(|store| store.stage_candidates(&candidates, &now))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok((created, already_present))) => response(
            StatusCode::OK,
            json!({
                "ok": true,
                "created": created,
                "already_present": already_present,
                "duplicate_rows": preview.duplicate_rows,
                "preserved_repetitions": preview.preserved_repetitions,
                "ignored_non_transaction_rows": preview.ignored_non_transaction_rows,
            }),
        ),
        Ok(Err(error)) => failed(error),
        Err(_) => failed("task panicked".into()),
    }
}

async fn list_candidates(State(state): State<AppState>) -> ApiResponse {
    let database_url = state.database_url.clone();
    match tokio::task::spawn_blocking(move || {
        FinanceStore::open(&database_url)
            .and_then(|store| store.list_candidates())
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(candidates)) => response(StatusCode::OK, candidates),
        Ok(Err(error)) => failed(error),
        Err(_) => failed("task panicked".into()),
    }
}

async fn list_csv_mappings(State(state): State<AppState>) -> Json<Vec<CsvMappingProfile>> {
    Json(state.csv_mappings.as_ref().clone())
}

#[derive(Debug, Deserialize)]
struct InvestmentPreviewRequest {
    content: String,
    mapping: InvestmentCsvMapping,
}

async fn preview_investments(Json(request): Json<InvestmentPreviewRequest>) -> ApiResponse {
    match investment::preview_csv(request.content.as_bytes(), &request.mapping) {
        Ok(preview) => response(StatusCode::OK, preview),
        Err(error) => response(
            StatusCode::BAD_REQUEST,
            json!({ "error": error.to_string() }),
        ),
    }
}

#[derive(Debug, Deserialize)]
struct InvestmentConfirmRequest {
    content: String,
    mapping: InvestmentCsvMapping,
    source_key: String,
    expected_snapshot_id: String,
    #[serde(default)]
    coverage: HoldingsCoverage,
}

async fn confirm_investments(
    State(state): State<AppState>,
    Json(request): Json<InvestmentConfirmRequest>,
) -> ApiResponse {
    let Some(path) = state.investment_snapshot.clone() else {
        return response(
            StatusCode::CONFLICT,
            json!({ "error": "no private holdings snapshot is configured" }),
        );
    };
    let database_url = state.database_url.clone();
    let projection_write = state.projection_write.clone();
    let reviewed_at = today();
    match tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let preview = investment::preview_csv(request.content.as_bytes(), &request.mapping)
            .map_err(|error| error.to_string())?;
        if preview.snapshot_id != request.expected_snapshot_id {
            return Err("investment preview changed before confirmation".into());
        }
        let snapshot = investment::reviewed_snapshot(
            &preview,
            &request.mapping,
            &reviewed_at,
            request.coverage,
        )
        .map_err(|error| error.to_string())?;
        let _write_guard = projection_write
            .lock()
            .map_err(|_| "finance projection writer lock is unavailable".to_string())?;
        let created = investment::write_reviewed_snapshot(&path, &request.source_key, &snapshot)
            .map_err(|error| error.to_string())?;
        let canonical = investment::read_reviewed_snapshot(&path)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "holdings snapshot disappeared after confirmation".to_string())?;
        FinanceStore::open(&database_url)
            .and_then(|store| store.replace_holding_projection(&canonical))
            .map_err(|error| error.to_string())?;
        Ok(json!({
            "ok": true,
            "created": created,
            "snapshot": canonical,
        }))
    })
    .await
    {
        Ok(Ok(body)) => response(StatusCode::OK, body),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(_) => failed("task panicked".into()),
    }
}

async fn list_investment_mappings(
    State(state): State<AppState>,
) -> Json<Vec<InvestmentCsvMappingProfile>> {
    Json(state.investment_csv_mappings.as_ref().clone())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ReviewDecision {
    Confirm,
    Reject,
}

#[derive(Debug, Deserialize)]
struct ReviewRequest {
    decision: ReviewDecision,
    account: Option<String>,
}

async fn review_candidate(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<ReviewRequest>,
) -> ApiResponse {
    let Some(journal) = state.journal.clone() else {
        return no_journal();
    };
    let database_url = state.database_url.clone();
    let budgets = state.budgets.clone();
    let investment_snapshot = state.investment_snapshot.clone();
    let journal_write = state.journal_write.clone();
    let projection_write = state.projection_write.clone();
    let now = today();
    match tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let store = FinanceStore::open(&database_url).map_err(|error| error.to_string())?;
        let candidate = store
            .candidate(&id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "candidate not found".to_string())?;
        match request.decision {
            ReviewDecision::Reject => {
                if candidate.state == CandidateState::Confirmed {
                    return Err(
                        "a confirmed candidate cannot be rejected because its posting is canonical"
                            .into(),
                    );
                }
                store
                    .review_candidate(
                        &id,
                        CandidateState::Rejected,
                        &candidate.proposed_account,
                        &now,
                    )
                    .map_err(|error| error.to_string())?;
                Ok(json!({ "ok": true, "id": id, "state": "rejected", "journal_written": false }))
            }
            ReviewDecision::Confirm => {
                if candidate.state == CandidateState::Rejected {
                    return Err("a rejected candidate must be restaged before confirmation".into());
                }
                let account = if candidate.state == CandidateState::Confirmed {
                    if request
                        .account
                        .as_deref()
                        .is_some_and(|account| account != candidate.proposed_account)
                    {
                        return Err(
                            "a confirmed candidate cannot be moved to another account".into()
                        );
                    }
                    candidate.proposed_account.as_str()
                } else {
                    request
                        .account
                        .as_deref()
                        .ok_or_else(|| "account is required for confirmation".to_string())?
                };
                import::validate_account(account).map_err(|error| error.to_string())?;
                let _write_guard = journal_write
                    .lock()
                    .map_err(|_| "journal writer lock is unavailable".to_string())?;
                let entry = import::render_journal_entry(&candidate, account)
                    .map_err(|error| error.to_string())?;
                validate_journal_append(&journal, &entry)?;
                let journal_written = import::append_confirmed(&journal, &candidate, account)
                    .map_err(|error| error.to_string())?;
                store
                    .review_candidate(&id, CandidateState::Confirmed, account, &now)
                    .map_err(|error| error.to_string())?;
                let _projection_guard = projection_write
                    .lock()
                    .map_err(|_| "finance projection writer lock is unavailable".to_string())?;
                rebuild_projection(
                    &database_url,
                    &journal,
                    &budgets,
                    investment_snapshot.as_deref(),
                )?;
                Ok(json!({
                    "ok": true,
                    "id": id,
                    "state": "confirmed",
                    "journal_written": journal_written,
                }))
            }
        }
    })
    .await
    {
        Ok(Ok(body)) => response(StatusCode::OK, body),
        Ok(Err(error)) if error == "candidate not found" => {
            response(StatusCode::NOT_FOUND, json!({ "error": error }))
        }
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(_) => failed("task panicked".into()),
    }
}

fn validate_journal_append(journal: &std::path::Path, entry: &str) -> Result<(), String> {
    let existing = std::fs::read_to_string(journal)
        .map_err(|error| format!("journal could not be read: {error}"))?;
    let parent = journal
        .parent()
        .ok_or_else(|| "journal has no parent directory".to_string())?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let name = format!(".axon-finance-check-{}-{nonce}", std::process::id());
    let temporary = parent.join(name);
    let result = (|| {
        std::fs::write(&temporary, format!("{existing}{entry}"))
            .map_err(|error| format!("journal validation file could not be written: {error}"))?;
        HledgerEngine::new(&temporary)
            .check()
            .map_err(|error| error.to_string())
    })();
    let _ = std::fs::remove_file(&temporary);
    result
}

async fn check_ledger(State(state): State<AppState>) -> ApiResponse {
    let Some(journal) = state.journal.clone() else {
        return no_journal();
    };
    match tokio::task::spawn_blocking(move || HledgerEngine::new(journal).check()).await {
        Ok(Ok(())) => response(StatusCode::OK, json!({ "ok": true })),
        Ok(Err(error)) => response(
            StatusCode::UNPROCESSABLE_ENTITY,
            json!({ "ok": false, "error": error.to_string() }),
        ),
        Err(_) => failed("task panicked".into()),
    }
}

async fn rebuild_ledger(State(state): State<AppState>) -> ApiResponse {
    let Some(journal) = state.journal.clone() else {
        return no_journal();
    };
    let database_url = state.database_url.clone();
    let budgets = state.budgets.clone();
    let investment_snapshot = state.investment_snapshot.clone();
    let projection_write = state.projection_write.clone();
    match tokio::task::spawn_blocking(move || {
        let _projection_guard = projection_write
            .lock()
            .map_err(|_| "finance projection writer lock is unavailable".to_string())?;
        rebuild_projection(
            &database_url,
            &journal,
            &budgets,
            investment_snapshot.as_deref(),
        )
    })
    .await
    {
        Ok(Ok(count)) => response(StatusCode::OK, json!({ "ok": true, "rows": count })),
        Ok(Err(error)) => response(
            StatusCode::UNPROCESSABLE_ENTITY,
            json!({ "ok": false, "error": error }),
        ),
        Err(_) => failed("task panicked".into()),
    }
}

fn rebuild_projection(
    database_url: &str,
    journal: &std::path::Path,
    budgets: &[BudgetTarget],
    investment_snapshot: Option<&std::path::Path>,
) -> Result<usize, String> {
    let engine = HledgerEngine::new(journal);
    engine.check().map_err(|error| error.to_string())?;
    let transactions = engine.transactions().map_err(|error| error.to_string())?;
    let mut currencies = std::collections::BTreeSet::from(["EUR".to_string()]);
    currencies.extend(budgets.iter().map(|budget| budget.currency.clone()));
    let rows: Vec<_> = currencies
        .iter()
        .flat_map(|currency| analytics::project(&transactions, currency))
        .collect();
    let store = FinanceStore::open(database_url).map_err(|error| error.to_string())?;
    store
        .replace_transaction_projection(&rows)
        .map_err(|error| error.to_string())?;
    match investment_snapshot {
        Some(path) => {
            match investment::read_reviewed_snapshot(path).map_err(|error| error.to_string())? {
                Some(snapshot) => store
                    .replace_holding_projection(&snapshot)
                    .map_err(|error| error.to_string())?,
                None => store
                    .clear_holding_projection()
                    .map_err(|error| error.to_string())?,
            }
        }
        None => store
            .clear_holding_projection()
            .map_err(|error| error.to_string())?,
    }
    Ok(rows.len())
}

async fn dashboard_projection(
    State(state): State<AppState>,
    Query(filter): Query<AnalyticsFilter>,
) -> ApiResponse {
    let database_url = state.database_url.clone();
    let budgets = state.budgets.clone();
    let projection_write = state.projection_write.clone();
    match tokio::task::spawn_blocking(move || {
        let _projection_guard = projection_write
            .lock()
            .map_err(|_| "finance projection writer lock is unavailable".to_string())?;
        let store = FinanceStore::open(&database_url).map_err(|error| error.to_string())?;
        let rows = store
            .transaction_projection()
            .map_err(|error| error.to_string())?;
        let mut view = analytics::dashboard(&rows, &budgets, &filter);
        let investment = store
            .holding_projection()
            .map_err(|error| error.to_string())?;
        view.portfolio_values = investment
            .as_ref()
            .map(investment::portfolio_valuations)
            .transpose()
            .map_err(|error| error.to_string())?
            .unwrap_or_default();
        view.investment = investment;
        Ok::<_, String>(view)
    })
    .await
    {
        Ok(Ok(view)) => response(StatusCode::OK, view),
        Ok(Err(error)) => failed(error),
        Err(_) => failed("task panicked".into()),
    }
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

fn valid_iso_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return false;
    }
    if bytes
        .iter()
        .enumerate()
        .any(|(index, byte)| !matches!(index, 4 | 7) && !byte.is_ascii_digit())
    {
        return false;
    }

    let Ok(year) = value[0..4].parse::<u32>() else {
        return false;
    };
    let Ok(month) = value[5..7].parse::<u32>() else {
        return false;
    };
    let Ok(day) = value[8..10].parse::<u32>() else {
        return false;
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let last_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=last_day).contains(&day)
}

fn validate_price(price: &PricePoint) -> Result<(), String> {
    if !valid_iso_date(&price.valid_from) {
        return Err("valid_from must be a real date in YYYY-MM-DD form".into());
    }
    if price.amount_cents < 0 {
        return Err("amount_cents must not be negative".into());
    }
    if price.currency.len() != 3 || !price.currency.bytes().all(|byte| byte.is_ascii_uppercase()) {
        return Err("currency must be a three-letter uppercase code".into());
    }
    if price.reason.trim().is_empty() {
        return Err("reason is required for an append-only price point".into());
    }
    if price
        .plan
        .as_ref()
        .is_some_and(|plan| plan.trim().is_empty())
    {
        return Err("plan must be omitted instead of blank".into());
    }
    Ok(())
}

/// Burn on a date, computed from each subscription's series.
///
/// There is no stored total to return, by design: a cached figure is a second
/// source of truth that goes stale the moment a price point lands.
async fn burn(State(state): State<AppState>, Query(query): Query<AtQuery>) -> ApiResponse {
    let database_url = state.database_url.clone();
    let at = query.at.unwrap_or_else(today);
    if !valid_iso_date(&at) {
        return response(
            StatusCode::BAD_REQUEST,
            json!({ "error": "at must be a real date in YYYY-MM-DD form" }),
        );
    }
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
    if let Err(error) = validate_price(&price) {
        return response(StatusCode::BAD_REQUEST, json!({ "error": error }));
    }
    let database_url = state.database_url.clone();
    let now = today();
    match tokio::task::spawn_blocking(move || {
        FinanceStore::open(&database_url)
            .and_then(|store| {
                store
                    .append_price(&id, &price, &now)
                    .map(|created| (id.clone(), created))
            })
            .map_err(|e| e.to_string())
    })
    .await
    {
        Ok(Ok((id, created))) => response(
            if created {
                StatusCode::CREATED
            } else {
                StatusCode::OK
            },
            json!({ "ok": true, "id": id, "created": created }),
        ),
        Ok(Err(e)) => failed(e),
        Err(_) => failed("task panicked".into()),
    }
}

async fn append_state(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(change): Json<StateChange>,
) -> ApiResponse {
    if !valid_iso_date(&change.effective) {
        return response(
            StatusCode::BAD_REQUEST,
            json!({ "error": "effective must be a real date in YYYY-MM-DD form" }),
        );
    }
    let database_url = state.database_url.clone();
    let now = today();
    match tokio::task::spawn_blocking(move || {
        FinanceStore::open(&database_url)
            .and_then(|store| {
                store
                    .append_state(&id, &change, &now)
                    .map(|created| (id.clone(), created))
            })
            .map_err(|e| e.to_string())
    })
    .await
    {
        Ok(Ok((id, created))) => response(
            if created {
                StatusCode::CREATED
            } else {
                StatusCode::OK
            },
            json!({ "ok": true, "id": id, "created": created }),
        ),
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
        journal: config.journal,
        budgets: Arc::new(config.budgets),
        csv_mappings: Arc::new(config.csv_mappings),
        investment_csv_mappings: Arc::new(config.investment_csv_mappings),
        investment_snapshot: config.investment_snapshot,
        journal_write: Arc::new(std::sync::Mutex::new(())),
        projection_write: Arc::new(std::sync::Mutex::new(())),
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
        .route("/api/import/csv/preview", post(preview_csv))
        .route("/api/import/csv", post(import_csv))
        .route("/api/import/csv/mappings", get(list_csv_mappings))
        .route(
            "/api/import/investments/mappings",
            get(list_investment_mappings),
        )
        .route("/api/import/investments/preview", post(preview_investments))
        .route("/api/import/investments/confirm", post(confirm_investments))
        .route("/api/import/candidates", get(list_candidates))
        .route("/api/import/candidates/:id/review", post(review_candidate))
        .route("/api/ledger/check", get(check_ledger))
        .route("/api/ledger/rebuild", post(rebuild_ledger))
        .route("/api/dashboard", get(dashboard_projection))
        .layer(CorsLayer::permissive())
        .with_state(state);
    axon_server::serve_local("finance-server", config.port, app).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use finance::import::{AmountSign, CsvDateFormat, CsvRowPolicy};

    fn synthetic_csv_mapping() -> CsvMapping {
        CsvMapping {
            delimiter: ';',
            decimal_separator: ',',
            date_column: "Date".into(),
            amount_column: "Amount".into(),
            description_column: "Description".into(),
            reference_column: Some("Reference".into()),
            currency_column: Some("Currency".into()),
            default_currency: "EUR".into(),
            source_account: "assets:bank:checking".into(),
            amount_sign: AmountSign::AsProvided,
            date_formats: vec![
                CsvDateFormat::IsoYearMonthDay,
                CsvDateFormat::DayMonthYearDots,
            ],
            row_policy: CsvRowPolicy::Strict,
        }
    }

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
    fn dates_are_calendar_dates_not_only_ten_characters() {
        assert!(valid_iso_date("2024-02-29"));
        assert!(valid_iso_date("2026-08-08"));
        assert!(!valid_iso_date("2026-02-29"));
        assert!(!valid_iso_date("2026-13-01"));
        assert!(!valid_iso_date("202610-01-08"));
    }

    #[test]
    fn a_price_append_needs_an_auditable_reason() {
        let price = PricePoint {
            valid_from: "2026-10-01".into(),
            amount_cents: 10_000,
            currency: "EUR".into(),
            cycle: finance::BillingCycle::Monthly,
            plan: Some("Max".into()),
            reason: String::new(),
        };
        assert_eq!(
            validate_price(&price),
            Err("reason is required for an append-only price point".into())
        );
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
            "/api/import/csv/preview",
            "/api/import/csv",
            "/api/import/csv/mappings",
            "/api/import/investments/mappings",
            "/api/import/investments/preview",
            "/api/import/investments/confirm",
        ] {
            assert!(
                ROUTES.iter().any(|r| r.path == path),
                "{path} is served but undeclared"
            );
        }
    }

    #[tokio::test]
    async fn transaction_csv_preview_is_summary_only_and_staging_requires_its_identity() {
        let content = "Date;Amount;Description;Reference;Currency\n2026-08-09;-12,34;Synthetic service;one;EUR\n";
        let (status, Json(body)) = preview_csv(Json(CsvPreviewRequest {
            content: content.into(),
            mapping: synthetic_csv_mapping(),
        }))
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["candidate_count"], 1);
        assert_eq!(body["preserved_repetitions"], 0);
        assert!(body.get("candidates").is_none());
        assert!(body.get("description").is_none());

        let state = AppState {
            database_url: Arc::new(String::new()),
            obsidian: None,
            journal: None,
            budgets: Arc::new(Vec::new()),
            csv_mappings: Arc::new(Vec::new()),
            investment_csv_mappings: Arc::new(Vec::new()),
            investment_snapshot: None,
            journal_write: Arc::new(std::sync::Mutex::new(())),
            projection_write: Arc::new(std::sync::Mutex::new(())),
        };
        let (status, Json(body)) = import_csv(
            State(state),
            Json(CsvImportRequest {
                content: content.into(),
                mapping: synthetic_csv_mapping(),
                expected_preview_id: "changed-preview".into(),
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body["error"], "CSV preview changed before staging");
    }

    #[tokio::test]
    async fn csv_mapping_endpoint_returns_private_profiles_without_mutating_them() {
        let profile = CsvMappingProfile {
            label: "Synthetic semicolon export".into(),
            mapping: synthetic_csv_mapping(),
        };
        let state = AppState {
            database_url: Arc::new(String::new()),
            obsidian: None,
            journal: None,
            budgets: Arc::new(Vec::new()),
            csv_mappings: Arc::new(vec![profile.clone()]),
            investment_csv_mappings: Arc::new(Vec::new()),
            investment_snapshot: None,
            journal_write: Arc::new(std::sync::Mutex::new(())),
            projection_write: Arc::new(std::sync::Mutex::new(())),
        };

        let Json(returned) = list_csv_mappings(State(state)).await;
        assert_eq!(returned, vec![profile]);
    }

    #[tokio::test]
    async fn investment_mapping_endpoint_returns_private_profiles_without_mutating_them() {
        let profile = InvestmentCsvMappingProfile {
            source_key: "synthetic-broker".into(),
            label: "Synthetic activity export".into(),
            coverage: HoldingsCoverage::Partial,
            mapping: InvestmentCsvMapping {
                delimiter: ';',
                decimal_separator: ',',
                date_column: "Date".into(),
                instrument_column: "Instrument".into(),
                quantity_column: "Quantity".into(),
                activity_type_column: None,
                position_activity_values: Vec::new(),
                non_position_activity_values: Vec::new(),
                reference_column: Some("Reference".into()),
                price_column: Some("Price".into()),
                currency_column: Some("Currency".into()),
                default_currency: "EUR".into(),
                instrument_aliases: Default::default(),
            },
        };
        let state = AppState {
            database_url: Arc::new(String::new()),
            obsidian: None,
            journal: None,
            budgets: Arc::new(Vec::new()),
            csv_mappings: Arc::new(Vec::new()),
            investment_csv_mappings: Arc::new(vec![profile.clone()]),
            investment_snapshot: None,
            journal_write: Arc::new(std::sync::Mutex::new(())),
            projection_write: Arc::new(std::sync::Mutex::new(())),
        };

        let Json(returned) = list_investment_mappings(State(state)).await;
        assert_eq!(returned, vec![profile]);
    }

    #[tokio::test]
    async fn investment_confirmation_requires_a_private_snapshot_path() {
        let state = AppState {
            database_url: Arc::new(String::new()),
            obsidian: None,
            journal: None,
            budgets: Arc::new(Vec::new()),
            csv_mappings: Arc::new(Vec::new()),
            investment_csv_mappings: Arc::new(Vec::new()),
            investment_snapshot: None,
            journal_write: Arc::new(std::sync::Mutex::new(())),
            projection_write: Arc::new(std::sync::Mutex::new(())),
        };
        let request = InvestmentConfirmRequest {
            content: String::new(),
            mapping: InvestmentCsvMapping {
                delimiter: ';',
                decimal_separator: ',',
                date_column: "Date".into(),
                instrument_column: "Instrument".into(),
                quantity_column: "Quantity".into(),
                activity_type_column: None,
                position_activity_values: Vec::new(),
                non_position_activity_values: Vec::new(),
                reference_column: None,
                price_column: None,
                currency_column: None,
                default_currency: "EUR".into(),
                instrument_aliases: Default::default(),
            },
            source_key: "synthetic-broker".into(),
            expected_snapshot_id: String::new(),
            coverage: HoldingsCoverage::Complete,
        };

        let (status, _) = confirm_investments(State(state), Json(request)).await;
        assert_eq!(status, StatusCode::CONFLICT);
    }
}
