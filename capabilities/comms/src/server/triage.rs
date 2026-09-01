use super::*;

#[derive(Debug, Deserialize)]
pub(super) struct TriageParams {
    status: Option<String>,
}

pub(super) async fn triage_handler(Query(params): Query<TriageParams>) -> Json<Value> {
    let result = tokio::task::spawn_blocking(move || -> Option<Vec<TriageOut>> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).ok()?;
        let items = store.list_triage(params.status.as_deref()).ok()?;
        Some(
            items
                .into_iter()
                .map(|item| {
                    let relevance = store.triage_relevance(&item.id).unwrap_or_default();
                    TriageOut::from_store(item, relevance)
                })
                .collect(),
        )
    })
    .await
    .ok()
    .flatten();

    match result {
        Some(items) => Json(json!(items)),
        None => Json(json!({ "error": "triage query failed" })),
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct TriageSweepBody {
    limit: Option<usize>,
    cursor: Option<String>,
}

/// The people registry's state, as a receipt reports it.
///
/// One function because two receipts state it — the manual refresh and the
/// sweep — and a reader comparing the two must not have to work out whether
/// "absent" and "not loaded" are the same word for the same thing.
fn people_registry_receipt() -> (&'static str, usize) {
    match people_registry::state() {
        people_registry::State::Loaded(names) => ("loaded", names),
        people_registry::State::Absent => ("absent", 0),
        people_registry::State::Unreadable => ("unreadable", 0),
    }
}

/// Counts from one sweep pass. No field here can carry mail content — this is
/// what both the HTTP response and the unattended schedule's log are built
/// from, and the schedule writes to a log nobody is watching at the time.
#[derive(Debug, Serialize)]
pub(super) struct SweepOutcome {
    pub(super) fetched: usize,
    pub(super) new_count: usize,
    pub(super) skipped: usize,
    pub(super) redacted: usize,
    /// Whether the named-person rule could run at all, and against how many
    /// names. This is the path that *persists* rows, so a pass run with the
    /// overlay unmounted stores the verbatim subject of every mail that names a
    /// vault-known person and reports the same counts as a pass that simply
    /// found nobody (Q27 ruling 3, `people_registry::State::Absent`). Stated
    /// here, in the same words the refresh receipt uses, so the two runs are
    /// distinguishable afterwards rather than only while somebody is watching.
    pub(super) people_registry: &'static str,
    pub(super) people_registry_names: usize,
    next_cursor: Option<String>,
}

/// The sweep itself, with no HTTP and no scheduling in it. Both the manual
/// route and the timer call this, for the same reason both go through
/// `intake`: two copies of a mail-reading loop is how one of them ends up
/// missing the gate.
pub(super) fn run_inbox_sweep(
    cfg: &Config,
    limit: usize,
    cursor: Option<&str>,
) -> Result<SweepOutcome, String> {
    let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
    let token = google::access_token(&cfg.google_env_path).map_err(|error| error.to_string())?;
    let page = google::list_inbox_threads_page(&token, limit, cursor)
        .map_err(|error| error.to_string())?;
    let (people_registry, people_registry_names) = people_registry_receipt();
    let mut outcome = SweepOutcome {
        fetched: 0,
        new_count: 0,
        skipped: 0,
        redacted: 0,
        people_registry,
        people_registry_names,
        next_cursor: page.next_page_token.clone(),
    };
    for stub in &page.threads {
        let meta = match google::thread_meta(&token, &stub.id) {
            Ok(meta) => meta,
            Err(_) => {
                outcome.skipped += 1;
                continue;
            }
        };
        let intake = intake::from_thread(meta, &cfg.rules);
        if intake.redaction_count() > 0 {
            outcome.redacted += 1;
        }
        if store
            .upsert_triage(&intake.item)
            .map_err(|error| error.to_string())?
        {
            outcome.new_count += 1;
        }
        outcome.fetched += 1;
    }
    Ok(outcome)
}

/// Which bucket a failure falls in, for the stored state. Deliberately lossy:
/// the classes drive backoff and are safe to display, while the provider's own
/// message can quote a request URL or a subject line and is only ever logged.
pub(super) fn sweep_error_class(error: &str) -> &'static str {
    let lowered = error.to_ascii_lowercase();
    if lowered.contains("401") || lowered.contains("403") || lowered.contains("auth") {
        "auth"
    } else if lowered.contains("429") || lowered.contains("quota") || lowered.contains("rate") {
        "quota"
    } else if lowered.contains("timeout") || lowered.contains("connect") || lowered.contains("dns")
    {
        "network"
    } else {
        "unknown"
    }
}

pub(super) async fn triage_sweep_handler(Json(body): Json<TriageSweepBody>) -> HttpResponse {
    let limit = body.limit.unwrap_or(100).clamp(1, 100);
    let cursor = body.cursor.filter(|value| !value.trim().is_empty());
    let result = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let cfg = Config::load();
        let outcome = run_inbox_sweep(&cfg, limit, cursor.as_deref())?;
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        let total_stored = store
            .list_triage(None)
            .map_err(|error| error.to_string())?
            .len();
        Ok(json!({
            "fetched": outcome.fetched,
            "new_count": outcome.new_count,
            "skipped": outcome.skipped,
            "redacted": outcome.redacted,
            "total_stored": total_stored,
            // Same shape and same words as the refresh receipt's, because the
            // question is the same one: did the c2 escalation run blind?
            "people_registry": {
                "state": outcome.people_registry,
                "names": outcome.people_registry_names,
            },
            "next_cursor": outcome.next_cursor,
            "exhausted": outcome.next_cursor.is_none(),
        }))
    })
    .await;

    match result {
        Ok(Ok(value)) => (StatusCode::OK, Json(value)),
        Ok(Err(error)) => (StatusCode::BAD_GATEWAY, Json(json!({ "error": error }))),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        ),
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct TriageRelevanceBody {
    limit: Option<usize>,
}

pub(super) fn loopback_inference_url(value: &str) -> bool {
    let value = value.trim().to_ascii_lowercase();
    value.starts_with("http://127.0.0.1:")
        || value.starts_with("https://127.0.0.1:")
        || value.starts_with("http://localhost:")
        || value.starts_with("https://localhost:")
        || value.starts_with("http://[::1]:")
        || value.starts_with("https://[::1]:")
}

pub(super) async fn triage_relevance_handler(
    Json(body): Json<TriageRelevanceBody>,
) -> HttpResponse {
    let limit = body.limit.unwrap_or(200).clamp(1, 500);
    let result = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        let profiles = relevance::load_profiles(&cfg.relevance);
        let triage = store
            .list_triage(None)
            .map_err(|error| error.to_string())?
            .into_iter()
            .filter(|item| item.status == "proposed" || item.status == "approved")
            .take(limit)
            .collect::<Vec<_>>();
        let items = triage
            .iter()
            .map(|proposal| {
                let mut item = FeedItem::new(
                    &format!("https://mail.google.com/mail/u/0/#all/{}", proposal.id),
                    "news",
                    "mail",
                );
                item.id = proposal.id.clone();
                item.title = proposal.subject.clone();
                item.author = proposal.from_addr.clone();
                item.transcript = proposal.snippet.clone();
                item
            })
            .collect::<Vec<_>>();
        let embedding_role = cfg
            .embedding_role()
            .filter(|role| loopback_inference_url(&role.backend.base_url));
        let reranking_role = cfg
            .reranking_role()
            .filter(|role| loopback_inference_url(&role.backend.base_url));
        let scored = relevance::score_items(
            &items,
            &profiles,
            embedding_role.as_ref(),
            reranking_role.as_ref(),
        );
        let mode = scored
            .iter()
            .flat_map(|item| item.matches.first())
            .map(|matched| matched.mode.clone())
            .next();
        for item in &scored {
            store
                .replace_triage_relevance(&item.feed_id, &item.matches)
                .map_err(|error| error.to_string())?;
        }
        Ok(json!({
            "scored": scored.len(),
            "profile_count": profiles.len(),
            "mode": mode,
            "local_only": true,
        }))
    })
    .await;

    match result {
        Ok(Ok(value)) => (StatusCode::OK, Json(value)),
        Ok(Err(error)) => error_response(StatusCode::BAD_REQUEST, error),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        ),
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct TriageBulkBody {
    ids: Vec<String>,
    action: String,
    stream: Option<String>,
    data_class: Option<String>,
    /// One reason for the whole batch. Required when the change lowers a class
    /// for any item in it, which is decided per item: a batch that raises nine
    /// and lowers one refuses exactly that one, by id, in `failures`.
    rationale: Option<String>,
}

pub(super) fn thread_action_for_job(job: &GmailActionJob) -> Result<ThreadAction, String> {
    match job.action.as_str() {
        "archive" => Ok(ThreadAction::Archive),
        "trash" => Ok(ThreadAction::Trash),
        "restore" if job.source_status == "archived" => Ok(ThreadAction::RestoreArchive),
        "restore" if job.source_status == "trashed" => Ok(ThreadAction::RestoreTrash),
        _ => Err("stored Gmail action has an invalid source state".into()),
    }
}

pub(super) fn target_location(job: &GmailActionJob) -> Result<ThreadLocation, String> {
    match job.action.as_str() {
        "archive" => Ok(ThreadLocation::Archive),
        "trash" => Ok(ThreadLocation::Trash),
        "restore" => Ok(ThreadLocation::Inbox),
        _ => Err("stored Gmail action is invalid".into()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GmailActionOutcome {
    Confirmed { changed: bool },
    Missing,
}

/// Execute one durable intent. Reading the current labels first makes replay
/// safe when Gmail succeeded but the process stopped before the local commit.
pub(super) fn execute_gmail_action_job(
    store: &Store,
    token: &str,
    job: &GmailActionJob,
) -> Result<GmailActionOutcome, String> {
    let meta = match google::thread_meta_lookup(token, &job.triage_id) {
        Ok(Some(meta)) => meta,
        Ok(None) => {
            return store
                .observe_gmail_missing(&job.triage_id)
                .map_err(|_| "Axon could not record that the Gmail thread is missing".to_string())
                .and_then(|updated| {
                    if updated {
                        Ok(GmailActionOutcome::Missing)
                    } else {
                        Err(
                            "mail proposal disappeared before the missing state was recorded"
                                .into(),
                        )
                    }
                });
        }
        Err(error) => {
            let message = error.to_string();
            let _ = store.fail_gmail_action(job.job_id, &message);
            return Err(message);
        }
    };
    let result = (|| -> Result<GmailActionOutcome, String> {
        let changed = ThreadLocation::from_labels(&meta.label_ids) != target_location(job)?;
        if changed {
            google::apply_thread_action(token, &job.triage_id, thread_action_for_job(job)?)
                .map_err(|error| error.to_string())?;
        }
        match store
            .complete_gmail_action(job.job_id)
            .map_err(|_| "Axon could not commit the confirmed Gmail result".to_string())?
        {
            true => Ok(GmailActionOutcome::Confirmed { changed }),
            false => Err("mail proposal disappeared before local completion".into()),
        }
    })();

    if let Err(error) = &result {
        let _ = store.fail_gmail_action(job.job_id, error);
    }
    result
}

#[derive(Debug, Serialize)]
pub(super) struct GmailMaintenanceCounts {
    retried: usize,
    pub(super) recovered: usize,
    pub(super) retry_failures: usize,
    reconciled: usize,
    pub(super) changed: usize,
    read_failures: usize,
    missing: usize,
    content_fetched: bool,
}

pub(super) fn reconciled_status(current: &str, location: ThreadLocation) -> &str {
    match location {
        ThreadLocation::Trash => "trashed",
        ThreadLocation::Archive => "archived",
        ThreadLocation::Inbox if matches!(current, "archived" | "trashed" | "executed") => {
            "proposed"
        }
        ThreadLocation::Inbox => current,
    }
}

pub(super) fn run_gmail_maintenance(
    cfg: &Config,
    reconcile_limit: i64,
) -> Result<GmailMaintenanceCounts, String> {
    let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
    let token = google::access_token(&cfg.google_env_path)
        .map_err(|_| "Google authorization unavailable".to_string())?;
    let jobs = store
        .pending_gmail_actions(50)
        .map_err(|error| error.to_string())?;
    let mut counts = GmailMaintenanceCounts {
        retried: jobs.len(),
        recovered: 0,
        retry_failures: 0,
        reconciled: 0,
        changed: 0,
        read_failures: 0,
        missing: 0,
        content_fetched: false,
    };
    for job in &jobs {
        match execute_gmail_action_job(&store, &token, job) {
            Ok(GmailActionOutcome::Confirmed { .. }) => counts.recovered += 1,
            Ok(GmailActionOutcome::Missing) => counts.missing += 1,
            Err(_) => counts.retry_failures += 1,
        }
    }

    let candidates = store
        .gmail_reconcile_candidates(reconcile_limit)
        .map_err(|error| error.to_string())?;
    for candidate in candidates {
        match google::thread_meta_lookup(&token, &candidate.triage_id) {
            Ok(Some(meta)) => {
                let location = ThreadLocation::from_labels(&meta.label_ids);
                if reconciled_status(&candidate.status, location) != candidate.status {
                    counts.changed += 1;
                }
                store
                    .observe_gmail_location(&candidate.triage_id, location.as_str())
                    .map_err(|error| error.to_string())?;
                counts.reconciled += 1;
            }
            Ok(None) => {
                store
                    .observe_gmail_missing(&candidate.triage_id)
                    .map_err(|error| error.to_string())?;
                counts.reconciled += 1;
                counts.changed += usize::from(candidate.status != "missing");
                counts.missing += 1;
            }
            Err(_) => counts.read_failures += 1,
        }
    }
    Ok(counts)
}

pub(super) async fn triage_bulk_handler(Json(body): Json<TriageBulkBody>) -> HttpResponse {
    if body.ids.is_empty() || body.ids.len() > 100 {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "select between 1 and 100 proposals" })),
        );
    }
    let mut seen = HashSet::new();
    let ids = body
        .ids
        .into_iter()
        .filter(|id| seen.insert(id.clone()))
        .collect::<Vec<_>>();
    let action = body.action;
    let stream = body.stream;
    let selected_data_class = body.data_class;
    let rationale = body.rationale;
    if !matches!(
        action.as_str(),
        "dismiss"
            | "categorize"
            | "set-data-class"
            | "archive"
            | "trash"
            | "waiting"
            | "clear-waiting"
    ) {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                json!({ "error": "action must be dismiss, categorize, set-data-class, archive, trash, waiting, or clear-waiting" }),
            ),
        );
    }
    if action == "categorize" && stream.as_deref().is_none() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "categorize requires a stream" })),
        );
    }
    if action == "set-data-class"
        && !selected_data_class
            .as_deref()
            .is_some_and(content_item::valid)
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": format!(
                    "set-data-class requires one of: {}",
                    content_item::DATA_CLASSES.join(", ")
                )
            })),
        );
    }

    let result = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        let gmail_action = ThreadAction::parse(&action);
        let waiting_action = matches!(action.as_str(), "waiting" | "clear-waiting");
        let token = (gmail_action.is_some() || waiting_action)
            .then(|| google::access_token(&cfg.google_env_path));
        // Resolved once for the batch, not once per thread: the label id is the
        // same for every id here, and looking it up a hundred times would spend
        // a hundred requests to learn the same string.
        let waiting_label = match (waiting_action, token.as_ref()) {
            (true, Some(Ok(token))) => Some(google::ensure_waiting_label(token)),
            _ => None,
        };
        let mut succeeded = Vec::new();
        let mut failures = Vec::new();
        // Rows whose stored subject and snippet this batch narrowed, because
        // the class it set on them does not admit what they held. Zero for
        // every other action.
        let mut narrowed = 0usize;
        for id in ids {
            let known = store
                .get_triage_status(&id)
                .map_err(|error| error.to_string())?;
            if known.is_none() {
                failures.push(json!({ "id": id, "error": "not found" }));
                continue;
            }
            let outcome = match action.as_str() {
                "dismiss" => store
                    .set_triage_status(&id, "dismissed")
                    .map_err(|error| error.to_string())
                    .map(|updated| updated.then_some(())),
                "categorize" => store
                    .set_triage_stream(&id, stream.as_deref().unwrap_or_default())
                    .map_err(|error| error.to_string())
                    .map(|updated| updated.then_some(())),
                "set-data-class" => store
                    .set_triage_data_class(
                        &id,
                        selected_data_class.as_deref().unwrap_or_default(),
                        rationale.as_deref(),
                    )
                    .map_err(|error| error.to_string())
                    .map(|write| {
                        if write.narrowed {
                            narrowed += 1;
                        }
                        write.changed.then_some(())
                    }),
                // Not queued the way archive and trash are, deliberately.
                // The queue exists for a mutation with a restore path and a
                // reconcile story; applying a label has neither. It is
                // idempotent in Gmail, carries no state to drift, and a failure
                // leaves nothing half-done — so a retry is just pressing it
                // again. Gmail first, store second: see `set_triage_waiting`.
                "waiting" | "clear-waiting" => {
                    let want = action == "waiting";
                    match (token.as_ref(), waiting_label.as_ref()) {
                        (Some(Ok(token)), Some(Ok(label))) => {
                            google::set_thread_waiting(token, &id, label, want)
                                .map_err(|error| error.to_string())
                                .and_then(|()| {
                                    store
                                        .set_triage_waiting(&id, want)
                                        .map_err(|error| error.to_string())
                                })
                                .map(|updated| updated.then_some(()))
                        }
                        (Some(Err(_)), _) => Err("Google authorization unavailable".to_string()),
                        (_, Some(Err(error))) => Err(error.to_string()),
                        _ => Err("Google authorization unavailable".to_string()),
                    }
                }
                "archive" | "trash" => store
                    .queue_gmail_action(&id, action.as_str())
                    .map_err(|error| error.to_string())
                    .and_then(|job| match token.as_ref() {
                        Some(Ok(token)) => execute_gmail_action_job(&store, token, &job),
                        Some(Err(_)) => {
                            let _ = store.fail_gmail_action(
                                job.job_id,
                                "Google authorization unavailable",
                            );
                            Err("Google authorization unavailable; the action remains queued for retry".into())
                        }
                        None => unreachable!(),
                    })
                    .and_then(|outcome| match outcome {
                        GmailActionOutcome::Confirmed { .. } => Ok(Some(())),
                        GmailActionOutcome::Missing => {
                            Err("Gmail thread no longer exists; Axon retained its local record".into())
                        }
                    }),
                _ => unreachable!(),
            };
            match outcome {
                Ok(Some(())) => succeeded.push(id),
                Ok(None) => failures.push(json!({ "id": id, "error": "not found" })),
                Err(error) => failures.push(json!({ "id": id, "error": error.to_string() })),
            }
        }
        Ok(json!({
            "succeeded": succeeded,
            "failures": failures,
            "gmail_changed": gmail_action.is_some(),
            "narrowed": narrowed,
        }))
    })
    .await;

    match result {
        Ok(Ok(value)) => (StatusCode::OK, Json(value)),
        Ok(Err(error)) => error_response(StatusCode::BAD_REQUEST, error),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        ),
    }
}

pub(super) async fn triage_status_handler(
    Path(id): Path<String>,
    Json(body): Json<StatusBody>,
) -> HttpResponse {
    if !matches!(body.status.as_str(), "proposed" | "approved" | "dismissed") {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "status must be proposed, approved, or dismissed; Gmail lifecycle states require the Gmail action endpoint"
            })),
        );
    }
    let result = tokio::task::spawn_blocking(move || -> Result<bool, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        store
            .set_triage_status(&id, &body.status)
            .map_err(|error| error.to_string())
    })
    .await;

    match result {
        Ok(Ok(true)) => (StatusCode::OK, Json(json!({ "ok": true }))),
        Ok(Ok(false)) => error_response(StatusCode::NOT_FOUND, "not found"),
        Ok(Err(error)) => error_response(StatusCode::BAD_REQUEST, error),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        ),
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct TriageStreamBody {
    stream: String,
}

pub(super) async fn triage_stream_handler(
    Path(id): Path<String>,
    Json(body): Json<TriageStreamBody>,
) -> HttpResponse {
    let result = tokio::task::spawn_blocking(move || -> Result<bool, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        store
            .set_triage_stream(&id, &body.stream)
            .map_err(|error| error.to_string())
    })
    .await;

    match result {
        Ok(Ok(true)) => (StatusCode::OK, Json(json!({ "ok": true }))),
        Ok(Ok(false)) => error_response(StatusCode::NOT_FOUND, "not found"),
        Ok(Err(error)) => error_response(StatusCode::BAD_REQUEST, error),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        ),
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct TriageDataClassBody {
    data_class: String,
    /// Required when the change lowers the class, ignored when it raises it.
    /// Which of the two this is depends on what is stored, so the store
    /// decides and this handler stays out of it.
    rationale: Option<String>,
}

pub(super) async fn triage_data_class_handler(
    Path(id): Path<String>,
    Json(body): Json<TriageDataClassBody>,
) -> HttpResponse {
    let result = tokio::task::spawn_blocking(move || -> Result<ClassWrite, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        store
            .set_triage_data_class(&id, &body.data_class, body.rationale.as_deref())
            .map_err(|error| error.to_string())
    })
    .await;

    match result {
        // `narrowed` rather than an assumption either way: selecting Others or
        // Secret remediates the stored subject and snippet in the same
        // transaction, and the caller is told whether this row had anything
        // left to remove. A row swept after the intake gate existed answers
        // false, and that is a clean row, not a skipped one.
        Ok(Ok(write)) if write.changed => (
            StatusCode::OK,
            Json(json!({ "ok": true, "narrowed": write.narrowed })),
        ),
        Ok(Ok(_)) => error_response(StatusCode::NOT_FOUND, "not found"),
        Ok(Err(error)) => error_response(StatusCode::BAD_REQUEST, error),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        ),
    }
}

/// Freshness and failure of the unattended schedule, for a dashboard to read.
/// Unauthenticated with the other reads: it carries counts, timestamps and an
/// error class, and no mail ever reaches it.
pub(super) async fn triage_sweep_status_handler() -> HttpResponse {
    let result = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        let state = store
            .get_source_state(INBOX_SWEEP_SOURCE)
            .map_err(|error| error.to_string())?;
        Ok(json!({
            "enabled": cfg.inbox_sweep_minutes > 0,
            "every_minutes": cfg.inbox_sweep_minutes,
            "max_threads": cfg.inbox_sweep_max_threads,
            "quiet_hours": cfg.inbox_sweep_quiet_hours.map(|(s, e)| json!({"start": s, "end": e})),
            "last_run_at": state.as_ref().map(|s| s.last_run_at.clone()),
            "last_success_at": state.as_ref().and_then(|s| s.last_success_at.clone()),
            "last_failure_at": state.as_ref().and_then(|s| s.last_failure_at.clone()),
            "last_error": state.as_ref().and_then(|s| s.last_error.clone()),
            "considered_count": state.as_ref().map(|s| s.considered_count).unwrap_or(0),
            "new_count": state.as_ref().map(|s| s.new_count).unwrap_or(0),
            "consecutive_failures": state.as_ref().map(|s| s.consecutive_failures).unwrap_or(0),
        }))
    })
    .await;

    match result {
        Ok(Ok(value)) => (StatusCode::OK, Json(value)),
        Ok(Err(error)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error })),
        ),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        ),
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct TriageRedactBody {
    limit: Option<usize>,
    /// Report what would change without writing. The default is to write:
    /// this endpoint exists because material is already stored, and a preview
    /// that has to be run twice is a preview that gets run once.
    dry_run: Option<bool>,
}

/// Remediate rows persisted before the intake gate existed.
///
/// Bounded, idempotent, and reviewable: it reports how many rows it examined,
/// how many it changed and what kinds of entity it removed — never the removed
/// values, and never the values it left. Running it twice reports zero changes
/// the second time, which is how you know it finished.
pub(super) async fn triage_redact_handler(Json(body): Json<TriageRedactBody>) -> HttpResponse {
    let limit = body.limit.unwrap_or(500).clamp(1, 2_000);
    let dry_run = body.dry_run.unwrap_or(false);
    let result = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        let items = store
            .list_triage(None)
            .map_err(|error| error.to_string())?
            .into_iter()
            .take(limit)
            .collect::<Vec<_>>();
        let reviewed = items.len();
        let mut in_scope = 0usize;
        let mut changed = 0usize;
        let mut entity_types: BTreeMap<&'static str, usize> = BTreeMap::new();
        let mut digests = Vec::new();

        for item in items {
            let Some(remediation) = intake::remediate(
                &item.data_class,
                item.subject.as_deref(),
                item.snippet.as_deref(),
            ) else {
                continue;
            };
            in_scope += 1;
            if !remediation.changed {
                continue;
            }
            for finding in &remediation.redactions {
                *entity_types.entry(finding.entity_type).or_default() += finding.count;
            }
            if let Some(digest) = remediation.audit_digest.clone() {
                digests.push(json!({ "id": item.id, "digest": digest }));
            }
            if dry_run
                || store
                    .redact_triage_review_fields(
                        &item.id,
                        remediation.subject.as_deref(),
                        remediation.snippet.as_deref(),
                    )
                    .map_err(|error| error.to_string())?
            {
                changed += 1;
            }
        }

        Ok(json!({
            "reviewed": reviewed,
            "in_scope": in_scope,
            "changed": changed,
            "dry_run": dry_run,
            "entity_types": entity_types,
            "audit": digests,
            "transformation": cloud_derivative::REDACTION_VERSION,
            "provider_calls": 0,
        }))
    })
    .await;

    match result {
        Ok(Ok(value)) => (StatusCode::OK, Json(value)),
        Ok(Err(error)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error })),
        ),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        ),
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct TriageDataClassRefreshBody {
    limit: Option<usize>,
}

/// Re-derive one row's class and leave the row in the state that class
/// demands. Returns `(class was rewritten, review fields were narrowed)`.
///
/// Both halves, because they are one decision — and one transaction, because
/// the store owns that. The sweep decides them together in
/// `intake::from_thread`: classify, then redact before the row is written.
/// Splitting them here would mean every row this pass raises to `c2` keeps the
/// verbatim subject `c2` exists to hide until an operator remembers a second
/// endpoint — and the dashboard prints "Redacted" from the class alone, so the
/// gap is invisible in the one place a human would look.
fn refresh_one_triage_class(store: &Store, item: &TriageItem) -> Result<(bool, bool), String> {
    // The sweep's rule set, registry pass included (Q27), so a row re-derived
    // here and a row freshly swept land on the same class.
    let classification = intake::classify_mail(
        &item.stream,
        item.from_addr.as_deref().unwrap_or_default(),
        item.subject.as_deref().unwrap_or_default(),
        item.snippet.as_deref().unwrap_or_default(),
    );
    let write = store
        .refresh_triage_data_class_and_redact(&item.id, &classification)
        .map_err(|error| error.to_string())?;
    Ok((write.changed, write.narrowed))
}

pub(super) async fn triage_data_class_refresh_handler(
    Json(body): Json<TriageDataClassRefreshBody>,
) -> HttpResponse {
    let limit = body.limit.unwrap_or(500).clamp(1, 2_000);
    let result = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        let items = store
            .list_triage(None)
            .map_err(|error| error.to_string())?
            .into_iter()
            .take(limit)
            .collect::<Vec<_>>();
        let reviewed = items.len();
        let mut updated = 0usize;
        let mut redacted = 0usize;
        let mut preserved_human = 0usize;
        for item in items {
            if item.data_classification_method == "human" {
                preserved_human += 1;
                continue;
            }
            let (reclassified, narrowed) = refresh_one_triage_class(&store, &item)?;
            if reclassified {
                updated += 1;
            }
            if narrowed {
                redacted += 1;
            }
        }
        let (registry_state, registry_names) = people_registry_receipt();
        Ok(json!({
            "reviewed": reviewed,
            "updated": updated,
            // Rows whose stored subject/snippet this pass narrowed, because the
            // class they now carry requires it. Reported beside `updated`: the
            // two counts differ when a row was already strict but never
            // remediated, and an operator reading only `updated` would see a
            // quiet run and conclude nothing was exposed.
            "redacted": redacted,
            "preserved_human": preserved_human,
            "transformation": cloud_derivative::REDACTION_VERSION,
            "classifier_version": content_item::MAIL_CLASSIFIER_VERSION,
            "content_inputs": ["sender", "subject", "snippet", "category"],
            "people_registry": {
                "state": registry_state,
                "names": registry_names,
            },
            "provider_calls": 0,
        }))
    })
    .await;

    match result {
        Ok(Ok(value)) => (StatusCode::OK, Json(value)),
        Ok(Err(error)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error })),
        ),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        ),
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct TriageGmailBody {
    action: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct TriageGmailJobBody {
    decision: String,
}

pub(super) async fn triage_gmail_handler(
    Path(id): Path<String>,
    Json(body): Json<TriageGmailBody>,
) -> HttpResponse {
    if !matches!(body.action.as_str(), "archive" | "trash" | "restore") {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "action must be archive, trash, or restore" })),
        );
    }
    let action_name = body.action;
    let response_action = action_name.clone();
    let result = tokio::task::spawn_blocking(
        move || -> Result<GmailActionOutcome, (StatusCode, String)> {
            let cfg = Config::load();
            let store = Store::open(&cfg.database_path)
                .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
            let job = store
                .queue_gmail_action(&id, &action_name)
                .map_err(|error| {
                    let message = error.to_string();
                    let status = if message == "mail proposal not found" {
                        StatusCode::NOT_FOUND
                    } else {
                        StatusCode::CONFLICT
                    };
                    (status, message)
                })?;
            let token = google::access_token(&cfg.google_env_path).map_err(|_| {
                let _ = store.fail_gmail_action(job.job_id, "Google authorization unavailable");
                (
                    StatusCode::BAD_GATEWAY,
                    "Google authorization unavailable; the action remains queued for retry".into(),
                )
            })?;
            execute_gmail_action_job(&store, &token, &job).map_err(|error| {
                (
                    StatusCode::BAD_GATEWAY,
                    format!("{error}; the action remains queued for bounded retry"),
                )
            })
        },
    )
    .await;

    match result {
        Ok(Ok(GmailActionOutcome::Confirmed { changed })) => (
            StatusCode::OK,
            Json(json!({
                "ok": true,
                "action": response_action,
                "gmail_changed": changed,
                "gmail_confirmed": true
            })),
        ),
        Ok(Ok(GmailActionOutcome::Missing)) => (
            StatusCode::GONE,
            Json(json!({
                "error": "Gmail thread no longer exists; Axon retained its local record"
            })),
        ),
        Ok(Err((status, error))) => (status, Json(json!({ "error": error }))),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        ),
    }
}

pub(super) async fn triage_gmail_job_handler(
    Path(id): Path<String>,
    Json(body): Json<TriageGmailJobBody>,
) -> HttpResponse {
    if !matches!(body.decision.as_str(), "retry" | "cancel") {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "decision must be retry or cancel" })),
        );
    }
    let decision = body.decision;
    let result = tokio::task::spawn_blocking(move || {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path)
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
        if decision == "cancel" {
            return store
                .cancel_abandoned_gmail_action(&id)
                .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
                .and_then(|canceled| {
                    if canceled {
                        Ok((StatusCode::OK, json!({ "ok": true, "state": "canceled" })))
                    } else {
                        Err((
                            StatusCode::CONFLICT,
                            "no Gmail action needs operator attention".into(),
                        ))
                    }
                });
        }

        let job = store
            .retry_abandoned_gmail_action(&id)
            .map_err(|error| (StatusCode::CONFLICT, error.to_string()))?;
        let token = google::access_token(&cfg.google_env_path).map_err(|_| {
            let _ = store.fail_gmail_action(job.job_id, "Google authorization unavailable");
            (
                StatusCode::BAD_GATEWAY,
                "Google authorization unavailable; the action remains queued for retry".into(),
            )
        })?;
        match execute_gmail_action_job(&store, &token, &job).map_err(|error| {
            (
                StatusCode::BAD_GATEWAY,
                format!("{error}; the action remains queued for bounded retry"),
            )
        })? {
            GmailActionOutcome::Confirmed { changed } => Ok((
                StatusCode::OK,
                json!({ "ok": true, "state": "completed", "gmail_changed": changed }),
            )),
            GmailActionOutcome::Missing => Ok((
                StatusCode::GONE,
                json!({
                    "error": "Gmail thread no longer exists; Axon retained its local record"
                }),
            )),
        }
    })
    .await;

    match result {
        Ok(Ok((status, value))) => (status, Json(value)),
        Ok(Err((status, error))) => (status, Json(json!({ "error": error }))),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        ),
    }
}

pub(super) async fn triage_reconcile_handler() -> HttpResponse {
    let result = tokio::task::spawn_blocking(|| {
        let cfg = Config::load();
        run_gmail_maintenance(&cfg, 200)
    })
    .await;
    match result {
        Ok(Ok(counts)) => (
            StatusCode::OK,
            Json(serde_json::to_value(counts).unwrap_or_else(|_| json!({}))),
        ),
        Ok(Err(error)) => (StatusCode::BAD_GATEWAY, Json(json!({ "error": error }))),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temp file this test owns. The live store is never opened here: the
    /// rows below are fixtures, and `refresh_one_triage_class` writes.
    fn test_store(name: &str) -> Store {
        let directory =
            std::env::temp_dir().join(format!("comms-server-test-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("a writable temp directory");
        let path = directory.join(format!("{name}.db"));
        for tail in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{tail}", path.display()));
        }
        Store::open(&path).expect("a store on a path this test owns")
    }

    fn stored_row(id: &str, subject: &str, snippet: &str) -> TriageItem {
        TriageItem {
            id: id.into(),
            from_addr: Some("security@example.com".into()),
            subject: Some(subject.into()),
            snippet: Some(snippet.into()),
            internal_date_ms: Some(1_754_000_000_000),
            internal_date_text: None,
            stream: "aktiv".into(),
            rationale: "test".into(),
            classification_method: content_item::METHOD_DETERMINISTIC.into(),
            classification_version: "mail-rules-v1".into(),
            // The state this endpoint has to repair: a row stored before the
            // intake gate existed, holding its subject verbatim.
            data_class: "c1".into(),
            data_class_rationale: "Mail metadata is Mine by default.".into(),
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

    /// The receipt's `redacted` count is not decoration: a class raised to c3
    /// and the narrowing that class demands happen in the same pass, so no
    /// stored row is left labelled Redacted while still holding the code.
    ///
    /// The one-time-code rule is used rather than the named-person rule
    /// deliberately — it reads no registry, so the outcome is a property of the
    /// classifier and not of the machine's contact list.
    #[test]
    fn a_refresh_that_raises_a_class_redacts_in_the_same_pass() {
        let store = test_store("data_class_refresh_redacts");
        let item = stored_row(
            "thread:code",
            "Your verification code is 448215",
            "Use 448215 to sign in",
        );
        store.upsert_triage(&item).expect("the fixture row stores");

        let (reclassified, narrowed) =
            refresh_one_triage_class(&store, &item).expect("the pass completes");
        assert!(reclassified, "the code rule raises this row");
        assert!(narrowed, "and the same pass narrows it");

        let row = store
            .get_triage("thread:code")
            .expect("readable")
            .expect("still there");
        assert_eq!(row.data_class, "c3");
        let subject = row.subject.clone().unwrap();
        let snippet = row.snippet.clone().unwrap();
        assert!(!subject.contains("448215"), "the code survived: {subject}");
        assert!(!snippet.contains("448215"), "the code survived: {snippet}");
        assert!(subject.contains("[number]"));
    }

    /// Running it twice reports zero narrowed rows the second time, which is
    /// how an operator knows the first pass finished rather than half-ran.
    #[test]
    fn a_second_refresh_pass_has_nothing_left_to_narrow() {
        let store = test_store("data_class_refresh_idempotent");
        let item = stored_row(
            "thread:code",
            "Your verification code is 448215",
            "Use 448215 to sign in",
        );
        store.upsert_triage(&item).expect("the fixture row stores");
        refresh_one_triage_class(&store, &item).expect("the first pass completes");

        let stored = store
            .get_triage("thread:code")
            .expect("readable")
            .expect("still there");
        let (_, narrowed) =
            refresh_one_triage_class(&store, &stored).expect("the second pass completes");
        assert!(!narrowed, "a second pass must find nothing to remove");
    }

    /// Redaction stays scoped by class. A c1 row keeps its readable subject, or
    /// the review list stops being reviewable — which is its own safety failure.
    #[test]
    fn a_refresh_leaves_an_ordinary_row_readable() {
        let store = test_store("data_class_refresh_verbatim");
        let item = stored_row("thread:lunch", "Lunch on Tuesday?", "Half twelve as usual");
        store.upsert_triage(&item).expect("the fixture row stores");

        let (_, narrowed) = refresh_one_triage_class(&store, &item).expect("the pass completes");
        assert!(!narrowed);
        let row = store
            .get_triage("thread:lunch")
            .expect("readable")
            .expect("still there");
        assert_eq!(row.subject.as_deref(), Some("Lunch on Tuesday?"));
    }
}
