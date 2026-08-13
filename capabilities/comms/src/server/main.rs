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
mod auth;
mod background;
mod cloud;
mod content;
mod contracts;
mod error;
mod feed;
mod source_handlers;
mod triage;
mod vault;

use auth::*;
use background::*;
use cloud::*;
use content::*;
use contracts::*;
use error::*;
use feed::*;
use source_handlers::*;
use triage::*;
use vault::*;

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
        "/feed/:id/data-class",
        "Set a feed item's data classification by hand. Lowering one needs a rationale.",
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
    route_manifest::get(method, path, summary)
}

async fn routes() -> Json<Value> {
    Json(route_manifest::manifest("comms", ROUTES))
}

/// Assemble the public HTTP surface and its authentication boundary.
///
/// Kept separate from `main` so tests can exercise the real middleware stack
/// over an ephemeral loopback listener.
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
        .route("/feed/:id/data-class", post(feed_data_class_handler))
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

    let _background_services = BackgroundServices::start(&cfg);

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
            (
                "/feed/0123456789abcdef/data-class",
                json!({ "data_class": "public" }),
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

    /// The de-escalation door answers 400 — not 404, not 500 — and says what it
    /// refused on. The class is checked before the item is looked up, so this
    /// reaches the refusal without a stored row being involved in the answer.
    #[tokio::test]
    async fn a_reclassification_the_rule_refuses_comes_back_as_a_bad_request() {
        let base = serve(Some("s3cret")).await;
        let response = reqwest::Client::new()
            .post(format!("{base}/feed/0123456789abcdef/data-class"))
            .header("x-axon-token", "s3cret")
            .json(&json!({ "data_class": "confidential" }))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), 400);
        let body: Value = response.json().await.unwrap();
        assert!(
            body["error"]
                .as_str()
                .unwrap_or_default()
                .contains("public, personal, vault"),
            "the refusal names the vocabulary, got: {body}"
        );
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
            classification_method: content_item::METHOD_DETERMINISTIC.into(),
            classification_version: "mail-rules-v1".into(),
            data_class: "personal".into(),
            data_class_rationale: "Mail metadata is Personal by default.".into(),
            data_classification_method: content_item::METHOD_DETERMINISTIC.into(),
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
        let missing = route_manifest::undeclared_routes(include_str!("main.rs"), super::ROUTES);
        assert!(missing.is_empty(), "served but undocumented: {missing:?}");
    }
}
