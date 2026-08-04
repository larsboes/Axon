//! Local preparation of bounded documents for the reviewed cloud-processing queue.
//! Nothing in this module performs network I/O. A preview must be reviewed and
//! its exact hash approved before the derivative can be staged in the store.

use serde::Serialize;
use sha2::{Digest, Sha256};

pub const PREVIEW_SCHEMA_VERSION: &str = "cloud-derivative-preview-v1";
pub const REDACTION_VERSION: &str = "deterministic-entity-redaction-v2";
pub const PASSTHROUGH_VERSION: &str = "bounded-public-v1";
const MAX_DOCUMENT_CHARS: usize = 16_000;

#[derive(Debug, Clone)]
pub struct CloudDocumentInput {
    pub source: String,
    pub id: String,
    pub title: Option<String>,
    pub author: Option<String>,
    pub summary: Option<String>,
    pub content: Option<String>,
    pub data_class: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RedactionFinding {
    pub entity_type: &'static str,
    pub marker: &'static str,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CloudDerivativePreview {
    pub schema_version: &'static str,
    pub source: String,
    pub id: String,
    pub source_revision: String,
    pub preview_hash: String,
    pub original_data_class: String,
    pub derivative_data_class: String,
    pub transformation: &'static str,
    pub document: String,
    pub redaction_count: usize,
    pub redactions: Vec<RedactionFinding>,
    pub entity_detection: &'static str,
    pub truncated: bool,
    pub approval_required: bool,
    pub provider_calls: u8,
    pub limitations: Vec<&'static str>,
}

pub fn prepare(input: &CloudDocumentInput) -> CloudDerivativePreview {
    let source_revision = source_revision(input);
    let needs_redaction = input.data_class != "public";
    let transformation = if needs_redaction {
        REDACTION_VERSION
    } else {
        PASSTHROUGH_VERSION
    };
    let derivative_data_class = if needs_redaction {
        "personal"
    } else {
        "public"
    };

    let mut redactions = Vec::new();
    let mut sections = Vec::new();
    if let Some(title) = input
        .title
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        let value = transform_text(title, needs_redaction, &mut redactions);
        sections.push(format!("Title\n{value}"));
    }
    if !needs_redaction {
        if let Some(author) = input
            .author
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            sections.push(format!("Author\n{}", author.trim()));
        }
    } else if input
        .author
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
    {
        record_redaction(&mut redactions, "identity", "[identity removed]");
        sections.push("Author\n[identity removed]".into());
    }
    if let Some(summary) = input
        .summary
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        let value = transform_text(summary, needs_redaction, &mut redactions);
        sections.push(format!("Summary\n{value}"));
    }
    if let Some(content) = input
        .content
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        let value = transform_text(content, needs_redaction, &mut redactions);
        sections.push(format!("Source content\n{value}"));
    }

    let unbounded_document = sections.join("\n\n");
    let (document, truncated) = bounded_chars(&unbounded_document, MAX_DOCUMENT_CHARS);
    let preview_hash = digest(&[
        PREVIEW_SCHEMA_VERSION,
        &source_revision,
        transformation,
        derivative_data_class,
        &document,
    ]);

    let mut limitations = vec![
        "Only the bounded reader document is included; attachments and linked pages are excluded.",
    ];
    if needs_redaction {
        limitations.push(
            "Local deterministic entity detection removes recognized people after salutations, email addresses, links, phone or account numbers, and token-like secrets; unrecognized names and contextual clues may remain.",
        );
        limitations.push("Human review is required before this derivative becomes cloud-eligible.");
    } else {
        limitations.push("Public classification permits cloud use but does not select a provider or send the document.");
    }

    CloudDerivativePreview {
        schema_version: PREVIEW_SCHEMA_VERSION,
        source: input.source.clone(),
        id: input.id.clone(),
        source_revision,
        preview_hash,
        original_data_class: input.data_class.clone(),
        derivative_data_class: derivative_data_class.into(),
        transformation,
        document,
        redaction_count: redactions.iter().map(|finding| finding.count).sum(),
        redactions,
        entity_detection: if needs_redaction {
            "local-deterministic-v2"
        } else {
            "not-required"
        },
        truncated,
        approval_required: true,
        provider_calls: 0,
        limitations,
    }
}

pub fn source_revision(input: &CloudDocumentInput) -> String {
    digest(&[
        &input.source,
        &input.id,
        input.title.as_deref().unwrap_or_default(),
        input.author.as_deref().unwrap_or_default(),
        input.summary.as_deref().unwrap_or_default(),
        input.content.as_deref().unwrap_or_default(),
        &input.data_class,
    ])
}

fn transform_text(value: &str, redact: bool, redactions: &mut Vec<RedactionFinding>) -> String {
    if !redact {
        return value.trim().to_string();
    }

    let tokens = value.split_whitespace().collect::<Vec<_>>();
    let mut output = Vec::with_capacity(tokens.len());
    let mut redact_next_person = false;
    let mut redact_person_tail = false;

    for token in tokens {
        let lowered = token.to_ascii_lowercase();
        let finding = if looks_like_url(&lowered) {
            Some(("link", "[link]"))
        } else if looks_like_email(token) {
            Some(("email", "[email]"))
        } else if looks_like_iban(token) {
            Some(("financial_identifier", "[account]"))
        } else if looks_like_phone(token) {
            Some(("phone_number", "[phone]"))
        } else if looks_like_token(token) {
            Some(("secret_token", "[token]"))
        } else if looks_like_sensitive_number(token) {
            Some(("long_number", "[number]"))
        } else if (redact_next_person || redact_person_tail) && looks_like_person_name(token) {
            Some(("person", "[person]"))
        } else {
            None
        };

        if let Some((entity_type, marker)) = finding {
            record_redaction(redactions, entity_type, marker);
            output.push(marker);
            redact_person_tail = entity_type == "person";
            redact_next_person = false;
        } else {
            output.push(token);
            redact_person_tail = false;
            redact_next_person = is_salutation(token);
        }
    }

    output.join(" ")
}

fn record_redaction(
    redactions: &mut Vec<RedactionFinding>,
    entity_type: &'static str,
    marker: &'static str,
) {
    if let Some(finding) = redactions
        .iter_mut()
        .find(|finding| finding.entity_type == entity_type)
    {
        finding.count += 1;
    } else {
        redactions.push(RedactionFinding {
            entity_type,
            marker,
            count: 1,
        });
    }
}

fn looks_like_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://") || value.starts_with("www.")
}

fn looks_like_email(value: &str) -> bool {
    let trimmed = value.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '@' && c != '.');
    let Some((local, domain)) = trimmed.split_once('@') else {
        return false;
    };
    !local.is_empty() && domain.contains('.')
}

fn looks_like_iban(value: &str) -> bool {
    let cleaned = value
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>();
    (15..=34).contains(&cleaned.len())
        && cleaned.chars().take(2).all(|c| c.is_ascii_alphabetic())
        && cleaned.chars().skip(2).take(2).all(|c| c.is_ascii_digit())
        && cleaned.chars().skip(4).all(|c| c.is_ascii_alphanumeric())
}

fn looks_like_phone(value: &str) -> bool {
    let digit_count = value.chars().filter(char::is_ascii_digit).count();
    digit_count >= 7
        && (value.trim_start().starts_with('+')
            || value.contains('-')
            || value.contains('(')
            || value.contains(')'))
}

fn looks_like_sensitive_number(value: &str) -> bool {
    value.chars().filter(char::is_ascii_digit).count() >= 6
}

fn looks_like_token(value: &str) -> bool {
    let cleaned: String = value.chars().filter(char::is_ascii_alphanumeric).collect();
    cleaned.len() >= 16
        && cleaned.chars().any(|c| c.is_ascii_alphabetic())
        && cleaned.chars().any(|c| c.is_ascii_digit())
}

fn is_salutation(value: &str) -> bool {
    matches!(
        value
            .trim_matches(|c: char| !c.is_alphabetic())
            .to_lowercase()
            .as_str(),
        "dear" | "hello" | "hi" | "hallo" | "liebe" | "lieber"
    )
}

fn looks_like_person_name(value: &str) -> bool {
    let cleaned = value.trim_matches(|c: char| !c.is_alphabetic() && c != '-' && c != '\'');
    let mut chars = cleaned.chars();
    cleaned.chars().filter(|c| c.is_alphabetic()).count() >= 2
        && chars.next().is_some_and(char::is_uppercase)
        && chars.all(|c| c.is_alphabetic() || c == '-' || c == '\'')
}

fn bounded_chars(value: &str, limit: usize) -> (String, bool) {
    let mut chars = value.chars();
    let bounded: String = chars.by_ref().take(limit).collect();
    let truncated = chars.next().is_some();
    (bounded, truncated)
}

fn digest(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(data_class: &str) -> CloudDocumentInput {
        CloudDocumentInput {
            source: "mail".into(),
            id: "thread-1".into(),
            title: Some("Security code 123456".into()),
            author: Some("Alice <alice@example.com>".into()),
            summary: None,
            content: Some(
                "Open https://example.com/a with token AbCd1234567890Ef or email alice@example.com"
                    .into(),
            ),
            data_class: data_class.into(),
        }
    }

    #[test]
    fn personal_preview_redacts_obvious_identifiers_without_provider_calls() {
        let preview = prepare(&input("personal"));
        assert_eq!(preview.derivative_data_class, "personal");
        assert_eq!(preview.transformation, REDACTION_VERSION);
        assert!(preview.document.contains("[identity removed]"));
        assert!(preview.document.contains("[number]"));
        assert!(preview.document.contains("[link]"));
        assert!(preview.document.contains("[token]"));
        assert!(!preview.document.contains("alice@example.com"));
        assert_eq!(preview.entity_detection, "local-deterministic-v2");
        assert!(preview
            .redactions
            .iter()
            .any(|finding| finding.entity_type == "email"));
        assert_eq!(preview.provider_calls, 0);
    }

    #[test]
    fn public_preview_is_bounded_but_not_pseudonymized() {
        let preview = prepare(&input("public"));
        assert_eq!(preview.derivative_data_class, "public");
        assert_eq!(preview.transformation, PASSTHROUGH_VERSION);
        assert!(preview.document.contains("alice@example.com"));
        assert_eq!(preview.redaction_count, 0);
        assert!(preview.redactions.is_empty());
        assert_eq!(preview.entity_detection, "not-required");
    }

    #[test]
    fn preview_hash_changes_with_the_source() {
        let first = prepare(&input("personal"));
        let mut changed = input("personal");
        changed.content = Some("different".into());
        let second = prepare(&changed);
        assert_ne!(first.source_revision, second.source_revision);
        assert_ne!(first.preview_hash, second.preview_hash);
    }

    #[test]
    fn local_entity_detection_reports_people_phone_and_financial_identifiers() {
        let mut value = input("vault");
        value.content =
            Some("Hello Alice Example, call +49-170-1234567 or use DE89370400440532013000.".into());

        let preview = prepare(&value);
        assert!(preview.document.contains("Hello [person] [person]"));
        assert!(preview.document.contains("[phone]"));
        assert!(preview.document.contains("[account]"));
        assert!(preview
            .redactions
            .iter()
            .any(|finding| finding.entity_type == "person" && finding.count == 2));
    }
}
