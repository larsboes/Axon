//! Trips' commands that are not the server.
//!
//! `draft-intent` turns a sentence into a plan draft nobody has submitted. It posts
//! to the local model rung directly rather than depending on `libs/inference`, because
//! this is one request to a loopback URL and a role lookup would be more machinery
//! than the call it wraps. It has no HTTP route on purpose: the question it answers is
//! "does a small local model turn a travel sentence into a valid form", and until that
//! has started a real trip more than twice it does not need a surface.
//!
//! `export-vault` writes the safety copy PRD Q47 requires. It has no HTTP route
//! either, and for a different reason: the server already re-exports after every write
//! (`src/server.rs`, `project_after_write`), so a route would be a third caller of one
//! function with no reader. What a human needs is a command they can run when the
//! server is down — which is exactly when a safety copy matters — and that is this.

use std::io::Read;

const USAGE: &str = "\
Usage:
  trips draft-intent \"somewhere warm in October, under 300 euro, by train\"
  trips draft-intent -            read the sentence from stdin
  trips export-vault              write every plan to the vault projection
  trips export-vault --dry-run    print what would be written, touch nothing
  trips gear import               propose interior items from the overlay's gear notes
  trips gear import --json        the same proposals as JSON
  trips gear import --apply       refused for now; prints what has to exist first

draft-intent prints a CreatePlan-shaped draft plus what it could not resolve.
Persists nothing and resolves no station: every destination comes back as a
place slug with null coordinates, exactly as typed text does.

export-vault writes one Markdown file per plan under Resources/Axon/Trips/ in
the vault named by <overlay>/config/trips.json, each carrying every plan item's
payload verbatim. The server does the same after every write; this is the copy
you can take by hand.

gear import READS the overlay's item notes and PROPOSES one interior item per
note. It writes nothing: interior_item has no column for category, weight_g,
packable, waterproof, quick_dry, pack_location or trip_types, so an imported
gear row would be indistinguishable from a piece of furniture. Proposal ids are
derived from the note file names, so running it twice proposes the same set.

Environment:
  AXON_INTENT_URL     chat-completions endpoint (default http://127.0.0.1:8091/v1/chat/completions)
  AXON_INTENT_MODEL   model name (default apple-foundationmodel)
  AXON_TRIPS_GEAR_DIR gear notes directory (default <overlay>/data/items/vault-notes)";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match (args.first().map(String::as_str), args.get(1)) {
        (Some("draft-intent"), Some(text)) => {
            let sentence = if text == "-" {
                read_stdin()
            } else {
                text.to_string()
            };
            if let Err(error) = draft(&sentence) {
                eprintln!("trips: {error}");
                std::process::exit(1);
            }
        }
        (Some("gear"), Some(verb)) if verb == "import" => {
            let flag = args.get(2).map(String::as_str);
            if !matches!(flag, None | Some("--json") | Some("--apply")) {
                eprintln!("{USAGE}");
                std::process::exit(2);
            }
            if let Err(error) = gear_import(flag) {
                eprintln!("trips: {error}");
                std::process::exit(1);
            }
        }
        (Some("export-vault"), rest) => {
            let dry_run = matches!(rest.map(String::as_str), Some("--dry-run"));
            if rest.is_some() && !dry_run {
                eprintln!("{USAGE}");
                std::process::exit(2);
            }
            if let Err(error) = export_vault(dry_run) {
                eprintln!("trips: {error}");
                std::process::exit(1);
            }
        }
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    }
}

/// Write every plan into the vault, or say what a write would do.
///
/// An unconfigured vault is an error rather than a quiet zero: the whole point of the
/// command is that a copy exists, and "nothing to do" is indistinguishable from
/// "nothing was saved" once the terminal scrolls.
fn export_vault(dry_run: bool) -> Result<(), String> {
    let config = trips::config::Config::load();
    let vault = config
        .obsidian
        .ok_or("no vault configured: set obsidian.root in <overlay>/config/trips.json")?;
    let root =
        markdown_root::MarkdownRoot::declare(vault.root.clone()).map_err(|e| e.to_string())?;

    let store = trips::store::TripsStore::open(&config.database_path).map_err(|e| e.to_string())?;
    let plans = store.list_every_plan().map_err(|e| e.to_string())?;
    let items: usize = plans.iter().map(|p| p.items.len()).sum();

    if dry_run {
        for projection in trips::projection::render_all(&plans) {
            println!("{}  ({} bytes)", projection.path, projection.body.len());
        }
        println!("{} plan(s), {items} item(s) — nothing written", plans.len());
        return Ok(());
    }

    let report = trips::projection::export_all(&root, &plans).map_err(|e| e.to_string())?;
    println!(
        "{} plan(s), {items} item(s) → {}: {} created, {} updated, {} unchanged",
        plans.len(),
        trips::projection::DIR,
        report.created,
        report.updated,
        report.unchanged
    );
    for path in &report.removed {
        println!("  removed (plan gone or renamed): {path}");
    }
    for path in &report.refused {
        println!("  refused, a human owns this file now: {path}");
    }
    Ok(())
}

/// Read the overlay's gear notes and print the proposals.
///
/// `--apply` is refused with the reason rather than silently doing nothing:
/// the operator asked for a write, and "nothing happened" and "this cannot
/// happen yet" are different answers. See `trips::gear` for the whole
/// argument; when interior has the seven columns, the apply branch POSTs each
/// proposal to interior's own `POST /api/items`, which answers 409 for an id
/// that already exists and is therefore idempotent for free.
fn gear_import(flag: Option<&str>) -> Result<(), String> {
    if flag == Some("--apply") {
        return Err(trips::gear::APPLY_REFUSED.to_string());
    }
    let directory = trips::config::Config::load().gear_items_dir.ok_or(
        "no gear notes directory: set AXON_PERSONAL_ROOT, or gear.items_dir in \
         <overlay>/config/trips.json",
    )?;
    let report = trips::gear::scan(&directory)?;

    if flag == Some("--json") {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
        );
        return Ok(());
    }
    println!(
        "{} note(s) read, {} proposal(s), {} skipped (no item fields)",
        report.scanned,
        report.proposals.len(),
        report.skipped
    );
    for proposal in &report.proposals {
        let gaps = if proposal.incomplete.is_empty() {
            String::new()
        } else {
            format!("  [missing: {}]", proposal.incomplete.join(", "))
        };
        println!("  {}  {}{gaps}", proposal.id, proposal.label);
    }
    if !report.trip_type_vocabulary.is_empty() {
        println!(
            "trip_types in use ({}): {}",
            report.trip_type_vocabulary.len(),
            report.trip_type_vocabulary.join(", ")
        );
    }
    println!("\nNothing was written. {}", trips::gear::APPLY_REFUSED);
    Ok(())
}

fn read_stdin() -> String {
    let mut buffer = String::new();
    let _ = std::io::stdin().read_to_string(&mut buffer);
    buffer
}

fn draft(sentence: &str) -> Result<(), String> {
    let sentence = sentence.trim();
    if sentence.is_empty() {
        return Err("give me a sentence to draft from".into());
    }
    let url = std::env::var("AXON_INTENT_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:8091/v1/chat/completions".into());
    let model =
        std::env::var("AXON_INTENT_MODEL").unwrap_or_else(|_| "apple-foundationmodel".into());

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| format!("client build: {e}"))?;
    let response = client
        .post(&url)
        .json(&trips::intent::request_body(&model, sentence))
        .send()
        .map_err(|e| {
            format!(
                "could not reach the local model at {url} ({e}). Start it with \
                 `tools/service-runner.sh start foundation-models`, or point \
                 AXON_INTENT_URL somewhere else."
            )
        })?;
    if !response.status().is_success() {
        return Err(format!("{url} answered {}", response.status()));
    }
    let body: serde_json::Value = response
        .json()
        .map_err(|e| format!("unreadable reply: {e}"))?;
    let content = body["choices"][0]["message"]["content"]
        .as_str()
        .ok_or("the reply carried no message content")?;

    let drafted = trips::intent::draft_from_model_json(sentence, content)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&drafted).map_err(|e| e.to_string())?
    );
    Ok(())
}
