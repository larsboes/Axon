//! `vault` — read an Obsidian vault as data.
//!
//! Two verbs today, both read-only:
//!
//! ```text
//! vault links [--root PATH] [--json] [--dead] [--inbound FOLDER]
//! vault lint  [--root PATH] [--json] [--carrying KEY]
//! ```
//!
//! ## Why this exists as a binary rather than a skill
//!
//! Every planned vault operation — the archive move, the dialect normalisation,
//! the understanding axis, the session deposit — starts by asking the same two
//! questions: what is in here, and what links to what. A skill that describes
//! how to answer them gets a different answer each run. A binary with tests
//! gets the same one, and a migration can be gated on it.
//!
//! ## Why the counts are the acceptance test
//!
//! The figures this tool prints were first measured another way entirely, by
//! `find`, `rg` and hand-classification, before any of this existed. Those
//! numbers are the fixture: 2,248 live notes, 1,138 under `Knowledge/`, 996
//! carrying a `knowledge:` key, 69 MOCs, 133 notes linked into `Knowledge/`
//! from outside it. A disagreement means this tool is wrong, not the earlier
//! probe — and where the two differ for a reason (the shell probe counted
//! block references as links; this one separates them), the reason is stated
//! rather than the number quietly adjusted.

mod graph;
mod lint;
mod note;

fn flag(args: &[String], name: &str) -> Option<String> {
    let i = args.iter().position(|a| a == name)?;
    args.get(i + 1).filter(|v| !v.starts_with("--")).cloned()
}

fn has(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

fn die(msg: impl std::fmt::Display) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(1);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("");
    let json = has(&args, "--json");

    if matches!(cmd, "" | "-h" | "--help" | "help") {
        eprintln!(
            "vault — read an Obsidian vault as data\n\
             \n\
             usage:\n  \
               vault links [--root PATH] [--json] [--dead] [--inbound FOLDER]\n  \
               vault lint  [--root PATH] [--json] [--carrying KEY]\n\
             \n\
             The root comes from the overlay's config/knowledge.toml unless --root says otherwise."
        );
        std::process::exit(if cmd.is_empty() { 1 } else { 0 });
    }

    let root = note::resolve_root(flag(&args, "--root")).unwrap_or_else(|e| die(e));
    let (notes, problems) = note::load_all(&root).unwrap_or_else(|e| die(e));

    match cmd {
        "links" => {
            if let Some(folder) = flag(&args, "--inbound") {
                let targets = graph::inbound(&notes, &folder);
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "folder": folder,
                            "inbound_distinct": targets.len(),
                            "targets": targets,
                        }))
                        .unwrap_or_default()
                    );
                } else {
                    println!(
                        "{} distinct notes in {folder}/ are linked from outside it",
                        targets.len()
                    );
                    for t in &targets {
                        println!("  {t}");
                    }
                }
                return;
            }

            let rep = graph::report(&notes, has(&args, "--dead"));
            if json {
                println!("{}", serde_json::to_string_pretty(&rep).unwrap_or_default());
            } else {
                println!("notes                  {}", rep.notes);
                println!("wikilinks              {}", rep.links_total);
                println!("  in frontmatter       {}", rep.links_in_frontmatter);
                println!("  in body              {}", rep.links_in_body);
                println!("  resolved             {}", rep.links_resolved);
                println!("  dead                 {}", rep.links_dead);
                println!("  dead, note-shaped    {}", rep.dead_note_shaped);
                println!("  distinct dead targets {}", rep.distinct_dead_targets);
                println!("path-form links        {}", rep.path_form_total);
                println!("  of those, dead       {}", rep.path_form_dead);
                println!("ambiguous basenames    {}", rep.ambiguous_basenames.len());
                for a in &rep.ambiguous_basenames {
                    println!("  {} -> {}", a.basename, a.candidates.join(" | "));
                }
                for d in &rep.dead {
                    println!("  DEAD {} -> {}", d.from, d.target);
                }
            }
        }

        "lint" => {
            if let Some(key) = flag(&args, "--carrying") {
                let hits = lint::carrying(&notes, &key);
                if json {
                    let ids: Vec<&str> = hits.iter().map(|n| n.id.as_str()).collect();
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "key": key, "count": ids.len(), "notes": ids,
                        }))
                        .unwrap_or_default()
                    );
                } else {
                    println!("{} notes carry a `{key}:` key", hits.len());
                    for n in hits {
                        println!("  {}", n.id);
                    }
                }
                return;
            }

            let rep = lint::report(&notes, problems);
            if json {
                println!("{}", serde_json::to_string_pretty(&rep).unwrap_or_default());
            } else {
                println!(
                    "notes {} ({} without frontmatter)\n",
                    rep.notes, rep.no_frontmatter
                );
                for f in &rep.folders {
                    println!(
                        "{}  ({} notes, {} bare)",
                        f.folder, f.notes, f.no_frontmatter
                    );
                    for c in &f.fields {
                        if c.present == 0 {
                            continue;
                        }
                        let note = if c.filled == c.present {
                            String::new()
                        } else {
                            format!("  ({} empty)", c.present - c.filled)
                        };
                        println!("    {:<12} {:>5}{}", c.field, c.filled, note);
                    }
                    println!();
                }
                for d in &rep.dialects {
                    if d.forms.len() < 2 {
                        continue;
                    }
                    println!("dialect drift on `{}`:", d.field);
                    for (form, n) in &d.forms {
                        println!("    {:>6}  {}", n, form);
                    }
                    println!();
                }
                if !rep.problems.is_empty() {
                    println!("problems:");
                    for p in &rep.problems {
                        println!("  {p}");
                    }
                }
            }
        }

        other => die(format!("unknown command `{other}` (try --help)")),
    }
}
