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
    ///
    /// One transaction, because two of the columns it writes are governed by a
    /// third: the class decides whether `subject` and `snippet` may be stored
    /// as they arrived, and that class is only known after the stored row has
    /// been read. See `class_after_upsert` below for the split the two rules
    /// make — redaction follows the winning class, freshness follows the thread.
    pub fn upsert_triage(&self, item: &TriageItem) -> Result<bool, Box<dyn std::error::Error>> {
        // Gmail internalDate is epoch-ms; convert to fractional epoch-seconds so
        // the bound param is a plain double for the `unixepoch` modifier below.
        let internal_secs: Option<f64> = item.internal_date_ms.map(|ms| ms as f64 / 1000.0);
        // One predicate, four columns: keep the stored classification when a
        // human set it, and also when the incoming one would be *less* strict.
        // A resweep re-runs the rules, so without the second half an edit that
        // made the classifier less suspicious would walk the whole inbox
        // quietly downgrading rows it had previously called Secret -- a rule
        // lowering a class, which is the one thing the escalation rule forbids.
        // `t.` was an INSERT alias (`INSERT INTO x AS t`), which SQLite has no
        // syntax for: inside DO UPDATE it refers to the existing row by the
        // table's own name. So the predicate is built with the prefix.
        let table = format!("{}_triage_items", self.prefix);
        let preserve_class = format!(
            "{table}.data_classification_method = 'human' OR \
             (CASE excluded.data_class WHEN 'c3' THEN 30 WHEN 'c2' THEN 20 WHEN 'c1' THEN 10 \
              ELSE 0 END) < \
             (CASE {table}.data_class WHEN 'c3' THEN 30 WHEN 'c2' THEN 20 WHEN 'c1' THEN 10 \
              ELSE 0 END)"
        );
        // `?5` is Unix seconds; the column holds the canonical stamp, so the
        // conversion is SQL rather than Rust.
        let internal_date = format!("strftime('{}', ?5, 'unixepoch')", axon_store::STAMP_FORMAT);
        let mut conn = self.conn()?;
        // BEGIN IMMEDIATE, not the default deferred begin: this reads the stored
        // class and then writes, and SQLite answers a failed upgrade to the
        // writer lock with SQLITE_BUSY that `busy_timeout` deliberately does not
        // retry (`axon_store::migrate_once`). Two sweeps and a dashboard write
        // reach this at once.
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let stored: Option<(String, String)> = transaction
            .query_row(
                &format!(
                    "SELECT data_class, data_classification_method
                       FROM {}_triage_items WHERE id = ?1",
                    self.prefix
                ),
                params![&item.id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        let is_new = stored.is_none();

        // The class decides the review fields, one layer below the CASE above.
        // `intake` redacts subject and snippet *before* it builds the row, so an
        // incoming c0/c1 row carries verbatim Gmail text by construction, and
        // writing that over a stored c2/c3 row would undo the redaction while
        // the class column stays strict -- a row marked Redacted in the
        // dashboard holding the text it says it removed. The live way in is
        // ruling 3: the named-person escalation reads the people registry, so a
        // sweep run with the overlay unmounted answers c1 for a thread the last
        // refresh raised to c2 (`people_registry::State::Absent`).
        //
        // The answer is to redact the *incoming* text rather than keep the old,
        // because the two properties belong to different things. Redaction
        // follows the class that wins; freshness follows the thread. Freezing
        // the stored pair bought the first at the cost of the second: from_addr,
        // internal_date and stream keep advancing, so the row would show one
        // message's date beside an older message's subject, for the life of the
        // thread and with no path back short of a human de-escalation.
        let winner = class_after_upsert(stored.as_ref(), &item.data_class);
        let remediation =
            crate::intake::remediate(winner, item.subject.as_deref(), item.snippet.as_deref());
        let (subject, snippet) = match &remediation {
            Some(remediation) => (
                remediation.subject.as_deref(),
                remediation.snippet.as_deref(),
            ),
            None => (item.subject.as_deref(), item.snippet.as_deref()),
        };

        transaction.execute(
            &format!(
                "INSERT INTO {prefix}_triage_items
                    (id, from_addr, subject, snippet, internal_date, stream, rationale,
                     classification_method, classification_version, data_class,
                     data_class_rationale, data_classification_method,
                     data_classification_version, status, gmail_location,
                     gmail_observed_at, gmail_sync_status, first_seen, last_seen)
                 VALUES (?1,?2,?3,?4, {internal_date}, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                         'proposed', 'inbox', {now}, 'synced', {now}, {now})
                 ON CONFLICT (id) DO UPDATE SET
                     from_addr = excluded.from_addr,
                     subject = excluded.subject,
                     snippet = excluded.snippet,
                     internal_date = excluded.internal_date,
                     stream = CASE WHEN {table}.classification_method = 'human'
                        THEN {table}.stream ELSE excluded.stream END,
                     rationale = CASE WHEN {table}.classification_method = 'human'
                        THEN {table}.rationale ELSE excluded.rationale END,
                     classification_method = CASE WHEN {table}.classification_method = 'human'
                        THEN {table}.classification_method ELSE excluded.classification_method END,
                     classification_version = CASE WHEN {table}.classification_method = 'human'
                        THEN {table}.classification_version ELSE excluded.classification_version END,
                     data_class = CASE WHEN {preserve_class}
                        THEN {table}.data_class ELSE excluded.data_class END,
                     data_class_rationale = CASE WHEN {preserve_class}
                        THEN {table}.data_class_rationale ELSE excluded.data_class_rationale END,
                     data_classification_method = CASE WHEN {preserve_class}
                        THEN {table}.data_classification_method ELSE excluded.data_classification_method END,
                     data_classification_version = CASE WHEN {preserve_class}
                        THEN {table}.data_classification_version ELSE excluded.data_classification_version END,
                     status = CASE WHEN {table}.status IN ('archived','trashed','missing','executed')
                        THEN 'proposed' ELSE {table}.status END,
                     gmail_location = 'inbox',
                     gmail_observed_at = {now},
                     gmail_sync_status = 'synced',
                     gmail_sync_error = NULL,
                     purge_after = NULL,
                     last_seen = {now}",
                prefix = self.prefix,
                table = table,
                preserve_class = preserve_class,
                internal_date = internal_date,
                now = axon_store::NOW
            ),
            params![&item.id,
                &item.from_addr,
                &subject,
                &snippet,
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
        transaction.commit()?;
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
        let conn = self.conn()?;
        let affected = conn.execute(
            &format!(
                "UPDATE {}_triage_items SET
                    stream = ?1,
                    rationale = 'Category set manually in Axon.',
                    classification_method = 'human',
                    classification_version = 'manual-v1'
                 WHERE id = ?2",
                self.prefix
            ),
            params![&stream, &id],
        )?;
        Ok(affected > 0)
    }

    /// Set a mail's class by hand. Same rule as [`Store::set_feed_data_class`],
    /// decided by the same function, on the other table.
    ///
    /// One transaction over both halves, for the reason the sweep redacts
    /// before it writes and the refresh pass narrows in the same pass: an
    /// operator selecting **Others** or **Secret** in the dashboard is saying
    /// the stored subject is material this row may not hold. Writing only the
    /// class would leave the row labelled Redacted -- which is what the
    /// dashboard prints from the class alone -- while the one-time code the
    /// rules never matched stays in `subject` until somebody remembers a second
    /// endpoint. The receipt says whether it narrowed, so the operator sees
    /// which of the two happened rather than assuming.
    pub fn set_triage_data_class(
        &self,
        id: &str,
        data_class: &str,
        rationale: Option<&str>,
    ) -> Result<ClassWrite, Box<dyn std::error::Error>> {
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
        // Immediate, for the reason `upsert_triage` states: read then write.
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some((stored_class, stored_method, subject, snippet)) = transaction
            .query_row(
                &format!(
                    "SELECT data_class, data_classification_method, subject, snippet
                       FROM {}_triage_items WHERE id = ?1",
                    self.prefix
                ),
                params![&id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                },
            )
            .optional()?
        else {
            return Ok(ClassWrite::default());
        };
        // A refused reclassification returns here, and the transaction rolls
        // back with it: nothing was written, so there is nothing to undo.
        let classification =
            human_reclassification(&stored_class, &stored_method, data_class, rationale)?;
        let affected = transaction.execute(
            &format!(
                "UPDATE {}_triage_items SET
                    data_class = ?1,
                    data_class_rationale = ?2,
                    data_classification_method = ?3,
                    data_classification_version = ?4
                 WHERE id = ?5",
                self.prefix
            ),
            params![
                &classification.value,
                &classification.rationale,
                &classification.method,
                &classification.version,
                &id,
            ],
        )?;
        let narrowed = narrow_review_fields(
            &transaction,
            &self.prefix,
            id,
            &classification.value,
            subject.as_deref(),
            snippet.as_deref(),
        )?;
        transaction.commit()?;
        Ok(ClassWrite {
            changed: affected > 0,
            narrowed,
        })
    }

    /// Refresh a rule-produced data class while preserving an explicit human
    /// override. Returns false for a missing item or a preserved override.
    ///
    /// The rank comparison in the WHERE clause is the escalation rule, and it
    /// is here rather than only in the human path because this is the one that
    /// runs unattended: a rule edit that made the classifier *less* suspicious
    /// would otherwise walk the whole table quietly downgrading rows it had
    /// previously called Secret. It may raise a class, never lower one, and
    /// the human override is preserved on top of that.
    pub fn refresh_triage_data_class(
        &self,
        id: &str,
        classification: &crate::content_item::DataClass,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        let affected = conn.execute(
            &self.refresh_data_class_sql(),
            params![
                &classification.value,
                &classification.rationale,
                &classification.method,
                &classification.version,
                &id,
            ],
        )?;
        Ok(affected > 0)
    }

    /// [`Store::refresh_triage_data_class`] and the narrowing that class
    /// demands, as one transaction.
    ///
    /// They are one decision, and the sweep already takes them as one: classify,
    /// then redact before the row is written. Split across two connections a
    /// crash between them leaves the row at the new `c2` holding the verbatim
    /// subject `c2` exists to hide -- the exact state the pass set out to
    /// remove, and invisible in the dashboard, which prints "Redacted" from the
    /// class alone.
    ///
    /// The class the redaction is judged against is read back inside the
    /// transaction rather than assumed from the argument: the escalation guard
    /// can refuse the write (a stored `c3` against a re-derived `c2`), and what
    /// the row *now holds* is what the row is.
    pub fn refresh_triage_data_class_and_redact(
        &self,
        id: &str,
        classification: &crate::content_item::DataClass,
    ) -> Result<ClassWrite, Box<dyn std::error::Error>> {
        let mut conn = self.conn()?;
        // Immediate, for the reason `upsert_triage` states: this one writes then
        // reads back and writes again, which is the same upgrade hazard.
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let affected = transaction.execute(
            &self.refresh_data_class_sql(),
            params![
                &classification.value,
                &classification.rationale,
                &classification.method,
                &classification.version,
                &id,
            ],
        )?;
        let Some((stored_class, subject, snippet)) = transaction
            .query_row(
                &format!(
                    "SELECT data_class, subject, snippet FROM {}_triage_items WHERE id = ?1",
                    self.prefix
                ),
                params![&id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()?
        else {
            return Ok(ClassWrite::default());
        };
        let narrowed = narrow_review_fields(
            &transaction,
            &self.prefix,
            id,
            &stored_class,
            subject.as_deref(),
            snippet.as_deref(),
        )?;
        transaction.commit()?;
        Ok(ClassWrite {
            changed: affected > 0,
            narrowed,
        })
    }

    /// The escalation-guarded class UPDATE, written once because two callers
    /// issue it. The rank comparison in the WHERE clause *is* the escalation
    /// rule; a second copy is how one of them would come to be missing it.
    fn refresh_data_class_sql(&self) -> String {
        format!(
            "UPDATE {}_triage_items SET
                data_class = ?1,
                data_class_rationale = ?2,
                data_classification_method = ?3,
                data_classification_version = ?4
             WHERE id = ?5 AND data_classification_method <> 'human'
               AND (CASE ?1 WHEN 'c3' THEN 30 WHEN 'c2' THEN 20 WHEN 'c1' THEN 10 ELSE 0 END)
                 >= (CASE data_class
                         WHEN 'c3' THEN 30 WHEN 'c2' THEN 20 WHEN 'c1' THEN 10 ELSE 0 END)",
            self.prefix
        )
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
        let conn = self.conn()?;
        let affected = conn.execute(
            &format!(
                "UPDATE {}_triage_items SET subject = ?1, snippet = ?2 WHERE id = ?3",
                self.prefix
            ),
            params![&subject, &snippet, &id],
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
        let conn = self.conn()?;
        let affected = conn.execute(
            &format!(
                "UPDATE {}_triage_items
                    SET waiting = ?1,
                        waiting_at = CASE WHEN ?1 THEN {now} ELSE NULL END
                  WHERE id = ?2",
                self.prefix,
                now = axon_store::NOW
            ),
            params![&waiting, &id],
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
        let conn = self.conn()?;
        let affected = conn.execute(
            &format!(
                "UPDATE {}_triage_items SET status = ?1 WHERE id = ?2",
                self.prefix
            ),
            params![&status, &id],
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
        let conn = self.conn()?;
        let affected = match action {
            "archive" => conn.execute(
                &format!(
                    "UPDATE {}_triage_items SET
                        status = 'archived', gmail_action = 'archive',
                        gmail_action_at = {now}, purge_after = NULL,
                        gmail_location = 'archive', gmail_observed_at = {now},
                        gmail_sync_status = 'synced', gmail_sync_error = NULL
                     WHERE id = ?1",
                    self.prefix,
                    now = axon_store::NOW
                ),
                params![&id],
            )?,
            "trash" => conn.execute(
                &format!(
                    "UPDATE {}_triage_items SET
                        status = 'trashed', gmail_action = 'trash',
                        gmail_action_at = {now}, purge_after = {purge},
                        gmail_location = 'trash', gmail_observed_at = {now},
                        gmail_sync_status = 'synced', gmail_sync_error = NULL
                     WHERE id = ?1",
                    self.prefix,
                    now = axon_store::NOW,
                    purge = axon_store::now_offset("'+30 days'")
                ),
                params![&id],
            )?,
            "restore" => conn.execute(
                &format!(
                    "UPDATE {}_triage_items SET
                        status = 'proposed', gmail_action = 'restore',
                        gmail_action_at = {now}, purge_after = NULL,
                        gmail_location = 'inbox', gmail_observed_at = {now},
                        gmail_sync_status = 'synced', gmail_sync_error = NULL
                     WHERE id = ?1",
                    self.prefix,
                    now = axon_store::NOW
                ),
                params![&id],
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
        // No `FOR UPDATE`: SQLite has no row locks and needs none here. The
        // transaction is the lock, because there is exactly one writer.
        let transaction = conn.transaction()?;
        let Some(source_status) = transaction
            .query_row(
                &format!(
                    "SELECT status FROM {}_triage_items WHERE id = ?1",
                    self.prefix
                ),
                params![&id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        else {
            return Err("mail proposal not found".into());
        };
        let allowed = match action {
            "archive" | "trash" => matches!(source_status.as_str(), "proposed" | "approved"),
            "restore" => matches!(source_status.as_str(), "archived" | "trashed"),
            _ => false,
        };
        if !allowed {
            return Err(format!("cannot {action} mail in {source_status} state").into());
        }
        if transaction
            .query_row(
                &format!(
                    "SELECT job_id FROM {}_gmail_action_jobs
                     WHERE triage_id = ?1 AND state = 'queued'",
                    self.prefix
                ),
                params![&id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .is_some()
        {
            return Err("a Gmail action is already queued for this mail".into());
        }
        let job = transaction.query_row(
            &format!(
                "INSERT INTO {}_gmail_action_jobs (triage_id, action, source_status)
                 VALUES (?1,?2,?3)
                 RETURNING job_id, triage_id, action, source_status, attempts",
                self.prefix
            ),
            params![&id, &action, &source_status],
            |job| {
                Ok(GmailActionJob {
                    job_id: job.get(0)?,
                    triage_id: job.get(1)?,
                    action: job.get(2)?,
                    source_status: job.get(3)?,
                    attempts: job.get(4)?,
                })
            },
        )?;
        transaction.execute(
            &format!(
                "UPDATE {}_triage_items SET
                    gmail_sync_status = 'queued', gmail_sync_error = NULL
                 WHERE id = ?1",
                self.prefix
            ),
            params![&id],
        )?;
        transaction.commit()?;
        Ok(job)
    }

    /// Complete both halves of local state atomically after Gmail is known to
    /// be at the requested location. Replaying a completed job is harmless.
    pub fn complete_gmail_action(&self, job_id: i64) -> Result<bool, Box<dyn std::error::Error>> {
        let mut conn = self.conn()?;
        let transaction = conn.transaction()?;
        let Some((id, action, state)) = transaction
            .query_row(
                &format!(
                    "SELECT triage_id, action, state FROM {}_gmail_action_jobs
                     WHERE job_id = ?1",
                    self.prefix
                ),
                params![&job_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?
        else {
            return Ok(false);
        };
        if state == "completed" {
            return Ok(true);
        }
        if state != "queued" {
            return Err("Gmail action job is no longer retryable".into());
        }
        let affected = match action.as_str() {
            "archive" => transaction.execute(
                &format!(
                    "UPDATE {}_triage_items SET
                        status = 'archived', gmail_action = 'archive', gmail_action_at = {now},
                        purge_after = NULL, gmail_location = 'archive', gmail_observed_at = {now},
                        gmail_sync_status = 'synced', gmail_sync_error = NULL WHERE id = ?1",
                    self.prefix,
                    now = axon_store::NOW
                ),
                params![&id],
            )?,
            "trash" => transaction.execute(
                &format!(
                    "UPDATE {}_triage_items SET
                        status = 'trashed', gmail_action = 'trash', gmail_action_at = {now},
                        purge_after = COALESCE(purge_after, {purge}),
                        gmail_location = 'trash', gmail_observed_at = {now},
                        gmail_sync_status = 'synced', gmail_sync_error = NULL WHERE id = ?1",
                    self.prefix,
                    now = axon_store::NOW,
                    purge = axon_store::now_offset("'+30 days'")
                ),
                params![&id],
            )?,
            "restore" => transaction.execute(
                &format!(
                    "UPDATE {}_triage_items SET
                        status = 'proposed', gmail_action = 'restore', gmail_action_at = {now},
                        purge_after = NULL, gmail_location = 'inbox', gmail_observed_at = {now},
                        gmail_sync_status = 'synced', gmail_sync_error = NULL WHERE id = ?1",
                    self.prefix,
                    now = axon_store::NOW
                ),
                params![&id],
            )?,
            _ => return Err("stored Gmail action is invalid".into()),
        };
        if affected == 0 {
            return Ok(false);
        }
        transaction.execute(
            &format!(
                "UPDATE {}_gmail_action_jobs SET
                    state = 'completed', updated_at = {now}, completed_at = {now}, last_error = NULL
                 WHERE job_id = ?1",
                self.prefix,
                now = axon_store::NOW
            ),
            params![&job_id],
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
        let transaction = conn.transaction()?;
        // `LEAST(attempts + 1, 5)` becomes SQLite's two-argument `MIN`, and the
        // whole `now() + interval '1 minute' * n` becomes one `now_offset` with a
        // computed modifier -- so the deadline lands in the canonical format the
        // column's other values are in.
        let Some((triage_id, state)) = transaction
            .query_row(
                &format!(
                    "UPDATE {}_gmail_action_jobs SET
                        attempts = attempts + 1,
                        state = CASE WHEN attempts + 1 >= 5 THEN 'abandoned' ELSE 'queued' END,
                        last_error = ?2, updated_at = {now},
                        next_attempt = {backoff}
                     WHERE job_id = ?1 AND state = 'queued'
                     RETURNING triage_id, state",
                    self.prefix,
                    now = axon_store::NOW,
                    backoff = axon_store::now_offset("'+' || MIN(attempts + 1, 5) || ' minutes'")
                ),
                params![&job_id, &bounded_error],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?
        else {
            return Err("Gmail action job is not queued".into());
        };
        let sync_status = if state == "abandoned" {
            "attention"
        } else {
            "retrying"
        };
        transaction.execute(
            &format!(
                "UPDATE {}_triage_items SET gmail_sync_status = ?1, gmail_sync_error = ?2
                 WHERE id = ?3",
                self.prefix
            ),
            params![&sync_status, &bounded_error, &triage_id],
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
        let transaction = conn.transaction()?;
        let Some(job) = transaction
            .query_row(
                &format!(
                    "SELECT job_id, triage_id, action, source_status
                     FROM {}_gmail_action_jobs
                     WHERE triage_id = ?1 AND state = 'abandoned'
                     ORDER BY job_id DESC LIMIT 1",
                    self.prefix
                ),
                params![&id],
                |row| {
                    Ok(GmailActionJob {
                        job_id: row.get(0)?,
                        triage_id: row.get(1)?,
                        action: row.get(2)?,
                        source_status: row.get(3)?,
                        attempts: 0,
                    })
                },
            )
            .optional()?
        else {
            return Err("no Gmail action needs operator attention".into());
        };
        let job_id = job.job_id;
        transaction.execute(
            &format!(
                "UPDATE {}_gmail_action_jobs SET
                    state = 'queued', attempts = 0, last_error = NULL,
                    next_attempt = {now}, updated_at = {now}, completed_at = NULL
                 WHERE job_id = ?1",
                self.prefix,
                now = axon_store::NOW
            ),
            params![&job_id],
        )?;
        transaction.execute(
            &format!(
                "UPDATE {}_triage_items SET gmail_sync_status = 'queued', gmail_sync_error = NULL
                 WHERE id = ?1",
                self.prefix
            ),
            params![&id],
        )?;
        transaction.commit()?;
        Ok(job)
    }

    /// Cancel only an abandoned job. Queued jobs may already be in flight in
    /// the maintenance worker, so canceling them would create an ambiguous
    /// Gmail/local split.
    pub fn cancel_abandoned_gmail_action(
        &self,
        id: &str,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let mut conn = self.conn()?;
        let transaction = conn.transaction()?;
        let canceled = transaction
            .query_row(
                &format!(
                    "UPDATE {}_gmail_action_jobs SET
                        state = 'canceled', updated_at = {now}, completed_at = {now}
                     WHERE job_id = (
                        SELECT job_id FROM {}_gmail_action_jobs
                        WHERE triage_id = ?1 AND state = 'abandoned'
                        ORDER BY job_id DESC LIMIT 1
                     )
                     RETURNING triage_id",
                    self.prefix,
                    self.prefix,
                    now = axon_store::NOW
                ),
                params![&id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if canceled.is_none() {
            return Ok(false);
        }
        transaction.execute(
            &format!(
                "UPDATE {}_triage_items SET
                    gmail_sync_status = CASE WHEN gmail_location IS NULL THEN NULL ELSE 'synced' END,
                    gmail_sync_error = NULL
                 WHERE id = ?1",
                self.prefix
            ),
            params![&id],
        )?;
        transaction.commit()?;
        Ok(true)
    }

    pub fn pending_gmail_actions(
        &self,
        limit: i64,
    ) -> Result<Vec<GmailActionJob>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        Ok(conn.query_all(
            &format!(
                "SELECT job_id, triage_id, action, source_status, attempts
                 FROM {}_gmail_action_jobs
                 WHERE state = 'queued' AND next_attempt <= {now}
                 ORDER BY next_attempt, job_id LIMIT ?1",
                self.prefix,
                now = axon_store::NOW
            ),
            params![limit.clamp(1, 100)],
            |row| {
                Ok(GmailActionJob {
                    job_id: row.get(0)?,
                    triage_id: row.get(1)?,
                    action: row.get(2)?,
                    source_status: row.get(3)?,
                    attempts: row.get(4)?,
                })
            },
        )?)
    }

    pub fn gmail_reconcile_candidates(
        &self,
        limit: i64,
    ) -> Result<Vec<GmailReconcileCandidate>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        Ok(conn.query_all(
            &format!(
                "SELECT id, status FROM {}_triage_items t
                 WHERE status <> 'dismissed'
                   AND NOT EXISTS (
                     SELECT 1 FROM {}_gmail_action_jobs j
                     WHERE j.triage_id = t.id AND j.state IN ('queued','abandoned')
                   )
                 ORDER BY gmail_observed_at ASC NULLS FIRST, last_seen DESC
                 LIMIT ?1",
                self.prefix, self.prefix
            ),
            params![limit.clamp(1, 500)],
            |row| {
                Ok(GmailReconcileCandidate {
                    triage_id: row.get(0)?,
                    status: row.get(1)?,
                })
            },
        )?)
    }

    pub fn observe_gmail_location(
        &self,
        id: &str,
        location: &str,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        if !matches!(location, "inbox" | "archive" | "trash") {
            return Err("Gmail location must be inbox, archive, or trash".into());
        }
        let conn = self.conn()?;
        let affected = conn.execute(
            &format!(
                "UPDATE {}_triage_items SET
                    status = CASE
                      WHEN ?1 = 'trash' THEN 'trashed'
                      WHEN ?1 = 'archive' THEN 'archived'
                      WHEN status IN ('archived','trashed','missing','executed') THEN 'proposed'
                      ELSE status
                    END,
                    purge_after = CASE
                      WHEN ?1 = 'trash' THEN COALESCE(purge_after, {purge})
                      ELSE NULL
                    END,
                    gmail_location = ?1, gmail_observed_at = {now},
                    gmail_sync_status = 'synced', gmail_sync_error = NULL
                 WHERE id = ?2",
                self.prefix,
                now = axon_store::NOW,
                purge = axon_store::now_offset("'+30 days'")
            ),
            params![&location, &id],
        )?;
        Ok(affected > 0)
    }

    /// Record an authoritative Gmail 404/410 without discarding Axon's local
    /// metadata. Any queued or attention action is closed because Gmail can no
    /// longer apply it. A Trash retention deadline, if present, remains active.
    pub fn observe_gmail_missing(&self, id: &str) -> Result<bool, Box<dyn std::error::Error>> {
        let mut conn = self.conn()?;
        let transaction = conn.transaction()?;
        transaction.execute(
            &format!(
                "UPDATE {}_gmail_action_jobs SET
                    state = 'canceled', updated_at = {now}, completed_at = {now}
                 WHERE triage_id = ?1 AND state IN ('queued','abandoned')",
                self.prefix,
                now = axon_store::NOW
            ),
            params![&id],
        )?;
        let affected = transaction.execute(
            &format!(
                "UPDATE {}_triage_items SET
                    status = 'missing', gmail_location = 'missing', gmail_observed_at = {now},
                    gmail_sync_status = 'synced', gmail_sync_error = NULL
                 WHERE id = ?1",
                self.prefix,
                now = axon_store::NOW
            ),
            params![&id],
        )?;
        transaction.commit()?;
        Ok(affected > 0)
    }

    /// Remove expired Trash content and any staged cloud copy. Gmail owns its
    /// own Trash retention; this cleanup is strictly Axon's local copy.
    pub fn purge_expired_trashed(&self) -> Result<u64, Box<dyn std::error::Error>> {
        let mut conn = self.conn()?;
        let transaction = conn.transaction()?;
        transaction.execute(
            &format!(
                "DELETE FROM {prefix}_content_cloud_jobs
                 WHERE source = 'mail' AND item_id IN (
                    SELECT id FROM {prefix}_triage_items
                    WHERE status IN ('trashed','missing') AND purge_after <= {now}
                 )",
                prefix = self.prefix,
                now = axon_store::NOW
            ),
            [],
        )?;
        transaction.execute(
            &format!(
                "DELETE FROM {prefix}_content_cloud_derivatives
                 WHERE source = 'mail' AND item_id IN (
                    SELECT id FROM {prefix}_triage_items
                    WHERE status IN ('trashed','missing') AND purge_after <= {now}
                 )",
                prefix = self.prefix,
                now = axon_store::NOW
            ),
            [],
        )?;
        let purged = transaction.execute(
            &format!(
                "DELETE FROM {}_triage_items
                 WHERE status IN ('trashed','missing') AND purge_after <= {now}",
                self.prefix,
                now = axon_store::NOW
            ),
            [],
        )?;
        transaction.commit()?;
        Ok(purged as u64)
    }

    pub fn get_triage_status(
        &self,
        id: &str,
    ) -> Result<Option<String>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        Ok(conn
            .query_row(
                &format!(
                    "SELECT status FROM {}_triage_items WHERE id = ?1",
                    self.prefix
                ),
                params![&id],
                |r| r.get::<_, String>(0),
            )
            .optional()?)
    }

    /// List triage items, optionally filtered by status, newest first.
    pub fn list_triage(
        &self,
        status: Option<&str>,
    ) -> Result<Vec<TriageItem>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        // The `::text` casts on every timestamp column are gone: they are TEXT now.
        let base = format!(
            "SELECT id, from_addr, subject, snippet, internal_date, stream, rationale,
                    status, first_seen, last_seen,
                    classification_method, classification_version, data_class,
                    data_class_rationale, data_classification_method,
                    data_classification_version, gmail_action,
                    gmail_action_at, purge_after, gmail_location,
                    gmail_observed_at, gmail_sync_status,
                    (SELECT action FROM {prefix}_gmail_action_jobs j
                     WHERE j.triage_id = t.id AND j.state IN ('queued','abandoned')
                     ORDER BY job_id DESC LIMIT 1),
                    gmail_sync_error, waiting, waiting_at
             FROM {prefix}_triage_items t",
            prefix = self.prefix
        );
        Ok(match status {
            Some(s) => conn.query_all(
                &format!("{base} WHERE status = ?1 ORDER BY internal_date DESC NULLS LAST"),
                params![&s],
                row_to_triage,
            )?,
            None => conn.query_all(
                &format!("{base} ORDER BY internal_date DESC NULLS LAST"),
                [],
                row_to_triage,
            )?,
        })
    }

    /// Read one mail proposal for the shared content reader. Gmail-specific
    /// category and action state stays on `triage_items`; the HTTP adapter
    /// projects it into the same content contract as a normal Feed item.
    pub fn get_triage(&self, id: &str) -> Result<Option<TriageItem>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        Ok(conn
            .query_row(
                &format!(
                    "SELECT id, from_addr, subject, snippet, internal_date, stream, rationale,
                        status, first_seen, last_seen,
                        classification_method, classification_version, data_class,
                        data_class_rationale, data_classification_method,
                        data_classification_version, gmail_action,
                        gmail_action_at, purge_after, gmail_location,
                        gmail_observed_at, gmail_sync_status,
                        (SELECT action FROM {prefix}_gmail_action_jobs j
                         WHERE j.triage_id = t.id AND j.state IN ('queued','abandoned')
                         ORDER BY job_id DESC LIMIT 1),
                        gmail_sync_error, waiting, waiting_at
                     FROM {prefix}_triage_items t WHERE id = ?1",
                    prefix = self.prefix
                ),
                params![&id],
                row_to_triage,
            )
            .optional()?)
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
        let transaction = conn.transaction()?;
        transaction.execute(
            &format!(
                "DELETE FROM {}_triage_relevance WHERE triage_id = ?1",
                self.prefix
            ),
            params![&triage_id],
        )?;
        for relevance in matches {
            transaction.execute(
                &format!(
                    "INSERT INTO {prefix}_triage_relevance
                        (triage_id, profile_key, profile_label, score, rationale, mode,
                         profile_revision, scored_at)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,{now})",
                    prefix = self.prefix,
                    now = axon_store::NOW
                ),
                params![
                    &triage_id,
                    &relevance.profile_key,
                    &relevance.profile_label,
                    relevance.score,
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
        let conn = self.conn()?;
        Ok(conn.query_all(
            &format!(
                "SELECT profile_key, profile_label, score, rationale, mode, profile_revision
                 FROM {}_triage_relevance WHERE triage_id = ?1 ORDER BY score DESC",
                self.prefix
            ),
            params![&triage_id],
            |row| {
                Ok(RelevanceMatch {
                    profile_key: row.get(0)?,
                    profile_label: row.get(1)?,
                    score: row.get(2)?,
                    rationale: row.get(3)?,
                    mode: row.get(4)?,
                    profile_revision: row.get(5)?,
                })
            },
        )?)
    }
}

/// Which class the row holds once an upsert lands: the stored one, or the
/// incoming one.
///
/// The Rust twin of the `preserve_class` predicate `upsert_triage` builds into
/// its SQL -- the same two clauses in the same order. A human's decision stands,
/// and a rule may never lower a class it once raised. It exists in Rust as well
/// because the answer decides more than a column: the two review fields have to
/// be redacted against the class that wins *before* the row is written, and SQL
/// cannot run the redactor.
///
/// `class_rank` answers `None` for a value outside the vocabulary, and `None`
/// sorts below every `Some` -- which is the `ELSE 0` arm of the SQL CASE, so an
/// unknown class loses to a known one on either side, exactly as it does there.
fn class_after_upsert<'a>(stored: Option<&'a (String, String)>, incoming: &'a str) -> &'a str {
    match stored {
        Some((stored_class, stored_method))
            if stored_method == crate::content_item::METHOD_HUMAN
                || crate::content_item::class_rank(incoming)
                    < crate::content_item::class_rank(stored_class) =>
        {
            stored_class
        }
        _ => incoming,
    }
}

/// Narrow one row's two review fields to what its class admits, on a connection
/// the caller owns.
///
/// A connection rather than the pool, so the class write and this one commit
/// together or not at all. Returns whether anything was removed -- false both
/// for a class that governs nothing (`c0`, `c1`) and for a row already clean,
/// which is what makes a second pass report zero.
fn narrow_review_fields(
    conn: &Connection,
    prefix: &str,
    id: &str,
    data_class: &str,
    subject: Option<&str>,
    snippet: Option<&str>,
) -> Result<bool, Box<dyn std::error::Error>> {
    let Some(remediation) = crate::intake::remediate(data_class, subject, snippet) else {
        return Ok(false);
    };
    if !remediation.changed {
        return Ok(false);
    }
    conn.execute(
        &format!("UPDATE {prefix}_triage_items SET subject = ?1, snippet = ?2 WHERE id = ?3"),
        params![&remediation.subject, &remediation.snippet, &id],
    )?;
    Ok(true)
}
