//! One path from a swept Gmail thread to a stored proposal.
//!
//! Both sweep entry points — the `comms` CLI and comms-server's HTTP sweep —
//! come through here, and that is the entire point of the module existing. The
//! redaction gate below is only a gate if there is no second way in; when the
//! two call sites each built their own `TriageItem` they had already drifted
//! apart once, and a gate added to one of them would have been a gate over
//! half the traffic.
//!
//! What the gate does: classify from metadata, then, for the classes whose
//! metadata is itself the payload, redact the stored review fields *before*
//! the row is written. A one-time code arrives in a subject line; persisting
//! that subject verbatim publishes it to a log, an API response and a
//! dashboard in one step, and no later deletion un-publishes it.

use crate::cloud_derivative::{redact_review_field, redaction_digest, RedactionFinding};
use crate::content_item::{self, DataClass};
use crate::google::ThreadMeta;
use crate::rules;
use crate::store::TriageItem;

/// A proposal that is safe to persist, plus the evidence of what was removed
/// to make it safe. The findings carry counts and entity types only — never
/// the matched text.
#[derive(Debug, Clone)]
pub struct Intake {
    pub item: TriageItem,
    pub redactions: Vec<RedactionFinding>,
}

impl Intake {
    /// A digest identifying this redaction pass, for provenance. Empty when
    /// nothing was removed, so callers can tell "clean" from "cleaned".
    pub fn audit_digest(&self) -> Option<String> {
        (!self.redactions.is_empty()).then(|| redaction_digest(&self.redactions))
    }

    pub fn redaction_count(&self) -> usize {
        self.redactions.iter().map(|finding| finding.count).sum()
    }
}

/// Classify a swept thread and build the row to store, redacting first when
/// the class demands it.
pub fn from_thread(meta: ThreadMeta, config_rules: &[rules::Rule]) -> Intake {
    let from = meta.from_addr.clone().unwrap_or_default();
    let subject = meta.subject.clone().unwrap_or_default();
    let facts = rules::MailFacts {
        from: &from,
        subject: &subject,
        has_list_unsubscribe: meta.has_list_unsubscribe(),
    };
    let (stream, rationale) = rules::classify(&facts, config_rules);
    let classification = DataClass::classify_mail(&stream, &from, &subject);

    let mut redactions = Vec::new();
    let (subject, snippet) = if content_item::redact_before_persistence(&classification.value) {
        (
            redact_review_field(meta.subject.as_deref(), &mut redactions),
            redact_review_field(meta.snippet.as_deref(), &mut redactions),
        )
    } else {
        (meta.subject.clone(), meta.snippet.clone())
    };

    Intake {
        item: TriageItem {
            id: meta.id,
            // The sender survives redaction deliberately. It is the one field a
            // human needs to recognize a proposal they cannot read the subject
            // of, and it is already personal rather than secret.
            from_addr: meta.from_addr,
            subject,
            snippet,
            internal_date_ms: meta.internal_date_ms,
            internal_date_text: None,
            stream,
            rationale,
            classification_method: content_item::METHOD_DETERMINISTIC.into(),
            classification_version: "mail-rules-v1".into(),
            data_class: classification.value,
            data_class_rationale: classification.rationale,
            data_classification_method: classification.method,
            data_classification_version: classification.version,
            status: "proposed".into(),
            gmail_action: None,
            gmail_action_at: None,
            purge_after: None,
            gmail_location: None,
            gmail_observed_at: None,
            gmail_sync_status: None,
            gmail_sync_action: None,
            // A freshly swept proposal is not waiting on anyone: the label is only
            // ever set by an explicit action, never inferred from a sweep.
            gmail_sync_error: None,
            waiting: false,
            waiting_since: None,
            first_seen: String::new(),
            last_seen: String::new(),
        },
        redactions,
    }
}

/// Redact an already-stored row's review fields. Returns `None` when the row's
/// class does not require redaction, so a caller can distinguish "left alone
/// on purpose" from "scanned and already clean".
pub fn remediate(
    data_class: &str,
    subject: Option<&str>,
    snippet: Option<&str>,
) -> Option<Remediation> {
    if !content_item::redact_before_persistence(data_class) {
        return None;
    }
    let mut redactions = Vec::new();
    let redacted_subject = redact_review_field(subject, &mut redactions);
    let redacted_snippet = redact_review_field(snippet, &mut redactions);
    Some(Remediation {
        changed: redacted_subject.as_deref() != subject || redacted_snippet.as_deref() != snippet,
        subject: redacted_subject,
        snippet: redacted_snippet,
        audit_digest: (!redactions.is_empty()).then(|| redaction_digest(&redactions)),
        redaction_count: redactions.iter().map(|finding| finding.count).sum(),
        redactions,
    })
}

#[derive(Debug, Clone)]
pub struct Remediation {
    pub subject: Option<String>,
    pub snippet: Option<String>,
    /// False when the pass ran and found nothing — which is the expected
    /// result on a second run, and is what makes remediation idempotent.
    pub changed: bool,
    pub audit_digest: Option<String>,
    pub redaction_count: usize,
    pub redactions: Vec<RedactionFinding>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(subject: &str, snippet: &str) -> ThreadMeta {
        ThreadMeta {
            id: "thread-1".into(),
            from_addr: Some("security@example.com".into()),
            subject: Some(subject.into()),
            date: None,
            list_unsubscribe: None,
            snippet: Some(snippet.into()),
            label_ids: Vec::new(),
            internal_date_ms: Some(1_754_000_000_000),
        }
    }

    /// The case that produced Axon#129: a live sweep stored a one-time code
    /// because it arrived in the subject line, where nothing was looking.
    #[test]
    fn a_one_time_code_never_reaches_the_stored_row() {
        let intake = from_thread(
            meta(
                "Your verification code is 448215",
                "Use 448215 to sign in, or open https://example.com/verify?t=AbCd1234567890Ef",
            ),
            &[],
        );

        assert_eq!(intake.item.data_class, "vault");
        let subject = intake.item.subject.clone().unwrap();
        let snippet = intake.item.snippet.clone().unwrap();
        assert!(!subject.contains("448215"), "the code survived: {subject}");
        assert!(!snippet.contains("448215"), "the code survived: {snippet}");
        assert!(!snippet.contains("AbCd1234567890Ef"), "token survived");
        assert!(subject.contains("[number]"));
        assert!(snippet.contains("[link]"));
        assert!(intake.redaction_count() >= 3);
        assert!(intake.audit_digest().is_some());
    }

    /// Redaction is scoped by class, not applied to everything: an ordinary
    /// mail stays readable, or the review list becomes useless and gets
    /// ignored, which is its own safety failure.
    #[test]
    fn ordinary_mail_is_stored_verbatim() {
        let intake = from_thread(
            meta("Lunch on Tuesday?", "Half twelve at the usual place"),
            &[],
        );
        assert_eq!(intake.item.data_class, "personal");
        assert_eq!(intake.item.subject.as_deref(), Some("Lunch on Tuesday?"));
        assert_eq!(
            intake.item.snippet.as_deref(),
            Some("Half twelve at the usual place")
        );
        assert!(intake.redactions.is_empty());
        assert!(intake.audit_digest().is_none());
    }

    /// The sender is what makes a redacted proposal reviewable at all.
    #[test]
    fn the_sender_survives_redaction() {
        let intake = from_thread(meta("Security alert: new sign in", "Code 998877"), &[]);
        assert_eq!(
            intake.item.from_addr.as_deref(),
            Some("security@example.com")
        );
    }

    #[test]
    fn remediation_is_idempotent_and_reports_the_second_pass_as_clean() {
        let first = remediate("vault", Some("Your code is 448215"), Some("expires soon"))
            .expect("vault rows are in scope");
        assert!(first.changed);
        assert!(first.audit_digest.is_some());

        let second = remediate("vault", first.subject.as_deref(), first.snippet.as_deref())
            .expect("vault rows are in scope");
        assert!(!second.changed, "a second pass must find nothing to remove");
        assert_eq!(second.subject, first.subject);
    }

    #[test]
    fn remediation_leaves_rows_it_does_not_own_alone() {
        assert!(remediate("personal", Some("Lunch on Tuesday?"), None).is_none());
        assert!(remediate("public", Some("Release notes"), None).is_none());
    }

    /// Two passes that removed the same shapes must agree, or the digest is
    /// not evidence of anything.
    #[test]
    fn the_audit_digest_is_stable_for_the_same_findings() {
        let one = remediate("vault", Some("code 123456"), None).unwrap();
        let two = remediate("vault", Some("code 654321"), None).unwrap();
        assert_eq!(one.audit_digest, two.audit_digest);
        assert!(one.audit_digest.is_some());
    }
}
