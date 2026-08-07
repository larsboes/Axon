use super::*;

#[derive(Debug, Serialize)]
pub(super) struct CloudProviderOut {
    role: String,
    name: String,
    model: String,
    provider_label: &'static str,
    location: &'static str,
    data_tier: &'static str,
    billing_mode: &'static str,
    failover_priority: u16,
    max_requests_per_day: u32,
    requests_used_today: Option<u32>,
    requests_remaining_today: Option<u32>,
    max_input_tokens: u32,
    credit_expires_on: Option<String>,
    available: bool,
    unavailable_reason: Option<&'static str>,
}

pub(super) fn cloud_provider_options(
    cfg: &Config,
    store: Option<&Store>,
    utc_date: Option<&str>,
) -> Vec<CloudProviderOut> {
    cfg.inference
        .roles_with_prefix("cloud_")
        .into_iter()
        .filter(|(_, role)| role.has_cloud_policy())
        .map(|(name, role)| {
            let request_limit = role.max_requests_per_day.unwrap();
            let calls = store.and_then(|store| store.cloud_provider_calls_today(&name).ok());
            let unavailable_reason = if !role.credential_ready() {
                Some("missing_credential")
            } else if !utc_date.is_some_and(|date| role.billing_active_on(date)) {
                Some("billing_expired_or_unknown")
            } else if calls.is_none() {
                Some("budget_unavailable")
            } else if calls.is_some_and(|used| used >= request_limit) {
                Some("daily_request_limit_reached")
            } else {
                None
            };
            CloudProviderOut {
                role: name,
                name: role.provider_name.clone().unwrap_or_default(),
                model: role.model.clone(),
                provider_label: role.provider_label(),
                location: "cloud",
                data_tier: role.cloud_data_tier.unwrap().as_str(),
                billing_mode: role.billing_mode.unwrap().as_str(),
                failover_priority: role.failover_priority(),
                max_requests_per_day: request_limit,
                requests_used_today: calls,
                requests_remaining_today: calls.map(|used| request_limit.saturating_sub(used)),
                max_input_tokens: role.max_input_tokens.unwrap(),
                credit_expires_on: role.credit_expires_on.clone(),
                available: unavailable_reason.is_none(),
                unavailable_reason,
            }
        })
        .collect()
}

pub(super) fn cloud_tier_allows(
    tier: Option<&str>,
    original_data_class: &str,
    derivative_data_class: &str,
    transformation: &str,
) -> bool {
    if original_data_class == "vault" {
        return false;
    }
    let public_derivative = original_data_class == "public"
        && derivative_data_class == "public"
        && transformation == cloud_derivative::PASSTHROUGH_VERSION;
    match tier {
        Some("public") => public_derivative,
        Some("pseudonymized_personal") => {
            public_derivative
                || (original_data_class == "personal"
                    && derivative_data_class == "personal"
                    && transformation == cloud_derivative::REDACTION_VERSION)
        }
        _ => false,
    }
}

pub(super) async fn cloud_providers_handler() -> Json<Value> {
    let providers = tokio::task::spawn_blocking(|| {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_url).ok();
        let utc_date = store.as_ref().and_then(|store| store.utc_date().ok());
        cloud_provider_options(&cfg, store.as_ref(), utc_date.as_deref())
    })
    .await
    .unwrap_or_default();
    Json(json!(providers))
}

pub(super) async fn cloud_preview_handler(
    Path((source, id)): Path<(String, String)>,
) -> HttpResponse {
    let result =
        tokio::task::spawn_blocking(move || -> Result<Option<CloudDerivativePreview>, String> {
            let cfg = Config::load();
            let store = Store::open(&cfg.database_url).map_err(|error| error.to_string())?;
            Ok(load_content_item(&store, &source, &id)?
                .map(|item| cloud_derivative::prepare(&item.cloud_input())))
        })
        .await;

    match result {
        Ok(Ok(Some(preview))) => (StatusCode::OK, Json(json!(preview))),
        Ok(Ok(None)) => error_response(StatusCode::NOT_FOUND, "not found"),
        Ok(Err(error)) if error == "source must be 'feed' or 'mail'" => {
            error_response(StatusCode::BAD_REQUEST, error)
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "cloud preview failed" })),
        ),
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct CloudApprovalBody {
    preview_hash: String,
}

pub(super) async fn cloud_approval_handler(
    Path((source, id)): Path<(String, String)>,
    Json(body): Json<CloudApprovalBody>,
) -> HttpResponse {
    let result =
        tokio::task::spawn_blocking(move || -> Result<Option<CloudDerivativeState>, String> {
            let cfg = Config::load();
            let store = Store::open(&cfg.database_url).map_err(|error| error.to_string())?;
            let Some(item) = load_content_item(&store, &source, &id)? else {
                return Ok(None);
            };
            let preview = cloud_derivative::prepare(&item.cloud_input());
            if preview.preview_hash != body.preview_hash {
                return Err(
                    "preview is stale; prepare and review the current document again".into(),
                );
            }
            if preview.original_data_class == "vault" {
                return Err("vault content cannot be staged for cloud processing".into());
            }
            let approval = CloudDerivativeApproval {
                source: preview.source,
                item_id: preview.id,
                source_revision: preview.source_revision,
                preview_hash: preview.preview_hash,
                original_data_class: preview.original_data_class,
                derivative_data_class: preview.derivative_data_class,
                transformation: preview.transformation.into(),
                document: preview.document,
                redaction_count: preview.redaction_count as i32,
            };
            store
                .stage_cloud_derivative(&approval)
                .map(Some)
                .map_err(|error| error.to_string())
        })
        .await;

    match result {
        Ok(Ok(Some(state))) => (StatusCode::OK, Json(json!(state))),
        Ok(Ok(None)) => error_response(StatusCode::NOT_FOUND, "not found"),
        Ok(Err(error)) if error.starts_with("preview is stale") => {
            (StatusCode::CONFLICT, Json(json!({ "error": error })))
        }
        Ok(Err(error)) if error.starts_with("vault content") => {
            error_response(StatusCode::BAD_REQUEST, error)
        }
        Ok(Err(error)) if error == "source must be 'feed' or 'mail'" => {
            error_response(StatusCode::BAD_REQUEST, error)
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "cloud approval staging failed" })),
        ),
    }
}
#[derive(Debug, Deserialize)]
pub(super) struct CloudQueueBody {
    preview_hash: String,
    provider_role: String,
}

pub(super) async fn cloud_queue_handler(
    Path((source, id)): Path<(String, String)>,
    Json(body): Json<CloudQueueBody>,
) -> HttpResponse {
    let result =
        tokio::task::spawn_blocking(move || -> Result<Option<CloudDerivativeState>, String> {
            let cfg = Config::load();
            let role = cfg
                .inference
                .roles_with_prefix("cloud_")
                .into_iter()
                .find_map(|(name, role)| (name == body.provider_role).then_some(role))
                .filter(|role| role.has_cloud_policy())
                .ok_or_else(|| {
                    "provider role is not a configured reviewed HTTPS cloud role".to_string()
                })?;
            if !role.credential_ready() {
                return Err("provider credential is not materialized".into());
            }

            let store = Store::open(&cfg.database_url).map_err(|error| error.to_string())?;
            let Some(item) = load_content_item(&store, &source, &id)? else {
                return Ok(None);
            };
            let preview = cloud_derivative::prepare(&item.cloud_input());
            if preview.preview_hash != body.preview_hash {
                return Err("approved derivative is stale; prepare and review it again".into());
            }
            if !cloud_tier_allows(
                role.cloud_data_tier.map(|tier| tier.as_str()),
                &preview.original_data_class,
                &preview.derivative_data_class,
                preview.transformation,
            ) {
                return Err("provider role does not allow this reviewed derivative".into());
            }
            let utc_date = store.utc_date().map_err(|error| error.to_string())?;
            if !role.billing_active_on(&utc_date) {
                return Err("provider billing policy is expired or inactive".into());
            }
            let input_upper_bound = cloud_dispatch::input_token_upper_bound(&preview.document);
            if input_upper_bound > role.max_input_tokens.unwrap_or(0) {
                return Err(
                    "provider input token ceiling is below this reviewed derivative".into(),
                );
            }
            let calls = store
                .cloud_provider_calls_today(&body.provider_role)
                .map_err(|error| error.to_string())?;
            if calls >= role.max_requests_per_day.unwrap_or(0) {
                return Err("provider daily request ceiling is reached".into());
            }
            store
                .queue_cloud_derivative(&CloudQueueRequest {
                    source: preview.source,
                    item_id: preview.id,
                    source_revision: preview.source_revision,
                    preview_hash: preview.preview_hash,
                    provider_role: body.provider_role,
                })
                .map(Some)
                .map_err(|error| error.to_string())
        })
        .await;

    match result {
        Ok(Ok(Some(state))) => (StatusCode::OK, Json(json!(state))),
        Ok(Ok(None)) => error_response(StatusCode::NOT_FOUND, "not found"),
        Ok(Err(error))
            if error.starts_with("provider role") || error.starts_with("provider credential") =>
        {
            error_response(StatusCode::BAD_REQUEST, error)
        }
        Ok(Err(error)) if error.contains("stale") || error.starts_with("approved derivative") => {
            (StatusCode::CONFLICT, Json(json!({ "error": error })))
        }
        Ok(Err(error)) if error == "source must be 'feed' or 'mail'" => {
            error_response(StatusCode::BAD_REQUEST, error)
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "cloud queue failed" })),
        ),
    }
}

pub(super) async fn cloud_run_handler(Path(job_id): Path<String>) -> HttpResponse {
    let result = tokio::task::spawn_blocking(move || -> Result<CloudDerivativeState, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_url).map_err(|error| error.to_string())?;
        let job = store
            .cloud_job_for_dispatch(&job_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| {
                "cloud job is completed, running, stale, or past its retry limit".to_string()
            })?;
        if job.task != cloud_dispatch::TASK_VERSION {
            return Err("cloud job task is unsupported".into());
        }
        let selected_role = cfg
            .inference
            .role(&job.provider_role)
            .filter(|role| role.has_cloud_policy())
            .ok_or_else(|| "provider role is no longer a reviewed HTTPS cloud role".to_string())?;
        if !cloud_tier_allows(
            selected_role.cloud_data_tier.map(|tier| tier.as_str()),
            &job.original_data_class,
            &job.derivative_data_class,
            &job.transformation,
        ) {
            return Err("provider role no longer allows the staged derivative".into());
        }
        let utc_date = store.utc_date().map_err(|error| error.to_string())?;
        let input_upper_bound = cloud_dispatch::input_token_upper_bound(&job.document);
        let mut requested = false;
        let mut outcomes = Vec::new();

        for (candidate_name, role) in cfg.inference.cloud_failover_roles(&job.provider_role) {
            if !cloud_tier_allows(
                role.cloud_data_tier.map(|tier| tier.as_str()),
                &job.original_data_class,
                &job.derivative_data_class,
                &job.transformation,
            ) {
                continue;
            }
            if !role.credential_ready() {
                outcomes.push(format!("{candidate_name}: credential unavailable"));
                continue;
            }
            if !role.billing_active_on(&utc_date) {
                outcomes.push(format!("{candidate_name}: billing policy inactive"));
                continue;
            }
            if input_upper_bound > role.max_input_tokens.unwrap_or(0) {
                outcomes.push(format!("{candidate_name}: input ceiling exceeded"));
                continue;
            }

            let attempt_id = match store
                .claim_cloud_job_attempt(
                    &job.job_id,
                    &candidate_name,
                    &role.model,
                    role.max_requests_per_day.unwrap_or(0),
                )
                .map_err(|error| error.to_string())?
            {
                CloudAttemptClaim::Started(attempt_id) => attempt_id,
                CloudAttemptClaim::DailyLimitReached => {
                    outcomes.push(format!("{candidate_name}: daily request ceiling reached"));
                    continue;
                }
                CloudAttemptClaim::JobUnavailable => {
                    return Err("cloud job was claimed by another request".into());
                }
            };
            requested = true;
            let analysis = match cloud_dispatch::analyze(&role, &job.document) {
                Ok(analysis) => analysis,
                Err(error) => {
                    store
                        .fail_cloud_job_attempt(&job.job_id, attempt_id, &error)
                        .map_err(|store_error| store_error.to_string())?;
                    outcomes.push(format!("{candidate_name}: {error}"));
                    continue;
                }
            };
            let result = serde_json::to_value(analysis).map_err(|error| error.to_string())?;
            if !store
                .complete_cloud_job_attempt(&job.job_id, attempt_id, &result)
                .map_err(|error| error.to_string())?
            {
                return Err("cloud job result could not be committed".into());
            }
            return store
                .cloud_derivative_state(
                    &job.source,
                    &job.item_id,
                    &job.source_revision,
                    &job.preview_hash,
                )
                .map_err(|error| error.to_string());
        }

        let detail = if outcomes.is_empty() {
            "no same-tier provider is configured".to_string()
        } else {
            outcomes.join("; ")
        };
        if requested {
            Err(format!("dispatch failed: {detail}"))
        } else {
            Err(format!("provider policy blocked dispatch: {detail}"))
        }
    })
    .await;

    match result {
        Ok(Ok(state)) => (StatusCode::OK, Json(json!(state))),
        Ok(Err(error)) if error.starts_with("provider role") => {
            error_response(StatusCode::BAD_REQUEST, error)
        }
        Ok(Err(error)) if error.starts_with("provider policy blocked") => {
            error_response(StatusCode::BAD_REQUEST, error)
        }
        Ok(Err(error)) if error.starts_with("dispatch failed:") => {
            (StatusCode::BAD_GATEWAY, Json(json!({ "error": error })))
        }
        Ok(Err(error)) if error.starts_with("cloud job") => {
            (StatusCode::CONFLICT, Json(json!({ "error": error })))
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "cloud job execution failed" })),
        ),
    }
}
