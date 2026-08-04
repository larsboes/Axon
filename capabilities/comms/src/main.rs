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
use comms::store::{Store, TriageItem};
use comms::{data_class, google, media, normalize, rules};

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
    println!(
        "  keep <id>                       mark a feed item 'keeper' (+ export if configured)"
    );
    println!("  dismiss <id>                    mark a feed item 'dismissed'");
    println!("  summarize --pending             summarize feed items that still lack a summary");
    println!("  normalize --explain             print the normalization rules and what each drops");
    println!("  normalize --all                 re-run normalization over stored raw content");
    println!("  --help, -h                      show this help");
    println!("\nThis CLI's Gmail sweep is READ-ONLY. Archive and Trash require an explicit authenticated dashboard action.");
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

    for stub in &stubs {
        let meta = match google::thread_meta(&token, &stub.id) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("  warning: skipping thread {}: {e}", stub.id);
                continue;
            }
        };
        let from = meta.from_addr.clone().unwrap_or_default();
        let subject = meta.subject.clone().unwrap_or_default();
        let facts = rules::MailFacts {
            from: &from,
            subject: &subject,
            has_list_unsubscribe: meta.has_list_unsubscribe(),
        };
        let (stream, rationale) = rules::classify(&facts, &cfg.rules);
        let data_classification = data_class::classify_mail(&stream, &from, &subject);
        total += 1;

        if let Some(st) = &store {
            let item = TriageItem {
                id: meta.id.clone(),
                from_addr: meta.from_addr.clone(),
                subject: meta.subject.clone(),
                snippet: meta.snippet.clone(),
                internal_date_ms: meta.internal_date_ms,
                internal_date_text: None,
                stream: stream.clone(),
                rationale: rationale.clone(),
                classification_method: "rules".into(),
                classification_version: "mail-rules-v1".into(),
                data_class: data_classification.class,
                data_class_rationale: data_classification.rationale,
                data_classification_method: data_classification.method,
                data_classification_version: data_classification.version,
                status: "proposed".into(),
                gmail_action: None,
                gmail_action_at: None,
                purge_after: None,
                gmail_location: None,
                gmail_observed_at: None,
                gmail_sync_status: None,
                gmail_sync_action: None,
                gmail_sync_error: None,
                first_seen: String::new(),
                last_seen: String::new(),
            };
            match st.upsert_triage(&item) {
                Ok(true) => persisted_new += 1,
                Ok(false) => {}
                Err(e) => eprintln!("  warning: could not persist {}: {e}", meta.id),
            }
        }

        grouped
            .entry(stream)
            .or_default()
            .push((from, subject, rationale));
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
    match store.set_feed_status(id, status) {
        Ok(true) => println!("{id} -> {status}"),
        Ok(false) => {
            eprintln!("error: no feed item with id '{id}'");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }

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
