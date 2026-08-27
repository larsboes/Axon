//! Connection ownership and the ordered comms migration.

use super::*;

impl Store {
    pub fn open(database_path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        Self::open_with_prefix(database_path, "comms")
    }

    /// `prefix` is always either the literal `"comms"` (production) or a
    /// test-generated name -- never user input. SQL has no
    /// parametrized-identifier syntax for CREATE TABLE, so prefixed names are
    /// built via `format!`; that is safe specifically because the prefix's
    /// origin is one of those two controlled cases, not because SQL
    /// interpolation is safe in general.
    pub(super) fn open_with_prefix(
        database_path: &Path,
        prefix: &str,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        // A pool checkout, and the migration runs once per process per (file,
        // prefix) rather than once per open -- libs/axon-store/README.md has why.
        let pool = axon_store::open_pool(database_path, prefix, |conn| {
            Self::run_migration(conn, prefix)
        })?;
        Ok(Self {
            pool,
            prefix: prefix.to_string(),
        })
    }

    /// A connection from the shared pool, for the duration of one statement.
    ///
    /// Returns a `Result` where this used to be `self.conn.lock().unwrap()`. That
    /// unwrap could only fail on a poisoned mutex, which is to say never in
    /// practice; a checkout can genuinely fail — the file is unreachable, or every
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
        let conn = self.conn()?;
        conn.query_row("SELECT 1", [], |row| row.get::<_, i64>(0))?;
        Ok(())
    }

    /// The current shape of the nineteen tables, not the history that produced them.
    ///
    /// This is where PRD Q45's DDL rule does the most work. The Postgres migration
    /// replayed 62 `ALTER TABLE`s, 21 DROP/ADD CONSTRAINT pairs, seven backfill
    /// `UPDATE`s and one PL/pgSQL `DO` block, because `CREATE TABLE IF NOT EXISTS`
    /// never touches an installed table and every widened CHECK had to be re-stated
    /// for databases that already held rows. SQLite has none of those forms: no
    /// conditional ADD COLUMN, no alterable constraint, no procedural block. So each
    /// retrofitted column is declared here and each widened CHECK is the inline one.
    ///
    /// That is only correct because no deployed SQLite file predates this migration.
    /// The backfills are gone for the same reason — every one of them described rows
    /// written before a column existed, and there are none.
    ///
    /// Order is load-bearing: a batch executes top to bottom, so a referenced table
    /// is declared before the table that references it.
    fn run_migration(conn: &Connection, prefix: &str) -> Result<(), Box<dyn std::error::Error>> {
        conn.execute_batch(&format!(
            "
            CREATE TABLE IF NOT EXISTS {prefix}_triage_items (
                id TEXT PRIMARY KEY,
                from_addr TEXT,
                subject TEXT,
                snippet TEXT,
                internal_date TEXT,
                stream TEXT NOT NULL CHECK (stream IN ('aktiv','issue','feed','werbung','belege','steuern','sonstiges')),
                rationale TEXT NOT NULL,
                classification_method TEXT NOT NULL DEFAULT 'deterministic'
                    CHECK (classification_method IN ('legacy','deterministic','model','human')),
                classification_version TEXT NOT NULL DEFAULT 'mail-rules-v1',
                data_class TEXT NOT NULL DEFAULT 'personal'
                    CHECK (data_class IN ('public','personal','vault')),
                data_class_rationale TEXT NOT NULL DEFAULT 'Mail metadata is Personal by default.',
                data_classification_method TEXT NOT NULL DEFAULT 'deterministic'
                    CHECK (data_classification_method IN ('legacy','deterministic','model','human')),
                data_classification_version TEXT NOT NULL DEFAULT 'data-class-rules-v1',
                -- The widened set: archived/trashed/missing arrived with the Gmail
                -- mirror and were a DROP/ADD CONSTRAINT pair under Postgres.
                status TEXT NOT NULL DEFAULT 'proposed'
                    CHECK (status IN ('proposed','approved','executed','archived','trashed','missing','dismissed')),
                gmail_action TEXT
                    CHECK (gmail_action IS NULL OR gmail_action IN ('archive','trash','restore')),
                gmail_action_at TEXT,
                purge_after TEXT,
                gmail_location TEXT
                    CHECK (gmail_location IS NULL OR gmail_location IN ('inbox','archive','trash','missing')),
                gmail_observed_at TEXT,
                gmail_sync_status TEXT
                    CHECK (gmail_sync_status IS NULL OR gmail_sync_status IN ('synced','queued','retrying','attention')),
                gmail_sync_error TEXT,
                -- The doctrine's one state label, mirrored locally so the dashboard
                -- can rank on it without asking Gmail. Gmail stays authoritative:
                -- this is only written after its modify call succeeds, and a sweep
                -- that sees the label gone clears it. INTEGER because SQLite has no
                -- boolean; rusqlite writes 0/1 and reads a `bool` back.
                waiting INTEGER NOT NULL DEFAULT 0,
                waiting_at TEXT,
                first_seen TEXT NOT NULL,
                last_seen TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS {prefix}_feed_items (
                id TEXT PRIMARY KEY,
                stream TEXT NOT NULL CHECK (stream IN ('news','media')),
                -- The widened set: the share-link extractors (github/arxiv/reddit/
                -- huggingface) were a DROP/ADD CONSTRAINT pair under Postgres.
                kind TEXT NOT NULL CHECK (kind IN ('youtube','instagram','podcast','article','mail','github','arxiv','reddit','huggingface')),
                title TEXT,
                url TEXT NOT NULL,
                author TEXT,
                summary TEXT,
                transcript TEXT,
                day TEXT NOT NULL,
                created_at TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'new' CHECK (status IN ('new','keeper','dismissed')),
                -- #74: enrichment state -- content_status and the summary retry ledger.
                content_status TEXT NOT NULL DEFAULT 'unknown'
                    CHECK (content_status IN ('full','thin','none','unknown')),
                summary_attempts INTEGER NOT NULL DEFAULT 0,
                summary_last_error TEXT,
                summary_next_attempt TEXT,
                summary_attempt_revision TEXT,
                -- #81: which client handed the content over. NULL means the server
                -- fetched it. Free-form rather than a CHECK: the set of clients is
                -- open (extension, CLI, a future share sheet) and a constraint here
                -- would need a migration every time one is added.
                captured_via TEXT,
                -- #78: what the stored text IS, as against how much of it there is.
                -- Constrained, unlike captured_via, because this set is closed by
                -- the enum that writes it (extraction::TranscriptSource).
                transcript_source TEXT NOT NULL DEFAULT 'unknown'
                    CHECK (transcript_source IN ('full-text','abstract','unknown')),
                -- #77: producer provenance lives beside each stage value.
                normalization_tier TEXT
                    CHECK (normalization_tier IS NULL OR normalization_tier IN ('legacy','deterministic','model','human')),
                normalization_revision TEXT,
                normalization_completed_at TEXT,
                summary_tier TEXT
                    CHECK (summary_tier IS NULL OR summary_tier IN ('legacy','deterministic','model','human')),
                summary_revision TEXT,
                summary_completed_at TEXT,
                -- Data classification. The default is the fail-closed pair, and both
                -- halves carry weight: an item nobody classified is 'personal', so a
                -- collector cannot produce a cloud-eligible row by omission, and its
                -- method is 'legacy' (rank 0) so the first real decision of any kind
                -- outranks it. 'public' has to be positively declared.
                data_class TEXT NOT NULL DEFAULT 'personal'
                    CHECK (data_class IN ('public','personal','vault')),
                data_class_rationale TEXT NOT NULL
                    DEFAULT 'No collector declared a class for this item; Personal by default.',
                data_classification_method TEXT NOT NULL DEFAULT 'legacy'
                    CHECK (data_classification_method IN ('legacy','deterministic','model','human')),
                data_classification_version TEXT NOT NULL DEFAULT 'data-class-legacy-v1'
            );

            -- Outcome of the last pass, so an unattended schedule can be read
            -- rather than trusted. Counts and an error class only: a scheduler
            -- log that quotes a subject is the same leak the sweep gate closes.
            CREATE TABLE IF NOT EXISTS {prefix}_source_state (
                source_name TEXT PRIMARY KEY,
                last_run_at TEXT,
                cursor TEXT,
                last_success_at TEXT,
                last_failure_at TEXT,
                last_error TEXT,
                considered_count INTEGER NOT NULL DEFAULT 0,
                new_count INTEGER NOT NULL DEFAULT 0,
                consecutive_failures INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS {prefix}_feed_relevance (
                feed_id TEXT NOT NULL REFERENCES {prefix}_feed_items(id) ON DELETE CASCADE,
                profile_key TEXT NOT NULL,
                profile_label TEXT NOT NULL,
                score REAL NOT NULL,
                rationale TEXT NOT NULL,
                -- #71: cross-encoder scores are distinct from bi-encoder cosine
                -- scores, so 'reranked' is its own mode rather than 'semantic'.
                mode TEXT NOT NULL CHECK (mode IN ('reranked','semantic','lexical')),
                profile_revision TEXT NOT NULL,
                scored_at TEXT NOT NULL DEFAULT ({now}),
                PRIMARY KEY (feed_id, profile_key)
            );

            CREATE TABLE IF NOT EXISTS {prefix}_triage_relevance (
                triage_id TEXT NOT NULL REFERENCES {prefix}_triage_items(id) ON DELETE CASCADE,
                profile_key TEXT NOT NULL,
                profile_label TEXT NOT NULL,
                score REAL NOT NULL,
                rationale TEXT NOT NULL,
                mode TEXT NOT NULL CHECK (mode IN ('reranked','semantic','lexical')),
                profile_revision TEXT NOT NULL,
                scored_at TEXT NOT NULL DEFAULT ({now}),
                PRIMARY KEY (triage_id, profile_key)
            );

            CREATE TABLE IF NOT EXISTS {prefix}_feed_origins (
                feed_id TEXT NOT NULL REFERENCES {prefix}_feed_items(id) ON DELETE CASCADE,
                source_id TEXT NOT NULL,
                source_ref TEXT NOT NULL,
                label TEXT,
                first_seen TEXT NOT NULL DEFAULT ({now}),
                last_seen TEXT NOT NULL DEFAULT ({now}),
                PRIMARY KEY (feed_id, source_id, source_ref)
            );

            -- #86: extraction output, kept beside the normalized transcript so a
            -- normalization rule change re-runs from here instead of re-fetching.
            -- Its own table, not a column: feed list queries have no business
            -- dragging 20k-character bodies they never read.
            CREATE TABLE IF NOT EXISTS {prefix}_feed_raw_content (
                feed_id TEXT PRIMARY KEY REFERENCES {prefix}_feed_items(id) ON DELETE CASCADE,
                raw TEXT NOT NULL,
                tier TEXT NOT NULL DEFAULT 'legacy'
                    CHECK (tier IN ('legacy','deterministic','model','human')),
                revision TEXT NOT NULL DEFAULT 'legacy-unknown',
                extracted_at TEXT NOT NULL DEFAULT ({now})
            );

            CREATE TABLE IF NOT EXISTS {prefix}_feed_evaluations (
                feed_id TEXT PRIMARY KEY REFERENCES {prefix}_feed_items(id) ON DELETE CASCADE,
                overall_score REAL NOT NULL CHECK (overall_score BETWEEN 0 AND 1),
                explanation TEXT NOT NULL,
                mode TEXT NOT NULL CHECK (mode IN ('reranked','semantic','lexical','unscored')),
                item_revision TEXT NOT NULL,
                context_revision TEXT NOT NULL,
                evaluator_revision TEXT NOT NULL,
                tier TEXT NOT NULL DEFAULT 'legacy'
                    CHECK (tier IN ('legacy','deterministic','model','human')),
                evaluated_at TEXT NOT NULL DEFAULT ({now})
            );

            CREATE TABLE IF NOT EXISTS {prefix}_feed_evaluation_factors (
                feed_id TEXT NOT NULL REFERENCES {prefix}_feed_evaluations(feed_id) ON DELETE CASCADE,
                factor_key TEXT NOT NULL,
                label TEXT NOT NULL,
                score REAL NOT NULL CHECK (score BETWEEN 0 AND 1),
                weight REAL NOT NULL CHECK (weight BETWEEN 0 AND 1),
                rationale TEXT NOT NULL,
                context_json TEXT,
                position INTEGER NOT NULL,
                PRIMARY KEY (feed_id, factor_key)
            );

            CREATE TABLE IF NOT EXISTS {prefix}_feed_context_snapshots (
                context_kind TEXT PRIMARY KEY,
                revision TEXT NOT NULL,
                payload TEXT NOT NULL,
                refreshed_at TEXT NOT NULL DEFAULT ({now})
            );

            -- #79: deterministic suggestions for the human review queue. The
            -- computation replaces this set explicitly; reading it has no side
            -- effects and never invokes an inference provider.
            CREATE TABLE IF NOT EXISTS {prefix}_feed_quality_flags (
                feed_id TEXT NOT NULL REFERENCES {prefix}_feed_items(id) ON DELETE CASCADE,
                signal TEXT NOT NULL,
                reason TEXT NOT NULL,
                evidence TEXT NOT NULL,
                derived_at TEXT NOT NULL DEFAULT ({now}),
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
            CREATE TABLE IF NOT EXISTS {prefix}_content_digests (
                source TEXT NOT NULL CHECK (source IN ('feed','mail','calendar')),
                item_id TEXT NOT NULL,
                text TEXT,
                state TEXT NOT NULL,
                shape TEXT NOT NULL CHECK (shape IN ('none','brief','standard','sectioned')),
                depth TEXT NOT NULL DEFAULT 'standard' CHECK (depth IN ('standard','detailed')),
                focus TEXT NOT NULL DEFAULT '',
                producer TEXT NOT NULL,
                source_chars INTEGER NOT NULL DEFAULT 0 CHECK (source_chars >= 0),
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
                -- Earliest time a failed digest may be retried. Null means
                -- eligible immediately. See RETRYABLE_DIGEST_STATES.
                next_attempt TEXT,
                generated_at TEXT NOT NULL DEFAULT ({now}),
                PRIMARY KEY (source, item_id)
            );

            -- A human-approved derivative is staged locally before a cloud job
            -- can consume it. Staging has no provider identity or side effect.
            --
            -- `original_data_class` admits no 'vault': `cloud_derivative::prepare`
            -- returns Err for vault, so no code can produce one to stage, and a
            -- vocabulary that accepts a value only a binary predating that refusal
            -- could write describes history rather than what may exist.
            CREATE TABLE IF NOT EXISTS {prefix}_content_cloud_derivatives (
                source TEXT NOT NULL CHECK (source IN ('feed','mail')),
                item_id TEXT NOT NULL,
                source_revision TEXT NOT NULL,
                preview_hash TEXT NOT NULL,
                original_data_class TEXT NOT NULL CHECK (original_data_class IN ('public','personal')),
                derivative_data_class TEXT NOT NULL CHECK (derivative_data_class IN ('public','personal')),
                transformation TEXT NOT NULL,
                document TEXT NOT NULL,
                redaction_count INTEGER NOT NULL CHECK (redaction_count >= 0),
                approved_at TEXT NOT NULL DEFAULT ({now}),
                PRIMARY KEY (source, item_id)
            );

            -- One reviewed cloud intent and its bounded execution ledger. The
            -- joined derivative, never the original source, is the provider input.
            CREATE TABLE IF NOT EXISTS {prefix}_content_cloud_jobs (
                job_id TEXT PRIMARY KEY,
                source TEXT NOT NULL CHECK (source IN ('feed','mail')),
                item_id TEXT NOT NULL,
                source_revision TEXT NOT NULL,
                preview_hash TEXT NOT NULL,
                provider_role TEXT NOT NULL CHECK (provider_role LIKE 'cloud\\_%' ESCAPE '\\'),
                task TEXT NOT NULL DEFAULT 'content-analysis-v1',
                status TEXT NOT NULL DEFAULT 'queued' CHECK (status IN ('queued','running','succeeded','failed')),
                queued_at TEXT NOT NULL DEFAULT ({now}),
                provider_calls INTEGER NOT NULL DEFAULT 0 CHECK (provider_calls BETWEEN 0 AND 5),
                started_at TEXT,
                completed_at TEXT,
                last_error TEXT,
                result_json TEXT,
                -- `task` is part of the intent, not decoration: one reviewed
                -- derivative can be asked two different questions (an analysis
                -- and a digest), and without the task in the key the second
                -- upsert would silently overwrite the first job's task.
                CONSTRAINT content_cloud_jobs_intent_key
                    UNIQUE (source, item_id, preview_hash, provider_role, task)
            );

            -- One row per actual provider request. Policy-disabled candidates
            -- never enter this ledger because no request was made. The exact
            -- approved hash follows every attempt, including failover.
            --
            -- AUTOINCREMENT rather than the bare rowid alias BIGSERIAL maps to:
            -- an attempt id is quoted in a ledger, and a rowid reused after the
            -- highest row is deleted would make two attempts share one.
            CREATE TABLE IF NOT EXISTS {prefix}_content_cloud_attempts (
                attempt_id INTEGER PRIMARY KEY AUTOINCREMENT,
                job_id TEXT NOT NULL REFERENCES {prefix}_content_cloud_jobs(job_id) ON DELETE CASCADE,
                sequence INTEGER NOT NULL CHECK (sequence BETWEEN 1 AND 5),
                provider_role TEXT NOT NULL CHECK (provider_role LIKE 'cloud\\_%' ESCAPE '\\'),
                model TEXT NOT NULL,
                preview_hash TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'running' CHECK (status IN ('running','succeeded','failed')),
                started_at TEXT NOT NULL DEFAULT ({now}),
                completed_at TEXT,
                last_error TEXT,
                result_json TEXT,
                UNIQUE (job_id, sequence)
            );

            -- Durable intent for Gmail mutations. The thread id is already the
            -- triage primary key; no message content is copied into this ledger.
            CREATE TABLE IF NOT EXISTS {prefix}_gmail_action_jobs (
                job_id INTEGER PRIMARY KEY AUTOINCREMENT,
                triage_id TEXT NOT NULL REFERENCES {prefix}_triage_items(id) ON DELETE CASCADE,
                action TEXT NOT NULL CHECK (action IN ('archive','trash','restore')),
                source_status TEXT NOT NULL,
                state TEXT NOT NULL DEFAULT 'queued'
                    CHECK (state IN ('queued','completed','abandoned','canceled')),
                attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts BETWEEN 0 AND 5),
                last_error TEXT,
                next_attempt TEXT NOT NULL DEFAULT ({now}),
                created_at TEXT NOT NULL DEFAULT ({now}),
                updated_at TEXT NOT NULL DEFAULT ({now}),
                completed_at TEXT
            );

            -- Index names carry the prefix too: one file is one namespace now, so
            -- `idx_feed_status` would collide where two schemas kept it apart.
            CREATE INDEX IF NOT EXISTS idx_{prefix}_triage_stream ON {prefix}_triage_items(stream);
            CREATE INDEX IF NOT EXISTS idx_{prefix}_triage_status ON {prefix}_triage_items(status);
            CREATE INDEX IF NOT EXISTS idx_{prefix}_triage_purge_after
                ON {prefix}_triage_items(purge_after) WHERE purge_after IS NOT NULL;
            CREATE UNIQUE INDEX IF NOT EXISTS idx_{prefix}_gmail_action_jobs_one_queued
                ON {prefix}_gmail_action_jobs(triage_id) WHERE state = 'queued';
            CREATE INDEX IF NOT EXISTS idx_{prefix}_gmail_action_jobs_retry
                ON {prefix}_gmail_action_jobs(next_attempt) WHERE state = 'queued';
            CREATE INDEX IF NOT EXISTS idx_{prefix}_feed_stream ON {prefix}_feed_items(stream);
            CREATE INDEX IF NOT EXISTS idx_{prefix}_feed_status ON {prefix}_feed_items(status);
            CREATE INDEX IF NOT EXISTS idx_{prefix}_feed_day ON {prefix}_feed_items(day);
            CREATE INDEX IF NOT EXISTS idx_{prefix}_feed_relevance_score
                ON {prefix}_feed_relevance(score DESC);
            CREATE INDEX IF NOT EXISTS idx_{prefix}_triage_relevance_score
                ON {prefix}_triage_relevance(score DESC);
            CREATE INDEX IF NOT EXISTS idx_{prefix}_feed_origins_source
                ON {prefix}_feed_origins(source_id);
            CREATE INDEX IF NOT EXISTS idx_{prefix}_feed_evaluations_score
                ON {prefix}_feed_evaluations(overall_score DESC);
            CREATE INDEX IF NOT EXISTS idx_{prefix}_feed_evaluations_revision
                ON {prefix}_feed_evaluations(context_revision, evaluator_revision);
            CREATE INDEX IF NOT EXISTS idx_{prefix}_feed_quality_flags_derived
                ON {prefix}_feed_quality_flags(derived_at DESC);
            CREATE INDEX IF NOT EXISTS idx_{prefix}_content_cloud_attempts_role_started
                ON {prefix}_content_cloud_attempts(provider_role, started_at);
            CREATE INDEX IF NOT EXISTS idx_{prefix}_content_cloud_derivatives_approved
                ON {prefix}_content_cloud_derivatives(approved_at DESC);
            CREATE INDEX IF NOT EXISTS idx_{prefix}_content_cloud_jobs_queued
                ON {prefix}_content_cloud_jobs(queued_at ASC) WHERE status = 'queued';
            ",
            prefix = prefix,
            now = axon_store::NOW
        ))?;
        Ok(())
    }
}
