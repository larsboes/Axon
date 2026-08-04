//! Pure, deterministic, config-driven classifier. No network, no LLM in this
//! build. Given the headers of a mail thread, it assigns exactly one stream and
//! a one-sentence English rationale.
//!
//! Order: config `rules` (first match wins) → built-in heuristics → the
//! conservative default (`aktiv`). The built-in heuristics are intentionally
//! generic (no personal senders); anything personal belongs in the overlay's
//! `rules` list.

use serde::Deserialize;

/// A config rule. `r#match` is `match` in JSON (a Rust keyword, hence raw).
/// All match conditions present must hold (AND); absent conditions are ignored.
/// A rule with an entirely empty match spec never fires (guards against an
/// accidental catch-all).
#[derive(Debug, Clone, Deserialize)]
pub struct Rule {
    pub r#match: MatchSpec,
    pub stream: String,
    pub note: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct MatchSpec {
    /// Any of these substrings present (case-insensitive) in the From header.
    pub from_contains: Option<Vec<String>>,
    /// Any of these substrings present (case-insensitive) in the Subject.
    pub subject_contains: Option<Vec<String>>,
    /// Require the List-Unsubscribe header present (true) or absent (false).
    pub has_list_unsubscribe: Option<bool>,
}

/// The header facts a single thread's classification depends on. Borrowed so
/// the classifier stays allocation-free and trivially testable.
#[derive(Debug, Clone, Copy)]
pub struct MailFacts<'a> {
    pub from: &'a str,
    pub subject: &'a str,
    pub has_list_unsubscribe: bool,
}

/// The seven ratified triage streams. Kept in sync with the CHECK constraint
/// in store.rs manually (single-user tool, not worth generating).
pub const STREAMS: [&str; 7] = [
    "aktiv",
    "issue",
    "feed",
    "werbung",
    "belege",
    "steuern",
    "sonstiges",
];

const SHOPPING_KEYWORDS: [&str; 6] = ["sale", "rabatt", "%", "deal", "shop", "angebot"];
const TECH_KEYWORDS: [&str; 6] = [
    "release",
    "changelog",
    "engineering",
    "ai",
    "newsletter",
    "digest",
];
const RECEIPT_KEYWORDS: [&str; 6] = [
    "rechnung", "receipt", "invoice", "zahlung", "payment", "beleg",
];

fn contains_ci(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

fn any_contains(haystack: &str, needles: &[String]) -> bool {
    needles.iter().any(|n| contains_ci(haystack, n))
}

fn any_contains_static(haystack: &str, needles: &[&str]) -> bool {
    let low = haystack.to_lowercase();
    needles.iter().any(|n| low.contains(n))
}

/// Evaluate a single config rule against the facts. Returns the matched
/// condition description if it fires, else None.
fn eval_rule(rule: &Rule, f: &MailFacts) -> Option<String> {
    let m = &rule.r#match;
    let mut conditions = 0;
    let mut matched: Vec<String> = Vec::new();

    if let Some(subs) = &m.from_contains {
        conditions += 1;
        if any_contains(f.from, subs) {
            matched.push("sender".into());
        } else {
            return None;
        }
    }
    if let Some(subs) = &m.subject_contains {
        conditions += 1;
        if any_contains(f.subject, subs) {
            matched.push("subject".into());
        } else {
            return None;
        }
    }
    if let Some(want) = m.has_list_unsubscribe {
        conditions += 1;
        if f.has_list_unsubscribe == want {
            matched.push(if want {
                "List-Unsubscribe present".into()
            } else {
                "no List-Unsubscribe".into()
            });
        } else {
            return None;
        }
    }

    if conditions == 0 {
        // Empty match spec: never a catch-all.
        return None;
    }
    Some(matched.join(" + "))
}

/// Classify a thread into (stream, rationale). Config rules first (first match
/// wins), then the built-in heuristics, then the conservative `aktiv` default.
pub fn classify(f: &MailFacts, rules: &[Rule]) -> (String, String) {
    // 1. Config rules — first match wins.
    for rule in rules {
        if let Some(cond) = eval_rule(rule, f) {
            return (rule.stream.clone(), format!("{} ({})", rule.note, cond));
        }
    }

    // 2. Built-in heuristics (generic, no personal facts).
    if f.has_list_unsubscribe && any_contains_static(f.subject, &SHOPPING_KEYWORDS) {
        return (
            "werbung".into(),
            "List-Unsubscribe plus a shopping signal in the subject; classified as advertising."
                .into(),
        );
    }
    if f.has_list_unsubscribe && any_contains_static(f.subject, &TECH_KEYWORDS) {
        return (
            "feed".into(),
            "List-Unsubscribe plus a development or technology signal in the subject; classified as a Feed newsletter.".into(),
        );
    }
    let noreply = contains_ci(f.from, "noreply") || contains_ci(f.from, "no-reply");
    if noreply && any_contains_static(f.subject, &RECEIPT_KEYWORDS) {
        return (
            "belege".into(),
            "A no-reply sender plus a receipt or invoice signal in the subject; classified as a receipt.".into(),
        );
    }
    if f.has_list_unsubscribe {
        return (
            "sonstiges".into(),
            "List-Unsubscribe is present, but no specific rule matched; classified as other."
                .into(),
        );
    }

    // 3. Conservative default.
    (
        "aktiv".into(),
        "No rule matched; kept active as the conservative default.".into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts<'a>(from: &'a str, subject: &'a str, lu: bool) -> MailFacts<'a> {
        MailFacts {
            from,
            subject,
            has_list_unsubscribe: lu,
        }
    }

    #[test]
    fn builtin_shopping_promo_is_werbung() {
        let (stream, why) = classify(
            &facts("news@shop.example", "Winter SALE -50% Rabatt", true),
            &[],
        );
        assert_eq!(stream, "werbung");
        assert!(!why.is_empty());
    }

    #[test]
    fn builtin_tech_newsletter_is_feed() {
        let (stream, _) = classify(
            &facts("hello@bytes.dev", "This week in AI: new release", true),
            &[],
        );
        assert_eq!(stream, "feed");
    }

    #[test]
    fn builtin_noreply_invoice_is_belege() {
        let (stream, _) = classify(
            &facts("noreply@vendor.example", "Ihre Rechnung 2026-07", false),
            &[],
        );
        assert_eq!(stream, "belege");
    }

    #[test]
    fn builtin_bare_list_unsubscribe_is_sonstiges() {
        let (stream, _) = classify(
            &facts("info@social.example", "Weekly community update", true),
            &[],
        );
        assert_eq!(stream, "sonstiges");
    }

    #[test]
    fn no_signal_is_conservative_aktiv() {
        let (stream, why) = classify(
            &facts("a.person@gmail.com", "Re: lunch tomorrow?", false),
            &[],
        );
        assert_eq!(stream, "aktiv");
        assert!(why.contains("conservative default"));
    }

    #[test]
    fn config_rule_wins_over_builtin() {
        // Without the rule this would be a tech `feed`; the config rule reroutes it.
        let rules = vec![Rule {
            r#match: MatchSpec {
                from_contains: Some(vec!["bytes.dev".into()]),
                subject_contains: None,
                has_list_unsubscribe: None,
            },
            stream: "feed".into(),
            note: "curated development newsletter".into(),
        }];
        let (stream, why) = classify(&facts("hello@bytes.dev", "random subject", true), &rules);
        assert_eq!(stream, "feed");
        assert!(why.contains("curated development newsletter"));
        assert!(why.contains("sender"));
    }

    #[test]
    fn config_rule_can_produce_steuern() {
        let rules = vec![Rule {
            r#match: MatchSpec {
                from_contains: None,
                subject_contains: Some(vec!["Steuerbescheid".into()]),
                has_list_unsubscribe: None,
            },
            stream: "steuern".into(),
            note: "steuerrelevant".into(),
        }];
        let (stream, _) = classify(
            &facts("amt@example.gov", "Ihr Steuerbescheid 2025", false),
            &rules,
        );
        assert_eq!(stream, "steuern");
    }

    #[test]
    fn empty_match_spec_never_fires() {
        let rules = vec![Rule {
            r#match: MatchSpec::default(),
            stream: "werbung".into(),
            note: "should never match".into(),
        }];
        let (stream, _) = classify(&facts("a@b.com", "hello", false), &rules);
        assert_eq!(
            stream, "aktiv",
            "empty match spec must not act as a catch-all"
        );
    }
}
