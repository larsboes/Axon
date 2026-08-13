//! Mail triage state, Gmail lifecycle actions, and triage relevance.

use super::*;

impl Store {
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
        // One predicate, four columns: keep the stored classification when a
        // human set it, and also when the incoming one would be *less* strict.
        // A resweep re-runs the rules, so without the second half an edit that
        // made the classifier less suspicious would walk the whole inbox
        // quietly downgrading rows it had previously called Private -- a rule
        // lowering a class, which is the one thing the escalation rule forbids.
        let preserve_class = "t.data_classification_method = 'human' OR \
             (CASE excluded.data_class WHEN 'vault' THEN 20 WHEN 'personal' THEN 10 ELSE 0 END) < \
             (CASE t.data_class WHEN 'vault' THEN 20 WHEN 'personal' THEN 10 ELSE 0 END)";
        let mut conn = self.conn()?;
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
                     data_class = CASE WHEN {preserve_class}
                        THEN t.data_class ELSE excluded.data_class END,
                     data_class_rationale = CASE WHEN {preserve_class}
                        THEN t.data_class_rationale ELSE excluded.data_class_rationale END,
                     data_classification_method = CASE WHEN {preserve_class}
                        THEN t.data_classification_method ELSE excluded.data_classification_method END,
                     data_classification_version = CASE WHEN {preserve_class}
                        THEN t.data_classification_version ELSE excluded.data_classification_version END,
                     status = CASE WHEN t.status IN ('archived','trashed','missing','executed')
                        THEN 'proposed' ELSE t.status END,
                     gmail_location = 'inbox',
                     gmail_observed_at = now(),
                     gmail_sync_status = 'synced',
                     gmail_sync_error = NULL,
                     purge_after = NULL,
                     last_seen = now()",
                schema = self.schema,
                preserve_class = preserve_class
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
        let mut conn = self.conn()?;
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

    /// Set a mail's class by hand. Same rule as [`Store::set_feed_data_class`],
    /// decided by the same function, on the other table.
    pub fn set_triage_data_class(
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
                "SELECT data_class FROM {}.triage_items WHERE id = $1",
                self.schema
            ),
            &[&id],
        )?
        else {
            return Ok(false);
        };
        let classification =
            human_reclassification(row.get::<_, String>(0).as_str(), data_class, rationale)?;
        let affected = conn.execute(
            &format!(
                "UPDATE {}.triage_items SET
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

    /// Refresh a rule-produced data class while preserving an explicit human
    /// override. Returns false for a missing item or a preserved override.
    ///
    /// The rank comparison in the WHERE clause is the escalation rule, and it
    /// is here rather than only in the human path because this is the one that
    /// runs unattended: a rule edit that made the classifier *less* suspicious
    /// would otherwise walk the whole table quietly downgrading rows it had
    /// previously called Private. It may raise a class, never lower one, and
    /// the human override is preserved on top of that.
    pub fn refresh_triage_data_class(
        &self,
        id: &str,
        classification: &crate::content_item::DataClass,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let mut conn = self.conn()?;
        let affected = conn.execute(
            &format!(
                "UPDATE {}.triage_items SET
                    data_class = $1,
                    data_class_rationale = $2,
                    data_classification_method = $3,
                    data_classification_version = $4
                 WHERE id = $5 AND data_classification_method <> 'human'
                   AND (CASE $1::text WHEN 'vault' THEN 20 WHEN 'personal' THEN 10 ELSE 0 END)
                     >= (CASE data_class WHEN 'vault' THEN 20 WHEN 'personal' THEN 10 ELSE 0 END)",
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
        let mut conn = self.conn()?;
        let affected = conn.execute(
            &format!(
                "UPDATE {}.triage_items SET subject = $1, snippet = $2 WHERE id = $3",
                self.schema
            ),
            &[&subject, &snippet, &id],
        )?;
        Ok(affected > 0)
    }

    /// Record the `Waiting` label locally, after Gmail has already accepted it.
    ///
    /// Ordering is the point: Gmail first, this second. A local flag written
    /// optimistically and then a failed modify call leaves the dashboard showing
    /// a state the mailbox does not have, and the operator's inbox is the thing
    /// they actually look at.
    ///
    /// `waiting_at` is cleared rather than kept on unset. "Waiting since" is only
    /// meaningful while it is true, and a stale timestamp on a cleared row reads
    /// like a currently-blocked thread in any query that forgets to check the
    /// boolean.
    pub fn set_triage_waiting(
        &self,
        id: &str,
        waiting: bool,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let mut conn = self.conn()?;
        let affected = conn.execute(
            &format!(
                "UPDATE {}.triage_items
                    SET waiting = $1,
                        waiting_at = CASE WHEN $1 THEN now() ELSE NULL END
                  WHERE id = $2",
                self.schema
            ),
            &[&waiting, &id],
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
        let mut conn = self.conn()?;
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
        let mut conn = self.conn()?;
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
        let mut conn = self.conn()?;
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
        let mut conn = self.conn()?;
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
        let mut conn = self.conn()?;
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
        let mut conn = self.conn()?;
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
        let mut conn = self.conn()?;
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
        let mut conn = self.conn()?;
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
        let mut conn = self.conn()?;
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
        let mut conn = self.conn()?;
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
        let mut conn = self.conn()?;
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
        let mut conn = self.conn()?;
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

    pub fn get_triage_status(
        &self,
        id: &str,
    ) -> Result<Option<String>, Box<dyn std::error::Error>> {
        let mut conn = self.conn()?;
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
        let mut conn = self.conn()?;
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
                    gmail_sync_error, waiting, waiting_at::text
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
        let mut conn = self.conn()?;
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
                        gmail_sync_error, waiting, waiting_at::text
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
        let mut conn = self.conn()?;
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
        let mut conn = self.conn()?;
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
}
