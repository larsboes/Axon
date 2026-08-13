//! Postgres-backed persistence (sync `postgres` client -- matches reqwest's
//! "blocking" feature; this crate carries no async runtime outside comms-server).
//! Own schema (`comms`) inside the one shared local instance
//! (`capabilities/postgres`), same discipline as scouting's store.rs.
//!
//! The status-preserving upsert is the load-bearing correctness property on
//! both tables: a human's triage/keeper decision must survive the same item
//! being re-swept or re-ingested. `status` is set only on first INSERT and is
//! deliberately absent from the ON CONFLICT DO UPDATE SET list. See
//! `upsert_preserves_status_across_refetch_*` for the proof.
//!
//! [`Store`] remains the only connection-owning facade. Its inherent methods
//! are grouped under `store/` by the workflow that owns their SQL: migrations,
//! triage/Gmail, cloud/digests, feed, evaluation, capture origins, source run
//! state, and row mapping. Callers therefore keep one stable type without one
//! file becoming the owner of every persistence concern.

use postgres::types::ToSql;
use postgres::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::evaluation::{EvaluationFactor, EvaluationFactorContext, FeedEvaluation};
use crate::provenance::{self, StageProvenance};
use crate::quality::QualityFlag;
use crate::relevance::RelevanceMatch;

pub struct Store {
    /// Shared with every other `Store` in this process on the same database. A
    /// `Store` is now a cheap handle rather than a connection, which is what makes
    /// 43 handlers each opening one acceptable.
    pool: axon_store::Pool,
    schema: String,
}

/// A media/news feed item. On write, `day`/`created_at`/`status` are owned by
/// the DB (CURRENT_DATE / now() / default 'new') and ignored; on read they are
/// populated. `transcript` is None in list views and Some in single-item reads.
#[derive(Debug, Clone)]
pub struct FeedItem {
    pub id: String,
    pub stream: String,
    pub kind: String,
    pub title: Option<String>,
    pub url: String,
    pub author: Option<String>,
    pub summary: Option<String>,
    pub transcript: Option<String>,
    pub day: String,
    pub created_at: String,
    pub status: String,
    /// Extraction quality: `full` (≥1k chars), `thin` (abstract/card), `none`
    /// (no transcript extracted), `unknown` (legacy, not yet classified).
    pub content_status: String,
    /// What the stored text IS: `full-text` (the document), `abstract` (a
    /// stand-in the source offered instead), `unknown` (legacy). A separate
    /// axis from `content_status`, which answers how much text there is — a
    /// long abstract is `full`/`abstract`, a one-line article `thin`/`full-text`.
    ///
    /// Queryable on purpose (#78): an `Abstract:` prefix inside the text would
    /// have to be parsed back out by every reader, and would be indexed by the
    /// embedder as if the paper had said it.
    pub transcript_source: String,
    /// How many times summarization was attempted and failed.
    pub summary_attempts: i32,
    /// Error class of the last failed summarization attempt, if any.
    pub summary_last_error: Option<String>,
    /// Earliest time the next summarization retry is allowed (exponential
    /// backoff). `None` means immediately eligible.
    pub summary_next_attempt: Option<String>,
    /// Which client handed this content over, or `None` when the server fetched
    /// it itself (#81). A login-gated page and a page the server could have
    /// fetched are otherwise indistinguishable once stored, which matters for
    /// judging why an item's content looks the way it does.
    pub captured_via: Option<String>,
    /// What the extractor emitted, before normalization (#86). Carried on the
    /// item only between `media::fetch` and `upsert_feed`; it lives in its own
    /// table, so reading an item back leaves this `None` unless the caller asks
    /// for it via `get_raw_content`. Retaining it is what lets a normalization
    /// rule change re-run over stored content instead of re-fetching the web.
    pub raw_content: Option<String>,
    pub summary_provenance: Option<StageProvenance>,
    /// What this item is worth protecting: `public`, `personal` or `vault`.
    /// Never absent — an item nobody classified reads back Personal, decided by
    /// `legacy`, which is the value the cloud gate is meant to see.
    pub data_class: String,
    pub data_class_rationale: String,
    pub data_classification_method: String,
    pub data_classification_version: String,
}

impl FeedItem {
    /// Build a fresh item for ingest. DB-owned fields are left blank/defaulted,
    /// and the class starts undeclared — Personal, method `legacy`.
    ///
    /// Every ingest path goes through here, which is what makes the default
    /// actually default: a page pasted by hand, a link imported from the vault
    /// and a URL captured from a logged-in session all arrive Personal unless
    /// something positively declares otherwise. Only a collector that declares
    /// a class calls [`FeedItem::declare_class`] on top.
    pub fn new(url: &str, stream: &str, kind: &str) -> Self {
        let undeclared = crate::content_item::DataClass::undeclared();
        Self {
            id: feed_id(url),
            stream: stream.to_string(),
            kind: kind.to_string(),
            title: None,
            url: url.to_string(),
            author: None,
            summary: None,
            transcript: None,
            day: String::new(),
            created_at: String::new(),
            status: "new".into(),
            content_status: "unknown".into(),
            transcript_source: "unknown".into(),
            summary_attempts: 0,
            summary_last_error: None,
            summary_next_attempt: None,
            captured_via: None,
            raw_content: None,
            summary_provenance: None,
            data_class: undeclared.value,
            data_class_rationale: undeclared.rationale,
            data_classification_method: undeclared.method,
            data_classification_version: undeclared.version,
        }
    }

    /// Stamp a collector's declaration onto an item it discovered.
    pub fn declare_class(&mut self, classification: &crate::content_item::DataClass) {
        self.data_class = classification.value.clone();
        self.data_class_rationale = classification.rationale.clone();
        self.data_classification_method = classification.method.clone();
        self.data_classification_version = classification.version.clone();
    }
}

/// Enrichment backlog counts for the evaluation status endpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct EnrichmentCounts {
    pub pending_summaries: i64,
    pub failed_summaries: i64,
}

/// Content status distribution across all feed items.
#[derive(Debug, Clone, PartialEq)]
pub struct ContentStatusCounts {
    pub full: i64,
    pub thin: i64,
    pub none: i64,
    pub unknown: i64,
}

/// One persisted signal in the Feed review queue, joined with the item facts a
/// reviewer needs. Reasons and evidence are stored rather than reconstructed by
/// the dashboard, so the UI cannot drift from the computation that fired.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct QualityReviewRow {
    pub feed_id: String,
    pub title: Option<String>,
    pub url: String,
    pub status: String,
    pub content_status: String,
    pub signal: String,
    pub reason: String,
    pub evidence: String,
    pub derived_at: String,
}

/// A triage proposal for one inbox thread. On write `status`/`first_seen`/
/// `last_seen` are DB-owned and ignored; on read they are populated.
#[derive(Debug, Clone)]
pub struct TriageItem {
    pub id: String,
    pub from_addr: Option<String>,
    pub subject: Option<String>,
    pub snippet: Option<String>,
    /// Gmail internalDate in epoch milliseconds (latest message). Write-side
    /// input; on read this stays None and `internal_date_text` carries the
    /// stored TIMESTAMPTZ instead.
    pub internal_date_ms: Option<i64>,
    /// Read-side text form of the stored `internal_date` TIMESTAMPTZ. None on
    /// write. Surfaced by the server as `internal_date`.
    pub internal_date_text: Option<String>,
    pub stream: String,
    pub rationale: String,
    /// `rules` for the deterministic sweep, `human` after an explicit
    /// dashboard correction. Human corrections survive later sweeps.
    pub classification_method: String,
    pub classification_version: String,
    /// Shared trust class. `vault` is presented as Private in the product.
    pub data_class: String,
    pub data_class_rationale: String,
    pub data_classification_method: String,
    pub data_classification_version: String,
    pub status: String,
    /// Last Gmail lifecycle action recorded after the Gmail request succeeded.
    pub gmail_action: Option<String>,
    pub gmail_action_at: Option<String>,
    /// Only trashed items have a purge deadline. Archived items remain in Axon.
    pub purge_after: Option<String>,
    /// Last observed Gmail label location, kept separate from an Axon-requested
    /// action so direct Gmail changes are not misattributed.
    pub gmail_location: Option<String>,
    pub gmail_observed_at: Option<String>,
    pub gmail_sync_status: Option<String>,
    /// Action owned by the current queued or attention job, if one exists.
    pub gmail_sync_action: Option<String>,
    pub gmail_sync_error: Option<String>,
    /// The doctrine's one state label, mirrored from Gmail so the board can rank
    /// and render it without asking Gmail per row. Gmail stays authoritative: this
    /// is written only after its modify call succeeds.
    pub waiting: bool,
    /// Only meaningful while `waiting` is true, and cleared with it. "Blocked since"
    /// is the question a Waiting list is actually asked.
    pub waiting_since: Option<String>,
    pub first_seen: String,
    pub last_seen: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GmailActionJob {
    pub job_id: i64,
    pub triage_id: String,
    pub action: String,
    pub source_status: String,
    pub attempts: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GmailReconcileCandidate {
    pub triage_id: String,
    pub status: String,
}

/// A reviewed, bounded derivative staged locally before queueing.
/// The original content is never stored here and staging never dispatches it.
#[derive(Debug, Clone)]
pub struct CloudDerivativeApproval {
    pub source: String,
    pub item_id: String,
    pub source_revision: String,
    pub preview_hash: String,
    pub original_data_class: String,
    pub derivative_data_class: String,
    pub transformation: String,
    pub document: String,
    pub redaction_count: i32,
}

#[derive(Debug, Clone)]
pub struct CloudQueueRequest {
    pub source: String,
    pub item_id: String,
    pub source_revision: String,
    pub preview_hash: String,
    pub provider_role: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CloudDispatchJob {
    pub job_id: String,
    pub source: String,
    pub item_id: String,
    pub source_revision: String,
    pub preview_hash: String,
    pub provider_role: String,
    pub task: String,
    pub original_data_class: String,
    pub derivative_data_class: String,
    pub transformation: String,
    pub document: String,
    pub provider_calls: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudAttemptClaim {
    Started(i64),
    DailyLimitReached,
    JobUnavailable,
}

/// The digest states a later run could plausibly do better on — the storage
/// mirror of `summarize::Outcome::retryable`.
///
/// One list because two statements key on it: the query that finds work and the
/// upsert that decides whether to arm a backoff. Two hand-maintained copies is
/// how a state ends up retryable in one and terminal in the other, which strands
/// rows in a way nothing reports.
pub const RETRYABLE_DIGEST_STATES: [&str; 5] = [
    "http_error",
    "model_error",
    "capacity_aborted",
    "empty_response",
    "timeout",
];

/// The states above, as a SQL literal list. Built from the const rather
/// than typed out, so the two cannot drift. Every element is a compile-time
/// literal, so this never carries caller input.
fn retryable_digest_states_sql() -> String {
    RETRYABLE_DIGEST_STATES
        .iter()
        .map(|state| format!("'{state}'"))
        .collect::<Vec<_>>()
        .join(",")
}

/// One row of `content_digests`, as stored.
///
/// The wire shape is `content_item::Digest`; this is the database's view of the
/// same thing, kept separate so a column type change does not reach the reader
/// contract by accident. `focus` is comma-joined here and split for the wire —
/// it is display state read back into a text field, not something anything
/// queries by.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredDigest {
    pub source: String,
    pub item_id: String,
    pub text: Option<String>,
    pub state: String,
    pub shape: String,
    pub depth: String,
    pub focus: String,
    pub producer: String,
    pub source_chars: i64,
    pub redactions: i32,
    pub attempts: i32,
    pub last_error: Option<String>,
    pub diagram: Option<String>,
    pub diagram_state: Option<String>,
    pub diagram_error: Option<String>,
    pub chart: Option<String>,
    pub chart_state: Option<String>,
    pub chart_error: Option<String>,
    pub generated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CloudDerivativeState {
    pub status: String,
    pub preview_hash: Option<String>,
    pub approved_at: Option<String>,
    pub dispatch_status: String,
    pub job_id: Option<String>,
    pub provider_role: Option<String>,
    pub queued_at: Option<String>,
    pub provider_calls: u8,
    pub task: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub last_error: Option<String>,
    pub result: Option<serde_json::Value>,
}

impl CloudDerivativeState {
    pub fn not_prepared() -> Self {
        Self {
            status: "not_prepared".into(),
            preview_hash: None,
            approved_at: None,
            dispatch_status: "not_queued".into(),
            job_id: None,
            provider_role: None,
            queued_at: None,
            provider_calls: 0,
            task: None,
            started_at: None,
            completed_at: None,
            last_error: None,
            result: None,
        }
    }
}

/// Per-source run bookkeeping (round-trips via record_run/get_source_state).
#[derive(Debug, Clone, PartialEq)]
pub struct SourceState {
    pub source_name: String,
    pub last_run_at: String,
    pub cursor: Option<String>,
    /// Distinct from `last_run_at`: a run that failed still ran. Kept apart so
    /// "we last actually collected something at T" survives a failing streak,
    /// which is the number a human wants when deciding whether to intervene.
    pub last_success_at: Option<String>,
    pub last_failure_at: Option<String>,
    /// Error *class*, never a message body or a mail field.
    pub last_error: Option<String>,
    pub considered_count: i64,
    pub new_count: i64,
    /// Drives the backoff. Reset to 0 by any success.
    pub consecutive_failures: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EvaluationSummary {
    pub evaluated: i64,
    pub reranked: i64,
    pub semantic: i64,
    pub lexical: i64,
    pub unscored: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TravelContextSnapshot {
    pub revision: String,
    pub payload: String,
    pub refreshed_at: String,
}

/// Canonicalize a URL for stable identity: trim, drop the `#fragment`, drop a
/// single trailing slash, lowercase the scheme+host. Deliberately conservative
/// -- it does not try to normalize query params or youtu.be↔youtube.com.
pub fn canonical_url(url: &str) -> String {
    let mut s = url.trim().to_string();
    if let Some(hash) = s.find('#') {
        s.truncate(hash);
    }
    if let Some(scheme_end) = s.find("://") {
        let host_start = scheme_end + 3;
        let host_end = s[host_start..]
            .find('/')
            .map(|i| host_start + i)
            .unwrap_or(s.len());
        let lowered = s[..host_end].to_lowercase();
        s = format!("{}{}", lowered, &s[host_end..]);
    }
    if s.ends_with('/') {
        s.pop();
    }
    s
}

/// feed_items PK: sha256 hex of the canonical URL.
pub fn feed_id(url: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(canonical_url(url).as_bytes());
    format!("{:x}", hasher.finalize())
}

/// The classification a human's reclassification request should write, or the
/// reason it is refused.
///
/// Shared by feed and mail because it is one rule, and the two tables having
/// their own copy of it is how they would come to disagree. Escalating needs no
/// reason and none is invented: the canned sentence is honest about being a
/// manual change. Lowering needs the operator's own words, and gets them —
/// stored, so the decision is answerable afterwards.
pub(crate) fn human_reclassification(
    current: &str,
    proposed: &str,
    rationale: Option<&str>,
) -> Result<crate::content_item::DataClass, Box<dyn std::error::Error>> {
    let written = rationale.map(str::trim).unwrap_or_default();
    crate::content_item::admit_reclassification(
        current,
        proposed,
        crate::content_item::METHOD_HUMAN,
        written,
    )?;
    let rationale = if written.is_empty() {
        "Data class set manually in Axon.".to_string()
    } else {
        written.to_string()
    };
    Ok(crate::content_item::DataClass::set_by_human(
        proposed, &rationale,
    ))
}

fn cloud_job_id(request: &CloudQueueRequest) -> String {
    let mut hasher = Sha256::new();
    for part in [
        request.source.as_str(),
        request.item_id.as_str(),
        request.source_revision.as_str(),
        request.preview_hash.as_str(),
        request.provider_role.as_str(),
    ] {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    format!("cloud-job-{:x}", hasher.finalize())
}

mod cloud;
mod evaluation;
mod feed;
mod migrations;
mod origins;
mod rows;
mod source_state;
mod triage;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedOrigin {
    pub source_id: String,
    pub source_ref: String,
    pub label: Option<String>,
}

/// Gap between two arrivals of the same source that reads as "a different run".
const RUN_GAP_MINUTES: i64 = 30;

/// One item's place in a collector run, derived at read time. The reader groups
/// on `run_key`; an item with no origin row appears in none of these and stays
/// an ordinary ungrouped row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedRun {
    pub feed_id: String,
    pub source_id: String,
    pub label: Option<String>,
    pub run_key: String,
    pub run_started: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OriginSummary {
    pub source_id: String,
    pub item_count: i64,
    pub first_seen: Option<String>,
    pub last_seen: Option<String>,
}

use rows::{epoch_now, row_to_feed_full, row_to_feed_list, row_to_triage};

#[cfg(test)]
mod unit_tests {
    use super::{canonical_url, feed_id};

    #[test]
    fn canonical_url_and_feed_id_stable() {
        assert_eq!(
            canonical_url("HTTPS://YouTube.com/watch?v=abc/#t=10"),
            "https://youtube.com/watch?v=abc"
        );
        // Identity is stable across trailing slash / fragment / host case.
        assert_eq!(
            feed_id("https://example.com/x/"),
            feed_id("https://EXAMPLE.com/x#frag")
        );
        assert_eq!(feed_id("https://a.example").len(), 64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Only the test helpers still connect directly: `drop_test_schema` tears down a
    // schema outside the pool, deliberately, so it works even when the pool is
    // exhausted or the store under test is broken.
    use postgres::NoTls;

    /// The same connection the binaries use, so a rotated Postgres password
    /// can't leave the tests behind. `Config::load()` reads the overlay's
    /// `postgres.env` via `AXON_PERSONAL_ROOT`; `COMMS_TEST_DATABASE_URL`
    /// overrides it for a throwaway database. A second hardcoded default here
    /// is what made these tests fail against a live server for weeks.
    fn test_database_url() -> String {
        static URL: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        // Resolved once: the config tests mutate process-global env while these
        // run alongside them, and every store test must agree on one database.
        URL.get_or_init(|| {
            std::env::var("COMMS_TEST_DATABASE_URL")
                .unwrap_or_else(|_| crate::config::Config::load().database_url)
        })
        .clone()
    }

    fn open_test_store(name: &str) -> (Store, TestSchema) {
        let schema = format!("comms_test_{name}_{}", std::process::id());
        let store = Store::open_with_schema(&test_database_url(), &schema).unwrap_or_else(|e| {
            panic!(
                "could not open test store: {e} — needs capabilities/postgres running and \
                 AXON_PERSONAL_ROOT exported (or COMMS_TEST_DATABASE_URL set); see README"
            )
        });
        (store, TestSchema(schema))
    }

    /// The readiness probe has to reach the database, not merely hold a pool handle —
    /// a check that passes without touching Postgres is the bug #126 is about.
    #[test]
    fn ping_reaches_the_database() {
        let (store, _schema) = open_test_store("ping");
        store.ping().expect("a live store answers its own ping");
    }

    #[test]
    fn a_store_cannot_be_opened_against_an_unreachable_database() {
        // Port 1 is reserved and nothing listens there, so this fails the way a stopped
        // Postgres container does. The readiness handler turns exactly this into a 503.
        assert!(
            Store::open("host=127.0.0.1 port=1 user=axon password=axon dbname=axon").is_err(),
            "an unreachable database opened anyway"
        );
    }

    /// Drops the schema when it goes out of scope, including on unwind.
    ///
    /// The tests used to call drop_test_schema() as their last statement, which cleans up
    /// exactly when nothing goes wrong. A failing assertion panics first, the drop never
    /// runs, and the schema stays behind -- four of them were sitting in the shared
    /// database on 2026-07-28, from two long-finished processes, and every one would have
    /// gone into the next pg_dumpall. A guard runs on the way out either way.
    struct TestSchema(String);

    impl Drop for TestSchema {
        fn drop(&mut self) {
            drop_test_schema(&self.0);
        }
    }

    /// So a test that needs the schema name for raw SQL can still say `{schema}`.
    impl std::fmt::Display for TestSchema {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(&self.0)
        }
    }

    fn drop_test_schema(schema: &str) {
        if let Ok(mut client) = Client::connect(&test_database_url(), NoTls) {
            let _ = client.batch_execute(&format!("DROP SCHEMA IF EXISTS {schema} CASCADE"));
        }
    }

    fn mk_triage(id: &str, stream: &str) -> TriageItem {
        TriageItem {
            id: id.into(),
            from_addr: Some("news@shop.example".into()),
            subject: Some("SALE".into()),
            snippet: Some("snippet".into()),
            internal_date_ms: Some(1_700_000_000_000),
            internal_date_text: None,
            stream: stream.into(),
            rationale: "test".into(),
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
            first_seen: String::new(),
            last_seen: String::new(),
        }
    }

    fn mk_feed(url: &str, kind: &str, stream: &str) -> FeedItem {
        let mut f = FeedItem::new(url, stream, kind);
        f.title = Some("A Title".into());
        f.transcript = Some("some transcript".into());
        f
    }

    #[test]
    fn triage_upsert_is_idempotent_and_updates_fields() {
        let (store, _schema) = open_test_store("triage_idem");
        let mut item = mk_triage("thread:1", "werbung");
        assert!(store.upsert_triage(&item).unwrap(), "first insert is new");
        item.rationale = "changed".into();
        item.stream = "feed".into();
        assert!(!store.upsert_triage(&item).unwrap(), "second is not new");
        let rows = store.list_triage(None).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].stream, "feed");
        assert_eq!(rows[0].rationale, "changed");
    }

    #[test]
    fn triage_set_status_validates() {
        let (store, _schema) = open_test_store("triage_status");
        store
            .upsert_triage(&mk_triage("thread:s", "aktiv"))
            .unwrap();
        assert!(store.set_triage_status("thread:s", "approved").unwrap());
        assert_eq!(
            store.get_triage_status("thread:s").unwrap().as_deref(),
            Some("approved")
        );
        assert!(
            store.set_triage_status("thread:s", "bogus").is_err(),
            "invalid status must error"
        );
        assert!(
            !store
                .set_triage_status("thread:missing", "dismissed")
                .unwrap(),
            "unknown id -> false"
        );
    }

    /// `waiting` is its own axis, and clearing it takes the timestamp with it.
    ///
    /// The stale-timestamp case is the one worth pinning: a row left with
    /// `waiting = false` but a `waiting_at` still set reads as a currently
    /// blocked thread to any query that ranks on the timestamp and forgets the
    /// boolean, which is exactly the query someone writes first.
    #[test]
    fn waiting_is_recorded_and_fully_cleared() {
        let (store, schema) = open_test_store("triage_waiting");
        store
            .upsert_triage(&mk_triage("thread:w", "aktiv"))
            .unwrap();

        let waiting_at = || -> Option<String> {
            let mut conn = store.conn().unwrap();
            conn.query_one(
                &format!(
                    "SELECT waiting, waiting_at::TEXT FROM {}.triage_items WHERE id = $1",
                    schema.0
                ),
                &[&"thread:w"],
            )
            .map(|row| {
                let flag: bool = row.get(0);
                let at: Option<String> = row.get(1);
                assert_eq!(flag, at.is_some(), "the flag and the timestamp must agree");
                at
            })
            .unwrap()
        };

        assert!(waiting_at().is_none(), "a fresh proposal is not waiting");

        assert!(store.set_triage_waiting("thread:w", true).unwrap());
        assert!(waiting_at().is_some(), "marking must stamp when");

        assert!(store.set_triage_waiting("thread:w", false).unwrap());
        assert!(
            waiting_at().is_none(),
            "clearing must drop the timestamp, not only the flag"
        );

        assert!(
            !store.set_triage_waiting("thread:missing", true).unwrap(),
            "unknown id -> false"
        );
    }

    #[test]
    fn gmail_lifecycle_distinguishes_archive_trash_and_restore() {
        let (store, _schema) = open_test_store("gmail_lifecycle");
        store
            .upsert_triage(&mk_triage("thread:archive", "aktiv"))
            .unwrap();
        store
            .upsert_triage(&mk_triage("thread:trash", "aktiv"))
            .unwrap();

        assert!(store
            .record_gmail_action("thread:archive", "archive")
            .unwrap());
        let archived = store.get_triage("thread:archive").unwrap().unwrap();
        assert_eq!(archived.status, "archived");
        assert_eq!(archived.gmail_action.as_deref(), Some("archive"));
        assert!(archived.gmail_action_at.is_some());
        assert!(archived.purge_after.is_none());

        assert!(store.record_gmail_action("thread:trash", "trash").unwrap());
        let trashed = store.get_triage("thread:trash").unwrap().unwrap();
        assert_eq!(trashed.status, "trashed");
        assert_eq!(trashed.gmail_action.as_deref(), Some("trash"));
        assert!(trashed.purge_after.is_some());

        assert!(store
            .record_gmail_action("thread:trash", "restore")
            .unwrap());
        let restored = store.get_triage("thread:trash").unwrap().unwrap();
        assert_eq!(restored.status, "proposed");
        assert_eq!(restored.gmail_action.as_deref(), Some("restore"));
        assert!(restored.purge_after.is_none());
        assert!(store.record_gmail_action("thread:trash", "delete").is_err());
    }

    #[test]
    fn gmail_action_intent_is_durable_idempotent_and_bounded() {
        let (store, _schema) = open_test_store("gmail_action_jobs");
        store
            .upsert_triage(&mk_triage("thread:job", "aktiv"))
            .unwrap();

        let job = store.queue_gmail_action("thread:job", "archive").unwrap();
        assert_eq!(job.action, "archive");
        assert_eq!(job.source_status, "proposed");
        assert!(store.queue_gmail_action("thread:job", "trash").is_err());
        assert_eq!(store.pending_gmail_actions(10).unwrap(), vec![job.clone()]);
        let queued = store.get_triage("thread:job").unwrap().unwrap();
        assert_eq!(queued.gmail_sync_status.as_deref(), Some("queued"));

        assert!(store.complete_gmail_action(job.job_id).unwrap());
        assert!(store.complete_gmail_action(job.job_id).unwrap());
        assert!(store.pending_gmail_actions(10).unwrap().is_empty());
        let archived = store.get_triage("thread:job").unwrap().unwrap();
        assert_eq!(archived.status, "archived");
        assert_eq!(archived.gmail_location.as_deref(), Some("archive"));
        assert_eq!(archived.gmail_sync_status.as_deref(), Some("synced"));

        let restore = store.queue_gmail_action("thread:job", "restore").unwrap();
        for attempt in 1..=5 {
            let state = store
                .fail_gmail_action(restore.job_id, "bounded test error")
                .unwrap();
            assert_eq!(state, if attempt == 5 { "abandoned" } else { "queued" });
            if attempt < 5 {
                let mut conn = store.conn().unwrap();
                conn.execute(
                    &format!(
                        "UPDATE {}.gmail_action_jobs SET next_attempt = now() WHERE job_id = $1",
                        store.schema
                    ),
                    &[&restore.job_id],
                )
                .unwrap();
            }
        }
        assert!(store.pending_gmail_actions(10).unwrap().is_empty());
        let attention = store.get_triage("thread:job").unwrap().unwrap();
        assert_eq!(attention.gmail_sync_status.as_deref(), Some("attention"));
        assert_eq!(attention.gmail_sync_action.as_deref(), Some("restore"));
        assert_eq!(
            attention.gmail_sync_error.as_deref(),
            Some("bounded test error")
        );
        assert!(store
            .gmail_reconcile_candidates(10)
            .unwrap()
            .iter()
            .all(|candidate| candidate.triage_id != "thread:job"));

        let retried = store.retry_abandoned_gmail_action("thread:job").unwrap();
        assert_eq!(retried.action, "restore");
        assert_eq!(retried.attempts, 0);
        assert_eq!(
            store
                .get_triage("thread:job")
                .unwrap()
                .unwrap()
                .gmail_sync_status
                .as_deref(),
            Some("queued")
        );
        for _ in 0..5 {
            store
                .fail_gmail_action(retried.job_id, "still unavailable")
                .unwrap();
        }
        assert!(store.cancel_abandoned_gmail_action("thread:job").unwrap());
        assert!(!store.cancel_abandoned_gmail_action("thread:job").unwrap());
        let canceled = store.get_triage("thread:job").unwrap().unwrap();
        assert_eq!(canceled.gmail_sync_status.as_deref(), Some("synced"));
        assert!(canceled.gmail_sync_action.is_none());
        assert!(canceled.gmail_sync_error.is_none());
    }

    #[test]
    fn observed_gmail_location_reconciles_direct_changes() {
        let (store, _schema) = open_test_store("gmail_observed_location");
        store
            .upsert_triage(&mk_triage("thread:observed", "aktiv"))
            .unwrap();
        assert!(store
            .observe_gmail_location("thread:observed", "archive")
            .unwrap());
        let archived = store.get_triage("thread:observed").unwrap().unwrap();
        assert_eq!(archived.status, "archived");
        assert_eq!(archived.gmail_location.as_deref(), Some("archive"));
        assert!(archived.gmail_action.is_none());

        store
            .observe_gmail_location("thread:observed", "trash")
            .unwrap();
        let trashed = store.get_triage("thread:observed").unwrap().unwrap();
        assert_eq!(trashed.status, "trashed");
        assert!(trashed.purge_after.is_some());

        store
            .observe_gmail_location("thread:observed", "inbox")
            .unwrap();
        let restored = store.get_triage("thread:observed").unwrap().unwrap();
        assert_eq!(restored.status, "proposed");
        assert_eq!(restored.gmail_location.as_deref(), Some("inbox"));
        assert!(restored.purge_after.is_none());
        assert!(store
            .observe_gmail_location("thread:observed", "spam")
            .is_err());
    }

    #[test]
    fn missing_gmail_thread_is_retained_and_closes_pending_work() {
        let (store, _schema) = open_test_store("gmail_missing");
        store
            .upsert_triage(&mk_triage("thread:missing", "aktiv"))
            .unwrap();
        store
            .observe_gmail_location("thread:missing", "trash")
            .unwrap();
        let deadline = store
            .get_triage("thread:missing")
            .unwrap()
            .unwrap()
            .purge_after;

        assert!(store.observe_gmail_missing("thread:missing").unwrap());
        let missing = store.get_triage("thread:missing").unwrap().unwrap();
        assert_eq!(missing.status, "missing");
        assert_eq!(missing.gmail_location.as_deref(), Some("missing"));
        assert_eq!(missing.gmail_sync_status.as_deref(), Some("synced"));
        assert_eq!(missing.purge_after, deadline);

        store
            .upsert_triage(&mk_triage("thread:missing", "aktiv"))
            .unwrap();
        let returned = store.get_triage("thread:missing").unwrap().unwrap();
        assert_eq!(returned.status, "proposed");
        assert_eq!(returned.gmail_location.as_deref(), Some("inbox"));
        assert!(returned.purge_after.is_none());
    }

    #[test]
    fn expired_trash_is_purged_but_archive_is_retained() {
        let (store, _schema) = open_test_store("gmail_trash_purge");
        store
            .upsert_triage(&mk_triage("thread:expired", "aktiv"))
            .unwrap();
        store
            .upsert_triage(&mk_triage("thread:archive", "aktiv"))
            .unwrap();
        store
            .record_gmail_action("thread:expired", "trash")
            .unwrap();
        store
            .record_gmail_action("thread:archive", "archive")
            .unwrap();
        {
            let mut conn = store.conn().unwrap();
            conn.execute(
                &format!(
                    "UPDATE {}.triage_items SET purge_after = now() - interval '1 second' WHERE id = $1",
                    store.schema
                ),
                &[&"thread:expired"],
            )
            .unwrap();
        }

        assert_eq!(store.purge_expired_trashed().unwrap(), 1);
        assert!(store.get_triage("thread:expired").unwrap().is_none());
        assert_eq!(
            store
                .get_triage_status("thread:archive")
                .unwrap()
                .as_deref(),
            Some("archived")
        );
    }

    #[test]
    fn human_triage_category_survives_a_resweep() {
        let (store, _schema) = open_test_store("triage_human_category");
        store
            .upsert_triage(&mk_triage("thread:manual", "aktiv"))
            .unwrap();
        assert!(store.set_triage_stream("thread:manual", "belege").unwrap());

        let mut refetched = mk_triage("thread:manual", "werbung");
        refetched.rationale = "new rule result".into();
        store.upsert_triage(&refetched).unwrap();

        let row = store.list_triage(None).unwrap().remove(0);
        assert_eq!(row.stream, "belege");
        assert_eq!(row.rationale, "Category set manually in Axon.");
        assert_eq!(row.classification_method, "human");
        assert_eq!(row.classification_version, "manual-v1");
        assert_eq!(
            row.status, "proposed",
            "categorizing does not resolve the proposal"
        );
        assert!(store.set_triage_stream("thread:manual", "bogus").is_err());
    }

    #[test]
    fn human_data_class_survives_rule_refresh_and_resweep() {
        let (store, _schema) = open_test_store("triage_human_data_class");
        store
            .upsert_triage(&mk_triage("thread:private", "aktiv"))
            .unwrap();
        assert!(store
            .set_triage_data_class("thread:private", "vault", None)
            .unwrap());

        let rules =
            crate::content_item::DataClass::classify_mail("aktiv", "friend@example.com", "Hello");
        assert!(!store
            .refresh_triage_data_class("thread:private", &rules)
            .unwrap());
        store
            .upsert_triage(&mk_triage("thread:private", "feed"))
            .unwrap();

        let row = store.get_triage("thread:private").unwrap().unwrap();
        assert_eq!(row.data_class, "vault");
        assert_eq!(row.data_classification_method, "human");
        assert_eq!(row.data_classification_version, "manual-v1");
        assert_eq!(row.data_class_rationale, "Data class set manually in Axon.");
        assert!(store
            .set_triage_data_class("thread:private", "secret", Some("why"))
            .is_err());
    }

    #[test]
    fn reviewed_cloud_derivative_is_staged_and_becomes_stale_with_its_source() {
        let (store, _schema) = open_test_store("cloud_derivative");
        let approval = CloudDerivativeApproval {
            source: "mail".into(),
            item_id: "thread:cloud".into(),
            source_revision: "source-v1".into(),
            preview_hash: "preview-v1".into(),
            original_data_class: "vault".into(),
            derivative_data_class: "personal".into(),
            transformation: "deterministic-entity-redaction-v2".into(),
            document: "Title\n[identity removed]".into(),
            redaction_count: 1,
        };

        let staged = store.stage_cloud_derivative(&approval).unwrap();
        assert_eq!(staged.status, "staged");
        assert_eq!(staged.preview_hash.as_deref(), Some("preview-v1"));
        assert_eq!(staged.dispatch_status, "not_queued");
        assert_eq!(staged.provider_calls, 0);

        let current = store
            .cloud_derivative_state("mail", "thread:cloud", "source-v1", "preview-v1")
            .unwrap();
        assert_eq!(current.status, "staged");

        let stale = store
            .cloud_derivative_state("mail", "thread:cloud", "source-v2", "preview-v2")
            .unwrap();
        assert_eq!(stale.status, "stale");
        assert_eq!(stale.provider_calls, 0);

        let changed_policy = store
            .cloud_derivative_state("mail", "thread:cloud", "source-v1", "preview-v2")
            .unwrap();
        assert_eq!(changed_policy.status, "stale");
    }

    #[test]
    fn approved_derivative_queues_once_for_an_explicit_cloud_role() {
        let (store, _schema) = open_test_store("cloud_queue");
        store
            .stage_cloud_derivative(&CloudDerivativeApproval {
                source: "mail".into(),
                item_id: "thread:queue".into(),
                source_revision: "source-v1".into(),
                preview_hash: "preview-v1".into(),
                original_data_class: "personal".into(),
                derivative_data_class: "personal".into(),
                transformation: "deterministic-entity-redaction-v2".into(),
                document: "Title\n[person]".into(),
                redaction_count: 1,
            })
            .unwrap();
        let request = CloudQueueRequest {
            source: "mail".into(),
            item_id: "thread:queue".into(),
            source_revision: "source-v1".into(),
            preview_hash: "preview-v1".into(),
            provider_role: "cloud_summarization".into(),
        };

        let first = store.queue_cloud_derivative(&request).unwrap();
        let again = store.queue_cloud_derivative(&request).unwrap();
        assert_eq!(first.dispatch_status, "queued");
        assert_eq!(first.job_id, again.job_id);
        assert_eq!(first.provider_calls, 0);

        let state = store
            .cloud_derivative_state("mail", "thread:queue", "source-v1", "preview-v1")
            .unwrap();
        assert_eq!(state.dispatch_status, "queued");
        assert_eq!(state.provider_role.as_deref(), Some("cloud_summarization"));
        let stale = CloudQueueRequest {
            preview_hash: "stale".into(),
            ..request
        };
        assert!(store.queue_cloud_derivative(&stale).is_err());
    }

    #[test]
    fn queued_cloud_job_claims_once_and_persists_a_bounded_result() {
        let (store, _schema) = open_test_store("cloud_dispatch");
        store
            .stage_cloud_derivative(&CloudDerivativeApproval {
                source: "mail".into(),
                item_id: "thread:dispatch".into(),
                source_revision: "source-v1".into(),
                preview_hash: "preview-v1".into(),
                original_data_class: "personal".into(),
                derivative_data_class: "personal".into(),
                transformation: "deterministic-entity-redaction-v2".into(),
                document: "Title\n[person] visits on 2026-08-10".into(),
                redaction_count: 1,
            })
            .unwrap();
        let queued = store
            .queue_cloud_derivative(&CloudQueueRequest {
                source: "mail".into(),
                item_id: "thread:dispatch".into(),
                source_revision: "source-v1".into(),
                preview_hash: "preview-v1".into(),
                provider_role: "cloud_summarization".into(),
            })
            .unwrap();
        let job_id = queued.job_id.unwrap();
        let job = store.cloud_job_for_dispatch(&job_id).unwrap().unwrap();
        assert_eq!(job.task, "content-analysis-v1");
        assert_eq!(job.original_data_class, "personal");
        assert_eq!(job.derivative_data_class, "personal");
        assert_eq!(job.transformation, "deterministic-entity-redaction-v2");
        assert_eq!(job.document, "Title\n[person] visits on 2026-08-10");
        assert_eq!(job.provider_calls, 0);
        let attempt_id = match store
            .claim_cloud_job_attempt(&job_id, "cloud_summarization", "model-a", 10)
            .unwrap()
        {
            CloudAttemptClaim::Started(attempt_id) => attempt_id,
            other => panic!("unexpected claim: {other:?}"),
        };
        assert_eq!(
            store
                .claim_cloud_job_attempt(&job_id, "cloud_summarization", "model-a", 10)
                .unwrap(),
            CloudAttemptClaim::JobUnavailable
        );

        let result = serde_json::json!({
            "schema_version": "cloud-content-analysis-v1",
            "summary": "A visit is planned.",
            "importance": "high",
            "importance_rationale": "A fixed date is present.",
            "important_dates": [{ "label": "Visit", "date": "2026-08-10", "source_text": "2026-08-10" }],
            "action_items": [],
            "topics": ["travel"]
        });
        assert!(store
            .complete_cloud_job_attempt(&job_id, attempt_id, &result)
            .unwrap());

        let state = store
            .cloud_derivative_state("mail", "thread:dispatch", "source-v1", "preview-v1")
            .unwrap();
        assert_eq!(state.dispatch_status, "succeeded");
        assert_eq!(state.provider_calls, 1);
        assert_eq!(
            state.result.unwrap()["important_dates"][0]["date"],
            "2026-08-10"
        );
        assert!(store.cloud_job_for_dispatch(&job_id).unwrap().is_none());

        let mut conn = store.conn().unwrap();
        let attempt = conn
            .query_one(
                &format!(
                    "SELECT provider_role, model, preview_hash, status, result_json
                     FROM {}.content_cloud_attempts WHERE attempt_id = $1",
                    store.schema
                ),
                &[&attempt_id],
            )
            .unwrap();
        assert_eq!(attempt.get::<_, String>(0), "cloud_summarization");
        assert_eq!(attempt.get::<_, String>(1), "model-a");
        assert_eq!(attempt.get::<_, String>(2), "preview-v1");
        assert_eq!(attempt.get::<_, String>(3), "succeeded");
        assert!(attempt.get::<_, Option<String>>(4).is_some());
    }

    #[test]
    fn cloud_daily_ceiling_blocks_before_a_second_provider_attempt() {
        let (store, _schema) = open_test_store("cloud_daily_budget");
        store
            .stage_cloud_derivative(&CloudDerivativeApproval {
                source: "mail".into(),
                item_id: "thread:budget".into(),
                source_revision: "source-v1".into(),
                preview_hash: "preview-v1".into(),
                original_data_class: "personal".into(),
                derivative_data_class: "personal".into(),
                transformation: "deterministic-entity-redaction-v2".into(),
                document: "Reviewed pseudonymized text".into(),
                redaction_count: 1,
            })
            .unwrap();
        let job_id = store
            .queue_cloud_derivative(&CloudQueueRequest {
                source: "mail".into(),
                item_id: "thread:budget".into(),
                source_revision: "source-v1".into(),
                preview_hash: "preview-v1".into(),
                provider_role: "cloud_primary".into(),
            })
            .unwrap()
            .job_id
            .unwrap();
        let attempt_id = match store
            .claim_cloud_job_attempt(&job_id, "cloud_primary", "model-a", 1)
            .unwrap()
        {
            CloudAttemptClaim::Started(attempt_id) => attempt_id,
            other => panic!("unexpected claim: {other:?}"),
        };
        assert!(store
            .fail_cloud_job_attempt(&job_id, attempt_id, "synthetic provider failure")
            .unwrap());
        assert_eq!(
            store.cloud_provider_calls_today("cloud_primary").unwrap(),
            1
        );
        assert_eq!(
            store
                .claim_cloud_job_attempt(&job_id, "cloud_primary", "model-a", 1)
                .unwrap(),
            CloudAttemptClaim::DailyLimitReached
        );
        assert_eq!(
            store
                .cloud_job_for_dispatch(&job_id)
                .unwrap()
                .unwrap()
                .provider_calls,
            1,
            "policy rejection must not consume another provider call"
        );
    }

    #[test]
    fn triage_relevance_replaces_stale_profiles_without_changing_the_proposal() {
        let (store, _schema) = open_test_store("triage_relevance");
        store
            .upsert_triage(&mk_triage("thread:relevant", "feed"))
            .unwrap();
        let first = vec![
            RelevanceMatch {
                profile_key: "systems".into(),
                profile_label: "Systems".into(),
                score: 0.9,
                rationale: "Semantic similarity for Systems".into(),
                mode: "semantic".into(),
                profile_revision: "systems-v1".into(),
            },
            RelevanceMatch {
                profile_key: "travel".into(),
                profile_label: "Travel".into(),
                score: 0.4,
                rationale: "Semantic similarity for Travel".into(),
                mode: "semantic".into(),
                profile_revision: "travel-v1".into(),
            },
        ];
        store
            .replace_triage_relevance("thread:relevant", &first)
            .unwrap();
        store
            .replace_triage_relevance("thread:relevant", &first[..1])
            .unwrap();

        let stored = store.triage_relevance("thread:relevant").unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].profile_key, "systems");
        let proposal = store.list_triage(None).unwrap().remove(0);
        assert_eq!(proposal.stream, "feed");
        assert_eq!(proposal.status, "proposed");
    }

    /// Critical: a human's triage decision must survive a re-sweep of the same
    /// thread. upsert_triage's ON CONFLICT must not touch `status`.
    #[test]
    fn upsert_preserves_status_across_refetch_triage() {
        let (store, _schema) = open_test_store("triage_preserve");
        let item = mk_triage("thread:keep", "werbung");
        store.upsert_triage(&item).unwrap();
        store.set_triage_status("thread:keep", "dismissed").unwrap();

        // Re-sweep: same id, fresh rationale/stream.
        let mut refetched = mk_triage("thread:keep", "feed");
        refetched.rationale = "re-swept".into();
        assert!(!store.upsert_triage(&refetched).unwrap(), "still same id");

        assert_eq!(
            store.get_triage_status("thread:keep").unwrap().as_deref(),
            Some("dismissed"),
            "dismiss decision must survive re-sweep"
        );
        // Other fields legitimately update.
        let rows = store.list_triage(None).unwrap();
        assert_eq!(rows[0].rationale, "re-swept");
    }

    /// The fail-closed default, proved against the database rather than against
    /// the constructor. A pasted URL is what an operator ingests from a page
    /// they were logged into, and it must not come back cloud-eligible.
    #[test]
    fn an_ingested_item_nobody_declared_is_stored_personal_and_legacy() {
        let (store, _schema) = open_test_store("feed_class_default");
        let item = mk_feed("https://example.com/undeclared", "article", "news");
        store.upsert_feed(&item).unwrap();

        let stored = store.get_feed(&item.id).unwrap().unwrap();
        assert_eq!(stored.data_class, "personal");
        assert_eq!(stored.data_classification_method, "legacy");
        assert_eq!(
            content_item::processing_policy(&stored.data_class).cloud_handling,
            "pseudonymization_required"
        );
    }

    /// A collector declares at discovery. That is the whole window: the class
    /// lands with the INSERT, and every later pass may only raise it.
    #[test]
    fn a_collector_declares_its_class_when_it_first_stores_the_item() {
        let (store, _schema) = open_test_store("feed_class_declared");
        let mut item = mk_feed("https://example.com/declared", "arxiv", "news");
        item.declare_class(&content_item::DataClass::declared_by_source(
            "public",
            "Declared by feed source 'arxiv-ai-recent'.",
        ));
        store.upsert_feed(&item).unwrap();

        let stored = store.get_feed(&item.id).unwrap().unwrap();
        assert_eq!(stored.data_class, "public");
        assert_eq!(stored.data_classification_method, "deterministic");
        assert_eq!(
            content_item::processing_policy(&stored.data_class).cloud_handling,
            "eligible",
            "a positively declared public item is the only kind that is"
        );
    }

    /// Ingest is a machine path: it may raise a class and never lower one.
    ///
    /// The first assertion is the one the anti-claim rests on. A row stored
    /// undeclared is Personal, and no collector can relabel it afterwards --
    /// so a `legacy` item has no machine route to Public at all, and the 187
    /// backfilled rows can only be lifted by a human who says why.
    #[test]
    fn a_re_ingest_can_raise_a_feed_class_but_never_lower_one() {
        let (store, _schema) = open_test_store("feed_class_escalation");
        let mut item = mk_feed("https://example.com/escalate", "article", "news");
        store.upsert_feed(&item).unwrap();

        item.declare_class(&content_item::DataClass::declared_by_source(
            "public",
            "Declared by feed source 'test'.",
        ));
        store.upsert_feed(&item).unwrap();
        let stored = store.get_feed(&item.id).unwrap().unwrap();
        assert_eq!(
            stored.data_class, "personal",
            "a collector cannot lift a row that was already stored undeclared"
        );
        assert_eq!(stored.data_classification_method, "legacy");

        // Escalation, on the other hand, needs nobody's permission.
        item.declare_class(&content_item::DataClass::declared_by_source(
            "vault",
            "Declared Private by its collector.",
        ));
        store.upsert_feed(&item).unwrap();
        assert_eq!(
            store.get_feed(&item.id).unwrap().unwrap().data_class,
            "vault"
        );

        // A human lowers it, with a reason. The collector then re-scans and
        // re-declares Private, which is an escalation, so that one lands.
        store
            .set_feed_data_class(&item.id, "public", Some("Published preprint."))
            .unwrap();
        store.upsert_feed(&item).unwrap();
        let stored = store.get_feed(&item.id).unwrap().unwrap();
        assert_eq!(stored.data_class, "vault");
        assert_eq!(stored.data_classification_method, "deterministic");
    }

    /// The de-escalation door: it opens for a human with a reason, and for
    /// nobody else. `Ok(false)` is reserved for a missing item, so a refusal is
    /// distinguishable from a typo'd id — that distinction is what lets the
    /// server answer 400 rather than 404.
    #[test]
    fn lowering_a_feed_class_needs_a_written_reason_and_says_so() {
        let (store, _schema) = open_test_store("feed_class_deescalation");
        let mut item = mk_feed("https://example.com/lower", "article", "news");
        item.declare_class(&content_item::DataClass::declared_by_source(
            "vault",
            "Declared Private by its collector.",
        ));
        store.upsert_feed(&item).unwrap();

        for empty in [None, Some(""), Some("   ")] {
            let error = store
                .set_feed_data_class(&item.id, "public", empty)
                .expect_err("a silent de-escalation must be refused");
            assert!(
                error.to_string().contains("rationale"),
                "the refusal must name what is missing, got: {error}"
            );
        }
        assert_eq!(
            store.get_feed(&item.id).unwrap().unwrap().data_class,
            "vault",
            "a refused request writes nothing"
        );

        assert!(store
            .set_feed_data_class(&item.id, "public", Some("Published preprint, no session."))
            .unwrap());
        let stored = store.get_feed(&item.id).unwrap().unwrap();
        assert_eq!(stored.data_class, "public");
        assert_eq!(
            stored.data_class_rationale, "Published preprint, no session.",
            "the operator's own words are what gets stored"
        );

        assert!(
            !store
                .set_feed_data_class("no-such-id", "vault", None)
                .unwrap(),
            "a missing item is false, not an error"
        );
        assert!(
            store
                .set_feed_data_class(&item.id, "confidential", Some("why"))
                .is_err(),
            "a class outside the vocabulary is refused"
        );
    }

    /// The mail sweep re-runs the rules on every pass. A rule edit that made
    /// the classifier less suspicious must not walk the inbox downgrading rows
    /// it had already called Private.
    #[test]
    fn a_resweep_cannot_downgrade_a_mail_the_rules_once_called_private() {
        let (store, _schema) = open_test_store("triage_class_escalation");
        let mut item = mk_triage("thread:class", "aktiv");
        item.data_class = "vault".into();
        item.data_class_rationale = "Authentication metadata is Private.".into();
        item.data_classification_method = content_item::METHOD_DETERMINISTIC.into();
        store.upsert_triage(&item).unwrap();

        item.data_class = "personal".into();
        item.data_class_rationale = "Mail metadata is Personal by default.".into();
        store.upsert_triage(&item).unwrap();

        let stored = store.list_triage(None).unwrap();
        assert_eq!(stored[0].data_class, "vault");
        assert_eq!(
            stored[0].data_class_rationale,
            "Authentication metadata is Private."
        );
    }

    #[test]
    fn feed_upsert_is_idempotent_and_coalesces_summary() {
        let (store, _schema) = open_test_store("feed_idem");
        let url = "https://youtu.be/xyz";
        let mut item = mk_feed(url, "youtube", "media");
        assert!(store.upsert_feed(&item).unwrap(), "first insert is new");

        // Give it a summary out of band (as summarize would).
        store
            .update_feed_summary(&item.id, "distilled", "test-summarizer-v1")
            .unwrap();

        // Re-ingest with summary=None must NOT wipe the stored summary.
        item.summary = None;
        item.title = Some("Better Title".into());
        assert!(!store.upsert_feed(&item).unwrap(), "second is not new");

        let stored = store.get_feed(&item.id).unwrap().unwrap();
        assert_eq!(
            stored.summary.as_deref(),
            Some("distilled"),
            "summary preserved via COALESCE"
        );
        assert_eq!(
            stored.title.as_deref(),
            Some("Better Title"),
            "title updated"
        );

        item.summary = Some("older imported summary".into());
        item.summary_provenance = Some(StageProvenance::legacy("old-import"));
        store.upsert_feed(&item).unwrap();
        assert_eq!(
            store
                .get_feed(&item.id)
                .unwrap()
                .unwrap()
                .summary
                .as_deref(),
            Some("distilled"),
            "a legacy summary cannot replace a model-tier result"
        );
        let stages = store.feed_stage_results(&item.id).unwrap();
        assert_eq!(
            stages
                .iter()
                .find(|stage| stage.stage == "summary")
                .unwrap()
                .tier,
            "model"
        );
    }

    #[test]
    fn relevance_replace_removes_stale_profiles_without_touching_status() {
        let (store, _schema) = open_test_store("feed_relevance_replace");
        let item = mk_feed("https://example.com/relevant", "article", "news");
        store.upsert_feed(&item).unwrap();
        store.set_feed_status(&item.id, "keeper").unwrap();
        let first = vec![
            RelevanceMatch {
                profile_key: "a".into(),
                profile_label: "Polymath".into(),
                score: 0.8,
                rationale: "match".into(),
                mode: "reranked".into(),
                profile_revision: "one".into(),
            },
            RelevanceMatch {
                profile_key: "b".into(),
                profile_label: "Career".into(),
                score: 0.4,
                rationale: "match".into(),
                mode: "reranked".into(),
                profile_revision: "one".into(),
            },
        ];
        store.replace_feed_relevance(&item.id, &first).unwrap();
        store.replace_feed_relevance(&item.id, &first[..1]).unwrap();
        let stored = store.feed_relevance(&item.id).unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].profile_label, "Polymath");
        assert_eq!(stored[0].mode, "reranked");
        assert_eq!(
            store.get_feed_status(&item.id).unwrap().as_deref(),
            Some("keeper")
        );
    }

    #[test]
    fn quality_flags_round_trip_replaces_stale_reasons_and_keeps_item_status() {
        let (store, _schema) = open_test_store("feed_quality_flags");
        let item = mk_feed("https://example.com/quality", "article", "news");
        store.upsert_feed(&item).unwrap();
        store.set_feed_status(&item.id, "keeper").unwrap();

        store
            .replace_feed_quality_flags(
                &item.id,
                &[
                    QualityFlag {
                        signal: "retention".into(),
                        reason: "retention fired: old reason".into(),
                        evidence: "retained=95.0%".into(),
                    },
                    QualityFlag {
                        signal: "summary_attempts".into(),
                        reason: "summary_attempts fired: retrying".into(),
                        evidence: "attempts=2; last_error=timeout".into(),
                    },
                ],
            )
            .unwrap();
        store
            .replace_feed_quality_flags(
                &item.id,
                &[QualityFlag {
                    signal: "retention".into(),
                    reason: "retention fired: current reason".into(),
                    evidence: "retained=92.0%".into(),
                }],
            )
            .unwrap();

        let rows = store.feed_quality_review_queue(20).unwrap();
        assert_eq!(rows.len(), 1, "signals absent from the new set are removed");
        assert_eq!(rows[0].signal, "retention");
        assert_eq!(rows[0].reason, "retention fired: current reason");
        assert_eq!(rows[0].evidence, "retained=92.0%");
        assert_eq!(rows[0].status, "keeper", "flagging never owns status");

        store.replace_feed_quality_flags(&item.id, &[]).unwrap();
        assert!(store.feed_quality_review_queue(20).unwrap().is_empty());
    }

    #[test]
    fn evaluation_replace_round_trips_factors_and_revision() {
        let (store, _schema) = open_test_store("feed_evaluation_replace");
        let item = mk_feed("https://example.com/evaluated", "article", "news");
        store.upsert_feed(&item).unwrap();
        let mut evaluation = FeedEvaluation {
            feed_id: item.id.clone(),
            overall_score: 0.72,
            explanation: "transparent".into(),
            mode: "reranked".into(),
            item_revision: "item-one".into(),
            context_revision: "context-one".into(),
            evaluator_revision: "feed-evaluator-v4-english".into(),
            evaluated_at: String::new(),
            factors: vec![EvaluationFactor {
                key: "interest".into(),
                label: "Interest fit".into(),
                score: 0.8,
                weight: 0.6,
                rationale: "matched".into(),
                context: Some(EvaluationFactorContext {
                    kind: "trip".into(),
                    id: "trip:one".into(),
                    label: "Berlin".into(),
                    date_start: Some("2026-09-10".into()),
                    date_end: Some("2026-09-12".into()),
                    matched_terms: vec!["Berlin".into()],
                }),
            }],
        };
        store.replace_feed_evaluation(&evaluation).unwrap();
        evaluation.overall_score = 0.4;
        evaluation.item_revision = "item-two".into();
        evaluation.factors[0].score = 0.3;
        store.replace_feed_evaluation(&evaluation).unwrap();

        let stored = store.feed_evaluation(&item.id).unwrap().unwrap();
        assert_eq!(stored.item_revision, "item-two");
        assert_eq!(stored.mode, "reranked");
        assert_eq!(stored.factors.len(), 1);
        assert_eq!(stored.factors[0].score, 0.3);
        assert_eq!(
            stored.factors[0]
                .context
                .as_ref()
                .map(|context| context.id.as_str()),
            Some("trip:one")
        );
        assert_eq!(store.evaluation_summary().unwrap().reranked, 1);

        let mut lower = evaluation.clone();
        lower.mode = "lexical".into();
        lower.overall_score = 0.1;
        lower.evaluator_revision = "fallback-v2".into();
        assert!(!store.replace_feed_evaluation(&lower).unwrap());
        assert_eq!(
            store
                .feed_evaluation(&item.id)
                .unwrap()
                .unwrap()
                .overall_score,
            0.4,
            "a deterministic ranking cannot replace a model-tier result"
        );
        let stages = store.feed_stage_results(&item.id).unwrap();
        let ranking = stages
            .iter()
            .find(|stage| stage.stage == "ranking")
            .unwrap();
        assert_eq!(ranking.tier, "model");
        assert_eq!(ranking.revision, "feed-evaluator-v4-english");
    }

    #[test]
    fn travel_context_snapshot_round_trips() {
        let (store, _schema) = open_test_store("travel_context_snapshot");
        store
            .replace_travel_context_snapshot("revision-one", "[{\"id\":\"trip:one\"}]")
            .unwrap();
        let snapshot = store.travel_context_snapshot().unwrap().unwrap();
        assert_eq!(snapshot.revision, "revision-one");
        assert_eq!(snapshot.payload, "[{\"id\":\"trip:one\"}]");
        assert!(!snapshot.refreshed_at.is_empty());
    }

    #[test]
    fn enrichment_ledger_and_attempts_cap() {
        let (store, _schema) = open_test_store("enrichment_ledger");
        let mut item = mk_feed("https://example.com/failed-item", "article", "news");
        item.transcript = Some("Short transcript for test".into());
        item.content_status = "thin".into();
        store.upsert_feed(&item).unwrap();

        // Initially 1 pending summary, 0 failed.
        let counts = store.feed_enrichment_counts(None).unwrap();
        assert_eq!(counts.pending_summaries, 1);
        assert_eq!(counts.failed_summaries, 0);

        let status_counts = store.feed_content_status_counts().unwrap();
        assert_eq!(status_counts.thin, 1);

        // Record 3 failed attempts.
        store
            .record_summary_attempt(&item.id, "http_error", "summary-v1")
            .unwrap();
        store
            .record_summary_attempt(&item.id, "http_error", "summary-v1")
            .unwrap();
        store
            .record_summary_attempt(&item.id, "http_error", "summary-v1")
            .unwrap();

        // Item should now be marked failed (summary_attempts >= 3) and no longer returned by feed_pending_summaries.
        let pending = store.feed_pending_summaries(Some("summary-v1")).unwrap();
        assert!(pending.iter().all(|i| i.id != item.id));

        let counts_after = store.feed_enrichment_counts(Some("summary-v1")).unwrap();
        assert_eq!(counts_after.pending_summaries, 0);
        assert_eq!(counts_after.failed_summaries, 1);

        // A new producer revision gets its own bounded retry ledger.
        let new_revision = store.feed_enrichment_counts(Some("summary-v2")).unwrap();
        assert_eq!(new_revision.pending_summaries, 1);
        assert_eq!(new_revision.failed_summaries, 0);
        store
            .record_summary_attempt(&item.id, "timeout", "summary-v2")
            .unwrap();
        assert_eq!(
            store.get_feed(&item.id).unwrap().unwrap().summary_attempts,
            1
        );

        // Updating summary resets attempt counters.
        store
            .update_feed_summary(&item.id, "Summary fixed", "test-summarizer-v1")
            .unwrap();
        let fetched = store.get_feed(&item.id).unwrap().unwrap();
        assert_eq!(fetched.summary.as_deref(), Some("Summary fixed"));
        assert_eq!(fetched.summary_attempts, 0);
        assert!(fetched.summary_last_error.is_none());
        assert!(fetched.summary_next_attempt.is_none());
    }

    #[test]
    fn feed_list_filters_stream_and_dismissed() {
        let (store, _schema) = open_test_store("feed_list");
        store
            .upsert_feed(&mk_feed("https://youtu.be/a", "youtube", "media"))
            .unwrap();
        store
            .upsert_feed(&mk_feed("https://example.com/post", "article", "news"))
            .unwrap();
        let dismissed = mk_feed("https://youtu.be/b", "youtube", "media");
        store.upsert_feed(&dismissed).unwrap();
        store.set_feed_status(&dismissed.id, "dismissed").unwrap();

        let media = store.list_feed(Some("media"), None, 7, false).unwrap();
        assert_eq!(
            media.len(),
            1,
            "one visible media item (other is dismissed)"
        );
        assert!(media[0].transcript.is_none(), "list view omits transcript");

        let media_all = store.list_feed(Some("media"), None, 7, true).unwrap();
        assert_eq!(media_all.len(), 2, "include_dismissed shows both");

        let news = store.list_feed(Some("news"), None, 7, false).unwrap();
        assert_eq!(news.len(), 1);
    }

    #[test]
    fn feed_origins_queries_and_grouping() {
        let (store, _schema) = open_test_store("feed_origins");
        let item1 = mk_feed("https://example.com/item1", "article", "news");
        let item2 = mk_feed("https://example.com/item2", "article", "news");
        store.upsert_feed(&item1).unwrap();
        store.upsert_feed(&item2).unwrap();

        store
            .record_feed_origin(
                &item1.id,
                "github-trending",
                "https://github.com/trending",
                Some("Trending Repo 1"),
            )
            .unwrap();
        store
            .record_feed_origin(
                &item2.id,
                "github-trending",
                "https://github.com/trending",
                Some("Trending Repo 2"),
            )
            .unwrap();
        store
            .record_feed_origin(
                &item1.id,
                "vault-scan",
                "/notes/ai.md",
                Some("Obsidian Link"),
            )
            .unwrap();

        let origins1 = store.feed_origins(&item1.id).unwrap();
        assert_eq!(origins1.len(), 2);

        let filtered = store
            .list_feed(None, Some("github-trending"), 7, false)
            .unwrap();
        assert_eq!(filtered.len(), 2);

        let filtered_vault = store.list_feed(None, Some("vault-scan"), 7, false).unwrap();
        assert_eq!(filtered_vault.len(), 1);

        let summaries = store.list_origin_summaries().unwrap();
        assert_eq!(summaries.len(), 2);
        let gh_summary = summaries
            .iter()
            .find(|s| s.source_id == "github-trending")
            .unwrap();
        assert_eq!(gh_summary.item_count, 2);
    }

    /// Critical: a keeper/dismiss decision must survive a re-ingest of the same
    /// URL. upsert_feed's ON CONFLICT must not touch `status`.
    #[test]
    fn upsert_preserves_status_across_refetch_feed() {
        let (store, _schema) = open_test_store("feed_preserve");
        let item = mk_feed("https://youtu.be/keepme", "youtube", "media");
        store.upsert_feed(&item).unwrap();
        store.set_feed_status(&item.id, "keeper").unwrap();

        let mut refetched = mk_feed("https://youtu.be/keepme", "youtube", "media");
        refetched.title = Some("Re-ingested".into());
        assert!(!store.upsert_feed(&refetched).unwrap(), "still same id");

        assert_eq!(
            store.get_feed_status(&item.id).unwrap().as_deref(),
            Some("keeper"),
            "keeper decision must survive re-ingest"
        );
    }

    #[test]
    fn runs_are_derived_from_arrival_gaps_and_leave_ungrouped_items_alone() {
        let (store, schema) = open_test_store("feed_runs");

        let together_a = mk_feed("https://github.com/o/a", "github", "news");
        let together_b = mk_feed("https://github.com/o/b", "github", "news");
        let later = mk_feed("https://github.com/o/c", "github", "news");
        let manual = mk_feed("https://example.com/pasted", "article", "news");
        for item in [&together_a, &together_b, &later, &manual] {
            store.upsert_feed(item).unwrap();
        }

        for item in [&together_a, &together_b, &later] {
            store
                .record_feed_origin(
                    &item.id,
                    "gh-trending",
                    "https://github.com/trending",
                    Some("GitHub Trending (daily)"),
                )
                .unwrap();
        }

        // Push one item's arrival two hours back: same source, different run.
        store
            .conn()
            .unwrap()
            .execute(
                &format!(
                    "UPDATE {schema}.feed_origins SET first_seen = first_seen - interval '2 hours' WHERE feed_id = $1"
                ),
                &[&later.id],
            )
            .unwrap();

        let runs = store.list_feed_runs(7).unwrap();
        let key_of = |id: &str| {
            runs.iter()
                .find(|r| r.feed_id == id)
                .map(|r| r.run_key.clone())
        };

        assert_eq!(
            key_of(&together_a.id),
            key_of(&together_b.id),
            "items that arrived together share a run"
        );
        assert_ne!(
            key_of(&together_a.id),
            key_of(&later.id),
            "an arrival two hours later is a different run"
        );
        assert_eq!(
            key_of(&manual.id),
            None,
            "an item with no origin is ungrouped"
        );
        assert!(runs
            .iter()
            .all(|r| r.label.as_deref() == Some("GitHub Trending (daily)")));
    }

    #[test]
    fn capture_provenance_follows_the_body_it_describes() {
        let (store, _schema) = open_test_store("captured_via");

        let mut captured = mk_feed("https://example.com/members", "article", "news");
        captured.captured_via = Some("axon-clip".into());
        store.upsert_feed(&captured).unwrap();
        assert_eq!(
            store
                .get_feed(&captured.id)
                .unwrap()
                .unwrap()
                .captured_via
                .as_deref(),
            Some("axon-clip")
        );

        // A later server-side fetch that yields nothing must not relabel a
        // captured body as fetched — the column describes the stored content.
        let mut empty_refetch = FeedItem::new("https://example.com/members", "news", "article");
        empty_refetch.transcript = None;
        store.upsert_feed(&empty_refetch).unwrap();
        assert_eq!(
            store
                .get_feed(&captured.id)
                .unwrap()
                .unwrap()
                .captured_via
                .as_deref(),
            Some("axon-clip"),
            "an empty re-fetch left the old body but took its provenance"
        );

        // A fetch that DOES replace the body owns the provenance again.
        let mut real_refetch = FeedItem::new("https://example.com/members", "news", "article");
        real_refetch.transcript = Some("server-fetched body".into());
        store.upsert_feed(&real_refetch).unwrap();
        assert_eq!(
            store.get_feed(&captured.id).unwrap().unwrap().captured_via,
            None,
            "content the server fetched must not still claim to be a capture"
        );
    }

    #[test]
    fn transcript_source_round_trips_and_is_not_relabelled_by_an_empty_refetch() {
        let (store, _schema) = open_test_store("transcript_source");

        // A paper stored as its abstract, which is what every arXiv item is
        // until a PDF extractor is registered (#78).
        let mut item = mk_feed("https://arxiv.org/abs/2501.00001", "arxiv", "news");
        item.transcript = Some("We show that ...".into());
        item.transcript_source = "abstract".into();
        store.upsert_feed(&item).unwrap();
        assert_eq!(
            store.get_feed(&item.id).unwrap().unwrap().transcript_source,
            "abstract"
        );

        // A re-fetch that brings back nothing must not relabel it: the field
        // describes the text actually stored, and the stored text did not
        // change. Same guard content_status is under.
        let mut empty = item.clone();
        empty.transcript = None;
        empty.transcript_source = "full-text".into();
        store.upsert_feed(&empty).unwrap();
        assert_eq!(
            store.get_feed(&item.id).unwrap().unwrap().transcript_source,
            "abstract",
            "an empty re-fetch relabelled an abstract as full text"
        );

        // A re-fetch that DOES bring the paper back relabels it, because now
        // the stored text really is the document.
        let mut full = item.clone();
        full.transcript = Some("1 Introduction ...".into());
        full.transcript_source = "full-text".into();
        store.upsert_feed(&full).unwrap();
        assert_eq!(
            store.get_feed(&item.id).unwrap().unwrap().transcript_source,
            "full-text"
        );

        // Legacy rows predate the distinction and stay unknown rather than
        // being backfilled into a claim nobody checked.
        let legacy = mk_feed("https://example.com/legacy", "article", "news");
        store.upsert_feed(&legacy).unwrap();
        assert_eq!(
            store
                .get_feed(&legacy.id)
                .unwrap()
                .unwrap()
                .transcript_source,
            "unknown"
        );
    }

    #[test]
    fn raw_content_is_retained_beside_the_normalized_transcript() {
        let (store, _schema) = open_test_store("raw_content");
        let mut item = mk_feed("https://example.com/raw", "article", "news");
        item.raw_content = Some("Menu\n\nThe body.".into());
        item.transcript = Some("The body.".into());
        store.upsert_feed(&item).unwrap();

        assert_eq!(
            store.get_raw_content(&item.id).unwrap().as_deref(),
            Some("Menu\n\nThe body.")
        );
        assert_eq!(
            store
                .get_feed(&item.id)
                .unwrap()
                .unwrap()
                .transcript
                .as_deref(),
            Some("The body.")
        );
        assert_eq!(
            store.feed_ids_with_raw_content().unwrap(),
            vec![item.id.clone()]
        );

        // The point of retention: a rule change rewrites the body from stored
        // raw, and the raw itself is untouched so it can be done again.
        store
            .set_normalized(&item.id, Some("Rewritten."), "thin")
            .unwrap();
        let after = store.get_feed(&item.id).unwrap().unwrap();
        assert_eq!(after.transcript.as_deref(), Some("Rewritten."));
        assert_eq!(after.content_status, "thin");
        assert_eq!(
            store.get_raw_content(&item.id).unwrap().as_deref(),
            Some("Menu\n\nThe body."),
            "re-normalizing must never disturb the extractor's output"
        );
    }

    fn mk_digest(source: &str, item_id: &str, producer: &str) -> StoredDigest {
        StoredDigest {
            source: source.into(),
            item_id: item_id.into(),
            text: Some("- A point\n- Another".into()),
            state: "generated".into(),
            shape: "brief".into(),
            depth: "standard".into(),
            focus: String::new(),
            producer: producer.into(),
            source_chars: 1_200,
            redactions: 0,
            attempts: 0,
            last_error: None,
            diagram: None,
            diagram_state: None,
            diagram_error: None,
            chart: None,
            chart_state: None,
            chart_error: None,
            generated_at: String::new(),
        }
    }

    #[test]
    fn content_digest_round_trips_and_replaces_in_place() {
        let (store, _schema) = open_test_store("digest_round_trip");
        let item = mk_feed("https://example.com/digested", "article", "news");
        store.upsert_feed(&item).unwrap();
        assert!(store.content_digest("feed", &item.id).unwrap().is_none());

        store
            .upsert_content_digest(&mk_digest("feed", &item.id, "p1"))
            .unwrap();
        let stored = store.content_digest("feed", &item.id).unwrap().unwrap();
        assert_eq!(stored.state, "generated");
        assert_eq!(stored.shape, "brief");
        assert!(
            !stored.generated_at.is_empty(),
            "the row stamps its own time"
        );

        // Replace in place: a refine overwrites rather than appending, and the
        // directive that produced it comes back with it.
        let mut refined = mk_digest("feed", &item.id, "p1");
        refined.text = Some("## Method\n- Deeper".into());
        refined.shape = "sectioned".into();
        refined.depth = "detailed".into();
        refined.focus = "cost, latency".into();
        store.upsert_content_digest(&refined).unwrap();
        let after = store.content_digest("feed", &item.id).unwrap().unwrap();
        assert_eq!(after.depth, "detailed");
        assert_eq!(after.focus, "cost, latency");
        assert_eq!(after.text.as_deref(), Some("## Method\n- Deeper"));

        // The diagram is a separate press and updates without touching the text.
        assert_eq!(
            store
                .update_content_diagram(
                    "feed",
                    &item.id,
                    Some("flowchart TD\n  A --> B"),
                    "generated",
                    None,
                    "d1"
                )
                .unwrap(),
            1
        );
        let with_diagram = store.content_digest("feed", &item.id).unwrap().unwrap();
        assert_eq!(
            with_diagram.diagram.as_deref(),
            Some("flowchart TD\n  A --> B")
        );
        assert_eq!(with_diagram.text.as_deref(), Some("## Method\n- Deeper"));
    }

    /// The automatic pass must never overwrite a digest an operator asked for.
    /// A model upgrade changes the producer on every row, and without the
    /// `depth = 'standard'` guard that upgrade would silently throw away every
    /// refinement in the store.
    #[test]
    fn the_automatic_pass_leaves_an_operators_refinement_alone() {
        let (store, _schema) = open_test_store("digest_queue");
        let missing = mk_feed("https://example.com/no-digest", "article", "news");
        let stale = mk_feed("https://example.com/stale-digest", "article", "news");
        let refined = mk_feed("https://example.com/refined-digest", "article", "news");
        let current = mk_feed("https://example.com/current-digest", "article", "news");
        let parked = mk_feed("https://example.com/parked-digest", "article", "news");
        for item in [&missing, &stale, &refined, &current, &parked] {
            store.upsert_feed(item).unwrap();
        }

        // A list, because the role is chosen per item: one machine can hold a
        // light model for short sources and a strong one for long sources, and
        // a digest from either is current.
        let producers = vec!["current-producer".to_string()];

        store
            .upsert_content_digest(&mk_digest("feed", &stale.id, "old-producer"))
            .unwrap();
        let mut refined_row = mk_digest("feed", &refined.id, "old-producer");
        refined_row.depth = "detailed".into();
        store.upsert_content_digest(&refined_row).unwrap();
        store
            .upsert_content_digest(&mk_digest("feed", &current.id, "current-producer"))
            .unwrap();
        let mut parked_row = mk_digest("feed", &parked.id, "current-producer");
        parked_row.state = "timeout".into();
        parked_row.text = None;
        parked_row.attempts = 3;
        store.upsert_content_digest(&parked_row).unwrap();

        let queued = store
            .items_needing_digest("feed", &producers, 3, 50)
            .unwrap();
        assert!(queued.contains(&missing.id), "no digest at all");
        assert!(queued.contains(&stale.id), "produced by an older model");
        assert!(
            !queued.contains(&refined.id),
            "an operator's detailed digest must survive a model change"
        );
        assert!(!queued.contains(&current.id), "already current");
        assert!(
            !queued.contains(&parked.id),
            "a row at the attempt cap is parked, not retried forever"
        );

        // One retry left is no longer enough on its own: writing the failure
        // arms a backoff, and the row stays parked until that window elapses.
        parked_row.attempts = 2;
        store.upsert_content_digest(&parked_row).unwrap();
        assert!(
            !store
                .items_needing_digest("feed", &producers, 3, 50)
                .unwrap()
                .contains(&parked.id),
            "a failure just written is inside its own backoff window"
        );

        // Once it has, the retry is due.
        expire_digest_backoff(&store, "feed", &parked.id);
        assert!(store
            .items_needing_digest("feed", &producers, 3, 50)
            .unwrap()
            .contains(&parked.id));

        assert!(store
            .items_needing_digest("scouting", &producers, 3, 50)
            .is_err());
    }

    /// Move a digest's retry deadline into the past.
    ///
    /// Raw SQL rather than a field on `StoredDigest`: the deadline is derived
    /// from the database's own clock at write time precisely so no caller can
    /// set it, and adding a settable field to production code to serve tests
    /// would give that guarantee away. Tests live inside this module, so they
    /// can reach the connection without one.
    fn expire_digest_backoff(store: &Store, source: &str, item_id: &str) {
        let schema = store.schema.clone();
        store
            .conn()
            .unwrap()
            .execute(
                &format!(
                    "UPDATE {schema}.content_digests
                        SET next_attempt = now() - interval '1 hour'
                      WHERE source = $1 AND item_id = $2"
                ),
                &[&source, &item_id],
            )
            .unwrap();
    }

    /// Concurrent openers must not deadlock.
    ///
    /// `Store::open` used to migrate every time it was called, and the comms
    /// server calls it from several timers plus every HTTP handler. Three of
    /// those timers share a fifteen-minute period, so "two sessions migrate at
    /// the same instant" was the normal case rather than the rare one, and it
    /// produced `deadlock detected` in the drain log roughly every other pass.
    ///
    /// Migration now runs once per process per (database, schema), so seven of
    /// these eight threads do no DDL at all. Kept at eight anyway: this is the
    /// shape that failed reliably before, and it is the only place the
    /// in-process guard is exercised under real contention rather than in
    /// isolation (libs/axon-store has the isolated cases).
    #[test]
    fn concurrent_openers_do_not_deadlock() {
        let schema = format!("comms_test_open_race_{}", std::process::id());
        let url = test_database_url();
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let url = url.clone();
                let schema = schema.clone();
                // Flattened to a String inside the thread: `Box<dyn Error>` is
                // not Send, and the detail lives on the source chain anyway —
                // `postgres::Error` Displays as the bare words "db error", so
                // joining the causes is the only way this failure names itself.
                std::thread::spawn(move || {
                    Store::open_with_schema(&url, &schema)
                        .map(|_| ())
                        .map_err(|error| {
                            let mut text = error.to_string();
                            let mut cause = error.source();
                            while let Some(next) = cause {
                                text.push_str(&format!(": {next}"));
                                cause = next.source();
                            }
                            text
                        })
                })
            })
            .collect();

        let failures: Vec<String> = threads
            .into_iter()
            .filter_map(|t| t.join().expect("opener panicked").err())
            .collect();
        drop_test_schema(&schema);
        assert!(
            failures.is_empty(),
            "every concurrent open must succeed, got: {failures:?}"
        );
    }

    /// Twenty opens are not twenty connections.
    ///
    /// The whole point of the pool, asserted against r2d2's own count rather than
    /// against a stopwatch: a latency assertion on a shared database is a flake
    /// generator, while "how many sessions did this open" is the thing that
    /// actually changed.
    ///
    /// The bound is the pool ceiling rather than an exact number, because the test
    /// binary runs cases in parallel against one process-wide pool per URL and the
    /// others contribute connections too. It is still the claim that matters:
    /// before this, twenty opens meant twenty connect-and-authenticate round trips,
    /// and there was no ceiling at all.
    #[test]
    fn many_opens_share_one_pool() {
        let (store, schema) = open_test_store("pool_shared");
        let url = test_database_url();

        let opened: Vec<Store> = (0..20)
            .map(|_| Store::open_with_schema(&url, &schema.0).expect("open"))
            .collect();
        assert_eq!(opened.len(), 20);

        let connections = store.pool.state().connections;
        assert!(
            connections <= 10,
            "20 opens produced {connections} connections; the pool ceiling is 10"
        );
    }

    /// The second open of a schema this process already migrated does no DDL.
    ///
    /// Asserted by removing the schema behind the process's back: an `open` that
    /// still migrated would put the tables straight back, and the read below
    /// would succeed. It failing is the proof that the DDL ran exactly once.
    ///
    /// That is also the honest cost of the design, which is why it is pinned
    /// rather than left implicit. Rebuilding a schema dropped underneath a live
    /// process would mean DDL on every open again, and DDL on every open is the
    /// deadlock this replaced.
    #[test]
    fn a_second_open_does_no_ddl() {
        let schema = format!("comms_test_migrate_once_{}", std::process::id());
        let url = test_database_url();

        let first = Store::open_with_schema(&url, &schema).expect("first open migrates");
        first
            .list_feed(None, None, 1, false)
            .expect("the first open must leave a usable schema behind");
        drop(first);

        drop_test_schema(&schema);

        let second =
            Store::open_with_schema(&url, &schema).expect("the second open still connects");
        let read = second.list_feed(None, None, 1, false);
        drop_test_schema(&schema);

        assert!(
            read.is_err(),
            "the second open re-ran the migration; it must trust the first"
        );
    }

    /// The backoff is what makes a *timed* drain safe. With the attempt cap
    /// alone, a drain every 15 minutes spends all three attempts inside
    /// three-quarters of an hour, so an outage lasting an hour would leave the
    /// row permanently dead — the exact failure the drain exists to end.
    #[test]
    fn a_retryable_failure_waits_out_a_growing_backoff() {
        let (store, _schema) = open_test_store("digest_backoff");
        let item = mk_feed("https://example.com/backoff-digest", "article", "news");
        store.upsert_feed(&item).unwrap();

        let mut row = mk_digest("feed", &item.id, "current-producer");
        row.state = "empty_response".into();
        row.text = None;

        // Each successive failure parks the row for longer: 5, 10, then 20
        // minutes. Read back as a delta so this asserts the ladder, not a clock.
        let mut previous = 0_f64;
        for attempt in 1..=3 {
            row.attempts = attempt;
            store.upsert_content_digest(&row).unwrap();
            let seconds = backoff_seconds(&store, "feed", &item.id)
                .expect("a retryable state arms a deadline");
            assert!(
                seconds > previous,
                "attempt {attempt} must wait longer than the one before it \
                 ({seconds}s vs {previous}s)"
            );
            previous = seconds;
        }

        // A success clears the deadline outright — there is no next attempt to
        // schedule, and a stale one left behind would park a healthy row.
        row.state = "generated".into();
        row.text = Some("- A point".into());
        row.attempts = 0;
        store.upsert_content_digest(&row).unwrap();
        assert!(
            backoff_seconds(&store, "feed", &item.id).is_none(),
            "a generated digest carries no retry deadline"
        );
    }

    /// Seconds from now until the row's retry deadline, or None when it has none.
    fn backoff_seconds(store: &Store, source: &str, item_id: &str) -> Option<f64> {
        let schema = store.schema.clone();
        let row = store
            .conn()
            .unwrap()
            .query_one(
                &format!(
                    "SELECT EXTRACT(EPOCH FROM (next_attempt - now()))::float8
                       FROM {schema}.content_digests
                      WHERE source = $1 AND item_id = $2"
                ),
                &[&source, &item_id],
            )
            .unwrap();
        row.get::<_, Option<f64>>(0)
    }

    #[test]
    fn feed_pending_summaries_includes_stale_model_output() {
        let (store, _schema) = open_test_store("feed_pending");
        let with_t = mk_feed("https://youtu.be/hastranscript", "youtube", "media");
        store.upsert_feed(&with_t).unwrap();
        let mut no_t = FeedItem::new("https://example.com/no-transcript", "news", "article");
        no_t.transcript = None;
        store.upsert_feed(&no_t).unwrap();
        let stale = mk_feed("https://example.com/stale-summary", "article", "news");
        store.upsert_feed(&stale).unwrap();
        store
            .update_feed_summary(&stale.id, "Old model output", "summary-v1")
            .unwrap();
        let current = mk_feed("https://example.com/current-summary", "article", "news");
        store.upsert_feed(&current).unwrap();
        store
            .update_feed_summary(&current.id, "Current model output", "summary-v2")
            .unwrap();
        let mut legacy = mk_feed("https://example.com/legacy-summary", "article", "news");
        legacy.summary = Some("Historical generated output".into());
        store.upsert_feed(&legacy).unwrap();
        store
            .conn()
            .unwrap()
            .execute(
                &format!(
                    "UPDATE {}.feed_items SET summary_revision = 'legacy-unknown' WHERE id = $1",
                    store.schema
                ),
                &[&legacy.id],
            )
            .unwrap();

        let pending = store.feed_pending_summaries(Some("summary-v2")).unwrap();
        let ids = pending
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            ids.len(),
            3,
            "missing, stale model, and pre-provenance summaries need work"
        );
        assert!(ids.contains(&with_t.id.as_str()));
        assert!(ids.contains(&stale.id.as_str()));
        assert!(ids.contains(&legacy.id.as_str()));
        assert!(!ids.contains(&current.id.as_str()));
        assert!(store
            .feed_summary_needs_revision(&stale.id, "summary-v2")
            .unwrap());
        assert!(!store
            .feed_summary_needs_revision(&current.id, "summary-v2")
            .unwrap());
        assert!(store
            .feed_summary_needs_revision(&legacy.id, "summary-v2")
            .unwrap());
    }

    #[test]
    fn source_state_round_trip() {
        let (store, _schema) = open_test_store("source_state");
        assert!(store.get_source_state("gmail").unwrap().is_none());
        store.record_run("gmail", Some("cur-1")).unwrap();
        let st = store.get_source_state("gmail").unwrap().unwrap();
        assert_eq!(st.cursor.as_deref(), Some("cur-1"));
        store.record_run("gmail", None).unwrap();
        let st2 = store.get_source_state("gmail").unwrap().unwrap();
        assert_eq!(
            st2.cursor.as_deref(),
            Some("cur-1"),
            "cursor preserved when not given"
        );
    }

    /// A failing streak must not erase the last time collection actually
    /// worked: "last success" is the number that tells a human whether a red
    /// schedule is an outage or a five-minute blip.
    #[test]
    fn a_failure_streak_preserves_the_last_success_and_recovery_clears_it() {
        let (store, _schema) = open_test_store("sweep_outcome");

        store.record_sweep_success("gmail-inbox", 25, 3).unwrap();
        let ok = store.get_source_state("gmail-inbox").unwrap().unwrap();
        let first_success = ok.last_success_at.clone().expect("success is recorded");
        assert_eq!((ok.considered_count, ok.new_count), (25, 3));
        assert_eq!(ok.consecutive_failures, 0);
        assert!(ok.last_error.is_none());

        assert_eq!(
            store.record_sweep_failure("gmail-inbox", "auth").unwrap(),
            1
        );
        assert_eq!(
            store.record_sweep_failure("gmail-inbox", "quota").unwrap(),
            2
        );
        let failing = store.get_source_state("gmail-inbox").unwrap().unwrap();
        assert_eq!(failing.consecutive_failures, 2);
        assert_eq!(failing.last_error.as_deref(), Some("quota"));
        assert_eq!(
            failing.last_success_at.as_deref(),
            Some(first_success.as_str()),
            "a failing run must not overwrite when collection last worked"
        );
        assert!(failing.last_failure_at.is_some());

        store.record_sweep_success("gmail-inbox", 25, 0).unwrap();
        let recovered = store.get_source_state("gmail-inbox").unwrap().unwrap();
        assert_eq!(
            recovered.consecutive_failures, 0,
            "success clears the streak"
        );
        assert!(recovered.last_error.is_none(), "and clears the error class");
        assert!(
            recovered.last_failure_at.is_some(),
            "but keeps that a failure happened"
        );
    }

    /// The window that wraps midnight is the one people actually configure, so
    /// it is the one worth a test. Uses the store's own clock, so the assertion
    /// is on the two windows that must always disagree.
    #[test]
    fn quiet_hours_wrap_midnight() {
        let (store, _schema) = open_test_store("quiet_hours");
        let all_day = store.within_quiet_hours(0, 23).unwrap();
        let inverse = store.within_quiet_hours(23, 0).unwrap();
        assert_ne!(
            all_day, inverse,
            "a window and its complement cannot both hold at one instant"
        );
        assert!(
            !store.within_quiet_hours(9, 9).unwrap(),
            "an empty window is never quiet"
        );
    }
}
