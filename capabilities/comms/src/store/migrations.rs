//! Connection ownership and the ordered comms schema migration.

use super::*;

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
    pub(super) fn open_with_schema(
        database_url: &str,
        schema: &str,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        // A pool checkout, not a connect, and the migration runs once per process
        // per (database, schema) rather than once per open. Both halves of the
        // Store::open problem -- libs/axon-store/README.md has the numbers.
        let pool = axon_store::open_pool(database_url, schema, |client| {
            Self::run_migration(client, schema)
        })?;
        Ok(Self {
            pool,
            schema: schema.to_string(),
        })
    }

    /// A connection from the shared pool, for the duration of one statement.
    ///
    /// Returns a `Result` where this used to be `self.conn.lock().unwrap()`. That
    /// unwrap could only fail on a poisoned mutex, which is to say never in
    /// practice; a checkout can genuinely fail — the database is down, or every
    /// connection is busy — and a store method has somewhere to put that.
    pub(super) fn conn(&self) -> Result<axon_store::PooledClient, Box<dyn std::error::Error>> {
        Ok(self.pool.get()?)
    }

    /// The cheapest statement that proves this store can actually reach its database.
    ///
    /// A checkout from the pool is not enough on its own — the point is to fail exactly when a
    /// real query would, which is what the readiness surface promises its caller (#126). `pub`
    /// rather than `pub(super)` because the server's `/ready` handler is the caller.
    pub fn ping(&self) -> Result<(), Box<dyn std::error::Error>> {
        let mut conn = self.conn()?;
        conn.query_one("SELECT 1", &[])?;
        Ok(())
    }

    fn run_migration(client: &mut Client, schema: &str) -> Result<(), Box<dyn std::error::Error>> {
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
            -- Earliest time a failed digest may be retried. Null means eligible
            -- immediately, which is exactly what every pre-existing row means:
            -- they were written when nothing retried them at all, so the drain
            -- should pick them up on its first pass. See RETRYABLE_DIGEST_STATES.
            ALTER TABLE {schema}.content_digests
                ADD COLUMN IF NOT EXISTS next_attempt TIMESTAMPTZ;

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

            -- The doctrine's one state label, mirrored locally so the dashboard
            -- can rank on it without asking Gmail. Gmail stays authoritative:
            -- this is only written after its modify call succeeds, and a sweep
            -- that sees the label gone clears it.
            ALTER TABLE {schema}.triage_items
                ADD COLUMN IF NOT EXISTS waiting BOOLEAN NOT NULL DEFAULT false;
            ALTER TABLE {schema}.triage_items
                ADD COLUMN IF NOT EXISTS waiting_at TIMESTAMPTZ;

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
}
