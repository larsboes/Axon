//! Feed ingest, normalized content, review signals, and relevance.

use super::*;

impl Store {
    // -- feed ------------------------------------------------------------

    pub const FEED_STATUSES: [&'static str; 3] = ["new", "keeper", "dismissed"];

    /// Upsert a feed item. `status`/`day`/`created_at` are set only on first
    /// INSERT and are absent from the ON CONFLICT update. `summary`/`transcript`
    /// use COALESCE so a re-ingest that lacks them never wipes a previously
    /// stored value. Returns `is_new`.
    pub fn upsert_feed(&self, item: &FeedItem) -> Result<bool, Box<dyn std::error::Error>> {
        let mut conn = self.conn()?;
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
            &format!(
                "SELECT data_class, data_classification_method FROM {}.feed_items WHERE id = $1",
                self.schema
            ),
            &[&item.id],
        )?;
        let is_new = existing.is_none();

        // Escalation only, decided in one place. A re-ingest may raise a class
        // -- a collector reclassifying its own source, or a first declaration
        // landing on a row that was undeclared -- and may never lower one. The
        // rule lives in `content_item::admit_reclassification` rather than in
        // this statement so there is one copy of it: this path proposes a
        // machine decision, so a lowering is refused here whatever the item
        // says, and the row keeps what it had.
        //
        // Who decided the stored class is read alongside it, because the four
        // columns move together: at equal class there is no class to change,
        // only a human's method and rationale to erase, and this path erased
        // them until the stored method became an input to the rule.
        let reclassifies = existing.as_ref().is_none_or(|row| {
            crate::content_item::admit_reclassification(
                row.get::<_, String>(0).as_str(),
                row.get::<_, String>(1).as_str(),
                &item.data_class,
                &item.data_classification_method,
                &item.data_class_rationale,
            )
            .is_ok()
        });

        conn.execute(
            &format!(
                "INSERT INTO {schema}.feed_items AS f
                    (id, stream, kind, title, url, author, summary, transcript, day, created_at,
                     status, content_status, transcript_source, captured_via, normalization_tier,
                     normalization_revision, normalization_completed_at,
                     summary_tier, summary_revision, summary_completed_at,
                     data_class, data_class_rationale,
                     data_classification_method, data_classification_version)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8, CURRENT_DATE, now(), 'new',
                         $9,$10,$11,$12,$13, CASE WHEN $8::text IS NOT NULL THEN now() END,
                         $14,$15, CASE WHEN $7::text IS NOT NULL THEN now() END,
                         $17,$18,$19,$20)
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
                     END,
                     data_class = CASE WHEN $16 THEN excluded.data_class ELSE f.data_class END,
                     data_class_rationale = CASE WHEN $16
                         THEN excluded.data_class_rationale ELSE f.data_class_rationale END,
                     data_classification_method = CASE WHEN $16
                         THEN excluded.data_classification_method
                         ELSE f.data_classification_method END,
                     data_classification_version = CASE WHEN $16
                         THEN excluded.data_classification_version
                         ELSE f.data_classification_version END",
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
                &reclassifies,
                &item.data_class,
                &item.data_class_rationale,
                &item.data_classification_method,
                &item.data_classification_version,
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

    /// Set a feed item's class by hand.
    ///
    /// The only path that can lower one, and only because a human is on the
    /// other end of it. `rationale` is optional for an escalation and required
    /// for a de-escalation — the refusal comes back as an `Err`, which the
    /// server turns into a 400, so "I made it Public and cannot say why" never
    /// reaches the database.
    ///
    /// `Ok(false)` is a missing item, never a refusal: a caller has to be able
    /// to tell "no such item" from "not allowed".
    pub fn set_feed_data_class(
        &self,
        id: &str,
        data_class: &str,
        rationale: Option<&str>,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        // Checked before the row is read, so an unknown class is a 400 whatever
        // id it was aimed at -- a caller with a typo learns it from the error
        // rather than from a 404 about the item.
        if !crate::content_item::valid(data_class) {
            return Err(format!(
                "data class must be one of: {}",
                crate::content_item::DATA_CLASSES.join(", ")
            )
            .into());
        }
        let mut conn = self.conn()?;
        let Some(row) = conn.query_opt(
            &format!(
                "SELECT data_class, data_classification_method FROM {}.feed_items WHERE id = $1",
                self.schema
            ),
            &[&id],
        )?
        else {
            return Ok(false);
        };
        let classification = human_reclassification(
            row.get::<_, String>(0).as_str(),
            row.get::<_, String>(1).as_str(),
            data_class,
            rationale,
        )?;
        let affected = conn.execute(
            &format!(
                "UPDATE {}.feed_items SET
                    data_class = $1,
                    data_class_rationale = $2,
                    data_classification_method = $3,
                    data_classification_version = $4
                 WHERE id = $5",
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

    /// The extractor's output for an item, if it was stored with one. Items
    /// ingested before #86 have none — they can only be re-fetched, not
    /// re-normalized, and `renormalize_all` reports them as skipped.
    pub fn get_raw_content(&self, id: &str) -> Result<Option<String>, Box<dyn std::error::Error>> {
        let mut conn = self.conn()?;
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
        let mut conn = self.conn()?;
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
        let mut conn = self.conn()?;
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
        let mut conn = self.conn()?;
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
        let mut conn = self.conn()?;
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
        let mut conn = self.conn()?;
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
        let mut conn = self.conn()?;
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
        let mut conn = self.conn()?;
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
        let mut conn = self.conn()?;
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
        let mut conn = self.conn()?;
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
        let mut conn = self.conn()?;
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
        let mut conn = self.conn()?;
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
        let mut conn = self.conn()?;
        let row = conn.query_opt(
            &format!(
                "SELECT id, stream, kind, title, url, author, summary, transcript, day::text, created_at::text, status,
                       content_status, summary_attempts, summary_last_error, summary_next_attempt::text, captured_via, transcript_source,
                       data_class, data_class_rationale, data_classification_method, data_classification_version
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
        let mut conn = self.conn()?;
        let mut sql = format!(
            "SELECT DISTINCT f.id, f.stream, f.kind, f.title, f.url, f.author, f.summary, NULL::text, f.day::text, f.created_at::text, f.status,
                    f.content_status, f.summary_attempts, f.summary_last_error, f.summary_next_attempt::text, f.captured_via, f.transcript_source,
                    f.data_class, f.data_class_rationale, f.data_classification_method, f.data_classification_version, f.created_at
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
        let mut conn = self.conn()?;
        let rows = conn.query(
            &format!(
                "SELECT id, stream, kind, title, url, author, summary, transcript, day::text, created_at::text, status,
                        content_status, summary_attempts, summary_last_error, summary_next_attempt::text, captured_via, transcript_source,
                        data_class, data_class_rationale, data_classification_method, data_classification_version
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
        let mut conn = self.conn()?;
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
        let mut conn = self.conn()?;
        let rows = conn.query(
            &format!(
                "SELECT id, stream, kind, title, url, author, summary, transcript, day::text, created_at::text, status,
                        content_status, summary_attempts, summary_last_error, summary_next_attempt::text, captured_via, transcript_source,
                        data_class, data_class_rationale, data_classification_method, data_classification_version
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
        let mut conn = self.conn()?;
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
        let mut conn = self.conn()?;
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
}
