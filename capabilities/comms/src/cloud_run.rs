//! Queueing and executing one reviewed cloud job, whatever it asks for.
//!
//! Two callers, one path. The dashboard queues an analysis of a document a human
//! previewed and approved, then presses run. The digest drain queues a digest of
//! a long `public` feed item that no local rung on this machine can hold, and
//! runs it on the same pass. Both go through [`run_job`], so the budget counter,
//! the attempts ledger, the five-call cap, provider failover and the
//! `preview_hash` pin are one implementation rather than two.
//!
//! ## What the door checks, and where
//!
//! `cloud_derivative::tier_allows` is the door (see `d6f7cb9`). It is asked here
//! twice, about different things, and neither is a copy of the other:
//!
//! - At **enqueue**, [`enqueue_digest_job`] asks `verbatim_send_allowed` — may
//!   this item's stored class go to this provider's tier *as it stands*. Only
//!   `public` may. A `personal` item has a cloud lane, and it is the redacted
//!   derivative behind human approval, not an unattended drain.
//! - At **dispatch**, [`run_job`] asks `tier_allows` about the exact staged
//!   representation, again per failover candidate, because the roster and the
//!   role's policy can both have changed since the job was queued.
//!
//! A digest job is asked both questions at dispatch. The narrow one is not
//! redundant there: `tier_allows` alone would admit a *redacted personal*
//! derivative to the pseudonymized tier, which is correct for the analysis task
//! a human approved and wrong for a job a timer created.

use axon_inference::ResolvedRole;

use crate::cloud_derivative::{self, CloudDocumentInput};
use crate::cloud_dispatch;
use crate::config::Config;
use crate::store::{
    CloudAttemptClaim, CloudDerivativeApproval, CloudDerivativeState, CloudDispatchJob,
    CloudQueueRequest, FeedItem, Store,
};

/// A digest job that now exists, and the provider it named first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedDigest {
    pub job_id: String,
    pub provider_role: String,
}

/// Why an item gets no cloud digest job. Typed rather than a string because the
/// first variant is the one the whole classification build exists to produce,
/// and a test asserting on `error.contains("class")` would pass for the wrong
/// reason the day the wording changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DigestNotQueued {
    /// No configured provider tier admits this item's stored class verbatim.
    /// `personal`, `vault` and anything undeclared land here, and so does a
    /// `public` item on a machine whose only cloud roles declare no tier.
    ClassNotCleared {
        data_class: String,
    },
    /// `vault` has no approvable representation at all, so there was nothing to
    /// stage. Reached only when a class the tier check admitted is nevertheless
    /// refused by `prepare` — which cannot happen today and is kept as the
    /// second lock rather than as an `unreachable!`.
    VaultRefused,
    /// A tier-cleared provider exists but cannot be used right now: no
    /// credential, billing lapsed, the document is past its input ceiling, or
    /// the day's request budget is spent.
    NoProviderAvailable(String),
    Store(String),
}

impl std::fmt::Display for DigestNotQueued {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ClassNotCleared { data_class } => write!(
                f,
                "no cloud provider tier admits a {data_class} item verbatim"
            ),
            Self::VaultRefused => f.write_str("vault content has no cloud derivative"),
            Self::NoProviderAvailable(detail) => {
                write!(f, "no cloud provider is available: {detail}")
            }
            Self::Store(detail) => write!(f, "store error: {detail}"),
        }
    }
}

/// Queue a cloud digest for one stored feed item.
///
/// Stages the reviewed derivative and queues the job; it does not dispatch. The
/// caller runs [`run_job`] when it wants the request made, which keeps "a job
/// exists" and "a provider was paid a call" as two observable steps.
pub fn enqueue_digest_job(
    store: &Store,
    cfg: &Config,
    item: &FeedItem,
) -> Result<QueuedDigest, DigestNotQueued> {
    let preview = cloud_derivative::prepare(&CloudDocumentInput::from_feed(item))
        .map_err(|_| DigestNotQueued::VaultRefused)?;
    let input_upper_bound = cloud_dispatch::input_token_upper_bound(&preview.document);
    let utc_date = store
        .utc_date()
        .map_err(|error| DigestNotQueued::Store(error.to_string()))?;

    let cleared = tier_cleared_roles(&cfg.inference, &item.data_class);
    if cleared.is_empty() {
        return Err(DigestNotQueued::ClassNotCleared {
            data_class: item.data_class.clone(),
        });
    }

    let mut blocked = Vec::new();
    for (name, role) in cleared {
        if !role.credential_ready() {
            blocked.push(format!("{name}: credential unavailable"));
            continue;
        }
        if !role.billing_active_on(&utc_date) {
            blocked.push(format!("{name}: billing policy inactive"));
            continue;
        }
        if input_upper_bound > role.max_input_tokens.unwrap_or(0) {
            blocked.push(format!("{name}: input ceiling exceeded"));
            continue;
        }
        let calls = store
            .cloud_provider_calls_today(&name)
            .map_err(|error| DigestNotQueued::Store(error.to_string()))?;
        if calls >= role.max_requests_per_day.unwrap_or(0) {
            blocked.push(format!("{name}: daily request ceiling reached"));
            continue;
        }

        store
            .stage_cloud_derivative(&CloudDerivativeApproval {
                source: preview.source.clone(),
                item_id: preview.id.clone(),
                source_revision: preview.source_revision.clone(),
                preview_hash: preview.preview_hash.clone(),
                original_data_class: preview.original_data_class.clone(),
                derivative_data_class: preview.derivative_data_class.clone(),
                transformation: preview.transformation.into(),
                document: preview.document.clone(),
                redaction_count: preview.redaction_count as i32,
            })
            .map_err(|error| DigestNotQueued::Store(error.to_string()))?;
        let state = store
            .queue_cloud_derivative(&CloudQueueRequest {
                source: preview.source.clone(),
                item_id: preview.id.clone(),
                source_revision: preview.source_revision.clone(),
                preview_hash: preview.preview_hash.clone(),
                provider_role: name.clone(),
                task: cloud_dispatch::DIGEST_TASK_VERSION.into(),
            })
            .map_err(|error| DigestNotQueued::Store(error.to_string()))?;
        return Ok(QueuedDigest {
            job_id: state.job_id.unwrap_or_default(),
            provider_role: name,
        });
    }
    Err(DigestNotQueued::NoProviderAvailable(blocked.join("; ")))
}

/// Configured cloud roles whose tier admits this class **verbatim**, best
/// failover priority first.
///
/// The narrow question, deliberately: an unattended digest hands the provider
/// the item's own text with nothing removed, which is precisely what
/// `verbatim_send_allowed` answers and precisely what `tier_allows` on its own
/// would not — that one also admits a redacted `personal` derivative, which
/// belongs to the reviewed queue and not to a timer.
fn tier_cleared_roles(
    inference: &axon_inference::InferenceConfig,
    data_class: &str,
) -> Vec<(String, ResolvedRole)> {
    let mut roles = inference
        .roles_with_prefix("cloud_")
        .into_iter()
        .filter(|(_, role)| role.has_cloud_policy())
        .filter(|(_, role)| {
            cloud_derivative::verbatim_send_allowed(
                role.cloud_data_tier.map(|tier| tier.as_str()),
                data_class,
            )
        })
        .collect::<Vec<_>>();
    roles.sort_by(|(left_name, left), (right_name, right)| {
        tier_rank(left)
            .cmp(&tier_rank(right))
            .then_with(|| left.failover_priority().cmp(&right.failover_priority()))
            .then_with(|| left_name.cmp(right_name))
    });
    roles
}

/// Narrowest declared tier first.
///
/// A tier says what the operator reviewed a provider to receive. A
/// pseudonymized-personal role admits a public document too, so ranking on
/// `failover_priority` alone sent every public digest to the role reviewed for
/// personal content and left the role declared `public` — the one that exists
/// for exactly this — never used. Two consequences, and the second is the one
/// that bites: `cloud_failover_roles` builds the retry roster from roles
/// sharing the *selected* role's tier, so the tier chosen here also decides
/// which providers can cover for it.
fn tier_rank(role: &ResolvedRole) -> u8 {
    match role.cloud_data_tier {
        Some(axon_inference::CloudDataTier::Public) => 0,
        Some(axon_inference::CloudDataTier::PseudonymizedPersonal) => 1,
        None => u8::MAX,
    }
}

/// Execute one queued cloud job through its failover roster.
///
/// Lifted out of the server handler unchanged in behaviour so the drain can call
/// it. The handler is now the HTTP shell around this.
pub fn run_job(store: &Store, cfg: &Config, job_id: &str) -> Result<CloudDerivativeState, String> {
    let job = store
        .cloud_job_for_dispatch(job_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| {
            "cloud job is completed, running, stale, or past its retry limit".to_string()
        })?;
    if !matches!(
        job.task.as_str(),
        cloud_dispatch::TASK_VERSION | cloud_dispatch::DIGEST_TASK_VERSION
    ) {
        return Err("cloud job task is unsupported".into());
    }
    let selected_role = cfg
        .inference
        .role(&job.provider_role)
        .filter(|role| role.has_cloud_policy())
        .ok_or_else(|| "provider role is no longer a reviewed HTTPS cloud role".to_string())?;
    if !admits(&selected_role, &job) {
        return Err("provider role no longer allows the staged derivative".into());
    }
    let utc_date = store.utc_date().map_err(|error| error.to_string())?;
    let input_upper_bound = cloud_dispatch::input_token_upper_bound(&job.document);
    let mut requested = false;
    let mut outcomes = Vec::new();

    for (candidate_name, role) in cfg.inference.cloud_failover_roles(&job.provider_role) {
        if !admits(&role, &job) {
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
        let result = match perform(store, &job, &role) {
            Ok(result) => result,
            Err(error) => {
                store
                    .fail_cloud_job_attempt(&job.job_id, attempt_id, &error)
                    .map_err(|store_error| store_error.to_string())?;
                outcomes.push(format!("{candidate_name}: {error}"));
                continue;
            }
        };
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
}

/// Whether this role's tier admits this job's staged derivative.
///
/// The digest task additionally has to clear the verbatim question: its document
/// is a passthrough of a `public` item and must stay one, whatever a tier would
/// accept in redacted form for the reviewed analysis queue.
fn admits(role: &ResolvedRole, job: &CloudDispatchJob) -> bool {
    let tier = role.cloud_data_tier.map(|tier| tier.as_str());
    if job.task == cloud_dispatch::DIGEST_TASK_VERSION
        && !cloud_derivative::verbatim_send_allowed(tier, &job.original_data_class)
    {
        return false;
    }
    cloud_derivative::tier_allows(
        tier,
        &job.original_data_class,
        &job.derivative_data_class,
        &job.transformation,
    )
}

/// Make the one provider request this job asks for, and persist whatever the
/// task's own home needs persisted, returning the JSON stored on the attempt.
fn perform(
    store: &Store,
    job: &CloudDispatchJob,
    role: &ResolvedRole,
) -> Result<serde_json::Value, String> {
    match job.task.as_str() {
        cloud_dispatch::TASK_VERSION => {
            let analysis = cloud_dispatch::analyze(role, &job.document)?;
            serde_json::to_value(analysis).map_err(|error| error.to_string())
        }
        cloud_dispatch::DIGEST_TASK_VERSION => {
            let shape =
                crate::summarize::Directive::default().shape_for(job.document.chars().count());
            let text = cloud_dispatch::digest(role, &job.document, shape)?;
            // Written before the attempt is completed: an attempt marked
            // succeeded with no digest row behind it is a job the drain will
            // never retry and a reader will never see.
            crate::digest::store_cloud_digest(store, job, role, &text, shape)?;
            Ok(serde_json::json!({
                "schema_version": cloud_dispatch::DIGEST_RESULT_SCHEMA_VERSION,
                "text": text,
            }))
        }
        other => Err(format!("cloud job task {other:?} is unsupported")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::CloudDispatchJob;

    fn inference(tiers: &[(&str, &str, u16)]) -> axon_inference::InferenceConfig {
        let mut roles = serde_json::Map::new();
        for (name, tier, priority) in tiers {
            roles.insert(
                (*name).into(),
                serde_json::json!({
                    "backend": "hosted",
                    "model": "some-model",
                    "provider_name": "Some Provider",
                    "cloud_data_tier": tier,
                    "billing_mode": "free_only",
                    "failover_priority": priority,
                    "max_requests_per_day": 10,
                    "max_input_tokens": 24000,
                }),
            );
        }
        serde_json::from_value(serde_json::json!({
            "backends": {
                "hosted": {
                    "api": "openai",
                    "base_url": "https://example.invalid/v1",
                    "api_key_file": probe_key_file(),
                },
            },
            "roles": roles,
        }))
        .expect("the probe config is well formed")
    }

    /// A materialized credential for the probe backend. Without one the roles
    /// are policy-complete but not dispatchable, and every enqueue would be
    /// refused for the wrong reason — which would make the refusal tests below
    /// pass while proving nothing.
    fn probe_key_file() -> String {
        let path =
            std::env::temp_dir().join(format!("axon-cloud-run-probe-key-{}", std::process::id()));
        std::fs::write(&path, "probe-key\n").expect("the probe key file is writable");
        path.to_string_lossy().into_owned()
    }

    fn job(task: &str, original: &str, derivative: &str, transformation: &str) -> CloudDispatchJob {
        CloudDispatchJob {
            job_id: "cloud-job-probe".into(),
            source: "feed".into(),
            item_id: "item".into(),
            source_revision: "rev".into(),
            preview_hash: "hash".into(),
            provider_role: "cloud_pseudonymized".into(),
            task: task.into(),
            original_data_class: original.into(),
            derivative_data_class: derivative.into(),
            transformation: transformation.into(),
            document: "document".into(),
            provider_calls: 0,
        }
    }

    /// C21's refusal, at the selection step that decides where a digest could go
    /// at all. Only `public` is cleared, and it is cleared only by a tier that
    /// declares itself.
    #[test]
    fn only_a_public_item_finds_a_tier_cleared_provider() {
        let inference = inference(&[
            ("cloud_public", "public", 30),
            ("cloud_pseudonymized", "pseudonymized_personal", 10),
        ]);
        assert_eq!(
            tier_cleared_roles(&inference, "public")
                .into_iter()
                .map(|(name, _)| name)
                .collect::<Vec<_>>(),
            vec![
                "cloud_public".to_string(),
                "cloud_pseudonymized".to_string()
            ],
            "both tiers admit public verbatim; the one declared for public data \
             goes first, even though its failover_priority is worse"
        );
        for class in ["personal", "vault", "something-new", ""] {
            assert!(
                tier_cleared_roles(&inference, class).is_empty(),
                "{class} found a cloud provider for a verbatim send"
            );
        }
    }

    /// The dispatch-time half of the same refusal. A digest job whose original
    /// class is `personal` must find no candidate, even against the tier whose
    /// entire purpose is pseudonymized personal content — because that tier
    /// admits the *redacted* derivative a human approved, and this job's
    /// document is a passthrough.
    #[test]
    fn dispatch_refuses_a_personal_digest_that_a_reviewed_analysis_would_pass() {
        let role = inference(&[("cloud_pseudonymized", "pseudonymized_personal", 10)])
            .role("cloud_pseudonymized")
            .expect("the probe role resolves");

        let analysis = job(
            cloud_dispatch::TASK_VERSION,
            "personal",
            "personal",
            cloud_derivative::REDACTION_VERSION,
        );
        assert!(
            admits(&role, &analysis),
            "the reviewed analysis lane for personal content is unchanged"
        );

        let digest = job(
            cloud_dispatch::DIGEST_TASK_VERSION,
            "personal",
            "personal",
            cloud_derivative::REDACTION_VERSION,
        );
        assert!(!admits(&role, &digest));

        let vault = job(
            cloud_dispatch::DIGEST_TASK_VERSION,
            "vault",
            "personal",
            cloud_derivative::REDACTION_VERSION,
        );
        assert!(!admits(&role, &vault));
    }

    /// A public passthrough digest is the one shape that passes, and only
    /// against a declared tier.
    #[test]
    fn dispatch_admits_a_public_passthrough_digest() {
        let role = inference(&[("cloud_public", "public", 30)])
            .role("cloud_public")
            .expect("the probe role resolves");
        assert!(admits(
            &role,
            &job(
                cloud_dispatch::DIGEST_TASK_VERSION,
                "public",
                "public",
                cloud_derivative::PASSTHROUGH_VERSION,
            )
        ));
    }

    /// The two tests that need a real Postgres. Their own module so the
    /// bazel split can name them: `comms_test` is the hermetic target and
    /// skips this path, `comms_postgres_test` selects it, exactly as both
    /// already do for `store::tests::`.
    #[cfg(test)]
    mod postgres_tests {
        use super::*;

        fn feed_item(data_class: &str) -> FeedItem {
            let mut item = FeedItem::new(
                &format!("https://example.com/axon-c21-{data_class}"),
                "news",
                "article",
            );
            item.title = Some("A long document".into());
            item.author = Some("Someone".into());
            item.transcript = Some("word ".repeat(4_000));
            item.data_class = data_class.into();
            item
        }

        /// C21's enqueue refusal, against a live store that would have written the
        /// row. Asserted on the typed reason *and* on the store being untouched:
        /// "it returned an error" is not the claim — the claim is that no
        /// derivative was staged and no job exists for a non-`public` item.
        #[test]
        fn a_personal_item_gets_no_cloud_digest_job() {
            let (store, _schema) = crate::store::tests::open_test_store("cloud_digest_refusal");
            let cfg = Config::with_inference(inference(&[
                ("cloud_public", "public", 30),
                ("cloud_pseudonymized", "pseudonymized_personal", 10),
            ]));

            for class in ["personal", "vault", "something-new"] {
                let item = feed_item(class);
                let refusal = enqueue_digest_job(&store, &cfg, &item)
                    .expect_err("a non-public item must not reach a cloud provider verbatim");
                let expected = if class == "vault" {
                    // `prepare` refuses Private before the tier question is asked.
                    DigestNotQueued::VaultRefused
                } else {
                    DigestNotQueued::ClassNotCleared {
                        data_class: class.into(),
                    }
                };
                assert_eq!(
                    refusal, expected,
                    "{class} was refused for the wrong reason"
                );
                assert_eq!(
                    store
                        .cloud_derivative_state("feed", &item.id, "any", "any")
                        .expect("the state query answers")
                        .dispatch_status,
                    "not_queued",
                    "{class} left a queued cloud job behind"
                );
            }
        }

        /// The same call, same store, for an item that is positively `public`, has
        /// to actually queue — otherwise the test above would pass on a machine
        /// where enqueueing never works at all.
        #[test]
        fn a_public_item_does_get_a_cloud_digest_job() {
            let (store, _schema) = crate::store::tests::open_test_store("cloud_digest_enqueue");
            let cfg = Config::with_inference(inference(&[("cloud_public", "public", 30)]));
            let item = feed_item("public");

            let queued = enqueue_digest_job(&store, &cfg, &item)
                .expect("a public item finds the public-tier provider");
            assert_eq!(queued.provider_role, "cloud_public");
            let job = store
                .cloud_job_for_dispatch(&queued.job_id)
                .expect("the job query answers")
                .expect("the queued job is dispatchable");
            assert_eq!(job.task, cloud_dispatch::DIGEST_TASK_VERSION);
            assert_eq!(job.original_data_class, "public");
            assert_eq!(job.transformation, cloud_derivative::PASSTHROUGH_VERSION);
        }
    }
}
