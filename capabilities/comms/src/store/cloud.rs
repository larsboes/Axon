//! Reviewed cloud derivatives, dispatch jobs, and content digests.

use super::*;

impl Store {
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

        let mut conn = self.conn()?;
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
        let mut conn = self.conn()?;
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
        let mut conn = self.conn()?;
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
        let mut conn = self.conn()?;
        conn.execute(
            &format!(
                // The backoff is computed here, from `now()` and the attempt
                // count already being written, rather than passed in: the
                // deadline must be on the database's clock, and deriving it
                // from the row's own state is what keeps it from disagreeing
                // with `attempts`. A non-retryable state clears it — a success
                // or a verdict has no next attempt to schedule.
                "INSERT INTO {schema}.content_digests
                     (source, item_id, text, state, shape, depth, focus, producer,
                      source_chars, redactions, attempts, last_error, generated_at, next_attempt)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12, now(),
                     CASE WHEN $4 IN ({retryable})
                          THEN now() + (interval '5 minutes' * power(2, GREATEST($11 - 1, 0)))
                          ELSE NULL END)
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
                     generated_at = now(),
                     next_attempt = EXCLUDED.next_attempt",
                schema = self.schema,
                retryable = retryable_digest_states_sql()
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
        let mut conn = self.conn()?;
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
        let mut conn = self.conn()?;
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
    /// failed retryably, has attempts left, and its backoff window has elapsed.
    /// The `depth = 'standard'` guard on the stale case is load-bearing — an
    /// operator who pressed *detailed* has made a decision, and a model upgrade
    /// must not quietly overwrite it with the automatic rung.
    ///
    /// The backoff clause is what makes a *timed* drain safe. With the attempt
    /// cap alone, a drain every 15 minutes spends all three attempts inside
    /// three-quarters of an hour, so an outage lasting an hour leaves the row
    /// permanently dead — which is the failure this whole path exists to end.
    ///
    /// `producers` is every producer string this machine could currently write,
    /// not one: the role is chosen per item, so a short source may be digested
    /// by a light model and a long one by the strong model on the same pass and
    /// both are current. A row is stale only when its producer is in none of
    /// them.
    pub fn items_needing_digest(
        &self,
        source: &str,
        producers: &[String],
        max_attempts: i32,
        limit: i64,
    ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let (table, order) = match source {
            "mail" => ("triage_items", "internal_date DESC NULLS LAST"),
            "feed" => ("feed_items", "created_at DESC"),
            other => return Err(format!("no digest queue for source {other:?}").into()),
        };
        let mut conn = self.conn()?;
        let rows = conn.query(
            &format!(
                "SELECT i.id
                   FROM {schema}.{table} i
                   LEFT JOIN {schema}.content_digests d
                          ON d.source = $1 AND d.item_id = i.id
                  WHERE d.item_id IS NULL
                     OR (d.producer <> ALL($2) AND d.depth = 'standard')
                     OR (d.state IN ({retryable})
                         AND d.attempts < $3
                         AND (d.next_attempt IS NULL OR d.next_attempt <= now()))
                  ORDER BY i.{order}
                  LIMIT $4",
                schema = self.schema,
                table = table,
                order = order,
                retryable = retryable_digest_states_sql()
            ),
            &[&source, &producers, &max_attempts, &limit],
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
        let mut conn = self.conn()?;
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
        let mut conn = self.conn()?;
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
        let mut conn = self.conn()?;
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
        let mut conn = self.conn()?;
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
        let mut conn = self.conn()?;
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
        let mut conn = self.conn()?;
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
        let mut conn = self.conn()?;
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
}
