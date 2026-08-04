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
//! ## Dependency rule
//!
//! Compiled into consumers by `#[path]` include (see `libs/axon-config/README.md`
//! for why), so it may only use crates **every** consumer already has: `serde`
//! and `serde_json`. Adding any other dependency here silently changes a
//! consumer's dependency resolution.

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
    pub mail: Option<MailExtension>,
    pub calendar: Option<CalendarExtension>,
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

    pub fn public_source_default() -> Self {
        Self::new(
            "public",
            "Publicly fetched source content is Public by default.",
            "source-default",
            "data-class-source-v1",
        )
    }

    /// The operator's own schedule. Where they are and when is personal by
    /// construction, whatever the event itself is — a public concert still
    /// tells you the building is empty that evening.
    pub fn personal_source_default(rationale: impl Into<String>) -> Self {
        Self::new("personal", rationale, "source-default", "data-class-source-v1")
    }
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

// Gated on the standalone-tests feature, not bare cfg(test), matching
// libs/axon-config and libs/axon-server: this file is compiled into every
// consumer by #[path] include, and a lib's own suite has no business running
// inside each consumer's test binary. //libs/content-item:content_item_test
// sets the feature and runs them.
#[cfg(all(test, feature = "standalone-tests"))]
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
            data_class: DataClass::public_source_default(),
            processing_policy: processing_policy("public"),
            cloud_processing: CloudProcessing::not_prepared(),
            relevance: Vec::new(),
            evaluation: None,
            processing: Vec::new(),
            origins: Vec::new(),
            links: Vec::new(),
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
            "schema_version", "source", "id", "kind", "title", "url", "author", "summary",
            "content", "content_label", "day", "created_at", "status", "content_status",
            "data_class", "processing_policy", "cloud_processing", "relevance", "evaluation",
            "processing", "origins", "links", "mail", "calendar",
        ] {
            assert!(object.contains_key(field), "{field} missing from the wire shape");
        }
        assert!(object["title"].is_null(), "an absent title is null, not omitted");
        assert_eq!(object["links"], json!([]), "an empty collection is [], not null");
    }

    #[test]
    fn the_stored_vault_class_is_labelled_private() {
        assert_eq!(DataClass::new("vault", "r", "human", "v1").label, "Private");
        assert_eq!(DataClass::new("personal", "r", "rules", "v1").label, "Personal");
        assert_eq!(DataClass::public_source_default().label, "Public");
    }

    /// Personal content must never come back cloud-eligible; that mapping is
    /// the whole reason policy is derived rather than passed in.
    #[test]
    fn policy_is_derived_from_the_class_and_never_widens_it() {
        assert_eq!(processing_policy("public").cloud_handling, "eligible");
        assert_eq!(processing_policy("personal").cloud_handling, "pseudonymization_required");
        assert_eq!(processing_policy("vault").cloud_handling, "blocked");
        // An unknown class gets the strictest policy, not the loosest.
        assert_eq!(processing_policy("something-new").cloud_handling, "blocked");
        assert!(processing_policy("something-new").pseudonymization_required);
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
        assert_eq!(value["status"], "committed", "status mirrors the triage axis of the source");
        assert_eq!(value["relevance"], json!([]), "a decided item is not ranked");
        assert!(value["evaluation"].is_null());
        assert!(value["mail"].is_null(), "one extension at a time");
    }
}
