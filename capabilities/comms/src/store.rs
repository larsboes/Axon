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
    pub status: String,
    pub first_seen: String,
    pub last_seen: String,
}

/// Per-source run bookkeeping (round-trips via record_run/get_source_state).
#[derive(Debug, Clone, PartialEq)]
pub struct SourceState {
    pub source_name: String,
    pub last_run_at: String,
    pub cursor: Option<String>,
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
        let host_end = s[host_start..].find('/').map(|i| host_start + i).unwrap_or(s.len());
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
    fn open_with_schema(database_url: &str, schema: &str) -> Result<Self, Box<dyn std::error::Error>> {
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
                status TEXT NOT NULL DEFAULT 'proposed' CHECK (status IN ('proposed','approved','executed','dismissed')),
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

            ALTER TABLE {schema}.feed_evaluation_factors
                ADD COLUMN IF NOT EXISTS context_json TEXT;

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

            -- #81: which client handed the content over. NULL means the server
            -- fetched it. Free-form rather than a CHECK: the set of clients is
            -- open (extension, CLI, a future share sheet) and a constraint here
            -- would need a migration every time one is added.
            ALTER TABLE {schema}.feed_items
                ADD COLUMN IF NOT EXISTS captured_via TEXT;

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
            CREATE INDEX IF NOT EXISTS idx_feed_stream ON {schema}.feed_items(stream);
            CREATE INDEX IF NOT EXISTS idx_feed_status ON {schema}.feed_items(status);
            CREATE INDEX IF NOT EXISTS idx_feed_day ON {schema}.feed_items(day);
            CREATE INDEX IF NOT EXISTS idx_feed_relevance_score ON {schema}.feed_relevance(score DESC);
            CREATE INDEX IF NOT EXISTS idx_feed_origins_source ON {schema}.feed_origins(source_id);
            CREATE INDEX IF NOT EXISTS idx_feed_evaluations_score ON {schema}.feed_evaluations(overall_score DESC);
            CREATE INDEX IF NOT EXISTS idx_feed_evaluations_revision
                ON {schema}.feed_evaluations(context_revision, evaluator_revision);
            CREATE INDEX IF NOT EXISTS idx_feed_quality_flags_derived
                ON {schema}.feed_quality_flags(derived_at DESC);
            "
        ))?;
        Ok(())
    }

    // -- triage ----------------------------------------------------------

    pub const TRIAGE_STATUSES: [&'static str; 4] = ["proposed", "approved", "executed", "dismissed"];

    /// Upsert a triage proposal. `status` is set to 'proposed' only on first
    /// INSERT and is absent from the ON CONFLICT update; a human's decision
    /// survives the same thread being re-swept. Returns `is_new`.
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
                    (id, from_addr, subject, snippet, internal_date, stream, rationale, status, first_seen, last_seen)
                 VALUES ($1,$2,$3,$4, to_timestamp($5), $6, $7, 'proposed', now(), now())
                 ON CONFLICT (id) DO UPDATE SET
                     from_addr = excluded.from_addr,
                     subject = excluded.subject,
                     snippet = excluded.snippet,
                     internal_date = excluded.internal_date,
                     stream = excluded.stream,
                     rationale = excluded.rationale,
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
            ],
        )?;
        Ok(is_new)
    }

    pub fn set_triage_status(&self, id: &str, status: &str) -> Result<bool, Box<dyn std::error::Error>> {
        if !Self::TRIAGE_STATUSES.contains(&status) {
            return Err(format!(
                "invalid triage status '{status}' -- must be one of: {}",
                Self::TRIAGE_STATUSES.join(", ")
            )
            .into());
        }
        let mut conn = self.conn.lock().unwrap();
        let affected = conn.execute(
            &format!("UPDATE {}.triage_items SET status = $1 WHERE id = $2", self.schema),
            &[&status, &id],
        )?;
        Ok(affected > 0)
    }

    pub fn get_triage_status(&self, id: &str) -> Result<Option<String>, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let row = conn.query_opt(
            &format!("SELECT status FROM {}.triage_items WHERE id = $1", self.schema),
            &[&id],
        )?;
        Ok(row.map(|r| r.get::<_, String>(0)))
    }

    /// List triage items, optionally filtered by status, newest first.
    pub fn list_triage(&self, status: Option<&str>) -> Result<Vec<TriageItem>, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let base = format!(
            "SELECT id, from_addr, subject, snippet, internal_date::text, stream, rationale, status, first_seen::text, last_seen::text
             FROM {}.triage_items",
            self.schema
        );
        let rows = match status {
            Some(s) => conn.query(
                &format!("{base} WHERE status = $1 ORDER BY internal_date DESC NULLS LAST"),
                &[&s],
            )?,
            None => conn.query(&format!("{base} ORDER BY internal_date DESC NULLS LAST"), &[])?,
        };
        Ok(rows.iter().map(row_to_triage).collect())
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
        let summary_revision = summary_provenance.as_ref().map(|value| value.revision.as_str());
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
                     status, content_status, captured_via, normalization_tier,
                     normalization_revision, normalization_completed_at,
                     summary_tier, summary_revision, summary_completed_at)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8, CURRENT_DATE, now(), 'new',
                         $9,$10,$11,$12, CASE WHEN $8::text IS NOT NULL THEN now() END,
                         $13,$14, CASE WHEN $7::text IS NOT NULL THEN now() END)
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
            &format!("SELECT raw FROM {}.feed_raw_content WHERE feed_id = $1", self.schema),
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
            &[&transcript, &content_status, &id, &provenance::NORMALIZATION_REVISION],
        )?;
        Ok(affected > 0)
    }

    pub fn set_feed_status(&self, id: &str, status: &str) -> Result<bool, Box<dyn std::error::Error>> {
        if !Self::FEED_STATUSES.contains(&status) {
            return Err(format!(
                "invalid feed status '{status}' -- must be one of: {}",
                Self::FEED_STATUSES.join(", ")
            )
            .into());
        }
        let mut conn = self.conn.lock().unwrap();
        let affected = conn.execute(
            &format!("UPDATE {}.feed_items SET status = $1 WHERE id = $2", self.schema),
            &[&status, &id],
        )?;
        Ok(affected > 0)
    }

    pub fn get_feed_status(&self, id: &str) -> Result<Option<String>, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let row = conn.query_opt(
            &format!("SELECT status FROM {}.feed_items WHERE id = $1", self.schema),
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
                    summary_last_error = NULL, summary_next_attempt = NULL
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
        let Some(row) = row else { return Ok(Vec::new()) };
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
                    completed_at: row
                        .get::<_, Option<String>>(time_index)
                        .unwrap_or_default(),
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
    pub fn record_summary_attempt(&self, id: &str, error_class: &str) -> Result<bool, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let affected = conn.execute(
            &format!(
                "UPDATE {}.feed_items SET
                    summary_attempts = summary_attempts + 1,
                    summary_last_error = $1,
                    summary_next_attempt = now() + (interval '5 minutes' * power(2, summary_attempts))
                 WHERE id = $2",
                self.schema
            ),
            &[&error_class, &id],
        )?;
        Ok(affected > 0)
    }

    /// Enrichment backlog: items eligible for summarization vs. permanently
    /// failed (≥3 attempts).
    pub fn feed_enrichment_counts(&self) -> Result<EnrichmentCounts, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let row = conn.query_one(
            &format!(
                "SELECT
                    COUNT(*) FILTER (WHERE summary IS NULL AND transcript IS NOT NULL AND summary_attempts < 3) AS pending,
                    COUNT(*) FILTER (WHERE summary IS NULL AND transcript IS NOT NULL AND summary_attempts >= 3) AS failed
                 FROM {}.feed_items",
                self.schema
            ),
            &[],
        )?;
        Ok(EnrichmentCounts {
            pending_summaries: row.get(0),
            failed_summaries: row.get(1),
        })
    }

    /// Distribution of `content_status` across all feed items.
    pub fn feed_content_status_counts(&self) -> Result<ContentStatusCounts, Box<dyn std::error::Error>> {
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
                       content_status, summary_attempts, summary_last_error, summary_next_attempt::text, captured_via
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
                    f.content_status, f.summary_attempts, f.summary_last_error, f.summary_next_attempt::text, f.captured_via, f.created_at
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
    pub fn feed_pending_summaries(&self) -> Result<Vec<FeedItem>, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let rows = conn.query(
            &format!(
                "SELECT id, stream, kind, title, url, author, summary, transcript, day::text, created_at::text, status,
                        content_status, summary_attempts, summary_last_error, summary_next_attempt::text, captured_via
                 FROM {}.feed_items
                 WHERE summary IS NULL
                   AND transcript IS NOT NULL
                   AND summary_attempts < 3
                   AND (summary_next_attempt IS NULL OR summary_next_attempt <= now())
                 ORDER BY created_at DESC",
                self.schema
            ),
            &[],
        )?;
        Ok(rows.iter().map(row_to_feed_full).collect())
    }

    /// Full feed items for a bounded relevance refresh. This intentionally
    /// includes dismissed items: a later filter change can make an old item
    /// useful again, while the human status remains untouched.
    pub fn feed_for_relevance(&self, days: i32, limit: usize) -> Result<Vec<FeedItem>, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let rows = conn.query(
            &format!(
                "SELECT id, stream, kind, title, url, author, summary, transcript, day::text, created_at::text, status,
                        content_status, summary_attempts, summary_last_error, summary_next_attempt::text, captured_via
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
            &format!("SELECT tier FROM {}.feed_evaluations WHERE feed_id = $1", self.schema),
            &[&feed_id],
        )?;
        if current.is_some_and(|row| {
            provenance::tier_rank(incoming_tier)
                < provenance::tier_rank(row.get::<_, String>(0).as_str())
        }) {
            return Ok(false);
        }
        transaction.execute(
            &format!("DELETE FROM {}.feed_relevance WHERE feed_id = $1", self.schema),
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

    pub fn feed_relevance(&self, feed_id: &str) -> Result<Vec<RelevanceMatch>, Box<dyn std::error::Error>> {
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
    pub fn replace_feed_evaluation(&self, evaluation: &FeedEvaluation) -> Result<bool, Box<dyn std::error::Error>> {
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
            &format!("DELETE FROM {}.feed_evaluation_factors WHERE feed_id = $1", self.schema),
            &[&evaluation.feed_id],
        )?;
        for (position, factor) in evaluation.factors.iter().enumerate() {
            let context_json = factor.context.as_ref().map(serde_json::to_string).transpose()?;
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

    pub fn feed_evaluation(&self, feed_id: &str) -> Result<Option<FeedEvaluation>, Box<dyn std::error::Error>> {
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

    pub fn travel_context_snapshot(&self) -> Result<Option<TravelContextSnapshot>, Box<dyn std::error::Error>> {
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

    pub fn feed_origins(&self, feed_id: &str) -> Result<Vec<FeedOrigin>, Box<dyn std::error::Error>> {
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

    pub fn record_run(&self, source_name: &str, cursor: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
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

    pub fn get_source_state(&self, source_name: &str) -> Result<Option<SourceState>, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let row = conn.query_opt(
            &format!(
                "SELECT source_name, last_run_at, cursor FROM {}.source_state WHERE source_name = $1",
                self.schema
            ),
            &[&source_name],
        )?;
        Ok(row.map(|r| SourceState {
            source_name: r.get(0),
            last_run_at: r.get(1),
            cursor: r.get(2),
        }))
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
        content_status: r.get::<_, Option<String>>(11).unwrap_or_else(|| "unknown".into()),
        summary_attempts: r.get::<_, Option<i32>>(12).unwrap_or(0),
        summary_last_error: r.get(13),
        summary_next_attempt: r.get(14),
        captured_via: r.get(15),
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
        content_status: r.get::<_, Option<String>>(11).unwrap_or_else(|| "unknown".into()),
        summary_attempts: r.get::<_, Option<i32>>(12).unwrap_or(0),
        summary_last_error: r.get(13),
        summary_next_attempt: r.get(14),
        captured_via: r.get(15),
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
            std::env::var("COMMS_TEST_DATABASE_URL").unwrap_or_else(|_| crate::config::Config::load().database_url)
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
            status: "proposed".into(),
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
        assert_eq!(feed_id("https://example.com/x/"), feed_id("https://EXAMPLE.com/x#frag"));
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
        store.upsert_triage(&mk_triage("thread:s", "aktiv")).unwrap();
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
            !store.set_triage_status("thread:missing", "dismissed").unwrap(),
            "unknown id -> false"
        );
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
        store.update_feed_summary(&item.id, "distilled", "test-summarizer-v1").unwrap();

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
        assert_eq!(stored.title.as_deref(), Some("Better Title"), "title updated");

        item.summary = Some("older imported summary".into());
        item.summary_provenance = Some(StageProvenance::legacy("old-import"));
        store.upsert_feed(&item).unwrap();
        assert_eq!(
            store.get_feed(&item.id).unwrap().unwrap().summary.as_deref(),
            Some("distilled"),
            "a legacy summary cannot replace a model-tier result"
        );
        let stages = store.feed_stage_results(&item.id).unwrap();
        assert_eq!(
            stages.iter().find(|stage| stage.stage == "summary").unwrap().tier,
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
        assert_eq!(store.get_feed_status(&item.id).unwrap().as_deref(), Some("keeper"));
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
            evaluator_revision: "feed-evaluator-v3-reranking".into(),
            evaluated_at: String::new(),
            factors: vec![EvaluationFactor {
                key: "interest".into(),
                label: "Interessen-Fit".into(),
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
            stored.factors[0].context.as_ref().map(|context| context.id.as_str()),
            Some("trip:one")
        );
        assert_eq!(store.evaluation_summary().unwrap().reranked, 1);

        let mut lower = evaluation.clone();
        lower.mode = "lexical".into();
        lower.overall_score = 0.1;
        lower.evaluator_revision = "fallback-v2".into();
        assert!(!store.replace_feed_evaluation(&lower).unwrap());
        assert_eq!(
            store.feed_evaluation(&item.id).unwrap().unwrap().overall_score,
            0.4,
            "a deterministic ranking cannot replace a model-tier result"
        );
        let stages = store.feed_stage_results(&item.id).unwrap();
        let ranking = stages.iter().find(|stage| stage.stage == "ranking").unwrap();
        assert_eq!(ranking.tier, "model");
        assert_eq!(ranking.revision, "feed-evaluator-v3-reranking");
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
        let counts = store.feed_enrichment_counts().unwrap();
        assert_eq!(counts.pending_summaries, 1);
        assert_eq!(counts.failed_summaries, 0);

        let status_counts = store.feed_content_status_counts().unwrap();
        assert_eq!(status_counts.thin, 1);

        // Record 3 failed attempts.
        store.record_summary_attempt(&item.id, "http_error").unwrap();
        store.record_summary_attempt(&item.id, "http_error").unwrap();
        store.record_summary_attempt(&item.id, "http_error").unwrap();

        // Item should now be marked failed (summary_attempts >= 3) and no longer returned by feed_pending_summaries.
        let pending = store.feed_pending_summaries().unwrap();
        assert!(pending.iter().all(|i| i.id != item.id));

        let counts_after = store.feed_enrichment_counts().unwrap();
        assert_eq!(counts_after.pending_summaries, 0);
        assert_eq!(counts_after.failed_summaries, 1);

        // Updating summary resets attempt counters.
        store.update_feed_summary(&item.id, "Summary fixed", "test-summarizer-v1").unwrap();
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
        assert_eq!(media.len(), 1, "one visible media item (other is dismissed)");
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
            .record_feed_origin(&item1.id, "vault-scan", "/notes/ai.md", Some("Obsidian Link"))
            .unwrap();

        let origins1 = store.feed_origins(&item1.id).unwrap();
        assert_eq!(origins1.len(), 2);

        let filtered = store.list_feed(None, Some("github-trending"), 7, false).unwrap();
        assert_eq!(filtered.len(), 2);

        let filtered_vault = store.list_feed(None, Some("vault-scan"), 7, false).unwrap();
        assert_eq!(filtered_vault.len(), 1);

        let summaries = store.list_origin_summaries().unwrap();
        assert_eq!(summaries.len(), 2);
        let gh_summary = summaries.iter().find(|s| s.source_id == "github-trending").unwrap();
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
        let key_of = |id: &str| runs.iter().find(|r| r.feed_id == id).map(|r| r.run_key.clone());

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
        assert_eq!(key_of(&manual.id), None, "an item with no origin is ungrouped");
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
            store.get_feed(&captured.id).unwrap().unwrap().captured_via.as_deref(),
            Some("axon-clip")
        );

        // A later server-side fetch that yields nothing must not relabel a
        // captured body as fetched — the column describes the stored content.
        let mut empty_refetch = FeedItem::new("https://example.com/members", "news", "article");
        empty_refetch.transcript = None;
        store.upsert_feed(&empty_refetch).unwrap();
        assert_eq!(
            store.get_feed(&captured.id).unwrap().unwrap().captured_via.as_deref(),
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
            store.get_feed(&item.id).unwrap().unwrap().transcript.as_deref(),
            Some("The body.")
        );
        assert_eq!(store.feed_ids_with_raw_content().unwrap(), vec![item.id.clone()]);

        // The point of retention: a rule change rewrites the body from stored
        // raw, and the raw itself is untouched so it can be done again.
        store.set_normalized(&item.id, Some("Rewritten."), "thin").unwrap();
        let after = store.get_feed(&item.id).unwrap().unwrap();
        assert_eq!(after.transcript.as_deref(), Some("Rewritten."));
        assert_eq!(after.content_status, "thin");
        assert_eq!(
            store.get_raw_content(&item.id).unwrap().as_deref(),
            Some("Menu\n\nThe body."),
            "re-normalizing must never disturb the extractor's output"
        );
    }

    #[test]
    fn feed_pending_summaries_finds_only_missing() {
        let (store, _schema) = open_test_store("feed_pending");
        let with_t = mk_feed("https://youtu.be/hastranscript", "youtube", "media");
        store.upsert_feed(&with_t).unwrap();
        let mut no_t = FeedItem::new("https://example.com/no-transcript", "news", "article");
        no_t.transcript = None;
        store.upsert_feed(&no_t).unwrap();

        let pending = store.feed_pending_summaries().unwrap();
        assert_eq!(pending.len(), 1, "only the transcript-bearing, summary-less item");
        assert_eq!(pending[0].id, with_t.id);
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
        assert_eq!(st2.cursor.as_deref(), Some("cur-1"), "cursor preserved when not given");
    }
}
