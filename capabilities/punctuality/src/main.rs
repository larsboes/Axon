//! punctuality — ingest published DB stop history, read the resulting statistics.

use punctuality::config::{redact, Config};
use punctuality::dataset::{self, FIRST_FULL_COVERAGE_MONTH};
use punctuality::ingest::{self, CellKey};
use punctuality::stats::Cell;
use punctuality::store::Store;
use std::collections::HashMap;

const USAGE: &str = "\
Usage:
  punctuality ingest [--from YYYY-MM] [--to YYYY-MM]   download + aggregate monthly releases
  punctuality stats <station|eva> [--type ICE] [--min-n N]
  punctuality stations <needle>                        eva lookup by name
  punctuality ride --type ICE --number 611 --date YYYY-MM-DD [--eva 8000044]
                                                       one train's actual stops that day

Defaults: --from 2025-12 (first month covering every station, not just the largest ~100),
--to the newest published month, --min-n 30.";

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("ingest") => ingest_cmd(&args),
        Some("stats") => stats_cmd(&args),
        Some("stations") => stations_cmd(&args),
        Some("ride") => ride_cmd(&args),
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    };
    if let Err(e) = result {
        eprintln!("punctuality: {e}");
        std::process::exit(1);
    }
}

fn ingest_cmd(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let cfg = Config::load();
    let from = flag(args, "--from").unwrap_or_else(|| FIRST_FULL_COVERAGE_MONTH.to_string());
    let to = flag(args, "--to");

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(1800))
        .build()?;
    let months = dataset::select(dataset::list_months(&client)?, &from, to.as_deref())?;

    eprintln!(
        "punctuality: {} month(s) {}..={}, cache {}",
        months.len(),
        months[0].id,
        months[months.len() - 1].id,
        cfg.raw_dir.display()
    );

    let mut cells: HashMap<CellKey, Cell> = HashMap::new();
    let mut stations: HashMap<String, String> = HashMap::new();
    let (mut rows, mut skipped) = (0u64, 0u64);

    for month in &months {
        let path = dataset::ensure_local(&client, month, &cfg.raw_dir)?;
        let counts = ingest::fold_file(&path, &mut cells, &mut stations)?;
        rows += counts.rows;
        skipped += counts.skipped;
        eprintln!(
            "  {}  {:>11} rows  {:>8} skipped  {:>8} cells so far",
            month.id,
            counts.rows,
            counts.skipped,
            cells.len()
        );
    }

    eprintln!("punctuality: writing to {}", redact(&cfg.database_url));
    let mut store = Store::open(&cfg.database_url)?;
    store.replace_stats(&cells, &stations)?;
    store.record_run(
        &months[0].id,
        &months[months.len() - 1].id,
        months.len() as i32,
        rows as i64,
        skipped as i64,
        cells.len() as i32,
    )?;

    println!(
        "{} cells from {} rows ({} skipped) across {} month(s), {} stations",
        cells.len(),
        rows,
        skipped,
        months.len(),
        stations.len()
    );
    Ok(())
}

fn stats_cmd(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let target = args.get(1).ok_or("stats needs a station name or eva")?;
    let train_type = flag(args, "--type");
    let min_n: i64 = flag(args, "--min-n").as_deref().unwrap_or("30").parse()?;

    let cfg = Config::load();
    let mut store = Store::open(&cfg.database_url)?;

    // A name is resolved through the stations table; digits are taken as an eva.
    let eva = if target.chars().all(|c| c.is_ascii_digit()) {
        target.clone()
    } else {
        let hits = store.find_stations(target)?;
        match hits.len() {
            0 => return Err(format!("no station matching '{target}'").into()),
            _ => {
                if hits.len() > 1 {
                    eprintln!(
                        "punctuality: '{target}' matched {} stations, using {}",
                        hits.len(),
                        hits[0].1
                    );
                }
                hits[0].0.clone()
            }
        }
    };

    let rows = store.station_stats(&eva, train_type.as_deref(), min_n)?;
    if rows.is_empty() {
        println!("no cells with n >= {min_n} for {eva}. Ingested anything yet?");
        return Ok(());
    }
    println!(
        "{} ({})    n >= {}",
        rows[0].station_name.clone().unwrap_or_else(|| "?".into()),
        rows[0].eva,
        min_n
    );
    println!(
        "{:<6} {:>3} {:<3} {:>7} {:>7} {:>5} {:>5} {:>8} {:>8}",
        "typ", "std", "we", "n", "mittel", "p50", "p90", ">=6min", "ausfall"
    );
    for r in &rows {
        println!(
            "{:<6} {:>3} {:<3} {:>7} {:>7.1} {:>5} {:>5} {:>7.1}% {:>7.1}%",
            r.train_type,
            r.hour,
            if r.weekend { "we" } else { "" },
            r.n,
            r.mean_delay,
            r.p50,
            r.p90,
            r.share_late_6 * 100.0,
            r.cancel_rate * 100.0
        );
    }
    Ok(())
}

fn stations_cmd(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let needle = args.get(1).ok_or("stations needs a search string")?;
    let cfg = Config::load();
    let mut store = Store::open(&cfg.database_url)?;
    for (eva, name) in store.find_stations(needle)? {
        println!("{eva}  {name}");
    }
    Ok(())
}

/// One train's actual stops on one day, straight from the cached monthly file.
///
/// Reads the columns `ingest` throws away. Deliberately CLI-only and
/// deliberately not stored: this is a lookup against files already on disk, and
/// giving it a table would create a second copy of DB's own published data.
fn ride_cmd(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let cfg = Config::load();
    let train_type = flag(args, "--type").ok_or("ride needs --type (e.g. ICE)")?;
    let number = flag(args, "--number").ok_or("ride needs --number (e.g. 611)")?;
    let date = flag(args, "--date").ok_or("ride needs --date YYYY-MM-DD")?;
    let eva = flag(args, "--eva");

    let answer = punctuality::ride::find(
        &cfg.raw_dir,
        &train_type,
        &number,
        &date,
        eva.as_deref(),
    )?;
    if answer.stops.is_empty() && answer.unavailable.is_none() {
        eprintln!(
            "punctuality: no stops for {train_type} {number} on {date}. Cached months: {}",
            punctuality::ride::cached_months(&cfg.raw_dir).join(", ")
        );
    }
    println!("{}", serde_json::to_string_pretty(&answer)?);
    Ok(())
}
