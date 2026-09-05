use super::*;

#[derive(Debug, Deserialize)]
pub(super) struct StatusBody {
    pub(super) status: String,
    /// Where the press happened. Absent means `api`, so every caller that
    /// predates the ledger keeps working and still leaves a row.
    pub(super) surface: Option<String>,
}

/// The only writer of the decisive verbs.
///
/// `set_feed_status` commits the UPDATE and one `kept`/`dismissed`/`unkept`
/// ledger row together. `POST /feed/:id/interactions` refuses those three and
/// takes `opened`/`reopened` only, so one decision is one row.
pub(super) async fn feed_status_handler(
    Path(id): Path<String>,
    Json(body): Json<StatusBody>,
) -> HttpResponse {
    let result = tokio::task::spawn_blocking(move || -> Result<bool, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        store
            .set_feed_status(&id, &body.status, body.surface.as_deref().unwrap_or("api"))
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
pub(super) struct InteractionBody {
    event: String,
    surface: Option<String>,
}

/// Record that the operator looked at an item.
///
/// Reads only. The decisive verbs are refused here by name, with the route
/// that owns them in the message: a client that becomes a second writer of a
/// keep doubles every count the learned factor is gated on, and the gate's own
/// sample report is the first thing that would lie.
pub(super) async fn feed_interactions_handler(
    Path(id): Path<String>,
    Json(body): Json<InteractionBody>,
) -> HttpResponse {
    let result = tokio::task::spawn_blocking(move || -> Result<(bool, String), String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        let event = body.event;
        let recorded = store
            .record_interaction(&id, &event, body.surface.as_deref().unwrap_or("api"))
            .map_err(|error| error.to_string())?;
        Ok((recorded, event))
    })
    .await;

    match result {
        Ok(Ok((true, event))) => (
            StatusCode::OK,
            Json(json!({ "ok": true, "recorded": event })),
        ),
        Ok(Ok((false, _))) => error_response(StatusCode::NOT_FOUND, "not found"),
        Ok(Err(error)) => error_response(StatusCode::BAD_REQUEST, error),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        ),
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct FeedDataClassBody {
    data_class: String,
    /// Required when the change lowers the class, ignored when it raises it.
    /// Which of the two this is depends on what is stored, so the store decides
    /// and this handler stays out of it.
    rationale: Option<String>,
}

/// The only path by which a feed item's class goes down, and the reason the
/// endpoint exists at all: everything automatic may escalate, so escalation
/// needs no door — de-escalation needs one that can say no.
pub(super) async fn feed_data_class_handler(
    Path(id): Path<String>,
    Json(body): Json<FeedDataClassBody>,
) -> HttpResponse {
    // Before the connection, not after. A class outside the vocabulary is
    // decidable from the request alone, and answering it here keeps a
    // malformed request from opening a database handle at all — which is also
    // what lets this route be tested without a live store behind it.
    if !content_item::valid(&body.data_class) {
        return error_response(
            StatusCode::BAD_REQUEST,
            format!(
                "data class must be one of: {}",
                content_item::DATA_CLASSES.join(", ")
            ),
        );
    }
    let result = tokio::task::spawn_blocking(move || -> Result<bool, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        store
            .set_feed_data_class(&id, &body.data_class, body.rationale.as_deref())
            .map_err(|error| error.to_string())
    })
    .await;

    match result {
        Ok(Ok(true)) => (StatusCode::OK, Json(json!({ "ok": true }))),
        Ok(Ok(false)) => error_response(StatusCode::NOT_FOUND, "not found"),
        // A refused de-escalation and an unknown class are both the caller
        // asking for something that cannot be granted. Both carry the store's
        // own sentence, so the operator reads why rather than just "400".
        Ok(Err(error)) => error_response(StatusCode::BAD_REQUEST, error),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        ),
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct IngestBody {
    url: String,
    content: Option<String>,
    title: Option<String>,
    author: Option<String>,
    /// Who is handing the content over — `axon-clip`, a CLI, a future share
    /// sheet. Recorded as the item's capture provenance; absent means the
    /// server fetched the page itself (#81).
    client: Option<String>,
}

pub(super) fn enrich_many_in_background(ids: Vec<String>) {
    tokio::task::spawn_blocking(move || {
        let cfg = Config::load();
        let store = match Store::open(&cfg.database_path) {
            Ok(store) => store,
            Err(error) => {
                eprintln!("ingest: enrichment skipped, store unavailable: {error}");
                return;
            }
        };
        for id in &ids {
            if let Err(error) = media::summarize_item(&store, &cfg, id) {
                eprintln!("ingest: summarize failed for {id}: {error}");
            }
        }
        let mut items = ids
            .iter()
            .filter_map(|id| store.get_feed(id).ok().flatten())
            .collect::<Vec<_>>();
        // Reported rather than failed open. A declared lens directory that has
        // moved used to leave this path scoring against nothing, which stores
        // an `unscored` row over a perfectly good item.
        let profiles = match relevance::load_profiles(&cfg.relevance) {
            Ok(profiles) => profiles,
            Err(error) => {
                eprintln!("ingest: relevance skipped, TELOS profiles unreadable: {error}");
                return;
            }
        };
        let embedding_role = cfg.embedding_role();
        let embedding_producer = embedding_role.as_ref().map(|role| role.cache_key());
        let reranking_role = cfg.reranking_role();
        let reranking_producer = reranking_role.as_ref().map(|role| role.cache_key());
        let travel_context = travel::load(&store, &cfg.travel_context);
        let context_revision = evaluation::context_revision(
            &profiles,
            embedding_producer.as_deref(),
            reranking_producer.as_deref(),
            &travel_context.revision,
        );
        let semantic_available = relevance::embedding_backend_reachable(embedding_role.as_ref());
        items.retain(|item| {
            let item_revision = evaluation::item_revision(item);
            let stored = store.feed_evaluation(&item.id).ok().flatten();
            !evaluation::is_current(
                stored.as_ref(),
                &item_revision,
                &context_revision,
                semantic_available,
            )
        });
        if items.is_empty() {
            return;
        }
        let outcome = relevance::score_items(
            &items,
            &profiles,
            embedding_role.as_ref(),
            reranking_role.as_ref(),
        );
        for (item, result) in items.iter().zip(outcome.items) {
            if let Err(error) = store.replace_feed_relevance(&item.id, &result.matches) {
                eprintln!("ingest: relevance failed for {}: {error}", item.id);
                continue;
            }
            let evaluated = evaluation::evaluate(
                item,
                result.matches.first(),
                &context_revision,
                &travel_context.contexts,
                result.refused_class,
            );
            if let Err(error) = store.replace_feed_evaluation(&evaluated) {
                eprintln!("ingest: evaluation failed for {}: {error}", item.id);
            }
        }
    });
}

pub(super) fn enrich_in_background(id: String) {
    enrich_many_in_background(vec![id]);
}

/// Store first, then summarize and score behind the response.
pub(super) async fn ingest_handler(Json(body): Json<IngestBody>) -> HttpResponse {
    let url = body.url.trim().to_string();
    if url.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "url is required" })),
        );
    }
    let content = body.content;
    let title = body.title;
    let author = body.author;
    let client = body.client;

    let stored = tokio::task::spawn_blocking(move || -> Result<FeedFullItem, String> {
        let cfg = Config::load();
        let item = media::fetch_with_content(
            &url,
            content.as_deref(),
            title.as_deref(),
            author.as_deref(),
            client.as_deref(),
        )
        .map_err(|error| error.to_string())?;
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        store
            .upsert_feed(&item)
            .map_err(|error| error.to_string())?;
        let item = store
            .get_feed(&item.id)
            .map_err(|error| error.to_string())?
            .unwrap_or(item);
        full_item(&store, item)
    })
    .await;

    match stored {
        Ok(Ok(item)) => {
            enrich_in_background(item.id.clone());
            (StatusCode::CREATED, Json(json!(item)))
        }
        Ok(Err(error)) => error_response(StatusCode::BAD_REQUEST, error),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        ),
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct RefreshBody {
    days: Option<i32>,
    limit: Option<usize>,
    offset: Option<usize>,
    ids: Option<Vec<String>>,
    force: Option<bool>,
}

/// What the last completed sweep recorded, parsed back out of the receipt's
/// cursor. Three fields in one TEXT column, because `migrations.rs` states why
/// a new column is the expensive option.
///
/// `completed` is the relevance revision of the last sweep that ran from
/// offset 0 through `has_more = false`. `progress` is how far the sweep now
/// running has covered, so a page that arrives out of order cannot mark the
/// corpus done — a partial page advancing the revision would leave the rows it
/// never saw looking current forever.
struct PassCursor {
    mode: String,
    completed: String,
    progress_revision: String,
    progress_end: usize,
}

impl PassCursor {
    fn parse(cursor: Option<&str>) -> Self {
        let raw = cursor.unwrap_or_default();
        let mut parts = raw.split('|');
        let mode = parts.next().unwrap_or_default().to_string();
        let completed = parts.next().unwrap_or_default().to_string();
        let progress = parts.next().unwrap_or_default();
        let (progress_revision, progress_end) = progress
            .rsplit_once(':')
            .map(|(revision, end)| (revision.to_string(), end.parse().unwrap_or(0)))
            .unwrap_or_default();
        Self {
            mode,
            completed,
            progress_revision,
            progress_end,
        }
    }

    fn render(&self) -> String {
        format!(
            "{}|{}|{}:{}",
            self.mode, self.completed, self.progress_revision, self.progress_end
        )
    }
}

/// Re-score and re-evaluate a bounded window of the feed.
///
/// The currency check is split in two, which is the whole point of this
/// rewrite. The old handler retained items by `is_current` and handed the
/// retained set straight to `score_items`, so every term in `context_revision`
/// — including the travel snapshot — was an embedding trigger. Now:
///
/// * an item is RE-SCORED (embedded) when the sweep's relevance revision has
///   moved, when it has no stored matches, when its own content changed, or
///   when its stored matches read `lexical` while an embedding role answers;
/// * every other stale item is RE-EVALUATED from `feed_relevance_map`, one
///   batched read and no model call;
/// * an item the class ladder refuses is scored by neither path, has its
///   stored matches cleared, and gets a refusal evaluation.
pub(super) async fn relevance_refresh_handler(Json(body): Json<RefreshBody>) -> HttpResponse {
    let days = body.days.unwrap_or(90);
    // Widened from 365. A backfill has to be able to reach the whole corpus --
    // `comms relevance backfill` and the nightly sweep both ask for ten years --
    // and a window that cannot cover the store makes the oldest rows permanently
    // unreachable, which is the same defect as the ids-after-LIMIT bug.
    if !(1..=3650).contains(&days) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "days must be between 1 and 3650" })),
        );
    }
    let limit = body.limit.unwrap_or(200);
    if !(1..=500).contains(&limit) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "limit must be between 1 and 500" })),
        );
    }
    let offset = body.offset.unwrap_or(0);
    let result = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        // Fallible now: a declared lens directory that has moved is reported
        // rather than scored against, which is what produced 48 `unscored`
        // evaluations over good rows.
        let profiles = relevance::load_profiles(&cfg.relevance)?;
        let requested = body.ids.unwrap_or_default();
        // The ids filter is applied by the SELECT, not after the LIMIT. The old
        // order silently dropped any named item outside the newest page.
        let items = if requested.is_empty() {
            store
                .feed_for_relevance(days, limit, offset)
                .map_err(|error| error.to_string())?
        } else {
            store
                .feed_items_by_ids(&requested)
                .map_err(|error| error.to_string())?
        };
        let found = items
            .iter()
            .map(|item| item.id.clone())
            .collect::<HashSet<_>>();
        let missing_ids = requested
            .iter()
            .filter(|id| !found.contains(*id))
            .cloned()
            .collect::<Vec<_>>();
        let has_more = requested.is_empty() && items.len() == limit;

        let embedding_role = cfg.embedding_role();
        let embedding_producer = embedding_role.as_ref().map(|role| role.cache_key());
        let reranking_role = cfg.reranking_role();
        let reranking_producer = reranking_role.as_ref().map(|role| role.cache_key());
        let travel_context = travel::load(&store, &cfg.travel_context);
        let context_revision = evaluation::context_revision(
            &profiles,
            embedding_producer.as_deref(),
            reranking_producer.as_deref(),
            &travel_context.revision,
        );
        let relevance_revision = evaluation::relevance_revision(
            &profiles,
            embedding_producer.as_deref(),
            reranking_producer.as_deref(),
        );
        let semantic_available = relevance::embedding_backend_reachable(embedding_role.as_ref());
        let receipt = store.relevance_pass().map_err(|error| error.to_string())?;
        let mut cursor =
            PassCursor::parse(receipt.as_ref().and_then(|state| state.cursor.as_deref()));
        let vector_space_moved = cursor.completed != relevance_revision;

        let considered = items.len();
        let ids = items.iter().map(|item| item.id.clone()).collect::<Vec<_>>();
        let stored_matches = store
            .feed_relevance_map(&ids)
            .map_err(|error| error.to_string())?;
        let force = body.force.unwrap_or(false);

        let mut to_rescore = Vec::new();
        let mut to_reevaluate = Vec::new();
        let mut refused = Vec::new();
        let mut skipped_current = 0usize;
        for item in items {
            if !content_item::local_prompt_allowed(&item.data_class) {
                refused.push(item);
                continue;
            }
            let stored_evaluation = store.feed_evaluation(&item.id).ok().flatten();
            let item_revision = evaluation::item_revision(&item);
            let matches = stored_matches.get(&item.id);
            let matches_are_lexical = matches
                .and_then(|rows| rows.first())
                .is_some_and(|matched| matched.mode == "lexical");
            let needs_embedding = force
                || vector_space_moved
                || matches.is_none_or(|rows| rows.is_empty())
                || stored_evaluation
                    .as_ref()
                    .is_none_or(|stored| stored.item_revision != item_revision)
                || (matches_are_lexical && semantic_available);
            if needs_embedding {
                to_rescore.push(item);
            } else if !evaluation::is_current(
                stored_evaluation.as_ref(),
                &item_revision,
                &context_revision,
                semantic_available,
            ) {
                to_reevaluate.push(item);
            } else {
                skipped_current += 1;
            }
        }

        let refused_class = refused.len();
        let mut refused_lower_tier = 0usize;
        let mut written = 0usize;
        for item in &refused {
            // The rows already derived from a refused item are removed, not
            // left to be read as a score nobody may have.
            store
                .replace_feed_relevance(&item.id, &[])
                .map_err(|error| error.to_string())?;
            let evaluated = evaluation::evaluate(
                item,
                None,
                &context_revision,
                &travel_context.contexts,
                true,
            );
            if store
                .replace_feed_evaluation(&evaluated)
                .map_err(|error| error.to_string())?
            {
                written += 1;
            } else {
                refused_lower_tier += 1;
            }
        }

        let outcome = relevance::score_items(
            &to_rescore,
            &profiles,
            embedding_role.as_ref(),
            reranking_role.as_ref(),
        );
        for (item, scored) in to_rescore.iter().zip(&outcome.items) {
            if !store
                .replace_feed_relevance(&item.id, &scored.matches)
                .map_err(|error| error.to_string())?
            {
                refused_lower_tier += 1;
                continue;
            }
            let evaluated = evaluation::evaluate(
                item,
                scored.matches.first(),
                &context_revision,
                &travel_context.contexts,
                scored.refused_class,
            );
            if store
                .replace_feed_evaluation(&evaluated)
                .map_err(|error| error.to_string())?
            {
                written += 1;
            } else {
                refused_lower_tier += 1;
            }
        }

        // The half a refit pays for: an evaluation rewritten from matches that
        // are already stored, with no model call at all.
        let reused_relevance = to_reevaluate.len();
        for item in &to_reevaluate {
            let matches = stored_matches
                .get(&item.id)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let evaluated = evaluation::evaluate(
                item,
                matches.first(),
                &context_revision,
                &travel_context.contexts,
                false,
            );
            if store
                .replace_feed_evaluation(&evaluated)
                .map_err(|error| error.to_string())?
            {
                written += 1;
            } else {
                refused_lower_tier += 1;
            }
        }

        let mode = if to_rescore.is_empty() {
            cursor.mode.clone()
        } else {
            outcome.mode.to_string()
        };
        if requested.is_empty() {
            // The chain only extends from a page that continues it, and only a
            // chain that reached the end marks the corpus done.
            let extends = offset == 0
                || (cursor.progress_revision == relevance_revision
                    && cursor.progress_end == offset);
            if extends {
                cursor.progress_revision = relevance_revision.clone();
                cursor.progress_end = offset + considered;
                if !has_more {
                    cursor.completed = relevance_revision.clone();
                    cursor.progress_revision = String::new();
                    cursor.progress_end = 0;
                }
            }
        }
        cursor.mode = mode.clone();
        store
            .record_relevance_pass(
                &cursor.render(),
                considered as i64,
                written as i64,
                outcome.error_class,
            )
            .map_err(|error| error.to_string())?;

        Ok(json!({
            "scored": outcome.items.len(),
            "evaluated": written,
            "considered": considered,
            "skipped_current": skipped_current,
            "rescored": to_rescore.len(),
            "reused_relevance": reused_relevance,
            "refused_class": refused_class,
            "refused_lower_tier": refused_lower_tier,
            "missing_ids": missing_ids,
            "profile_count": profiles.len(),
            "mode": mode,
            "offset": offset,
            "limit": limit,
            "has_more": has_more,
            "bounded_to": limit,
            "relevance_revision": relevance_revision,
            "evaluator_revision": evaluation::EVALUATOR_REVISION,
            "embedding": {
                "mode": outcome.mode,
                "error_class": outcome.error_class,
                "chunks": outcome.chunks,
                "chunks_failed": outcome.chunks_failed,
            },
            "travel_context": {
                "upcoming_count": travel_context.contexts.len(),
                "reachable": travel_context.reachable,
                "from_cache": travel_context.from_cache,
                "refreshed_at": travel_context.refreshed_at,
            },
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

pub(super) async fn evaluation_status_handler() -> HttpResponse {
    let result = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        // A machine with no declared lens is the ordinary case and answers 200
        // with `profile_count: 0`. A machine whose declared lens directory has
        // moved is a fault, and now says so instead of reporting zero lenses as
        // if that were a configuration.
        let profiles = relevance::load_profiles(&cfg.relevance)?;
        let embedding_role = cfg.embedding_role();
        let embedding_producer = embedding_role.as_ref().map(|role| role.cache_key());
        let reranking_role = cfg.reranking_role();
        let reranking_producer = reranking_role.as_ref().map(|role| role.cache_key());
        let summarization_role = cfg.summarization_role();
        let summary_producer_revision = media::summary_producer_revision(&cfg);
        let travel_context = travel::cached(&store);
        let travel_revision = travel_context
            .as_ref()
            .map(|context| context.revision.as_str())
            .unwrap_or_default();
        let summary = store
            .evaluation_summary()
            .map_err(|error| error.to_string())?;
        let enrichment = store
            .feed_enrichment_counts(summary_producer_revision.as_deref())
            .map_err(|error| error.to_string())?;
        let content_status = store
            .feed_content_status_counts()
            .map_err(|error| error.to_string())?;
        let capacity_state = store
            .get_source_state(comms::capacity::LOCAL_INFERENCE_SOURCE)
            .map_err(|error| error.to_string())?;
        let unattended_role = cfg.light_summarization_role();
        let summarizer_reachable = media::summarizer_reachable(&cfg);
        // Probed rather than assumed: an operator deciding whether to press
        // Regenerate on an over-window item is asking exactly this, and a
        // stopped oMLX is the ordinary state of this machine now.
        let strong_reachable = summarization_role
            .as_ref()
            .is_some_and(|role| role.model_reachable());
        let relevance_reachable =
            relevance::embedding_backend_reachable(embedding_role.as_ref());
        // The receipt of the last pass, and the line that would have made the
        // 2026-08-30 degradation visible the day it happened: 525 rows were
        // written lexical in one pass and nothing on the machine said so.
        let last_pass = store.relevance_pass().map_err(|error| error.to_string())?;
        let last_pass_mode = last_pass
            .as_ref()
            .and_then(|state| state.cursor.as_deref())
            .and_then(|cursor| cursor.split('|').next())
            .filter(|mode| !mode.is_empty())
            .map(str::to_string);
        let reranking_reachable =
            relevance::embedding_backend_reachable(reranking_role.as_ref());
        Ok(json!({
            "evaluator_revision": evaluation::EVALUATOR_REVISION,
            "context_revision": evaluation::context_revision(
                &profiles,
                embedding_producer.as_deref(),
                reranking_producer.as_deref(),
                travel_revision,
            ),
            "ledger": {
                "evaluated": summary.evaluated,
                "reranked": summary.reranked,
                "semantic": summary.semantic,
                "lexical": summary.lexical,
                "unscored": summary.unscored,
            },
            // Two rungs, named separately, because they are now used by
            // different callers and one number cannot describe both. `model`
            // and `reachable` are the *unattended* rung — the light local role
            // every drain runs on — since that is what the reader is asking
            // about when the feed has no digests. Reporting the strong role's
            // name beside the light role's reachability, which is what this
            // block did for one build, is worse than reporting neither: it
            // said the 9B model was up while its server was stopped.
            "summarizer": {
                "provider": unattended_role
                    .as_ref()
                    .map(|role| role.provider_label())
                    .unwrap_or("No unattended summarization role configured"),
                "model": unattended_role
                    .as_ref()
                    .map(|role| role.model.as_str())
                    .unwrap_or(""),
                "configured": unattended_role.is_some(),
                "reachable": summarizer_reachable,
                // The rung only a press reaches. Kept in the payload because a
                // reader looking at `skipped_over_window` rows wants to know
                // what pressing Regenerate would actually engage.
                "strong": {
                    "provider": summarization_role
                        .as_ref()
                        .map(|role| role.provider_label())
                        .unwrap_or("No summarization role configured"),
                    "model": summarization_role
                        .as_ref()
                        .map(|role| role.model.as_str())
                        .unwrap_or(""),
                    "configured": summarization_role.is_some(),
                    "reachable": strong_reachable,
                },
                // The durable half of the capacity alert. The drain says it on
                // stderr when the streak crosses the threshold; this is where
                // it can still be read an hour later by someone who was not
                // watching. Same `source_state` row, same shape the inbox
                // sweep's own streak is served in at /triage/sweep/status.
                "capacity": {
                    "alert_after": cfg.capacity_alert_after,
                    "consecutive_aborts": capacity_state
                        .as_ref()
                        .map(|state| state.consecutive_failures)
                        .unwrap_or(0),
                    "alerting": cfg.capacity_alert_after > 0
                        && capacity_state
                            .as_ref()
                            .map(|state| state.consecutive_failures)
                            .unwrap_or(0)
                            >= cfg.capacity_alert_after,
                    "last_abort_at": capacity_state
                        .as_ref()
                        .and_then(|state| state.last_failure_at.clone()),
                    "last_success_at": capacity_state
                        .as_ref()
                        .and_then(|state| state.last_success_at.clone()),
                },
            },
            "enrichment": {
                "pending_summaries": enrichment.pending_summaries,
                "failed_summaries": enrichment.failed_summaries,
                "content_status": {
                    "full": content_status.full,
                    "thin": content_status.thin,
                    "none": content_status.none,
                    "unknown": content_status.unknown,
                },
            },
            "last_pass": last_pass.as_ref().map(|state| json!({
                "mode": last_pass_mode,
                "at": state.last_run_at,
                "considered": state.considered_count,
                "written": state.new_count,
                "error_class": state.last_error,
                "consecutive_fallbacks": state.consecutive_failures,
                "completed_revision": state.cursor
                    .as_deref()
                    .and_then(|cursor| cursor.split('|').nth(1))
                    .unwrap_or_default(),
            })),
            "relevance": {
                "provider": relevance::embedding_provider_label(embedding_role.as_ref()),
                "model": embedding_role
                    .as_ref()
                    .map(|role| role.model.as_str())
                    .unwrap_or(""),
                "configured": relevance::embedding_backend_configured(embedding_role.as_ref()),
                "reachable": relevance_reachable,
                "profile_count": profiles.len(),
                "active_mode": if relevance_reachable && reranking_reachable {
                    "reranked"
                } else if relevance_reachable {
                    "semantic"
                } else {
                    "lexical"
                },
            },
            "reranker": {
                "provider": reranking_role
                    .as_ref()
                    .map(|role| role.provider_label())
                    .unwrap_or("No reranking role configured"),
                "model": reranking_role
                    .as_ref()
                    .map(|role| role.model.as_str())
                    .unwrap_or(""),
                "configured": reranking_role.is_some(),
                "reachable": reranking_reachable,
            },
            "travel_context": {
                "enabled": cfg.travel_context.enabled,
                "source": cfg.travel_context.base_url,
                "upcoming_count": travel_context.as_ref().map(|context| context.contexts.len()).unwrap_or(0),
                "reachable": travel_context.as_ref().is_some_and(|context| context.reachable),
                "from_cache": travel_context.as_ref().is_some_and(|context| context.from_cache),
                "refreshed_at": travel_context.as_ref().map(|context| context.refreshed_at.clone()).unwrap_or_default(),
                "plans": travel_context
                    .as_ref()
                    .map(|snapshot| snapshot.contexts.iter().map(|plan| json!({
                        "id": plan.id,
                        "label": plan.title,
                        "date_start": plan.date_start,
                        "date_end": plan.date_end,
                    })).collect::<Vec<_>>())
                    .unwrap_or_default(),
            }
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
