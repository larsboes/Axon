//! The wikilink graph, and which links resolve to a note that exists.
//!
//! ## Why resolution is a hashmap and not a search
//!
//! Obsidian resolves a bare `[[Name]]` by shortest-unique-path, which sounds
//! like it needs the whole tree. Measured on this vault it does not: 2,233
//! distinct basenames, 14 of them duplicated. So a map keyed on basename
//! answers 99.4% of links exactly, and the remaining 14 are reported as
//! ambiguous rather than silently resolved to whichever the walk hit first.
//! That is the honest shape — a resolver that picks one and says nothing turns
//! a wrong target into a passing check, which is worse than a broken link
//! because nothing flags it.
//!
//! ## Why frontmatter links are counted, and counted separately
//!
//! Roughly 11,000 of this vault's 18,000 wikilinks do not sit in prose at all.
//! They sit in `categories:`, `related:` and `sources:` — which is to say the
//! membership graph every MOC is fed by, and the provenance edges the whole
//! four-bucket model rests on. A link check that reads only the body misses
//! them and reports a vault 60% smaller than it is. This one was written that
//! way first and the acceptance fixture caught it.
//!
//! They stay a separate count because they behave differently: a folder move
//! rewrites a prose link and an editor rewrites a `categories:` entry, and a
//! number that mixes them cannot tell you which kind of repair you owe.
//!
//! ## Why dead links are counted twice
//!
//! A vault carries link-shaped text that was never meant to resolve: block
//! references (`[[#^abc123]]`), embeds of `.base` files, bare numbers. Counting
//! those as rot inflates the number and, worse, makes it unexplainable — the
//! figure moves and nobody can say whether the vault got better. So the report
//! carries both: every unresolved target, and the subset that actually looks
//! like a note. A migration is judged on the second and audited on the first.

use std::collections::HashMap;

use serde::Serialize;

use crate::note::Note;

#[derive(Debug, Clone, Serialize)]
pub struct DeadLink {
    pub from: String,
    pub target: String,
    /// True when the link spelled a path (`Atlas/Media/X`) rather than a bare
    /// name. Path links are the ones a folder move breaks, and the ones a move
    /// must therefore rewrite.
    pub path_form: bool,
    pub in_frontmatter: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Ambiguity {
    pub basename: String,
    pub candidates: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct LinkReport {
    pub notes: usize,
    pub links_total: usize,
    pub links_in_frontmatter: usize,
    pub links_in_body: usize,
    pub links_resolved: usize,
    /// Of the resolved, how many name a file that is not a note — a PDF, an image, a `.base`.
    /// Reported rather than merged: a reader who sees the resolved count rise wants to know
    /// what started resolving.
    pub links_to_files: usize,
    pub links_dead: usize,
    /// Dead links whose target looks like a note, excluding block refs, `.base`
    /// embeds and bare numbers. The number a migration is judged on.
    pub dead_note_shaped: usize,
    pub path_form_total: usize,
    pub path_form_dead: usize,
    pub distinct_dead_targets: usize,
    pub ambiguous_basenames: Vec<Ambiguity>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub dead: Vec<DeadLink>,
}

/// Pull every `[[target]]` out of a body, alias and heading stripped.
///
/// Embeds (`![[x]]`) count: an embed that does not resolve renders as nothing,
/// which is exactly the failure a link check exists to catch.
fn targets_in(text: &str, body_start: usize) -> Vec<(String, bool)> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'[' && bytes[i + 1] == b'[' {
            if let Some(rel) = text[i + 2..].find("]]") {
                let inner = &text[i + 2..i + 2 + rel];
                // Newlines never appear inside a wikilink; hitting one means the
                // opening brackets were something else (a code sample, a table).
                if !inner.contains('\n') {
                    let head = inner.split('|').next().unwrap_or(inner);
                    let head = head.split('#').next().unwrap_or(head);
                    let t = head.trim();
                    if !t.is_empty() {
                        out.push((t.to_string(), i < body_start));
                    }
                }
                i += 2 + rel + 2;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// `targets_in`, for the one other module that needs the same parse.
///
/// `people::linked_basenames` counts journal backlinks and must count exactly what the link
/// checker counts — a second wikilink parser would drift from this one within a week.
pub fn targets_for_test(text: &str, body_start: usize) -> Vec<String> {
    targets_in(text, body_start)
        .into_iter()
        .map(|(target, _)| target)
        .collect()
}

/// Does this target look like it names a note at all?
fn note_shaped(target: &str) -> bool {
    if target.starts_with('^') {
        return false; // block reference
    }
    if target.ends_with(".base") || target.ends_with(".canvas") {
        return false;
    }
    if target.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    true
}

fn key(s: &str) -> String {
    s.to_lowercase()
}

/// The link report.
///
/// `attachments` are the vault's non-note files as root-relative ids — PDFs, images, `.base`
/// and `.canvas`. A wikilink may name any of them, and a checker that only knows notes calls
/// every such link dead: measured on the operator's vault on 2026-09-07, that was **685 of
/// 9,362** reported dead links, including every Base a hub note embeds. They resolve here and
/// are counted separately, because "resolved to a note" and "resolved to a file" are different
/// facts and the second one is the one a reader is surprised by.
pub fn report(notes: &[Note], attachments: &[String], include_dead: bool) -> LinkReport {
    // Two indexes, because a link may name either identity.
    let mut by_basename: HashMap<String, Vec<usize>> = HashMap::new();
    let mut by_id: HashMap<String, usize> = HashMap::new();
    for (i, n) in notes.iter().enumerate() {
        by_basename.entry(key(&n.basename)).or_default().push(i);
        by_id.insert(key(n.id.trim_end_matches(".md")), i);
    }

    let mut ambiguous: Vec<Ambiguity> = by_basename
        .iter()
        .filter(|(_, v)| v.len() > 1)
        .map(|(k, v)| Ambiguity {
            basename: k.clone(),
            candidates: v.iter().map(|&i| notes[i].id.clone()).collect(),
        })
        .collect();
    ambiguous.sort_by(|a, b| a.basename.cmp(&b.basename));

    let mut total = 0usize;
    let mut resolved = 0usize;
    let mut path_form_total = 0usize;
    let mut path_form_dead = 0usize;
    let mut dead_note_shaped = 0usize;
    let mut dead: Vec<DeadLink> = Vec::new();
    let mut distinct_dead: HashMap<String, ()> = HashMap::new();

    // The same two identities, for files that are not notes. An extension is NOT trimmed: a
    // link to `Dokumente.base` names that file and not a note called `Dokumente`.
    let mut attachment_by_path: HashMap<String, ()> = HashMap::new();
    let mut attachment_by_name: HashMap<String, usize> = HashMap::new();
    for id in attachments {
        attachment_by_path.insert(key(id), ());
        let base = id.rsplit('/').next().unwrap_or(id);
        *attachment_by_name.entry(key(base)).or_default() += 1;
    }

    let mut links_to_files = 0usize;
    let mut in_fm = 0usize;
    for n in notes {
        for (target, frontmatter) in targets_in(&n.text, n.body_start) {
            total += 1;
            if frontmatter {
                in_fm += 1;
            }
            let path_form = target.contains('/');
            if path_form {
                path_form_total += 1;
            }

            let hit = if path_form {
                by_id.get(&key(target.trim_end_matches(".md"))).copied()
            } else {
                None
            }
            .or_else(|| {
                let base = target.rsplit('/').next().unwrap_or(&target);
                by_basename
                    .get(&key(base.trim_end_matches(".md")))
                    .and_then(|v| (v.len() == 1).then(|| v[0]))
            });

            let file_hit = hit.is_none() && {
                let base = target.rsplit('/').next().unwrap_or(&target);
                attachment_by_path.contains_key(&key(&target))
                    // An ambiguous basename is not a resolution, the same rule the note index
                    // above applies: two files with one name mean the link names neither.
                    || attachment_by_name.get(&key(base)) == Some(&1)
            };

            match hit {
                Some(_) => resolved += 1,
                None if file_hit => {
                    resolved += 1;
                    links_to_files += 1;
                }
                None => {
                    if path_form {
                        path_form_dead += 1;
                    }
                    if note_shaped(&target) {
                        dead_note_shaped += 1;
                    }
                    distinct_dead.insert(key(&target), ());
                    if include_dead {
                        dead.push(DeadLink {
                            from: n.id.clone(),
                            target: target.clone(),
                            path_form,
                            in_frontmatter: frontmatter,
                        });
                    }
                }
            }
        }
    }

    dead.sort_by(|a, b| (&a.from, &a.target).cmp(&(&b.from, &b.target)));

    LinkReport {
        notes: notes.len(),
        links_total: total,
        links_in_frontmatter: in_fm,
        links_in_body: total - in_fm,
        links_resolved: resolved,
        links_to_files,
        links_dead: total - resolved,
        dead_note_shaped,
        path_form_total,
        path_form_dead,
        distinct_dead_targets: distinct_dead.len(),
        ambiguous_basenames: ambiguous,
        dead,
    }
}

/// Which notes outside `folder` link INTO it.
///
/// The question a folder move has to answer before it runs: what breaks that is
/// not already broken. Returns the distinct targets, because a move rewrites a
/// target once however many times it is referenced.
pub fn inbound(notes: &[Note], folder: &str) -> Vec<String> {
    let inside: HashMap<String, ()> = notes
        .iter()
        .filter(|n| n.folder == folder)
        .map(|n| (key(&n.basename), ()))
        .collect();

    let mut hits: HashMap<String, ()> = HashMap::new();
    for n in notes.iter().filter(|n| n.folder != folder) {
        for (target, _) in targets_in(&n.text, n.body_start) {
            let base = target.rsplit('/').next().unwrap_or(&target);
            let k = key(base.trim_end_matches(".md"));
            if inside.contains_key(&k) {
                hits.insert(k, ());
            }
        }
    }
    let mut out: Vec<String> = hits.into_keys().collect();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(id: &str, body: &str) -> Note {
        Note {
            id: id.to_string(),
            basename: id
                .rsplit('/')
                .next()
                .unwrap_or(id)
                .trim_end_matches(".md")
                .to_string(),
            folder: id.split('/').next().unwrap_or("").to_string(),
            text: body.to_string(),
            body_start: 0,
            fields: Default::default(),
            raw_frontmatter: None,
            path: std::path::PathBuf::from(id),
        }
    }

    /// The 2026-09-07 finding, as a test: a hub note embeds a Base, and the checker called it
    /// dead. 1,330 of 9,362 reported dead links on the operator's vault were this.
    #[test]
    fn a_link_to_a_base_or_a_pdf_resolves_when_the_file_exists() {
        let notes = vec![note(
            "Atlas/Documents/Documents.md",
            "![[Resources/Bases/Dokumente.base]] and [[Abiturzeugnis.pdf]]",
        )];
        let attachments = vec![
            "Resources/Bases/Dokumente.base".to_string(),
            "Atlas/Documents/Bildung/Schule/Abiturzeugnis.pdf".to_string(),
        ];
        let report = report(&notes, &attachments, true);
        assert_eq!((report.links_dead, report.links_to_files), (0, 2));
        assert!(report.dead.is_empty());
    }

    #[test]
    fn a_file_that_is_not_there_is_still_dead() {
        let notes = vec![note("A.md", "[[Missing.pdf]]")];
        let report = report(&notes, &[], true);
        assert_eq!(report.links_dead, 1);
    }

    /// The same rule the note index applies: two files with one name resolve neither, because
    /// a link that could mean either means nothing checkable.
    #[test]
    fn an_ambiguous_attachment_basename_does_not_resolve() {
        let notes = vec![note("A.md", "[[Scan.pdf]]")];
        let attachments = vec!["One/Scan.pdf".to_string(), "Two/Scan.pdf".to_string()];
        assert_eq!(report(&notes, &attachments, true).links_dead, 1);
    }

    /// An extension is part of an attachment's name and is not trimmed. `Dokumente.base` and a
    /// note called `Dokumente` are different targets, and Obsidian treats them that way.
    #[test]
    fn an_attachment_extension_is_part_of_its_name() {
        let notes = vec![note("A.md", "[[Dokumente]]")];
        let attachments = vec!["Resources/Bases/Dokumente.base".to_string()];
        assert_eq!(report(&notes, &attachments, true).links_dead, 1);
    }
}
