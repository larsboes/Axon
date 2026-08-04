//! `comms-server` — HTTP surface for the general Feed.
//!
//! Feed persistence, TELOS relevance, explicit Vault-link discovery and reader
//! payloads live here. Scouting remains a separate opportunity engine. Network
//! fetches and embedding calls run in spawn_blocking; the server binds only to
//! loopback because ingest is allowed to fetch external URLs.

use std::collections::HashSet;

use axum::http::{HeaderName, HeaderValue, Method};
use axum::{
    extract::{DefaultBodyLimit, Path, Query},
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tower_http::cors::CorsLayer;

use comms::config::Config;
use comms::evaluation::{self, EvaluationFactor, FeedEvaluation};
use comms::media;
use comms::provenance::StageProvenance;
use comms::quality;
use comms::relevance::{self, RelevanceMatch};
use comms::sources;
use comms::store::{FeedItem, FeedOrigin, FeedRun, OriginSummary, Store, TriageItem};
use comms::travel;
use comms::vault_links;

/// Constant-time comparison for the shared secret. Avoids timing side-channels
/// that could leak the secret length or prefix.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

/// Axum middleware layer that rejects requests without a valid shared secret.
/// Reads `Authorization: Bearer <token>` or `X-Axon-Token: <token>` and
/// compares constant-time against the configured value. Health and read-only
/// routes bypass this entirely — only mutating routes carry the layer.
async fn require_auth(
    headers: axum::http::HeaderMap,
    axum::extract::State(secret): axum::extract::State<Option<String>>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let expected = match &secret {
        Some(s) if !s.is_empty() => s.as_str(),
        _ => {
            // No secret configured: block mutating routes with a helpful message.
            return (
                StatusCode::FORBIDDEN,
                Json(json!({
                    "error": "api_secret_file is not configured — mutating routes are disabled. See comms.config.example.json."
                })),
            ).into_response();
        }
    };

    // Try Authorization: Bearer first, then X-Axon-Token.
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .or_else(|| headers.get("x-axon-token").and_then(|v| v.to_str().ok()));

    match token {
        Some(t) if constant_time_eq(t.as_bytes(), expected.as_bytes()) => next.run(request).await,
        _ => (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "invalid or missing authentication token" })),
        )
            .into_response(),
    }
}

/// Convert a (StatusCode, Json<Value>) into an axum Response for the auth
/// middleware's error paths.
use axum::response::IntoResponse;

#[derive(Debug, Deserialize)]
struct FeedParams {
    stream: Option<String>,
    source_id: Option<String>,
    days: Option<i32>,
    include_dismissed: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct QualityParams {
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct QualityRefreshBody {
    days: Option<i32>,
}

#[derive(Debug, Serialize)]
struct RelevanceOut {
    profile_key: String,
    profile_label: String,
    score: f64,
    rationale: String,
    mode: String,
    profile_revision: String,
}

impl From<RelevanceMatch> for RelevanceOut {
    fn from(relevance: RelevanceMatch) -> Self {
        Self {
            profile_key: relevance.profile_key,
            profile_label: relevance.profile_label,
            score: relevance.score,
            rationale: relevance.rationale,
            mode: relevance.mode,
            profile_revision: relevance.profile_revision,
        }
    }
}

#[derive(Debug, Serialize)]
struct EvaluationFactorOut {
    key: String,
    label: String,
    score: f64,
    weight: f64,
    rationale: String,
    context: Option<evaluation::EvaluationFactorContext>,
}

impl From<EvaluationFactor> for EvaluationFactorOut {
    fn from(factor: EvaluationFactor) -> Self {
        Self {
            key: factor.key,
            label: factor.label,
            score: factor.score,
            weight: factor.weight,
            rationale: factor.rationale,
            context: factor.context,
        }
    }
}

#[derive(Debug, Serialize)]
struct EvaluationOut {
    overall_score: f64,
    explanation: String,
    mode: String,
    item_revision: String,
    context_revision: String,
    evaluator_revision: String,
    evaluated_at: String,
    factors: Vec<EvaluationFactorOut>,
}

impl From<FeedEvaluation> for EvaluationOut {
    fn from(evaluation: FeedEvaluation) -> Self {
        Self {
            overall_score: evaluation.overall_score,
            explanation: evaluation.explanation,
            mode: evaluation.mode,
            item_revision: evaluation.item_revision,
            context_revision: evaluation.context_revision,
            evaluator_revision: evaluation.evaluator_revision,
            evaluated_at: evaluation.evaluated_at,
            factors: evaluation
                .factors
                .into_iter()
                .map(EvaluationFactorOut::from)
                .collect(),
        }
    }
}

#[derive(Debug, Serialize)]
struct OriginOut {
    source_id: String,
    source_ref: String,
    label: Option<String>,
}

impl From<FeedOrigin> for OriginOut {
    fn from(origin: FeedOrigin) -> Self {
        Self {
            source_id: origin.source_id,
            source_ref: origin.source_ref,
            label: origin.label,
        }
    }
}

#[derive(Debug, Serialize)]
struct StageProvenanceOut {
    stage: String,
    tier: String,
    revision: String,
    completed_at: String,
}

impl From<StageProvenance> for StageProvenanceOut {
    fn from(value: StageProvenance) -> Self {
        Self {
            stage: value.stage,
            tier: value.tier,
            revision: value.revision,
            completed_at: value.completed_at,
        }
    }
}

/// List payload omits the transcript and carries only the strongest TELOS
/// match. The reader endpoint returns every stored match.
#[derive(Debug, Serialize)]
struct FeedListItem {
    id: String,
    stream: String,
    kind: String,
    title: Option<String>,
    url: String,
    author: Option<String>,
    summary: Option<String>,
    day: String,
    created_at: String,
    status: String,
    relevance: Option<RelevanceOut>,
    evaluation: Option<EvaluationOut>,
}

impl FeedListItem {
    fn from_store(
        item: FeedItem,
        relevance: Option<RelevanceMatch>,
        evaluation: Option<FeedEvaluation>,
    ) -> Self {
        Self {
            id: item.id,
            stream: item.stream,
            kind: item.kind,
            title: item.title,
            url: item.url,
            author: item.author,
            summary: item.summary,
            day: item.day,
            created_at: item.created_at,
            status: item.status,
            relevance: relevance.map(RelevanceOut::from),
            evaluation: evaluation.map(EvaluationOut::from),
        }
    }
}

#[derive(Debug, Serialize)]
struct FeedFullItem {
    id: String,
    stream: String,
    kind: String,
    title: Option<String>,
    url: String,
    author: Option<String>,
    summary: Option<String>,
    transcript: Option<String>,
    day: String,
    created_at: String,
    status: String,
    content_status: String,
    /// Which client handed this content over; null when the server fetched it.
    captured_via: Option<String>,
    relevance: Vec<RelevanceOut>,
    evaluation: Option<EvaluationOut>,
    processing: Vec<StageProvenanceOut>,
    origins: Vec<OriginOut>,
}

impl FeedFullItem {
    fn from_store(
        item: FeedItem,
        relevance: Vec<RelevanceMatch>,
        evaluation: Option<FeedEvaluation>,
        processing: Vec<StageProvenance>,
        origins: Vec<FeedOrigin>,
    ) -> Self {
        Self {
            id: item.id,
            stream: item.stream,
            kind: item.kind,
            title: item.title,
            url: item.url,
            author: item.author,
            summary: item.summary,
            transcript: item.transcript,
            day: item.day,
            created_at: item.created_at,
            status: item.status,
            content_status: item.content_status,
            captured_via: item.captured_via,
            relevance: relevance.into_iter().map(RelevanceOut::from).collect(),
            evaluation: evaluation.map(EvaluationOut::from),
            processing: processing.into_iter().map(StageProvenanceOut::from).collect(),
            origins: origins.into_iter().map(OriginOut::from).collect(),
        }
    }
}

#[derive(Debug, Serialize)]
struct TriageOut {
    id: String,
    from_addr: Option<String>,
    subject: Option<String>,
    snippet: Option<String>,
    internal_date: Option<String>,
    stream: String,
    rationale: String,
    status: String,
    first_seen: String,
    last_seen: String,
}

impl From<TriageItem> for TriageOut {
    fn from(item: TriageItem) -> Self {
        Self {
            id: item.id,
            from_addr: item.from_addr,
            subject: item.subject,
            snippet: item.snippet,
            internal_date: item.internal_date_text,
            stream: item.stream,
            rationale: item.rationale,
            status: item.status,
            first_seen: item.first_seen,
            last_seen: item.last_seen,
        }
    }
}

async fn health_handler() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "service": "comms",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

async fn feed_handler(Query(params): Query<FeedParams>) -> Json<Value> {
    let result = tokio::task::spawn_blocking(move || -> Result<Vec<FeedListItem>, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_url).map_err(|error| error.to_string())?;
        let items = store
            .list_feed(
                params.stream.as_deref(),
                params.source_id.as_deref(),
                params.days.unwrap_or(7).clamp(1, 3650),
                params.include_dismissed.unwrap_or(false),
            )
            .map_err(|error| error.to_string())?;
        items
            .into_iter()
            .map(|item| {
                let relevance = store
                    .feed_relevance(&item.id)
                    .map_err(|error| error.to_string())?
                    .into_iter()
                    .next();
                let evaluation = store
                    .feed_evaluation(&item.id)
                    .map_err(|error| error.to_string())?;
                Ok(FeedListItem::from_store(item, relevance, evaluation))
            })
            .collect()
    })
    .await;

    match result {
        Ok(Ok(items)) => Json(json!(items)),
        _ => Json(json!({ "error": "feed query failed" })),
    }
}

async fn feed_origins_handler() -> Json<Value> {
    let result = tokio::task::spawn_blocking(move || -> Result<Vec<OriginSummary>, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_url).map_err(|error| error.to_string())?;
        store
            .list_origin_summaries()
            .map_err(|error| error.to_string())
    })
    .await;

    match result {
        Ok(Ok(summaries)) => Json(json!(summaries)),
        _ => Json(json!({ "error": "feed origins query failed" })),
    }
}

/// Which items arrived together, so the reader can collapse a collector run
/// into one row instead of showing a dozen unrelated-looking ones (#84).
/// Derived per request from `feed_origins`; nothing about grouping is stored.
async fn feed_runs_handler(Query(params): Query<FeedParams>) -> Json<Value> {
    let result = tokio::task::spawn_blocking(move || -> Result<Vec<FeedRun>, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_url).map_err(|error| error.to_string())?;
        store
            .list_feed_runs(params.days.unwrap_or(7))
            .map_err(|error| error.to_string())
    })
    .await;

    match result {
        Ok(Ok(runs)) => Json(json!(runs)),
        _ => Json(json!({ "error": "feed runs query failed" })),
    }
}

async fn quality_queue_handler(Query(params): Query<QualityParams>) -> (StatusCode, Json<Value>) {
    let limit = params.limit.unwrap_or(500).clamp(1, 2_000);
    let result = tokio::task::spawn_blocking(move || {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_url).map_err(|error| error.to_string())?;
        store
            .feed_quality_review_queue(limit)
            .map_err(|error| error.to_string())
    })
    .await;

    match result {
        Ok(Ok(flags)) => (StatusCode::OK, Json(json!(flags))),
        Ok(Err(error)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error })),
        ),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        ),
    }
}

async fn quality_refresh_handler(
    Json(body): Json<QualityRefreshBody>,
) -> (StatusCode, Json<Value>) {
    let days = body.days.unwrap_or(3650);
    if !(1..=3650).contains(&days) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "days must be between 1 and 3650" })),
        );
    }

    let result = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_url).map_err(|error| error.to_string())?;
        let items = store
            .feed_for_relevance(days, 500)
            .map_err(|error| error.to_string())?;
        let reviewed = items.len();
        let mut flagged_items = 0usize;
        let mut flag_count = 0usize;

        for item in items {
            let raw_content = store
                .get_raw_content(&item.id)
                .map_err(|error| error.to_string())?;
            let stages = store
                .feed_stage_results(&item.id)
                .map_err(|error| error.to_string())?;
            let has_ranking = store
                .feed_evaluation(&item.id)
                .map_err(|error| error.to_string())?
                .is_some();
            let flags = quality::derive(
                &item,
                raw_content.as_deref(),
                &stages,
                has_ranking,
                &cfg.quality_flags,
            );
            if !flags.is_empty() {
                flagged_items += 1;
                flag_count += flags.len();
            }
            store
                .replace_feed_quality_flags(&item.id, &flags)
                .map_err(|error| error.to_string())?;
        }

        Ok(json!({
            "reviewed": reviewed,
            "flagged_items": flagged_items,
            "flag_count": flag_count,
            "bounded_to": 500,
            "days": days,
            "provider_calls": 0,
        }))
    })
    .await;

    match result {
        Ok(Ok(value)) => (StatusCode::OK, Json(value)),
        Ok(Err(error)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error })),
        ),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        ),
    }
}

fn full_item(store: &Store, item: FeedItem) -> Result<FeedFullItem, String> {
    let relevance = store
        .feed_relevance(&item.id)
        .map_err(|error| error.to_string())?;
    let origins = store
        .feed_origins(&item.id)
        .map_err(|error| error.to_string())?;
    let evaluation = store
        .feed_evaluation(&item.id)
        .map_err(|error| error.to_string())?;
    let processing = store
        .feed_stage_results(&item.id)
        .map_err(|error| error.to_string())?;
    Ok(FeedFullItem::from_store(
        item, relevance, evaluation, processing, origins,
    ))
}

async fn feed_item_handler(Path(id): Path<String>) -> (StatusCode, Json<Value>) {
    let result = tokio::task::spawn_blocking(move || -> Result<Option<FeedFullItem>, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_url).map_err(|error| error.to_string())?;
        store
            .get_feed(&id)
            .map_err(|error| error.to_string())?
            .map(|item| full_item(&store, item))
            .transpose()
    })
    .await;

    match result {
        Ok(Ok(Some(item))) => (StatusCode::OK, Json(json!(item))),
        Ok(Ok(None)) => (StatusCode::NOT_FOUND, Json(json!({ "error": "not found" }))),
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "feed query failed" })),
        ),
    }
}

#[derive(Debug, Deserialize)]
struct StatusBody {
    status: String,
}

async fn feed_status_handler(
    Path(id): Path<String>,
    Json(body): Json<StatusBody>,
) -> (StatusCode, Json<Value>) {
    let result = tokio::task::spawn_blocking(move || -> Result<bool, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_url).map_err(|error| error.to_string())?;
        store
            .set_feed_status(&id, &body.status)
            .map_err(|error| error.to_string())
    })
    .await;

    match result {
        Ok(Ok(true)) => (StatusCode::OK, Json(json!({ "ok": true }))),
        Ok(Ok(false)) => (StatusCode::NOT_FOUND, Json(json!({ "error": "not found" }))),
        Ok(Err(error)) => (StatusCode::BAD_REQUEST, Json(json!({ "error": error }))),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        ),
    }
}

#[derive(Debug, Deserialize)]
struct IngestBody {
    url: String,
    content: Option<String>,
    title: Option<String>,
    author: Option<String>,
    /// Who is handing the content over — `axon-clip`, a CLI, a future share
    /// sheet. Recorded as the item's capture provenance; absent means the
    /// server fetched the page itself (#81).
    client: Option<String>,
}

fn enrich_many_in_background(ids: Vec<String>) {
    tokio::task::spawn_blocking(move || {
        let cfg = Config::load();
        let store = match Store::open(&cfg.database_url) {
            Ok(store) => store,
            Err(error) => {
                eprintln!("ingest: enrichment skipped, store unavailable: {error}");
                return;
            }
        };
        for id in &ids {
            if let Err(error) = media::summarize_item(&store, &cfg, id) {
                eprintln!("ingest: summarize failed for {id}: {error}");
            }
        }
        let mut items = ids
            .iter()
            .filter_map(|id| store.get_feed(id).ok().flatten())
            .collect::<Vec<_>>();
        let profiles = relevance::load_profiles(&cfg.relevance);
        let embedding_role = cfg.embedding_role();
        let embedding_producer = embedding_role.as_ref().map(|role| role.cache_key());
        let reranking_role = cfg.reranking_role();
        let reranking_producer = reranking_role.as_ref().map(|role| role.cache_key());
        let travel_context = travel::load(&store, &cfg.travel_context);
        let context_revision = evaluation::context_revision(
            &profiles,
            embedding_producer.as_deref(),
            reranking_producer.as_deref(),
            &travel_context.revision,
        );
        items.retain(|item| {
            let item_revision = evaluation::item_revision(item);
            let stored = store.feed_evaluation(&item.id).ok().flatten();
            !evaluation::is_current(stored.as_ref(), &item_revision, &context_revision)
        });
        if items.is_empty() {
            return;
        }
        let scored = relevance::score_items(
            &items,
            &profiles,
            embedding_role.as_ref(),
            reranking_role.as_ref(),
        );
        for (item, result) in items.iter().zip(scored) {
            if let Err(error) = store.replace_feed_relevance(&item.id, &result.matches) {
                eprintln!("ingest: relevance failed for {}: {error}", item.id);
                continue;
            }
            let evaluated = evaluation::evaluate(
                item,
                result.matches.first(),
                &context_revision,
                &travel_context.contexts,
            );
            if let Err(error) = store.replace_feed_evaluation(&evaluated) {
                eprintln!("ingest: evaluation failed for {}: {error}", item.id);
            }
        }
    });
}

fn enrich_in_background(id: String) {
    enrich_many_in_background(vec![id]);
}

/// Store first, then summarize and score behind the response.
async fn ingest_handler(Json(body): Json<IngestBody>) -> (StatusCode, Json<Value>) {
    let url = body.url.trim().to_string();
    if url.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "url is required" })),
        );
    }
    let content = body.content;
    let title = body.title;
    let author = body.author;
    let client = body.client;

    let stored = tokio::task::spawn_blocking(move || -> Result<FeedFullItem, String> {
        let cfg = Config::load();
        let item = media::fetch_with_content(
            &url,
            content.as_deref(),
            title.as_deref(),
            author.as_deref(),
            client.as_deref(),
        )
        .map_err(|error| error.to_string())?;
        let store = Store::open(&cfg.database_url).map_err(|error| error.to_string())?;
        store
            .upsert_feed(&item)
            .map_err(|error| error.to_string())?;
        let item = store
            .get_feed(&item.id)
            .map_err(|error| error.to_string())?
            .unwrap_or(item);
        full_item(&store, item)
    })
    .await;

    match stored {
        Ok(Ok(item)) => {
            enrich_in_background(item.id.clone());
            (StatusCode::CREATED, Json(json!(item)))
        }
        Ok(Err(error)) => (StatusCode::BAD_REQUEST, Json(json!({ "error": error }))),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        ),
    }
}

#[derive(Debug, Deserialize)]
struct RefreshBody {
    days: Option<i32>,
    ids: Option<Vec<String>>,
    force: Option<bool>,
}

async fn relevance_refresh_handler(Json(body): Json<RefreshBody>) -> (StatusCode, Json<Value>) {
    let days = body.days.unwrap_or(90);
    if !(1..=365).contains(&days) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "days must be between 1 and 365" })),
        );
    }
    let result = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_url).map_err(|error| error.to_string())?;
        let requested = body.ids.map(|ids| ids.into_iter().collect::<HashSet<_>>());
        let mut items = store
            .feed_for_relevance(days, 200)
            .map_err(|error| error.to_string())?;
        if let Some(ids) = requested {
            items.retain(|item| ids.contains(&item.id));
        }
        let profiles = relevance::load_profiles(&cfg.relevance);
        let embedding_role = cfg.embedding_role();
        let embedding_producer = embedding_role.as_ref().map(|role| role.cache_key());
        let reranking_role = cfg.reranking_role();
        let reranking_producer = reranking_role.as_ref().map(|role| role.cache_key());
        let travel_context = travel::load(&store, &cfg.travel_context);
        let context_revision = evaluation::context_revision(
            &profiles,
            embedding_producer.as_deref(),
            reranking_producer.as_deref(),
            &travel_context.revision,
        );
        let considered = items.len();
        if !body.force.unwrap_or(false) {
            items.retain(|item| {
                let item_revision = evaluation::item_revision(item);
                let stored = store.feed_evaluation(&item.id).ok().flatten();
                !evaluation::is_current(stored.as_ref(), &item_revision, &context_revision)
            });
        }
        let scored = relevance::score_items(
            &items,
            &profiles,
            embedding_role.as_ref(),
            reranking_role.as_ref(),
        );
        for (source, item) in items.iter().zip(&scored) {
            store
                .replace_feed_relevance(&item.feed_id, &item.matches)
                .map_err(|error| error.to_string())?;
            let evaluated = evaluation::evaluate(
                source,
                item.matches.first(),
                &context_revision,
                &travel_context.contexts,
            );
            store
                .replace_feed_evaluation(&evaluated)
                .map_err(|error| error.to_string())?;
        }
        let mode = scored
            .iter()
            .flat_map(|item| item.matches.first())
            .map(|relevance| relevance.mode.as_str())
            .next();
        Ok(json!({
            "scored": scored.len(),
            "evaluated": scored.len(),
            "considered": considered,
            "skipped_current": considered.saturating_sub(scored.len()),
            "profile_count": profiles.len(),
            "mode": mode,
            "bounded_to": 200,
            "evaluator_revision": evaluation::EVALUATOR_REVISION,
            "travel_context": {
                "upcoming_count": travel_context.contexts.len(),
                "reachable": travel_context.reachable,
                "from_cache": travel_context.from_cache,
                "refreshed_at": travel_context.refreshed_at,
            },
        }))
    })
    .await;

    match result {
        Ok(Ok(value)) => (StatusCode::OK, Json(value)),
        Ok(Err(error)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error })),
        ),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        ),
    }
}

async fn evaluation_status_handler() -> (StatusCode, Json<Value>) {
    let result = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_url).map_err(|error| error.to_string())?;
        let profiles = relevance::load_profiles(&cfg.relevance);
        let embedding_role = cfg.embedding_role();
        let embedding_producer = embedding_role.as_ref().map(|role| role.cache_key());
        let reranking_role = cfg.reranking_role();
        let reranking_producer = reranking_role.as_ref().map(|role| role.cache_key());
        let summarization_role = cfg.summarization_role();
        let travel_context = travel::cached(&store);
        let travel_revision = travel_context
            .as_ref()
            .map(|context| context.revision.as_str())
            .unwrap_or_default();
        let summary = store
            .evaluation_summary()
            .map_err(|error| error.to_string())?;
        let enrichment = store
            .feed_enrichment_counts()
            .map_err(|error| error.to_string())?;
        let content_status = store
            .feed_content_status_counts()
            .map_err(|error| error.to_string())?;
        let summarizer_reachable = media::summarizer_reachable(&cfg);
        let relevance_reachable =
            relevance::embedding_backend_reachable(embedding_role.as_ref());
        let reranking_reachable =
            relevance::embedding_backend_reachable(reranking_role.as_ref());
        Ok(json!({
            "evaluator_revision": evaluation::EVALUATOR_REVISION,
            "context_revision": evaluation::context_revision(
                &profiles,
                embedding_producer.as_deref(),
                reranking_producer.as_deref(),
                travel_revision,
            ),
            "ledger": {
                "evaluated": summary.evaluated,
                "reranked": summary.reranked,
                "semantic": summary.semantic,
                "lexical": summary.lexical,
                "unscored": summary.unscored,
            },
            "summarizer": {
                "provider": summarization_role
                    .as_ref()
                    .map(|role| role.provider_label())
                    .unwrap_or("No summarization role configured"),
                "model": summarization_role
                    .as_ref()
                    .map(|role| role.model.as_str())
                    .unwrap_or(""),
                "configured": summarization_role.is_some(),
                "reachable": summarizer_reachable,
            },
            "enrichment": {
                "pending_summaries": enrichment.pending_summaries,
                "failed_summaries": enrichment.failed_summaries,
                "content_status": {
                    "full": content_status.full,
                    "thin": content_status.thin,
                    "none": content_status.none,
                    "unknown": content_status.unknown,
                },
            },
            "relevance": {
                "provider": relevance::embedding_provider_label(embedding_role.as_ref()),
                "model": embedding_role
                    .as_ref()
                    .map(|role| role.model.as_str())
                    .unwrap_or(""),
                "configured": relevance::embedding_backend_configured(embedding_role.as_ref()),
                "reachable": relevance_reachable,
                "profile_count": profiles.len(),
                "active_mode": if relevance_reachable && reranking_reachable {
                    "reranked"
                } else if relevance_reachable {
                    "semantic"
                } else {
                    "lexical"
                },
            },
            "reranker": {
                "provider": reranking_role
                    .as_ref()
                    .map(|role| role.provider_label())
                    .unwrap_or("No reranking role configured"),
                "model": reranking_role
                    .as_ref()
                    .map(|role| role.model.as_str())
                    .unwrap_or(""),
                "configured": reranking_role.is_some(),
                "reachable": reranking_reachable,
            },
            "travel_context": {
                "enabled": cfg.travel_context.enabled,
                "source": cfg.travel_context.base_url,
                "upcoming_count": travel_context.as_ref().map(|context| context.contexts.len()).unwrap_or(0),
                "reachable": travel_context.as_ref().is_some_and(|context| context.reachable),
                "from_cache": travel_context.as_ref().is_some_and(|context| context.from_cache),
                "refreshed_at": travel_context.as_ref().map(|context| context.refreshed_at.clone()).unwrap_or_default(),
                "plans": travel_context
                    .as_ref()
                    .map(|snapshot| snapshot.contexts.iter().map(|plan| json!({
                        "id": plan.id,
                        "label": plan.title,
                        "date_start": plan.date_start,
                        "date_end": plan.date_end,
                    })).collect::<Vec<_>>())
                    .unwrap_or_default(),
            }
        }))
    })
    .await;

    match result {
        Ok(Ok(value)) => (StatusCode::OK, Json(value)),
        Ok(Err(error)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error })),
        ),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        ),
    }
}

#[derive(Debug, Serialize)]
struct FeedSourceOut {
    id: String,
    adapter: String,
    enabled: bool,
    source_url: String,
    query_configured: bool,
    limit: usize,
    last_run_at: Option<String>,
}

async fn sources_handler() -> (StatusCode, Json<Value>) {
    let result = tokio::task::spawn_blocking(move || -> Result<Vec<FeedSourceOut>, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_url).map_err(|error| error.to_string())?;
        cfg.feed_sources
            .into_iter()
            .map(|source| {
                let state = store
                    .get_source_state(&source.id)
                    .map_err(|error| error.to_string())?;
                Ok(FeedSourceOut {
                    id: source.id.clone(),
                    adapter: source.adapter.clone(),
                    enabled: source.enabled,
                    source_url: sources::source_url(&source),
                    query_configured: source
                        .query
                        .as_deref()
                        .is_some_and(|query| !query.trim().is_empty()),
                    limit: source.limit,
                    last_run_at: state.map(|state| state.last_run_at),
                })
            })
            .collect()
    })
    .await;

    match result {
        Ok(Ok(sources)) => (StatusCode::OK, Json(json!({ "sources": sources }))),
        Ok(Err(error)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error })),
        ),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        ),
    }
}

#[derive(Debug, Deserialize)]
struct SourceScanBody {
    source_id: Option<String>,
}

async fn source_scan_handler(Json(body): Json<SourceScanBody>) -> (StatusCode, Json<Value>) {
    let result = tokio::task::spawn_blocking(move || -> Result<(Value, Vec<String>), String> {
        let cfg = Config::load();
        let selected = cfg
            .feed_sources
            .into_iter()
            .filter(|source| source.enabled)
            .filter(|source| {
                body.source_id
                    .as_deref()
                    .is_none_or(|requested| requested == source.id)
            })
            .collect::<Vec<_>>();
        if selected.is_empty() {
            return Err("no matching enabled Feed source".into());
        }
        let store = Store::open(&cfg.database_url).map_err(|error| error.to_string())?;
        let mut results = Vec::new();
        let mut ids = HashSet::new();
        for source in selected {
            let found = sources::fetch(&source).map_err(|error| error.to_string())?;
            let mut new_count = 0;
            let mut failed = Vec::new();

            // The collector discovered URLs; the extractor builds the items, so
            // a repository found here and one pasted by hand land identical
            // (#79). One URL failing to extract does not fail the run.
            for discovered in &found {
                let item = match media::fetch(&discovered.url) {
                    Ok(item) => item,
                    Err(error) => {
                        eprintln!("source {}: {} -> {error}", source.id, discovered.url);
                        failed.push(discovered.url.clone());
                        continue;
                    }
                };

                if store
                    .upsert_feed(&item)
                    .map_err(|error| error.to_string())?
                {
                    new_count += 1;
                }
                store
                    .record_feed_origin(
                        &item.id,
                        &source.id,
                        &sources::source_url(&source),
                        discovered.label.as_deref().or(Some(&source.adapter)),
                    )
                    .map_err(|error| error.to_string())?;
                ids.insert(item.id.clone());
            }
            store
                .record_run(&source.id, None)
                .map_err(|error| error.to_string())?;
            let stored = found.len() - failed.len();
            results.push(json!({
                "source_id": source.id,
                "adapter": source.adapter,
                "discovered": found.len(),
                "fetched": stored,
                "new_count": new_count,
                "known_count": stored.saturating_sub(new_count),
                "failed": failed,
            }));
        }
        Ok((
            json!({
                "sources": results,
                "fetched": results.iter().map(|result| result["fetched"].as_u64().unwrap_or(0)).sum::<u64>(),
                "new_count": results.iter().map(|result| result["new_count"].as_u64().unwrap_or(0)).sum::<u64>(),
            }),
            ids.into_iter().collect(),
        ))
    })
    .await;

    match result {
        Ok(Ok((value, ids))) => {
            enrich_many_in_background(ids);
            (StatusCode::OK, Json(value))
        }
        Ok(Err(error)) => (StatusCode::BAD_REQUEST, Json(json!({ "error": error }))),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        ),
    }
}

#[derive(Debug, Serialize)]
struct VaultCandidateOut {
    id: String,
    source_id: String,
    source_ref: String,
    label: Option<String>,
    url: String,
    imported: bool,
}

async fn vault_scan_handler() -> (StatusCode, Json<Value>) {
    let result = tokio::task::spawn_blocking(move || -> Result<Vec<VaultCandidateOut>, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_url).map_err(|error| error.to_string())?;
        vault_links::scan(&cfg.vault_link_sources)
            .into_iter()
            .map(|candidate| {
                let imported = store
                    .get_feed_status(&candidate.id)
                    .map_err(|error| error.to_string())?
                    .is_some();
                Ok(VaultCandidateOut {
                    id: candidate.id,
                    source_id: candidate.source_id,
                    source_ref: candidate.source_ref,
                    label: candidate.label,
                    url: candidate.url,
                    imported,
                })
            })
            .collect()
    })
    .await;

    match result {
        Ok(Ok(candidates)) => (StatusCode::OK, Json(json!(candidates))),
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "vault link scan failed" })),
        ),
    }
}

#[derive(Debug, Deserialize)]
struct VaultImportBody {
    source_id: String,
    url: String,
}

async fn vault_import_handler(Json(body): Json<VaultImportBody>) -> (StatusCode, Json<Value>) {
    let result = tokio::task::spawn_blocking(move || -> Result<FeedFullItem, String> {
        let cfg = Config::load();
        let candidate = vault_links::scan(&cfg.vault_link_sources)
            .into_iter()
            .find(|candidate| candidate.source_id == body.source_id && candidate.url == body.url)
            .ok_or_else(|| "link is not in a configured Vault source".to_string())?;
        let mut item = media::fetch(&candidate.url).map_err(|error| error.to_string())?;
        if item.title.is_none() {
            item.title = candidate.label.clone();
        }
        let store = Store::open(&cfg.database_url).map_err(|error| error.to_string())?;
        store
            .upsert_feed(&item)
            .map_err(|error| error.to_string())?;
        store
            .record_feed_origin(
                &item.id,
                &candidate.source_id,
                &candidate.source_ref,
                candidate.label.as_deref(),
            )
            .map_err(|error| error.to_string())?;
        let item = store
            .get_feed(&item.id)
            .map_err(|error| error.to_string())?
            .unwrap_or(item);
        full_item(&store, item)
    })
    .await;

    match result {
        Ok(Ok(item)) => {
            enrich_in_background(item.id.clone());
            (StatusCode::CREATED, Json(json!(item)))
        }
        Ok(Err(error)) => (StatusCode::BAD_REQUEST, Json(json!({ "error": error }))),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        ),
    }
}

#[derive(Debug, Deserialize)]
struct TriageParams {
    status: Option<String>,
}

async fn triage_handler(Query(params): Query<TriageParams>) -> Json<Value> {
    let result = tokio::task::spawn_blocking(move || -> Option<Vec<TriageOut>> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_url).ok()?;
        let items = store.list_triage(params.status.as_deref()).ok()?;
        Some(items.into_iter().map(TriageOut::from).collect())
    })
    .await
    .ok()
    .flatten();

    match result {
        Some(items) => Json(json!(items)),
        None => Json(json!({ "error": "triage query failed" })),
    }
}

#[path = "../../../libs/axon-server/src/lib.rs"]
#[allow(dead_code)]
mod axon_server;

/// Drain the summary backlog on an interval, because ingest-triggered
/// enrichment only ever reaches items that arrive while the inference server is
/// up. Items ingested during an outage used to sit empty until someone
/// remembered `comms summarize --pending` by hand, which is how 36 of 39 items
/// ended up without a summary (#74).
///
/// Bounded by the ledger rather than by anything here: `feed_pending_summaries`
/// skips items at three failed attempts or with a backoff still in the future,
/// and each failure records its error class, so a permanently broken item stops
/// costing anything and says why. A pass that finds nothing is silent; anything
/// else is worth a line in the log.
fn spawn_enrichment_drain(every_minutes: u64) {
    if every_minutes == 0 {
        eprintln!("enrichment drain disabled (enrichment_drain_minutes = 0)");
        return;
    }

    eprintln!("enrichment drain: every {every_minutes} min");
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(every_minutes * 60));
        loop {
            ticker.tick().await;
            // The join result is inspected, not discarded: a panic inside the
            // blocking half would otherwise take the drain down without a word,
            // which is the same silence this issue exists to remove.
            let joined = tokio::task::spawn_blocking(|| {
                let cfg = Config::load();
                let store = match Store::open(&cfg.database_url) {
                    Ok(store) => store,
                    Err(error) => {
                        eprintln!("enrichment drain: store unavailable: {error}");
                        return;
                    }
                };
                let before = match store.feed_enrichment_counts() {
                    Ok(counts) => counts,
                    Err(error) => {
                        eprintln!("enrichment drain: backlog query failed: {error}");
                        return;
                    }
                };

                match media::summarize_pending(&store, &cfg) {
                    Ok(n) if n > 0 => eprintln!("enrichment drain: summarized {n} item(s)"),
                    // A backlog that did not move is the case this whole issue
                    // is about. Staying quiet here would rebuild the silence
                    // the ledger was supposed to end.
                    Ok(_) if before.pending_summaries > 0 => eprintln!(
                        "enrichment drain: {} pending, {} failed, none summarized — the 'summarization' inference role is unreachable or unconfigured",
                        before.pending_summaries, before.failed_summaries
                    ),
                    Ok(_) => {}
                    Err(error) => eprintln!("enrichment drain: {error}"),
                }
            })
            .await;

            if let Err(error) = joined {
                eprintln!("enrichment drain: pass did not finish: {error}");
            }
        }
    });
}

/// The whole HTTP surface, assembled from the two things that decide who may
/// call it. Split out of `main` so a test can serve it on an ephemeral port and
/// exercise the auth boundary over real HTTP rather than by reading the code
/// (#73) — an auth layer asserted only by inspection is the kind that regresses
/// quietly.
fn build_router(api_secret: Option<String>, dashboard_origin: &str) -> Router {
    // CORS: allow only the dashboard origin, not permissive.
    let cors = CorsLayer::new()
        .allow_origin(
            dashboard_origin
                .parse::<HeaderValue>()
                .unwrap_or_else(|_| "http://127.0.0.1:47117".parse().unwrap()),
        )
        .allow_methods([Method::GET, Method::POST])
        .allow_headers([
            axum::http::header::CONTENT_TYPE,
            axum::http::header::AUTHORIZATION,
            HeaderName::from_static("x-axon-token"),
        ]);

    // Read-only routes: no auth required.
    let read_routes = Router::new()
        .route("/health", get(health_handler))
        .route("/feed", get(feed_handler))
        .route("/feed/origins", get(feed_origins_handler))
        .route("/feed/runs", get(feed_runs_handler))
        .route("/feed/evaluation/status", get(evaluation_status_handler))
        .route("/feed/quality", get(quality_queue_handler))
        .route("/feed/:id", get(feed_item_handler))
        .route("/sources", get(sources_handler))
        .route("/triage", get(triage_handler));

    // Mutating routes: require shared secret.
    let write_routes = Router::new()
        .route("/feed/relevance/refresh", post(relevance_refresh_handler))
        .route("/feed/quality/refresh", post(quality_refresh_handler))
        .route("/feed/:id/status", post(feed_status_handler))
        .route("/ingest", post(ingest_handler))
        .route("/vault-links/scan", post(vault_scan_handler))
        .route("/vault-links/import", post(vault_import_handler))
        .route("/sources/scan", post(source_scan_handler))
        .layer(axum::middleware::from_fn_with_state(
            api_secret,
            require_auth,
        ));

    Router::new()
        .merge(read_routes)
        .merge(write_routes)
        .layer(cors)
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024)) // 2 MiB
}

#[tokio::main]
async fn main() {
    let cfg = Config::load();

    let api_secret = cfg.api_secret.clone();
    if api_secret.is_none() || api_secret.as_deref() == Some("") {
        eprintln!("warning: api_secret_file is not configured — mutating routes will reject all requests. See comms.config.example.json.");
    }

    spawn_enrichment_drain(cfg.enrichment_drain_minutes);

    let app = build_router(api_secret, &cfg.dashboard_origin);

    // Bind and the exit-on-failure behaviour live in axon_server now; this file
    // used to hand-roll the same five lines with an unwrap panic instead.
    axon_server::serve_local("comms-server", cfg.port, app).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serve the real router on an ephemeral port and return its base URL. The
    /// requests below go over actual HTTP, because the thing under test is the
    /// middleware stack — a handler called directly would skip the layer that
    /// is the entire point.
    async fn serve(api_secret: Option<&str>) -> String {
        let app = build_router(api_secret.map(str::to_string), "http://127.0.0.1:47117");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        format!("http://{addr}")
    }

    /// A page loaded in a browser on this host is already inside the loopback
    /// boundary, so `127.0.0.1` is not what contains `/ingest` — the token is.
    const HOSTILE_ORIGIN: &str = "https://attacker.example";

    #[tokio::test]
    async fn unauthenticated_cross_origin_post_to_ingest_is_rejected() {
        let base = serve(Some("s3cret")).await;
        let response = reqwest::Client::new()
            .post(format!("{base}/ingest"))
            .header("Origin", HOSTILE_ORIGIN)
            .json(&json!({ "url": "https://example.com/anything" }))
            .send()
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            401,
            "no token must not reach the handler"
        );
        assert!(
            response
                .headers()
                .get("access-control-allow-origin")
                .map(|v| v.to_str().unwrap_or_default() != HOSTILE_ORIGIN)
                .unwrap_or(true),
            "the hostile origin must never be echoed back as allowed"
        );
    }

    #[tokio::test]
    async fn computed_quality_refresh_is_a_protected_explicit_write() {
        let base = serve(Some("s3cret")).await;
        let response = reqwest::Client::new()
            .post(format!("{base}/feed/quality/refresh"))
            .json(&json!({ "days": 30 }))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), 401);
    }

    #[tokio::test]
    async fn a_wrong_token_is_rejected_too() {
        let base = serve(Some("s3cret")).await;
        for header in [("Authorization", "Bearer wrong"), ("X-Axon-Token", "wrong")] {
            let response = reqwest::Client::new()
                .post(format!("{base}/ingest"))
                .header(header.0, header.1)
                .header("Origin", HOSTILE_ORIGIN)
                .json(&json!({ "url": "https://example.com/anything" }))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 401, "{} with a wrong value", header.0);
        }
    }

    #[tokio::test]
    async fn an_unconfigured_secret_disables_mutating_routes_rather_than_opening_them() {
        for secret in [None, Some("")] {
            let base = serve(secret).await;
            let response = reqwest::Client::new()
                .post(format!("{base}/ingest"))
                .json(&json!({ "url": "https://example.com/anything" }))
                .send()
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                403,
                "an absent secret must close the route, never leave it open"
            );
        }
    }

    #[tokio::test]
    async fn the_right_token_gets_past_the_layer_and_read_routes_never_need_one() {
        let base = serve(Some("s3cret")).await;

        // Past the auth layer the handler fails on its own (no database in a
        // unit test); what matters is that the failure is no longer 401/403.
        let authorized = reqwest::Client::new()
            .post(format!("{base}/ingest"))
            .header("Authorization", "Bearer s3cret")
            .json(&json!({ "url": "https://example.com/anything" }))
            .send()
            .await
            .unwrap();
        assert!(
            authorized.status() != 401 && authorized.status() != 403,
            "a valid token was still refused: {}",
            authorized.status()
        );

        let health = reqwest::get(format!("{base}/health")).await.unwrap();
        assert_eq!(health.status(), 200, "read routes carry no auth layer");
    }
}
