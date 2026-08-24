//! The known-person registry: rung 0 of the redaction ladder.
//!
//! Rung 1 lives in `capabilities/comms/src/cloud_derivative.rs` and matches
//! shapes — a URL looks like a URL, an IBAN looks like an IBAN. It also carries a
//! person detector, and that detector is gated on a salutation: `is_salutation`
//! sets `redact_next_person`, so "Herr Müller" is caught and a bare "Erika" is
//! not.
//!
//! Measured against this vault on 2026-08-23: across 353 `Journal/` notes there
//! are 3,086 occurrences of people the vault already knows by name, and the
//! salutation gate catches **one** of them. That ratio is the whole argument for
//! this file. A language model guesses at which token is a person. A lookup
//! table over notes the operator wrote by hand does not.
//!
//! ## Why the registry is the hard part
//!
//! The matching is a set lookup. The cost is deciding what goes in the set, and
//! the first build of it produced two false-positive classes within minutes:
//!
//! 1. **A misfiled note poisons it.** `Atlas/People/Social & Personal Links.md`
//!    is not a person, so "Social" and "Personal" entered the registry and
//!    matched 244 times. The accuracy of this rung is the folder's filing
//!    discipline, which is why [`looks_like_a_person`] refuses a note
//!    rather than trusting its location.
//! 2. **Names collide with words.** "Jan" is a person here and a month
//!    everywhere. "Alle" is a surname fragment and German for "all". No lookup
//!    resolves that, so ambiguous tokens are held back by [`STOPLIST`] and
//!    reported instead of silently dropped.
//!
//! ## The bias, stated once
//!
//! Where a token is ambiguous this file prefers to redact. A wrongly redacted
//! month costs a slightly worse answer from a cloud model. A leaked name costs
//! the thing §6 of the PRD exists to prevent. Those are not comparable, so the
//! tie is not broken by accuracy.

use std::collections::BTreeSet;

use serde::Serialize;

use crate::note::Note;

/// Where person notes live. A flag overrides it; the default is this vault's
/// ruling, not a guess.
pub const DEFAULT_FOLDER: &str = "Atlas/People";

/// Tokens that are a name here *and* an ordinary word somewhere. Held back from
/// the registry and reported, because a silent false redaction degrades an
/// answer with no trace of why.
///
/// Deliberately short. A long stoplist is a registry that has stopped being
/// about this operator's actual contacts, and every entry here is a name that
/// really is in `Atlas/People/`.
const STOPLIST: &[&str] = &[
    // Months, in both languages the vault writes in.
    "Jan", "Mai", "Juni", "Juli", "March", "May", "June", "July", "August",
    // German function words that survive the length filter.
    "Alle", "Aber", "Auch", "Dann", "Denn", "Mehr", "Sehr", "Viel", "Ende",
    // English words that are also surnames.
    "Personal", "Social", "Best", "Young", "Green", "Brown", "Church",
];

/// A token shorter than this is dropped. Two-letter fragments match everywhere
/// and identify nobody.
const MIN_TOKEN: usize = 3;

#[derive(Serialize)]
pub struct Registry {
    /// How many notes were read as people.
    pub people: usize,
    /// Notes in the folder that were refused, and why. Surfaced rather than
    /// skipped: a refusal is a filing problem the operator can fix, and one of
    /// them cost 244 false matches before anyone looked.
    pub refused: Vec<Refused>,
    /// The matchable tokens, sorted and deduplicated.
    ///
    /// A token may contain a space: a multi-word alias is kept whole, because
    /// splitting it produces fragments that match everywhere and identify
    /// nobody. The consumer matches single tokens by equality and multi-word
    /// entries as a phrase.
    pub tokens: Vec<String>,
    /// Tokens a person note contributed that the stoplist held back. Reported so
    /// the registry never loses a name silently.
    pub withheld: Vec<String>,
}

#[derive(Serialize)]
pub struct Refused {
    pub note: String,
    pub reason: &'static str,
}

/// Is this note about a person, or is it something filed next to people?
///
/// Location is not evidence. The folder is where a human put a file, and humans
/// put a links page in there once already.
fn looks_like_a_person(n: &Note) -> Result<(), &'static str> {
    // The folder hub describes the folder, not a person.
    if n.basename == "People" {
        return Err("folder hub, not a person");
    }
    // An explicit type wins in both directions. Nothing is currently stamped
    // `type: person`, so this only ever refuses today — which is the safe half
    // to have working first.
    if let Some(t) = n.field("type") {
        let t = t.trim();
        if !t.is_empty() && t != "person" {
            return Err("frontmatter says it is not a person");
        }
    }
    // "&" joins two things. A person is one thing.
    if n.basename.contains('&') || n.basename.contains(" and ") {
        return Err("title names more than one thing");
    }
    // A name is a name, not a sentence.
    if n.basename.split_whitespace().count() > 4 {
        return Err("title is too long to be a name");
    }
    Ok(())
}

/// Every alias a note declares, in either YAML dialect.
///
/// Both forms are in this vault, and a parser that reads one silently loses the
/// other — the same class of defect as `People.base` querying lowercase keys
/// against capitalised frontmatter.
fn aliases(n: &Note) -> Vec<String> {
    let Some(raw) = n.raw_frontmatter.as_deref() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut in_block = false;
    for line in raw.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("aliases:") {
            let rest = rest.trim();
            if rest.is_empty() {
                in_block = true; // block form: entries follow on their own lines
            } else {
                // inline form: aliases: [a, b] or aliases: a
                for part in rest
                    .trim_start_matches('[')
                    .trim_end_matches(']')
                    .split(',')
                {
                    push_clean(&mut out, part);
                }
            }
            continue;
        }
        if in_block {
            if let Some(item) = trimmed.strip_prefix("- ") {
                push_clean(&mut out, item);
            } else if !trimmed.is_empty() {
                in_block = false; // the next key ended the block
            }
        }
    }
    out
}

fn push_clean(out: &mut Vec<String>, raw: &str) {
    let v = raw.trim().trim_matches('"').trim_matches('\'').trim();
    if !v.is_empty() {
        out.push(v.to_string());
    }
}

/// Build the registry from every note under `folder`.
pub fn build(notes: &[Note], folder: &str) -> Registry {
    let mut tokens: BTreeSet<String> = BTreeSet::new();
    let mut withheld: BTreeSet<String> = BTreeSet::new();
    let mut refused = Vec::new();
    let mut people = 0usize;

    for n in notes.iter().filter(|n| n.id.starts_with(folder)) {
        if let Err(reason) = looks_like_a_person(n) {
            refused.push(Refused {
                note: n.id.clone(),
                reason,
            });
            continue;
        }
        people += 1;

        let mut candidates: Vec<String> =
            n.basename.split_whitespace().map(str::to_string).collect();
        candidates.extend(aliases(n));

        for c in candidates {
            let c = c.trim_matches(|ch: char| !ch.is_alphanumeric()).to_string();
            if c.chars().count() < MIN_TOKEN {
                continue;
            }
            if STOPLIST.iter().any(|s| s.eq_ignore_ascii_case(&c)) {
                withheld.insert(c);
            } else {
                tokens.insert(c);
            }
        }
    }

    Registry {
        people,
        refused,
        tokens: tokens.into_iter().collect(),
        withheld: withheld.into_iter().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn note(id: &str, basename: &str, fm: &str) -> Note {
        let mut fields = HashMap::new();
        for line in fm.lines() {
            if let Some((k, v)) = line.split_once(':') {
                if !k.starts_with(' ') && !k.starts_with('-') {
                    fields.insert(k.trim().to_string(), v.trim().to_string());
                }
            }
        }
        Note {
            id: id.to_string(),
            path: PathBuf::from(id),
            basename: basename.to_string(),
            folder: "Atlas".into(),
            fields,
            body_start: 0,
            raw_frontmatter: Some(fm.to_string()),
            text: String::new(),
        }
    }

    #[test]
    fn takes_the_name_and_its_aliases_in_both_yaml_dialects() {
        let block = note(
            "Atlas/People/Erika Mustermann.md",
            "Erika Mustermann",
            "aliases:\n  - About Me\n  - Erika\nsummary: x",
        );
        let inline = note(
            "Atlas/People/Ada Lovelace.md",
            "Ada Lovelace",
            "aliases: [Ada, Countess]",
        );
        let r = build(&[block, inline], "Atlas/People");
        assert_eq!(r.people, 2);
        for want in ["Erika", "Mustermann", "Ada", "Lovelace", "Countess"] {
            assert!(
                r.tokens.iter().any(|t| t == want),
                "missing {want}: {:?}",
                r.tokens
            );
        }
        // A multi-word alias stays one token. Splitting "About Me" yields "About"
        // and "Me", which match everywhere and identify nobody — the same failure
        // the stoplist exists to prevent, arrived at by a different route. The
        // consumer matches it as a phrase.
        assert!(
            r.tokens.iter().any(|t| t == "About Me"),
            "multi-word alias must survive whole: {:?}",
            r.tokens
        );
    }

    #[test]
    fn refuses_the_note_that_cost_244_false_matches() {
        let r = build(
            &[note(
                "Atlas/People/Social & Personal Links.md",
                "Social & Personal Links",
                "type: note",
            )],
            "Atlas/People",
        );
        assert_eq!(r.people, 0);
        assert_eq!(r.refused.len(), 1);
        assert!(
            r.tokens.is_empty(),
            "a refused note contributed tokens: {:?}",
            r.tokens
        );
    }

    #[test]
    fn holds_back_an_ambiguous_name_instead_of_dropping_it() {
        let r = build(
            &[note("Atlas/People/Jan Schmidt.md", "Jan Schmidt", "")],
            "Atlas/People",
        );
        assert!(r.tokens.iter().any(|t| t == "Schmidt"));
        assert!(
            !r.tokens.iter().any(|t| t == "Jan"),
            "Jan must not match a month"
        );
        assert!(
            r.withheld.iter().any(|t| t == "Jan"),
            "withholding must be visible"
        );
    }

    #[test]
    fn ignores_the_folder_hub_and_short_fragments() {
        let r = build(
            &[
                note("Atlas/People/People.md", "People", "type: moc"),
                note("Atlas/People/Al Bo.md", "Al Bo", ""),
            ],
            "Atlas/People",
        );
        assert_eq!(r.people, 1, "the hub is not a person");
        assert!(r.tokens.is_empty(), "two-letter fragments identify nobody");
    }
}
