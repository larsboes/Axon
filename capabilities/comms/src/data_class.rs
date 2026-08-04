//! Data classification and processing policy for the shared content contract.
//! Classification is inspectable and conservative; policy is derived from the
//! stored class rather than chosen independently by each inference call.

use serde::Serialize;

pub const DATA_CLASSES: [&str; 3] = ["public", "personal", "vault"];
pub const CLASSIFIER_VERSION: &str = "data-class-rules-v1";

#[derive(Debug, Clone, PartialEq)]
pub struct DataClassification {
    pub class: String,
    pub rationale: String,
    pub method: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProcessingPolicy {
    pub local_processing: &'static str,
    pub cloud_handling: &'static str,
    pub pseudonymization_required: bool,
    pub rationale: &'static str,
}

pub fn valid(value: &str) -> bool {
    DATA_CLASSES.contains(&value)
}

/// Classify only from the metadata already admitted by the Gmail sweep. The
/// body and attachments are deliberately unavailable at this stage.
pub fn classify_mail(stream: &str, from: &str, subject: &str) -> DataClassification {
    let text = format!("{} {}", from, subject).to_ascii_lowercase();

    let private_reason = if stream == "steuern" {
        Some("Tax-related mail is Private by default.")
    } else if stream == "belege" {
        Some("Receipts and invoices are Private by default.")
    } else if contains_any(
        &text,
        &[
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
            "2fa",
            " otp ",
        ],
    ) {
        Some("Authentication or account-recovery metadata is Private.")
    } else if contains_any(
        &text,
        &[
            "bank statement",
            "kontoauszug",
            "rechnung",
            "invoice",
            "payment",
            "zahlung",
            "insurance",
            "versicherung",
        ],
    ) {
        Some("Financial or insurance metadata is Private.")
    } else if contains_any(
        &text,
        &[
            "diagnosis",
            "diagnose",
            "prescription",
            "rezept",
            "medical result",
            "befund",
            "krankenversicherung",
        ],
    ) {
        Some("Health-related metadata is Private.")
    } else {
        None
    };

    match private_reason {
        Some(rationale) => DataClassification {
            class: "vault".into(),
            rationale: rationale.into(),
            method: "rules".into(),
            version: CLASSIFIER_VERSION.into(),
        },
        None => DataClassification {
            class: "personal".into(),
            rationale: "Mail metadata is Personal by default.".into(),
            method: "rules".into(),
            version: CLASSIFIER_VERSION.into(),
        },
    }
}

pub fn public_source_default() -> DataClassification {
    DataClassification {
        class: "public".into(),
        rationale: "Publicly fetched source content is Public by default.".into(),
        method: "source-default".into(),
        version: "data-class-source-v1".into(),
    }
}

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

fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_mail_is_personal() {
        let result = classify_mail("aktiv", "friend@example.com", "Weekend plan");
        assert_eq!(result.class, "personal");
        assert_eq!(
            processing_policy(&result.class).cloud_handling,
            "pseudonymization_required"
        );
    }

    #[test]
    fn financial_and_authentication_mail_is_private() {
        assert_eq!(
            classify_mail("belege", "shop@example.com", "Your order").class,
            "vault"
        );
        assert_eq!(
            classify_mail("aktiv", "account@example.com", "Your verification code").class,
            "vault"
        );
        assert_eq!(
            classify_mail(
                "aktiv",
                "account@example.com",
                "Security alert: new sign in"
            )
            .class,
            "vault"
        );
        assert_eq!(processing_policy("vault").cloud_handling, "blocked");
    }

    #[test]
    fn public_content_is_cloud_eligible_but_not_forced_to_cloud() {
        let result = public_source_default();
        assert_eq!(result.class, "public");
        assert_eq!(processing_policy(&result.class).cloud_handling, "eligible");
    }
}
