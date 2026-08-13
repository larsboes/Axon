//! `content-item-v1` — the canonical reader contract, in Rust.
//!
//! One shape for every kind of observed thing: a feed article, a mail, a
//! calendar entry. Source adapters own collection, storage and actions; the
//! dashboard owns **one** renderer for this contract. `schemas/content-item.schema.json`
//! is the normative artifact — this file exists so the capabilities that emit
//! it cannot drift from each other by hand.
//!
//! ## What this lib is not
//!
//! It is not a unified *storage* model. Each capability keeps its own tables
//! and its own invariants — calendar's exclusive `ends_at` and `(source,
//! external_id)` uniqueness, mail's retention window — because those are
//! genuinely different constraints and a merged table could not enforce any of
//! them. This is a **projection** the stores render themselves into on read.
//!
//! ## Ranking belongs to the source, not to the contract
//!
//! `relevance` and `evaluation` exist because feed is an unbounded inbox that
//! has to be ranked. A calendar entry is something the operator already decided
//! about — its triage axis is `commitment`, surfaced through `status`. A source
//! with no ranking leaves these empty rather than inventing a score; a `0.0`
//! sitting on a committed event is noise that reads as a judgement.
//!
//! Consumers link this as an ordinary crate. Capability boundaries still use
//! the serialized contract rather than reaching into another capability's
//! storage or implementation.

use serde::Serialize;
use serde_json::Value;

/// Bump only with the schema's `schema_version` const, and only for a change
/// readers cannot absorb — adding an optional field is not one.
pub const SCHEMA_VERSION: &str = "content-item-v1";

/// One reader shape for every kind of observed content.
///
/// Every field is always present. The schema is `additionalProperties: false`
/// with everything required, so absent data is an explicit `null` or an empty
/// collection — never a missing key a reader has to probe for.
#[derive(Debug, Clone, Serialize)]
pub struct ContentItem {
    pub schema_version: &'static str,
    pub source: &'static str,
    pub id: String,
    pub kind: String,
    pub title: Option<String>,
    pub url: String,
    pub author: Option<String>,
    pub summary: Option<String>,
    pub content: Option<String>,
    pub content_label: String,
    pub day: String,
    pub created_at: String,
    pub status: String,
    pub content_status: &'static str,
    pub data_class: DataClass,
    pub processing_policy: ProcessingPolicy,
    pub cloud_processing: CloudProcessing,
    pub relevance: Vec<Relevance>,
    pub evaluation: Option<Evaluation>,
    pub processing: Vec<Processing>,
    pub origins: Vec<Origin>,
    pub links: Vec<Link>,
    pub digest: Option<Digest>,
    pub mail: Option<MailExtension>,
    pub calendar: Option<CalendarExtension>,
}

/// What the local model wrote about this thing, and what was asked of it.
///
/// Deliberately not `summary`. `summary` is what the *source* said it is —
/// calendar reads it from the entry's own description, and a generated
/// paragraph written over that destroys the only verbatim text an entry has. A
/// reader that wants "the short version" prefers `digest.text` and falls back to
/// `summary`; both being present is normal, not a conflict.
///
/// Every field is present so the reader never probes. `text` is null whenever
/// `state` is anything but `generated`, and `state` then says why — including
/// `skipped_short`, which is a verdict about the source rather than a failure.
#[derive(Debug, Clone, Serialize)]
pub struct Digest {
    pub text: Option<String>,
    /// `generated` · `skipped_short` · `remote_refused` · `unconfigured` ·
    /// `http_error` · `model_error` · `capacity_aborted` · `empty_response` ·
    /// `timeout`.
    pub state: String,
    /// The rung the ladder landed on: `none` · `brief` · `standard` · `sectioned`.
    pub shape: String,
    /// `standard` for the automatic pass, `detailed` when an operator asked for
    /// one rung more.
    pub depth: String,
    /// The operator's focus terms, as typed. Shown back to them so a
    /// differently-shaped digest is explained rather than mysterious.
    pub focus: Vec<String>,
    /// Backend, model and prompt revision. A change to any of the three makes
    /// this row legibly stale instead of silently mixed with newer ones.
    pub producer: String,
    /// How much source the ladder measured. Carrying it means a reader can see
    /// *why* a short item has no digest without re-deriving the length.
    pub source_chars: i64,
    /// How many entities the deterministic redactor removed before this text was
    /// written. Non-zero only for Private content, where the metadata is the
    /// payload and a digest could otherwise republish what the subject line was
    /// redacted for.
    pub redactions: i32,
    pub attempts: i32,
    pub last_error: Option<String>,
    /// Mermaid source, validated before it was stored — see `libs/summarize`.
    pub diagram: Option<String>,
    pub diagram_state: Option<String>,
    pub diagram_error: Option<String>,
    /// The chartable table pulled out of the source, as `chart-data` JSON, or
    /// null. Not a chart *spec*: the reader compiles one, so the model never
    /// reaches the rendering layer. Every value in it appeared verbatim in the
    /// source text before it was allowed in.
    pub chart: Option<Value>,
    /// `generated` · `skipped_short` (no comparable numbers, the answer for most
    /// prose) · a failure class.
    pub chart_state: Option<String>,
    pub chart_error: Option<String>,
    pub generated_at: String,
}

/// A named way out of this item: the source page, the mail that carried the
/// ticket, a map, a vault note.
///
/// The field every capability was about to invent separately. Deliberately not
/// typed as an enum of destinations — `kind` is a hint for the icon, and a
/// reader that meets an unknown one still renders a working link.
#[derive(Debug, Clone, Serialize)]
pub struct Link {
    pub label: String,
    pub kind: String,
    pub url: String,
}

impl Link {
    pub fn new(label: impl Into<String>, kind: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            kind: kind.into(),
            url: url.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DataClass {
    pub value: String,
    pub label: &'static str,
    pub rationale: String,
    pub method: String,
    pub version: String,
}

impl DataClass {
    /// `vault` is stored, `Private` is what the product calls it.
    pub fn new(
        value: impl Into<String>,
        rationale: impl Into<String>,
        method: impl Into<String>,
        version: impl Into<String>,
    ) -> Self {
        let value = value.into();
        let label = match value.as_str() {
            "vault" => "Private",
            "personal" => "Personal",
            _ => "Public",
        };
        Self {
            value,
            label,
            rationale: rationale.into(),
            method: method.into(),
            version: version.into(),
        }
    }

    /// Read a stored classification back out of a row.
    ///
    /// The label is re-derived rather than stored, so a row written before the
    /// product renamed `vault` to Private still reads back correctly.
    pub fn stored(value: &str, rationale: &str, method: &str, version: &str) -> Self {
        Self::new(value, rationale, method, version)
    }

    /// What an item nobody classified is worth: Personal, decided by nobody.
    ///
    /// The fail-closed default, and the reason it is a real value rather than a
    /// NULL. `legacy` is rank 0, so any later decision — a collector's, a
    /// rule's, a human's — outranks it without a special case for "unset".
    pub fn undeclared() -> Self {
        Self::new(
            "personal",
            UNDECLARED_RATIONALE,
            METHOD_LEGACY,
            LEGACY_CLASSIFIER_VERSION,
        )
    }

    /// A class a collector positively declared for everything it fetches.
    ///
    /// `public` is only reachable through here. A collector that declares
    /// nothing gets [`DataClass::undeclared`], which is Personal — that is the
    /// entire difference from the literal this replaced, which stamped `public`
    /// on every feed item whether or not anyone had ever looked at the source.
    pub fn declared_by_source(value: &str, rationale: &str) -> Self {
        Self::new(
            value,
            rationale,
            METHOD_DETERMINISTIC,
            SOURCE_CLASSIFIER_VERSION,
        )
    }

    /// The operator's own schedule. Where they are and when is personal by
    /// construction, whatever the event itself is — a public concert still
    /// tells you the building is empty that evening.
    pub fn personal_source_default(rationale: impl Into<String>) -> Self {
        Self::new(
            "personal",
            rationale,
            METHOD_DETERMINISTIC,
            SOURCE_CLASSIFIER_VERSION,
        )
    }

    /// A class a human set by hand, carrying the reason they gave for it.
    pub fn set_by_human(value: &str, rationale: &str) -> Self {
        Self::new(value, rationale, METHOD_HUMAN, MANUAL_CLASSIFIER_VERSION)
    }

    /// Classify a mail from the metadata a read-only sweep has already
    /// admitted: the stream its mail rules picked, the sender and the subject.
    ///
    /// The body is deliberately not a parameter. Classification decides whether
    /// a body may be fetched at all, so it must not depend on having one.
    ///
    /// Never returns `public`. A mailbox holds no public content — someone
    /// chose to write to *this* operator, and that choice is itself personal.
    pub fn classify_mail(stream: &str, from: &str, subject: &str) -> Self {
        let text = format!("{from} {subject}").to_ascii_lowercase();
        match mail_vault_reason(stream, &text) {
            Some(rationale) => Self::new(
                "vault",
                rationale,
                METHOD_DETERMINISTIC,
                MAIL_CLASSIFIER_VERSION,
            ),
            None => Self::new(
                "personal",
                "Mail metadata is Personal by default.",
                METHOD_DETERMINISTIC,
                MAIL_CLASSIFIER_VERSION,
            ),
        }
    }
}

/// The stored classes, in the order a reader should offer them. `vault` is the
/// stored value; `Private` is the word the product uses (see [`DataClass::new`]).
pub const DATA_CLASSES: [&str; 3] = ["public", "personal", "vault"];

/// Stamped on every rules-produced classification, so a stored row records
/// which rule set decided it. Bump with any change to [`mail_vault_reason`].
pub const MAIL_CLASSIFIER_VERSION: &str = "data-class-rules-v1";

/// Stamped by a collector that declared a class for what it fetches.
pub const SOURCE_CLASSIFIER_VERSION: &str = "data-class-source-v1";

/// Stamped on a row nobody ever classified, including every row that predates
/// the class existing on its table.
pub const LEGACY_CLASSIFIER_VERSION: &str = "data-class-legacy-v1";

/// Stamped when a human set the class by hand.
pub const MANUAL_CLASSIFIER_VERSION: &str = "manual-v1";

/// Why an undeclared item is Personal. Kept next to the DB DEFAULT that writes
/// the same sentence, because the two have to agree for a backfilled row and a
/// freshly ingested one to read alike.
pub const UNDECLARED_RATIONALE: &str =
    "No collector declared a class for this item; Personal by default.";

pub const METHOD_LEGACY: &str = "legacy";
pub const METHOD_DETERMINISTIC: &str = "deterministic";
pub const METHOD_MODEL: &str = "model";
pub const METHOD_HUMAN: &str = "human";

/// Who decided, in order. One vocabulary for every classification and every
/// processing stage in the contract — `comms::provenance::tier_rank` is this
/// function, and the DB CHECK constraints spell out this list.
///
/// The order is the point. Without a rank there is no way to say "a rule may
/// not overwrite what a human decided" except by enumerating pairs, and the
/// enumeration is what drifts.
pub const CLASSIFICATION_METHODS: [&str; 4] = [
    METHOD_LEGACY,
    METHOD_DETERMINISTIC,
    METHOD_MODEL,
    METHOD_HUMAN,
];

pub fn method_rank(method: &str) -> Option<i16> {
    match method {
        METHOD_LEGACY => Some(0),
        METHOD_DETERMINISTIC => Some(10),
        METHOD_MODEL => Some(20),
        METHOD_HUMAN => Some(30),
        _ => None,
    }
}

/// How exposed the content is, ascending: Public < Personal < Private.
///
/// A higher rank is a stricter class, so "escalate" is "raise the rank" and the
/// whole escalation-only rule is one comparison rather than a table of cases.
pub fn class_rank(data_class: &str) -> Option<i16> {
    match data_class {
        "public" => Some(0),
        "personal" => Some(10),
        "vault" => Some(20),
        _ => None,
    }
}

pub fn valid(data_class: &str) -> bool {
    DATA_CLASSES.contains(&data_class)
}

/// Whether a proposed reclassification may be written over the stored one.
///
/// The single home of the escalation rule, and the reason it is a function
/// rather than a check each caller performs: a gate every caller has to
/// remember to call is the shape that produced three disagreeing copies of the
/// cloud verdict already.
///
/// Escalation — proposing a stricter class — is always admitted, whoever
/// proposes it. De-escalation is admitted only for a human, and only with a
/// rationale they actually wrote. Both halves matter. A rule that can lower a
/// class is a declassification primitive: it runs over attacker-controlled feed
/// text, and "this looks like a public blog post" is a sentence an injected
/// page can arrange to be true of itself. And a de-escalation with no reason
/// recorded is indistinguishable afterwards from a mistake.
///
/// The third case is equality, and it is why the *stored* method is a parameter
/// rather than an implementation detail of whoever stored it. A write proposing
/// the class already on the row changes no class, so both rules above wave it
/// through — yet it still overwrites the method and the rationale. A machine
/// re-ingest arriving at `personal` on a row a human had classified `personal`
/// erased the human's record exactly that way: the class survived, the reason
/// it was chosen did not. So at equal class a non-human write over a stored
/// human decision writes nothing. There is no classification to make there,
/// only a record to lose.
///
/// Deliberately narrow: `human` is protected this way, method rank in general
/// is not. A rules resweep re-deciding at the same class is the classifier
/// doing its job and has to keep landing; only what a person wrote is
/// unreproducible.
pub fn admit_reclassification(
    stored_class: &str,
    stored_method: &str,
    proposed_class: &str,
    proposed_method: &str,
    rationale: &str,
) -> Result<(), &'static str> {
    let Some(stored_rank) = class_rank(stored_class) else {
        return Err("stored data class is not a known class");
    };
    let Some(proposed_rank) = class_rank(proposed_class) else {
        return Err("data class must be one of: public, personal, vault");
    };
    if method_rank(stored_method).is_none() {
        return Err("stored classification method is not a known method");
    }
    if method_rank(proposed_method).is_none() {
        return Err("classification method must be one of: legacy, deterministic, model, human");
    }
    if proposed_rank > stored_rank {
        return Ok(());
    }
    if proposed_rank == stored_rank {
        if stored_method == METHOD_HUMAN && proposed_method != METHOD_HUMAN {
            return Err("only a human may replace a human's classification at the same class");
        }
        return Ok(());
    }
    if proposed_method != METHOD_HUMAN {
        return Err("only a human may lower a data class");
    }
    if rationale.trim().is_empty() {
        return Err("lowering a data class requires a written rationale");
    }
    Ok(())
}

/// Whether a stored *review representation* of this class must be redacted
/// before it is persisted.
///
/// The distinction that matters: for every other class the sensitive material
/// is in the body, and keeping bodies transient is enough. For `vault` the
/// metadata is the payload — a one-time code arrives in the subject line, and
/// storing that subject verbatim puts it in a log, an API response and a
/// dashboard at once.
pub fn redact_before_persistence(data_class: &str) -> bool {
    data_class == "vault"
}

/// Why a mail's metadata alone is enough to call it Private, or `None` for the
/// personal default.
///
/// Conservative on purpose: a false `vault` costs a redacted subject line in a
/// review list, a false `personal` costs a leaked credential.
fn mail_vault_reason(stream: &str, lowercased_text: &str) -> Option<&'static str> {
    if stream == "steuern" {
        return Some("Tax-related mail is Private by default.");
    }
    if stream == "belege" {
        return Some("Receipts and invoices are Private by default.");
    }
    const AUTHENTICATION: [&str; 22] = [
        "verification code",
        "security code",
        "security alert",
        "account alert",
        "one-time code",
        "one-time access token",
        "access token",
        "recovery code",
        "new sign in",
        "new sign-in",
        "new login",
        "trusted device",
        "suspicious activity",
        "magic link",
        "secure mail",
        "securemail",
        "vertraulich",
        "bestätigungscode",
        "sicherheitscode",
        "einmalcode",
        "passwort",
        "password",
    ];
    const FINANCIAL: [&str; 8] = [
        "bank statement",
        "kontoauszug",
        "rechnung",
        "invoice",
        "payment",
        "zahlung",
        "insurance",
        "versicherung",
    ];
    const HEALTH: [&str; 7] = [
        "diagnosis",
        "diagnose",
        "prescription",
        "rezept",
        "medical result",
        "befund",
        "krankenversicherung",
    ];

    // `2fa` and `otp` are matched as whole words: `otp` alone also sits inside
    // ordinary German words, and a substring match there classified unrelated
    // mail as Private.
    let bounded_token = lowercased_text
        .split(|c: char| !c.is_alphanumeric())
        .any(|word| word == "2fa" || word == "otp");

    if bounded_token || contains_any(lowercased_text, &AUTHENTICATION) {
        return Some("Authentication or account-recovery metadata is Private.");
    }
    if contains_any(lowercased_text, &FINANCIAL) {
        return Some("Financial or insurance metadata is Private.");
    }
    if contains_any(lowercased_text, &HEALTH) {
        return Some("Health-related metadata is Private.");
    }
    None
}

fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

/// Derived permission boundary. Records eligibility; never evidence that a
/// pseudonymization step actually ran.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProcessingPolicy {
    pub local_processing: &'static str,
    pub cloud_handling: &'static str,
    pub pseudonymization_required: bool,
    pub rationale: &'static str,
}

/// Policy follows from the stored class rather than being chosen per call.
pub fn processing_policy(data_class: &str) -> ProcessingPolicy {
    match data_class {
        "public" => ProcessingPolicy {
            local_processing: "allowed",
            cloud_handling: "eligible",
            pseudonymization_required: false,
            rationale: "Public content may be processed locally or by an approved cloud role.",
        },
        "personal" => ProcessingPolicy {
            local_processing: "allowed",
            cloud_handling: "pseudonymization_required",
            pseudonymization_required: true,
            rationale: "Personal content stays local until an explicit pseudonymization step produces a reviewed derivative.",
        },
        _ => ProcessingPolicy {
            local_processing: "allowed",
            cloud_handling: "blocked",
            pseudonymization_required: true,
            rationale: "Private source content is local-only; cloud use requires a separate reviewed derivative with a lower data class.",
        },
    }
}

/// Local approval plus explicit cloud execution state. Only `running` or later
/// implies a provider was actually called.
#[derive(Debug, Clone, Serialize)]
pub struct CloudProcessing {
    pub status: String,
    pub preview_hash: Option<String>,
    pub approved_at: Option<String>,
    pub dispatch_status: String,
    pub job_id: Option<String>,
    pub provider_role: Option<String>,
    pub queued_at: Option<String>,
    pub provider_calls: u8,
    pub task: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub last_error: Option<String>,
    pub result: Option<Value>,
}

impl CloudProcessing {
    /// The honest state for a source with no cloud pipeline at all. Not a
    /// placeholder: "nothing was prepared and nothing was sent" is the claim.
    pub fn not_prepared() -> Self {
        Self {
            status: "not_prepared".into(),
            preview_hash: None,
            approved_at: None,
            dispatch_status: "not_queued".into(),
            job_id: None,
            provider_role: None,
            queued_at: None,
            provider_calls: 0,
            task: None,
            started_at: None,
            completed_at: None,
            last_error: None,
            result: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Relevance {
    pub profile_key: String,
    pub profile_label: String,
    pub score: f64,
    pub rationale: String,
    pub mode: String,
    pub profile_revision: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Evaluation {
    pub overall_score: f64,
    pub explanation: String,
    pub mode: String,
    pub item_revision: String,
    pub context_revision: String,
    pub evaluator_revision: String,
    pub evaluated_at: String,
    pub factors: Vec<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Processing {
    pub stage: String,
    pub tier: String,
    pub revision: String,
    pub completed_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Origin {
    pub source_id: String,
    pub source_ref: String,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MailExtension {
    pub category: String,
    pub rationale: String,
    pub classification_method: String,
    pub classification_version: String,
    pub gmail_action: Option<String>,
    pub gmail_action_at: Option<String>,
    pub purge_after: Option<String>,
    pub gmail_location: Option<String>,
    pub gmail_observed_at: Option<String>,
    pub gmail_sync_status: Option<String>,
    pub gmail_sync_action: Option<String>,
    pub gmail_sync_error: Option<String>,
}

/// What only a calendar entry has: it occupies time, and the operator has
/// taken a position on it.
///
/// `commitment` rather than a score is the whole point — see the module note on
/// ranking. `all_day` and the exclusive `ends_at` are carried verbatim so the
/// reader never re-derives them and gets the boundary wrong.
#[derive(Debug, Clone, Serialize)]
pub struct CalendarExtension {
    pub starts_at: String,
    /// Exclusive, as everywhere in calendar.
    pub ends_at: String,
    pub all_day: bool,
    pub commitment: String,
    pub location: Option<String>,
    /// The operator's own note. Distinct from `summary`, which describes what
    /// the thing *is* — this is why they care, and no machine writes it.
    pub notes: Option<String>,
    /// Which adapter contributed the entry — `manual`, `luma`, `google`. Not
    /// the item's `source`, which is always `calendar`: that says which
    /// capability serves this, this says where the row came from.
    ///
    /// A reader needs it to know which actions are honest. An entry imported
    /// *from* Google must not offer to export back to Google.
    pub entry_source: String,
    /// Set when this entry was materialized from a rhythm. Carried for the same
    /// reason: a rhythm instance is not exported individually, and any patch
    /// detaches it from its rhythm — both facts a surface has to know before it
    /// offers an action.
    pub rhythm_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn minimal(source: &'static str) -> ContentItem {
        ContentItem {
            schema_version: SCHEMA_VERSION,
            source,
            id: "x".into(),
            kind: "event".into(),
            title: None,
            url: "/calendar".into(),
            author: None,
            summary: None,
            content: None,
            content_label: "Notes".into(),
            day: "2026-08-10".into(),
            created_at: "2026-08-04T00:00:00Z".into(),
            status: "committed".into(),
            content_status: "none",
            data_class: DataClass::declared_by_source("public", "Test fixture."),
            processing_policy: processing_policy("public"),
            cloud_processing: CloudProcessing::not_prepared(),
            relevance: Vec::new(),
            evaluation: None,
            processing: Vec::new(),
            origins: Vec::new(),
            links: Vec::new(),
            digest: None,
            mail: None,
            calendar: None,
        }
    }

    /// The schema is `additionalProperties: false` with every field required,
    /// so a reader may index straight in. An `Option` that skipped serializing
    /// would break that silently, on one source only.
    #[test]
    fn every_contract_field_is_emitted_even_when_empty() {
        let value = serde_json::to_value(minimal("calendar")).unwrap();
        let object = value.as_object().unwrap();
        for field in [
            "schema_version",
            "source",
            "id",
            "kind",
            "title",
            "url",
            "author",
            "summary",
            "content",
            "content_label",
            "day",
            "created_at",
            "status",
            "content_status",
            "data_class",
            "processing_policy",
            "cloud_processing",
            "relevance",
            "evaluation",
            "processing",
            "origins",
            "links",
            "digest",
            "mail",
            "calendar",
        ] {
            assert!(
                object.contains_key(field),
                "{field} missing from the wire shape"
            );
        }
        assert!(
            object["title"].is_null(),
            "an absent title is null, not omitted"
        );
        assert_eq!(
            object["links"],
            json!([]),
            "an empty collection is [], not null"
        );
    }

    #[test]
    fn the_stored_vault_class_is_labelled_private() {
        assert_eq!(DataClass::new("vault", "r", "human", "v1").label, "Private");
        assert_eq!(
            DataClass::new("personal", "r", "deterministic", "v1").label,
            "Personal"
        );
        assert_eq!(
            DataClass::declared_by_source("public", "Declared.").label,
            "Public"
        );
    }

    /// Personal content must never come back cloud-eligible; that mapping is
    /// the whole reason policy is derived rather than passed in.
    #[test]
    fn policy_is_derived_from_the_class_and_never_widens_it() {
        assert_eq!(processing_policy("public").cloud_handling, "eligible");
        assert_eq!(
            processing_policy("personal").cloud_handling,
            "pseudonymization_required"
        );
        assert_eq!(processing_policy("vault").cloud_handling, "blocked");
        // An unknown class gets the strictest policy, not the loosest.
        assert_eq!(processing_policy("something-new").cloud_handling, "blocked");
        assert!(processing_policy("something-new").pseudonymization_required);
    }

    /// The escalation rule over its whole input space, not over the three pairs
    /// someone thought of. Nine ordered class pairs times four stored methods
    /// times four proposing methods, with and without a rationale: every one of
    /// the 432 combinations is decided here, and the assertion is the rule
    /// restated independently rather than the implementation called twice.
    #[test]
    fn no_machine_lowers_a_class_or_overwrites_a_human_and_no_human_lowers_silently() {
        for stored_class in DATA_CLASSES {
            for stored_method in CLASSIFICATION_METHODS {
                for proposed_class in DATA_CLASSES {
                    for proposed_method in CLASSIFICATION_METHODS {
                        for rationale in ["", "   ", "The paper is on arXiv."] {
                            let admitted = admit_reclassification(
                                stored_class,
                                stored_method,
                                proposed_class,
                                proposed_method,
                                rationale,
                            )
                            .is_ok();
                            let stored_rank = class_rank(stored_class);
                            let proposed_rank = class_rank(proposed_class);
                            let expected = if proposed_rank < stored_rank {
                                proposed_method == METHOD_HUMAN && !rationale.trim().is_empty()
                            } else if proposed_rank == stored_rank {
                                stored_method != METHOD_HUMAN || proposed_method == METHOD_HUMAN
                            } else {
                                true
                            };
                            assert_eq!(
                                admitted, expected,
                                "{stored_class}/{stored_method} -> {proposed_class} by \
                                 {proposed_method} with rationale {rationale:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    /// The equal-class case stated on its own, because the property test above
    /// proves it holds everywhere and this one says what it is: a machine
    /// re-ingest of an item a human already classified keeps the human's
    /// record, and a genuine raise by that same machine still lands.
    #[test]
    fn a_machine_may_raise_a_humans_class_but_not_restate_it() {
        for machine in [METHOD_LEGACY, METHOD_DETERMINISTIC, METHOD_MODEL] {
            assert!(
                admit_reclassification("personal", METHOD_HUMAN, "personal", machine, "").is_err(),
                "{machine} restated a human's class and would have taken the record with it"
            );
            assert!(
                admit_reclassification("personal", METHOD_HUMAN, "vault", machine, "").is_ok(),
                "escalation is allowed to everyone, human-decided row or not"
            );
        }
        // A human revisiting their own decision is not a machine overwriting it.
        assert!(
            admit_reclassification("personal", METHOD_HUMAN, "personal", METHOD_HUMAN, "").is_ok()
        );
        // And nothing here loosened the rules-over-rules case: a resweep that
        // re-decides at the same class still lands.
        assert!(admit_reclassification(
            "personal",
            METHOD_DETERMINISTIC,
            "personal",
            METHOD_DETERMINISTIC,
            ""
        )
        .is_ok());
    }

    /// Rank is what the rule is written in terms of, so its order is part of
    /// the contract rather than an implementation detail of the match arm.
    #[test]
    fn a_stricter_class_and_a_more_authoritative_method_rank_higher() {
        assert!(class_rank("public") < class_rank("personal"));
        assert!(class_rank("personal") < class_rank("vault"));
        assert_eq!(class_rank("something-new"), None);
        assert!(method_rank(METHOD_LEGACY) < method_rank(METHOD_DETERMINISTIC));
        assert!(method_rank(METHOD_DETERMINISTIC) < method_rank(METHOD_MODEL));
        assert!(method_rank(METHOD_MODEL) < method_rank(METHOD_HUMAN));
        assert_eq!(method_rank("source-default"), None);
    }

    /// A class outside the vocabulary is refused rather than ranked, in both
    /// positions. Reaching the comparison with an unknown value would make
    /// `None < Some(_)` decide it, and `None` sorts lowest — an unknown stored
    /// class would then be silently overwritable by anything.
    #[test]
    fn an_unknown_class_is_refused_rather_than_ranked() {
        assert!(admit_reclassification(
            "something-new",
            METHOD_HUMAN,
            "public",
            METHOD_HUMAN,
            "why"
        )
        .is_err());
        assert!(admit_reclassification(
            "vault",
            METHOD_HUMAN,
            "something-new",
            METHOD_HUMAN,
            "why"
        )
        .is_err());
        assert!(
            admit_reclassification("vault", METHOD_HUMAN, "public", "source-default", "why")
                .is_err()
        );
        // Same reasoning one column over: an unranked *stored* method would
        // make `None == Some(30)` false and quietly leave a human's record
        // overwritable at equal class.
        assert!(
            admit_reclassification("vault", "source-default", "public", METHOD_HUMAN, "why")
                .is_err()
        );
    }

    /// The fail-closed default is a value, not an absence: Personal, decided by
    /// nobody, and therefore replaceable by the first real decision of any kind
    /// without needing a special case for "unset".
    #[test]
    fn an_undeclared_item_is_personal_and_outranked_by_every_real_decision() {
        let undeclared = DataClass::undeclared();
        assert_eq!(undeclared.value, "personal");
        assert_eq!(undeclared.method, METHOD_LEGACY);
        assert_eq!(
            processing_policy(&undeclared.value).cloud_handling,
            "pseudonymization_required"
        );
        for method in CLASSIFICATION_METHODS {
            assert!(method_rank(method) >= method_rank(&undeclared.method));
        }
        // Escalating it needs nobody's permission; lowering it to Public still
        // needs a human with a reason.
        assert!(admit_reclassification(
            &undeclared.value,
            &undeclared.method,
            "vault",
            METHOD_DETERMINISTIC,
            ""
        )
        .is_ok());
        assert!(admit_reclassification(
            &undeclared.value,
            &undeclared.method,
            "public",
            METHOD_DETERMINISTIC,
            ""
        )
        .is_err());
    }

    #[test]
    fn ordinary_mail_is_personal_and_never_public() {
        let result = DataClass::classify_mail("aktiv", "friend@example.com", "Weekend plan");
        assert_eq!(result.value, "personal");
        assert_eq!(result.method, METHOD_DETERMINISTIC);
        assert_eq!(
            processing_policy(&result.value).cloud_handling,
            "pseudonymization_required"
        );
        assert!(!redact_before_persistence(&result.value));
    }

    #[test]
    fn authentication_financial_and_health_mail_is_private() {
        for (stream, from, subject) in [
            ("belege", "shop@example.com", "Your order"),
            ("steuern", "amt@example.com", "Bescheid"),
            ("aktiv", "account@example.com", "Your verification code"),
            (
                "aktiv",
                "account@example.com",
                "Security alert: new sign in",
            ),
            ("aktiv", "bank@example.com", "Kontoauszug Juli"),
            ("aktiv", "praxis@example.com", "Ihr Befund"),
        ] {
            let result = DataClass::classify_mail(stream, from, subject);
            assert_eq!(result.value, "vault", "{subject} should be Private");
            assert!(redact_before_persistence(&result.value));
        }
        assert_eq!(processing_policy("vault").cloud_handling, "blocked");
    }

    /// The previous rule matched `" otp "` with literal surrounding spaces, so
    /// a subject that *began* with the code word slipped through as Personal —
    /// exactly the shape a one-time-code mail actually has.
    #[test]
    fn a_code_word_at_a_string_boundary_still_classifies_as_private() {
        for subject in ["OTP for your login", "Login 2FA", "your otp"] {
            assert_eq!(
                DataClass::classify_mail("aktiv", "noreply@example.com", subject).value,
                "vault",
                "{subject} should be Private"
            );
        }
    }

    /// Whole-word matching has to cut both ways, or the conservative default
    /// turns into "everything is Private" and the class stops carrying signal.
    #[test]
    fn a_code_word_inside_an_unrelated_word_does_not_trigger() {
        assert_eq!(
            DataClass::classify_mail("aktiv", "kollege@example.com", "Laptop-Adapter mitbringen?")
                .value,
            "personal"
        );
    }

    /// A skipped digest is a claim about the source, not a missing value: the
    /// reader has to be able to say "too short to be worth one" rather than
    /// showing an empty box that looks like a failure.
    #[test]
    fn a_skipped_digest_carries_its_verdict_and_no_text() {
        let mut item = minimal("mail");
        item.digest = Some(Digest {
            text: None,
            state: "skipped_short".into(),
            shape: "none".into(),
            depth: "standard".into(),
            focus: Vec::new(),
            producer: "openai|http://127.0.0.1:8080|gemma:content-digest-v1-adaptive".into(),
            source_chars: 148,
            redactions: 0,
            attempts: 0,
            last_error: None,
            diagram: None,
            diagram_state: None,
            diagram_error: None,
            chart: None,
            chart_state: None,
            chart_error: None,
            generated_at: "2026-08-05T12:00:00Z".into(),
        });
        let value = serde_json::to_value(&item).unwrap();
        assert_eq!(value["digest"]["state"], "skipped_short");
        assert!(value["digest"]["text"].is_null());
        assert_eq!(value["digest"]["source_chars"], 148);
        assert_eq!(value["digest"]["focus"], json!([]));
    }

    /// `summary` and `digest` are different nouns and both may be present:
    /// calendar's summary is the source's own description, and a generated
    /// digest must not be written over it.
    #[test]
    fn a_digest_never_replaces_the_sources_own_summary() {
        let mut item = minimal("calendar");
        item.summary = Some("A theme park in Brühl.".into());
        item.digest = Some(Digest {
            text: Some("- Opens 09:00\n- Ticket is dated".into()),
            state: "generated".into(),
            shape: "brief".into(),
            depth: "detailed".into(),
            focus: vec!["opening hours".into()],
            producer: "p".into(),
            source_chars: 1_400,
            redactions: 0,
            attempts: 1,
            last_error: None,
            diagram: None,
            diagram_state: None,
            diagram_error: None,
            chart: None,
            chart_state: None,
            chart_error: None,
            generated_at: "2026-08-05T12:00:00Z".into(),
        });
        let value = serde_json::to_value(&item).unwrap();
        assert_eq!(value["summary"], "A theme park in Brühl.");
        assert_eq!(value["digest"]["depth"], "detailed");
        assert_eq!(value["digest"]["focus"], json!(["opening hours"]));
    }

    #[test]
    fn not_prepared_claims_no_provider_was_called() {
        let state = CloudProcessing::not_prepared();
        assert_eq!(state.status, "not_prepared");
        assert_eq!(state.dispatch_status, "not_queued");
        assert_eq!(state.provider_calls, 0);
        assert!(state.result.is_none());
    }

    #[test]
    fn a_calendar_item_carries_commitment_and_no_score() {
        let mut item = minimal("calendar");
        item.calendar = Some(CalendarExtension {
            starts_at: "2026-08-10T09:00:00".into(),
            ends_at: "2026-08-10T19:00:00".into(),
            all_day: false,
            commitment: "committed".into(),
            location: Some("Brühl".into()),
            notes: Some("Dated ticket.".into()),
            entry_source: "manual".into(),
            rhythm_id: None,
        });
        let value = serde_json::to_value(&item).unwrap();
        assert_eq!(value["calendar"]["commitment"], "committed");
        assert_eq!(
            value["status"], "committed",
            "status mirrors the triage axis of the source"
        );
        assert_eq!(
            value["relevance"],
            json!([]),
            "a decided item is not ranked"
        );
        assert!(value["evaluation"].is_null());
        assert!(value["mail"].is_null(), "one extension at a time");
    }
}
