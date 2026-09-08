//! What the Journal already knows about each person, computed rather than typed.
//!
//! PRD D2: `last_contact`, `met_at` and `mention_count` sit on 70 of the 89 `Atlas/People`
//! notes and **have no producer**. The defect's own note says all three are computable from
//! `Journal/` backlinks, and this file is the proof of that claim — it computes them and
//! reports the drift against what the notes carry.
//!
//! **It reads and never writes, and the reason is D3, not caution.** Machine-owned frontmatter
//! has no protection mechanism: marked regions are body-only, so a producer writing these keys
//! could not tell its own value from one a human corrected, and would overwrite the correction
//! on the next run. That ruling is owed before a writer exists. Until it lands, the honest
//! deliverable is the number and the disagreement.
//!
//! **`contact_frequency` is not here** (D1), and that is a different kind of absence: how often
//! you want to see someone is a judgement nobody can derive from a backlink count. §8.1's band
//! 540 either gets it by hand or the band goes.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::note::Note;

/// One person, as the Journal describes them.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PersonFacts {
    pub id: String,
    pub name: String,
    /// How many `Journal/` notes link this person. Notes, not occurrences: two mentions in one
    /// entry are one day on which they came up.
    pub mention_count: usize,
    /// The newest journal date that links them, `YYYY-MM-DD`.
    pub last_contact: Option<String>,
    /// The oldest journal date that links them. Read as "first appears in the record" and not
    /// as "the day we met", which is why the field it feeds is compared and never written.
    pub met_at: Option<String>,
    /// What the note itself carries today, for each of the three keys, where it carries it.
    pub stored: BTreeMap<String, String>,
    /// The keys whose stored value disagrees with the computed one.
    pub disagrees: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct PeopleReport {
    pub people: usize,
    /// People with at least one journal backlink.
    pub with_mentions: usize,
    /// Notes carrying at least one of the three keys.
    pub carrying_any: usize,
    /// Notes where a carried key disagrees with what the Journal says.
    pub disagreeing: usize,
    pub facts: Vec<PersonFacts>,
}

/// The date a journal note is about, from its id: `Journal/2026-09-07.md` and
/// `Journal/2026/2026-09-07 Monday.md` both answer `2026-09-07`.
///
/// From the id and never from a frontmatter field: the filename is what the operator controls
/// and what survives a plugin, and a journal note whose name is not a date is not a day.
fn journal_date(id: &str) -> Option<String> {
    let base = id.rsplit('/').next()?;
    let candidate: String = base.chars().take(10).collect();
    let bytes = candidate.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    let digits_ok = candidate
        .chars()
        .enumerate()
        .all(|(i, c)| matches!(i, 4 | 7) || c.is_ascii_digit());
    digits_ok.then_some(candidate)
}

/// Every `[[target]]` in a note, basename-folded, so a path-form link and a bare one count once.
fn linked_basenames(note: &Note) -> Vec<String> {
    crate::graph::targets_for_test(&note.text, note.body_start)
        .into_iter()
        .map(|target| {
            target
                .rsplit('/')
                .next()
                .unwrap_or(&target)
                .trim_end_matches(".md")
                .to_lowercase()
        })
        .collect()
}

pub fn report(notes: &[Note]) -> PeopleReport {
    let people: Vec<&Note> = notes
        .iter()
        .filter(|note| note.folder == "Atlas" && note.id.starts_with("Atlas/People/"))
        .collect();

    // basename -> (mentions, earliest, latest)
    let mut seen: BTreeMap<String, (usize, Option<String>, Option<String>)> = BTreeMap::new();
    for note in notes.iter().filter(|note| note.folder == "Journal") {
        let Some(date) = journal_date(&note.id) else {
            continue;
        };
        let mut once: Vec<String> = linked_basenames(note);
        once.sort();
        once.dedup();
        for name in once {
            let entry = seen.entry(name).or_insert((0, None, None));
            entry.0 += 1;
            if entry.1.as_ref().is_none_or(|first| date < *first) {
                entry.1 = Some(date.clone());
            }
            if entry.2.as_ref().is_none_or(|last| date > *last) {
                entry.2 = Some(date.clone());
            }
        }
    }

    let mut out = PeopleReport {
        people: people.len(),
        ..PeopleReport::default()
    };

    for note in people {
        let key = note.basename.to_lowercase();
        let (mention_count, met_at, last_contact) =
            seen.get(&key).cloned().unwrap_or((0, None, None));

        let mut stored = BTreeMap::new();
        for field in ["last_contact", "met_at", "mention_count"] {
            if let Some(value) = note.field(field).map(str::trim).filter(|v| !v.is_empty()) {
                stored.insert(field.to_string(), value.to_string());
            }
        }

        let mut disagrees = Vec::new();
        if let Some(value) = stored.get("mention_count") {
            if value.trim() != mention_count.to_string() {
                disagrees.push("mention_count".to_string());
            }
        }
        for (field, computed) in [("last_contact", &last_contact), ("met_at", &met_at)] {
            if let (Some(value), Some(computed)) = (stored.get(field), computed.as_ref()) {
                // Compared on the first ten characters: a stored value may carry a time or a
                // wikilink around the date, and the disagreement worth reporting is the day.
                let stored_day: String = value
                    .chars()
                    .filter(|c| !"[]".contains(*c))
                    .take(10)
                    .collect();
                if &stored_day != computed {
                    disagrees.push(field.to_string());
                }
            }
        }

        if mention_count > 0 {
            out.with_mentions += 1;
        }
        if !stored.is_empty() {
            out.carrying_any += 1;
        }
        if !disagrees.is_empty() {
            out.disagreeing += 1;
        }

        out.facts.push(PersonFacts {
            id: note.id.clone(),
            name: note.basename.clone(),
            mention_count,
            last_contact,
            met_at,
            stored,
            disagrees,
        });
    }

    out.facts.sort_by(|a, b| {
        b.mention_count
            .cmp(&a.mention_count)
            .then(a.name.cmp(&b.name))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(id: &str, text: &str) -> Note {
        Note {
            id: id.to_string(),
            path: std::path::PathBuf::from(id),
            basename: id
                .rsplit('/')
                .next()
                .unwrap_or(id)
                .trim_end_matches(".md")
                .to_string(),
            folder: id.split('/').next().unwrap_or("").to_string(),
            fields: Default::default(),
            body_start: 0,
            raw_frontmatter: None,
            text: text.to_string(),
        }
    }

    #[test]
    fn the_three_fields_come_out_of_the_journal() {
        let notes = vec![
            note("Atlas/People/Erika.md", ""),
            note("Journal/2026-01-05.md", "coffee with [[Erika]]"),
            note(
                "Journal/2026-03-09 Monday.md",
                "[[Erika]] again, and [[Erika]] twice",
            ),
        ];
        let report = report(&notes);
        let erika = &report.facts[0];
        // Two notes, not three mentions: a day on which someone came up is one day.
        assert_eq!(erika.mention_count, 2);
        assert_eq!(erika.met_at.as_deref(), Some("2026-01-05"));
        assert_eq!(erika.last_contact.as_deref(), Some("2026-03-09"));
    }

    #[test]
    fn a_journal_note_that_is_not_a_date_is_not_a_day() {
        let notes = vec![
            note("Atlas/People/Erika.md", ""),
            note("Journal/Notes on journalling.md", "[[Erika]]"),
        ];
        assert_eq!(report(&notes).facts[0].mention_count, 0);
    }

    #[test]
    fn a_stored_value_that_disagrees_is_named() {
        let mut person = note("Atlas/People/Erika.md", "");
        person.fields.insert("mention_count".into(), "40".into());
        person
            .fields
            .insert("last_contact".into(), "2020-01-01".into());
        let notes = vec![person, note("Journal/2026-01-05.md", "[[Erika]]")];
        let report = report(&notes);
        assert_eq!(report.disagreeing, 1);
        assert_eq!(
            report.facts[0].disagrees,
            vec!["mention_count", "last_contact"]
        );
    }

    #[test]
    fn a_person_the_journal_never_names_reads_as_zero_and_not_as_missing() {
        // Zero is the measurement here, and it is the one a `contact_frequency` band would act
        // on. An absent value would be indistinguishable from a producer that did not run.
        let notes = vec![note("Atlas/People/Erika.md", "")];
        let facts = &report(&notes).facts[0];
        assert_eq!((facts.mention_count, facts.last_contact.clone()), (0, None));
    }
}
