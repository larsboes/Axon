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

use postgres::types::ToSql;
use postgres::{Client, NoTls};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Mutex;

use crate::evaluation::{EvaluationFactor, EvaluationFactorContext, FeedEvaluation};
use crate::provenance::{self, StageProvenance};
use crate::quality::QualityFlag;
use crate::relevance::RelevanceMatch;

pub struct Store {
    conn: Mutex<Client>,
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
}

impl FeedItem {
    /// Build a fresh item for ingest. DB-owned fields are left blank/defaulted.
    pub fn new(url: &str, stream: &str, kind: &str) -> Self {
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
        }
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

impl Store {
    pub fn open(database_url: &str) -> Result<Self, Box<dyn std::error::Error>> {
        Self::open_with_schema(database_url, "comms")
    }

    /// `schema` is always either the literal `"comms"` (production) or a
    /// test-generated name built from a static prefix + pid -- never user
    /// input. Postgres has no parametrized-identifier syntax for CREATE
    /// SCHEMA/TABLE, so schema-qualified names are built via `format!`; that is
    /// safe specifically because the schema name's origin is one of those two
    /// controlled cases, not because SQL interpolation is safe in general.
    fn open_with_schema(
        database_url: &str,
        schema: &str,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let mut client = Client::connect(database_url, NoTls)?;
        Self::init_schema(&mut client, schema)?;
        Ok(Self {
            conn: Mutex::new(client),
            schema: schema.to_string(),
        })
    }

    fn init_schema(client: &mut Client, schema: &str) -> Result<(), Box<dyn std::error::Error>> {
        client.batch_execute(&format!(
            "
            CREATE SCHEMA IF NOT EXISTS {schema};

            CREATE TABLE IF NOT EXISTS {schema}.triage_items (
                id TEXT PRIMARY KEY,
                from_addr TEXT,
                subject TEXT,
                snippet TEXT,
                internal_date TIMESTAMPTZ,
                stream TEXT NOT NULL CHECK (stream IN ('aktiv','issue','feed','werbung','belege','steuern','sonstiges')),
                rationale TEXT NOT NULL,
                classification_method TEXT NOT NULL DEFAULT 'rules',
                classification_version TEXT NOT NULL DEFAULT 'mail-rules-v1',
                data_class TEXT NOT NULL DEFAULT 'personal' CHECK (data_class IN ('public','personal','vault')),
                data_class_rationale TEXT NOT NULL DEFAULT 'Mail metadata is Personal by default.',
                data_classification_method TEXT NOT NULL DEFAULT 'rules' CHECK (data_classification_method IN ('rules','human')),
                data_classification_version TEXT NOT NULL DEFAULT 'data-class-rules-v1',
                status TEXT NOT NULL DEFAULT 'proposed' CHECK (status IN ('proposed','approved','executed','dismissed')),
                gmail_action TEXT,
                gmail_action_at TIMESTAMPTZ,
                purge_after TIMESTAMPTZ,
                gmail_location TEXT,
                gmail_observed_at TIMESTAMPTZ,
                gmail_sync_status TEXT,
                gmail_sync_error TEXT,
                first_seen TIMESTAMPTZ NOT NULL,
                last_seen TIMESTAMPTZ NOT NULL
            );

            CREATE TABLE IF NOT EXISTS {schema}.feed_items (
                id TEXT PRIMARY KEY,
                stream TEXT NOT NULL CHECK (stream IN ('news','media')),
                kind TEXT NOT NULL CHECK (kind IN ('youtube','instagram','podcast','article','mail')),
                title TEXT,
                url TEXT NOT NULL,
                author TEXT,
                summary TEXT,
                transcript TEXT,
                day DATE NOT NULL,
                created_at TIMESTAMPTZ NOT NULL,
                status TEXT NOT NULL DEFAULT 'new' CHECK (status IN ('new','keeper','dismissed'))
            );

            CREATE TABLE IF NOT EXISTS {schema}.source_state (
                source_name TEXT PRIMARY KEY,
                last_run_at TEXT,
                cursor TEXT
            );

            -- Outcome of the last pass, so an unattended schedule can be read
            -- rather than trusted. Counts and an error class only: a scheduler
            -- log that quotes a subject is the same leak the sweep gate closes.
            ALTER TABLE {schema}.source_state
                ADD COLUMN IF NOT EXISTS last_success_at TEXT;
            ALTER TABLE {schema}.source_state
                ADD COLUMN IF NOT EXISTS last_failure_at TEXT;
            ALTER TABLE {schema}.source_state
                ADD COLUMN IF NOT EXISTS last_error TEXT;
            ALTER TABLE {schema}.source_state
                ADD COLUMN IF NOT EXISTS considered_count BIGINT NOT NULL DEFAULT 0;
            ALTER TABLE {schema}.source_state
                ADD COLUMN IF NOT EXISTS new_count BIGINT NOT NULL DEFAULT 0;
            ALTER TABLE {schema}.source_state
                ADD COLUMN IF NOT EXISTS consecutive_failures INTEGER NOT NULL DEFAULT 0;

            CREATE TABLE IF NOT EXISTS {schema}.feed_relevance (
                feed_id TEXT NOT NULL REFERENCES {schema}.feed_items(id) ON DELETE CASCADE,
                profile_key TEXT NOT NULL,
                profile_label TEXT NOT NULL,
                score DOUBLE PRECISION NOT NULL,
                rationale TEXT NOT NULL,
                mode TEXT NOT NULL CHECK (mode IN ('reranked','semantic','lexical')),
                profile_revision TEXT NOT NULL,
                scored_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                PRIMARY KEY (feed_id, profile_key)
            );

            CREATE TABLE IF NOT EXISTS {schema}.triage_relevance (
                triage_id TEXT NOT NULL REFERENCES {schema}.triage_items(id) ON DELETE CASCADE,
                profile_key TEXT NOT NULL,
                profile_label TEXT NOT NULL,
                score DOUBLE PRECISION NOT NULL,
                rationale TEXT NOT NULL,
                mode TEXT NOT NULL CHECK (mode IN ('reranked','semantic','lexical')),
                profile_revision TEXT NOT NULL,
                scored_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                PRIMARY KEY (triage_id, profile_key)
            );

            CREATE TABLE IF NOT EXISTS {schema}.feed_origins (
                feed_id TEXT NOT NULL REFERENCES {schema}.feed_items(id) ON DELETE CASCADE,
                source_id TEXT NOT NULL,
                source_ref TEXT NOT NULL,
                label TEXT,
                first_seen TIMESTAMPTZ NOT NULL DEFAULT now(),
                last_seen TIMESTAMPTZ NOT NULL DEFAULT now(),
                PRIMARY KEY (feed_id, source_id, source_ref)
            );

            -- #86: extraction output, kept beside the normalized transcript so a
            -- normalization rule change re-runs from here instead of re-fetching.
            -- Its own table, not a column: feed list queries have no business
            -- dragging 20k-character bodies they never read.
            CREATE TABLE IF NOT EXISTS {schema}.feed_raw_content (
                feed_id TEXT PRIMARY KEY REFERENCES {schema}.feed_items(id) ON DELETE CASCADE,
                raw TEXT NOT NULL,
                tier TEXT NOT NULL DEFAULT 'legacy',
                revision TEXT NOT NULL DEFAULT 'legacy-unknown',
                extracted_at TIMESTAMPTZ NOT NULL DEFAULT now()
            );

            CREATE TABLE IF NOT EXISTS {schema}.feed_evaluations (
                feed_id TEXT PRIMARY KEY REFERENCES {schema}.feed_items(id) ON DELETE CASCADE,
                overall_score DOUBLE PRECISION NOT NULL CHECK (overall_score BETWEEN 0 AND 1),
                explanation TEXT NOT NULL,
                mode TEXT NOT NULL CHECK (mode IN ('reranked','semantic','lexical','unscored')),
                item_revision TEXT NOT NULL,
                context_revision TEXT NOT NULL,
                evaluator_revision TEXT NOT NULL,
                tier TEXT NOT NULL DEFAULT 'legacy',
                evaluated_at TIMESTAMPTZ NOT NULL DEFAULT now()
            );

            CREATE TABLE IF NOT EXISTS {schema}.feed_evaluation_factors (
                feed_id TEXT NOT NULL REFERENCES {schema}.feed_evaluations(feed_id) ON DELETE CASCADE,
                factor_key TEXT NOT NULL,
                label TEXT NOT NULL,
                score DOUBLE PRECISION NOT NULL CHECK (score BETWEEN 0 AND 1),
                weight DOUBLE PRECISION NOT NULL CHECK (weight BETWEEN 0 AND 1),
                rationale TEXT NOT NULL,
                context_json TEXT,
                position INTEGER NOT NULL,
                PRIMARY KEY (feed_id, factor_key)
            );

            CREATE TABLE IF NOT EXISTS {schema}.feed_context_snapshots (
                context_kind TEXT PRIMARY KEY,
                revision TEXT NOT NULL,
                payload TEXT NOT NULL,
                refreshed_at TIMESTAMPTZ NOT NULL DEFAULT now()
            );

            -- #79: deterministic suggestions for the human review queue. The
            -- computation replaces this set explicitly; reading it has no side
            -- effects and never invokes an inference provider.
            CREATE TABLE IF NOT EXISTS {schema}.feed_quality_flags (
                feed_id TEXT NOT NULL REFERENCES {schema}.feed_items(id) ON DELETE CASCADE,
                signal TEXT NOT NULL,
                reason TEXT NOT NULL,
                evidence TEXT NOT NULL,
                derived_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                PRIMARY KEY (feed_id, signal)
            );

            -- What the local model wrote about an item, for every source.
            --
            -- One table rather than a summary column on each item table.
            -- libs/content-item keeps *storage* apart because the items have
            -- genuinely different invariants -- calendar's exclusive ends_at,
            -- mail's retention window -- and a merged table could enforce
            -- neither. A digest has none of those: it is derived data with the
            -- same axes and the same refine action everywhere, so three
            -- migrations and three upsert paths would buy nothing but drift.
            --
            -- `text` is null unless `state` = 'generated'. `shape` is the rung
            -- the length ladder landed on and `depth` records whether an
            -- operator asked for one more; both are stored rather than
            -- re-derived, because source_chars alone cannot tell you that a
            -- short item was digested on purpose.
            --
            -- No raw source is kept here. A mail body is fetched, digested and
            -- dropped inside one call; this row is the only thing that survives
            -- it.
            CREATE TABLE IF NOT EXISTS {schema}.content_digests (
                source TEXT NOT NULL CHECK (source IN ('feed','mail','calendar')),
                item_id TEXT NOT NULL,
                text TEXT,
                state TEXT NOT NULL,
                shape TEXT NOT NULL CHECK (shape IN ('none','brief','standard','sectioned')),
                depth TEXT NOT NULL DEFAULT 'standard' CHECK (depth IN ('standard','detailed')),
                focus TEXT NOT NULL DEFAULT '',
                producer TEXT NOT NULL,
                source_chars BIGINT NOT NULL DEFAULT 0 CHECK (source_chars >= 0),
                redactions INTEGER NOT NULL DEFAULT 0 CHECK (redactions >= 0),
                attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
                last_error TEXT,
                diagram TEXT,
                diagram_state TEXT,
                diagram_error TEXT,
                diagram_producer TEXT,
                -- The extracted table, as chart-data JSON. Not a Vega-Lite
                -- spec: the reader compiles one, which is what keeps the model
                -- out of the rendering layer. Every value in here appeared
                -- verbatim in the source -- see libs/summarize/src/chart.rs.
                chart TEXT,
                chart_state TEXT,
                chart_error TEXT,
                chart_producer TEXT,
                generated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                PRIMARY KEY (source, item_id)
            );

            -- Added after content_digests first shipped. `CREATE TABLE IF NOT
            -- EXISTS` above never touches an installed table, so the block
            -- alone works on a fresh database and silently leaves every
            -- existing one without these columns -- the same trap the
            -- feed_items `kind` constraint documents (README.md#schema).
            ALTER TABLE {schema}.content_digests
                ADD COLUMN IF NOT EXISTS chart TEXT;
            ALTER TABLE {schema}.content_digests
                ADD COLUMN IF NOT EXISTS chart_state TEXT;
            ALTER TABLE {schema}.content_digests
                ADD COLUMN IF NOT EXISTS chart_error TEXT;
            ALTER TABLE {schema}.content_digests
                ADD COLUMN IF NOT EXISTS chart_producer TEXT;

            -- A human-approved derivative is staged locally before a cloud job
            -- can consume it. Staging has no provider identity or side effect.
            CREATE TABLE IF NOT EXISTS {schema}.content_cloud_derivatives (
                source TEXT NOT NULL CHECK (source IN ('feed','mail')),
                item_id TEXT NOT NULL,
                source_revision TEXT NOT NULL,
                preview_hash TEXT NOT NULL,
                original_data_class TEXT NOT NULL CHECK (original_data_class IN ('public','personal','vault')),
                derivative_data_class TEXT NOT NULL CHECK (derivative_data_class IN ('public','personal')),
                transformation TEXT NOT NULL,
                document TEXT NOT NULL,
                redaction_count INTEGER NOT NULL CHECK (redaction_count >= 0),
                approved_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                PRIMARY KEY (source, item_id)
            );

            -- One reviewed cloud intent and its bounded execution ledger. The
            -- joined derivative, never the original source, is the provider input.
            CREATE TABLE IF NOT EXISTS {schema}.content_cloud_jobs (
                job_id TEXT PRIMARY KEY,
                source TEXT NOT NULL CHECK (source IN ('feed','mail')),
                item_id TEXT NOT NULL,
                source_revision TEXT NOT NULL,
                preview_hash TEXT NOT NULL,
                provider_role TEXT NOT NULL CHECK (provider_role LIKE 'cloud\\_%' ESCAPE '\\'),
                task TEXT NOT NULL DEFAULT 'content-analysis-v1',
                status TEXT NOT NULL DEFAULT 'queued' CHECK (status IN ('queued','running','succeeded','failed')),
                queued_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                provider_calls INTEGER NOT NULL DEFAULT 0 CHECK (provider_calls BETWEEN 0 AND 5),
                started_at TIMESTAMPTZ,
                completed_at TIMESTAMPTZ,
                last_error TEXT,
                result_json TEXT,
                UNIQUE (source, item_id, preview_hash, provider_role)
            );

            -- One row per actual provider request. Policy-disabled candidates
            -- never enter this ledger because no request was made. The exact
            -- approved hash follows every attempt, including failover.
            CREATE TABLE IF NOT EXISTS {schema}.content_cloud_attempts (
                attempt_id BIGSERIAL PRIMARY KEY,
                job_id TEXT NOT NULL REFERENCES {schema}.content_cloud_jobs(job_id) ON DELETE CASCADE,
                sequence INTEGER NOT NULL CHECK (sequence BETWEEN 1 AND 5),
                provider_role TEXT NOT NULL CHECK (provider_role LIKE 'cloud\\_%' ESCAPE '\\'),
                model TEXT NOT NULL,
                preview_hash TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'running' CHECK (status IN ('running','succeeded','failed')),
                started_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                completed_at TIMESTAMPTZ,
                last_error TEXT,
                result_json TEXT,
                UNIQUE (job_id, sequence)
            );
            CREATE INDEX IF NOT EXISTS content_cloud_attempts_role_started_idx
                ON {schema}.content_cloud_attempts(provider_role, started_at);

            -- Durable intent for Gmail mutations. The thread id is already the
            -- triage primary key; no message content is copied into this ledger.
            CREATE TABLE IF NOT EXISTS {schema}.gmail_action_jobs (
                job_id BIGSERIAL PRIMARY KEY,
                triage_id TEXT NOT NULL REFERENCES {schema}.triage_items(id) ON DELETE CASCADE,
                action TEXT NOT NULL CHECK (action IN ('archive','trash','restore')),
                source_status TEXT NOT NULL,
                state TEXT NOT NULL DEFAULT 'queued' CHECK (state IN ('queued','completed','abandoned','canceled')),
                attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts BETWEEN 0 AND 5),
                last_error TEXT,
                next_attempt TIMESTAMPTZ NOT NULL DEFAULT now(),
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                completed_at TIMESTAMPTZ
            );

            ALTER TABLE {schema}.feed_evaluation_factors
                ADD COLUMN IF NOT EXISTS context_json TEXT;

            ALTER TABLE {schema}.triage_items
                ADD COLUMN IF NOT EXISTS classification_method TEXT NOT NULL DEFAULT 'rules';
            ALTER TABLE {schema}.triage_items
                ADD COLUMN IF NOT EXISTS classification_version TEXT NOT NULL DEFAULT 'mail-rules-v1';
            ALTER TABLE {schema}.triage_items
                DROP CONSTRAINT IF EXISTS triage_items_classification_method_check;
            ALTER TABLE {schema}.triage_items
                ADD CONSTRAINT triage_items_classification_method_check
                CHECK (classification_method IN ('rules','human'));
            ALTER TABLE {schema}.triage_items
                ADD COLUMN IF NOT EXISTS data_class TEXT NOT NULL DEFAULT 'personal';
            ALTER TABLE {schema}.triage_items
                ADD COLUMN IF NOT EXISTS data_class_rationale TEXT NOT NULL DEFAULT 'Mail metadata is Personal by default.';
            ALTER TABLE {schema}.triage_items
                ADD COLUMN IF NOT EXISTS data_classification_method TEXT NOT NULL DEFAULT 'rules';
            ALTER TABLE {schema}.triage_items
                ADD COLUMN IF NOT EXISTS data_classification_version TEXT NOT NULL DEFAULT 'data-class-rules-v1';
            ALTER TABLE {schema}.triage_items
                DROP CONSTRAINT IF EXISTS triage_items_data_class_check;
            ALTER TABLE {schema}.triage_items
                ADD CONSTRAINT triage_items_data_class_check
                CHECK (data_class IN ('public','personal','vault'));
            ALTER TABLE {schema}.triage_items
                DROP CONSTRAINT IF EXISTS triage_items_data_classification_method_check;
            ALTER TABLE {schema}.triage_items
                ADD CONSTRAINT triage_items_data_classification_method_check
                CHECK (data_classification_method IN ('rules','human'));
            ALTER TABLE {schema}.triage_items
                ADD COLUMN IF NOT EXISTS gmail_action TEXT;
            ALTER TABLE {schema}.triage_items
                ADD COLUMN IF NOT EXISTS gmail_action_at TIMESTAMPTZ;
            ALTER TABLE {schema}.triage_items
                ADD COLUMN IF NOT EXISTS purge_after TIMESTAMPTZ;
            ALTER TABLE {schema}.triage_items
                ADD COLUMN IF NOT EXISTS gmail_location TEXT;
            ALTER TABLE {schema}.triage_items
                ADD COLUMN IF NOT EXISTS gmail_observed_at TIMESTAMPTZ;
            ALTER TABLE {schema}.triage_items
                ADD COLUMN IF NOT EXISTS gmail_sync_status TEXT;
            ALTER TABLE {schema}.triage_items
                ADD COLUMN IF NOT EXISTS gmail_sync_error TEXT;
            ALTER TABLE {schema}.triage_items
                DROP CONSTRAINT IF EXISTS triage_items_status_check;
            ALTER TABLE {schema}.triage_items
                ADD CONSTRAINT triage_items_status_check
                CHECK (status IN ('proposed','approved','executed','archived','trashed','missing','dismissed'));
            ALTER TABLE {schema}.triage_items
                DROP CONSTRAINT IF EXISTS triage_items_gmail_action_check;
            ALTER TABLE {schema}.triage_items
                ADD CONSTRAINT triage_items_gmail_action_check
                CHECK (gmail_action IS NULL OR gmail_action IN ('archive','trash','restore'));
            ALTER TABLE {schema}.triage_items
                DROP CONSTRAINT IF EXISTS triage_items_gmail_location_check;
            ALTER TABLE {schema}.triage_items
                ADD CONSTRAINT triage_items_gmail_location_check
                CHECK (gmail_location IS NULL OR gmail_location IN ('inbox','archive','trash','missing'));
            ALTER TABLE {schema}.triage_items
                DROP CONSTRAINT IF EXISTS triage_items_gmail_sync_status_check;
            ALTER TABLE {schema}.triage_items
                ADD CONSTRAINT triage_items_gmail_sync_status_check
                CHECK (gmail_sync_status IS NULL OR gmail_sync_status IN ('synced','queued','retrying','attention'));
            ALTER TABLE {schema}.gmail_action_jobs
                DROP CONSTRAINT IF EXISTS gmail_action_jobs_state_check;
            ALTER TABLE {schema}.gmail_action_jobs
                ADD CONSTRAINT gmail_action_jobs_state_check
                CHECK (state IN ('queued','completed','abandoned','canceled'));
            ALTER TABLE {schema}.content_cloud_jobs
                ADD COLUMN IF NOT EXISTS task TEXT NOT NULL DEFAULT 'content-analysis-v1';
            ALTER TABLE {schema}.content_cloud_jobs
                ADD COLUMN IF NOT EXISTS started_at TIMESTAMPTZ;
            ALTER TABLE {schema}.content_cloud_jobs
                ADD COLUMN IF NOT EXISTS completed_at TIMESTAMPTZ;
            ALTER TABLE {schema}.content_cloud_jobs
                ADD COLUMN IF NOT EXISTS last_error TEXT;
            ALTER TABLE {schema}.content_cloud_jobs
                ADD COLUMN IF NOT EXISTS result_json TEXT;
            ALTER TABLE {schema}.content_cloud_jobs
                DROP CONSTRAINT IF EXISTS content_cloud_jobs_status_check;
            ALTER TABLE {schema}.content_cloud_jobs
                ADD CONSTRAINT content_cloud_jobs_status_check
                CHECK (status IN ('queued','running','succeeded','failed'));
            ALTER TABLE {schema}.content_cloud_jobs
                DROP CONSTRAINT IF EXISTS content_cloud_jobs_provider_calls_check;
            ALTER TABLE {schema}.content_cloud_jobs
                ADD CONSTRAINT content_cloud_jobs_provider_calls_check
                CHECK (provider_calls BETWEEN 0 AND 5);

            -- Share-link extractors (github/arxiv/reddit) widen the kind CHECK. The
            -- inline `CHECK (kind IN (...))` above was auto-named
            -- `feed_items_kind_check` by Postgres (the deterministic
            -- `<table>_<column>_check` convention), and CREATE TABLE IF NOT EXISTS
            -- never touches an existing table's constraints -- so an installed
            -- database keeps the narrow set until this targeted DROP + re-ADD runs.
            -- Idempotent: on a fresh install the IF EXISTS no-ops and the named
            -- constraint is added once. No existing row violates it: the new set is
            -- a superset of the old one.
            ALTER TABLE {schema}.feed_items DROP CONSTRAINT IF EXISTS feed_items_kind_check;
            ALTER TABLE {schema}.feed_items
                ADD CONSTRAINT feed_items_kind_check
                CHECK (kind IN ('youtube','instagram','podcast','article','mail','github','arxiv','reddit','huggingface'));

            -- #74: enrichment state -- content_status and summary retry ledger.
            ALTER TABLE {schema}.feed_items
                ADD COLUMN IF NOT EXISTS content_status TEXT NOT NULL DEFAULT 'unknown';
            ALTER TABLE {schema}.feed_items DROP CONSTRAINT IF EXISTS feed_items_content_status_check;
            ALTER TABLE {schema}.feed_items
                ADD CONSTRAINT feed_items_content_status_check
                CHECK (content_status IN ('full','thin','none','unknown'));
            ALTER TABLE {schema}.feed_items
                ADD COLUMN IF NOT EXISTS summary_attempts INTEGER NOT NULL DEFAULT 0;
            ALTER TABLE {schema}.feed_items
                ADD COLUMN IF NOT EXISTS summary_last_error TEXT;
            ALTER TABLE {schema}.feed_items
                ADD COLUMN IF NOT EXISTS summary_next_attempt TIMESTAMPTZ;
            ALTER TABLE {schema}.feed_items
                ADD COLUMN IF NOT EXISTS summary_attempt_revision TEXT;

            -- #81: which client handed the content over. NULL means the server
            -- fetched it. Free-form rather than a CHECK: the set of clients is
            -- open (extension, CLI, a future share sheet) and a constraint here
            -- would need a migration every time one is added.
            ALTER TABLE {schema}.feed_items
                ADD COLUMN IF NOT EXISTS captured_via TEXT;

            -- #78: what the stored text IS, as against how much of it there is.
            -- Constrained, unlike captured_via, because this set is closed by
            -- the enum that writes it (extraction::TranscriptSource) rather
            -- than open to whatever a new client calls itself. Existing rows
            -- stay 'unknown': every one of them predates the distinction, and
            -- backfilling them as 'full-text' would assert something about
            -- arXiv items that is false.
            ALTER TABLE {schema}.feed_items
                ADD COLUMN IF NOT EXISTS transcript_source TEXT NOT NULL DEFAULT 'unknown';
            ALTER TABLE {schema}.feed_items DROP CONSTRAINT IF EXISTS feed_items_transcript_source_check;
            ALTER TABLE {schema}.feed_items
                ADD CONSTRAINT feed_items_transcript_source_check
                CHECK (transcript_source IN ('full-text','abstract','unknown'));

            -- #77: producer provenance lives beside each stage value. Legacy
            -- rows are labelled unknown rather than assigned a guessed model.
            ALTER TABLE {schema}.feed_raw_content
                ADD COLUMN IF NOT EXISTS tier TEXT NOT NULL DEFAULT 'legacy';
            ALTER TABLE {schema}.feed_raw_content
                ADD COLUMN IF NOT EXISTS revision TEXT NOT NULL DEFAULT 'legacy-unknown';
            ALTER TABLE {schema}.feed_items
                ADD COLUMN IF NOT EXISTS normalization_tier TEXT;
            ALTER TABLE {schema}.feed_items
                ADD COLUMN IF NOT EXISTS normalization_revision TEXT;
            ALTER TABLE {schema}.feed_items
                ADD COLUMN IF NOT EXISTS normalization_completed_at TIMESTAMPTZ;
            ALTER TABLE {schema}.feed_items
                ADD COLUMN IF NOT EXISTS summary_tier TEXT;
            ALTER TABLE {schema}.feed_items
                ADD COLUMN IF NOT EXISTS summary_revision TEXT;
            ALTER TABLE {schema}.feed_items
                ADD COLUMN IF NOT EXISTS summary_completed_at TIMESTAMPTZ;
            ALTER TABLE {schema}.feed_evaluations
                ADD COLUMN IF NOT EXISTS tier TEXT NOT NULL DEFAULT 'legacy';
            ALTER TABLE {schema}.feed_raw_content DROP CONSTRAINT IF EXISTS feed_raw_content_tier_check;
            ALTER TABLE {schema}.feed_raw_content ADD CONSTRAINT feed_raw_content_tier_check
                CHECK (tier IN ('legacy','deterministic','model','human'));
            ALTER TABLE {schema}.feed_items DROP CONSTRAINT IF EXISTS feed_items_normalization_tier_check;
            ALTER TABLE {schema}.feed_items ADD CONSTRAINT feed_items_normalization_tier_check
                CHECK (normalization_tier IS NULL OR normalization_tier IN ('legacy','deterministic','model','human'));
            ALTER TABLE {schema}.feed_items DROP CONSTRAINT IF EXISTS feed_items_summary_tier_check;
            ALTER TABLE {schema}.feed_items ADD CONSTRAINT feed_items_summary_tier_check
                CHECK (summary_tier IS NULL OR summary_tier IN ('legacy','deterministic','model','human'));
            ALTER TABLE {schema}.feed_evaluations DROP CONSTRAINT IF EXISTS feed_evaluations_tier_check;
            ALTER TABLE {schema}.feed_evaluations ADD CONSTRAINT feed_evaluations_tier_check
                CHECK (tier IN ('legacy','deterministic','model','human'));
            UPDATE {schema}.feed_items SET
                normalization_tier = 'legacy', normalization_revision = 'legacy-unknown',
                normalization_completed_at = created_at
            WHERE transcript IS NOT NULL AND normalization_tier IS NULL;
            UPDATE {schema}.feed_items SET
                summary_tier = 'legacy', summary_revision = 'legacy-unknown',
                summary_completed_at = created_at
            WHERE summary IS NOT NULL AND summary_tier IS NULL;
            UPDATE {schema}.feed_evaluations SET tier =
                CASE WHEN mode IN ('reranked','semantic') THEN 'model' ELSE 'deterministic' END
            WHERE tier = 'legacy';

            -- #71: cross-encoder scores are distinct from bi-encoder cosine
            -- scores. Widen both persisted mode ledgers so an existing database
            -- can record that distinction instead of mislabelling it semantic.
            ALTER TABLE {schema}.feed_relevance
                DROP CONSTRAINT IF EXISTS feed_relevance_mode_check;
            ALTER TABLE {schema}.feed_relevance
                ADD CONSTRAINT feed_relevance_mode_check
                CHECK (mode IN ('reranked','semantic','lexical'));
            ALTER TABLE {schema}.feed_evaluations
                DROP CONSTRAINT IF EXISTS feed_evaluations_mode_check;
            ALTER TABLE {schema}.feed_evaluations
                ADD CONSTRAINT feed_evaluations_mode_check
                CHECK (mode IN ('reranked','semantic','lexical','unscored'));

            -- Backfill content_status for existing rows (idempotent: only touches 'unknown').
            UPDATE {schema}.feed_items SET content_status = 'full'
            WHERE content_status = 'unknown'
              AND transcript IS NOT NULL AND length(transcript) >= 1000;
            UPDATE {schema}.feed_items SET content_status = 'thin'
            WHERE content_status = 'unknown'
              AND transcript IS NOT NULL AND length(transcript) < 1000;
            UPDATE {schema}.feed_items SET content_status = 'none'
            WHERE content_status = 'unknown'
              AND transcript IS NULL;

            CREATE INDEX IF NOT EXISTS idx_triage_stream ON {schema}.triage_items(stream);
            CREATE INDEX IF NOT EXISTS idx_triage_status ON {schema}.triage_items(status);
            CREATE INDEX IF NOT EXISTS idx_triage_purge_after
                ON {schema}.triage_items(purge_after) WHERE purge_after IS NOT NULL;
            CREATE UNIQUE INDEX IF NOT EXISTS idx_gmail_action_jobs_one_queued
                ON {schema}.gmail_action_jobs(triage_id) WHERE state = 'queued';
            CREATE INDEX IF NOT EXISTS idx_gmail_action_jobs_retry
                ON {schema}.gmail_action_jobs(next_attempt) WHERE state = 'queued';
            CREATE INDEX IF NOT EXISTS idx_feed_stream ON {schema}.feed_items(stream);
            CREATE INDEX IF NOT EXISTS idx_feed_status ON {schema}.feed_items(status);
            CREATE INDEX IF NOT EXISTS idx_feed_day ON {schema}.feed_items(day);
            CREATE INDEX IF NOT EXISTS idx_feed_relevance_score ON {schema}.feed_relevance(score DESC);
            CREATE INDEX IF NOT EXISTS idx_triage_relevance_score ON {schema}.triage_relevance(score DESC);
            CREATE INDEX IF NOT EXISTS idx_feed_origins_source ON {schema}.feed_origins(source_id);
            CREATE INDEX IF NOT EXISTS idx_feed_evaluations_score ON {schema}.feed_evaluations(overall_score DESC);
            CREATE INDEX IF NOT EXISTS idx_feed_evaluations_revision
                ON {schema}.feed_evaluations(context_revision, evaluator_revision);
            CREATE INDEX IF NOT EXISTS idx_feed_quality_flags_derived
                ON {schema}.feed_quality_flags(derived_at DESC);
            CREATE INDEX IF NOT EXISTS idx_content_cloud_derivatives_approved
                ON {schema}.content_cloud_derivatives(approved_at DESC);
            CREATE INDEX IF NOT EXISTS idx_content_cloud_jobs_queued
                ON {schema}.content_cloud_jobs(queued_at ASC) WHERE status = 'queued';
            "
        ))?;
        Ok(())
    }

    // -- triage ----------------------------------------------------------

    pub const TRIAGE_STATUSES: [&'static str; 7] = [
        "proposed",
        "approved",
        "executed",
        "archived",
        "trashed",
        "missing",
        "dismissed",
    ];

    /// Upsert a triage proposal observed in the Gmail Inbox. Human category
    /// decisions survive. A previously archived/trashed legacy row returns to
    /// the queue because the inbox observation is authoritative.
    pub fn upsert_triage(&self, item: &TriageItem) -> Result<bool, Box<dyn std::error::Error>> {
        // Gmail internalDate is epoch-ms; convert to fractional epoch-seconds so
        // the bound param is plain double precision for to_timestamp().
        let internal_secs: Option<f64> = item.internal_date_ms.map(|ms| ms as f64 / 1000.0);
        let mut conn = self.conn.lock().unwrap();
        let existing = conn.query_opt(
            &format!("SELECT id FROM {}.triage_items WHERE id = $1", self.schema),
            &[&item.id],
        )?;
        let is_new = existing.is_none();

        conn.execute(
            &format!(
                "INSERT INTO {schema}.triage_items AS t
                    (id, from_addr, subject, snippet, internal_date, stream, rationale,
                     classification_method, classification_version, data_class,
                     data_class_rationale, data_classification_method,
                     data_classification_version, status, gmail_location,
                     gmail_observed_at, gmail_sync_status, first_seen, last_seen)
                 VALUES ($1,$2,$3,$4, to_timestamp($5), $6, $7, $8, $9, $10, $11, $12, $13,
                         'proposed', 'inbox', now(), 'synced', now(), now())
                 ON CONFLICT (id) DO UPDATE SET
                     from_addr = excluded.from_addr,
                     subject = excluded.subject,
                     snippet = excluded.snippet,
                     internal_date = excluded.internal_date,
                     stream = CASE WHEN t.classification_method = 'human'
                        THEN t.stream ELSE excluded.stream END,
                     rationale = CASE WHEN t.classification_method = 'human'
                        THEN t.rationale ELSE excluded.rationale END,
                     classification_method = CASE WHEN t.classification_method = 'human'
                        THEN t.classification_method ELSE excluded.classification_method END,
                     classification_version = CASE WHEN t.classification_method = 'human'
                        THEN t.classification_version ELSE excluded.classification_version END,
                     data_class = CASE WHEN t.data_classification_method = 'human'
                        THEN t.data_class ELSE excluded.data_class END,
                     data_class_rationale = CASE WHEN t.data_classification_method = 'human'
                        THEN t.data_class_rationale ELSE excluded.data_class_rationale END,
                     data_classification_method = CASE WHEN t.data_classification_method = 'human'
                        THEN t.data_classification_method ELSE excluded.data_classification_method END,
                     data_classification_version = CASE WHEN t.data_classification_method = 'human'
                        THEN t.data_classification_version ELSE excluded.data_classification_version END,
                     status = CASE WHEN t.status IN ('archived','trashed','missing','executed')
                        THEN 'proposed' ELSE t.status END,
                     gmail_location = 'inbox',
                     gmail_observed_at = now(),
                     gmail_sync_status = 'synced',
                     gmail_sync_error = NULL,
                     purge_after = NULL,
                     last_seen = now()",
                schema = self.schema
            ),
            &[
                &item.id,
                &item.from_addr,
                &item.subject,
                &item.snippet,
                &internal_secs,
                &item.stream,
                &item.rationale,
                &item.classification_method,
                &item.classification_version,
                &item.data_class,
                &item.data_class_rationale,
                &item.data_classification_method,
                &item.data_classification_version,
            ],
        )?;
        Ok(is_new)
    }

    /// Record a human category correction without resolving the proposal. The
    /// separate classification provenance is what prevents the next sweep from
    /// overwriting the correction with a deterministic rule result.
    pub fn set_triage_stream(
        &self,
        id: &str,
        stream: &str,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        if !crate::rules::STREAMS.contains(&stream) {
            return Err(format!(
                "invalid triage stream '{stream}' -- must be one of: {}",
                crate::rules::STREAMS.join(", ")
            )
            .into());
        }
        let mut conn = self.conn.lock().unwrap();
        let affected = conn.execute(
            &format!(
                "UPDATE {}.triage_items SET
                    stream = $1,
                    rationale = 'Category set manually in Axon.',
                    classification_method = 'human',
                    classification_version = 'manual-v1'
                 WHERE id = $2",
                self.schema
            ),
            &[&stream, &id],
        )?;
        Ok(affected > 0)
    }

    pub fn set_triage_data_class(
        &self,
        id: &str,
        data_class: &str,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        if !crate::content_item::valid(data_class) {
            return Err(format!(
                "invalid data class '{data_class}' -- must be one of: {}",
                crate::content_item::DATA_CLASSES.join(", ")
            )
            .into());
        }
        let mut conn = self.conn.lock().unwrap();
        let affected = conn.execute(
            &format!(
                "UPDATE {}.triage_items SET
                    data_class = $1,
                    data_class_rationale = 'Data class set manually in Axon.',
                    data_classification_method = 'human',
                    data_classification_version = 'manual-v1'
                 WHERE id = $2",
                self.schema
            ),
            &[&data_class, &id],
        )?;
        Ok(affected > 0)
    }

    /// Refresh a rule-produced data class while preserving an explicit human
    /// override. Returns false for a missing item or a preserved override.
    pub fn refresh_triage_data_class(
        &self,
        id: &str,
        classification: &crate::content_item::DataClass,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let affected = conn.execute(
            &format!(
                "UPDATE {}.triage_items SET
                    data_class = $1,
                    data_class_rationale = $2,
                    data_classification_method = $3,
                    data_classification_version = $4
                 WHERE id = $5 AND data_classification_method <> 'human'",
                self.schema
            ),
            &[
                &classification.value,
                &classification.rationale,
                &classification.method,
                &classification.version,
                &id,
            ],
        )?;
        Ok(affected > 0)
    }

    /// Overwrite a stored row's review fields with their redacted form.
    ///
    /// Deliberately the only write that narrows these two columns, and
    /// deliberately not a delete: the proposal, its decision and its Gmail
    /// identity all stay reviewable — only the material that should never have
    /// been persisted goes. A resweep cannot undo it, because the sweep now
    /// redacts before it writes (see `intake`).
    pub fn redact_triage_review_fields(
        &self,
        id: &str,
        subject: Option<&str>,
        snippet: Option<&str>,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let affected = conn.execute(
            &format!(
                "UPDATE {}.triage_items SET subject = $1, snippet = $2 WHERE id = $3",
                self.schema
            ),
            &[&subject, &snippet, &id],
        )?;
        Ok(affected > 0)
    }

    pub fn set_triage_status(
        &self,
        id: &str,
        status: &str,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        if !Self::TRIAGE_STATUSES.contains(&status) {
            return Err(format!(
                "invalid triage status '{status}' -- must be one of: {}",
                Self::TRIAGE_STATUSES.join(", ")
            )
            .into());
        }
        let mut conn = self.conn.lock().unwrap();
        let affected = conn.execute(
            &format!(
                "UPDATE {}.triage_items SET status = $1 WHERE id = $2",
                self.schema
            ),
            &[&status, &id],
        )?;
        Ok(affected > 0)
    }

    /// Persist the local half of a Gmail lifecycle action. Callers must invoke
    /// this only after Gmail has confirmed the matching mutation.
    pub fn record_gmail_action(
        &self,
        id: &str,
        action: &str,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        if !matches!(action, "archive" | "trash" | "restore") {
            return Err("Gmail action must be archive, trash, or restore".into());
        }
        let mut conn = self.conn.lock().unwrap();
        let affected = match action {
            "archive" => conn.execute(
                &format!(
                    "UPDATE {}.triage_items SET
                        status = 'archived', gmail_action = 'archive',
                        gmail_action_at = now(), purge_after = NULL,
                        gmail_location = 'archive', gmail_observed_at = now(),
                        gmail_sync_status = 'synced', gmail_sync_error = NULL
                     WHERE id = $1",
                    self.schema
                ),
                &[&id],
            )?,
            "trash" => conn.execute(
                &format!(
                    "UPDATE {}.triage_items SET
                        status = 'trashed', gmail_action = 'trash',
                        gmail_action_at = now(), purge_after = now() + interval '30 days',
                        gmail_location = 'trash', gmail_observed_at = now(),
                        gmail_sync_status = 'synced', gmail_sync_error = NULL
                     WHERE id = $1",
                    self.schema
                ),
                &[&id],
            )?,
            "restore" => conn.execute(
                &format!(
                    "UPDATE {}.triage_items SET
                        status = 'proposed', gmail_action = 'restore',
                        gmail_action_at = now(), purge_after = NULL,
                        gmail_location = 'inbox', gmail_observed_at = now(),
                        gmail_sync_status = 'synced', gmail_sync_error = NULL
                     WHERE id = $1",
                    self.schema
                ),
                &[&id],
            )?,
            _ => unreachable!(),
        };
        Ok(affected > 0)
    }

    /// Write intent before contacting Gmail. A single queued job per thread
    /// prevents conflicting retries while allowing completed history to remain.
    pub fn queue_gmail_action(
        &self,
        id: &str,
        action: &str,
    ) -> Result<GmailActionJob, Box<dyn std::error::Error>> {
        if !matches!(action, "archive" | "trash" | "restore") {
            return Err("Gmail action must be archive, trash, or restore".into());
        }
        let mut conn = self.conn.lock().unwrap();
        let mut transaction = conn.transaction()?;
        let row = transaction.query_opt(
            &format!(
                "SELECT status FROM {}.triage_items WHERE id = $1 FOR UPDATE",
                self.schema
            ),
            &[&id],
        )?;
        let Some(row) = row else {
            return Err("mail proposal not found".into());
        };
        let source_status = row.get::<_, String>(0);
        let allowed = match action {
            "archive" | "trash" => matches!(source_status.as_str(), "proposed" | "approved"),
            "restore" => matches!(source_status.as_str(), "archived" | "trashed"),
            _ => false,
        };
        if !allowed {
            return Err(format!("cannot {action} mail in {source_status} state").into());
        }
        if transaction
            .query_opt(
                &format!(
                    "SELECT job_id FROM {}.gmail_action_jobs
                     WHERE triage_id = $1 AND state = 'queued'",
                    self.schema
                ),
                &[&id],
            )?
            .is_some()
        {
            return Err("a Gmail action is already queued for this mail".into());
        }
        let job = transaction.query_one(
            &format!(
                "INSERT INTO {}.gmail_action_jobs (triage_id, action, source_status)
                 VALUES ($1,$2,$3)
                 RETURNING job_id, triage_id, action, source_status, attempts",
                self.schema
            ),
            &[&id, &action, &source_status],
        )?;
        transaction.execute(
            &format!(
                "UPDATE {}.triage_items SET
                    gmail_sync_status = 'queued', gmail_sync_error = NULL
                 WHERE id = $1",
                self.schema
            ),
            &[&id],
        )?;
        transaction.commit()?;
        Ok(GmailActionJob {
            job_id: job.get(0),
            triage_id: job.get(1),
            action: job.get(2),
            source_status: job.get(3),
            attempts: job.get(4),
        })
    }

    /// Complete both halves of local state atomically after Gmail is known to
    /// be at the requested location. Replaying a completed job is harmless.
    pub fn complete_gmail_action(&self, job_id: i64) -> Result<bool, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let mut transaction = conn.transaction()?;
        let row = transaction.query_opt(
            &format!(
                "SELECT triage_id, action, state FROM {}.gmail_action_jobs
                 WHERE job_id = $1 FOR UPDATE",
                self.schema
            ),
            &[&job_id],
        )?;
        let Some(row) = row else {
            return Ok(false);
        };
        let id = row.get::<_, String>(0);
        let action = row.get::<_, String>(1);
        let state = row.get::<_, String>(2);
        if state == "completed" {
            return Ok(true);
        }
        if state != "queued" {
            return Err("Gmail action job is no longer retryable".into());
        }
        let affected = match action.as_str() {
            "archive" => transaction.execute(
                &format!(
                    "UPDATE {}.triage_items SET
                        status = 'archived', gmail_action = 'archive', gmail_action_at = now(),
                        purge_after = NULL, gmail_location = 'archive', gmail_observed_at = now(),
                        gmail_sync_status = 'synced', gmail_sync_error = NULL WHERE id = $1",
                    self.schema
                ),
                &[&id],
            )?,
            "trash" => transaction.execute(
                &format!(
                    "UPDATE {}.triage_items SET
                        status = 'trashed', gmail_action = 'trash', gmail_action_at = now(),
                        purge_after = COALESCE(purge_after, now() + interval '30 days'),
                        gmail_location = 'trash', gmail_observed_at = now(),
                        gmail_sync_status = 'synced', gmail_sync_error = NULL WHERE id = $1",
                    self.schema
                ),
                &[&id],
            )?,
            "restore" => transaction.execute(
                &format!(
                    "UPDATE {}.triage_items SET
                        status = 'proposed', gmail_action = 'restore', gmail_action_at = now(),
                        purge_after = NULL, gmail_location = 'inbox', gmail_observed_at = now(),
                        gmail_sync_status = 'synced', gmail_sync_error = NULL WHERE id = $1",
                    self.schema
                ),
                &[&id],
            )?,
            _ => return Err("stored Gmail action is invalid".into()),
        };
        if affected == 0 {
            return Ok(false);
        }
        transaction.execute(
            &format!(
                "UPDATE {}.gmail_action_jobs SET
                    state = 'completed', updated_at = now(), completed_at = now(), last_error = NULL
                 WHERE job_id = $1",
                self.schema
            ),
            &[&job_id],
        )?;
        transaction.commit()?;
        Ok(true)
    }

    pub fn fail_gmail_action(
        &self,
        job_id: i64,
        error: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let bounded_error = error.chars().take(240).collect::<String>();
        let mut conn = self.conn.lock().unwrap();
        let mut transaction = conn.transaction()?;
        let row = transaction.query_opt(
            &format!(
                "UPDATE {}.gmail_action_jobs SET
                    attempts = attempts + 1,
                    state = CASE WHEN attempts + 1 >= 5 THEN 'abandoned' ELSE 'queued' END,
                    last_error = $2, updated_at = now(),
                    next_attempt = now() + interval '1 minute' * LEAST(attempts + 1, 5)
                 WHERE job_id = $1 AND state = 'queued'
                 RETURNING triage_id, state",
                self.schema
            ),
            &[&job_id, &bounded_error],
        )?;
        let Some(row) = row else {
            return Err("Gmail action job is not queued".into());
        };
        let triage_id = row.get::<_, String>(0);
        let state = row.get::<_, String>(1);
        let sync_status = if state == "abandoned" {
            "attention"
        } else {
            "retrying"
        };
        transaction.execute(
            &format!(
                "UPDATE {}.triage_items SET gmail_sync_status = $1, gmail_sync_error = $2
                 WHERE id = $3",
                self.schema
            ),
            &[&sync_status, &bounded_error, &triage_id],
        )?;
        transaction.commit()?;
        Ok(state)
    }

    /// Reset the newest attention job after an explicit operator decision.
    /// The action and its original source state are preserved; only the bounded
    /// attempt window is reopened.
    pub fn retry_abandoned_gmail_action(
        &self,
        id: &str,
    ) -> Result<GmailActionJob, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let mut transaction = conn.transaction()?;
        let row = transaction.query_opt(
            &format!(
                "SELECT job_id, triage_id, action, source_status
                 FROM {}.gmail_action_jobs
                 WHERE triage_id = $1 AND state = 'abandoned'
                 ORDER BY job_id DESC LIMIT 1 FOR UPDATE",
                self.schema
            ),
            &[&id],
        )?;
        let Some(row) = row else {
            return Err("no Gmail action needs operator attention".into());
        };
        let job_id = row.get::<_, i64>(0);
        transaction.execute(
            &format!(
                "UPDATE {}.gmail_action_jobs SET
                    state = 'queued', attempts = 0, last_error = NULL,
                    next_attempt = now(), updated_at = now(), completed_at = NULL
                 WHERE job_id = $1",
                self.schema
            ),
            &[&job_id],
        )?;
        transaction.execute(
            &format!(
                "UPDATE {}.triage_items SET gmail_sync_status = 'queued', gmail_sync_error = NULL
                 WHERE id = $1",
                self.schema
            ),
            &[&id],
        )?;
        transaction.commit()?;
        Ok(GmailActionJob {
            job_id,
            triage_id: row.get(1),
            action: row.get(2),
            source_status: row.get(3),
            attempts: 0,
        })
    }

    /// Cancel only an abandoned job. Queued jobs may already be in flight in
    /// the maintenance worker, so canceling them would create an ambiguous
    /// Gmail/local split.
    pub fn cancel_abandoned_gmail_action(
        &self,
        id: &str,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let mut transaction = conn.transaction()?;
        let row = transaction.query_opt(
            &format!(
                "UPDATE {}.gmail_action_jobs SET
                    state = 'canceled', updated_at = now(), completed_at = now()
                 WHERE job_id = (
                    SELECT job_id FROM {}.gmail_action_jobs
                    WHERE triage_id = $1 AND state = 'abandoned'
                    ORDER BY job_id DESC LIMIT 1
                 )
                 RETURNING triage_id",
                self.schema, self.schema
            ),
            &[&id],
        )?;
        if row.is_none() {
            return Ok(false);
        }
        transaction.execute(
            &format!(
                "UPDATE {}.triage_items SET
                    gmail_sync_status = CASE WHEN gmail_location IS NULL THEN NULL ELSE 'synced' END,
                    gmail_sync_error = NULL
                 WHERE id = $1",
                self.schema
            ),
            &[&id],
        )?;
        transaction.commit()?;
        Ok(true)
    }

    pub fn pending_gmail_actions(
        &self,
        limit: i64,
    ) -> Result<Vec<GmailActionJob>, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let rows = conn.query(
            &format!(
                "SELECT job_id, triage_id, action, source_status, attempts
                 FROM {}.gmail_action_jobs
                 WHERE state = 'queued' AND next_attempt <= now()
                 ORDER BY next_attempt, job_id LIMIT $1",
                self.schema
            ),
            &[&limit.clamp(1, 100)],
        )?;
        Ok(rows
            .iter()
            .map(|row| GmailActionJob {
                job_id: row.get(0),
                triage_id: row.get(1),
                action: row.get(2),
                source_status: row.get(3),
                attempts: row.get(4),
            })
            .collect())
    }

    pub fn gmail_reconcile_candidates(
        &self,
        limit: i64,
    ) -> Result<Vec<GmailReconcileCandidate>, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let rows = conn.query(
            &format!(
                "SELECT id, status FROM {}.triage_items t
                 WHERE status <> 'dismissed'
                   AND NOT EXISTS (
                     SELECT 1 FROM {}.gmail_action_jobs j
                     WHERE j.triage_id = t.id AND j.state IN ('queued','abandoned')
                   )
                 ORDER BY gmail_observed_at ASC NULLS FIRST, last_seen DESC
                 LIMIT $1",
                self.schema, self.schema
            ),
            &[&limit.clamp(1, 500)],
        )?;
        Ok(rows
            .iter()
            .map(|row| GmailReconcileCandidate {
                triage_id: row.get(0),
                status: row.get(1),
            })
            .collect())
    }

    pub fn observe_gmail_location(
        &self,
        id: &str,
        location: &str,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        if !matches!(location, "inbox" | "archive" | "trash") {
            return Err("Gmail location must be inbox, archive, or trash".into());
        }
        let mut conn = self.conn.lock().unwrap();
        let affected = conn.execute(
            &format!(
                "UPDATE {}.triage_items SET
                    status = CASE
                      WHEN $1 = 'trash' THEN 'trashed'
                      WHEN $1 = 'archive' THEN 'archived'
                      WHEN status IN ('archived','trashed','missing','executed') THEN 'proposed'
                      ELSE status
                    END,
                    purge_after = CASE
                      WHEN $1 = 'trash' THEN COALESCE(purge_after, now() + interval '30 days')
                      ELSE NULL
                    END,
                    gmail_location = $1, gmail_observed_at = now(),
                    gmail_sync_status = 'synced', gmail_sync_error = NULL
                 WHERE id = $2",
                self.schema
            ),
            &[&location, &id],
        )?;
        Ok(affected > 0)
    }

    /// Record an authoritative Gmail 404/410 without discarding Axon's local
    /// metadata. Any queued or attention action is closed because Gmail can no
    /// longer apply it. A Trash retention deadline, if present, remains active.
    pub fn observe_gmail_missing(&self, id: &str) -> Result<bool, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let mut transaction = conn.transaction()?;
        transaction.execute(
            &format!(
                "UPDATE {}.gmail_action_jobs SET
                    state = 'canceled', updated_at = now(), completed_at = now()
                 WHERE triage_id = $1 AND state IN ('queued','abandoned')",
                self.schema
            ),
            &[&id],
        )?;
        let affected = transaction.execute(
            &format!(
                "UPDATE {}.triage_items SET
                    status = 'missing', gmail_location = 'missing', gmail_observed_at = now(),
                    gmail_sync_status = 'synced', gmail_sync_error = NULL
                 WHERE id = $1",
                self.schema
            ),
            &[&id],
        )?;
        transaction.commit()?;
        Ok(affected > 0)
    }

    /// Remove expired Trash content and any staged cloud copy. Gmail owns its
    /// own Trash retention; this cleanup is strictly Axon's local copy.
    pub fn purge_expired_trashed(&self) -> Result<u64, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let mut transaction = conn.transaction()?;
        transaction.execute(
            &format!(
                "DELETE FROM {schema}.content_cloud_jobs
                 WHERE source = 'mail' AND item_id IN (
                    SELECT id FROM {schema}.triage_items
                    WHERE status IN ('trashed','missing') AND purge_after <= now()
                 )",
                schema = self.schema
            ),
            &[],
        )?;
        transaction.execute(
            &format!(
                "DELETE FROM {schema}.content_cloud_derivatives
                 WHERE source = 'mail' AND item_id IN (
                    SELECT id FROM {schema}.triage_items
                    WHERE status IN ('trashed','missing') AND purge_after <= now()
                 )",
                schema = self.schema
            ),
            &[],
        )?;
        let purged = transaction.execute(
            &format!(
                "DELETE FROM {}.triage_items
                 WHERE status IN ('trashed','missing') AND purge_after <= now()",
                self.schema
            ),
            &[],
        )?;
        transaction.commit()?;
        Ok(purged)
    }

    pub fn stage_cloud_derivative(
        &self,
        approval: &CloudDerivativeApproval,
    ) -> Result<CloudDerivativeState, Box<dyn std::error::Error>> {
        if !matches!(approval.source.as_str(), "feed" | "mail") {
            return Err("cloud derivative source must be 'feed' or 'mail'".into());
        }
        if !crate::content_item::valid(&approval.original_data_class) {
            return Err("cloud derivative has an invalid original data class".into());
        }
        if !matches!(
            approval.derivative_data_class.as_str(),
            "public" | "personal"
        ) {
            return Err("cloud derivative must be Public or Personal".into());
        }

        let mut conn = self.conn.lock().unwrap();
        let row = conn.query_one(
            &format!(
                "INSERT INTO {schema}.content_cloud_derivatives
                    (source, item_id, source_revision, preview_hash,
                     original_data_class, derivative_data_class, transformation,
                     document, redaction_count, approved_at)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,now())
                 ON CONFLICT (source, item_id) DO UPDATE SET
                    source_revision = excluded.source_revision,
                    preview_hash = excluded.preview_hash,
                    original_data_class = excluded.original_data_class,
                    derivative_data_class = excluded.derivative_data_class,
                    transformation = excluded.transformation,
                    document = excluded.document,
                    redaction_count = excluded.redaction_count,
                    approved_at = now()
                 RETURNING preview_hash, approved_at::text",
                schema = self.schema
            ),
            &[
                &approval.source,
                &approval.item_id,
                &approval.source_revision,
                &approval.preview_hash,
                &approval.original_data_class,
                &approval.derivative_data_class,
                &approval.transformation,
                &approval.document,
                &approval.redaction_count,
            ],
        )?;
        Ok(CloudDerivativeState {
            status: "staged".into(),
            preview_hash: Some(row.get(0)),
            approved_at: Some(row.get(1)),
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
        })
    }

    pub fn queue_cloud_derivative(
        &self,
        request: &CloudQueueRequest,
    ) -> Result<CloudDerivativeState, Box<dyn std::error::Error>> {
        if !matches!(request.source.as_str(), "feed" | "mail") {
            return Err("cloud queue source must be 'feed' or 'mail'".into());
        }
        if !request.provider_role.starts_with("cloud_") {
            return Err("cloud queue provider role must start with 'cloud_'".into());
        }

        let job_id = cloud_job_id(request);
        let mut conn = self.conn.lock().unwrap();
        let row = conn.query_opt(
            &format!(
                "WITH approved AS (
                    SELECT approved_at
                    FROM {schema}.content_cloud_derivatives
                    WHERE source = $1 AND item_id = $2
                      AND source_revision = $3 AND preview_hash = $4
                 ), queued AS (
                    INSERT INTO {schema}.content_cloud_jobs
                        (job_id, source, item_id, source_revision, preview_hash, provider_role)
                    SELECT $5, $1, $2, $3, $4, $6 FROM approved
                    ON CONFLICT (source, item_id, preview_hash, provider_role)
                    DO UPDATE SET provider_role = excluded.provider_role
                    RETURNING job_id, provider_role, queued_at::text, status,
                              provider_calls, task, started_at::text, completed_at::text,
                              last_error, result_json
                 )
                 SELECT queued.job_id, queued.provider_role, queued.queued_at,
                        approved.approved_at::text, queued.status, queued.provider_calls,
                        queued.task, queued.started_at, queued.completed_at,
                        queued.last_error, queued.result_json
                 FROM queued CROSS JOIN approved",
                schema = self.schema
            ),
            &[
                &request.source,
                &request.item_id,
                &request.source_revision,
                &request.preview_hash,
                &job_id,
                &request.provider_role,
            ],
        )?;
        let Some(row) = row else {
            return Err("approved derivative is missing or stale; review it again".into());
        };
        let result_json = row.get::<_, Option<String>>(10);
        Ok(CloudDerivativeState {
            status: "staged".into(),
            preview_hash: Some(request.preview_hash.clone()),
            approved_at: Some(row.get(3)),
            dispatch_status: row.get(4),
            job_id: Some(row.get(0)),
            provider_role: Some(row.get(1)),
            queued_at: Some(row.get(2)),
            provider_calls: row.get::<_, i32>(5).try_into()?,
            task: Some(row.get(6)),
            started_at: row.get(7),
            completed_at: row.get(8),
            last_error: row.get(9),
            result: result_json
                .map(|value| serde_json::from_str(&value))
                .transpose()?,
        })
    }

    /// The stored digest for one item, if it has one.
    pub fn content_digest(
        &self,
        source: &str,
        item_id: &str,
    ) -> Result<Option<StoredDigest>, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let row = conn.query_opt(
            &format!(
                "SELECT source, item_id, text, state, shape, depth, focus, producer,
                        source_chars, redactions, attempts, last_error,
                        diagram, diagram_state, diagram_error,
                        chart, chart_state, chart_error, generated_at::text
                 FROM {}.content_digests
                 WHERE source = $1 AND item_id = $2",
                self.schema
            ),
            &[&source, &item_id],
        )?;
        Ok(row.map(|row| StoredDigest {
            source: row.get(0),
            item_id: row.get(1),
            text: row.get(2),
            state: row.get(3),
            shape: row.get(4),
            depth: row.get(5),
            focus: row.get(6),
            producer: row.get(7),
            source_chars: row.get(8),
            redactions: row.get(9),
            attempts: row.get(10),
            last_error: row.get(11),
            diagram: row.get(12),
            diagram_state: row.get(13),
            diagram_error: row.get(14),
            chart: row.get(15),
            chart_state: row.get(16),
            chart_error: row.get(17),
            generated_at: row.get(18),
        }))
    }

    /// Replace an item's digest.
    ///
    /// Replace rather than append: a digest is the current best answer, not a
    /// history. `attempts` is carried by the caller so a failing row accumulates
    /// its own count instead of resetting every pass.
    pub fn upsert_content_digest(
        &self,
        digest: &StoredDigest,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        conn.execute(
            &format!(
                "INSERT INTO {}.content_digests
                     (source, item_id, text, state, shape, depth, focus, producer,
                      source_chars, redactions, attempts, last_error, generated_at)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12, now())
                 ON CONFLICT (source, item_id) DO UPDATE SET
                     text = EXCLUDED.text,
                     state = EXCLUDED.state,
                     shape = EXCLUDED.shape,
                     depth = EXCLUDED.depth,
                     focus = EXCLUDED.focus,
                     producer = EXCLUDED.producer,
                     source_chars = EXCLUDED.source_chars,
                     redactions = EXCLUDED.redactions,
                     attempts = EXCLUDED.attempts,
                     last_error = EXCLUDED.last_error,
                     generated_at = now()",
                self.schema
            ),
            &[
                &digest.source,
                &digest.item_id,
                &digest.text,
                &digest.state,
                &digest.shape,
                &digest.depth,
                &digest.focus,
                &digest.producer,
                &digest.source_chars,
                &digest.redactions,
                &digest.attempts,
                &digest.last_error,
            ],
        )?;
        Ok(())
    }

    /// Attach or clear the Mermaid diagram beside an existing digest.
    ///
    /// Separate from [`Store::upsert_content_digest`] because the two are
    /// separate presses: regenerating a digest must not silently discard a
    /// diagram the operator already asked for, and vice versa.
    pub fn update_content_diagram(
        &self,
        source: &str,
        item_id: &str,
        diagram: Option<&str>,
        state: &str,
        error: Option<&str>,
        producer: &str,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let updated = conn.execute(
            &format!(
                "UPDATE {}.content_digests
                    SET diagram = $3, diagram_state = $4, diagram_error = $5, diagram_producer = $6
                  WHERE source = $1 AND item_id = $2",
                self.schema
            ),
            &[&source, &item_id, &diagram, &state, &error, &producer],
        )?;
        Ok(updated)
    }

    /// Attach or clear the extracted chart table beside an existing digest.
    ///
    /// Its own press, like the diagram: regenerating a digest must not discard
    /// a figure the operator already asked for.
    pub fn update_content_chart(
        &self,
        source: &str,
        item_id: &str,
        chart: Option<&str>,
        state: &str,
        error: Option<&str>,
        producer: &str,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let updated = conn.execute(
            &format!(
                "UPDATE {}.content_digests
                    SET chart = $3, chart_state = $4, chart_error = $5, chart_producer = $6
                  WHERE source = $1 AND item_id = $2",
                self.schema
            ),
            &[&source, &item_id, &chart, &state, &error, &producer],
        )?;
        Ok(updated)
    }

    /// Items of one source that the automatic pass should still digest.
    ///
    /// Three reasons a row qualifies: it has none, its producer is stale, or it
    /// failed retryably and has attempts left. The `depth = 'standard'` guard on
    /// the stale case is load-bearing — an operator who pressed *detailed* has
    /// made a decision, and a model upgrade must not quietly overwrite it with
    /// the automatic rung.
    pub fn items_needing_digest(
        &self,
        source: &str,
        producer: &str,
        max_attempts: i32,
        limit: i64,
    ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let (table, order) = match source {
            "mail" => ("triage_items", "internal_date DESC NULLS LAST"),
            "feed" => ("feed_items", "created_at DESC"),
            other => return Err(format!("no digest queue for source {other:?}").into()),
        };
        let mut conn = self.conn.lock().unwrap();
        let rows = conn.query(
            &format!(
                "SELECT i.id
                   FROM {schema}.{table} i
                   LEFT JOIN {schema}.content_digests d
                          ON d.source = $1 AND d.item_id = i.id
                  WHERE d.item_id IS NULL
                     OR (d.producer <> $2 AND d.depth = 'standard')
                     OR (d.state IN ('http_error','model_error','empty_response','timeout')
                         AND d.attempts < $3)
                  ORDER BY i.{order}
                  LIMIT $4",
                schema = self.schema,
                table = table,
                order = order
            ),
            &[&source, &producer, &max_attempts, &limit],
        )?;
        Ok(rows.iter().map(|row| row.get::<_, String>(0)).collect())
    }

    pub fn cloud_derivative_state(
        &self,
        source: &str,
        item_id: &str,
        current_source_revision: &str,
        current_preview_hash: &str,
    ) -> Result<CloudDerivativeState, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let row = conn.query_opt(
            &format!(
                "SELECT source_revision, preview_hash, approved_at::text
                 FROM {}.content_cloud_derivatives
                 WHERE source = $1 AND item_id = $2",
                self.schema
            ),
            &[&source, &item_id],
        )?;
        Ok(match row {
            None => CloudDerivativeState::not_prepared(),
            Some(row) => {
                let source_revision = row.get::<_, String>(0);
                let preview_hash = row.get::<_, String>(1);
                let current = source_revision == current_source_revision
                    && preview_hash == current_preview_hash;
                let job = if current {
                    conn.query_opt(
                        &format!(
                            "SELECT job_id, provider_role, queued_at::text, status,
                                    provider_calls, task, started_at::text, completed_at::text,
                                    last_error, result_json
                             FROM {}.content_cloud_jobs
                             WHERE source = $1 AND item_id = $2
                               AND source_revision = $3 AND preview_hash = $4
                             ORDER BY queued_at DESC LIMIT 1",
                            self.schema
                        ),
                        &[&source, &item_id, &source_revision, &preview_hash],
                    )?
                } else {
                    None
                };
                CloudDerivativeState {
                    status: if current {
                        "staged".into()
                    } else {
                        "stale".into()
                    },
                    preview_hash: Some(preview_hash),
                    approved_at: Some(row.get(2)),
                    dispatch_status: job
                        .as_ref()
                        .map(|job| job.get(3))
                        .unwrap_or_else(|| "not_queued".to_string()),
                    job_id: job.as_ref().map(|job| job.get(0)),
                    provider_role: job.as_ref().map(|job| job.get(1)),
                    queued_at: job.as_ref().map(|job| job.get(2)),
                    provider_calls: job
                        .as_ref()
                        .map(|job| job.get::<_, i32>(4).try_into())
                        .transpose()?
                        .unwrap_or(0),
                    task: job.as_ref().map(|job| job.get(5)),
                    started_at: job.as_ref().and_then(|job| job.get(6)),
                    completed_at: job.as_ref().and_then(|job| job.get(7)),
                    last_error: job.as_ref().and_then(|job| job.get(8)),
                    result: job
                        .as_ref()
                        .and_then(|job| job.get::<_, Option<String>>(9))
                        .map(|value| serde_json::from_str(&value))
                        .transpose()?,
                }
            }
        })
    }

    pub fn cloud_job_for_dispatch(
        &self,
        job_id: &str,
    ) -> Result<Option<CloudDispatchJob>, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let row = conn.query_opt(
            &format!(
                "SELECT j.job_id, j.source, j.item_id, j.source_revision,
                        j.preview_hash, j.provider_role, j.task,
                        d.original_data_class, d.derivative_data_class,
                        d.transformation, d.document, j.provider_calls
                 FROM {schema}.content_cloud_jobs j
                 JOIN {schema}.content_cloud_derivatives d
                   ON d.source = j.source AND d.item_id = j.item_id
                  AND d.source_revision = j.source_revision
                  AND d.preview_hash = j.preview_hash
                 WHERE j.job_id = $1
                   AND (j.status IN ('queued','failed')
                     OR (j.status = 'running' AND j.started_at < now() - interval '5 minutes'))
                   AND j.provider_calls < 5",
                schema = self.schema
            ),
            &[&job_id],
        )?;
        Ok(row.map(|row| CloudDispatchJob {
            job_id: row.get(0),
            source: row.get(1),
            item_id: row.get(2),
            source_revision: row.get(3),
            preview_hash: row.get(4),
            provider_role: row.get(5),
            task: row.get(6),
            original_data_class: row.get(7),
            derivative_data_class: row.get(8),
            transformation: row.get(9),
            document: row.get(10),
            provider_calls: row.get(11),
        }))
    }

    pub fn cloud_provider_calls_today(
        &self,
        provider_role: &str,
    ) -> Result<u32, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let row = conn.query_one(
            &format!(
                "SELECT COUNT(*)
                 FROM {}.content_cloud_attempts
                 WHERE provider_role = $1
                   AND (started_at AT TIME ZONE 'UTC')::date =
                       (now() AT TIME ZONE 'UTC')::date",
                self.schema
            ),
            &[&provider_role],
        )?;
        Ok(row.get::<_, i64>(0).try_into()?)
    }

    pub fn utc_date(&self) -> Result<String, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        Ok(conn
            .query_one(
                "SELECT to_char(now() AT TIME ZONE 'UTC', 'YYYY-MM-DD')",
                &[],
            )?
            .get(0))
    }

    pub fn claim_cloud_job_attempt(
        &self,
        job_id: &str,
        provider_role: &str,
        model: &str,
        max_requests_per_day: u32,
    ) -> Result<CloudAttemptClaim, Box<dyn std::error::Error>> {
        if !provider_role.starts_with("cloud_") || model.trim().is_empty() {
            return Err("cloud attempt requires a reviewed role and model".into());
        }
        let mut conn = self.conn.lock().unwrap();
        let mut transaction = conn.transaction()?;
        // Serialize budget decisions per provider role so two jobs cannot both
        // observe the final free slot and exceed the local hard ceiling.
        transaction.query_one(
            "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))",
            &[&provider_role],
        )?;
        let calls = transaction
            .query_one(
                &format!(
                    "SELECT COUNT(*)
                     FROM {}.content_cloud_attempts
                     WHERE provider_role = $1
                       AND (started_at AT TIME ZONE 'UTC')::date =
                           (now() AT TIME ZONE 'UTC')::date",
                    self.schema
                ),
                &[&provider_role],
            )?
            .get::<_, i64>(0);
        if calls >= i64::from(max_requests_per_day) {
            return Ok(CloudAttemptClaim::DailyLimitReached);
        }

        let row = transaction.query_opt(
            &format!(
                "UPDATE {}.content_cloud_jobs
                 SET status = 'running', provider_calls = provider_calls + 1,
                     started_at = now(), completed_at = NULL,
                     last_error = NULL, result_json = NULL
                 WHERE job_id = $1
                   AND (status IN ('queued','failed')
                     OR (status = 'running' AND started_at < now() - interval '5 minutes'))
                   AND provider_calls < 5
                 RETURNING provider_calls, preview_hash",
                self.schema
            ),
            &[&job_id],
        )?;
        let Some(row) = row else {
            return Ok(CloudAttemptClaim::JobUnavailable);
        };
        let sequence = row.get::<_, i32>(0);
        let preview_hash = row.get::<_, String>(1);
        let attempt_id = transaction
            .query_one(
                &format!(
                    "INSERT INTO {}.content_cloud_attempts
                        (job_id, sequence, provider_role, model, preview_hash)
                     VALUES ($1,$2,$3,$4,$5)
                     RETURNING attempt_id",
                    self.schema
                ),
                &[&job_id, &sequence, &provider_role, &model, &preview_hash],
            )?
            .get(0);
        transaction.commit()?;
        Ok(CloudAttemptClaim::Started(attempt_id))
    }

    pub fn complete_cloud_job_attempt(
        &self,
        job_id: &str,
        attempt_id: i64,
        result: &serde_json::Value,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let result = serde_json::to_string(result)?;
        let mut conn = self.conn.lock().unwrap();
        let mut transaction = conn.transaction()?;
        let attempt_updated = transaction.execute(
            &format!(
                "UPDATE {}.content_cloud_attempts
                 SET status = 'succeeded', result_json = $3,
                     completed_at = now(), last_error = NULL
                 WHERE attempt_id = $2 AND job_id = $1 AND status = 'running'",
                self.schema
            ),
            &[&job_id, &attempt_id, &result],
        )?;
        let job_updated = transaction.execute(
            &format!(
                "UPDATE {}.content_cloud_jobs
                 SET status = 'succeeded', result_json = $2,
                     completed_at = now(), last_error = NULL
                 WHERE job_id = $1 AND status = 'running'",
                self.schema
            ),
            &[&job_id, &result],
        )?;
        if attempt_updated == 1 && job_updated == 1 {
            transaction.commit()?;
            Ok(true)
        } else {
            transaction.rollback()?;
            Ok(false)
        }
    }

    pub fn fail_cloud_job_attempt(
        &self,
        job_id: &str,
        attempt_id: i64,
        error: &str,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let error: String = error.chars().take(500).collect();
        let mut conn = self.conn.lock().unwrap();
        let mut transaction = conn.transaction()?;
        let attempt_updated = transaction.execute(
            &format!(
                "UPDATE {}.content_cloud_attempts
                 SET status = 'failed', last_error = $3, completed_at = now()
                 WHERE attempt_id = $2 AND job_id = $1 AND status = 'running'",
                self.schema
            ),
            &[&job_id, &attempt_id, &error],
        )?;
        let job_updated = transaction.execute(
            &format!(
                "UPDATE {}.content_cloud_jobs
                 SET status = 'failed', last_error = $2, completed_at = now()
                 WHERE job_id = $1 AND status = 'running'",
                self.schema
            ),
            &[&job_id, &error],
        )?;
        if attempt_updated == 1 && job_updated == 1 {
            transaction.commit()?;
            Ok(true)
        } else {
            transaction.rollback()?;
            Ok(false)
        }
    }

    pub fn get_triage_status(
        &self,
        id: &str,
    ) -> Result<Option<String>, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let row = conn.query_opt(
            &format!(
                "SELECT status FROM {}.triage_items WHERE id = $1",
                self.schema
            ),
            &[&id],
        )?;
        Ok(row.map(|r| r.get::<_, String>(0)))
    }

    /// List triage items, optionally filtered by status, newest first.
    pub fn list_triage(
        &self,
        status: Option<&str>,
    ) -> Result<Vec<TriageItem>, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let base = format!(
            "SELECT id, from_addr, subject, snippet, internal_date::text, stream, rationale,
                    status, first_seen::text, last_seen::text,
                    classification_method, classification_version, data_class,
                    data_class_rationale, data_classification_method,
                    data_classification_version, gmail_action,
                    gmail_action_at::text, purge_after::text, gmail_location,
                    gmail_observed_at::text, gmail_sync_status,
                    (SELECT action FROM {schema}.gmail_action_jobs j
                     WHERE j.triage_id = t.id AND j.state IN ('queued','abandoned')
                     ORDER BY job_id DESC LIMIT 1),
                    gmail_sync_error
             FROM {schema}.triage_items t",
            schema = self.schema
        );
        let rows = match status {
            Some(s) => conn.query(
                &format!("{base} WHERE status = $1 ORDER BY internal_date DESC NULLS LAST"),
                &[&s],
            )?,
            None => conn.query(
                &format!("{base} ORDER BY internal_date DESC NULLS LAST"),
                &[],
            )?,
        };
        Ok(rows.iter().map(row_to_triage).collect())
    }

    /// Read one mail proposal for the shared content reader. Gmail-specific
    /// category and action state stays on `triage_items`; the HTTP adapter
    /// projects it into the same content contract as a normal Feed item.
    pub fn get_triage(&self, id: &str) -> Result<Option<TriageItem>, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let row = conn.query_opt(
            &format!(
                "SELECT id, from_addr, subject, snippet, internal_date::text, stream, rationale,
                        status, first_seen::text, last_seen::text,
                        classification_method, classification_version, data_class,
                        data_class_rationale, data_classification_method,
                        data_classification_version, gmail_action,
                        gmail_action_at::text, purge_after::text, gmail_location,
                        gmail_observed_at::text, gmail_sync_status,
                        (SELECT action FROM {schema}.gmail_action_jobs j
                         WHERE j.triage_id = t.id AND j.state IN ('queued','abandoned')
                         ORDER BY job_id DESC LIMIT 1),
                        gmail_sync_error
                 FROM {schema}.triage_items t WHERE id = $1",
                schema = self.schema
            ),
            &[&id],
        )?;
        Ok(row.as_ref().map(row_to_triage))
    }

    /// Replace the TELOS matches for one mail proposal. This is relevance
    /// annotation only: it never changes the category, proposal status, or a
    /// TELOS source file.
    pub fn replace_triage_relevance(
        &self,
        triage_id: &str,
        matches: &[RelevanceMatch],
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let mut transaction = conn.transaction()?;
        transaction.execute(
            &format!(
                "DELETE FROM {}.triage_relevance WHERE triage_id = $1",
                self.schema
            ),
            &[&triage_id],
        )?;
        for relevance in matches {
            transaction.execute(
                &format!(
                    "INSERT INTO {schema}.triage_relevance
                        (triage_id, profile_key, profile_label, score, rationale, mode,
                         profile_revision, scored_at)
                     VALUES ($1,$2,$3,$4,$5,$6,$7,now())",
                    schema = self.schema
                ),
                &[
                    &triage_id,
                    &relevance.profile_key,
                    &relevance.profile_label,
                    &relevance.score,
                    &relevance.rationale,
                    &relevance.mode,
                    &relevance.profile_revision,
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn triage_relevance(
        &self,
        triage_id: &str,
    ) -> Result<Vec<RelevanceMatch>, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let rows = conn.query(
            &format!(
                "SELECT profile_key, profile_label, score, rationale, mode, profile_revision
                 FROM {}.triage_relevance WHERE triage_id = $1 ORDER BY score DESC",
                self.schema
            ),
            &[&triage_id],
        )?;
        Ok(rows
            .iter()
            .map(|row| RelevanceMatch {
                profile_key: row.get(0),
                profile_label: row.get(1),
                score: row.get(2),
                rationale: row.get(3),
                mode: row.get(4),
                profile_revision: row.get(5),
            })
            .collect())
    }

    // -- feed ------------------------------------------------------------

    pub const FEED_STATUSES: [&'static str; 3] = ["new", "keeper", "dismissed"];

    /// Upsert a feed item. `status`/`day`/`created_at` are set only on first
    /// INSERT and are absent from the ON CONFLICT update. `summary`/`transcript`
    /// use COALESCE so a re-ingest that lacks them never wipes a previously
    /// stored value. Returns `is_new`.
    pub fn upsert_feed(&self, item: &FeedItem) -> Result<bool, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let summary_provenance = item.summary.as_ref().map(|_| {
            item.summary_provenance
                .clone()
                .unwrap_or_else(|| StageProvenance::legacy("inline-summary-unknown"))
        });
        let summary_tier = summary_provenance.as_ref().map(|value| value.tier.as_str());
        let summary_revision = summary_provenance
            .as_ref()
            .map(|value| value.revision.as_str());
        let normalization_tier = item.transcript.as_ref().map(|_| "deterministic");
        let normalization_revision = item
            .transcript
            .as_ref()
            .map(|_| provenance::NORMALIZATION_REVISION);
        let existing = conn.query_opt(
            &format!("SELECT id FROM {}.feed_items WHERE id = $1", self.schema),
            &[&item.id],
        )?;
        let is_new = existing.is_none();

        conn.execute(
            &format!(
                "INSERT INTO {schema}.feed_items AS f
                    (id, stream, kind, title, url, author, summary, transcript, day, created_at,
                     status, content_status, transcript_source, captured_via, normalization_tier,
                     normalization_revision, normalization_completed_at,
                     summary_tier, summary_revision, summary_completed_at)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8, CURRENT_DATE, now(), 'new',
                         $9,$10,$11,$12,$13, CASE WHEN $8::text IS NOT NULL THEN now() END,
                         $14,$15, CASE WHEN $7::text IS NOT NULL THEN now() END)
                 ON CONFLICT (id) DO UPDATE SET
                     stream = excluded.stream,
                     kind = excluded.kind,
                     title = COALESCE(excluded.title, f.title),
                     url = excluded.url,
                     author = COALESCE(excluded.author, f.author),
                     summary = CASE WHEN excluded.summary IS NOT NULL AND
                         CASE excluded.summary_tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END >=
                         CASE f.summary_tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END
                         THEN excluded.summary ELSE f.summary END,
                     summary_tier = CASE WHEN excluded.summary IS NOT NULL AND
                         CASE excluded.summary_tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END >=
                         CASE f.summary_tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END
                         THEN excluded.summary_tier ELSE f.summary_tier END,
                     summary_revision = CASE WHEN excluded.summary IS NOT NULL AND
                         CASE excluded.summary_tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END >=
                         CASE f.summary_tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END
                         THEN excluded.summary_revision ELSE f.summary_revision END,
                     summary_completed_at = CASE WHEN excluded.summary IS NOT NULL AND
                         CASE excluded.summary_tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END >=
                         CASE f.summary_tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END
                         THEN now() ELSE f.summary_completed_at END,
                     transcript = CASE WHEN excluded.transcript IS NOT NULL AND
                         CASE excluded.normalization_tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END >=
                         CASE f.normalization_tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END
                         THEN excluded.transcript ELSE f.transcript END,
                     normalization_tier = CASE WHEN excluded.transcript IS NOT NULL AND
                         CASE excluded.normalization_tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END >=
                         CASE f.normalization_tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END
                         THEN excluded.normalization_tier ELSE f.normalization_tier END,
                     normalization_revision = CASE WHEN excluded.transcript IS NOT NULL AND
                         CASE excluded.normalization_tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END >=
                         CASE f.normalization_tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END
                         THEN excluded.normalization_revision ELSE f.normalization_revision END,
                     normalization_completed_at = CASE WHEN excluded.transcript IS NOT NULL AND
                         CASE excluded.normalization_tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END >=
                         CASE f.normalization_tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END
                         THEN now() ELSE f.normalization_completed_at END,
                     -- Follows the transcript: provenance describes the content
                     -- actually stored, so a re-fetch that yields nothing must
                     -- not relabel a captured body as server-fetched.
                     captured_via = CASE
                         WHEN excluded.transcript IS NOT NULL AND
                           CASE excluded.normalization_tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END >=
                           CASE f.normalization_tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END
                         THEN excluded.captured_via
                         ELSE f.captured_via
                     END,
                     content_status = CASE
                         WHEN excluded.transcript IS NOT NULL AND
                           CASE excluded.normalization_tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END >=
                           CASE f.normalization_tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END
                         THEN excluded.content_status
                         ELSE f.content_status
                     END,
                     -- Same guard as content_status: both describe the text
                     -- actually stored, so a re-fetch that loses to the
                     -- existing transcript must not relabel it.
                     transcript_source = CASE
                         WHEN excluded.transcript IS NOT NULL AND
                           CASE excluded.normalization_tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END >=
                           CASE f.normalization_tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END
                         THEN excluded.transcript_source
                         ELSE f.transcript_source
                     END",
                schema = self.schema
            ),
            &[
                &item.id,
                &item.stream,
                &item.kind,
                &item.title,
                &item.url,
                &item.author,
                &item.summary,
                &item.transcript,
                &item.content_status,
                &item.transcript_source,
                &item.captured_via,
                &normalization_tier,
                &normalization_revision,
                &summary_tier,
                &summary_revision,
            ],
        )?;

        // Raw extraction output, when this upsert carries one. A re-fetch
        // replaces it; a re-normalize never touches it.
        if let Some(raw) = &item.raw_content {
            conn.execute(
                &format!(
                    "INSERT INTO {schema}.feed_raw_content AS current (feed_id, raw, tier, revision)
                     VALUES ($1, $2, 'deterministic', $3)
                     ON CONFLICT (feed_id) DO UPDATE SET
                         raw = excluded.raw,
                         tier = excluded.tier,
                         revision = excluded.revision,
                         extracted_at = now()
                     WHERE CASE excluded.tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END >=
                           CASE current.tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END",
                    schema = self.schema
                ),
                &[&item.id, raw, &provenance::EXTRACTION_REVISION],
            )?;
        }

        Ok(is_new)
    }

    /// The extractor's output for an item, if it was stored with one. Items
    /// ingested before #86 have none — they can only be re-fetched, not
    /// re-normalized, and `renormalize_all` reports them as skipped.
    pub fn get_raw_content(&self, id: &str) -> Result<Option<String>, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let row = conn.query_opt(
            &format!(
                "SELECT raw FROM {}.feed_raw_content WHERE feed_id = $1",
                self.schema
            ),
            &[&id],
        )?;
        Ok(row.map(|r| r.get(0)))
    }

    /// Every item that has retained raw content, oldest first. The input to a
    /// re-normalization pass.
    pub fn feed_ids_with_raw_content(&self) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let rows = conn.query(
            &format!(
                "SELECT feed_id FROM {}.feed_raw_content ORDER BY extracted_at ASC",
                self.schema
            ),
            &[],
        )?;
        Ok(rows.iter().map(|r| r.get(0)).collect())
    }

    /// Replace an item's normalized body and its derived status. Used by the
    /// re-normalization pass; leaves the raw and the summary alone, because a
    /// rule change is not a reason to re-fetch or re-summarize.
    pub fn set_normalized(
        &self,
        id: &str,
        transcript: Option<&str>,
        content_status: &str,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let affected = conn.execute(
            &format!(
                "UPDATE {}.feed_items SET transcript = $1, content_status = $2,
                    normalization_tier = 'deterministic', normalization_revision = $4,
                    normalization_completed_at = now()
                 WHERE id = $3 AND 10 >= CASE normalization_tier
                    WHEN 'human' THEN 30 WHEN 'model' THEN 20
                    WHEN 'deterministic' THEN 10 ELSE 0 END",
                self.schema
            ),
            &[
                &transcript,
                &content_status,
                &id,
                &provenance::NORMALIZATION_REVISION,
            ],
        )?;
        Ok(affected > 0)
    }

    pub fn set_feed_status(
        &self,
        id: &str,
        status: &str,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        if !Self::FEED_STATUSES.contains(&status) {
            return Err(format!(
                "invalid feed status '{status}' -- must be one of: {}",
                Self::FEED_STATUSES.join(", ")
            )
            .into());
        }
        let mut conn = self.conn.lock().unwrap();
        let affected = conn.execute(
            &format!(
                "UPDATE {}.feed_items SET status = $1 WHERE id = $2",
                self.schema
            ),
            &[&status, &id],
        )?;
        Ok(affected > 0)
    }

    pub fn get_feed_status(&self, id: &str) -> Result<Option<String>, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let row = conn.query_opt(
            &format!(
                "SELECT status FROM {}.feed_items WHERE id = $1",
                self.schema
            ),
            &[&id],
        )?;
        Ok(row.map(|r| r.get::<_, String>(0)))
    }

    pub fn update_feed_summary(
        &self,
        id: &str,
        summary: &str,
        producer_revision: &str,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let affected = conn.execute(
            &format!(
                "UPDATE {}.feed_items SET summary = $1, summary_tier = 'model',
                    summary_revision = $3, summary_completed_at = now(), summary_attempts = 0,
                    summary_last_error = NULL, summary_next_attempt = NULL,
                    summary_attempt_revision = NULL
                 WHERE id = $2 AND 20 >= CASE summary_tier
                    WHEN 'human' THEN 30 WHEN 'model' THEN 20
                    WHEN 'deterministic' THEN 10 ELSE 0 END",
                self.schema
            ),
            &[&summary, &id, &producer_revision],
        )?;
        Ok(affected > 0)
    }

    /// Read producer provenance from the same rows that own each stage value.
    pub fn feed_stage_results(
        &self,
        id: &str,
    ) -> Result<Vec<StageProvenance>, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let row = conn.query_opt(
            &format!(
                "SELECT r.tier, r.revision, r.extracted_at::text,
                        f.normalization_tier, f.normalization_revision, f.normalization_completed_at::text,
                        f.summary_tier, f.summary_revision, f.summary_completed_at::text,
                        e.tier, e.evaluator_revision, e.evaluated_at::text
                 FROM {schema}.feed_items f
                 LEFT JOIN {schema}.feed_raw_content r ON r.feed_id = f.id
                 LEFT JOIN {schema}.feed_evaluations e ON e.feed_id = f.id
                 WHERE f.id = $1",
                schema = self.schema
            ),
            &[&id],
        )?;
        let Some(row) = row else {
            return Ok(Vec::new());
        };
        let mut stages = Vec::new();
        for (stage, tier_index, revision_index, time_index) in [
            ("extraction", 0, 1, 2),
            ("normalization", 3, 4, 5),
            ("summary", 6, 7, 8),
            ("ranking", 9, 10, 11),
        ] {
            if let Some(tier) = row.get::<_, Option<String>>(tier_index) {
                stages.push(StageProvenance {
                    stage: stage.to_string(),
                    tier,
                    revision: row
                        .get::<_, Option<String>>(revision_index)
                        .unwrap_or_else(|| "legacy-unknown".into()),
                    completed_at: row.get::<_, Option<String>>(time_index).unwrap_or_default(),
                });
            }
        }
        Ok(stages)
    }

    /// Atomically replace the computed review signals for one item. An empty
    /// set clears flags that no longer fire on the current stored evidence.
    pub fn replace_feed_quality_flags(
        &self,
        feed_id: &str,
        flags: &[QualityFlag],
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let mut transaction = conn.transaction()?;
        transaction.execute(
            &format!(
                "DELETE FROM {}.feed_quality_flags WHERE feed_id = $1",
                self.schema
            ),
            &[&feed_id],
        )?;
        for flag in flags {
            transaction.execute(
                &format!(
                    "INSERT INTO {schema}.feed_quality_flags
                        (feed_id, signal, reason, evidence, derived_at)
                     VALUES ($1, $2, $3, $4, now())",
                    schema = self.schema
                ),
                &[&feed_id, &flag.signal, &flag.reason, &flag.evidence],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn feed_quality_review_queue(
        &self,
        limit: usize,
    ) -> Result<Vec<QualityReviewRow>, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let rows = conn.query(
            &format!(
                "SELECT q.feed_id, f.title, f.url, f.status, f.content_status,
                        q.signal, q.reason, q.evidence, q.derived_at::text
                 FROM {schema}.feed_quality_flags q
                 JOIN {schema}.feed_items f ON f.id = q.feed_id
                 ORDER BY f.created_at DESC, q.signal ASC
                 LIMIT $1",
                schema = self.schema
            ),
            &[&(limit as i64)],
        )?;
        Ok(rows
            .iter()
            .map(|row| QualityReviewRow {
                feed_id: row.get(0),
                title: row.get(1),
                url: row.get(2),
                status: row.get(3),
                content_status: row.get(4),
                signal: row.get(5),
                reason: row.get(6),
                evidence: row.get(7),
                derived_at: row.get(8),
            })
            .collect())
    }

    /// Increment the attempt counter, record the error class, and set the next
    /// retry time with exponential backoff (5 min × 2^attempts, capped at 3
    /// attempts). After 3 failures `feed_pending_summaries` stops returning it.
    pub fn record_summary_attempt(
        &self,
        id: &str,
        error_class: &str,
        producer_revision: &str,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let affected = conn.execute(
            &format!(
                "UPDATE {}.feed_items SET
                    summary_attempts = CASE
                        WHEN summary_attempt_revision IS DISTINCT FROM $3 THEN 1
                        ELSE summary_attempts + 1 END,
                    summary_last_error = $1,
                    summary_next_attempt = now() + (interval '5 minutes' * power(2,
                        CASE WHEN summary_attempt_revision IS DISTINCT FROM $3
                            THEN 0 ELSE summary_attempts END)),
                    summary_attempt_revision = $3
                 WHERE id = $2",
                self.schema
            ),
            &[&error_class, &id, &producer_revision],
        )?;
        Ok(affected > 0)
    }

    /// Enrichment backlog: items eligible for summarization vs. permanently
    /// failed (≥3 attempts).
    pub fn feed_enrichment_counts(
        &self,
        producer_revision: Option<&str>,
    ) -> Result<EnrichmentCounts, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let row = conn.query_one(
            &format!(
                "SELECT
                    COUNT(*) FILTER (WHERE transcript IS NOT NULL
                        AND (summary IS NULL OR ($1::text IS NOT NULL AND
                            ((summary_tier = 'model' AND summary_revision IS DISTINCT FROM $1)
                             OR (summary_tier = 'legacy' AND summary_revision = 'legacy-unknown'))))
                        AND (summary_attempt_revision IS DISTINCT FROM $1
                            OR summary_attempts < 3)) AS pending,
                    COUNT(*) FILTER (WHERE transcript IS NOT NULL
                        AND (summary IS NULL OR ($1::text IS NOT NULL AND
                            ((summary_tier = 'model' AND summary_revision IS DISTINCT FROM $1)
                             OR (summary_tier = 'legacy' AND summary_revision = 'legacy-unknown'))))
                        AND summary_attempt_revision IS NOT DISTINCT FROM $1
                        AND summary_attempts >= 3) AS failed
                 FROM {}.feed_items",
                self.schema
            ),
            &[&producer_revision],
        )?;
        Ok(EnrichmentCounts {
            pending_summaries: row.get(0),
            failed_summaries: row.get(1),
        })
    }

    /// Distribution of `content_status` across all feed items.
    pub fn feed_content_status_counts(
        &self,
    ) -> Result<ContentStatusCounts, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let row = conn.query_one(
            &format!(
                "SELECT
                    COUNT(*) FILTER (WHERE content_status = 'full') AS full_count,
                    COUNT(*) FILTER (WHERE content_status = 'thin') AS thin_count,
                    COUNT(*) FILTER (WHERE content_status = 'none') AS none_count,
                    COUNT(*) FILTER (WHERE content_status = 'unknown') AS unknown_count
                 FROM {}.feed_items",
                self.schema
            ),
            &[],
        )?;
        Ok(ContentStatusCounts {
            full: row.get(0),
            thin: row.get(1),
            none: row.get(2),
            unknown: row.get(3),
        })
    }

    /// Single item incl. transcript (server /feed/:id, keeper export).
    pub fn get_feed(&self, id: &str) -> Result<Option<FeedItem>, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let row = conn.query_opt(
            &format!(
                "SELECT id, stream, kind, title, url, author, summary, transcript, day::text, created_at::text, status,
                       content_status, summary_attempts, summary_last_error, summary_next_attempt::text, captured_via, transcript_source
                 FROM {}.feed_items WHERE id = $1",
                self.schema
            ),
            &[&id],
        )?;
        Ok(row.as_ref().map(row_to_feed_full))
    }

    /// List feed items (no transcript in the payload), newest first. `days`
    /// bounds by `day >= CURRENT_DATE - days`. Excludes dismissed unless asked.
    /// Optionally filters by `source_id`.
    pub fn list_feed(
        &self,
        stream: Option<&str>,
        source_id: Option<&str>,
        days: i32,
        include_dismissed: bool,
    ) -> Result<Vec<FeedItem>, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let mut sql = format!(
            "SELECT DISTINCT f.id, f.stream, f.kind, f.title, f.url, f.author, f.summary, NULL::text, f.day::text, f.created_at::text, f.status,
                    f.content_status, f.summary_attempts, f.summary_last_error, f.summary_next_attempt::text, f.captured_via, f.transcript_source, f.created_at
             FROM {schema}.feed_items f",
            schema = self.schema
        );

        if source_id.is_some() {
            sql.push_str(&format!(
                " JOIN {schema}.feed_origins o ON f.id = o.feed_id",
                schema = self.schema
            ));
        }

        sql.push_str(" WHERE f.day >= CURRENT_DATE - $1::int");
        let mut params: Vec<&(dyn ToSql + Sync)> = vec![&days];

        if let Some(s) = &stream {
            sql.push_str(&format!(" AND f.stream = ${}", params.len() + 1));
            params.push(s);
        }
        if let Some(src) = &source_id {
            sql.push_str(&format!(" AND o.source_id = ${}", params.len() + 1));
            params.push(src);
        }
        if !include_dismissed {
            sql.push_str(" AND f.status != 'dismissed'");
        }
        sql.push_str(" ORDER BY f.created_at DESC");
        let rows = conn.query(&sql, &params)?;
        Ok(rows.iter().map(row_to_feed_list).collect())
    }

    /// Feed items eligible for summarization: transcript present, no summary,
    /// not past the attempt cap, and backoff window elapsed. Bounded retry
    /// replaces the old unbounded "summary IS NULL" scan.
    pub fn feed_pending_summaries(
        &self,
        producer_revision: Option<&str>,
    ) -> Result<Vec<FeedItem>, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let rows = conn.query(
            &format!(
                "SELECT id, stream, kind, title, url, author, summary, transcript, day::text, created_at::text, status,
                        content_status, summary_attempts, summary_last_error, summary_next_attempt::text, captured_via, transcript_source
                 FROM {}.feed_items
                 WHERE transcript IS NOT NULL
                   AND (summary IS NULL OR ($1::text IS NOT NULL AND
                        ((summary_tier = 'model' AND summary_revision IS DISTINCT FROM $1)
                         OR (summary_tier = 'legacy' AND summary_revision = 'legacy-unknown'))))
                   AND (summary_attempt_revision IS DISTINCT FROM $1
                        OR (summary_attempts < 3
                            AND (summary_next_attempt IS NULL OR summary_next_attempt <= now())))
                 ORDER BY created_at DESC",
                self.schema
            ),
            &[&producer_revision],
        )?;
        Ok(rows.iter().map(row_to_feed_full).collect())
    }

    pub fn feed_summary_needs_revision(
        &self,
        id: &str,
        producer_revision: &str,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let row = conn.query_opt(
            &format!(
                "SELECT summary IS NULL OR (summary_tier = 'model'
                    AND summary_revision IS DISTINCT FROM $2)
                    OR (summary_tier = 'legacy' AND summary_revision = 'legacy-unknown')
                 FROM {}.feed_items WHERE id = $1",
                self.schema
            ),
            &[&id, &producer_revision],
        )?;
        Ok(row.is_some_and(|row| row.get(0)))
    }

    /// Full feed items for a bounded relevance refresh. This intentionally
    /// includes dismissed items: a later filter change can make an old item
    /// useful again, while the human status remains untouched.
    pub fn feed_for_relevance(
        &self,
        days: i32,
        limit: usize,
    ) -> Result<Vec<FeedItem>, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let rows = conn.query(
            &format!(
                "SELECT id, stream, kind, title, url, author, summary, transcript, day::text, created_at::text, status,
                        content_status, summary_attempts, summary_last_error, summary_next_attempt::text, captured_via, transcript_source
                 FROM {}.feed_items
                 WHERE day >= CURRENT_DATE - $1::int
                 ORDER BY created_at DESC
                 LIMIT $2",
                self.schema
            ),
            &[&days, &(limit as i64)],
        )?;
        Ok(rows.iter().map(row_to_feed_full).collect())
    }

    /// Replace every profile result for one item in one transaction. Removed
    /// or renamed TELOS lenses therefore cannot leave stale matches behind.
    pub fn replace_feed_relevance(
        &self,
        feed_id: &str,
        matches: &[RelevanceMatch],
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let mut transaction = conn.transaction()?;
        let incoming_tier = matches
            .first()
            .map(|matched| provenance::ranking_tier(&matched.mode))
            .unwrap_or("deterministic");
        let current = transaction.query_opt(
            &format!(
                "SELECT tier FROM {}.feed_evaluations WHERE feed_id = $1",
                self.schema
            ),
            &[&feed_id],
        )?;
        if current.is_some_and(|row| {
            provenance::tier_rank(incoming_tier)
                < provenance::tier_rank(row.get::<_, String>(0).as_str())
        }) {
            return Ok(false);
        }
        transaction.execute(
            &format!(
                "DELETE FROM {}.feed_relevance WHERE feed_id = $1",
                self.schema
            ),
            &[&feed_id],
        )?;
        for relevance in matches {
            transaction.execute(
                &format!(
                    "INSERT INTO {schema}.feed_relevance
                        (feed_id, profile_key, profile_label, score, rationale, mode, profile_revision, scored_at)
                     VALUES ($1,$2,$3,$4,$5,$6,$7,now())",
                    schema = self.schema
                ),
                &[
                    &feed_id,
                    &relevance.profile_key,
                    &relevance.profile_label,
                    &relevance.score,
                    &relevance.rationale,
                    &relevance.mode,
                    &relevance.profile_revision,
                ],
            )?;
        }
        transaction.commit()?;
        Ok(true)
    }

    pub fn feed_relevance(
        &self,
        feed_id: &str,
    ) -> Result<Vec<RelevanceMatch>, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let rows = conn.query(
            &format!(
                "SELECT profile_key, profile_label, score, rationale, mode, profile_revision
                 FROM {}.feed_relevance WHERE feed_id = $1 ORDER BY score DESC",
                self.schema
            ),
            &[&feed_id],
        )?;
        Ok(rows
            .iter()
            .map(|row| RelevanceMatch {
                profile_key: row.get(0),
                profile_label: row.get(1),
                score: row.get(2),
                rationale: row.get(3),
                mode: row.get(4),
                profile_revision: row.get(5),
            })
            .collect())
    }

    /// Store the complete evaluation and its factors atomically. The factor
    /// table is normalized so future trip/deadline factors can be added without
    /// a schema migration or an opaque JSON payload.
    pub fn replace_feed_evaluation(
        &self,
        evaluation: &FeedEvaluation,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let mut transaction = conn.transaction()?;
        let tier = provenance::ranking_tier(&evaluation.mode);
        let affected = transaction.execute(
            &format!(
                "INSERT INTO {schema}.feed_evaluations
                    (feed_id, overall_score, explanation, mode, item_revision,
                     context_revision, evaluator_revision, tier, evaluated_at)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,now())
                 ON CONFLICT (feed_id) DO UPDATE SET
                    overall_score = excluded.overall_score,
                    explanation = excluded.explanation,
                    mode = excluded.mode,
                    item_revision = excluded.item_revision,
                    context_revision = excluded.context_revision,
                    evaluator_revision = excluded.evaluator_revision,
                    tier = excluded.tier,
                    evaluated_at = now()
                 WHERE CASE excluded.tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END >=
                       CASE feed_evaluations.tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END",
                schema = self.schema
            ),
            &[
                &evaluation.feed_id,
                &evaluation.overall_score,
                &evaluation.explanation,
                &evaluation.mode,
                &evaluation.item_revision,
                &evaluation.context_revision,
                &evaluation.evaluator_revision,
                &tier,
            ],
        )?;
        if affected == 0 {
            return Ok(false);
        }
        transaction.execute(
            &format!(
                "DELETE FROM {}.feed_evaluation_factors WHERE feed_id = $1",
                self.schema
            ),
            &[&evaluation.feed_id],
        )?;
        for (position, factor) in evaluation.factors.iter().enumerate() {
            let context_json = factor
                .context
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?;
            transaction.execute(
                &format!(
                    "INSERT INTO {schema}.feed_evaluation_factors
                        (feed_id, factor_key, label, score, weight, rationale, context_json, position)
                     VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
                    schema = self.schema
                ),
                &[
                    &evaluation.feed_id,
                    &factor.key,
                    &factor.label,
                    &factor.score,
                    &factor.weight,
                    &factor.rationale,
                    &context_json,
                    &(position as i32),
                ],
            )?;
        }
        transaction.commit()?;
        Ok(true)
    }

    pub fn feed_evaluation(
        &self,
        feed_id: &str,
    ) -> Result<Option<FeedEvaluation>, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let evaluation = conn.query_opt(
            &format!(
                "SELECT overall_score, explanation, mode, item_revision,
                        context_revision, evaluator_revision, evaluated_at::text
                 FROM {}.feed_evaluations WHERE feed_id = $1",
                self.schema
            ),
            &[&feed_id],
        )?;
        let Some(row) = evaluation else {
            return Ok(None);
        };
        let factor_rows = conn.query(
            &format!(
                "SELECT factor_key, label, score, weight, rationale, context_json
                 FROM {}.feed_evaluation_factors
                 WHERE feed_id = $1 ORDER BY position",
                self.schema
            ),
            &[&feed_id],
        )?;
        let factors = factor_rows
            .iter()
            .map(|factor| EvaluationFactor {
                key: factor.get(0),
                label: factor.get(1),
                score: factor.get(2),
                weight: factor.get(3),
                rationale: factor.get(4),
                context: factor
                    .get::<_, Option<String>>(5)
                    .and_then(|value| serde_json::from_str::<EvaluationFactorContext>(&value).ok()),
            })
            .collect();
        Ok(Some(FeedEvaluation {
            feed_id: feed_id.to_string(),
            overall_score: row.get(0),
            explanation: row.get(1),
            mode: row.get(2),
            item_revision: row.get(3),
            context_revision: row.get(4),
            evaluator_revision: row.get(5),
            evaluated_at: row.get::<_, Option<String>>(6).unwrap_or_default(),
            factors,
        }))
    }

    pub fn evaluation_summary(&self) -> Result<EvaluationSummary, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let row = conn.query_one(
            &format!(
                "SELECT COUNT(*)::bigint,
                        COUNT(*) FILTER (WHERE mode = 'reranked')::bigint,
                        COUNT(*) FILTER (WHERE mode = 'semantic')::bigint,
                        COUNT(*) FILTER (WHERE mode = 'lexical')::bigint,
                        COUNT(*) FILTER (WHERE mode = 'unscored')::bigint
                 FROM {}.feed_evaluations",
                self.schema
            ),
            &[],
        )?;
        Ok(EvaluationSummary {
            evaluated: row.get(0),
            reranked: row.get(1),
            semantic: row.get(2),
            lexical: row.get(3),
            unscored: row.get(4),
        })
    }

    pub fn replace_travel_context_snapshot(
        &self,
        revision: &str,
        payload: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        conn.execute(
            &format!(
                "INSERT INTO {schema}.feed_context_snapshots
                    (context_kind, revision, payload, refreshed_at)
                 VALUES ('travel',$1,$2,now())
                 ON CONFLICT (context_kind) DO UPDATE SET
                    revision = excluded.revision,
                    payload = excluded.payload,
                    refreshed_at = now()",
                schema = self.schema
            ),
            &[&revision, &payload],
        )?;
        Ok(())
    }

    pub fn travel_context_snapshot(
        &self,
    ) -> Result<Option<TravelContextSnapshot>, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let row = conn.query_opt(
            &format!(
                "SELECT revision, payload, refreshed_at::text
                 FROM {}.feed_context_snapshots WHERE context_kind = 'travel'",
                self.schema
            ),
            &[],
        )?;
        Ok(row.map(|row| TravelContextSnapshot {
            revision: row.get(0),
            payload: row.get(1),
            refreshed_at: row.get::<_, Option<String>>(2).unwrap_or_default(),
        }))
    }

    pub fn record_feed_origin(
        &self,
        feed_id: &str,
        source_id: &str,
        source_ref: &str,
        label: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        conn.execute(
            &format!(
                "INSERT INTO {schema}.feed_origins
                    (feed_id, source_id, source_ref, label, first_seen, last_seen)
                 VALUES ($1,$2,$3,$4,now(),now())
                 ON CONFLICT (feed_id, source_id, source_ref) DO UPDATE SET
                    label = COALESCE(excluded.label, {schema}.feed_origins.label),
                    last_seen = now()",
                schema = self.schema
            ),
            &[&feed_id, &source_id, &source_ref, &label],
        )?;
        Ok(())
    }

    pub fn feed_origins(
        &self,
        feed_id: &str,
    ) -> Result<Vec<FeedOrigin>, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let rows = conn.query(
            &format!(
                "SELECT source_id, source_ref, label
                 FROM {}.feed_origins WHERE feed_id = $1 ORDER BY first_seen",
                self.schema
            ),
            &[&feed_id],
        )?;
        Ok(rows
            .iter()
            .map(|row| FeedOrigin {
                source_id: row.get(0),
                source_ref: row.get(1),
                label: row.get(2),
            })
            .collect())
    }

    pub fn list_origin_summaries(&self) -> Result<Vec<OriginSummary>, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let rows = conn.query(
            &format!(
                "SELECT source_id, COUNT(DISTINCT feed_id), MIN(first_seen)::text, MAX(last_seen)::text
                 FROM {}.feed_origins
                 GROUP BY source_id
                 ORDER BY MAX(last_seen) DESC",
                self.schema
            ),
            &[],
        )?;
        Ok(rows
            .iter()
            .map(|r| OriginSummary {
                source_id: r.get(0),
                item_count: r.get(1),
                first_seen: r.get(2),
                last_seen: r.get(3),
            })
            .collect())
    }

    /// Which items arrived together, derived at read time from `feed_origins`
    /// alone (#84). A "run" is a cluster of arrivals for one source: ordered by
    /// `first_seen`, a gap longer than `RUN_GAP_MINUTES` starts a new one.
    ///
    /// Nothing is stored for this — no run id on the item, no batch table. A
    /// collector that fetches each URL can take a while, so the gap threshold
    /// is generous; two genuine runs of the same source inside half an hour
    /// read as one, which is the failure this trades for never having to
    /// migrate a grouping decision.
    pub fn list_feed_runs(&self, days: i32) -> Result<Vec<FeedRun>, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let rows = conn.query(
            &format!(
                "WITH ordered AS (
                     SELECT o.feed_id, o.source_id, o.label, o.first_seen,
                            LAG(o.first_seen) OVER (
                                PARTITION BY o.source_id ORDER BY o.first_seen
                            ) AS prev_seen
                     FROM {schema}.feed_origins o
                     JOIN {schema}.feed_items f ON f.id = o.feed_id
                     WHERE f.day >= CURRENT_DATE - $1::int
                 ),
                 marked AS (
                     SELECT *,
                            CASE
                                WHEN prev_seen IS NULL
                                  OR first_seen - prev_seen > interval '{gap} minutes'
                                THEN 1 ELSE 0
                            END AS starts_run
                     FROM ordered
                 ),
                 runs AS (
                     SELECT feed_id, source_id, label, first_seen,
                            SUM(starts_run) OVER (
                                PARTITION BY source_id ORDER BY first_seen
                                ROWS UNBOUNDED PRECEDING
                            ) AS run_seq
                     FROM marked
                 )
                 SELECT feed_id,
                        source_id,
                        label,
                        source_id || '#' || run_seq::text AS run_key,
                        MIN(first_seen) OVER (PARTITION BY source_id, run_seq)::text AS run_started
                 FROM runs
                 ORDER BY run_started DESC, first_seen ASC",
                schema = self.schema,
                gap = RUN_GAP_MINUTES
            ),
            &[&days],
        )?;
        Ok(rows
            .iter()
            .map(|r| FeedRun {
                feed_id: r.get(0),
                source_id: r.get(1),
                label: r.get(2),
                run_key: r.get(3),
                run_started: r.get(4),
            })
            .collect())
    }

    // -- source_state ----------------------------------------------------

    pub fn record_run(
        &self,
        source_name: &str,
        cursor: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let now = epoch_now();
        let mut conn = self.conn.lock().unwrap();
        conn.execute(
            &format!(
                "INSERT INTO {schema}.source_state (source_name, last_run_at, cursor)
                 VALUES ($1, $2, $3)
                 ON CONFLICT (source_name) DO UPDATE SET
                     last_run_at = excluded.last_run_at,
                     cursor = COALESCE(excluded.cursor, {schema}.source_state.cursor)",
                schema = self.schema
            ),
            &[&source_name, &now, &cursor],
        )?;
        Ok(())
    }

    pub fn get_source_state(
        &self,
        source_name: &str,
    ) -> Result<Option<SourceState>, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let row = conn.query_opt(
            &format!(
                "SELECT source_name, last_run_at, cursor, last_success_at, last_failure_at,
                        last_error, considered_count, new_count, consecutive_failures
                 FROM {}.source_state WHERE source_name = $1",
                self.schema
            ),
            &[&source_name],
        )?;
        Ok(row.map(|r| SourceState {
            source_name: r.get(0),
            last_run_at: r.get(1),
            cursor: r.get(2),
            last_success_at: r.get(3),
            last_failure_at: r.get(4),
            last_error: r.get(5),
            considered_count: r.get(6),
            new_count: r.get(7),
            consecutive_failures: r.get(8),
        }))
    }

    /// Record a completed pass. Success clears the failure streak; the counts
    /// describe the pass that just ran, not a running total, because "how much
    /// did the last run see" is the question a stale schedule raises.
    pub fn record_sweep_success(
        &self,
        source_name: &str,
        considered: i64,
        new_items: i64,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let now = epoch_now();
        let mut conn = self.conn.lock().unwrap();
        conn.execute(
            &format!(
                "INSERT INTO {schema}.source_state
                     (source_name, last_run_at, last_success_at, considered_count,
                      new_count, consecutive_failures)
                 VALUES ($1, $2, $2, $3, $4, 0)
                 ON CONFLICT (source_name) DO UPDATE SET
                     last_run_at = excluded.last_run_at,
                     last_success_at = excluded.last_success_at,
                     considered_count = excluded.considered_count,
                     new_count = excluded.new_count,
                     consecutive_failures = 0,
                     last_error = NULL",
                schema = self.schema
            ),
            &[&source_name, &now, &considered, &new_items],
        )?;
        Ok(())
    }

    /// `error_class` is a short stable label — `auth`, `quota`, `network`,
    /// `store`. Never a provider message: those quote request URLs and, for
    /// mail, occasionally the subject that failed.
    pub fn record_sweep_failure(
        &self,
        source_name: &str,
        error_class: &str,
    ) -> Result<i32, Box<dyn std::error::Error>> {
        let now = epoch_now();
        let mut conn = self.conn.lock().unwrap();
        let row = conn.query_one(
            &format!(
                "INSERT INTO {schema}.source_state
                     (source_name, last_run_at, last_failure_at, last_error, consecutive_failures)
                 VALUES ($1, $2, $2, $3, 1)
                 ON CONFLICT (source_name) DO UPDATE SET
                     last_run_at = excluded.last_run_at,
                     last_failure_at = excluded.last_failure_at,
                     last_error = excluded.last_error,
                     consecutive_failures = {schema}.source_state.consecutive_failures + 1
                 RETURNING consecutive_failures",
                schema = self.schema
            ),
            &[&source_name, &now, &error_class],
        )?;
        Ok(row.get(0))
    }

    /// Whether the store's clock currently sits inside a quiet window, given
    /// `[start, end)` in local hours. Asked of Postgres rather than computed in
    /// Rust: the store's clock is the one every other timestamp here comes
    /// from, and comms carries no date library to disagree with it.
    pub fn within_quiet_hours(
        &self,
        start_hour: u32,
        end_hour: u32,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        if start_hour == end_hour {
            return Ok(false);
        }
        let mut conn = self.conn.lock().unwrap();
        let row = conn.query_one("SELECT EXTRACT(HOUR FROM now())::INTEGER", &[])?;
        let hour: i32 = row.get(0);
        let (start, end) = (start_hour as i32, end_hour as i32);
        // A window that wraps midnight (22→7) is the normal case, so it is the
        // one spelled out rather than the one left to fall through.
        Ok(if start < end {
            hour >= start && hour < end
        } else {
            hour >= start || hour < end
        })
    }
}

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

// -- row mappers ---------------------------------------------------------

fn row_to_triage(r: &postgres::Row) -> TriageItem {
    TriageItem {
        id: r.get(0),
        from_addr: r.get(1),
        subject: r.get(2),
        snippet: r.get(3),
        internal_date_ms: None, // ms is the write-side field; reads carry text
        internal_date_text: r.get(4),
        stream: r.get(5),
        rationale: r.get(6),
        status: r.get(7),
        first_seen: r.get::<_, Option<String>>(8).unwrap_or_default(),
        last_seen: r.get::<_, Option<String>>(9).unwrap_or_default(),
        classification_method: r.get(10),
        classification_version: r.get(11),
        data_class: r.get(12),
        data_class_rationale: r.get(13),
        data_classification_method: r.get(14),
        data_classification_version: r.get(15),
        gmail_action: r.get(16),
        gmail_action_at: r.get(17),
        purge_after: r.get(18),
        gmail_location: r.get(19),
        gmail_observed_at: r.get(20),
        gmail_sync_status: r.get(21),
        gmail_sync_action: r.get(22),
        gmail_sync_error: r.get(23),
    }
}

fn row_to_feed_list(r: &postgres::Row) -> FeedItem {
    FeedItem {
        id: r.get(0),
        stream: r.get(1),
        kind: r.get(2),
        title: r.get(3),
        url: r.get(4),
        author: r.get(5),
        summary: r.get(6),
        transcript: r.get(7), // selected as NULL::text
        day: r.get::<_, Option<String>>(8).unwrap_or_default(),
        created_at: r.get::<_, Option<String>>(9).unwrap_or_default(),
        status: r.get(10),
        content_status: r
            .get::<_, Option<String>>(11)
            .unwrap_or_else(|| "unknown".into()),
        summary_attempts: r.get::<_, Option<i32>>(12).unwrap_or(0),
        summary_last_error: r.get(13),
        summary_next_attempt: r.get(14),
        captured_via: r.get(15),
        // By name, not by index: this column was appended to four SELECTs whose
        // trailing positions already differ (the list query carries created_at
        // after it, for ordering), and a positional read would have to agree
        // with all of them.
        transcript_source: r
            .get::<_, Option<String>>("transcript_source")
            .unwrap_or_else(|| "unknown".into()),
        // Raw extraction output lives in its own table; `get_raw_content` is
        // the only reader, and only the renormalize path asks for it.
        raw_content: None,
        summary_provenance: None,
    }
}

fn row_to_feed_full(r: &postgres::Row) -> FeedItem {
    FeedItem {
        id: r.get(0),
        stream: r.get(1),
        kind: r.get(2),
        title: r.get(3),
        url: r.get(4),
        author: r.get(5),
        summary: r.get(6),
        transcript: r.get(7),
        day: r.get::<_, Option<String>>(8).unwrap_or_default(),
        created_at: r.get::<_, Option<String>>(9).unwrap_or_default(),
        status: r.get(10),
        content_status: r
            .get::<_, Option<String>>(11)
            .unwrap_or_else(|| "unknown".into()),
        summary_attempts: r.get::<_, Option<i32>>(12).unwrap_or(0),
        summary_last_error: r.get(13),
        summary_next_attempt: r.get(14),
        captured_via: r.get(15),
        // By name, not by index: this column was appended to four SELECTs whose
        // trailing positions already differ (the list query carries created_at
        // after it, for ordering), and a positional read would have to agree
        // with all of them.
        transcript_source: r
            .get::<_, Option<String>>("transcript_source")
            .unwrap_or_else(|| "unknown".into()),
        // Raw extraction output lives in its own table; `get_raw_content` is
        // the only reader, and only the renormalize path asks for it.
        raw_content: None,
        summary_provenance: None,
    }
}

fn epoch_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}

#[cfg(test)]
mod tests {
    use super::*;

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
                let mut conn = store.conn.lock().unwrap();
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
            let mut conn = store.conn.lock().unwrap();
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
            .set_triage_data_class("thread:private", "vault")
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
            .set_triage_data_class("thread:private", "secret")
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

        let mut conn = store.conn.lock().unwrap();
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
            .conn
            .lock()
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
        assert!(!stored.generated_at.is_empty(), "the row stamps its own time");

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
            .items_needing_digest("feed", "current-producer", 3, 50)
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

        // One retry left brings it back.
        parked_row.attempts = 2;
        store.upsert_content_digest(&parked_row).unwrap();
        assert!(store
            .items_needing_digest("feed", "current-producer", 3, 50)
            .unwrap()
            .contains(&parked.id));

        assert!(store
            .items_needing_digest("scouting", "current-producer", 3, 50)
            .is_err());
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
            .conn
            .lock()
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

        assert_eq!(store.record_sweep_failure("gmail-inbox", "auth").unwrap(), 1);
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
        assert_eq!(recovered.consecutive_failures, 0, "success clears the streak");
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
