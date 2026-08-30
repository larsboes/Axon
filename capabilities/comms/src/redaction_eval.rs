//! Offline recall gate for the cloud redaction path.
//!
//! The corpus freezes real mail with a hand-written list of what a correct redaction must
//! remove, assigned from the text before any redactor was run. Mail rather than feed, because
//! mail is where redaction is load-bearing: after the 2026-08-30 re-derivation the feed's
//! pending backlog is 127 `public` and 1 `personal`, so almost nothing there takes this path.
//!
//! What it measures is **recall**, not similarity. A redaction is wrong in exactly one direction
//! that matters — a value that had to go and is still in the document — and `prepare` already
//! reports what it removed. What it cannot report is what it missed, which is the whole reason a
//! labelled corpus exists.
//!
//! `false_positive_markers` is counted but not gated. Over-redaction costs summary quality and
//! is worth watching; it is not a privacy failure, and a gate that fails on it would push the
//! next person to loosen the recognizers.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::cloud_derivative::{self, CloudDocumentInput};
use crate::{CommsError, Result};

#[derive(Debug, Deserialize)]
struct Corpus {
    #[serde(default)]
    acceptance: Acceptance,
    fixtures: Vec<Fixture>,
}

#[derive(Debug, Default, Deserialize)]
struct Acceptance {
    /// Recall the incumbent must hold, per entity type. Absent in the first pass on purpose:
    /// the first run's job is to find out what recall IS, and a threshold invented before the
    /// measurement is a number chosen to be met.
    #[serde(default)]
    minimum_recall_percent: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct Fixture {
    id: String,
    language: String,
    data_class: String,
    #[serde(default)]
    author: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    content: String,
    must_remove: Vec<Label>,
}

#[derive(Debug, Deserialize)]
struct Label {
    #[serde(rename = "type")]
    entity_type: String,
    value: String,
}

#[derive(Debug)]
pub struct Leak {
    pub fixture: String,
    pub language: String,
    pub entity_type: String,
    pub value: String,
}

#[derive(Debug, Default)]
pub struct EvaluationReport {
    pub fixtures: usize,
    /// `(type, language)` -> (caught, total). Split by language because the question the trial
    /// exists to answer is whether German prose is the gap.
    pub by_type_language: BTreeMap<(String, String), (usize, usize)>,
    pub leaks: Vec<Leak>,
    pub markers_written: usize,
    pub minimum_recall_percent: Option<f64>,
}

impl EvaluationReport {
    pub fn caught(&self) -> usize {
        self.by_type_language.values().map(|(c, _)| c).sum()
    }
    pub fn total(&self) -> usize {
        self.by_type_language.values().map(|(_, t)| t).sum()
    }
    pub fn recall_percent(&self) -> f64 {
        let total = self.total();
        if total == 0 {
            return 100.0;
        }
        self.caught() as f64 * 100.0 / total as f64
    }
    pub fn passed(&self) -> bool {
        match self.minimum_recall_percent {
            Some(minimum) => self.recall_percent() >= minimum,
            None => true,
        }
    }
}

pub fn evaluate_file(path: &Path) -> Result<EvaluationReport> {
    let body = fs::read_to_string(path)
        .map_err(|e| CommsError::Config(format!("{}: {e}", path.display())))?;
    let corpus: Corpus = serde_json::from_str(&body)
        .map_err(|e| CommsError::Config(format!("{}: {e}", path.display())))?;
    evaluate(corpus)
}

fn evaluate(corpus: Corpus) -> Result<EvaluationReport> {
    let mut report = EvaluationReport {
        minimum_recall_percent: corpus.acceptance.minimum_recall_percent,
        ..Default::default()
    };

    for fixture in &corpus.fixtures {
        let input = CloudDocumentInput {
            source: "redaction-eval".into(),
            id: fixture.id.clone(),
            title: Some(fixture.title.clone()),
            author: Some(fixture.author.clone()),
            summary: None,
            content: Some(fixture.content.clone()),
            data_class: fixture.data_class.clone(),
        };
        // A `vault` fixture is refused rather than redacted, which is the correct answer and not
        // a recall result. Counting it either way would be a claim about a document that was
        // never produced.
        let Ok(preview) = cloud_derivative::prepare(&input) else {
            continue;
        };
        report.fixtures += 1;
        report.markers_written += preview.redaction_count;

        let haystack = preview.document.to_lowercase();
        for label in &fixture.must_remove {
            let entry = report
                .by_type_language
                .entry((label.entity_type.clone(), fixture.language.clone()))
                .or_insert((0, 0));
            entry.1 += 1;
            // Case-insensitive containment. A redactor that lowercases a name has still leaked
            // it, and one that leaves `LARS BOES` where the label says `Lars Boes` has too.
            if haystack.contains(&label.value.to_lowercase()) {
                report.leaks.push(Leak {
                    fixture: fixture.id.clone(),
                    language: fixture.language.clone(),
                    entity_type: label.entity_type.clone(),
                    value: label.value.clone(),
                });
            } else {
                entry.0 += 1;
            }
        }
    }
    Ok(report)
}
