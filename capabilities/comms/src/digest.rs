//! Producing a digest for one stored item, whatever source it came from.
//!
//! `libs/summarize` owns the ladder, the prompt and the Mermaid gate. This
//! module owns the three things that are specific to Axon: **where the source
//! text comes from**, **whether a remote target is allowed to see it**, and
//! **what has to be removed before the result is written down**.
//!
//! ## The mail body never lands
//!
//! A mail digest reads the message body with `format=full`, hands it to the
//! model and drops it. Nothing writes it. The operator's mail doctrine is that
//! a message is distilled into an outcome and never retained as a local copy, so
//! the digest row is the only thing that survives [`generate`]. The sweep is
//! untouched and still reads
//! `format=metadata` — reading a body is a separate, bounded, explicit act.
//!
//! ## Personal and Private content never leaves the machine
//!
//! The stored data class decides, and it decides through the same gate the
//! reviewed-derivative queue uses: [`cloud_derivative::tier_allows`], asked
//! about the passthrough representation, because a digest hands the model the
//! source text as it stands. Anything it does not admit becomes
//! `Reach::LoopbackOnly`, and `libs/summarize` refuses a non-loopback endpoint
//! outright rather than downgrading it. Mail is never `public` by construction
//! (`content_item::DataClass::classify_mail`), so a mail digest is loopback-only
//! by the same rule that already governs mail relevance.
//!
//! This used to be its own `data_class == "public"` expression here — a second
//! copy of a policy that already had a home, and a laxer one: it admitted any
//! non-loopback target for a public item, including an https endpoint with no
//! reviewed cloud policy on it at all.
//!
//! ## A Private digest is redacted before it is stored
//!
//! For `vault` content the metadata *is* the payload — a one-time code arrives
//! in the subject line, and a model asked to summarize that mail will quote the
//! code back. The produced text therefore goes through the same deterministic
//! detector the sweep uses on subject and snippet, and the count is recorded, so
//! a digest cannot republish what the sweep redacted.

use crate::cloud_derivative::{self, redact_review_field, RedactionFinding};
use crate::config::Config;
use crate::content_item;
use crate::google;
use crate::store::{Store, StoredDigest};
use crate::summarize::{self, Depth, Directive, Outcome, Reach, Target};
use crate::Result;

/// How many retryable failures a row accumulates before the automatic pass
/// leaves it alone. An explicit press always runs regardless — the operator can
/// see the model is back up.
pub const MAX_ATTEMPTS: i32 = 3;

/// The target for one specific piece of work.
///
/// `Depth::Detailed` always takes the strongest configured role. That is the
/// press meaning "give me more", and answering it on the small model would be
/// the one case where the ladder's promise — that a rung is a step up rather
/// than a longer version of the same guess — stops being true.
///
/// Otherwise a light role is used when the source demonstrably fits its context
/// window, input and output together. Everything else falls through to the
/// strong role, so a machine with no light role configured, or a source too big
/// for it, behaves exactly as it did before this existed.
pub fn role_for(
    cfg: &Config,
    directive: &Directive,
    source_chars: usize,
) -> Option<axon_inference::ResolvedRole> {
    let shape = directive.shape_for(source_chars);
    if directive.depth == Depth::Standard {
        if let Some(light) = cfg.light_summarization_role() {
            let window = light.max_input_tokens.unwrap_or_default();
            if summarize::fits_context(source_chars, shape, window) {
                return Some(light);
            }
        }
    }
    cfg.summarization_role()
}

/// Every producer string this machine could currently write for a digest.
///
/// A list rather than one string because the role is chosen per item: a short
/// source may be digested by the light model and a long one by the strong model
/// on the same pass, and both are current. Checking staleness against a single
/// producer would mark every light-model digest stale on the next sweep and
/// re-digest it forever, which is worse than the gap it was meant to close.
pub fn producer_revisions(cfg: &Config) -> Vec<String> {
    [cfg.summarization_role(), cfg.light_summarization_role()]
        .into_iter()
        .flatten()
        .map(|role| summarize::producer(&role.cache_key(), summarize::DIGEST_PROMPT_REVISION))
        .collect()
}

/// Where one resolved role's requests go.
///
/// The `Target` shape is deliberately plain data: `libs/summarize` never learns
/// what an `InferenceConfig` is, so a capability with no inference dependency
/// can still call it.
fn to_target(cfg: &Config, role: &axon_inference::ResolvedRole) -> Target {
    let loopback = role.is_loopback();
    Target {
        endpoint: role.chat_completions_endpoint(),
        model: role.model.clone(),
        api_key: role.bearer_key(),
        loopback,
        // Only a local target gets a gate. A hosted provider queues for itself
        // and shares no GPU with anything here, so serialising against it would
        // cost latency and buy nothing.
        //
        // Keyed by backend, not by machine: AFM and oMLX are both loopback and
        // both local, and they share no memory pool. One key for both meant a
        // two-second AFM digest waited out a twenty-second oMLX prefill and
        // then reported the machine busy.
        gate: loopback.then(|| {
            crate::local_gate::AdvisoryGate::shared(&cfg.database_url, &role.backend_name)
        }),
    }
}

/// The stored producer string for the current target, or `None` when no
/// summarization role is configured.
pub fn producer_revision(cfg: &Config) -> Option<String> {
    cfg.summarization_role()
        .map(|role| summarize::producer(&role.cache_key(), summarize::DIGEST_PROMPT_REVISION))
}

pub fn diagram_producer_revision(cfg: &Config) -> Option<String> {
    cfg.summarization_role()
        .map(|role| summarize::producer(&role.cache_key(), summarize::DIAGRAM_PROMPT_REVISION))
}

pub fn chart_producer_revision(cfg: &Config) -> Option<String> {
    cfg.summarization_role()
        .map(|role| summarize::producer(&role.cache_key(), summarize::chart::CHART_PROMPT_REVISION))
}

/// What a source hands the model, and under which policy.
struct SourceText {
    text: String,
    data_class: String,
}

impl SourceText {
    /// How far this text may travel to reach `cloud_data_tier`.
    ///
    /// A digest sends the source text as it stands — nothing is redacted on the
    /// way out — which is exactly what `verbatim_send_allowed` answers. Handing
    /// the question to the reviewed-derivative gate is the point: the answer
    /// used to be an independent `data_class == "public"` here, which is a
    /// second copy of a policy that already had a home and, being a copy, was
    /// already laxer than it.
    ///
    /// `None` — every loopback role, and any endpoint nobody gave a reviewed
    /// cloud policy — admits nothing. That is not a restriction on local models:
    /// `libs/summarize` only consults the verdict for a non-loopback target.
    fn reach(&self, cloud_data_tier: Option<&str>) -> Reach {
        if cloud_derivative::verbatim_send_allowed(cloud_data_tier, &self.data_class) {
            Reach::CloudCleared
        } else {
            Reach::LoopbackOnly
        }
    }

    fn redact_before_persistence(&self) -> bool {
        content_item::redact_before_persistence(&self.data_class)
    }
}

/// The verdict for one resolved role, or `LoopbackOnly` when no role resolved.
///
/// An unresolved role produces `Outcome::Unconfigured` before the verdict is
/// consulted at all, so the value is unobservable — but defaulting it closed
/// keeps that true if the order ever changes.
fn reach_for(gathered: &SourceText, role: Option<&axon_inference::ResolvedRole>) -> Reach {
    match role {
        Some(role) => gathered.reach(role.cloud_data_tier.map(|tier| tier.as_str())),
        None => Reach::LoopbackOnly,
    }
}

/// Gather the text worth digesting for one stored item.
///
/// `None` means the item does not exist. An item that exists but has no usable
/// text comes back with an empty string, which the ladder correctly reads as
/// nothing worth digesting.
fn source_text(store: &Store, cfg: &Config, source: &str, id: &str) -> Result<Option<SourceText>> {
    match source {
        "feed" => Ok(store
            .get_feed(id)
            .map_err(|error| crate::CommsError::Other(detail(error.as_ref())))?
            .map(|item| SourceText {
                // The stored class, not a literal. `data_class: "public"` here
                // meant every feed digest reported itself remotely eligible and
                // skipped redaction, on an item nobody had classified.
                data_class: item.data_class,
                // The transcript is the source; the stored summary is a
                // previous answer, and digesting an answer compounds whatever
                // it got wrong.
                text: item.transcript.unwrap_or_default(),
            })),
        "mail" => {
            let Some(item) = store
                .get_triage(id)
                .map_err(|error| crate::CommsError::Other(detail(error.as_ref())))?
            else {
                return Ok(None);
            };
            // The body is fetched here and dropped when this function returns
            // its digest. The snippet is the fallback for a thread Gmail no
            // longer has, or a token that cannot be refreshed right now.
            let body = match google::access_token(&cfg.google_env_path) {
                Ok(token) => google::thread_body_text(&token, id).unwrap_or(None),
                Err(_) => None,
            };
            let text = body
                .filter(|body| !body.trim().is_empty())
                .or_else(|| item.snippet.clone())
                .unwrap_or_default();
            Ok(Some(SourceText {
                text,
                data_class: item.data_class,
            }))
        }
        "calendar" => Ok(calendar_entry_text(cfg, id)?),
        other => Err(crate::CommsError::Other(format!(
            "unknown digest source {other:?}"
        ))),
    }
}

/// One calendar entry, read over Calendar's own HTTP contract.
///
/// Comms already reads Trips this way for its evaluation context
/// (`travel.rs`); one more bounded cross-capability read follows that precedent
/// instead of opening a second capability's database schema. The entry's own
/// description and notes are the source — a calendar entry is Personal by
/// construction, so this never reaches a remote target.
fn calendar_entry_text(cfg: &Config, id: &str) -> Result<Option<SourceText>> {
    let base = cfg.calendar_context.base_url.trim_end_matches('/');
    let http = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_millis(
            cfg.calendar_context.timeout_ms,
        ))
        .build()?;
    let response = http
        .get(format!("{base}/api/content/calendar/{id}"))
        .send()?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(crate::CommsError::Other(format!(
            "calendar read failed with HTTP {}",
            response.status()
        )));
    }
    let item: serde_json::Value = response.json()?;
    let field = |key: &str| {
        item.get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    let mut text = field("title");
    for key in ["summary", "content"] {
        let value = field(key);
        if !value.is_empty() {
            text.push_str("\n\n");
            text.push_str(&value);
        }
    }
    Ok(Some(SourceText {
        text,
        data_class: item
            .get("data_class")
            .and_then(|class| class.get("value"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("personal")
            .to_string(),
    }))
}

/// Produce and store the digest for one item.
///
/// Returns the stored row, or `None` when the item does not exist. A failure
/// class is stored rather than raised: a model that was down is a fact about the
/// row, and the reader shows it.
pub fn generate(
    store: &Store,
    cfg: &Config,
    source: &str,
    id: &str,
    directive: &Directive,
) -> Result<Option<StoredDigest>> {
    let Some(gathered) = source_text(store, cfg, source, id)? else {
        return Ok(None);
    };
    let previous = store
        .content_digest(source, id)
        .map_err(|error| crate::CommsError::Other(detail(error.as_ref())))?;
    let chars = gathered.text.chars().count();
    let source_chars = chars as i64;
    let shape = directive.shape_for(chars);
    // Resolved once, so the stored producer names the role that actually ran.
    // Deriving it separately from `summarization_role` would label a light-model
    // digest as the strong model's work, and provenance that lies is worse than
    // provenance that is missing.
    let role = role_for(cfg, directive, chars);
    let producer = role
        .as_ref()
        .map(|role| summarize::producer(&role.cache_key(), summarize::DIGEST_PROMPT_REVISION))
        .unwrap_or_else(|| "unconfigured".into());

    let outcome = summarize::digest(
        role.as_ref().map(|role| to_target(cfg, role)).as_ref(),
        &gathered.text,
        directive,
        reach_for(&gathered, role.as_ref()),
    );

    let mut redactions: Vec<RedactionFinding> = Vec::new();
    let text = match &outcome {
        Outcome::Ok(text) if gathered.redact_before_persistence() => {
            redact_review_field(Some(text), &mut redactions)
        }
        Outcome::Ok(text) => Some(text.clone()),
        _ => None,
    };

    // A retryable failure accumulates; anything else starts the count over,
    // because a success or a verdict says the previous failures are no longer
    // the state of this row.
    let attempts = match (&outcome, &previous) {
        (outcome, Some(previous)) if outcome.retryable() => previous.attempts.saturating_add(1),
        (outcome, None) if outcome.retryable() => 1,
        _ => 0,
    };

    let stored = StoredDigest {
        source: source.to_string(),
        item_id: id.to_string(),
        text,
        state: outcome.state().to_string(),
        shape: shape.as_str().to_string(),
        depth: directive.depth.as_str().to_string(),
        focus: directive.focus_text(),
        producer,
        source_chars,
        redactions: redactions
            .iter()
            .map(|finding| finding.count)
            .sum::<usize>() as i32,
        attempts,
        last_error: outcome.error_detail().map(str::to_string),
        // The diagram is a separate press and survives a regenerated digest.
        diagram: previous.as_ref().and_then(|row| row.diagram.clone()),
        diagram_state: previous.as_ref().and_then(|row| row.diagram_state.clone()),
        diagram_error: previous.as_ref().and_then(|row| row.diagram_error.clone()),
        chart: previous.as_ref().and_then(|row| row.chart.clone()),
        chart_state: previous.as_ref().and_then(|row| row.chart_state.clone()),
        chart_error: previous.as_ref().and_then(|row| row.chart_error.clone()),
        generated_at: String::new(),
    };
    store
        .upsert_content_digest(&stored)
        .map_err(|error| crate::CommsError::Other(detail(error.as_ref())))?;
    store
        .content_digest(source, id)
        .map_err(|error| crate::CommsError::Other(detail(error.as_ref())))
}

/// Produce and store a Mermaid diagram beside an existing digest.
///
/// The digest is the input when there is one — it is the model's own compressed
/// reading of the item, and a diagram of a forty-page paper drawn from the
/// paper's first 15,000 characters is a diagram of its introduction. Falls back
/// to the source text when no digest exists yet.
pub fn generate_diagram(
    store: &Store,
    cfg: &Config,
    source: &str,
    id: &str,
) -> Result<Option<StoredDigest>> {
    let Some(gathered) = source_text(store, cfg, source, id)? else {
        return Ok(None);
    };
    let existing = store
        .content_digest(source, id)
        .map_err(|error| crate::CommsError::Other(detail(error.as_ref())))?;
    let input = existing
        .as_ref()
        .and_then(|row| row.text.clone())
        .unwrap_or_else(|| gathered.text.clone());

    // Resolved once, so the verdict is asked about the role that will actually
    // serve the request. Deriving the target and the verdict from two separate
    // lookups is how a gate ends up answering about a different endpoint than
    // the one the payload goes to.
    let role = cfg.summarization_role();
    let outcome = summarize::diagram(
        role.as_ref().map(|role| to_target(cfg, role)).as_ref(),
        &input,
        reach_for(&gathered, role.as_ref()),
    );
    let producer = diagram_producer_revision(cfg).unwrap_or_else(|| "unconfigured".into());

    // A diagram hangs off a digest row, so an item digested for the first time
    // by this press needs one to exist. Generating the digest first is also the
    // better diagram: see the note above.
    if existing.is_none() {
        generate(store, cfg, source, id, &Directive::default())?;
    }

    let (diagram, error) = match &outcome {
        Outcome::Ok(diagram) => (Some(diagram.as_str()), None),
        other => (None, other.error_detail()),
    };
    let updated = store
        .update_content_diagram(source, id, diagram, outcome.state(), error, &producer)
        .map_err(|error| crate::CommsError::Other(detail(error.as_ref())))?;
    if updated == 0 {
        return Ok(None);
    }
    store
        .content_digest(source, id)
        .map_err(|error| crate::CommsError::Other(detail(error.as_ref())))
}

/// Extract and store a chartable table for one item.
///
/// The **source text** is the input, never the digest: the digest is prose the
/// model already wrote, so verifying numbers against it would only prove the
/// model agrees with itself. Verification has to run against what the source
/// actually said.
pub fn generate_chart(
    store: &Store,
    cfg: &Config,
    source: &str,
    id: &str,
) -> Result<Option<StoredDigest>> {
    let Some(gathered) = source_text(store, cfg, source, id)? else {
        return Ok(None);
    };
    let existing = store
        .content_digest(source, id)
        .map_err(|error| crate::CommsError::Other(detail(error.as_ref())))?;

    let role = cfg.summarization_role();
    let outcome = summarize::chart::chart(
        role.as_ref().map(|role| to_target(cfg, role)).as_ref(),
        &gathered.text,
        reach_for(&gathered, role.as_ref()),
    );
    let producer = chart_producer_revision(cfg).unwrap_or_else(|| "unconfigured".into());

    // A chart hangs off a digest row, so an item charted before it was digested
    // needs one to exist.
    if existing.is_none() {
        generate(store, cfg, source, id, &Directive::default())?;
    }

    let (chart, error) = match &outcome {
        Outcome::Ok(chart) => (Some(chart.as_str()), None),
        other => (None, other.error_detail()),
    };
    let updated = store
        .update_content_chart(source, id, chart, outcome.state(), error, &producer)
        .map_err(|error| crate::CommsError::Other(detail(error.as_ref())))?;
    if updated == 0 {
        return Ok(None);
    }
    store
        .content_digest(source, id)
        .map_err(|error| crate::CommsError::Other(detail(error.as_ref())))
}

/// The bounded automatic pass: digest the items of one source that still need
/// one. Returns how many rows were written.
///
/// Bounded and explicit rather than timer-driven. For mail this reads message
/// bodies, and a background job that quietly pulls every body out of a mailbox
/// is not something a machine should start doing on its own.
/// An error plus everything underneath it.
///
/// `postgres::Error` Displays as the bare string "db error" and keeps the
/// statement, the SQLSTATE and the message on its `source()` chain, so
/// `error.to_string()` at an error-type boundary throws away the only part
/// worth reading. A drain that logs "digest drain: db error" every fifteen
/// minutes reports that something is wrong and nothing about what.
fn detail(error: &(dyn std::error::Error + 'static)) -> String {
    let mut out = error.to_string();
    let mut cause = error.source();
    while let Some(next) = cause {
        out.push_str(": ");
        out.push_str(&next.to_string());
        cause = next.source();
    }
    out
}

pub fn refresh_pending(store: &Store, cfg: &Config, source: &str, limit: i64) -> Result<usize> {
    let producers = producer_revisions(cfg);
    if producers.is_empty() {
        return Ok(0);
    }
    let ids = store
        .items_needing_digest(source, &producers, MAX_ATTEMPTS, limit.clamp(1, 500))
        .map_err(|error| crate::CommsError::Other(detail(error.as_ref())))?;
    let directive = Directive::new(Depth::Standard, []);
    let mut written = 0;
    for id in ids {
        match generate(store, cfg, source, &id, &directive) {
            Ok(Some(row)) => {
                written += 1;
                // The streak the alert threshold counts. Recorded here rather
                // than in `generate` because this is the *unattended* pass: an
                // operator pressing Regenerate on a busy machine is told so on
                // the spot and is not an alert condition.
                match row.state.as_str() {
                    "capacity_aborted" => {
                        if let Some(streak) =
                            crate::capacity::record_failure(store, cfg.capacity_alert_after)
                        {
                            eprintln!(
                                "digest drain: ALERT — {streak} consecutive capacity aborts from \
                                 the local inference server; digests are not being written"
                            );
                        }
                    }
                    "generated" => crate::capacity::record_success(store),
                    _ => {}
                }
                // A model that is not configured or not answering will not
                // answer for the next hundred items either.
                if row.state == "unconfigured" {
                    break;
                }
            }
            Ok(None) => {}
            Err(_) => {}
        }
    }
    Ok(written)
}

/// The wire shape, built from the stored row.
pub fn to_contract(row: &StoredDigest) -> content_item::Digest {
    content_item::Digest {
        text: row.text.clone(),
        state: row.state.clone(),
        shape: row.shape.clone(),
        depth: row.depth.clone(),
        focus: Directive::parse_focus(&row.focus),
        producer: row.producer.clone(),
        source_chars: row.source_chars,
        redactions: row.redactions,
        attempts: row.attempts,
        last_error: row.last_error.clone(),
        diagram: row.diagram.clone(),
        diagram_state: row.diagram_state.clone(),
        diagram_error: row.diagram_error.clone(),
        // Re-parsed rather than passed through as a string: the reader indexes
        // into it, and a JSON blob delivered as a quoted string is a second
        // parse every consumer has to remember to do.
        chart: row
            .chart
            .as_deref()
            .and_then(|raw| serde_json::from_str(raw).ok()),
        chart_state: row.chart_state.clone(),
        chart_error: row.chart_error.clone(),
        generated_at: row.generated_at.clone(),
    }
}

/// Parse a reader's refine request. An unknown depth is rejected rather than
/// defaulted: a button that silently does something else than it says is worse
/// than one that reports it could not.
pub fn parse_directive(
    depth: Option<&str>,
    focus: Vec<String>,
) -> std::result::Result<Directive, String> {
    let depth = match depth {
        None => Depth::Standard,
        Some(value) => Depth::parse(value)
            .ok_or_else(|| format!("depth must be \"standard\" or \"detailed\", not {value:?}"))?,
    };
    Ok(Directive::new(depth, focus))
}

/// A short, reader-facing account of why there is no digest text.
pub fn state_explanation(state: &str, shape: &str) -> &'static str {
    match state {
        "generated" => "",
        "skipped_short" if shape == "none" => {
            "Too short to be worth a digest — the source is already the summary."
        }
        "skipped_short" => "Nothing to digest.",
        "remote_refused" => {
            "This item is Personal or Private and the configured model is not local."
        }
        "unconfigured" => "No summarization model is configured on this machine.",
        "timeout" => "The local model did not answer in time.",
        // Deliberately about the machine, not the model. The server took this
        // request and ran out of memory part-way through it; the same item
        // digests fine once something stops holding the GPU. Saying "the model
        // answered with nothing" here sent a reader looking at the model, the
        // prompt and the transcript, none of which were involved.
        "capacity_aborted" => "The local server ran out of memory part-way through. It will retry.",
        "empty_response" => "The local model answered with nothing.",
        _ => "The local model could not be reached.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_depth_is_rejected_rather_than_defaulted() {
        assert!(parse_directive(Some("deeper"), Vec::new()).is_err());
        assert!(parse_directive(Some(""), Vec::new()).is_err());
        assert_eq!(
            parse_directive(None, Vec::new()).unwrap().depth,
            Depth::Standard
        );
        assert_eq!(
            parse_directive(Some("detailed"), vec!["cost".into()])
                .unwrap()
                .focus,
            vec!["cost".to_string()]
        );
    }

    fn gathered(class: &str) -> SourceText {
        SourceText {
            text: String::new(),
            data_class: class.into(),
        }
    }

    /// The one rule that keeps mail off a cloud endpoint: mail is never
    /// `public`, and nothing but `public` may be sent verbatim. A digest sends
    /// the source text verbatim, so this is the whole question.
    #[test]
    fn nothing_but_a_public_item_may_reach_a_cloud_tier() {
        for tier in [None, Some("public"), Some("pseudonymized_personal")] {
            for class in ["personal", "vault", "something-new"] {
                assert_eq!(
                    gathered(class).reach(tier),
                    Reach::LoopbackOnly,
                    "{class} must stay local against tier {tier:?}"
                );
            }
        }
        assert_eq!(
            gathered("public").reach(Some("public")),
            Reach::CloudCleared
        );
        assert_eq!(
            gathered("public").reach(Some("pseudonymized_personal")),
            Reach::CloudCleared
        );
    }

    /// A role with no reviewed cloud policy has no `cloud_data_tier`, and an
    /// undeclared tier admits nothing — including public content. The previous
    /// rule was laxer: it read the class alone and would have sent a public
    /// item to any https endpoint somebody pointed the summarization role at.
    #[test]
    fn an_endpoint_with_no_declared_tier_receives_nothing() {
        for class in ["public", "personal", "vault"] {
            assert_eq!(gathered(class).reach(None), Reach::LoopbackOnly);
        }
    }

    /// No role resolved means no request; the verdict still defaults closed.
    #[test]
    fn an_unresolved_role_is_loopback_only() {
        assert_eq!(reach_for(&gathered("public"), None), Reach::LoopbackOnly);
    }

    #[test]
    fn only_private_content_is_redacted_before_the_digest_is_stored() {
        let private = SourceText {
            text: String::new(),
            data_class: "vault".into(),
        };
        assert!(private.redact_before_persistence());
        for class in ["personal", "public"] {
            assert!(!SourceText {
                text: String::new(),
                data_class: class.into(),
            }
            .redact_before_persistence());
        }
    }

    /// A skip is a claim about the source, so the reader has to be able to say
    /// which one it was without re-deriving anything.
    #[test]
    fn every_non_generated_state_explains_itself() {
        for state in [
            "skipped_short",
            "remote_refused",
            "unconfigured",
            "timeout",
            "empty_response",
            "http_error",
            "model_error",
        ] {
            assert!(
                !state_explanation(state, "none").is_empty(),
                "{state} has no explanation"
            );
        }
        assert!(state_explanation("generated", "brief").is_empty());
    }

    /// The redactor the digest path reuses is the sweep's own. This is the
    /// property that matters: a model asked to summarize a one-time-code mail
    /// will quote the code, and the digest must not be where it gets published.
    #[test]
    fn a_private_digest_cannot_republish_a_one_time_code() {
        let model_answer = "- Your verification code is 448215\n- It expires in 10 minutes";
        let mut findings: Vec<RedactionFinding> = Vec::new();
        let stored = redact_review_field(Some(model_answer), &mut findings).unwrap();
        assert!(!stored.contains("448215"), "the code survived: {stored}");
        assert!(
            findings.iter().map(|finding| finding.count).sum::<usize>() > 0,
            "a redaction that removes something must be counted"
        );
    }
}
