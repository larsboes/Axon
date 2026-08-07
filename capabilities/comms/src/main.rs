//! `comms` CLI. Manual arg parsing (no clap), mirroring scouting's main.rs.
//!
//! Subcommands:
//!   comms sweep [--limit N=25] [--dry-run]   read-only inbox triage proposals
//!   comms ingest <url>                        media/article ingest -> feed
//!   comms feed [--stream S] [--days N=7] [--include-dismissed]
//!   comms keep <id> | dismiss <id>            set feed item status
//!   comms summarize --pending                 retry missing summaries
//!   comms --help
//!
//! `sweep` is strictly read-only against Gmail either way -- `--dry-run` only
//! controls whether proposals are persisted to the store. Gmail writes are
//! available only through authenticated, explicit server actions.

use std::collections::BTreeMap;

use comms::config::{redact_database_url, Config};
use comms::store::Store;
use comms::{google, intake, media, normalize};

fn arg_after<'a>(args: &'a [String], flag: &str) -> Option<&'a String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
}

fn open_store(cfg: &Config) -> Store {
    Store::open(&cfg.database_url).unwrap_or_else(|e| {
        eprintln!(
            "error: could not open store at {}: {e}",
            redact_database_url(&cfg.database_url)
        );
        std::process::exit(1);
    })
}

fn print_help() {
    println!("comms — read-only Gmail triage + share-link media ingest\n");
    println!("usage: comms <command> [flags]\n");
    println!("  sweep [--limit N] [--dry-run]   list inbox threads (READ-ONLY), classify into");
    println!(
        "                                  streams, print proposals; persists unless --dry-run"
    );
    println!("                                  (default --limit 25)");
    println!("  ingest <url>                    ingest a YouTube/Instagram/podcast/article URL");
    println!("  feed [--stream news|media]      list stored feed items grouped by day");
    println!("       [--days N] [--include-dismissed]   (default --days 7)");
    println!("  keep <id>                       feed item -> 'keeper' (+ export if configured);");
    println!("                                  mail -> a distilled note in keeper_export_dir.");
    println!("                                  Never a Gmail write: archiving stays explicit.");
    println!("  dismiss <id>                    mark a feed item or mail 'dismissed' (local only)");
    println!("  summarize --pending             summarize feed items that still lack a summary");
    println!("  normalize --explain             print the normalization rules and what each drops");
    println!("  normalize --all                 re-run normalization over stored raw content");
    println!("  --help, -h                      show this help");
    println!("\nThis CLI's Gmail sweep is READ-ONLY. Archive, Trash and the Waiting label require an explicit authenticated dashboard action.");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 || args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return;
    }

    let cfg = Config::load();
    let command = args[1].as_str();

    match command {
        "sweep" => cmd_sweep(&args, &cfg),
        "ingest" => cmd_ingest(&args, &cfg),
        "feed" => cmd_feed(&args, &cfg),
        "keep" => cmd_set_status(&args, &cfg, "keeper"),
        "dismiss" => cmd_set_status(&args, &cfg, "dismissed"),
        "summarize" => cmd_summarize(&args, &cfg),
        "normalize" => cmd_normalize(&args, &cfg),
        other => {
            eprintln!("error: unknown command '{other}'\n");
            print_help();
            std::process::exit(1);
        }
    }
}

// -- sweep ---------------------------------------------------------------

fn cmd_sweep(args: &[String], cfg: &Config) {
    let limit: usize = arg_after(args, "--limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(25);
    let dry_run = args.iter().any(|a| a == "--dry-run");

    let token = match google::access_token(&cfg.google_env_path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: could not obtain Gmail access token: {e}");
            eprintln!("       (expected creds in {:?})", cfg.google_env_path);
            std::process::exit(1);
        }
    };

    let stubs = match google::list_inbox_threads(&token, limit) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: could not list inbox threads: {e}");
            std::process::exit(1);
        }
    };

    println!(
        "comms sweep — {} inbox threads (READ-ONLY){}\n",
        stubs.len(),
        if dry_run { ", dry-run" } else { "" }
    );

    let store = if dry_run { None } else { Some(open_store(cfg)) };

    // stream -> Vec<(from, subject, rationale)>
    let mut grouped: BTreeMap<String, Vec<(String, String, String)>> = BTreeMap::new();
    let mut total = 0usize;
    let mut persisted_new = 0usize;
    let mut redacted = 0usize;

    for stub in &stubs {
        let meta = match google::thread_meta(&token, &stub.id) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("  warning: skipping thread {}: {e}", stub.id);
                continue;
            }
        };
        let id = meta.id.clone();
        let from = meta.from_addr.clone().unwrap_or_default();
        let intake = intake::from_thread(meta, &cfg.rules);
        total += 1;
        redacted += usize::from(intake.redaction_count() > 0);

        if let Some(st) = &store {
            match st.upsert_triage(&intake.item) {
                Ok(true) => persisted_new += 1,
                Ok(false) => {}
                Err(e) => eprintln!("  warning: could not persist {id}: {e}"),
            }
        }

        // The redacted subject, not the swept one: what the terminal prints is
        // as much an output surface as the database is.
        let subject = intake.item.subject.clone().unwrap_or_default();
        grouped
            .entry(intake.item.stream.clone())
            .or_default()
            .push((from, subject, intake.item.rationale.clone()));
    }

    for (stream, items) in &grouped {
        println!("── {} ({}) ──", stream, items.len());
        for (from, subject, rationale) in items {
            println!("  {} | {}", truncate(from, 40), truncate(subject, 60));
            println!("      {rationale}");
        }
        println!();
    }

    println!("total: {total} threads across {} streams", grouped.len());
    if redacted > 0 {
        println!(
            "redacted: {redacted} Private thread(s) — subject and snippet stored with markers"
        );
    }
    if let Some(_st) = &store {
        println!("persisted: {persisted_new} new proposals (existing decisions preserved)");
    } else {
        println!("dry-run: nothing persisted");
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

// -- ingest --------------------------------------------------------------

fn cmd_ingest(args: &[String], cfg: &Config) {
    let url = match args.get(2) {
        Some(u) if !u.starts_with("--") => u,
        _ => {
            eprintln!("usage: comms ingest <url>");
            std::process::exit(1);
        }
    };

    let item = match media::ingest(url, cfg) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("error: ingest failed: {e}");
            std::process::exit(1);
        }
    };

    let store = open_store(cfg);
    if let Err(e) = store.upsert_feed(&item) {
        eprintln!("error: could not persist feed item: {e}");
        std::process::exit(1);
    }
    // Read back the stored row for accurate day/created_at/status.
    let stored = store.get_feed(&item.id).ok().flatten().unwrap_or(item);

    println!("ingested:");
    println!("  id      : {}", stored.id);
    println!("  kind    : {} ({})", stored.kind, stored.stream);
    println!(
        "  title   : {}",
        stored.title.as_deref().unwrap_or("(none)")
    );
    match &stored.summary {
        Some(s) => println!("  summary :\n{}", indent(s, 4)),
        None => println!("  summary : summary pending"),
    }
}

fn indent(s: &str, spaces: usize) -> String {
    let pad = " ".repeat(spaces);
    s.lines()
        .map(|l| format!("{pad}{l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

// -- feed ----------------------------------------------------------------

fn cmd_feed(args: &[String], cfg: &Config) {
    let stream = arg_after(args, "--stream").map(|s| s.as_str());
    let days: i32 = arg_after(args, "--days")
        .and_then(|v| v.parse().ok())
        .unwrap_or(7);
    let include_dismissed = args.iter().any(|a| a == "--include-dismissed");

    let store = open_store(cfg);
    let items = match store.list_feed(stream, None, days, include_dismissed) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("error: could not list feed: {e}");
            std::process::exit(1);
        }
    };

    if items.is_empty() {
        println!("comms feed — no items in the last {days} days");
        return;
    }

    println!(
        "comms feed — {} items (last {days} days){}\n",
        items.len(),
        stream.map(|s| format!(", stream={s}")).unwrap_or_default()
    );

    // Grouped by day (list_feed already orders by created_at DESC).
    let mut current_day = String::new();
    for item in &items {
        if item.day != current_day {
            current_day = item.day.clone();
            println!("── {current_day} ──");
        }
        let title = item.title.as_deref().unwrap_or("(untitled)");
        println!(
            "  [{}] {} · {}",
            item.kind,
            truncate(title, 70),
            item.status
        );
        println!("      {}", item.url);
        if let Some(s) = &item.summary {
            println!("      {}", truncate(&s.replace('\n', " "), 100));
        }
        println!("      id: {}", item.id);
    }
}

// -- keep / dismiss ------------------------------------------------------

fn cmd_set_status(args: &[String], cfg: &Config, status: &str) {
    let id = match args.get(2) {
        Some(i) if !i.starts_with("--") => i,
        _ => {
            eprintln!(
                "usage: comms {} <id>",
                if status == "keeper" {
                    "keep"
                } else {
                    "dismiss"
                }
            );
            std::process::exit(1);
        }
    };

    let store = open_store(cfg);
    // `keep` names a thing, not a table. The feed first, since that is where most ids come from,
    // then mail — which had no keep path at all, and so had no way out of the inbox except
    // staying in it. That is the outcome the comms doctrine exists to prevent
    // — the Information lane of the comms doctrine: a kept mail becomes a distilled statement in
    // the system that owns it, never a second copy of the mail.
    match store.set_feed_status(id, status) {
        Ok(true) => {
            println!("{id} -> {status}");
            if status == "keeper" {
                if let Some(dir) = &cfg.keeper_export_dir {
                    match store.get_feed(id) {
                        Ok(Some(item)) => match export_keeper(&item, dir) {
                            Ok(path) => println!("exported: {}", path.display()),
                            Err(e) => eprintln!("warning: keeper export failed: {e}"),
                        },
                        _ => eprintln!("warning: could not re-read item for export"),
                    }
                }
            }
        }
        Ok(false) => keep_mail(&store, cfg, id, status),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

/// The mail half of `keep` / `dismiss`, reached when the id is not a feed item.
///
/// Keeping a mail writes the distilled note and changes nothing in Gmail. Not an oversight:
/// archiving is a mutation the doctrine permits only on explicit approval, and folding it into
/// "the information has been extracted" would archive as a side effect. The two are printed as
/// what they are — one done, one still yours to ask for.
fn keep_mail(store: &Store, cfg: &Config, id: &str, status: &str) {
    let item = match store.get_triage(id) {
        Ok(Some(item)) => item,
        Ok(None) => {
            eprintln!("error: no feed item or mail with id '{id}'");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };

    if status != "keeper" {
        // `dismiss` on a mail is a local status and never a Gmail write — the same word meaning
        // the same thing on both sides of the store.
        match store.set_triage_status(id, "dismissed") {
            Ok(_) => println!("{id} -> dismissed (mail; Gmail untouched)"),
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        }
        return;
    }

    let Some(dir) = &cfg.keeper_export_dir else {
        eprintln!(
            "error: keeping a mail means writing it somewhere, and keeper_export_dir is not set."
        );
        eprintln!("       Set it in the overlay's comms.json (see comms.config.example.json).");
        std::process::exit(1);
    };
    match export_mail_keeper(&item, dir) {
        Ok(path) => {
            println!("exported: {}", path.display());
            println!(
                "note: the mail is still in the Inbox — archiving is a separate, explicit action."
            );
        }
        Err(e) => {
            eprintln!("error: mail export failed: {e}");
            std::process::exit(1);
        }
    }
}

/// Write the distilled statement a kept mail leaves behind: what it was, where to find it, and
/// why it was classified as it was. Refuses to overwrite, like its feed sibling.
///
/// What is deliberately NOT in here is the mail. No snippet, no body, no re-fetch — the snippet
/// is the first couple of hundred characters of the message, which is exactly the raw mail this
/// lane exists to avoid keeping a copy of. Subject, sender and date are carried because they are
/// what makes the note findable, and because they are the same fields the tasks promotion edge
/// already carries. Every one comes from the STORED row, so for a Private mail they are the
/// redacted form the intake gate produced, and nothing here can reconstruct what it removed.
fn export_mail_keeper(
    item: &comms::store::TriageItem,
    dir: &std::path::Path,
) -> std::io::Result<std::path::PathBuf> {
    std::fs::create_dir_all(dir)?;
    let subject = item.subject.as_deref().unwrap_or("(no subject)");
    // The stored TIMESTAMPTZ, cut at the date. `internal_date_text` is the read-side field;
    // `internal_date_ms` is write-side only and is None here.
    let day = item
        .internal_date_text
        .as_deref()
        .and_then(|stamp| stamp.split(' ').next())
        .filter(|day| !day.is_empty())
        .unwrap_or("undated");
    let path = dir.join(format!("{day}-mail-{}.md", slug(subject)));
    if path.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("{} already exists — refusing to overwrite", path.display()),
        ));
    }

    // The one link back. Same shape the tasks capability already uses for a promoted mail, so a
    // task and a note about one thread point at the same thread.
    let permalink = format!("https://mail.google.com/mail/u/0/#all/{}", item.id);
    let mut body = format!("# {subject}\n\n");
    if let Some(from) = &item.from_addr {
        body.push_str(&format!("- From: {from}\n"));
    }
    body.push_str(&format!("- Date: {day}\n"));
    body.push_str(&format!("- Gmail: {permalink}\n"));
    body.push_str(&format!("- Stream: {}\n", item.stream));
    // The class travels with the content instead of being re-derived at the destination:
    // re-deriving it from a redacted note would classify the redaction, not the mail.
    body.push_str(&format!("- Class: {}\n", item.data_class));
    body.push_str(&format!("\n## Why this was kept\n\n{}\n", item.rationale));
    std::fs::write(&path, body)?;
    Ok(path)
}

/// Write a distilled keeper note (title, url, date, summary — NOT the raw
/// transcript). Refuses to overwrite an existing file.
fn export_keeper(
    item: &comms::store::FeedItem,
    dir: &std::path::Path,
) -> std::io::Result<std::path::PathBuf> {
    std::fs::create_dir_all(dir)?;
    let title = item.title.as_deref().unwrap_or("untitled");
    let day = if item.day.is_empty() {
        "undated"
    } else {
        &item.day
    };
    let path = dir.join(format!("{day}-{}.md", slug(title)));
    if path.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("{} already exists — refusing to overwrite", path.display()),
        ));
    }
    let mut body = format!("# {title}\n\n- URL: {}\n- Date: {}\n\n", item.url, day);
    match &item.summary {
        Some(s) => body.push_str(&format!("## Destillat\n\n{s}\n")),
        None => body.push_str("## Digest\n\n_(no summary yet)_\n"),
    }
    std::fs::write(&path, body)?;
    Ok(path)
}

fn slug(s: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for c in s.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    let capped: String = trimmed.chars().take(60).collect();
    if capped.is_empty() {
        "note".into()
    } else {
        capped
    }
}

// -- summarize -----------------------------------------------------------

fn cmd_summarize(args: &[String], cfg: &Config) {
    if !args.iter().any(|a| a == "--pending") {
        eprintln!("usage: comms summarize --pending");
        std::process::exit(1);
    }
    let store = open_store(cfg);
    match media::summarize_pending(&store, cfg) {
        Ok(n) => println!("summarized {n} pending feed item(s)"),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

// -- normalize -----------------------------------------------------------

fn cmd_normalize(args: &[String], cfg: &Config) {
    if args.iter().any(|a| a == "--explain") {
        println!("normalization rules — each line says what that rule throws away\n");
        for rule in normalize::RULES {
            println!("  {:<26} {}", rule.name, rule.drops);
        }
        for (name, drops) in normalize::structural_rules() {
            println!("  {name:<26} {drops}");
        }
        return;
    }

    if !args.iter().any(|a| a == "--all") {
        eprintln!("usage: comms normalize --all | --explain");
        std::process::exit(1);
    }

    let store = open_store(cfg);
    match media::renormalize_all(&store) {
        Ok(report) => {
            println!("renormalized {} item(s)", report.updated);
            if report.skipped > 0 {
                println!(
                    "{} item(s) skipped — no retained raw content, only a re-fetch can fix those",
                    report.skipped
                );
            }
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}
