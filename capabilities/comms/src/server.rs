//! `comms-server` — HTTP surface for the general Feed.
//!
//! Feed persistence, TELOS relevance, explicit Vault-link discovery and reader
//! payloads live here. Scouting remains a separate opportunity engine. Network
//! fetches and embedding calls run in spawn_blocking; the server binds only to
//! loopback because ingest is allowed to fetch external URLs.

use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};

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

use comms::cloud_derivative::{self, CloudDerivativePreview, CloudDocumentInput};
use comms::cloud_dispatch;
use comms::config::Config;
use comms::content_item::{self, DataClass};
use comms::digest;
use comms::evaluation::{self, EvaluationFactor, FeedEvaluation};
use comms::google::{self, ThreadAction, ThreadLocation};
use comms::intake;
use comms::media;
use comms::provenance::StageProvenance;
use comms::quality;
use comms::relevance::{self, RelevanceMatch};
use comms::sources;
use comms::store::{
    CloudAttemptClaim, CloudDerivativeApproval, CloudDerivativeState, CloudQueueRequest, FeedItem,
    FeedOrigin, FeedRun, GmailActionJob, OriginSummary, Store, TriageItem,
};
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
/// What this capability answers, served as data beside `/health`.
/// Required query parameters are named in the summary: a path alone cannot tell
/// a caller what it must send, and learning that from a 400 is the thing this
/// endpoint exists to avoid.
const ROUTES: &[route_manifest::Route] = &[
    r("GET", "/health", "Liveness."),
    r("GET", "/ready", "Readiness: liveness plus a reachable database."),
    r("GET", "/routes", "This manifest."),
    r(
        "GET",
        "/content/:source/:id",
        "An item as content-item-v1. :source is feed or mail.",
    ),
    r(
        "POST",
        "/content/:source/:id/digest",
        "Generate or refine this item's local digest. Optional depth and focus[].",
    ),
    r(
        "POST",
        "/content/:source/:id/diagram",
        "Draw this item as a validated Mermaid diagram.",
    ),
    r(
        "POST",
        "/content/:source/:id/chart",
        "Extract a chartable table, every value verified against the source.",
    ),
    r(
        "POST",
        "/content/digests/refresh",
        "Bounded automatic pass over one source. Requires source, optional limit.",
    ),
    r(
        "POST",
        "/content/:source/:id/cloud-preview",
        "Build a bounded, reviewable copy for cloud use.",
    ),
    r(
        "POST",
        "/content/:source/:id/cloud-approval",
        "Approve the exact previewed copy.",
    ),
    r(
        "POST",
        "/content/:source/:id/cloud-queue",
        "Queue an approved copy for a provider role.",
    ),
    r(
        "POST",
        "/content/cloud-jobs/:job_id/run",
        "Run a queued cloud job.",
    ),
    r(
        "GET",
        "/content/cloud-providers",
        "Provider roles and whether each is available.",
    ),
    r("GET", "/feed", "Feed entries. Optional status filter."),
    r("POST", "/ingest", "Ingest one URL into the feed."),
    r("GET", "/feed/:id", "One feed entry."),
    r(
        "POST",
        "/feed/:id/status",
        "Set a feed entry's status (keeper, dismissed).",
    ),
    r("GET", "/feed/runs", "Recent collector runs. Optional days."),
    r(
        "GET",
        "/feed/origins",
        "Which source run produced each entry.",
    ),
    r(
        "GET",
        "/feed/quality",
        "Entries flagged for quality review.",
    ),
    r("POST", "/feed/quality/refresh", "Recompute quality flags."),
    r(
        "GET",
        "/feed/evaluation/status",
        "Evaluation backlog and coverage.",
    ),
    r(
        "POST",
        "/feed/relevance/refresh",
        "Rescore feed relevance against the current profiles.",
    ),
    r("GET", "/sources", "Declared feed sources."),
    r(
        "POST",
        "/sources/scan",
        "Collect from the declared sources.",
    ),
    r("GET", "/triage", "Mail proposals. Optional status filter."),
    r(
        "POST",
        "/triage/:id/status",
        "Set a mail proposal's status.",
    ),
    r(
        "POST",
        "/triage/:id/stream",
        "Reclassify a mail into a category.",
    ),
    r(
        "POST",
        "/triage/:id/data-class",
        "Set a mail's data classification by hand.",
    ),
    r(
        "POST",
        "/triage/:id/gmail",
        "Apply a Gmail action (archive, trash, restore).",
    ),
    r(
        "POST",
        "/triage/:id/gmail-job",
        "Queue a Gmail action for retry.",
    ),
    r(
        "POST",
        "/triage/bulk",
        "Apply one action across many mails.",
    ),
    r("POST", "/triage/sweep", "Pull new mail from Gmail."),
    r(
        "GET",
        "/triage/sweep/status",
        "Freshness and failure state of the scheduled inbox sweep.",
    ),
    r(
        "POST",
        "/triage/reconcile",
        "Reconcile Axon's mail state against Gmail.",
    ),
    r(
        "POST",
        "/triage/relevance/refresh",
        "Rescore mail relevance against the current profiles.",
    ),
    r(
        "POST",
        "/triage/redact",
        "Redact stored review fields of Private mail already persisted.",
    ),
    r(
        "POST",
        "/vault-links/scan",
        "Vault notes that could be linked to feed entries.",
    ),
    r(
        "POST",
        "/vault-links/import",
        "Link scanned vault notes to their entries.",
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
    Json(route_manifest::manifest("comms", ROUTES))
}

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
            processing: processing
                .into_iter()
                .map(StageProvenanceOut::from)
                .collect(),
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
    classification_method: String,
    classification_version: String,
    data_class: String,
    data_class_rationale: String,
    data_classification_method: String,
    data_classification_version: String,
    status: String,
    gmail_action: Option<String>,
    gmail_action_at: Option<String>,
    purge_after: Option<String>,
    gmail_location: Option<String>,
    gmail_observed_at: Option<String>,
    gmail_sync_status: Option<String>,
    gmail_sync_action: Option<String>,
    gmail_sync_error: Option<String>,
    /// The doctrine's one state label. Rendered as a badge rather than folded into
    /// `status`: status is what Axon decided about a proposal, waiting is what the
    /// operator decided about the conversation, and collapsing them would make
    /// "I replied and I'm blocked" indistinguishable from "Axon dismissed it".
    waiting: bool,
    waiting_since: Option<String>,
    first_seen: String,
    last_seen: String,
    relevance: Vec<RelevanceOut>,
}

impl TriageOut {
    fn from_store(item: TriageItem, relevance: Vec<RelevanceMatch>) -> Self {
        Self {
            id: item.id,
            from_addr: item.from_addr,
            subject: item.subject,
            snippet: item.snippet,
            internal_date: item.internal_date_text,
            stream: item.stream,
            rationale: item.rationale,
            classification_method: item.classification_method,
            classification_version: item.classification_version,
            data_class: item.data_class,
            data_class_rationale: item.data_class_rationale,
            data_classification_method: item.data_classification_method,
            data_classification_version: item.data_classification_version,
            status: item.status,
            gmail_action: item.gmail_action,
            gmail_action_at: item.gmail_action_at,
            purge_after: item.purge_after,
            gmail_location: item.gmail_location,
            gmail_observed_at: item.gmail_observed_at,
            gmail_sync_status: item.gmail_sync_status,
            gmail_sync_action: item.gmail_sync_action,
            gmail_sync_error: item.gmail_sync_error,
            waiting: item.waiting,
            waiting_since: item.waiting_since,
            first_seen: item.first_seen,
            last_seen: item.last_seen,
            relevance: relevance.into_iter().map(RelevanceOut::from).collect(),
        }
    }
}

/// Source-specific fields attached to the canonical content reader contract.
/// They extend the content item without forcing Gmail workflow state into
/// every other Feed source.
#[derive(Debug, Serialize)]
struct MailContentExtensionOut {
    category: String,
    rationale: String,
    classification_method: String,
    classification_version: String,
    gmail_action: Option<String>,
    gmail_action_at: Option<String>,
    purge_after: Option<String>,
    gmail_location: Option<String>,
    gmail_observed_at: Option<String>,
    gmail_sync_status: Option<String>,
    gmail_sync_action: Option<String>,
    gmail_sync_error: Option<String>,
}

/// One reader shape for every kind of observed content. Source adapters own
/// collection and actions; the dashboard owns one renderer for this contract.
#[derive(Debug, Serialize)]
struct ContentItemOut {
    schema_version: &'static str,
    source: &'static str,
    id: String,
    kind: String,
    title: Option<String>,
    url: String,
    author: Option<String>,
    summary: Option<String>,
    content: Option<String>,
    content_label: String,
    day: String,
    created_at: String,
    status: String,
    content_status: String,
    data_class: DataClass,
    processing_policy: content_item::ProcessingPolicy,
    cloud_processing: CloudDerivativeState,
    relevance: Vec<RelevanceOut>,
    evaluation: Option<EvaluationOut>,
    processing: Vec<StageProvenanceOut>,
    origins: Vec<OriginOut>,
    digest: Option<content_item::Digest>,
    mail: Option<MailContentExtensionOut>,
}

impl ContentItemOut {
    fn from_feed(
        item: FeedItem,
        relevance: Vec<RelevanceMatch>,
        evaluation: Option<FeedEvaluation>,
        processing: Vec<StageProvenance>,
        origins: Vec<FeedOrigin>,
    ) -> Self {
        let classification = DataClass::public_source_default();
        let processing_policy = content_item::processing_policy(&classification.value);
        let content_label = match item.kind.as_str() {
            "github" => "README",
            "arxiv" => "Abstract",
            "youtube" | "podcast" | "instagram" => "Transcript",
            _ => "Article content",
        };
        Self {
            schema_version: "content-item-v1",
            source: "feed",
            id: item.id,
            kind: item.kind,
            title: item.title,
            url: item.url,
            author: item.author,
            summary: item.summary,
            content: item.transcript,
            content_label: content_label.into(),
            day: item.day,
            created_at: item.created_at,
            status: item.status,
            content_status: item.content_status,
            data_class: classification,
            processing_policy,
            cloud_processing: CloudDerivativeState::not_prepared(),
            relevance: relevance.into_iter().map(RelevanceOut::from).collect(),
            evaluation: evaluation.map(EvaluationOut::from),
            processing: processing
                .into_iter()
                .map(StageProvenanceOut::from)
                .collect(),
            origins: origins.into_iter().map(OriginOut::from).collect(),
            // Filled by `attach_digest` -- a projection cannot query.
            digest: None,
            mail: None,
        }
    }

    fn from_mail(item: TriageItem, relevance: Vec<RelevanceMatch>) -> Self {
        let created_at = item
            .internal_date_text
            .clone()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| item.first_seen.clone());
        let day = created_at.get(..10).unwrap_or_default().to_string();
        let content_status = if item
            .snippet
            .as_deref()
            .is_some_and(|snippet| !snippet.trim().is_empty())
        {
            "thin"
        } else {
            "none"
        };
        let classification = DataClass::new(
            item.data_class.clone(),
            item.data_class_rationale.clone(),
            item.data_classification_method.clone(),
            item.data_classification_version.clone(),
        );
        let processing_policy = content_item::processing_policy(&classification.value);
        Self {
            schema_version: "content-item-v1",
            source: "mail",
            url: format!("https://mail.google.com/mail/u/0/#all/{}", item.id),
            id: item.id,
            kind: "mail".into(),
            title: item.subject,
            author: item.from_addr,
            summary: None,
            content: item.snippet,
            content_label: "Message preview".into(),
            day,
            created_at,
            status: item.status,
            content_status: content_status.into(),
            data_class: classification,
            processing_policy,
            cloud_processing: CloudDerivativeState::not_prepared(),
            relevance: relevance.into_iter().map(RelevanceOut::from).collect(),
            evaluation: None,
            processing: Vec::new(),
            origins: Vec::new(),
            digest: None,
            mail: Some(MailContentExtensionOut {
                category: item.stream,
                rationale: item.rationale,
                classification_method: item.classification_method,
                classification_version: item.classification_version,
                gmail_action: item.gmail_action,
                gmail_action_at: item.gmail_action_at,
                purge_after: item.purge_after,
                gmail_location: item.gmail_location,
                gmail_observed_at: item.gmail_observed_at,
                gmail_sync_status: item.gmail_sync_status,
                gmail_sync_action: item.gmail_sync_action,
                gmail_sync_error: item.gmail_sync_error,
            }),
        }
    }

    fn cloud_input(&self) -> CloudDocumentInput {
        CloudDocumentInput {
            source: self.source.into(),
            id: self.id.clone(),
            title: self.title.clone(),
            author: self.author.clone(),
            summary: self.summary.clone(),
            content: self.content.clone(),
            data_class: self.data_class.value.clone(),
        }
    }

    /// Read the stored digest, if one exists.
    ///
    /// Reads only. A GET that quietly runs a local model turns opening an item
    /// into a two-minute wait and a load nobody asked for; generating is always
    /// an explicit press or the bounded pass.
    fn attach_digest(mut self, store: &Store) -> Result<Self, Box<dyn std::error::Error>> {
        self.digest = store
            .content_digest(self.source, &self.id)?
            .as_ref()
            .map(digest::to_contract);
        Ok(self)
    }

    fn attach_cloud_state(mut self, store: &Store) -> Result<Self, Box<dyn std::error::Error>> {
        let preview = cloud_derivative::prepare(&self.cloud_input());
        self.cloud_processing = store.cloud_derivative_state(
            self.source,
            &self.id,
            &preview.source_revision,
            &preview.preview_hash,
        )?;
        Ok(self)
    }
}

async fn health_handler() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "service": "comms",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

/// Readiness: whether this capability can actually serve, which liveness does not answer.
///
/// `health_handler` is a literal and cannot observe the database, so during a Postgres outage
/// this capability reported itself up while every query behind it failed (#126). Availability
/// is judged here instead.
///
/// A read route, like `/health`: an availability probe that needed the shared secret would
/// make every consumer hold a credential to ask whether the feed is up.
///
/// 503, not 500: the request was fine, the dependency is not, and a caller that retries should
/// be told to come back rather than to fix its input.
async fn ready_handler() -> (StatusCode, Json<Value>) {
    let probe = tokio::task::spawn_blocking(|| {
        let cfg = Config::load();
        Store::open(&cfg.database_url)
            .and_then(|store| store.ping())
            .map_err(|error| error.to_string())
    })
    .await;
    match probe {
        Ok(Ok(())) => (
            StatusCode::OK,
            Json(json!({ "status": "ready", "service": "comms" })),
        ),
        Ok(Err(error)) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "status": "unavailable", "service": "comms", "error": error })),
        ),
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "status": "unavailable",
                "service": "comms",
                "error": "readiness check failed"
            })),
        ),
    }
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
            .map(|item| -> Result<FeedListItem, String> {
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

fn load_content_item(
    store: &Store,
    source: &str,
    id: &str,
) -> Result<Option<ContentItemOut>, String> {
    let item = match source {
        "feed" => store
            .get_feed(id)
            .map_err(|error| error.to_string())?
            .map(|item| -> Result<ContentItemOut, String> {
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
                Ok(ContentItemOut::from_feed(
                    item, relevance, evaluation, processing, origins,
                ))
            })
            .transpose()?,
        "mail" => store
            .get_triage(id)
            .map_err(|error| error.to_string())?
            .map(|item| -> Result<ContentItemOut, String> {
                let relevance = store
                    .triage_relevance(&item.id)
                    .map_err(|error| error.to_string())?;
                Ok(ContentItemOut::from_mail(item, relevance))
            })
            .transpose()?,
        _ => return Err("source must be 'feed' or 'mail'".into()),
    };
    item.map(|item| {
        item.attach_digest(store)
            .and_then(|item| item.attach_cloud_state(store))
            .map_err(|error| error.to_string())
    })
    .transpose()
}

#[derive(Debug, Default, Deserialize)]
struct DigestBody {
    /// `"standard"` or `"detailed"`. Absent means the automatic rung.
    depth: Option<String>,
    /// What the reader wants the digest to pay attention to.
    #[serde(default)]
    focus: Vec<String>,
}

/// Generate or refine one item's digest.
///
/// Synchronous on purpose, unlike `POST /ingest`: this is a button the operator
/// just pressed and is watching, so answering before the model has finished
/// would mean showing them the *old* digest and hoping they refresh.
async fn digest_handler(
    Path((source, id)): Path<(String, String)>,
    raw: axum::body::Bytes,
) -> (StatusCode, Json<Value>) {
    // Raw bytes rather than `Option<Json<DigestBody>>`: that extractor yields
    // `None` for a *malformed* body exactly as it does for an absent one, so a
    // client typo silently produced a standard digest instead of the detailed
    // one it asked for. A button that quietly does something other than what it
    // says is worse than one that reports it could not.
    let body: DigestBody = if raw.is_empty() {
        DigestBody::default()
    } else {
        match serde_json::from_slice(&raw) {
            Ok(body) => body,
            Err(error) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": format!("could not read the request body: {error}") })),
                )
            }
        }
    };
    let directive = match digest::parse_directive(body.depth.as_deref(), body.focus) {
        Ok(directive) => directive,
        Err(error) => return (StatusCode::BAD_REQUEST, Json(json!({ "error": error }))),
    };
    let result = tokio::task::spawn_blocking(move || {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_url).map_err(|error| error.to_string())?;
        digest::generate(&store, &cfg, &source, &id, &directive)
            .map(|row| row.as_ref().map(digest::to_contract))
            .map_err(|error| error.to_string())
    })
    .await;
    digest_response(result, "digest failed")
}

/// Draw one item as a Mermaid diagram.
async fn diagram_handler(Path((source, id)): Path<(String, String)>) -> (StatusCode, Json<Value>) {
    let result = tokio::task::spawn_blocking(move || {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_url).map_err(|error| error.to_string())?;
        digest::generate_diagram(&store, &cfg, &source, &id)
            .map(|row| row.as_ref().map(digest::to_contract))
            .map_err(|error| error.to_string())
    })
    .await;
    digest_response(result, "diagram failed")
}

/// Pull a chartable table out of an item.
///
/// Most content has none, and that comes back as `skipped_short` rather than an
/// error: a reader that showed a failure for every ordinary blog post would
/// train the operator to stop reading the state.
async fn chart_handler(Path((source, id)): Path<(String, String)>) -> (StatusCode, Json<Value>) {
    let result = tokio::task::spawn_blocking(move || {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_url).map_err(|error| error.to_string())?;
        digest::generate_chart(&store, &cfg, &source, &id)
            .map(|row| row.as_ref().map(digest::to_contract))
            .map_err(|error| error.to_string())
    })
    .await;
    digest_response(result, "chart failed")
}

fn digest_response(
    result: Result<Result<Option<content_item::Digest>, String>, tokio::task::JoinError>,
    failure: &'static str,
) -> (StatusCode, Json<Value>) {
    match result {
        Ok(Ok(Some(digest))) => (StatusCode::OK, Json(json!(digest))),
        Ok(Ok(None)) => (StatusCode::NOT_FOUND, Json(json!({ "error": "not found" }))),
        Ok(Err(error)) if error.starts_with("unknown digest source") => {
            (StatusCode::BAD_REQUEST, Json(json!({ "error": error })))
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": failure })),
        ),
    }
}

#[derive(Debug, Deserialize)]
struct DigestRefreshBody {
    source: String,
    limit: Option<i64>,
}

/// The bounded automatic pass over one source.
///
/// Explicit rather than timer-driven: for mail this reads message bodies, and a
/// background job that quietly pulls every body out of a mailbox is not
/// something a machine should start doing on its own.
async fn digest_refresh_handler(Json(body): Json<DigestRefreshBody>) -> (StatusCode, Json<Value>) {
    let result = tokio::task::spawn_blocking(move || -> Result<(String, usize), String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_url).map_err(|error| error.to_string())?;
        let written = digest::refresh_pending(&store, &cfg, &body.source, body.limit.unwrap_or(25))
            .map_err(|error| error.to_string())?;
        Ok((body.source, written))
    })
    .await;

    match result {
        Ok(Ok((source, written))) => (
            StatusCode::OK,
            Json(json!({ "source": source, "digested": written })),
        ),
        Ok(Err(error)) if error.contains("no digest queue for source") => {
            (StatusCode::BAD_REQUEST, Json(json!({ "error": error })))
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "digest refresh failed" })),
        ),
    }
}

async fn content_item_handler(
    Path((source, id)): Path<(String, String)>,
) -> (StatusCode, Json<Value>) {
    let result = tokio::task::spawn_blocking(move || -> Result<Option<ContentItemOut>, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_url).map_err(|error| error.to_string())?;
        load_content_item(&store, &source, &id)
    })
    .await;

    match result {
        Ok(Ok(Some(item))) => (StatusCode::OK, Json(json!(item))),
        Ok(Ok(None)) => (StatusCode::NOT_FOUND, Json(json!({ "error": "not found" }))),
        Ok(Err(error)) if error == "source must be 'feed' or 'mail'" => {
            (StatusCode::BAD_REQUEST, Json(json!({ "error": error })))
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "content query failed" })),
        ),
    }
}

#[derive(Debug, Serialize)]
struct CloudProviderOut {
    role: String,
    name: String,
    model: String,
    provider_label: &'static str,
    location: &'static str,
    data_tier: &'static str,
    billing_mode: &'static str,
    failover_priority: u16,
    max_requests_per_day: u32,
    requests_used_today: Option<u32>,
    requests_remaining_today: Option<u32>,
    max_input_tokens: u32,
    credit_expires_on: Option<String>,
    available: bool,
    unavailable_reason: Option<&'static str>,
}

fn cloud_provider_options(
    cfg: &Config,
    store: Option<&Store>,
    utc_date: Option<&str>,
) -> Vec<CloudProviderOut> {
    cfg.inference
        .roles_with_prefix("cloud_")
        .into_iter()
        .filter(|(_, role)| role.has_cloud_policy())
        .map(|(name, role)| {
            let request_limit = role.max_requests_per_day.unwrap();
            let calls = store.and_then(|store| store.cloud_provider_calls_today(&name).ok());
            let unavailable_reason = if !role.credential_ready() {
                Some("missing_credential")
            } else if !utc_date.is_some_and(|date| role.billing_active_on(date)) {
                Some("billing_expired_or_unknown")
            } else if calls.is_none() {
                Some("budget_unavailable")
            } else if calls.is_some_and(|used| used >= request_limit) {
                Some("daily_request_limit_reached")
            } else {
                None
            };
            CloudProviderOut {
                role: name,
                name: role.provider_name.clone().unwrap_or_default(),
                model: role.model.clone(),
                provider_label: role.provider_label(),
                location: "cloud",
                data_tier: role.cloud_data_tier.unwrap().as_str(),
                billing_mode: role.billing_mode.unwrap().as_str(),
                failover_priority: role.failover_priority(),
                max_requests_per_day: request_limit,
                requests_used_today: calls,
                requests_remaining_today: calls.map(|used| request_limit.saturating_sub(used)),
                max_input_tokens: role.max_input_tokens.unwrap(),
                credit_expires_on: role.credit_expires_on.clone(),
                available: unavailable_reason.is_none(),
                unavailable_reason,
            }
        })
        .collect()
}

fn cloud_tier_allows(
    tier: Option<&str>,
    original_data_class: &str,
    derivative_data_class: &str,
    transformation: &str,
) -> bool {
    if original_data_class == "vault" {
        return false;
    }
    let public_derivative = original_data_class == "public"
        && derivative_data_class == "public"
        && transformation == cloud_derivative::PASSTHROUGH_VERSION;
    match tier {
        Some("public") => public_derivative,
        Some("pseudonymized_personal") => {
            public_derivative
                || (original_data_class == "personal"
                    && derivative_data_class == "personal"
                    && transformation == cloud_derivative::REDACTION_VERSION)
        }
        _ => false,
    }
}

async fn cloud_providers_handler() -> Json<Value> {
    let providers = tokio::task::spawn_blocking(|| {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_url).ok();
        let utc_date = store.as_ref().and_then(|store| store.utc_date().ok());
        cloud_provider_options(&cfg, store.as_ref(), utc_date.as_deref())
    })
    .await
    .unwrap_or_default();
    Json(json!(providers))
}

async fn cloud_preview_handler(
    Path((source, id)): Path<(String, String)>,
) -> (StatusCode, Json<Value>) {
    let result =
        tokio::task::spawn_blocking(move || -> Result<Option<CloudDerivativePreview>, String> {
            let cfg = Config::load();
            let store = Store::open(&cfg.database_url).map_err(|error| error.to_string())?;
            Ok(load_content_item(&store, &source, &id)?
                .map(|item| cloud_derivative::prepare(&item.cloud_input())))
        })
        .await;

    match result {
        Ok(Ok(Some(preview))) => (StatusCode::OK, Json(json!(preview))),
        Ok(Ok(None)) => (StatusCode::NOT_FOUND, Json(json!({ "error": "not found" }))),
        Ok(Err(error)) if error == "source must be 'feed' or 'mail'" => {
            (StatusCode::BAD_REQUEST, Json(json!({ "error": error })))
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "cloud preview failed" })),
        ),
    }
}

#[derive(Debug, Deserialize)]
struct CloudApprovalBody {
    preview_hash: String,
}

async fn cloud_approval_handler(
    Path((source, id)): Path<(String, String)>,
    Json(body): Json<CloudApprovalBody>,
) -> (StatusCode, Json<Value>) {
    let result =
        tokio::task::spawn_blocking(move || -> Result<Option<CloudDerivativeState>, String> {
            let cfg = Config::load();
            let store = Store::open(&cfg.database_url).map_err(|error| error.to_string())?;
            let Some(item) = load_content_item(&store, &source, &id)? else {
                return Ok(None);
            };
            let preview = cloud_derivative::prepare(&item.cloud_input());
            if preview.preview_hash != body.preview_hash {
                return Err(
                    "preview is stale; prepare and review the current document again".into(),
                );
            }
            if preview.original_data_class == "vault" {
                return Err("vault content cannot be staged for cloud processing".into());
            }
            let approval = CloudDerivativeApproval {
                source: preview.source,
                item_id: preview.id,
                source_revision: preview.source_revision,
                preview_hash: preview.preview_hash,
                original_data_class: preview.original_data_class,
                derivative_data_class: preview.derivative_data_class,
                transformation: preview.transformation.into(),
                document: preview.document,
                redaction_count: preview.redaction_count as i32,
            };
            store
                .stage_cloud_derivative(&approval)
                .map(Some)
                .map_err(|error| error.to_string())
        })
        .await;

    match result {
        Ok(Ok(Some(state))) => (StatusCode::OK, Json(json!(state))),
        Ok(Ok(None)) => (StatusCode::NOT_FOUND, Json(json!({ "error": "not found" }))),
        Ok(Err(error)) if error.starts_with("preview is stale") => {
            (StatusCode::CONFLICT, Json(json!({ "error": error })))
        }
        Ok(Err(error)) if error.starts_with("vault content") => {
            (StatusCode::BAD_REQUEST, Json(json!({ "error": error })))
        }
        Ok(Err(error)) if error == "source must be 'feed' or 'mail'" => {
            (StatusCode::BAD_REQUEST, Json(json!({ "error": error })))
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "cloud approval staging failed" })),
        ),
    }
}

#[derive(Debug, Deserialize)]
struct CloudQueueBody {
    preview_hash: String,
    provider_role: String,
}

async fn cloud_queue_handler(
    Path((source, id)): Path<(String, String)>,
    Json(body): Json<CloudQueueBody>,
) -> (StatusCode, Json<Value>) {
    let result =
        tokio::task::spawn_blocking(move || -> Result<Option<CloudDerivativeState>, String> {
            let cfg = Config::load();
            let role = cfg
                .inference
                .roles_with_prefix("cloud_")
                .into_iter()
                .find_map(|(name, role)| (name == body.provider_role).then_some(role))
                .filter(|role| role.has_cloud_policy())
                .ok_or_else(|| {
                    "provider role is not a configured reviewed HTTPS cloud role".to_string()
                })?;
            if !role.credential_ready() {
                return Err("provider credential is not materialized".into());
            }

            let store = Store::open(&cfg.database_url).map_err(|error| error.to_string())?;
            let Some(item) = load_content_item(&store, &source, &id)? else {
                return Ok(None);
            };
            let preview = cloud_derivative::prepare(&item.cloud_input());
            if preview.preview_hash != body.preview_hash {
                return Err("approved derivative is stale; prepare and review it again".into());
            }
            if !cloud_tier_allows(
                role.cloud_data_tier.map(|tier| tier.as_str()),
                &preview.original_data_class,
                &preview.derivative_data_class,
                preview.transformation,
            ) {
                return Err("provider role does not allow this reviewed derivative".into());
            }
            let utc_date = store.utc_date().map_err(|error| error.to_string())?;
            if !role.billing_active_on(&utc_date) {
                return Err("provider billing policy is expired or inactive".into());
            }
            let input_upper_bound = cloud_dispatch::input_token_upper_bound(&preview.document);
            if input_upper_bound > role.max_input_tokens.unwrap_or(0) {
                return Err(
                    "provider input token ceiling is below this reviewed derivative".into(),
                );
            }
            let calls = store
                .cloud_provider_calls_today(&body.provider_role)
                .map_err(|error| error.to_string())?;
            if calls >= role.max_requests_per_day.unwrap_or(0) {
                return Err("provider daily request ceiling is reached".into());
            }
            store
                .queue_cloud_derivative(&CloudQueueRequest {
                    source: preview.source,
                    item_id: preview.id,
                    source_revision: preview.source_revision,
                    preview_hash: preview.preview_hash,
                    provider_role: body.provider_role,
                })
                .map(Some)
                .map_err(|error| error.to_string())
        })
        .await;

    match result {
        Ok(Ok(Some(state))) => (StatusCode::OK, Json(json!(state))),
        Ok(Ok(None)) => (StatusCode::NOT_FOUND, Json(json!({ "error": "not found" }))),
        Ok(Err(error))
            if error.starts_with("provider role") || error.starts_with("provider credential") =>
        {
            (StatusCode::BAD_REQUEST, Json(json!({ "error": error })))
        }
        Ok(Err(error)) if error.contains("stale") || error.starts_with("approved derivative") => {
            (StatusCode::CONFLICT, Json(json!({ "error": error })))
        }
        Ok(Err(error)) if error == "source must be 'feed' or 'mail'" => {
            (StatusCode::BAD_REQUEST, Json(json!({ "error": error })))
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "cloud queue failed" })),
        ),
    }
}

async fn cloud_run_handler(Path(job_id): Path<String>) -> (StatusCode, Json<Value>) {
    let result = tokio::task::spawn_blocking(move || -> Result<CloudDerivativeState, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_url).map_err(|error| error.to_string())?;
        let job = store
            .cloud_job_for_dispatch(&job_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| {
                "cloud job is completed, running, stale, or past its retry limit".to_string()
            })?;
        if job.task != cloud_dispatch::TASK_VERSION {
            return Err("cloud job task is unsupported".into());
        }
        let selected_role = cfg
            .inference
            .role(&job.provider_role)
            .filter(|role| role.has_cloud_policy())
            .ok_or_else(|| "provider role is no longer a reviewed HTTPS cloud role".to_string())?;
        if !cloud_tier_allows(
            selected_role.cloud_data_tier.map(|tier| tier.as_str()),
            &job.original_data_class,
            &job.derivative_data_class,
            &job.transformation,
        ) {
            return Err("provider role no longer allows the staged derivative".into());
        }
        let utc_date = store.utc_date().map_err(|error| error.to_string())?;
        let input_upper_bound = cloud_dispatch::input_token_upper_bound(&job.document);
        let mut requested = false;
        let mut outcomes = Vec::new();

        for (candidate_name, role) in cfg.inference.cloud_failover_roles(&job.provider_role) {
            if !cloud_tier_allows(
                role.cloud_data_tier.map(|tier| tier.as_str()),
                &job.original_data_class,
                &job.derivative_data_class,
                &job.transformation,
            ) {
                continue;
            }
            if !role.credential_ready() {
                outcomes.push(format!("{candidate_name}: credential unavailable"));
                continue;
            }
            if !role.billing_active_on(&utc_date) {
                outcomes.push(format!("{candidate_name}: billing policy inactive"));
                continue;
            }
            if input_upper_bound > role.max_input_tokens.unwrap_or(0) {
                outcomes.push(format!("{candidate_name}: input ceiling exceeded"));
                continue;
            }

            let attempt_id = match store
                .claim_cloud_job_attempt(
                    &job.job_id,
                    &candidate_name,
                    &role.model,
                    role.max_requests_per_day.unwrap_or(0),
                )
                .map_err(|error| error.to_string())?
            {
                CloudAttemptClaim::Started(attempt_id) => attempt_id,
                CloudAttemptClaim::DailyLimitReached => {
                    outcomes.push(format!("{candidate_name}: daily request ceiling reached"));
                    continue;
                }
                CloudAttemptClaim::JobUnavailable => {
                    return Err("cloud job was claimed by another request".into());
                }
            };
            requested = true;
            let analysis = match cloud_dispatch::analyze(&role, &job.document) {
                Ok(analysis) => analysis,
                Err(error) => {
                    store
                        .fail_cloud_job_attempt(&job.job_id, attempt_id, &error)
                        .map_err(|store_error| store_error.to_string())?;
                    outcomes.push(format!("{candidate_name}: {error}"));
                    continue;
                }
            };
            let result = serde_json::to_value(analysis).map_err(|error| error.to_string())?;
            if !store
                .complete_cloud_job_attempt(&job.job_id, attempt_id, &result)
                .map_err(|error| error.to_string())?
            {
                return Err("cloud job result could not be committed".into());
            }
            return store
                .cloud_derivative_state(
                    &job.source,
                    &job.item_id,
                    &job.source_revision,
                    &job.preview_hash,
                )
                .map_err(|error| error.to_string());
        }

        let detail = if outcomes.is_empty() {
            "no same-tier provider is configured".to_string()
        } else {
            outcomes.join("; ")
        };
        if requested {
            Err(format!("dispatch failed: {detail}"))
        } else {
            Err(format!("provider policy blocked dispatch: {detail}"))
        }
    })
    .await;

    match result {
        Ok(Ok(state)) => (StatusCode::OK, Json(json!(state))),
        Ok(Err(error)) if error.starts_with("provider role") => {
            (StatusCode::BAD_REQUEST, Json(json!({ "error": error })))
        }
        Ok(Err(error)) if error.starts_with("provider policy blocked") => {
            (StatusCode::BAD_REQUEST, Json(json!({ "error": error })))
        }
        Ok(Err(error)) if error.starts_with("dispatch failed:") => {
            (StatusCode::BAD_GATEWAY, Json(json!({ "error": error })))
        }
        Ok(Err(error)) if error.starts_with("cloud job") => {
            (StatusCode::CONFLICT, Json(json!({ "error": error })))
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "cloud job execution failed" })),
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
        let summary_producer_revision = media::summary_producer_revision(&cfg);
        let travel_context = travel::cached(&store);
        let travel_revision = travel_context
            .as_ref()
            .map(|context| context.revision.as_str())
            .unwrap_or_default();
        let summary = store
            .evaluation_summary()
            .map_err(|error| error.to_string())?;
        let enrichment = store
            .feed_enrichment_counts(summary_producer_revision.as_deref())
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
        let mut source_errors = 0usize;
        let selected_count = selected.len();
        for source in selected {
            // One source failing does not fail the run — the rule the extractor loop below
            // already applies to one URL, applied one level up where it was missing. The `?` that
            // was here meant a single upstream rate-limiting the collector took every other
            // source down with it: arXiv answered 429, and GitHub trending, which was fine, was
            // never asked. Invisible while a human clicked the button and retried; not invisible
            // once capabilities/feed-sweep runs this on a timer with nobody watching.
            let found = match sources::fetch(&source) {
                Ok(found) => found,
                Err(error) => {
                    let error = error.to_string();
                    eprintln!("source {}: {error}", source.id);
                    source_errors += 1;
                    // Reported per source rather than only logged, so a surface can say WHICH one
                    // is failing. `record_run` is deliberately not called: the source did not run,
                    // and moving its timestamp forward would make a source that has been dead for
                    // a week look like it was collected minutes ago.
                    results.push(json!({
                        "source_id": source.id,
                        "adapter": source.adapter,
                        "discovered": 0,
                        "fetched": 0,
                        "new_count": 0,
                        "failed": [],
                        "error": error,
                    }));
                    continue;
                }
            };
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
        if source_errors > 0 && source_errors == selected_count {
            // Nothing was collected at all. Reported as the error it is, with the per-source
            // reasons still in the log, rather than as a 200 carrying zeroes.
            return Err(format!(
                "every selected Feed source failed ({source_errors} of {selected_count}) — see the comms log for each"
            ));
        }
        Ok((
            json!({
                "sources": results,
                "fetched": results.iter().map(|result| result["fetched"].as_u64().unwrap_or(0)).sum::<u64>(),
                "new_count": results.iter().map(|result| result["new_count"].as_u64().unwrap_or(0)).sum::<u64>(),
                // Every source failing is still an error, and has to stay one: a caller that
                // treats 200 as "collection happened" would otherwise read a total outage as a
                // quiet day. It is the partial case that is now a success.
                "source_errors": source_errors,
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
        Some(
            items
                .into_iter()
                .map(|item| {
                    let relevance = store.triage_relevance(&item.id).unwrap_or_default();
                    TriageOut::from_store(item, relevance)
                })
                .collect(),
        )
    })
    .await
    .ok()
    .flatten();

    match result {
        Some(items) => Json(json!(items)),
        None => Json(json!({ "error": "triage query failed" })),
    }
}

#[derive(Debug, Deserialize)]
struct TriageSweepBody {
    limit: Option<usize>,
    cursor: Option<String>,
}

/// Counts from one sweep pass. No field here can carry mail content — this is
/// what both the HTTP response and the unattended schedule's log are built
/// from, and the schedule writes to a log nobody is watching at the time.
struct SweepOutcome {
    fetched: usize,
    new_count: usize,
    skipped: usize,
    redacted: usize,
    next_cursor: Option<String>,
}

/// The sweep itself, with no HTTP and no scheduling in it. Both the manual
/// route and the timer call this, for the same reason both go through
/// `intake`: two copies of a mail-reading loop is how one of them ends up
/// missing the gate.
fn run_inbox_sweep(
    cfg: &Config,
    limit: usize,
    cursor: Option<&str>,
) -> Result<SweepOutcome, String> {
    let store = Store::open(&cfg.database_url).map_err(|error| error.to_string())?;
    let token = google::access_token(&cfg.google_env_path).map_err(|error| error.to_string())?;
    let page = google::list_inbox_threads_page(&token, limit, cursor)
        .map_err(|error| error.to_string())?;
    let mut outcome = SweepOutcome {
        fetched: 0,
        new_count: 0,
        skipped: 0,
        redacted: 0,
        next_cursor: page.next_page_token.clone(),
    };
    for stub in &page.threads {
        let meta = match google::thread_meta(&token, &stub.id) {
            Ok(meta) => meta,
            Err(_) => {
                outcome.skipped += 1;
                continue;
            }
        };
        let intake = intake::from_thread(meta, &cfg.rules);
        if intake.redaction_count() > 0 {
            outcome.redacted += 1;
        }
        if store
            .upsert_triage(&intake.item)
            .map_err(|error| error.to_string())?
        {
            outcome.new_count += 1;
        }
        outcome.fetched += 1;
    }
    Ok(outcome)
}

/// Which bucket a failure falls in, for the stored state. Deliberately lossy:
/// the classes drive backoff and are safe to display, while the provider's own
/// message can quote a request URL or a subject line and is only ever logged.
fn sweep_error_class(error: &str) -> &'static str {
    let lowered = error.to_ascii_lowercase();
    if lowered.contains("401") || lowered.contains("403") || lowered.contains("auth") {
        "auth"
    } else if lowered.contains("429") || lowered.contains("quota") || lowered.contains("rate") {
        "quota"
    } else if lowered.contains("timeout") || lowered.contains("connect") || lowered.contains("dns")
    {
        "network"
    } else {
        "unknown"
    }
}

async fn triage_sweep_handler(Json(body): Json<TriageSweepBody>) -> (StatusCode, Json<Value>) {
    let limit = body.limit.unwrap_or(100).clamp(1, 100);
    let cursor = body.cursor.filter(|value| !value.trim().is_empty());
    let result = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let cfg = Config::load();
        let outcome = run_inbox_sweep(&cfg, limit, cursor.as_deref())?;
        let store = Store::open(&cfg.database_url).map_err(|error| error.to_string())?;
        let total_stored = store
            .list_triage(None)
            .map_err(|error| error.to_string())?
            .len();
        Ok(json!({
            "fetched": outcome.fetched,
            "new_count": outcome.new_count,
            "skipped": outcome.skipped,
            "redacted": outcome.redacted,
            "total_stored": total_stored,
            "next_cursor": outcome.next_cursor,
            "exhausted": outcome.next_cursor.is_none(),
        }))
    })
    .await;

    match result {
        Ok(Ok(value)) => (StatusCode::OK, Json(value)),
        Ok(Err(error)) => (StatusCode::BAD_GATEWAY, Json(json!({ "error": error }))),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        ),
    }
}

#[derive(Debug, Deserialize)]
struct TriageRelevanceBody {
    limit: Option<usize>,
}

fn loopback_inference_url(value: &str) -> bool {
    let value = value.trim().to_ascii_lowercase();
    value.starts_with("http://127.0.0.1:")
        || value.starts_with("https://127.0.0.1:")
        || value.starts_with("http://localhost:")
        || value.starts_with("https://localhost:")
        || value.starts_with("http://[::1]:")
        || value.starts_with("https://[::1]:")
}

async fn triage_relevance_handler(
    Json(body): Json<TriageRelevanceBody>,
) -> (StatusCode, Json<Value>) {
    let limit = body.limit.unwrap_or(200).clamp(1, 500);
    let result = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_url).map_err(|error| error.to_string())?;
        let profiles = relevance::load_profiles(&cfg.relevance);
        let triage = store
            .list_triage(None)
            .map_err(|error| error.to_string())?
            .into_iter()
            .filter(|item| item.status == "proposed" || item.status == "approved")
            .take(limit)
            .collect::<Vec<_>>();
        let items = triage
            .iter()
            .map(|proposal| {
                let mut item = FeedItem::new(
                    &format!("https://mail.google.com/mail/u/0/#all/{}", proposal.id),
                    "news",
                    "mail",
                );
                item.id = proposal.id.clone();
                item.title = proposal.subject.clone();
                item.author = proposal.from_addr.clone();
                item.transcript = proposal.snippet.clone();
                item
            })
            .collect::<Vec<_>>();
        let embedding_role = cfg
            .embedding_role()
            .filter(|role| loopback_inference_url(&role.backend.base_url));
        let reranking_role = cfg
            .reranking_role()
            .filter(|role| loopback_inference_url(&role.backend.base_url));
        let scored = relevance::score_items(
            &items,
            &profiles,
            embedding_role.as_ref(),
            reranking_role.as_ref(),
        );
        let mode = scored
            .iter()
            .flat_map(|item| item.matches.first())
            .map(|matched| matched.mode.clone())
            .next();
        for item in &scored {
            store
                .replace_triage_relevance(&item.feed_id, &item.matches)
                .map_err(|error| error.to_string())?;
        }
        Ok(json!({
            "scored": scored.len(),
            "profile_count": profiles.len(),
            "mode": mode,
            "local_only": true,
        }))
    })
    .await;

    match result {
        Ok(Ok(value)) => (StatusCode::OK, Json(value)),
        Ok(Err(error)) => (StatusCode::BAD_REQUEST, Json(json!({ "error": error }))),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        ),
    }
}

#[derive(Debug, Deserialize)]
struct TriageBulkBody {
    ids: Vec<String>,
    action: String,
    stream: Option<String>,
    data_class: Option<String>,
}

fn thread_action_for_job(job: &GmailActionJob) -> Result<ThreadAction, String> {
    match job.action.as_str() {
        "archive" => Ok(ThreadAction::Archive),
        "trash" => Ok(ThreadAction::Trash),
        "restore" if job.source_status == "archived" => Ok(ThreadAction::RestoreArchive),
        "restore" if job.source_status == "trashed" => Ok(ThreadAction::RestoreTrash),
        _ => Err("stored Gmail action has an invalid source state".into()),
    }
}

fn target_location(job: &GmailActionJob) -> Result<ThreadLocation, String> {
    match job.action.as_str() {
        "archive" => Ok(ThreadLocation::Archive),
        "trash" => Ok(ThreadLocation::Trash),
        "restore" => Ok(ThreadLocation::Inbox),
        _ => Err("stored Gmail action is invalid".into()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GmailActionOutcome {
    Confirmed { changed: bool },
    Missing,
}

/// Execute one durable intent. Reading the current labels first makes replay
/// safe when Gmail succeeded but the process stopped before the local commit.
fn execute_gmail_action_job(
    store: &Store,
    token: &str,
    job: &GmailActionJob,
) -> Result<GmailActionOutcome, String> {
    let meta = match google::thread_meta_lookup(token, &job.triage_id) {
        Ok(Some(meta)) => meta,
        Ok(None) => {
            return store
                .observe_gmail_missing(&job.triage_id)
                .map_err(|_| "Axon could not record that the Gmail thread is missing".to_string())
                .and_then(|updated| {
                    if updated {
                        Ok(GmailActionOutcome::Missing)
                    } else {
                        Err(
                            "mail proposal disappeared before the missing state was recorded"
                                .into(),
                        )
                    }
                });
        }
        Err(error) => {
            let message = error.to_string();
            let _ = store.fail_gmail_action(job.job_id, &message);
            return Err(message);
        }
    };
    let result = (|| -> Result<GmailActionOutcome, String> {
        let changed = ThreadLocation::from_labels(&meta.label_ids) != target_location(job)?;
        if changed {
            google::apply_thread_action(token, &job.triage_id, thread_action_for_job(job)?)
                .map_err(|error| error.to_string())?;
        }
        match store
            .complete_gmail_action(job.job_id)
            .map_err(|_| "Axon could not commit the confirmed Gmail result".to_string())?
        {
            true => Ok(GmailActionOutcome::Confirmed { changed }),
            false => Err("mail proposal disappeared before local completion".into()),
        }
    })();

    if let Err(error) = &result {
        let _ = store.fail_gmail_action(job.job_id, error);
    }
    result
}

#[derive(Debug, Serialize)]
struct GmailMaintenanceCounts {
    retried: usize,
    recovered: usize,
    retry_failures: usize,
    reconciled: usize,
    changed: usize,
    read_failures: usize,
    missing: usize,
    content_fetched: bool,
}

fn reconciled_status(current: &str, location: ThreadLocation) -> &str {
    match location {
        ThreadLocation::Trash => "trashed",
        ThreadLocation::Archive => "archived",
        ThreadLocation::Inbox if matches!(current, "archived" | "trashed" | "executed") => {
            "proposed"
        }
        ThreadLocation::Inbox => current,
    }
}

fn run_gmail_maintenance(
    cfg: &Config,
    reconcile_limit: i64,
) -> Result<GmailMaintenanceCounts, String> {
    let store = Store::open(&cfg.database_url).map_err(|error| error.to_string())?;
    let token = google::access_token(&cfg.google_env_path)
        .map_err(|_| "Google authorization unavailable".to_string())?;
    let jobs = store
        .pending_gmail_actions(50)
        .map_err(|error| error.to_string())?;
    let mut counts = GmailMaintenanceCounts {
        retried: jobs.len(),
        recovered: 0,
        retry_failures: 0,
        reconciled: 0,
        changed: 0,
        read_failures: 0,
        missing: 0,
        content_fetched: false,
    };
    for job in &jobs {
        match execute_gmail_action_job(&store, &token, job) {
            Ok(GmailActionOutcome::Confirmed { .. }) => counts.recovered += 1,
            Ok(GmailActionOutcome::Missing) => counts.missing += 1,
            Err(_) => counts.retry_failures += 1,
        }
    }

    let candidates = store
        .gmail_reconcile_candidates(reconcile_limit)
        .map_err(|error| error.to_string())?;
    for candidate in candidates {
        match google::thread_meta_lookup(&token, &candidate.triage_id) {
            Ok(Some(meta)) => {
                let location = ThreadLocation::from_labels(&meta.label_ids);
                if reconciled_status(&candidate.status, location) != candidate.status {
                    counts.changed += 1;
                }
                store
                    .observe_gmail_location(&candidate.triage_id, location.as_str())
                    .map_err(|error| error.to_string())?;
                counts.reconciled += 1;
            }
            Ok(None) => {
                store
                    .observe_gmail_missing(&candidate.triage_id)
                    .map_err(|error| error.to_string())?;
                counts.reconciled += 1;
                counts.changed += usize::from(candidate.status != "missing");
                counts.missing += 1;
            }
            Err(_) => counts.read_failures += 1,
        }
    }
    Ok(counts)
}

async fn triage_bulk_handler(Json(body): Json<TriageBulkBody>) -> (StatusCode, Json<Value>) {
    if body.ids.is_empty() || body.ids.len() > 100 {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "select between 1 and 100 proposals" })),
        );
    }
    let mut seen = HashSet::new();
    let ids = body
        .ids
        .into_iter()
        .filter(|id| seen.insert(id.clone()))
        .collect::<Vec<_>>();
    let action = body.action;
    let stream = body.stream;
    let selected_data_class = body.data_class;
    if !matches!(
        action.as_str(),
        "dismiss"
            | "categorize"
            | "set-data-class"
            | "archive"
            | "trash"
            | "waiting"
            | "clear-waiting"
    ) {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                json!({ "error": "action must be dismiss, categorize, set-data-class, archive, trash, waiting, or clear-waiting" }),
            ),
        );
    }
    if action == "categorize" && stream.as_deref().is_none() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "categorize requires a stream" })),
        );
    }
    if action == "set-data-class"
        && !selected_data_class
            .as_deref()
            .is_some_and(content_item::valid)
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "set-data-class requires public, personal, or vault" })),
        );
    }

    let result = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_url).map_err(|error| error.to_string())?;
        let gmail_action = ThreadAction::parse(&action);
        let waiting_action = matches!(action.as_str(), "waiting" | "clear-waiting");
        let token = (gmail_action.is_some() || waiting_action)
            .then(|| google::access_token(&cfg.google_env_path));
        // Resolved once for the batch, not once per thread: the label id is the
        // same for every id here, and looking it up a hundred times would spend
        // a hundred requests to learn the same string.
        let waiting_label = match (waiting_action, token.as_ref()) {
            (true, Some(Ok(token))) => Some(google::ensure_waiting_label(token)),
            _ => None,
        };
        let mut succeeded = Vec::new();
        let mut failures = Vec::new();
        for id in ids {
            let known = store
                .get_triage_status(&id)
                .map_err(|error| error.to_string())?;
            if known.is_none() {
                failures.push(json!({ "id": id, "error": "not found" }));
                continue;
            }
            let outcome = match action.as_str() {
                "dismiss" => store
                    .set_triage_status(&id, "dismissed")
                    .map_err(|error| error.to_string())
                    .map(|updated| updated.then_some(())),
                "categorize" => store
                    .set_triage_stream(&id, stream.as_deref().unwrap_or_default())
                    .map_err(|error| error.to_string())
                    .map(|updated| updated.then_some(())),
                "set-data-class" => store
                    .set_triage_data_class(
                        &id,
                        selected_data_class.as_deref().unwrap_or_default(),
                    )
                    .map_err(|error| error.to_string())
                    .map(|updated| updated.then_some(())),
                // Not queued the way archive and trash are, deliberately.
                // The queue exists for a mutation with a restore path and a
                // reconcile story; applying a label has neither. It is
                // idempotent in Gmail, carries no state to drift, and a failure
                // leaves nothing half-done — so a retry is just pressing it
                // again. Gmail first, store second: see `set_triage_waiting`.
                "waiting" | "clear-waiting" => {
                    let want = action == "waiting";
                    match (token.as_ref(), waiting_label.as_ref()) {
                        (Some(Ok(token)), Some(Ok(label))) => {
                            google::set_thread_waiting(token, &id, label, want)
                                .map_err(|error| error.to_string())
                                .and_then(|()| {
                                    store
                                        .set_triage_waiting(&id, want)
                                        .map_err(|error| error.to_string())
                                })
                                .map(|updated| updated.then_some(()))
                        }
                        (Some(Err(_)), _) => Err("Google authorization unavailable".to_string()),
                        (_, Some(Err(error))) => Err(error.to_string()),
                        _ => Err("Google authorization unavailable".to_string()),
                    }
                }
                "archive" | "trash" => store
                    .queue_gmail_action(&id, action.as_str())
                    .map_err(|error| error.to_string())
                    .and_then(|job| match token.as_ref() {
                        Some(Ok(token)) => execute_gmail_action_job(&store, token, &job),
                        Some(Err(_)) => {
                            let _ = store.fail_gmail_action(
                                job.job_id,
                                "Google authorization unavailable",
                            );
                            Err("Google authorization unavailable; the action remains queued for retry".into())
                        }
                        None => unreachable!(),
                    })
                    .and_then(|outcome| match outcome {
                        GmailActionOutcome::Confirmed { .. } => Ok(Some(())),
                        GmailActionOutcome::Missing => {
                            Err("Gmail thread no longer exists; Axon retained its local record".into())
                        }
                    }),
                _ => unreachable!(),
            };
            match outcome {
                Ok(Some(())) => succeeded.push(id),
                Ok(None) => failures.push(json!({ "id": id, "error": "not found" })),
                Err(error) => failures.push(json!({ "id": id, "error": error.to_string() })),
            }
        }
        Ok(json!({
            "succeeded": succeeded,
            "failures": failures,
            "gmail_changed": gmail_action.is_some(),
        }))
    })
    .await;

    match result {
        Ok(Ok(value)) => (StatusCode::OK, Json(value)),
        Ok(Err(error)) => (StatusCode::BAD_REQUEST, Json(json!({ "error": error }))),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        ),
    }
}

async fn triage_status_handler(
    Path(id): Path<String>,
    Json(body): Json<StatusBody>,
) -> (StatusCode, Json<Value>) {
    if !matches!(body.status.as_str(), "proposed" | "approved" | "dismissed") {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "status must be proposed, approved, or dismissed; Gmail lifecycle states require the Gmail action endpoint"
            })),
        );
    }
    let result = tokio::task::spawn_blocking(move || -> Result<bool, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_url).map_err(|error| error.to_string())?;
        store
            .set_triage_status(&id, &body.status)
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
struct TriageStreamBody {
    stream: String,
}

async fn triage_stream_handler(
    Path(id): Path<String>,
    Json(body): Json<TriageStreamBody>,
) -> (StatusCode, Json<Value>) {
    let result = tokio::task::spawn_blocking(move || -> Result<bool, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_url).map_err(|error| error.to_string())?;
        store
            .set_triage_stream(&id, &body.stream)
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
struct TriageDataClassBody {
    data_class: String,
}

async fn triage_data_class_handler(
    Path(id): Path<String>,
    Json(body): Json<TriageDataClassBody>,
) -> (StatusCode, Json<Value>) {
    let result = tokio::task::spawn_blocking(move || -> Result<bool, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_url).map_err(|error| error.to_string())?;
        store
            .set_triage_data_class(&id, &body.data_class)
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

/// Freshness and failure of the unattended schedule, for a dashboard to read.
/// Unauthenticated with the other reads: it carries counts, timestamps and an
/// error class, and no mail ever reaches it.
async fn triage_sweep_status_handler() -> (StatusCode, Json<Value>) {
    let result = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_url).map_err(|error| error.to_string())?;
        let state = store
            .get_source_state(INBOX_SWEEP_SOURCE)
            .map_err(|error| error.to_string())?;
        Ok(json!({
            "enabled": cfg.inbox_sweep_minutes > 0,
            "every_minutes": cfg.inbox_sweep_minutes,
            "max_threads": cfg.inbox_sweep_max_threads,
            "quiet_hours": cfg.inbox_sweep_quiet_hours.map(|(s, e)| json!({"start": s, "end": e})),
            "last_run_at": state.as_ref().map(|s| s.last_run_at.clone()),
            "last_success_at": state.as_ref().and_then(|s| s.last_success_at.clone()),
            "last_failure_at": state.as_ref().and_then(|s| s.last_failure_at.clone()),
            "last_error": state.as_ref().and_then(|s| s.last_error.clone()),
            "considered_count": state.as_ref().map(|s| s.considered_count).unwrap_or(0),
            "new_count": state.as_ref().map(|s| s.new_count).unwrap_or(0),
            "consecutive_failures": state.as_ref().map(|s| s.consecutive_failures).unwrap_or(0),
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

#[derive(Debug, Deserialize)]
struct TriageRedactBody {
    limit: Option<usize>,
    /// Report what would change without writing. The default is to write:
    /// this endpoint exists because material is already stored, and a preview
    /// that has to be run twice is a preview that gets run once.
    dry_run: Option<bool>,
}

/// Remediate rows persisted before the intake gate existed.
///
/// Bounded, idempotent, and reviewable: it reports how many rows it examined,
/// how many it changed and what kinds of entity it removed — never the removed
/// values, and never the values it left. Running it twice reports zero changes
/// the second time, which is how you know it finished.
async fn triage_redact_handler(Json(body): Json<TriageRedactBody>) -> (StatusCode, Json<Value>) {
    let limit = body.limit.unwrap_or(500).clamp(1, 2_000);
    let dry_run = body.dry_run.unwrap_or(false);
    let result = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_url).map_err(|error| error.to_string())?;
        let items = store
            .list_triage(None)
            .map_err(|error| error.to_string())?
            .into_iter()
            .take(limit)
            .collect::<Vec<_>>();
        let reviewed = items.len();
        let mut in_scope = 0usize;
        let mut changed = 0usize;
        let mut entity_types: BTreeMap<&'static str, usize> = BTreeMap::new();
        let mut digests = Vec::new();

        for item in items {
            let Some(remediation) = intake::remediate(
                &item.data_class,
                item.subject.as_deref(),
                item.snippet.as_deref(),
            ) else {
                continue;
            };
            in_scope += 1;
            if !remediation.changed {
                continue;
            }
            for finding in &remediation.redactions {
                *entity_types.entry(finding.entity_type).or_default() += finding.count;
            }
            if let Some(digest) = remediation.audit_digest.clone() {
                digests.push(json!({ "id": item.id, "digest": digest }));
            }
            if !dry_run
                && store
                    .redact_triage_review_fields(
                        &item.id,
                        remediation.subject.as_deref(),
                        remediation.snippet.as_deref(),
                    )
                    .map_err(|error| error.to_string())?
            {
                changed += 1;
            } else if dry_run {
                changed += 1;
            }
        }

        Ok(json!({
            "reviewed": reviewed,
            "in_scope": in_scope,
            "changed": changed,
            "dry_run": dry_run,
            "entity_types": entity_types,
            "audit": digests,
            "transformation": cloud_derivative::REDACTION_VERSION,
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

#[derive(Debug, Deserialize)]
struct TriageDataClassRefreshBody {
    limit: Option<usize>,
}

async fn triage_data_class_refresh_handler(
    Json(body): Json<TriageDataClassRefreshBody>,
) -> (StatusCode, Json<Value>) {
    let limit = body.limit.unwrap_or(500).clamp(1, 2_000);
    let result = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_url).map_err(|error| error.to_string())?;
        let items = store
            .list_triage(None)
            .map_err(|error| error.to_string())?
            .into_iter()
            .take(limit)
            .collect::<Vec<_>>();
        let reviewed = items.len();
        let mut updated = 0usize;
        let mut preserved_human = 0usize;
        for item in items {
            if item.data_classification_method == "human" {
                preserved_human += 1;
                continue;
            }
            let classification = DataClass::classify_mail(
                &item.stream,
                item.from_addr.as_deref().unwrap_or_default(),
                item.subject.as_deref().unwrap_or_default(),
            );
            if store
                .refresh_triage_data_class(&item.id, &classification)
                .map_err(|error| error.to_string())?
            {
                updated += 1;
            }
        }
        Ok(json!({
            "reviewed": reviewed,
            "updated": updated,
            "preserved_human": preserved_human,
            "classifier_version": content_item::MAIL_CLASSIFIER_VERSION,
            "content_inputs": ["sender", "subject", "category"],
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

#[derive(Debug, Deserialize)]
struct TriageGmailBody {
    action: String,
}

#[derive(Debug, Deserialize)]
struct TriageGmailJobBody {
    decision: String,
}

async fn triage_gmail_handler(
    Path(id): Path<String>,
    Json(body): Json<TriageGmailBody>,
) -> (StatusCode, Json<Value>) {
    if !matches!(body.action.as_str(), "archive" | "trash" | "restore") {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "action must be archive, trash, or restore" })),
        );
    }
    let action_name = body.action;
    let response_action = action_name.clone();
    let result = tokio::task::spawn_blocking(
        move || -> Result<GmailActionOutcome, (StatusCode, String)> {
            let cfg = Config::load();
            let store = Store::open(&cfg.database_url)
                .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
            let job = store
                .queue_gmail_action(&id, &action_name)
                .map_err(|error| {
                    let message = error.to_string();
                    let status = if message == "mail proposal not found" {
                        StatusCode::NOT_FOUND
                    } else {
                        StatusCode::CONFLICT
                    };
                    (status, message)
                })?;
            let token = google::access_token(&cfg.google_env_path).map_err(|_| {
                let _ = store.fail_gmail_action(job.job_id, "Google authorization unavailable");
                (
                    StatusCode::BAD_GATEWAY,
                    "Google authorization unavailable; the action remains queued for retry".into(),
                )
            })?;
            execute_gmail_action_job(&store, &token, &job).map_err(|error| {
                (
                    StatusCode::BAD_GATEWAY,
                    format!("{error}; the action remains queued for bounded retry"),
                )
            })
        },
    )
    .await;

    match result {
        Ok(Ok(GmailActionOutcome::Confirmed { changed })) => (
            StatusCode::OK,
            Json(json!({
                "ok": true,
                "action": response_action,
                "gmail_changed": changed,
                "gmail_confirmed": true
            })),
        ),
        Ok(Ok(GmailActionOutcome::Missing)) => (
            StatusCode::GONE,
            Json(json!({
                "error": "Gmail thread no longer exists; Axon retained its local record"
            })),
        ),
        Ok(Err((status, error))) => (status, Json(json!({ "error": error }))),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        ),
    }
}

async fn triage_gmail_job_handler(
    Path(id): Path<String>,
    Json(body): Json<TriageGmailJobBody>,
) -> (StatusCode, Json<Value>) {
    if !matches!(body.decision.as_str(), "retry" | "cancel") {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "decision must be retry or cancel" })),
        );
    }
    let decision = body.decision;
    let result = tokio::task::spawn_blocking(move || {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_url)
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
        if decision == "cancel" {
            return store
                .cancel_abandoned_gmail_action(&id)
                .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
                .and_then(|canceled| {
                    if canceled {
                        Ok((StatusCode::OK, json!({ "ok": true, "state": "canceled" })))
                    } else {
                        Err((
                            StatusCode::CONFLICT,
                            "no Gmail action needs operator attention".into(),
                        ))
                    }
                });
        }

        let job = store
            .retry_abandoned_gmail_action(&id)
            .map_err(|error| (StatusCode::CONFLICT, error.to_string()))?;
        let token = google::access_token(&cfg.google_env_path).map_err(|_| {
            let _ = store.fail_gmail_action(job.job_id, "Google authorization unavailable");
            (
                StatusCode::BAD_GATEWAY,
                "Google authorization unavailable; the action remains queued for retry".into(),
            )
        })?;
        match execute_gmail_action_job(&store, &token, &job).map_err(|error| {
            (
                StatusCode::BAD_GATEWAY,
                format!("{error}; the action remains queued for bounded retry"),
            )
        })? {
            GmailActionOutcome::Confirmed { changed } => Ok((
                StatusCode::OK,
                json!({ "ok": true, "state": "completed", "gmail_changed": changed }),
            )),
            GmailActionOutcome::Missing => Ok((
                StatusCode::GONE,
                json!({
                    "error": "Gmail thread no longer exists; Axon retained its local record"
                }),
            )),
        }
    })
    .await;

    match result {
        Ok(Ok((status, value))) => (status, Json(value)),
        Ok(Err((status, error))) => (status, Json(json!({ "error": error }))),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        ),
    }
}

async fn triage_reconcile_handler() -> (StatusCode, Json<Value>) {
    let result = tokio::task::spawn_blocking(|| {
        let cfg = Config::load();
        run_gmail_maintenance(&cfg, 200)
    })
    .await;
    match result {
        Ok(Ok(counts)) => (
            StatusCode::OK,
            Json(serde_json::to_value(counts).unwrap_or_else(|_| json!({}))),
        ),
        Ok(Err(error)) => (StatusCode::BAD_GATEWAY, Json(json!({ "error": error }))),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        ),
    }
}

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
                let summary_producer_revision = media::summary_producer_revision(&cfg);
                let before = match store
                    .feed_enrichment_counts(summary_producer_revision.as_deref())
                {
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

/// Retry feed digests that failed retryably, on an interval.
///
/// `Outcome::EmptyResponse` and its siblings have always been marked retryable
/// and the ledger has always counted attempts, but nothing ever performed the
/// retry: `digest::refresh_pending`'s only caller was an HTTP endpoint no client
/// invoked. A digest lost to a transient failure stayed lost. Two rows sat at
/// `empty_response`, attempt 1 of 3, after oMLX aborted them under memory
/// pressure and the abort arrived shaped like a successful empty answer (#95).
///
/// **Feed only, deliberately.** `refresh_pending` for `mail` reads message
/// bodies, and a background job that quietly pulls every body out of a mailbox
/// is not something a machine should start doing on its own — the same reason
/// that pass is bounded and explicit rather than timer-driven. Mail digests stay
/// a press.
///
/// Bounded by the ledger rather than by anything here: `items_needing_digest`
/// skips rows at the attempt cap or inside their backoff window, so a
/// permanently broken item stops costing model calls and says why.
fn spawn_digest_drain(every_minutes: u64) {
    if every_minutes == 0 {
        eprintln!("digest drain disabled (digest_drain_minutes = 0)");
        return;
    }

    eprintln!("digest drain: every {every_minutes} min, feed only");
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(every_minutes * 60));
        loop {
            ticker.tick().await;
            // Inspected rather than discarded, matching the enrichment drain: a
            // panic in the blocking half would otherwise take the drain down
            // silently, rebuilding the exact silence this exists to end.
            let joined = tokio::task::spawn_blocking(|| {
                let cfg = Config::load();
                let store = match Store::open(&cfg.database_url) {
                    Ok(store) => store,
                    Err(error) => {
                        eprintln!("digest drain: store unavailable: {error}");
                        return;
                    }
                };
                match digest::refresh_pending(&store, &cfg, "feed", 25) {
                    Ok(n) if n > 0 => eprintln!("digest drain: wrote {n} digest row(s)"),
                    Ok(_) => {}
                    Err(error) => eprintln!("digest drain: {error}"),
                }
            })
            .await;

            if let Err(error) = joined {
                eprintln!("digest drain: pass did not finish: {error}");
            }
        }
    });
}

/// Expired Trash rows contain cached Gmail metadata and reviewed derivatives,
/// so cleanup runs independently of inbox sweeps. Gmail's own retention is not
/// modified here.
fn spawn_trash_cleanup() {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(60 * 60));
        loop {
            ticker.tick().await;
            let joined = tokio::task::spawn_blocking(|| {
                let cfg = Config::load();
                let store = Store::open(&cfg.database_url)
                    .map_err(|error| format!("store unavailable: {error}"))?;
                store
                    .purge_expired_trashed()
                    .map_err(|error| error.to_string())
            })
            .await;
            match joined {
                Ok(Ok(count)) if count > 0 => {
                    eprintln!("mail trash cleanup: purged {count} expired item(s)")
                }
                Ok(Ok(_)) => {}
                Ok(Err(error)) => eprintln!("mail trash cleanup: {error}"),
                Err(error) => eprintln!("mail trash cleanup: pass did not finish: {error}"),
            }
        }
    });
}

/// The stored name this schedule keeps its run state under. Matches the
/// existing `source_state` convention rather than inventing a second one.
const INBOX_SWEEP_SOURCE: &str = "gmail-inbox";

/// Unattended inbox collection. Off unless the overlay turns it on, bounded to
/// the newest N threads, and silent during quiet hours.
///
/// No persisted cursor, on purpose. A cursor that advances each pass walks
/// backwards through the entire mailbox over days, which is precisely the
/// unbounded rescan this is supposed to avoid; re-reading the newest page
/// instead is idempotent because proposals upsert on Gmail thread id and
/// preserve human decisions. Paging deeper stays a manual, cursor-carrying
/// call from the board.
fn spawn_inbox_sweep(every_minutes: u64, max_threads: usize, quiet: Option<(u32, u32)>) {
    if every_minutes == 0 {
        eprintln!("Inbox sweep schedule disabled (inbox_sweep_minutes = 0)");
        return;
    }
    match quiet {
        Some((start, end)) => eprintln!(
            "Inbox sweep: every {every_minutes} min, newest {max_threads}, quiet {start:02}:00-{end:02}:00"
        ),
        None => eprintln!("Inbox sweep: every {every_minutes} min, newest {max_threads}"),
    }
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(every_minutes * 60));
        loop {
            ticker.tick().await;
            let joined = tokio::task::spawn_blocking(move || -> Result<Option<String>, String> {
                let cfg = Config::load();
                if !cfg.google_env_path.is_file() {
                    return Ok(None);
                }
                let store = Store::open(&cfg.database_url).map_err(|error| error.to_string())?;

                if let Some((start, end)) = quiet {
                    if store
                        .within_quiet_hours(start, end)
                        .map_err(|error| error.to_string())?
                    {
                        return Ok(None);
                    }
                }

                // Backoff is expressed as skipped ticks rather than a sleep, so
                // a recovered connector is picked up on the next ordinary tick
                // instead of after whatever long sleep was already running.
                let state = store
                    .get_source_state(INBOX_SWEEP_SOURCE)
                    .map_err(|error| error.to_string())?;
                let failures = state.map(|s| s.consecutive_failures).unwrap_or(0);
                if failures > 0 {
                    let skip_ticks = 1i64 << failures.min(5);
                    let elapsed_ticks = TICKS_SINCE_START.fetch_add(1, Ordering::Relaxed) as i64;
                    if elapsed_ticks % skip_ticks != 0 {
                        return Ok(None);
                    }
                }

                match run_inbox_sweep(&cfg, max_threads, None) {
                    Ok(outcome) => {
                        store
                            .record_sweep_success(
                                INBOX_SWEEP_SOURCE,
                                outcome.fetched as i64,
                                outcome.new_count as i64,
                            )
                            .map_err(|error| error.to_string())?;
                        Ok(Some(format!(
                            "{} considered, {} new, {} redacted, {} skipped",
                            outcome.fetched, outcome.new_count, outcome.redacted, outcome.skipped
                        )))
                    }
                    Err(error) => {
                        let class = sweep_error_class(&error);
                        let streak = store
                            .record_sweep_failure(INBOX_SWEEP_SOURCE, class)
                            .map_err(|error| error.to_string())?;
                        Err(format!("{class} error, {streak} in a row"))
                    }
                }
            })
            .await;
            match joined {
                Ok(Ok(Some(summary))) => eprintln!("Inbox sweep: {summary}"),
                Ok(Ok(None)) => {}
                Ok(Err(error)) => eprintln!("Inbox sweep: {error}"),
                Err(error) => eprintln!("Inbox sweep: pass did not finish: {error}"),
            }
        }
    });
}

/// Ticks since boot, so the backoff above can skip them without sleeping.
static TICKS_SINCE_START: AtomicU64 = AtomicU64::new(0);

fn spawn_gmail_maintenance(every_minutes: u64) {
    if every_minutes == 0 {
        eprintln!("Gmail maintenance disabled (gmail_maintenance_minutes = 0)");
        return;
    }
    eprintln!("Gmail maintenance: every {every_minutes} min");
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(every_minutes * 60));
        loop {
            ticker.tick().await;
            let joined = tokio::task::spawn_blocking(|| {
                let cfg = Config::load();
                if !cfg.google_env_path.is_file() {
                    return Ok(None);
                }
                run_gmail_maintenance(&cfg, 200).map(Some)
            })
            .await;
            match joined {
                Ok(Ok(Some(counts)))
                    if counts.recovered > 0 || counts.changed > 0 || counts.retry_failures > 0 =>
                {
                    eprintln!(
                        "Gmail maintenance: {} recovered, {} reconciled changes, {} retry failures",
                        counts.recovered, counts.changed, counts.retry_failures
                    )
                }
                Ok(Ok(_)) => {}
                Ok(Err(error)) => eprintln!("Gmail maintenance: {error}"),
                Err(error) => eprintln!("Gmail maintenance: pass did not finish: {error}"),
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
        .route("/routes", get(routes))
        .route("/health", get(health_handler))
        .route("/ready", get(ready_handler))
        .route("/feed", get(feed_handler))
        .route("/feed/origins", get(feed_origins_handler))
        .route("/feed/runs", get(feed_runs_handler))
        .route("/feed/evaluation/status", get(evaluation_status_handler))
        .route("/feed/quality", get(quality_queue_handler))
        .route("/feed/:id", get(feed_item_handler))
        .route("/content/cloud-providers", get(cloud_providers_handler))
        .route("/content/:source/:id", get(content_item_handler))
        .route("/sources", get(sources_handler))
        .route("/triage", get(triage_handler))
        .route("/triage/sweep/status", get(triage_sweep_status_handler));

    // Mutating routes: require shared secret.
    let write_routes = Router::new()
        .route("/content/:source/:id/digest", post(digest_handler))
        .route("/content/:source/:id/diagram", post(diagram_handler))
        .route("/content/:source/:id/chart", post(chart_handler))
        .route("/content/digests/refresh", post(digest_refresh_handler))
        .route(
            "/content/:source/:id/cloud-preview",
            post(cloud_preview_handler),
        )
        .route(
            "/content/:source/:id/cloud-approval",
            post(cloud_approval_handler),
        )
        .route(
            "/content/:source/:id/cloud-queue",
            post(cloud_queue_handler),
        )
        .route("/content/cloud-jobs/:job_id/run", post(cloud_run_handler))
        .route("/feed/relevance/refresh", post(relevance_refresh_handler))
        .route("/feed/quality/refresh", post(quality_refresh_handler))
        .route("/feed/:id/status", post(feed_status_handler))
        .route("/triage/sweep", post(triage_sweep_handler))
        .route("/triage/relevance/refresh", post(triage_relevance_handler))
        .route("/triage/redact", post(triage_redact_handler))
        .route(
            "/triage/data-class/refresh",
            post(triage_data_class_refresh_handler),
        )
        .route("/triage/bulk", post(triage_bulk_handler))
        .route("/triage/:id/status", post(triage_status_handler))
        .route("/triage/:id/stream", post(triage_stream_handler))
        .route("/triage/:id/data-class", post(triage_data_class_handler))
        .route("/triage/:id/gmail", post(triage_gmail_handler))
        .route("/triage/:id/gmail-job", post(triage_gmail_job_handler))
        .route("/triage/reconcile", post(triage_reconcile_handler))
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
    spawn_digest_drain(cfg.digest_drain_minutes);
    spawn_trash_cleanup();
    spawn_gmail_maintenance(cfg.gmail_maintenance_minutes);
    spawn_inbox_sweep(
        cfg.inbox_sweep_minutes,
        cfg.inbox_sweep_max_threads,
        cfg.inbox_sweep_quiet_hours,
    );

    let app = build_router(api_secret, &cfg.dashboard_origin);

    // Bind and the exit-on-failure behaviour live in axon_server now; this file
    // used to hand-roll the same five lines with an unwrap panic instead.
    axon_server::serve_local("comms-server", cfg.port, app).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Backoff and the stored error class both key off this, and both are
    /// wrong in a way nobody notices if it silently answers "unknown" — an
    /// expired refresh token would then retry at full cadence forever.
    #[test]
    fn sweep_errors_land_in_the_class_that_drives_backoff() {
        assert_eq!(sweep_error_class("HTTP 401 Unauthorized"), "auth");
        assert_eq!(sweep_error_class("token refresh failed: auth"), "auth");
        assert_eq!(sweep_error_class("HTTP 429 Too Many Requests"), "quota");
        assert_eq!(sweep_error_class("userRateLimitExceeded"), "quota");
        assert_eq!(sweep_error_class("connect timeout"), "network");
        assert_eq!(sweep_error_class("dns failure"), "network");
        assert_eq!(sweep_error_class("something else entirely"), "unknown");
    }

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
    async fn every_sensitive_content_write_requires_authentication() {
        let base = serve(Some("s3cret")).await;
        let client = reqwest::Client::new();
        for (path, body) in [
            ("/content/mail/18f17d0a9bc123ef/cloud-preview", json!({})),
            (
                "/content/mail/18f17d0a9bc123ef/cloud-approval",
                json!({ "preview_hash": "reviewed-hash" }),
            ),
            (
                "/content/mail/18f17d0a9bc123ef/cloud-queue",
                json!({
                    "preview_hash": "reviewed-hash",
                    "provider_role": "cloud_summarization"
                }),
            ),
            ("/content/cloud-jobs/cloud-job-123/run", json!({})),
            (
                "/triage/18f17d0a9bc123ef/status",
                json!({ "status": "dismissed" }),
            ),
            (
                "/triage/18f17d0a9bc123ef/stream",
                json!({ "stream": "feed" }),
            ),
            (
                "/triage/18f17d0a9bc123ef/gmail",
                json!({ "action": "trash" }),
            ),
            (
                "/triage/18f17d0a9bc123ef/gmail-job",
                json!({ "decision": "retry" }),
            ),
            ("/triage/sweep", json!({ "limit": 100 })),
            ("/triage/relevance/refresh", json!({ "limit": 200 })),
            ("/triage/data-class/refresh", json!({ "limit": 500 })),
            ("/triage/reconcile", json!({})),
            (
                "/triage/18f17d0a9bc123ef/data-class",
                json!({ "data_class": "vault" }),
            ),
            (
                "/triage/bulk",
                json!({ "ids": ["18f17d0a9bc123ef"], "action": "dismiss" }),
            ),
        ] {
            let response = client
                .post(format!("{base}{path}"))
                .json(&body)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 401, "{path} must stay behind auth");
        }
    }

    #[test]
    fn mail_relevance_accepts_only_loopback_model_endpoints() {
        assert!(loopback_inference_url("http://127.0.0.1:8000/v1"));
        assert!(loopback_inference_url("http://localhost:11434"));
        assert!(loopback_inference_url("http://[::1]:8000/v1"));
        assert!(!loopback_inference_url("https://api.example.com/v1"));
    }

    #[test]
    fn cloud_tiers_accept_only_the_exact_reviewed_representation() {
        assert!(cloud_tier_allows(
            Some("public"),
            "public",
            "public",
            cloud_derivative::PASSTHROUGH_VERSION,
        ));
        assert!(!cloud_tier_allows(
            Some("public"),
            "personal",
            "personal",
            cloud_derivative::REDACTION_VERSION,
        ));
        assert!(cloud_tier_allows(
            Some("pseudonymized_personal"),
            "personal",
            "personal",
            cloud_derivative::REDACTION_VERSION,
        ));
        assert!(!cloud_tier_allows(
            Some("pseudonymized_personal"),
            "personal",
            "personal",
            cloud_derivative::PASSTHROUGH_VERSION,
        ));
        assert!(!cloud_tier_allows(
            Some("pseudonymized_personal"),
            "vault",
            "personal",
            cloud_derivative::REDACTION_VERSION,
        ));
    }

    #[test]
    fn reconciled_gmail_location_maps_to_truthful_local_status() {
        assert_eq!(
            reconciled_status("proposed", ThreadLocation::Archive),
            "archived"
        );
        assert_eq!(
            reconciled_status("archived", ThreadLocation::Inbox),
            "proposed"
        );
        assert_eq!(
            reconciled_status("approved", ThreadLocation::Inbox),
            "approved"
        );
        assert_eq!(
            reconciled_status("archived", ThreadLocation::Trash),
            "trashed"
        );
    }

    #[test]
    fn mail_adapts_to_the_versioned_content_reader_contract() {
        let item = TriageItem {
            id: "thread-1".into(),
            from_addr: Some("sender@example.com".into()),
            subject: Some("A useful subject".into()),
            snippet: Some("A bounded Gmail preview.".into()),
            internal_date_ms: None,
            internal_date_text: Some("2026-08-04 09:30:00+02".into()),
            stream: "aktiv".into(),
            rationale: "Safe fallback.".into(),
            classification_method: "rules".into(),
            classification_version: "mail-rules-v1".into(),
            data_class: "personal".into(),
            data_class_rationale: "Mail metadata is Personal by default.".into(),
            data_classification_method: "rules".into(),
            data_classification_version: "data-class-rules-v1".into(),
            status: "proposed".into(),
            gmail_action: None,
            gmail_action_at: None,
            purge_after: None,
            gmail_location: None,
            gmail_observed_at: None,
            gmail_sync_status: None,
            gmail_sync_action: None,
            gmail_sync_error: None,
            waiting: false,
            waiting_since: None,
            first_seen: "2026-08-04 09:31:00+02".into(),
            last_seen: "2026-08-04 09:31:00+02".into(),
        };

        let value = serde_json::to_value(ContentItemOut::from_mail(item, Vec::new())).unwrap();
        assert_eq!(value["schema_version"], "content-item-v1");
        assert_eq!(value["source"], "mail");
        assert_eq!(value["kind"], "mail");
        assert_eq!(value["content_label"], "Message preview");
        assert_eq!(value["content_status"], "thin");
        assert_eq!(value["data_class"]["value"], "personal");
        assert_eq!(
            value["processing_policy"]["cloud_handling"],
            "pseudonymization_required"
        );
        assert_eq!(value["cloud_processing"]["status"], "not_prepared");
        assert_eq!(value["cloud_processing"]["provider_calls"], 0);
        assert_eq!(value["mail"]["category"], "aktiv");
        assert!(value["mail"]["gmail_location"].is_null());
        assert!(value["mail"]["gmail_sync_status"].is_null());
        assert!(value["evaluation"].is_null());
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
