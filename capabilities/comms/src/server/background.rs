use super::*;

/// Owns every periodic server task. Dropping the server lifecycle aborts the
/// tasks instead of leaving detached work running beyond its owner.
pub(super) struct BackgroundServices {
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl BackgroundServices {
    pub(super) fn start(config: &Config) -> Self {
        let mut tasks = Vec::new();
        tasks.extend(spawn_enrichment_drain(config.enrichment_drain_minutes));
        tasks.extend(spawn_digest_drain(config.digest_drain_minutes));
        tasks.push(spawn_trash_cleanup());
        tasks.extend(spawn_gmail_maintenance(config.gmail_maintenance_minutes));
        tasks.extend(spawn_inbox_sweep(
            config.inbox_sweep_minutes,
            config.inbox_sweep_max_threads,
            config.inbox_sweep_quiet_hours,
        ));
        Self { tasks }
    }
}

impl Drop for BackgroundServices {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

/// Start the bounded enrichment drain. The persistence ledger owns retry and
/// backoff state, so this loop only owns scheduling and task isolation.
fn spawn_enrichment_drain(every_minutes: u64) -> Option<tokio::task::JoinHandle<()>> {
    if every_minutes == 0 {
        eprintln!("enrichment drain disabled (enrichment_drain_minutes = 0)");
        return None;
    }

    eprintln!("enrichment drain: every {every_minutes} min");
    Some(tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(every_minutes * 60));
        loop {
            ticker.tick().await;
            // The join result is inspected, not discarded: a panic inside the
            // blocking half would otherwise take the drain down without a word,
            // which is the same silence this issue exists to remove.
            let joined = tokio::task::spawn_blocking(|| {
                let cfg = Config::load();
                let store = match Store::open(&cfg.database_url) {
                    Ok(store) => store,
                    Err(error) => {
                        eprintln!("enrichment drain: store unavailable: {error}");
                        return;
                    }
                };
                let summary_producer_revision = media::summary_producer_revision(&cfg);
                let before = match store
                    .feed_enrichment_counts(summary_producer_revision.as_deref())
                {
                    Ok(counts) => counts,
                    Err(error) => {
                        eprintln!("enrichment drain: backlog query failed: {error}");
                        return;
                    }
                };

                match media::summarize_pending(&store, &cfg) {
                    Ok(n) if n > 0 => eprintln!("enrichment drain: summarized {n} item(s)"),
                    // A backlog that did not move is the case this whole issue
                    // is about. Staying quiet here would rebuild the silence
                    // the ledger was supposed to end.
                    Ok(_) if before.pending_summaries > 0 => eprintln!(
                        "enrichment drain: {} pending, {} failed, none summarized — the 'summarization' inference role is unreachable or unconfigured",
                        before.pending_summaries, before.failed_summaries
                    ),
                    Ok(_) => {}
                    Err(error) => eprintln!("enrichment drain: {error}"),
                }
            })
            .await;

            if let Err(error) = joined {
                eprintln!("enrichment drain: pass did not finish: {error}");
            }
        }
    }))
}

/// Retry feed digests that failed retryably, on an interval.
///
/// `Outcome::EmptyResponse` and its siblings have always been marked retryable
/// and the ledger has always counted attempts, but nothing ever performed the
/// retry: `digest::refresh_pending`'s only caller was an HTTP endpoint no client
/// invoked. A digest lost to a transient failure stayed lost. Two rows sat at
/// `empty_response`, attempt 1 of 3, after oMLX aborted them under memory
/// pressure and the abort arrived shaped like a successful empty answer (#95).
///
/// **Feed only, deliberately.** `refresh_pending` for `mail` reads message
/// bodies, and a background job that quietly pulls every body out of a mailbox
/// is not something a machine should start doing on its own — the same reason
/// that pass is bounded and explicit rather than timer-driven. Mail digests stay
/// a press.
///
/// Bounded by the ledger rather than by anything here: `items_needing_digest`
/// skips rows at the attempt cap or inside their backoff window, so a
/// permanently broken item stops costing model calls and says why.
fn spawn_digest_drain(every_minutes: u64) -> Option<tokio::task::JoinHandle<()>> {
    if every_minutes == 0 {
        eprintln!("digest drain disabled (digest_drain_minutes = 0)");
        return None;
    }

    eprintln!("digest drain: every {every_minutes} min, feed only");
    Some(tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(every_minutes * 60));
        loop {
            ticker.tick().await;
            // Inspected rather than discarded, matching the enrichment drain: a
            // panic in the blocking half would otherwise take the drain down
            // silently, rebuilding the exact silence this exists to end.
            let joined = tokio::task::spawn_blocking(|| {
                let cfg = Config::load();
                let store = match Store::open(&cfg.database_url) {
                    Ok(store) => store,
                    Err(error) => {
                        eprintln!("digest drain: store unavailable: {error}");
                        return;
                    }
                };
                match digest::refresh_pending(&store, &cfg, "feed", 25) {
                    Ok(n) if n > 0 => eprintln!("digest drain: wrote {n} digest row(s)"),
                    Ok(_) => {}
                    Err(error) => eprintln!("digest drain: {error}"),
                }
            })
            .await;

            if let Err(error) = joined {
                eprintln!("digest drain: pass did not finish: {error}");
            }
        }
    }))
}

/// Expired Trash rows contain cached Gmail metadata and reviewed derivatives,
/// so cleanup runs independently of inbox sweeps. Gmail's own retention is not
/// modified here.
fn spawn_trash_cleanup() -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(60 * 60));
        loop {
            ticker.tick().await;
            let joined = tokio::task::spawn_blocking(|| {
                let cfg = Config::load();
                let store = Store::open(&cfg.database_url)
                    .map_err(|error| format!("store unavailable: {error}"))?;
                store
                    .purge_expired_trashed()
                    .map_err(|error| error.to_string())
            })
            .await;
            match joined {
                Ok(Ok(count)) if count > 0 => {
                    eprintln!("mail trash cleanup: purged {count} expired item(s)")
                }
                Ok(Ok(_)) => {}
                Ok(Err(error)) => eprintln!("mail trash cleanup: {error}"),
                Err(error) => eprintln!("mail trash cleanup: pass did not finish: {error}"),
            }
        }
    })
}

/// The stored name this schedule keeps its run state under. Matches the
/// existing `source_state` convention rather than inventing a second one.
pub(super) const INBOX_SWEEP_SOURCE: &str = "gmail-inbox";

/// Unattended inbox collection. Off unless the overlay turns it on, bounded to
/// the newest N threads, and silent during quiet hours.
///
/// No persisted cursor, on purpose. A cursor that advances each pass walks
/// backwards through the entire mailbox over days, which is precisely the
/// unbounded rescan this is supposed to avoid; re-reading the newest page
/// instead is idempotent because proposals upsert on Gmail thread id and
/// preserve human decisions. Paging deeper stays a manual, cursor-carrying
/// call from the board.
fn spawn_inbox_sweep(
    every_minutes: u64,
    max_threads: usize,
    quiet: Option<(u32, u32)>,
) -> Option<tokio::task::JoinHandle<()>> {
    if every_minutes == 0 {
        eprintln!("Inbox sweep schedule disabled (inbox_sweep_minutes = 0)");
        return None;
    }
    match quiet {
        Some((start, end)) => eprintln!(
            "Inbox sweep: every {every_minutes} min, newest {max_threads}, quiet {start:02}:00-{end:02}:00"
        ),
        None => eprintln!("Inbox sweep: every {every_minutes} min, newest {max_threads}"),
    }
    Some(tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(every_minutes * 60));
        loop {
            ticker.tick().await;
            let joined = tokio::task::spawn_blocking(move || -> Result<Option<String>, String> {
                let cfg = Config::load();
                if !cfg.google_env_path.is_file() {
                    return Ok(None);
                }
                let store = Store::open(&cfg.database_url).map_err(|error| error.to_string())?;

                if let Some((start, end)) = quiet {
                    if store
                        .within_quiet_hours(start, end)
                        .map_err(|error| error.to_string())?
                    {
                        return Ok(None);
                    }
                }

                // Backoff is expressed as skipped ticks rather than a sleep, so
                // a recovered connector is picked up on the next ordinary tick
                // instead of after whatever long sleep was already running.
                let state = store
                    .get_source_state(INBOX_SWEEP_SOURCE)
                    .map_err(|error| error.to_string())?;
                let failures = state.map(|s| s.consecutive_failures).unwrap_or(0);
                if failures > 0 {
                    let skip_ticks = 1i64 << failures.min(5);
                    let elapsed_ticks = TICKS_SINCE_START.fetch_add(1, Ordering::Relaxed) as i64;
                    if elapsed_ticks % skip_ticks != 0 {
                        return Ok(None);
                    }
                }

                match run_inbox_sweep(&cfg, max_threads, None) {
                    Ok(outcome) => {
                        store
                            .record_sweep_success(
                                INBOX_SWEEP_SOURCE,
                                outcome.fetched as i64,
                                outcome.new_count as i64,
                            )
                            .map_err(|error| error.to_string())?;
                        Ok(Some(format!(
                            "{} considered, {} new, {} redacted, {} skipped",
                            outcome.fetched, outcome.new_count, outcome.redacted, outcome.skipped
                        )))
                    }
                    Err(error) => {
                        let class = sweep_error_class(&error);
                        let streak = store
                            .record_sweep_failure(INBOX_SWEEP_SOURCE, class)
                            .map_err(|error| error.to_string())?;
                        Err(format!("{class} error, {streak} in a row"))
                    }
                }
            })
            .await;
            match joined {
                Ok(Ok(Some(summary))) => eprintln!("Inbox sweep: {summary}"),
                Ok(Ok(None)) => {}
                Ok(Err(error)) => eprintln!("Inbox sweep: {error}"),
                Err(error) => eprintln!("Inbox sweep: pass did not finish: {error}"),
            }
        }
    }))
}

/// Ticks since boot, so the backoff above can skip them without sleeping.
pub(super) static TICKS_SINCE_START: AtomicU64 = AtomicU64::new(0);

fn spawn_gmail_maintenance(every_minutes: u64) -> Option<tokio::task::JoinHandle<()>> {
    if every_minutes == 0 {
        eprintln!("Gmail maintenance disabled (gmail_maintenance_minutes = 0)");
        return None;
    }
    eprintln!("Gmail maintenance: every {every_minutes} min");
    Some(tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(every_minutes * 60));
        loop {
            ticker.tick().await;
            let joined = tokio::task::spawn_blocking(|| {
                let cfg = Config::load();
                if !cfg.google_env_path.is_file() {
                    return Ok(None);
                }
                run_gmail_maintenance(&cfg, 200).map(Some)
            })
            .await;
            match joined {
                Ok(Ok(Some(counts)))
                    if counts.recovered > 0 || counts.changed > 0 || counts.retry_failures > 0 =>
                {
                    eprintln!(
                        "Gmail maintenance: {} recovered, {} reconciled changes, {} retry failures",
                        counts.recovered, counts.changed, counts.retry_failures
                    )
                }
                Ok(Ok(_)) => {}
                Ok(Err(error)) => eprintln!("Gmail maintenance: {error}"),
                Err(error) => eprintln!("Gmail maintenance: pass did not finish: {error}"),
            }
        }
    }))
}
