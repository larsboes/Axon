//! The Action kind, read out of the vault.
//!
//! The vault contract (`PRD Axon.md` §5.1b, "The no-doubling law") gives the
//! Action kind exactly one owner: `Projects/**/Tasks/`. Q48 (2026-08-27) put it
//! back there by retiring the `tasks` capability, so this module is the reader
//! that replaced a database table. It writes nothing — a task is marked done in
//! Obsidian, in a note a human owns.
//!
//! ## What counts as a task, and why not just "type: task"
//!
//! The vault already answers this question in `Resources/Bases/Tasks.base`,
//! whose filter is `hasTag("🔲") OR inFolder("Projects/Tasks") OR type ==
//! "task"`. That Base is the operator's own surface over the same notes, so
//! this reader tracks it rather than inventing a second definition of "task"
//! — two surfaces that disagree about the same folder is the failure the
//! no-doubling law exists to prevent.
//!
//! Measured against the live vault on 2026-08-28, the rule below is that filter
//! with three stated divergences:
//!
//! 1. **Scoped to `Projects/`.** §5.1b puts the Action kind there and nowhere
//!    else. Excludes nothing today (all 21 `type: task` notes are already under
//!    `Projects/`); it is the contract, not a filter.
//! 2. **No `archive`/`Archive` segment.** A wound-down project's leftovers are
//!    not on today's decision list. This is also why the Base's `hasTag("🔲")`
//!    clause is not implemented: every one of the 14 notes carrying that tag
//!    sits under `Projects/Soma/archive/`, a Vault-OS-era convention that no
//!    live note uses.
//! 3. **A `done:` key is required.** The Base's folder clause also catches
//!    `Projects/Tasks - to sort in…/Tasks.md`, which is `type: moc` — a hub,
//!    not an action. A row with no state cannot be ranked, so it is not served.
//!
//! Result on the live vault: 23 notes, 17 of them open.
//!
//! ## Why a key is served only when the ladder reads it
//!
//! A task note carries eleven frontmatter keys; five are served here because
//! the dashboard's decision ladder consumes them: `summary` renders the row
//! (with `title`, which is the filename, not a key), `due` and `priority`
//! rank it, `projects` labels it, and `done` decides whether it is a decision
//! at all. The other six — `scheduled`, `context`, `energy`, `focus`,
//! `events` and `blocked_by` — have no reader on the ladder, so serving them
//! would publish a contract nothing checks, and an unread field is the one
//! that rots without anything failing.

use std::collections::HashMap;
use std::path::Path;

use markdown_root::{frontmatter_spanned, MarkdownRoot};
use serde::Serialize;

/// The folder the Action kind lives under (vault contract §5.1b).
pub const PROJECTS: &str = "Projects";

/// The template's default when a note leaves `priority:` blank, and the same
/// fallback `Tasks.base` applies in `priority || 2`.
const DEFAULT_PRIORITY: u8 = 2;

/// One action, as the ladder needs it.
///
/// No `status` field: the vault expresses completion as `done`, and adding a
/// second spelling of the same fact here is exactly the dialect drift
/// `vault lint` exists to report.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Task {
    /// Vault-relative path, slash-separated. The identity that survives a
    /// machine, and the only handle the ladder needs to link back.
    pub id: String,
    /// The file name without `.md`. In Obsidian the file name *is* the title —
    /// an `# H1` inside the note repeats it where it exists at all (measured:
    /// identical on every note that has one), so the file name is the owner.
    pub title: String,
    pub done: bool,
    pub due: Option<String>,
    pub priority: u8,
    pub summary: Option<String>,
    /// Display names of the `projects:` wikilinks, path form stripped.
    pub projects: Vec<String>,
    /// Where the operator goes to act on it. Obsidian is the writer; this
    /// server is not.
    pub uri: String,
}

/// Resolve the folder the Action kind lives in.
///
/// A separate declared root rather than a filter over the whole vault: reading
/// every note to answer for 23 of them costs 220 ms against 6 ms for
/// `Projects/` alone (measured 2026-08-28, 2,102 notes / 14.6 MB against 334 /
/// 3.7 MB), and that is a per-request cost. Containment is unchanged — a nested
/// root is canonicalized and proven the same way.
pub fn projects_root(vault: &MarkdownRoot) -> Result<MarkdownRoot, String> {
    MarkdownRoot::declare(vault.path().join(PROJECTS))
        .map_err(|e| format!("{PROJECTS}/ under the vault root: {e}"))
}

/// The vault's name as Obsidian knows it: the root directory's own name. Taken
/// from the path rather than configured, because Obsidian derives it the same
/// way and a second declaration could only ever disagree.
pub fn vault_name(vault: &MarkdownRoot) -> String {
    vault
        .path()
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Every action in the vault, newest state on disk, read now.
///
/// A note that cannot be read is skipped rather than failing the request: one
/// unreadable file must not blank the whole list, and iCloud can hold a note
/// evicted at the moment of the read.
pub fn read(projects: &MarkdownRoot, vault_name: &str) -> Result<Vec<Task>, String> {
    let files = projects
        .markdown_files_recursive()
        .map_err(|e| format!("walking {PROJECTS}/: {e}"))?;

    let mut tasks = Vec::new();
    for path in files {
        let Some(relative) = projects.relative_id(&path) else {
            continue;
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(parsed) = frontmatter_spanned(&text) else {
            continue;
        };
        if !is_task(&relative, &parsed.fields) {
            continue;
        }
        let id = format!("{PROJECTS}/{relative}");
        tasks.push(Task {
            title: title_of(&path),
            done: is_done(&parsed.fields),
            due: value(&parsed.fields, "due"),
            priority: priority_of(&parsed.fields),
            summary: value(&parsed.fields, "summary"),
            projects: projects_of(&parsed.fields),
            uri: obsidian_uri(vault_name, &id),
            id,
        });
    }
    // Open first, then by due date, then by priority — the order the ladder
    // reads them in, and the same one `Tasks.base`'s Standard view sorts by.
    tasks.sort_by(|a, b| {
        a.done
            .cmp(&b.done)
            .then_with(|| by_due(&a.due).cmp(&by_due(&b.due)))
            .then_with(|| a.priority.cmp(&b.priority))
            .then_with(|| a.title.cmp(&b.title))
    });
    Ok(tasks)
}

/// Undated sorts after every dated task rather than before it: a deadline is
/// the thing that expires, and `None` sorting first would put "someday" above
/// "tomorrow".
fn by_due(due: &Option<String>) -> (bool, &str) {
    match due {
        Some(value) => (false, value.as_str()),
        None => (true, ""),
    }
}

/// The selection rule. See the module doc for what it tracks and where it
/// deliberately diverges.
fn is_task(relative: &str, fields: &HashMap<String, String>) -> bool {
    if !fields.contains_key("done") {
        return false;
    }
    let segments: Vec<&str> = relative.split('/').collect();
    let folders = &segments[..segments.len().saturating_sub(1)];
    if folders
        .iter()
        .any(|segment| segment.eq_ignore_ascii_case("archive"))
    {
        return false;
    }
    // `starts_with`, not equality: the project-less folder is really named
    // `Tasks - to sort in, each task needs a project`, and `Tasks.base` reaches
    // it with the prefix match `file.inFolder("Projects/Tasks")`.
    folders.iter().any(|segment| segment.starts_with("Tasks"))
        || fields.get("type").map(String::as_str) == Some("task")
}

/// `done: true` and nothing else. An empty or missing value is an open task,
/// which is what the template ships (`done: false`) and what `Tasks.base`
/// assumes with `done != true`.
fn is_done(fields: &HashMap<String, String>) -> bool {
    fields.get("done").map(String::as_str) == Some("true")
}

fn title_of(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Present AND carrying something. A key left blank by the template is not a
/// value, and reporting `due: ""` as a deadline would put an undated task in
/// the overdue band.
fn value(fields: &HashMap<String, String>, key: &str) -> Option<String> {
    fields
        .get(key)
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

fn priority_of(fields: &HashMap<String, String>) -> u8 {
    value(fields, "priority")
        .and_then(|v| v.parse().ok())
        .filter(|p| (1..=3).contains(p))
        .unwrap_or(DEFAULT_PRIORITY)
}

/// `projects: - "[[Projects/Home-Lab/Home-Lab|Home-Lab]]"` becomes `Home-Lab`.
///
/// The display half of a path-form wikilink, because that is what the operator
/// wrote it to read as. `markdown-root` has already flattened the YAML list to
/// one comma-separated string.
fn projects_of(fields: &HashMap<String, String>) -> Vec<String> {
    value(fields, "projects")
        .unwrap_or_default()
        .split(',')
        .map(|item| {
            let inner = item.trim().trim_start_matches("[[").trim_end_matches("]]");
            inner.rsplit('|').next().unwrap_or(inner).trim().to_string()
        })
        .filter(|name| !name.is_empty())
        .collect()
}

/// The address of one note in Obsidian, which is where it gets acted on.
///
/// `.md` is stripped: the `file` parameter takes a vault-relative path without
/// the extension, and passing it opens a "file not found" pane instead.
fn obsidian_uri(vault_name: &str, id: &str) -> String {
    let file = id.strip_suffix(".md").unwrap_or(id);
    format!(
        "obsidian://open?vault={}&file={}",
        percent_encode(vault_name),
        percent_encode(file)
    )
}

/// RFC 3986 unreserved set, everything else percent-encoded per UTF-8 byte.
///
/// Hand-written rather than a dependency: `markdown-root` is deliberately
/// dependency-free and this is the only encoding this crate does. The set that
/// matters here is real — task titles carry spaces, em dashes and umlauts
/// (`Verlustvortrag prüfen`, `Capture — Renewable Energy…`), and `/` must
/// survive so the path stays a path.
fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Reads a fixture vault on disk, because every rule above is a claim about
/// files and a rule tested against a hand-built `HashMap` would pass while the
/// walk, the containment check and the frontmatter parser disagreed with it.
#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        root: std::path::PathBuf,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let root =
                std::env::temp_dir().join(format!("vault-tasks-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join(PROJECTS)).expect("a writable temp directory");
            Self { root }
        }

        fn note(&self, relative: &str, frontmatter: &str) -> &Self {
            let path = self.root.join(PROJECTS).join(relative);
            std::fs::create_dir_all(path.parent().expect("a parent")).unwrap();
            std::fs::write(&path, format!("---\n{}\n---\n\nbody\n", frontmatter.trim())).unwrap();
            self
        }

        fn read(&self) -> Vec<Task> {
            let vault = MarkdownRoot::declare(&self.root).expect("the fixture root");
            let projects = projects_root(&vault).expect("a Projects folder");
            super::read(&projects, &vault_name(&vault)).expect("a readable fixture")
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    /// The three ways a note is claimed, and the two ways it is refused. Each
    /// row here is a shape measured in the live vault on 2026-08-28.
    #[test]
    fn the_selection_rule_matches_the_vault_it_was_measured_against() {
        let fixture = Fixture::new("selection");
        fixture
            // Claimed: a `Tasks/` folder under a project.
            .note("Home-Lab/Tasks/Buy a drive.md", "type: task\ndone: false")
            // Claimed: the project-less folder, whose name is a prefix rather
            // than `Tasks` — the case a folder-equality rule would drop.
            .note(
                "Tasks - to sort in, each task needs a project/Verlustvortrag.md",
                "done: false\npriority: 3",
            )
            // Claimed: `type: task` outside any Tasks folder.
            .note("Soma/Loose action.md", "type: task\ndone: false")
            // Refused: the hub note that shares the project-less folder.
            .note(
                "Tasks - to sort in, each task needs a project/Tasks.md",
                "type: moc\nsummary: \"Hub for all tasks\"",
            )
            // Refused: a wound-down project's leftovers.
            .note(
                "Soma/archive/Vault-OS/Tasks/Retire BRAT.md",
                "type: task\ndone: false",
            )
            // Refused: a project document that is not an action.
            .note("Home-Lab/Home-Lab.md", "type: project");

        let titles: Vec<String> = fixture.read().into_iter().map(|t| t.title).collect();
        assert_eq!(
            titles,
            vec!["Buy a drive", "Loose action", "Verlustvortrag"],
            "the selection rule drifted from the vault it was measured against"
        );
    }

    /// Open before decided, then by deadline. An undated task must not outrank
    /// one that is due tomorrow, which is what a naive `Option` ordering does.
    #[test]
    fn open_tasks_sort_first_and_undated_ones_sort_last() {
        let fixture = Fixture::new("ordering");
        fixture
            .note("P/Tasks/Undated.md", "type: task\ndone: false")
            .note(
                "P/Tasks/Later.md",
                "type: task\ndone: false\ndue: 2026-12-01",
            )
            .note(
                "P/Tasks/Sooner.md",
                "type: task\ndone: false\ndue: 2026-08-30",
            )
            .note(
                "P/Tasks/Finished.md",
                "type: task\ndone: true\ndue: 2026-08-01",
            );

        let titles: Vec<String> = fixture.read().into_iter().map(|t| t.title).collect();
        assert_eq!(titles, vec!["Sooner", "Later", "Undated", "Finished"]);
    }

    /// The template ships every key blank. An empty `due:` is not a deadline
    /// and an empty `priority:` is not a priority, so both fall back rather
    /// than becoming `Some("")` and `0`.
    #[test]
    fn a_blank_template_key_is_absent_rather_than_empty() {
        let fixture = Fixture::new("blanks");
        fixture.note(
            "P/Tasks/Fresh.md",
            "type: task\nsummary: \"\"\ndone: false\ndue:\nscheduled:\npriority:\nprojects: []",
        );

        let task = &fixture.read()[0];
        assert_eq!(task.due, None);
        assert_eq!(task.summary, None);
        assert_eq!(task.priority, DEFAULT_PRIORITY);
        assert!(task.projects.is_empty());
    }

    #[test]
    fn a_project_link_reads_as_its_display_name() {
        let fixture = Fixture::new("projects");
        fixture.note(
            "P/Tasks/Linked.md",
            "type: task\ndone: false\nprojects:\n  - \"[[Projects/Home-Lab/Home-Lab|Home-Lab]]\"\n  - \"[[Soma]]\"",
        );

        assert_eq!(fixture.read()[0].projects, vec!["Home-Lab", "Soma"]);
    }

    /// The link back is the whole of what the ladder can still do with a task,
    /// so an unencoded space or umlaut is a dead button rather than a cosmetic
    /// problem. `/` stays literal or the path stops being a path.
    #[test]
    fn the_obsidian_uri_encodes_what_a_real_title_contains() {
        assert_eq!(
            obsidian_uri(
                "Knowledge-Base",
                "Projects/Tasks - to sort in/Verlustvortrag prüfen.md"
            ),
            "obsidian://open?vault=Knowledge-Base&file=Projects/Tasks%20-%20to%20sort%20in/Verlustvortrag%20pr%C3%BCfen"
        );
    }

    /// A vault whose `Projects/` folder is missing is a misconfigured root, not
    /// an empty task list. Reporting zero tasks would read as "nothing to do".
    #[test]
    fn a_vault_without_a_projects_folder_is_an_error() {
        let root =
            std::env::temp_dir().join(format!("vault-tasks-{}-noprojects", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Knowledge")).unwrap();
        let vault = MarkdownRoot::declare(&root).unwrap();
        assert!(projects_root(&vault).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
}
