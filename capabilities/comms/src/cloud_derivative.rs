//! Local preparation of bounded documents for the reviewed cloud-processing queue.
//! Nothing in this module performs network I/O. A preview must be reviewed and
//! its exact hash approved before the derivative can be staged in the store.
//!
//! Two rules live here, and they are the same rule read from both ends:
//! [`prepare`] decides what representation of an item may exist at all, and
//! [`tier_allows`] decides which provider tier may receive that representation.
//! `vault` gets `Err` from the first and `false` from the second — a value the
//! store's own CHECK no longer accepts either.

use serde::Serialize;
use sha2::{Digest, Sha256};

pub const PREVIEW_SCHEMA_VERSION: &str = "cloud-derivative-preview-v1";
pub const REDACTION_VERSION: &str = "deterministic-entity-redaction-v3";
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

impl CloudDocumentInput {
    /// The cloud input for one stored feed item.
    ///
    /// The dashboard's own path builds this from `ContentItemOut` (which for a
    /// feed row carries exactly these fields, unchanged), and the drain has a
    /// `FeedItem` in hand and no reason to assemble a wire contract first. The
    /// two must produce identical fields or the same item would hash to two
    /// different `preview_hash` values and each would call the other stale —
    /// `a_drain_and_the_reader_prepare_the_same_document` pins that.
    pub fn from_feed(item: &crate::store::FeedItem) -> Self {
        Self {
            source: "feed".into(),
            id: item.id.clone(),
            title: item.title.clone(),
            author: item.author.clone(),
            summary: item.summary.clone(),
            content: item.transcript.clone(),
            data_class: item.data_class.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RedactionFinding {
    pub entity_type: &'static str,
    pub marker: &'static str,
    pub count: usize,
}

/// A `vault` document has no cloud preview.
///
/// Not a stricter preview, not a preview flagged ineligible: none at all. A
/// preview is a hashable, approvable object — the whole point of it is that a
/// human can sign the exact bytes and a queue can pin them. Producing one for
/// content that may never leave the machine means the refusal has to be
/// remembered again at every later step, and it was: staging checked for
/// `vault`, queueing did not, and the preview handler happily returned the
/// document itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VaultRefused;

impl std::fmt::Display for VaultRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("vault content has no cloud derivative and cannot be prepared for one")
    }
}

impl std::error::Error for VaultRefused {}

/// Whether a provider tier admits this exact original-plus-representation pair.
///
/// The home of the rule, rather than the cloud handlers that first needed it.
/// It is consulted from two directions now — the reviewed-derivative queue in
/// `capabilities/comms/src/server/cloud.rs`, and the digest path in
/// `capabilities/comms/src/digest.rs`, which sends the source text as it stands
/// and so asks about the passthrough representation. The digest path used to
/// answer the question itself with `data_class == "public"`, a second copy of a
/// policy that already lived here and was free to drift from it.
///
/// Every branch names the transformation as well as the classes: a tier that
/// accepts pseudonymized Personal accepts the *redacted* derivative, and the
/// same Personal document sent verbatim is a different question with a
/// different answer.
pub fn tier_allows(
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
        && transformation == PASSTHROUGH_VERSION;
    match tier {
        Some("public") => public_derivative,
        Some("pseudonymized_personal") => {
            public_derivative
                || (original_data_class == "personal"
                    && derivative_data_class == "personal"
                    && transformation == REDACTION_VERSION)
        }
        _ => false,
    }
}

/// Whether an item of this stored class may be sent to a provider tier **as it
/// stands** — no redaction, no derivative, the source text itself.
///
/// The question both prefill paths ask, because both send exactly that: the
/// digest ladder in `digest.rs` and the feed-summary drain in `media.rs`. Naming
/// it once means neither of them spells out the passthrough argument itself, and
/// there is one place to read to know what "verbatim" is allowed to mean.
///
/// The answer is `public` on a declared tier and nothing else. `personal` has a
/// cloud lane, but it runs through [`prepare`] and the redaction transformation;
/// a personal item handed over unchanged is a different question with a
/// different answer.
pub fn verbatim_send_allowed(cloud_data_tier: Option<&str>, data_class: &str) -> bool {
    tier_allows(cloud_data_tier, data_class, data_class, PASSTHROUGH_VERSION)
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

/// Build the reviewable, hashable derivative for one stored item.
///
/// `Err(VaultRefused)` for `vault`: there is no representation of Private
/// content this module is willing to hand to an approval flow, so the refusal
/// is the return type rather than a flag on an object that already exists. The
/// redaction path below therefore only ever describes `personal` — `vault` used
/// to fall through it and produce a fully approvable preview whose only
/// protection was that three separate later call sites remembered to look at
/// `original_data_class` again.
pub fn prepare(input: &CloudDocumentInput) -> Result<CloudDerivativePreview, VaultRefused> {
    if input.data_class == "vault" {
        return Err(VaultRefused);
    }
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
            "Local deterministic entity detection removes recognized people after a salutation or a self-introduction, a person named as being from an organisation, login handles, email addresses, links, phone or account numbers, and token-like secrets; unrecognized names and contextual clues may remain.",
        );
        limitations.push("Human review is required before this derivative becomes cloud-eligible.");
    } else {
        limitations.push("Public classification permits cloud use but does not select a provider or send the document.");
    }

    Ok(CloudDerivativePreview {
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
            "local-deterministic-v3"
        } else {
            "not-required"
        },
        truncated,
        approval_required: true,
        provider_calls: 0,
        limitations,
    })
}

/// Run one stored review field — a subject, a snippet — through the same
/// deterministic entity detection the cloud preview uses.
///
/// Exposed separately because the cloud boundary is not the first boundary.
/// A `vault` mail's subject line can itself be the secret, so this also runs
/// before the row is written (see `intake`), and again when an already-stored
/// row is remediated. Same detector, same version string, three call sites —
/// a second implementation is how the two drift apart.
pub fn redact_review_field(
    value: Option<&str>,
    redactions: &mut Vec<RedactionFinding>,
) -> Option<String> {
    let value = value?;
    if value.trim().is_empty() {
        return Some(value.to_string());
    }
    Some(transform_text(value, true, redactions))
}

/// Stable identifier for what a redaction pass did, without carrying any of
/// the material it removed. Safe to log, store and show.
pub fn redaction_digest(findings: &[RedactionFinding]) -> String {
    let mut parts: Vec<String> = findings
        .iter()
        .map(|finding| format!("{}:{}", finding.entity_type, finding.count))
        .collect();
    parts.sort();
    let joined = parts.join(",");
    digest(&[REDACTION_VERSION, &joined])
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
    let mut redact_person_tail = false;

    for (index, token) in tokens.iter().copied().enumerate() {
        let lowered = token.to_ascii_lowercase();
        // A cue reads backwards over the token slice rather than carrying a flag
        // forward, because "I am X" and "my name is X" are two tokens wide and a
        // one-token flag cannot see the word before the one that set it.
        let cued = redact_person_tail || introduces_person(&tokens, index);
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
        } else if crate::people_registry::is_known_person(token) {
            // Rung 0. The salutation gate below cannot see a bare first name, and
            // across 353 Journal notes it caught 2 of 2,802 known-person mentions.
            // A list the operator wrote by hand beats a heuristic here, so it is
            // consulted first.
            Some(("person", "[person]"))
        } else if names_a_person_in_apposition(&tokens, index) {
            Some(("person", "[person]"))
        } else if cued && (looks_like_person_name(token) || looks_like_handle(token)) {
            // A handle is only a person behind a cue. Ungated, `looks_like_handle`
            // fires on every "iPhone15" and order code in the corpus, which is the
            // over-redaction the Presidio trial rejected a whole runtime for.
            Some(("person", "[person]"))
        } else {
            None
        };

        if let Some((entity_type, marker)) = finding {
            record_redaction(redactions, entity_type, marker);
            output.push(marker);
            redact_person_tail = entity_type == "person";
        } else {
            output.push(token);
            redact_person_tail = false;
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

/// One token reduced to the word a cue test compares against: entities decoded,
/// lowercased, surrounding punctuation dropped.
///
/// The decode is not decoration. Stored mail reaches this module still holding
/// `&#39;`, so the token that carries the "I'm X" cue is literally `I&#39;m`, and
/// a matcher written against `i'm` would never fire on the corpus it was written
/// for. `extraction::decode_basic_entities` already owns that table; a second
/// copy here is how the two drift apart.
///
/// The apostrophe survives the trim on purpose. It is the only thing separating
/// the English cue `i'm` from the German preposition `im`, which precedes a
/// capitalised noun in a language that capitalises every noun.
fn cue_word(value: &str) -> String {
    crate::extraction::decode_basic_entities(value)
        .to_lowercase()
        .trim_matches(|c: char| !c.is_alphanumeric() && c != '\'')
        .to_string()
}

/// Whether the tokens before `index` announce that the next word names a person.
///
/// Two families, and the second one is PRD D14. A salutation gate sees `Dear X`
/// and nothing else; the shadow evaluation
/// (`<overlay>/config/comms-redaction-shadow.md`, 2026-08-30) found three of
/// Presidio's four unique catches were English self-introductions mid-body, where
/// there is no salutation to fire on.
///
/// German self-introduction (`ich bin X`, `mein Name ist X`) is deliberately
/// absent. No labelled miss asked for it — that measurement put German person
/// recall *above* English — and German capitalises every noun, so `ich bin Ihr
/// Ansprechpartner` would redact a pronoun. The rule is added when a miss asks
/// for it, not before.
fn introduces_person(tokens: &[&str], index: usize) -> bool {
    let word = |back: usize| index.checked_sub(back).map(|i| cue_word(tokens[i]));
    let Some(previous) = word(1) else {
        return false;
    };
    match previous.as_str() {
        "dear" | "hello" | "hi" | "hallo" | "liebe" | "lieber" => true,
        "i'm" => true,
        "am" => word(2).as_deref() == Some("i"),
        "is" => matches!(word(2).as_deref(), Some("this") | Some("name")),
        _ => false,
    }
}

/// Whether the token at `index` is a person named by the company they are from.
///
/// PRD D14's first gap, and the one with a price. `X from Y` is a person only
/// when `Y` is a proper noun too — the shape of "co-hosted by Rayn from
/// Scriptbee", not of "Regards from Berlin", where the first word is a common
/// noun that happens to start a line. Nothing here can tell those apart by
/// vocabulary, so the rule is kept to the narrow shape and the cost is measured
/// rather than argued: it moves the marker count over the whole personal mail
/// body by a number recorded in `<overlay>/config/comms-redaction-shadow.md`.
///
/// German is untouched by construction. `from` is not a German word, and the
/// corpus's German half already scores full person recall without this.
fn names_a_person_in_apposition(tokens: &[&str], index: usize) -> bool {
    looks_like_person_name(tokens[index])
        && tokens
            .get(index + 1)
            .is_some_and(|next| cue_word(next) == "from")
        && tokens
            .get(index + 2)
            .is_some_and(|after| looks_like_person_name(after))
}

/// A login handle: letters and digits fused into one opaque word.
///
/// PRD D14's second gap. `labo2764` identifies its owner as surely as the name
/// on the account, and it passes every other recognizer here — too short for
/// `looks_like_token`, too few digits for `looks_like_sensitive_number`, and
/// lowercase, so `looks_like_person_name` refuses it. Only reachable behind a
/// person cue; see the call site.
fn looks_like_handle(value: &str) -> bool {
    let cleaned = value.trim_matches(|c: char| !c.is_ascii_alphanumeric());
    let letters = cleaned.chars().filter(char::is_ascii_alphabetic).count();
    let digits = cleaned.chars().filter(char::is_ascii_digit).count();
    (4..=32).contains(&cleaned.len())
        && cleaned.chars().all(|c| c.is_ascii_alphanumeric())
        && letters >= 2
        && digits >= 2
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
        let preview = prepare(&input("personal")).unwrap();
        assert_eq!(preview.derivative_data_class, "personal");
        assert_eq!(preview.transformation, REDACTION_VERSION);
        assert!(preview.document.contains("[identity removed]"));
        assert!(preview.document.contains("[number]"));
        assert!(preview.document.contains("[link]"));
        assert!(preview.document.contains("[token]"));
        assert!(!preview.document.contains("alice@example.com"));
        assert_eq!(preview.entity_detection, "local-deterministic-v3");
        assert!(preview
            .redactions
            .iter()
            .any(|finding| finding.entity_type == "email"));
        assert_eq!(preview.provider_calls, 0);
    }

    #[test]
    fn public_preview_is_bounded_but_not_pseudonymized() {
        let preview = prepare(&input("public")).unwrap();
        assert_eq!(preview.derivative_data_class, "public");
        assert_eq!(preview.transformation, PASSTHROUGH_VERSION);
        assert!(preview.document.contains("alice@example.com"));
        assert_eq!(preview.redaction_count, 0);
        assert!(preview.redactions.is_empty());
        assert_eq!(preview.entity_detection, "not-required");
    }

    #[test]
    fn preview_hash_changes_with_the_source() {
        let first = prepare(&input("personal")).unwrap();
        let mut changed = input("personal");
        changed.content = Some("different".into());
        let second = prepare(&changed).unwrap();
        assert_ne!(first.source_revision, second.source_revision);
        assert_ne!(first.preview_hash, second.preview_hash);
    }

    #[test]
    fn local_entity_detection_reports_people_phone_and_financial_identifiers() {
        let mut value = input("personal");
        value.content =
            Some("Hello Alice Example, call +49-170-1234567 or use DE89370400440532013000.".into());

        let preview = prepare(&value).unwrap();
        assert!(preview.document.contains("Hello [person] [person]"));
        assert!(preview.document.contains("[phone]"));
        assert!(preview.document.contains("[account]"));
        assert!(preview
            .redactions
            .iter()
            .any(|finding| finding.entity_type == "person" && finding.count == 2));
    }

    /// PRD D14, gap one. Three of the four labels Presidio caught and the
    /// incumbent missed were English self-introductions in the body of a mail,
    /// where a salutation-only gate has nothing to fire on. The entity form is
    /// the one the corpus actually holds: stored mail reaches this module with
    /// `&#39;` unresolved, so a matcher written against a bare apostrophe would
    /// pass this test and still miss every real mail.
    #[test]
    fn a_self_introduction_names_a_person_without_any_salutation() {
        for phrasing in [
            "Thanks for signing up. I&#39;m Josh, one of the co-founders.",
            "Thanks for signing up. I'm Josh, one of the co-founders.",
            "Thanks for signing up. I am Josh, one of the co-founders.",
            "Thanks for signing up. My name is Josh, one of the co-founders.",
            "Thanks for signing up. This is Josh, one of the co-founders.",
        ] {
            let mut value = input("personal");
            value.content = Some(phrasing.into());
            let preview = prepare(&value).unwrap();
            assert!(
                !preview.document.contains("Josh"),
                "self-introduction leaked the name: {phrasing}"
            );
        }
    }

    /// The negative that pays for the rule above. German `im` reduces to the
    /// same letters as the English cue `i'm`, and German capitalises every noun,
    /// so a cue test that dropped the apostrophe would redact a noun after every
    /// `im` in half the corpus. The corpus's German person recall was already the
    /// higher of the two; this rule must not spend that.
    #[test]
    fn the_german_preposition_im_is_not_a_self_introduction() {
        let mut value = input("personal");
        value.content = Some("Die Unterlagen finden Sie im Anhang dieser Nachricht.".into());

        let preview = prepare(&value).unwrap();
        assert!(preview.document.contains("im Anhang"));
    }

    /// PRD D14, gap two. `labo2764` passes every other recognizer here — too
    /// short for `looks_like_token`, too few digits for
    /// `looks_like_sensitive_number`, lowercase so not a name — and it identifies
    /// its owner as surely as the name on the account.
    #[test]
    fn a_login_handle_after_a_salutation_is_a_person() {
        let mut value = input("personal");
        value.content = Some("Hallo labo2764, wir haben eine neue Login-Aktivitaet.".into());

        let preview = prepare(&value).unwrap();
        assert!(!preview.document.contains("labo2764"));
        assert!(preview
            .redactions
            .iter()
            .any(|finding| finding.entity_type == "person"));
    }

    /// The handle rule is reachable only behind a cue. Ungated it fires on every
    /// product name and order code in the corpus, which is the over-redaction the
    /// Presidio trial rejected a whole Python runtime for.
    #[test]
    fn a_handle_shaped_word_with_no_cue_before_it_stays() {
        let mut value = input("personal");
        value.content = Some("Your order for the iPhone15 ships on Tuesday.".into());

        let preview = prepare(&value).unwrap();
        assert!(preview.document.contains("iPhone15"));
    }

    /// PRD D14, gap one again, in its other shape: a person named by the
    /// organisation they are from, mid-sentence, with no cue in front.
    #[test]
    fn a_person_named_as_being_from_an_organisation_is_redacted() {
        let mut value = input("personal");
        value.content = Some("A workshop co-hosted by Rayn from Scriptbee this Thursday.".into());

        let preview = prepare(&value).unwrap();
        assert!(!preview.document.contains("Rayn"));
        assert!(preview.document.contains("[person] from"));
    }

    /// The apposition needs a proper noun on both sides. `from` followed by a
    /// lowercase word is the ordinary preposition and must cost nothing.
    #[test]
    fn from_followed_by_a_common_noun_is_not_an_apposition() {
        let mut value = input("personal");
        value.content = Some("Download the report from our website before Friday.".into());

        let preview = prepare(&value).unwrap();
        assert!(preview
            .document
            .contains("Download the report from our website"));
    }

    /// This test used to prepare a `vault` document and assert on the redacted
    /// preview it got back. That preview was a real, hashable, approvable
    /// object carrying the document itself; only the fact that three later call
    /// sites re-checked `original_data_class` kept it out of a cloud request.
    /// The refusal is the return type now, so there is nothing to approve.
    #[test]
    fn vault_content_has_no_approvable_preview_at_all() {
        assert_eq!(prepare(&input("vault")), Err(VaultRefused));

        let mut with_secrets = input("vault");
        with_secrets.content =
            Some("Hello Alice Example, call +49-170-1234567 or use DE89370400440532013000.".into());
        assert_eq!(prepare(&with_secrets), Err(VaultRefused));
    }

    #[test]
    fn cloud_tiers_accept_only_the_exact_reviewed_representation() {
        assert!(tier_allows(
            Some("public"),
            "public",
            "public",
            PASSTHROUGH_VERSION,
        ));
        assert!(!tier_allows(
            Some("public"),
            "personal",
            "personal",
            REDACTION_VERSION,
        ));
        assert!(tier_allows(
            Some("pseudonymized_personal"),
            "personal",
            "personal",
            REDACTION_VERSION,
        ));
        assert!(!tier_allows(
            Some("pseudonymized_personal"),
            "personal",
            "personal",
            PASSTHROUGH_VERSION,
        ));
        assert!(!tier_allows(
            Some("pseudonymized_personal"),
            "vault",
            "personal",
            REDACTION_VERSION,
        ));
    }

    /// The property both prefill paths depend on — `digest.rs` and the feed
    /// summary drain in `media.rs` — because both hand the model the item's own
    /// text with nothing removed. No configured tier admits that for anything
    /// but `public`, and a role with no declared tier admits nothing at all:
    /// that covers every local role, and every https endpoint somebody points a
    /// summarization role at without a reviewed cloud policy on it.
    #[test]
    fn nothing_but_public_may_be_sent_verbatim_and_an_undeclared_tier_sends_nothing() {
        for tier in [None, Some("public"), Some("pseudonymized_personal")] {
            for class in ["personal", "vault", "something-new"] {
                assert!(
                    !verbatim_send_allowed(tier, class),
                    "{tier:?} admitted {class} verbatim"
                );
            }
        }
        assert!(!verbatim_send_allowed(None, "public"));
        assert!(verbatim_send_allowed(Some("public"), "public"));
        assert!(verbatim_send_allowed(
            Some("pseudonymized_personal"),
            "public"
        ));
    }

    /// A personal item has a cloud lane, and it is not this one. Worth its own
    /// assertion because the pseudonymized tier's whole purpose is personal
    /// content, and reading the tier name alone would suggest it applies here.
    #[test]
    fn a_personal_item_reaches_cloud_only_as_a_redacted_derivative() {
        assert!(!verbatim_send_allowed(
            Some("pseudonymized_personal"),
            "personal"
        ));
        assert!(tier_allows(
            Some("pseudonymized_personal"),
            "personal",
            "personal",
            REDACTION_VERSION
        ));
    }
}
