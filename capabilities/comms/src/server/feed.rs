use super::*;

#[derive(Debug, Deserialize)]
pub(super) struct StatusBody {
    pub(super) status: String,
}

pub(super) async fn feed_status_handler(
    Path(id): Path<String>,
    Json(body): Json<StatusBody>,
) -> HttpResponse {
    let result = tokio::task::spawn_blocking(move || -> Result<bool, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        store
            .set_feed_status(&id, &body.status)
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
        let profiles = relevance::load_profiles(&cfg.relevance);
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
        items.retain(|item| {
            let item_revision = evaluation::item_revision(item);
            let stored = store.feed_evaluation(&item.id).ok().flatten();
            !evaluation::is_current(stored.as_ref(), &item_revision, &context_revision)
        });
        if items.is_empty() {
            return;
        }
        let scored = relevance::score_items(
            &items,
            &profiles,
            embedding_role.as_ref(),
            reranking_role.as_ref(),
        );
        for (item, result) in items.iter().zip(scored) {
            if let Err(error) = store.replace_feed_relevance(&item.id, &result.matches) {
                eprintln!("ingest: relevance failed for {}: {error}", item.id);
                continue;
            }
            let evaluated = evaluation::evaluate(
                item,
                result.matches.first(),
                &context_revision,
                &travel_context.contexts,
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
    ids: Option<Vec<String>>,
    force: Option<bool>,
}

pub(super) async fn relevance_refresh_handler(Json(body): Json<RefreshBody>) -> HttpResponse {
    let days = body.days.unwrap_or(90);
    if !(1..=365).contains(&days) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "days must be between 1 and 365" })),
        );
    }
    let result = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        let requested = body.ids.map(|ids| ids.into_iter().collect::<HashSet<_>>());
        let mut items = store
            .feed_for_relevance(days, 200)
            .map_err(|error| error.to_string())?;
        if let Some(ids) = requested {
            items.retain(|item| ids.contains(&item.id));
        }
        let profiles = relevance::load_profiles(&cfg.relevance);
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
        let considered = items.len();
        if !body.force.unwrap_or(false) {
            items.retain(|item| {
                let item_revision = evaluation::item_revision(item);
                let stored = store.feed_evaluation(&item.id).ok().flatten();
                !evaluation::is_current(stored.as_ref(), &item_revision, &context_revision)
            });
        }
        let scored = relevance::score_items(
            &items,
            &profiles,
            embedding_role.as_ref(),
            reranking_role.as_ref(),
        );
        for (source, item) in items.iter().zip(&scored) {
            store
                .replace_feed_relevance(&item.feed_id, &item.matches)
                .map_err(|error| error.to_string())?;
            let evaluated = evaluation::evaluate(
                source,
                item.matches.first(),
                &context_revision,
                &travel_context.contexts,
            );
            store
                .replace_feed_evaluation(&evaluated)
                .map_err(|error| error.to_string())?;
        }
        let mode = scored
            .iter()
            .flat_map(|item| item.matches.first())
            .map(|relevance| relevance.mode.as_str())
            .next();
        Ok(json!({
            "scored": scored.len(),
            "evaluated": scored.len(),
            "considered": considered,
            "skipped_current": considered.saturating_sub(scored.len()),
            "profile_count": profiles.len(),
            "mode": mode,
            "bounded_to": 200,
            "evaluator_revision": evaluation::EVALUATOR_REVISION,
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
        let profiles = relevance::load_profiles(&cfg.relevance);
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
