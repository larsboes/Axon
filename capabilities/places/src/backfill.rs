//! One-shot backfills (README "Backfills" section). Subcommands on the places
//! binary, not routes. Each is idempotent — stable ids and ON CONFLICT DO
//! NOTHING — and prints counts instead of guessing.
//!
//! `amex` re-derives the finance candidate fingerprint from the raw export
//! rows. The algorithm is a deliberate copy of
//! `capabilities/finance/src/import.rs` (SHA-256 over the exact tuple
//! `booked_at, amount_cents, currency, description, source_reference,
//! source_account`, 0xff-separated, with the same per-file occurrence rule for
//! reference-less repeats), because that fingerprint is the journal-stable
//! `source_id` the projection carries and the one identity a location link can
//! safely hang on (README D2).

use crate::geocode::{GeocodeQuery, Geocoder, StructuredQuery};
use crate::layers::{normalize_eva, parse_station_ref};
use crate::store::{stable_id, validate_schema, Fallible, Place, PlacesStore};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Address columns of the raw American Express export. Constants here rather
/// than config because finance's import mapping deliberately has no fields for
/// them — the import pipeline discards these columns, which is why the raw
/// files are the only surviving venue source (README D1).
const AMEX_ADDRESS_COLUMN: &str = "Adresse";
const AMEX_CITY_COLUMN: &str = "Stadt";
const AMEX_POSTAL_COLUMN: &str = "PLZ";
const AMEX_COUNTRY_COLUMN: &str = "Land";

/// Where transit's suggest surface answers (`capabilities/transit/service.toml`
/// port 3000, `GET /api/suggest`). Overridable for tests and odd deployments.
pub fn transit_url() -> String {
    std::env::var("AXON_PLACES_TRANSIT_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "http://127.0.0.1:3000".to_string())
}

// ---------------------------------------------------------------------------
// Fingerprint recomputation (the finance candidate identity)
// ---------------------------------------------------------------------------

/// Byte-identical to finance's `fingerprint`: each part, then 0xff.
pub fn fingerprint(parts: &[&str]) -> String {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update(part.as_bytes());
        hash.update([0xff]);
    }
    let mut encoded = String::with_capacity(64);
    for byte in hash.finalize() {
        use std::fmt::Write;
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}

/// finance's `normalize_text`: whitespace runs collapse to single spaces.
pub fn normalize_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `DD/MM/YYYY` → `YYYY-MM-DD`, the one date format the Amex mapping declares
/// (`day_month_year_slashes` in the overlay's finance.json).
pub fn normalize_date_dmy_slashes(value: &str) -> Option<String> {
    let value = value.trim();
    let bytes = value.as_bytes();
    if bytes.len() != 10 || bytes[2] != b'/' || bytes[5] != b'/' {
        return None;
    }
    let day: u32 = value.get(..2)?.parse().ok()?;
    let month: u32 = value.get(3..5)?.parse().ok()?;
    let year: u32 = value.get(6..10)?.parse().ok()?;
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    if day == 0 || day > days {
        return None;
    }
    Some(format!("{year:04}-{month:02}-{day:02}"))
}

/// finance's `parse_decimal_cents` for the subset the Amex export uses:
/// grouping separator stripped, one decimal mark, at most two fraction digits,
/// optional sign (leading or trailing).
pub fn parse_amount_cents(value: &str, decimal_separator: char) -> Option<i64> {
    let mut value = value.trim().replace(['\u{a0}', ' '], "");
    if value.ends_with('-') {
        value.pop();
        value.insert(0, '-');
    }
    let grouping = if decimal_separator == ',' { '.' } else { ',' };
    value = value.replace(grouping, "");
    if value.matches(decimal_separator).count() > 1 {
        return None;
    }
    let negative = value.starts_with('-');
    let unsigned = value.strip_prefix(['-', '+']).unwrap_or(value.as_str());
    if unsigned.is_empty()
        || !unsigned
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == decimal_separator as u8)
    {
        return None;
    }
    let (whole, fraction) = unsigned
        .split_once(decimal_separator)
        .map_or((unsigned, ""), |parts| parts);
    if fraction.len() > 2 {
        return None;
    }
    let whole: i64 = if whole.is_empty() {
        0
    } else {
        whole.parse().ok()?
    };
    let fraction_cents = fraction
        .bytes()
        .fold(0_i64, |cents, digit| cents * 10 + i64::from(digit - b'0'))
        * if fraction.len() == 1 { 10 } else { 1 };
    let cents = whole.checked_mul(100)?.checked_add(fraction_cents)?;
    if negative {
        cents.checked_neg()
    } else {
        Some(cents)
    }
}

/// The candidate fingerprint for one normalized Amex row. `occurrence` is the
/// zero-based repeat count of the same base tuple within one file — finance's
/// preserved-repetition rule, without which the second identical coffee in one
/// export would never match its candidate.
///
/// Parity with finance's fingerprint rests on two assumptions about the Amex
/// export, hardcoded here as reference `""` and currency `"EUR"` even though
/// the overlay mapping declares reference_column and currency_column: every
/// measured row has an empty reference cell and an EUR currency cell. A future
/// row that breaks either assumption hashes differently and is reported as
/// unmatched — never guessed (the raw-file phase's honest-failure rule).
pub fn amex_fingerprint(
    booked_at: &str,
    amount_cents: i64,
    description: &str,
    source_account: &str,
    occurrence: usize,
) -> String {
    let amount = amount_cents.to_string();
    let base = fingerprint(&[booked_at, &amount, "EUR", description, "", source_account]);
    if occurrence == 0 {
        base
    } else {
        let ordinal = occurrence.to_string();
        fingerprint(&["csv-occurrence", &base, &ordinal])
    }
}

// ---------------------------------------------------------------------------
// Overlay profile lookup
// ---------------------------------------------------------------------------

/// The pieces of the overlay's American Express CSV mapping this backfill
/// needs. Read from the same `finance.json` finance itself loads, so the
/// fingerprint inputs (source account, sign convention, separators) cannot
/// drift from the ones the candidates were staged with — and no private
/// account name is baked into public code.
#[derive(Debug, Clone, PartialEq)]
pub struct AmexProfile {
    pub source_account: String,
    pub invert: bool,
    pub delimiter: u8,
    pub decimal_separator: char,
    pub date_column: String,
    pub amount_column: String,
    pub description_column: String,
}

pub fn amex_profile_from(config: &Value) -> Fallible<AmexProfile> {
    let profiles = config
        .get("csv_mappings")
        .and_then(Value::as_array)
        .ok_or("finance.json has no csv_mappings")?;
    let profile = profiles
        .iter()
        .find(|profile| {
            profile
                .get("label")
                .and_then(Value::as_str)
                .map(str::to_lowercase)
                .is_some_and(|label| label.contains("american express") || label.contains("amex"))
        })
        .ok_or("no American Express profile in the overlay's finance.json csv_mappings")?;
    let mapping = profile
        .get("mapping")
        .ok_or("Amex profile has no mapping")?;
    let text = |key: &str| mapping.get(key).and_then(Value::as_str).map(str::to_string);
    Ok(AmexProfile {
        source_account: text("source_account").ok_or("Amex mapping has no source_account")?,
        invert: text("amount_sign").as_deref() == Some("invert"),
        delimiter: text("delimiter")
            .and_then(|d| d.bytes().next())
            .unwrap_or(b','),
        decimal_separator: text("decimal_separator")
            .and_then(|d| d.chars().next())
            .unwrap_or(','),
        date_column: text("date_column").unwrap_or_else(|| "Datum".into()),
        amount_column: text("amount_column").unwrap_or_else(|| "Betrag".into()),
        description_column: text("description_column").unwrap_or_else(|| "Beschreibung".into()),
    })
}

fn load_amex_profile() -> Fallible<AmexProfile> {
    let path = axon_config::overlay_config("finance.json")
        .ok_or("AXON_PERSONAL_ROOT is not set; the raw exports live in the private overlay")?;
    let body = std::fs::read_to_string(&path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    amex_profile_from(&serde_json::from_str(&body)?)
}

fn raw_import_dir() -> Fallible<PathBuf> {
    let dir = axon_config::overlay_root()
        .ok_or("AXON_PERSONAL_ROOT is not set; the raw exports live in the private overlay")?
        .join("data/finance/import/raw");
    if !dir.is_dir() {
        return Err(format!("raw import directory {} does not exist", dir.display()).into());
    }
    Ok(dir)
}

fn column(headers: &csv::StringRecord, name: &str) -> Option<usize> {
    headers
        .iter()
        .position(|header| header.trim().trim_start_matches('\u{feff}') == name)
}

/// The address block of one raw row: street line, city line, postal code,
/// country. `Adresse` is a quoted multi-line value (street on the first line,
/// city on the last); the separate `Stadt` column wins when it is filled.
fn split_address(adresse: &str, stadt: &str) -> (Option<String>, Option<String>) {
    let lines: Vec<String> = adresse
        .lines()
        .map(normalize_text)
        .filter(|line| !line.is_empty())
        .collect();
    let street = lines.first().cloned();
    let stadt = normalize_text(stadt);
    let city = if !stadt.is_empty() {
        Some(stadt)
    } else if lines.len() > 1 {
        lines.last().cloned()
    } else {
        None
    };
    (street, city)
}

// ---------------------------------------------------------------------------
// backfill amex
// ---------------------------------------------------------------------------

/// Counters for one Amex source (the stored candidate columns, or the raw
/// export files).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct AmexStats {
    /// Venue-precision links written.
    pub linked: usize,
    /// Rows the venue path could not serve — a structured miss, or no street
    /// at all — that still earned a city-precision link (D1: the city string
    /// comes from the raw data, so nothing is guessed).
    pub city_fallback: usize,
    /// Rows the provider could resolve at neither precision.
    pub not_found: usize,
    /// Rows with neither a street nor a city.
    pub no_address: usize,
    pub skipped_linked: usize,
    pub cache_hits: usize,
}

/// The address block of one Amex row, wherever it was read from: the stored
/// candidate columns (the finance contract) or the raw export.
#[derive(Debug, Clone, Default)]
pub struct RawAddress {
    pub street: Option<String>,
    pub city: Option<String>,
    pub postal_code: Option<String>,
    pub country: Option<String>,
}

/// The venue-first, city-second link path both Amex sources share. Place text
/// only reaches the provider (README D3): street, postal code, city, country —
/// never the description, amount or date. Venue precision requires a street
/// (D1): a street-less row goes straight to the city path, so city-level
/// geometry is never pinned as a venue. When the structured venue attempt
/// misses too, the city line alone (raw data, not a guess) earns a
/// city-precision link with source `amex-city-fallback`.
fn link_address(
    store: &PlacesStore,
    geocoder: &Geocoder,
    source_id: &str,
    address: RawAddress,
    today: &str,
    stats: &mut AmexStats,
) -> Fallible<()> {
    if address.street.is_none() && address.city.is_none() {
        stats.no_address += 1;
        return Ok(());
    }
    if address.street.is_some() {
        let query = GeocodeQuery::Structured(StructuredQuery {
            street: address.street,
            postalcode: address.postal_code,
            city: address.city.clone(),
            country: address.country,
        });
        let outcome = geocoder.geocode(&query, Some("venue"), today)?;
        if outcome.cached {
            stats.cache_hits += 1;
        }
        if let Some(place) = outcome.place {
            if store.link_transaction(source_id, &place.id, "venue", 9000, "amex-backfill", today)?
            {
                stats.linked += 1;
            } else {
                stats.skipped_linked += 1;
            }
            return Ok(());
        }
    }
    let Some(city) = address.city.filter(|city| !city.trim().is_empty()) else {
        stats.not_found += 1;
        return Ok(());
    };
    // No kind_override: the guard below tests the kind DERIVED from the
    // provider response, so a non-city top hit is counted as not found,
    // never force-registered and linked as a city (D1).
    let outcome = geocoder.geocode(&GeocodeQuery::Free(city), None, today)?;
    if outcome.cached {
        stats.cache_hits += 1;
    }
    match outcome.place {
        Some(place) if place.kind == "city" => {
            if store.link_transaction(
                source_id,
                &place.id,
                "city",
                6000,
                "amex-city-fallback",
                today,
            )? {
                stats.city_fallback += 1;
            } else {
                stats.skipped_linked += 1;
            }
        }
        _ => stats.not_found += 1,
    }
    Ok(())
}

/// Phase one of `backfill amex`: candidates whose raw location columns finance
/// stored at import time (`location_street` holds the verbatim Adresse value,
/// embedded newline and all — street on the first line, city on the second).
/// Returns `None` until finance's migration adds the columns; otherwise the
/// counters plus the fingerprints this phase covered, which the raw-file phase
/// skips. The schema is a parameter so the pg test can point the same SQL at a
/// scratch copy of the table; production passes `finance`.
pub fn amex_from_candidates(
    store: &PlacesStore,
    geocoder: &Geocoder,
    finance_schema: &str,
    source_account: &str,
    already_linked: &HashSet<String>,
    today: &str,
) -> Fallible<Option<(AmexStats, HashSet<String>)>> {
    validate_schema(finance_schema)?;
    type Located = (
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let located: Option<Vec<Located>> = {
        let mut conn = store.conn()?;
        let columns_present: i64 = conn
            .query_one(
                "SELECT COUNT(*) FROM information_schema.columns
                 WHERE table_schema = $1 AND table_name = 'transaction_candidates'
                   AND column_name = 'location_street'",
                &[&finance_schema],
            )?
            .get(0);
        if columns_present == 0 {
            None
        } else {
            Some(
                conn.query(
                    &format!(
                        "SELECT fingerprint, location_street, location_city,
                                location_postal_code, location_country
                         FROM {finance_schema}.transaction_candidates
                         WHERE source_account = $1 AND location_street IS NOT NULL
                         ORDER BY fingerprint"
                    ),
                    &[&source_account],
                )?
                .iter()
                .map(|row| (row.get(0), row.get(1), row.get(2), row.get(3), row.get(4)))
                .collect(),
            )
        }
    };
    let Some(located) = located else {
        return Ok(None);
    };

    let mut stats = AmexStats::default();
    let mut covered: HashSet<String> = HashSet::new();
    for (fingerprint, street_block, city, postal_code, country) in located {
        covered.insert(fingerprint.clone());
        if already_linked.contains(&fingerprint) {
            stats.skipped_linked += 1;
            continue;
        }
        let (street, city) = split_address(&street_block, city.as_deref().unwrap_or(""));
        let clean = |value: Option<String>| {
            value
                .as_deref()
                .map(normalize_text)
                .filter(|value| !value.is_empty())
        };
        link_address(
            store,
            geocoder,
            &fingerprint,
            RawAddress {
                street,
                city,
                postal_code: clean(postal_code),
                country: clean(country),
            },
            today,
            &mut stats,
        )?;
    }
    Ok(Some((stats, covered)))
}

pub fn amex(store: &PlacesStore, today: &str) -> Fallible<()> {
    let profile = load_amex_profile()?;
    let dir = raw_import_dir()?;
    let geocoder = Geocoder::new(store);

    let candidate_fingerprints: HashSet<String> = {
        let mut conn = store.conn()?;
        conn.query(
            "SELECT fingerprint FROM finance.transaction_candidates WHERE source_account = $1",
            &[&profile.source_account],
        )?
        .iter()
        .map(|row| row.get(0))
        .collect()
    };
    let already_linked = store.linked_source_ids()?;

    // Primary source: the location columns finance stores at import time.
    // The raw files below cover only the candidates imported before finance
    // kept those columns (all historical rows today).
    let candidate_phase = amex_from_candidates(
        store,
        &geocoder,
        "finance",
        &profile.source_account,
        &already_linked,
        today,
    )?;
    let (candidate_stats, covered) = match candidate_phase {
        Some((stats, covered)) => (Some(stats), covered),
        None => (None, HashSet::new()),
    };

    let mut files = 0_usize;
    let mut rows = 0_usize;
    let mut matched = 0_usize;
    let mut unmatched = 0_usize;
    let mut covered_by_candidates = 0_usize;
    let mut raw_stats = AmexStats::default();

    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("activity") && name.ends_with(".csv"))
        })
        .collect();
    paths.sort();
    if paths.is_empty() {
        return Err(format!("no activity*.csv files in {}", dir.display()).into());
    }

    for path in &paths {
        files += 1;
        let mut reader = csv::ReaderBuilder::new()
            .delimiter(profile.delimiter)
            .from_path(path)?;
        let headers = reader.headers()?.clone();
        let need = |name: &str| {
            column(&headers, name)
                .ok_or_else(|| format!("{}: column {name:?} is absent", path.display()))
        };
        let date_at = need(&profile.date_column)?;
        let amount_at = need(&profile.amount_column)?;
        let description_at = need(&profile.description_column)?;
        let address_at = need(AMEX_ADDRESS_COLUMN)?;
        let city_at = need(AMEX_CITY_COLUMN)?;
        let postal_at = need(AMEX_POSTAL_COLUMN)?;
        let country_at = need(AMEX_COUNTRY_COLUMN)?;

        // Per file, like finance's per-import staging: the ordinal counts
        // repeats of the same base tuple inside one export.
        let mut occurrences: HashMap<String, usize> = HashMap::new();

        for record in reader.records() {
            let record = record?;
            rows += 1;
            let get = |at: usize| record.get(at).unwrap_or_default();
            let Some(booked_at) = normalize_date_dmy_slashes(get(date_at)) else {
                unmatched += 1;
                continue;
            };
            let Some(mut amount_cents) =
                parse_amount_cents(get(amount_at), profile.decimal_separator)
            else {
                unmatched += 1;
                continue;
            };
            if profile.invert {
                let Some(inverted) = amount_cents.checked_neg() else {
                    unmatched += 1;
                    continue;
                };
                amount_cents = inverted;
            }
            let description = normalize_text(get(description_at));
            let base_key = format!("{booked_at}\u{1f}{amount_cents}\u{1f}{description}");
            let occurrence = {
                let counter = occurrences.entry(base_key).or_insert(0);
                let current = *counter;
                *counter += 1;
                current
            };
            let source_id = amex_fingerprint(
                &booked_at,
                amount_cents,
                &description,
                &profile.source_account,
                occurrence,
            );
            if !candidate_fingerprints.contains(&source_id) {
                unmatched += 1;
                continue;
            }
            matched += 1;
            if covered.contains(&source_id) {
                covered_by_candidates += 1;
                continue;
            }
            if already_linked.contains(&source_id) {
                raw_stats.skipped_linked += 1;
                continue;
            }
            let (street, city) = split_address(get(address_at), get(city_at));
            let postalcode = normalize_text(get(postal_at));
            let country = normalize_text(get(country_at));
            link_address(
                store,
                &geocoder,
                &source_id,
                RawAddress {
                    street,
                    city,
                    postal_code: (!postalcode.is_empty()).then_some(postalcode),
                    country: (!country.is_empty()).then_some(country),
                },
                today,
                &mut raw_stats,
            )?;
        }
    }

    // The source split the report promises: candidates-with-stored-location
    // first, raw files second.
    match &candidate_stats {
        Some(stats) => println!(
            "backfill amex (candidates): {} with stored location, {} venue links written, \
             {} city-fallback links, {} not geocodable, {} without address, \
             {} already linked, {} geocode cache hits",
            covered.len(),
            stats.linked,
            stats.city_fallback,
            stats.not_found,
            stats.no_address,
            stats.skipped_linked,
            stats.cache_hits,
        ),
        None => println!(
            "backfill amex (candidates): finance has no location columns yet; raw files only"
        ),
    }
    println!(
        "backfill amex (raw files): {files} files, {rows} rows, {matched} matched candidates, \
         {covered_by_candidates} covered by stored locations, \
         {unmatched} unmatched (reported, never guessed), {} without address, \
         {} not geocodable, {} venue links written, {} city-fallback links, \
         {} already linked, {} geocode cache hits",
        raw_stats.no_address,
        raw_stats.not_found,
        raw_stats.linked,
        raw_stats.city_fallback,
        raw_stats.skipped_linked,
        raw_stats.cache_hits,
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// backfill cities
// ---------------------------------------------------------------------------

/// A usable city token: letters (plus space, dot, hyphen) and at least three
/// characters. Filters the numeric junk that ends PayPal descriptions and the
/// pseudo-city lines some Amex rows carry.
fn plausible_city(token: &str) -> bool {
    token.chars().count() >= 3
        && token
            .chars()
            .all(|c| c.is_alphabetic() || matches!(c, ' ' | '.' | '-'))
}

/// The longest known city that ends the description, compared in uppercase —
/// `"MARKT 12345678 MUSTERSTADT"` matches `MUSTERSTADT`, `"CAFE BAD MUSTERSTADT"`
/// matches `BAD MUSTERSTADT`. No match means skip: conservative by contract
/// (README D1).
pub fn trailing_city<'a>(description: &str, cities: &'a HashSet<String>) -> Option<&'a str> {
    let upper = normalize_text(description).to_uppercase();
    cities
        .iter()
        .filter(|city| upper == **city || upper.ends_with(&format!(" {city}")))
        .max_by_key(|city| city.len())
        .map(String::as_str)
}

fn known_cities(store: &PlacesStore) -> Fallible<HashSet<String>> {
    let schema = store.schema();
    let mut conn = store.conn()?;
    let mut cities: HashSet<String> = conn
        .query(
            &format!(
                "SELECT DISTINCT city FROM {schema}.places WHERE city IS NOT NULL
                 UNION
                 SELECT name FROM {schema}.places WHERE kind = 'city'"
            ),
            &[],
        )?
        .iter()
        .map(|row| normalize_text(&row.get::<_, String>(0)).to_uppercase())
        .filter(|city| plausible_city(city))
        .collect();

    // Plus the city lines of the raw Amex exports, when the overlay is here.
    if let Ok(profile) = load_amex_profile() {
        if let Ok(dir) = raw_import_dir() {
            for entry in std::fs::read_dir(&dir)?.filter_map(|entry| entry.ok()) {
                let path = entry.path();
                let is_activity = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("activity") && name.ends_with(".csv"));
                if !is_activity {
                    continue;
                }
                let mut reader = csv::ReaderBuilder::new()
                    .delimiter(profile.delimiter)
                    .from_path(&path)?;
                let headers = reader.headers()?.clone();
                let (Some(address_at), Some(city_at)) = (
                    column(&headers, AMEX_ADDRESS_COLUMN),
                    column(&headers, AMEX_CITY_COLUMN),
                ) else {
                    continue;
                };
                for record in reader.records().filter_map(|record| record.ok()) {
                    let (_, city) = split_address(
                        record.get(address_at).unwrap_or_default(),
                        record.get(city_at).unwrap_or_default(),
                    );
                    if let Some(city) = city {
                        let upper = city.to_uppercase();
                        if plausible_city(&upper) {
                            cities.insert(upper);
                        }
                    }
                }
            }
        }
    }
    Ok(cities)
}

pub fn cities(store: &PlacesStore, today: &str) -> Fallible<()> {
    let cities = known_cities(store)?;
    let geocoder = Geocoder::new(store);
    let schema = store.schema();

    let unlinked: Vec<(String, String)> = {
        let mut conn = store.conn()?;
        conn.query(
            &format!(
                "SELECT p.source_id, p.description
                 FROM finance.transaction_projection p
                 WHERE p.kind = 'expense' AND p.currency = 'EUR'
                   AND p.source_id IS NOT NULL
                   AND NOT EXISTS (
                       SELECT 1 FROM {schema}.transaction_places tp
                       WHERE tp.source_id = p.source_id
                   )
                 ORDER BY p.booked_at, p.source_id"
            ),
            &[],
        )?
        .iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect()
    };

    let mut skipped = 0_usize;
    let mut linked = 0_usize;
    let mut not_found = 0_usize;
    let mut wrong_kind = 0_usize;
    for (source_id, description) in &unlinked {
        let Some(city) = trailing_city(description, &cities) else {
            skipped += 1;
            continue;
        };
        // No kind_override: a `city` override would stamp any first-seen OSM
        // object kind='city' and make the guard below vacuous. Deriving the
        // kind from the response is what lets `wrong_kind` actually fire (D1).
        let outcome = geocoder.geocode(&GeocodeQuery::Free(city.to_string()), None, today)?;
        match outcome.place {
            Some(place) if place.kind == "city" => {
                if store.link_transaction(
                    source_id,
                    &place.id,
                    "city",
                    6000,
                    "city-backfill",
                    today,
                )? {
                    linked += 1;
                }
            }
            Some(_) => wrong_kind += 1,
            None => not_found += 1,
        }
    }

    println!(
        "backfill cities: {} unlinked transactions, {} known city names, \
         {linked} city links written, {skipped} without a recognisable city token (skipped), \
         {not_found} city names the geocoder could not resolve, {wrong_kind} resolved to a non-city",
        unlinked.len(),
        cities.len(),
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// backfill stations
// ---------------------------------------------------------------------------

pub fn stations(store: &PlacesStore, today: &str) -> Fallible<()> {
    // Every station reference transit persists, from both tables. Legs carry
    // full HAFAS ids whose X=/Y= already embed the coordinate; trips carry
    // bare EVA codes.
    let refs: Vec<String> = {
        let mut conn = store.conn()?;
        conn.query(
            "SELECT origin_eva FROM transit.trips
             UNION SELECT destination_eva FROM transit.trips
             UNION SELECT origin_eva FROM transit.trip_legs
             UNION SELECT destination_eva FROM transit.trip_legs",
            &[],
        )?
        .iter()
        .map(|row| row.get(0))
        .collect()
    };

    // Group by EVA, preferring a reference that carries its own coordinates.
    let mut by_eva: BTreeMap<String, crate::layers::StationRef> = BTreeMap::new();
    for raw in &refs {
        let Some(parsed) = parse_station_ref(raw) else {
            continue;
        };
        let entry = by_eva
            .entry(parsed.eva.clone())
            .or_insert_with(|| parsed.clone());
        if entry.latitude.is_none() && parsed.latitude.is_some() {
            *entry = parsed;
        }
    }

    let mut from_refs = 0_usize;
    let mut via_suggest = 0_usize;
    let mut existing = 0_usize;
    let mut unresolved: Vec<String> = Vec::new();

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;
    let suggest_base = transit_url();

    for (eva, station) in &by_eva {
        let external_ref = format!("eva:{eva}");
        if store.place_by_external_ref(&external_ref)?.is_some() {
            existing += 1;
            continue;
        }
        let (name, latitude, longitude) =
            if let (Some(lat), Some(lon)) = (station.latitude, station.longitude) {
                let name = station.name.clone().unwrap_or_else(|| format!("EVA {eva}"));
                from_refs += 1;
                (name, lat, lon)
            } else {
                // Bare EVA: name it from punctuality's station list (zero-padded
                // there) when listed, then ask transit's own suggest surface for
                // coordinates — by that name, or by the bare EVA itself. The EVA
                // query is what resolves the HAFAS meta-stations (city-level
                // aggregate codes) that punctuality's station list lacks (ISA
                // PLC-6; the specific codes are live data and stay in the
                // overlay evidence note).
                let known_name: Option<String> = {
                    let mut conn = store.conn()?;
                    conn.query_opt(
                        "SELECT station_name FROM punctuality.stations
                     WHERE eva = $1 OR eva = lpad($1, 8, '0')",
                        &[&eva],
                    )?
                    .map(|row| row.get(0))
                };
                let query = known_name.clone().unwrap_or_else(|| eva.clone());
                let resolved = client
                    .get(format!("{suggest_base}/api/suggest"))
                    .query(&[("q", query.as_str())])
                    .send()
                    .ok()
                    .filter(|response| response.status().is_success())
                    .and_then(|response| response.json::<Value>().ok())
                    .and_then(|body| {
                        body.as_array()?.iter().find_map(|station| {
                            let id = station.get("id").and_then(Value::as_str)?;
                            if normalize_eva(id) != *eva {
                                return None;
                            }
                            Some((
                                station
                                    .get("name")
                                    .and_then(Value::as_str)
                                    .map(str::to_string),
                                station.get("latitude")?.as_f64()?,
                                station.get("longitude")?.as_f64()?,
                            ))
                        })
                    });
                let Some((suggest_name, lat, lon)) = resolved else {
                    unresolved.push(eva.clone());
                    continue;
                };
                let name = known_name
                    .or(suggest_name)
                    .unwrap_or_else(|| format!("EVA {eva}"));
                via_suggest += 1;
                (name, lat, lon)
            };
        store.upsert_place(
            &Place {
                id: stable_id("place", &external_ref),
                name,
                kind: "station".into(),
                address: None,
                city: None,
                country_code: None,
                latitude: Some(latitude),
                longitude: Some(longitude),
                source: "transit-backfill".into(),
                external_ref: Some(external_ref.clone()),
            },
            today,
        )?;
    }

    println!(
        "backfill stations: {} distinct EVA codes, {from_refs} from embedded HAFAS coordinates, \
         {via_suggest} via transit suggest ({suggest_base}), {existing} already registered, \
         {} unresolved{}",
        by_eva.len(),
        unresolved.len(),
        if unresolved.is_empty() {
            String::new()
        } else {
            format!(" ({})", unresolved.join(", "))
        }
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// backfill travelers (PRD 8.2: register proposals from trips' travelers)
// ---------------------------------------------------------------------------

/// Counts only. Person names and plan titles stay in the database — never in
/// logs or printed output (README D4: the register is C2).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct TravelersReport {
    /// Non-archived plans read.
    pub plans: usize,
    /// Plans with travelers but no destination that carries a coordinate —
    /// reported, never guessed.
    pub without_located_destination: usize,
    /// Traveler-name × plan pairs seen.
    pub pairs: usize,
    pub proposals_written: usize,
    pub proposals_existing: usize,
    pub places_created: usize,
}

/// The traveler names of one plan, as trips serializes them: a JSON array of
/// strings (`trips.plans.travelers`).
pub fn traveler_names(travelers: &str) -> Vec<String> {
    serde_json::from_str::<Value>(travelers)
        .ok()
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect()
}

/// The first destination `PlaceRef` that carries coordinates
/// (`capabilities/trips/src/store.rs` serializes `id, name, kind, latitude,
/// longitude`). Returns `(name, registry kind, latitude, longitude)`; `None`
/// when no destination can be drawn.
pub fn first_located_destination(destinations: &str) -> Option<(String, String, f64, f64)> {
    let refs = serde_json::from_str::<Value>(destinations).ok()?;
    refs.as_array()?.iter().find_map(|place_ref| {
        let latitude = place_ref.get("latitude").and_then(Value::as_f64)?;
        let longitude = place_ref.get("longitude").and_then(Value::as_f64)?;
        let name = normalize_text(place_ref.get("name").and_then(Value::as_str)?);
        if name.is_empty() {
            return None;
        }
        // trips' PlaceKind, mapped onto the registry's kinds; `airport` lands
        // as station, and trips' own default is station.
        let kind = match place_ref.get("kind").and_then(Value::as_str) {
            Some("city") => "city",
            Some("venue") => "venue",
            Some("address") => "address",
            _ => "station",
        };
        Some((name, kind.to_string(), latitude, longitude))
    })
}

/// Derive register proposals from `trips.plans` travelers (PRD 8.2). One
/// proposal per traveler × plan, id stable on that pair, state hardcoded to
/// `proposed` by `propose_person_place` — derivation never writes a confirmed
/// row (ISA PLC-7). Archived plans are skipped, the same reading of `status`
/// as `travel_layer`. The schema is a parameter so the pg test can point the
/// same SQL at a scratch copy of the table; production passes `trips`.
pub fn travelers_from(
    store: &PlacesStore,
    trips_schema: &str,
    today: &str,
) -> Fallible<TravelersReport> {
    validate_schema(trips_schema)?;
    let plans: Vec<(String, String, String, String, String)> = {
        let mut conn = store.conn()?;
        conn.query(
            &format!(
                "SELECT id, travelers, destinations, date_start, date_end
                 FROM {trips_schema}.plans
                 WHERE status != 'archived'
                 ORDER BY date_start, id"
            ),
            &[],
        )?
        .iter()
        .map(|row| (row.get(0), row.get(1), row.get(2), row.get(3), row.get(4)))
        .collect()
    };

    let mut report = TravelersReport::default();
    for (plan_id, travelers, destinations, date_start, date_end) in plans {
        report.plans += 1;
        let names = traveler_names(&travelers);
        if names.is_empty() {
            continue;
        }
        let Some((name, kind, latitude, longitude)) = first_located_destination(&destinations)
        else {
            report.without_located_destination += 1;
            continue;
        };
        // One registry row per destination name and kind, whatever plan named
        // it — the existing upsert-by-stable-identity path.
        let external_ref = format!("trips-dest:{kind}:{}", name.to_lowercase());
        let place = Place {
            id: stable_id("place", &external_ref),
            name,
            kind,
            address: None,
            city: None,
            country_code: None,
            latitude: Some(latitude),
            longitude: Some(longitude),
            source: "trips-travelers".into(),
            external_ref: Some(external_ref),
        };
        if store.upsert_place(&place, today)? {
            report.places_created += 1;
        }
        for person in &names {
            report.pairs += 1;
            let id = stable_id("pp", &format!("trips-travelers:{person}:{plan_id}"));
            if store.propose_person_place(
                &id,
                person,
                &place.id,
                Some(&date_start),
                Some(&date_end),
                5000,
                "trips-travelers",
                today,
            )? {
                report.proposals_written += 1;
            } else {
                report.proposals_existing += 1;
            }
        }
    }
    Ok(report)
}

pub fn travelers(store: &PlacesStore, today: &str) -> Fallible<()> {
    let report = travelers_from(store, "trips", today)?;
    println!(
        "backfill travelers: {} plans, {} traveler-plan pairs, {} proposals written, \
         {} already present, {} destination places created, \
         {} plans without a located destination (skipped)",
        report.plans,
        report.pairs,
        report.proposals_written,
        report.proposals_existing,
        report.places_created,
        report.without_located_destination,
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// backfill vault
// ---------------------------------------------------------------------------

/// Minimal frontmatter reader for the two shapes the vault uses. Returns the
/// scalar map and the list map of the block between the first `---` pair.
pub fn parse_frontmatter(body: &str) -> (BTreeMap<String, String>, BTreeMap<String, Vec<String>>) {
    let mut scalars = BTreeMap::new();
    let mut lists: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut lines = body.lines();
    if lines.next().map(str::trim) != Some("---") {
        return (scalars, lists);
    }
    let mut current_list: Option<String> = None;
    for line in lines {
        if line.trim() == "---" {
            break;
        }
        if let Some(item) = line.strip_prefix("  - ") {
            if let Some(key) = &current_list {
                lists
                    .entry(key.clone())
                    .or_default()
                    .push(item.trim().trim_matches('"').to_string());
            }
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim().to_string();
            let value = value.trim().trim_matches('"').to_string();
            if value.is_empty() {
                current_list = Some(key);
            } else {
                current_list = None;
                scalars.insert(key, value);
            }
        }
    }
    (scalars, lists)
}

/// Coordinates from frontmatter, in both spellings the vault holds today:
/// a two-item list of quoted strings (`- "40.75"` / `- "-74.00"`, latitude
/// first) or an inline `lat,lon` scalar. `[]` and absence are `None`.
pub fn frontmatter_coordinates(
    scalars: &BTreeMap<String, String>,
    lists: &BTreeMap<String, Vec<String>>,
) -> Option<(f64, f64)> {
    if let Some(items) = lists.get("coordinates") {
        if items.len() == 2 {
            return Some((items[0].parse().ok()?, items[1].parse().ok()?));
        }
    }
    let inline = scalars.get("coordinates")?;
    let inline = inline.trim();
    if inline.is_empty() || inline == "[]" {
        return None;
    }
    let (lat, lon) = inline.split_once(',')?;
    Some((lat.trim().parse().ok()?, lon.trim().parse().ok()?))
}

/// Great-circle distance in kilometres — haversine in code, the no-PostGIS
/// decision (README "Deliberately not built", following
/// `dashboard/src/lib/travel/travel-candidates.ts`).
pub fn haversine_km(a: (f64, f64), b: (f64, f64)) -> f64 {
    let radius_km = 6371.0;
    let (lat1, lon1) = (a.0.to_radians(), a.1.to_radians());
    let (lat2, lon2) = (b.0.to_radians(), b.1.to_radians());
    let dlat = lat2 - lat1;
    let dlon = lon2 - lon1;
    let h = (dlat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (dlon / 2.0).sin().powi(2);
    2.0 * radius_km * h.sqrt().asin()
}

fn note_stem(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::to_string)
}

/// A registry row for a coordinate-only proposal: ask the provider what the
/// locality there is called (REVERSE geocode — the query carries the two
/// numbers only, never the person's name or note text, README D3); when the
/// provider knows nothing there, keep a bare coordinate row rather than guess
/// (D1). Returns the place id and whether reverse geocoding named it.
pub fn resolve_unnamed_place(
    store: &PlacesStore,
    geocoder: &Geocoder,
    coordinates: (f64, f64),
    today: &str,
) -> Fallible<(String, bool)> {
    let outcome = geocoder.reverse(coordinates.0, coordinates.1, today)?;
    if let Some(place) = outcome.place {
        return Ok((place.id, true));
    }
    let label = format!("{:.4},{:.4}", coordinates.0, coordinates.1);
    let place = Place {
        id: stable_id("place", &format!("coord:{label}")),
        name: label.clone(),
        kind: "address".into(),
        address: None,
        city: None,
        country_code: None,
        latitude: Some(coordinates.0),
        longitude: Some(coordinates.1),
        source: "vault-frontmatter".into(),
        external_ref: Some(format!("coord:{label}")),
    };
    store.upsert_place(&place, today)?;
    Ok((place.id, false))
}

fn people_dir() -> Fallible<PathBuf> {
    if let Ok(dir) = std::env::var("AXON_PLACES_PEOPLE_DIR") {
        return Ok(axon_config::expand_tilde(&dir));
    }
    // The vault root the finance capability already declares in the overlay
    // (finance.json `obsidian.root`) — one declaration of where the vault is.
    let path =
        axon_config::overlay_config("finance.json").ok_or("AXON_PERSONAL_ROOT is not set")?;
    let body = std::fs::read_to_string(&path)?;
    let config: Value = serde_json::from_str(&body)?;
    let root = config
        .get("obsidian")
        .and_then(|obsidian| obsidian.get("root"))
        .and_then(Value::as_str)
        .ok_or("finance.json declares no obsidian.root; set AXON_PLACES_PEOPLE_DIR instead")?;
    Ok(axon_config::expand_tilde(root).join("Atlas/People"))
}

pub fn vault(store: &PlacesStore, today: &str) -> Fallible<()> {
    // Exported place notes in the overlay.
    let places_dir = axon_config::overlay_root()
        .ok_or("AXON_PERSONAL_ROOT is not set")?
        .join("data/places/vault-notes");
    let mut place_notes = 0_usize;
    let mut imported = 0_usize;
    let mut place_existing = 0_usize;
    let mut skipped = 0_usize;
    if places_dir.is_dir() {
        let mut paths: Vec<PathBuf> = std::fs::read_dir(&places_dir)?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("md"))
            .collect();
        paths.sort();
        for path in paths {
            place_notes += 1;
            let Some(name) = note_stem(&path) else {
                skipped += 1;
                continue;
            };
            let body = std::fs::read_to_string(&path)?;
            let (scalars, lists) = parse_frontmatter(&body);
            let Some((latitude, longitude)) = frontmatter_coordinates(&scalars, &lists) else {
                skipped += 1;
                continue;
            };
            let city = scalars.get("city").cloned().filter(|city| !city.is_empty());
            // A note that names a containing city is a venue inside it; a note
            // without one (Vienna) is the city itself.
            let kind = if city.is_some() { "venue" } else { "city" };
            let place = Place {
                id: stable_id("place", &format!("vault:{name}")),
                name: name.clone(),
                kind: kind.into(),
                address: None,
                city: city.or_else(|| (kind == "city").then(|| name.clone())),
                country_code: None,
                latitude: Some(latitude),
                longitude: Some(longitude),
                source: "vault".into(),
                external_ref: Some(format!("vault:{name}")),
            };
            if store.upsert_place(&place, today)? {
                imported += 1;
            } else {
                place_existing += 1;
            }
        }
    }

    // Person notes with populated coordinates become register PROPOSALS,
    // never confirmed rows (README D4, ISA PLC-7).
    let people = people_dir()?;
    let geocoder = Geocoder::new(store);
    let mut person_notes = 0_usize;
    let mut proposals = 0_usize;
    let mut proposals_existing = 0_usize;
    let mut reverse_named = 0_usize;
    if people.is_dir() {
        let known = store.places_with_coordinates()?;
        let mut paths: Vec<PathBuf> = std::fs::read_dir(&people)?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("md"))
            .collect();
        paths.sort();
        for path in paths {
            let Some(person) = note_stem(&path) else {
                continue;
            };
            let body = std::fs::read_to_string(&path)?;
            let (scalars, lists) = parse_frontmatter(&body);
            let Some(coordinates) = frontmatter_coordinates(&scalars, &lists) else {
                continue;
            };
            person_notes += 1;
            // Reuse the nearest registered place within 30 km; otherwise name
            // the coordinate by reverse geocode. The only thing that reaches
            // the provider is the coordinate pair — never the person's name or
            // any note text (README D3).
            let nearest = known
                .iter()
                .filter_map(|place| {
                    let position = (place.latitude?, place.longitude?);
                    let distance = haversine_km(coordinates, position);
                    (distance <= 30.0).then_some((distance, place))
                })
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(_, place)| place.id.clone());
            let place_id = match nearest {
                Some(id) => id,
                None => {
                    let (id, named) = resolve_unnamed_place(store, &geocoder, coordinates, today)?;
                    if named {
                        reverse_named += 1;
                    }
                    id
                }
            };
            // Id derived from the note's own content (person + coordinate),
            // never from the resolved place id: which registered place is
            // nearest shifts as other backfills grow the registry, and an id
            // that shifted with it would write a second proposal for the same
            // unchanged note on the next run — breaking the README's "each is
            // idempotent" promise. An edited coordinate is a changed fact and
            // correctly earns a fresh proposal.
            let id = stable_id(
                "pp",
                &format!(
                    "vault-frontmatter:{person}:{:.4},{:.4}",
                    coordinates.0, coordinates.1
                ),
            );
            if store.propose_person_place(
                &id,
                &person,
                &place_id,
                None,
                None,
                5000,
                "vault-frontmatter",
                today,
            )? {
                proposals += 1;
            } else {
                proposals_existing += 1;
            }
        }
    }

    println!(
        "backfill vault: {place_notes} place notes, {imported} places imported, \
         {place_existing} already present, {skipped} without coordinates (skipped), \
         {person_notes} person notes with coordinates, {proposals} proposals written, \
         {proposals_existing} proposals already present, \
         {reverse_named} coordinate-only places named by reverse geocode"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_recomputed_fingerprint_matches_finances_algorithm() {
        // Cross-checked against an independent SHA-256 implementation over the
        // exact tuple capabilities/finance/src/import.rs hashes: parts
        // separated by 0xff, reference empty, EUR implied.
        assert_eq!(
            amex_fingerprint(
                "2026-08-02",
                -1234,
                "SYNTHETIC MARKET BERLIN",
                "liabilities:card:amex",
                0,
            ),
            "b2ad3eca9d16b36389049659015c60ebd58e63712d904ff426cddd752a7baf57"
        );
        // The preserved-repetition rule: the second identical row in one file
        // hashes ("csv-occurrence", base, "1").
        assert_eq!(
            amex_fingerprint(
                "2026-08-02",
                -1234,
                "SYNTHETIC MARKET BERLIN",
                "liabilities:card:amex",
                1,
            ),
            "478fb1067bf98ea5ada39b358998c6dc36f0c73e0662489d147c2be026c4f901"
        );
    }

    #[test]
    fn a_synthetic_csv_row_normalizes_like_finances_import() {
        // The raw export shape: DD/MM/YYYY date, comma decimals, padded
        // description whitespace, quoted multi-line address.
        assert_eq!(
            normalize_date_dmy_slashes("03/08/2026").as_deref(),
            Some("2026-08-03")
        );
        assert_eq!(normalize_date_dmy_slashes("31/02/2026"), None);
        assert_eq!(normalize_date_dmy_slashes("2026-08-03"), None);
        assert_eq!(parse_amount_cents("44,00", ','), Some(4400));
        assert_eq!(parse_amount_cents("1.234,56", ','), Some(123_456));
        assert_eq!(parse_amount_cents("18,13", ','), Some(1813));
        assert_eq!(parse_amount_cents("not-a-number", ','), None);
        assert_eq!(
            normalize_text("SYNTHETIC MARKET        BERLIN"),
            "SYNTHETIC MARKET BERLIN"
        );
        let (street, city) = split_address("EXAMPLE STR. 1\nBERLIN", "");
        assert_eq!(street.as_deref(), Some("EXAMPLE STR. 1"));
        assert_eq!(city.as_deref(), Some("BERLIN"));
        let (street, city) = split_address("EXAMPLE STR. 1\nIGNORED", "MUNICH");
        assert_eq!(street.as_deref(), Some("EXAMPLE STR. 1"));
        assert_eq!(city.as_deref(), Some("MUNICH"));
        let (street, city) = split_address("ONLY ONE LINE", "");
        assert_eq!(street.as_deref(), Some("ONLY ONE LINE"));
        assert_eq!(city, None);
    }

    #[test]
    fn the_amex_profile_is_read_from_the_finance_config_shape() {
        let config = json!({
            "csv_mappings": [
                { "label": "Some bank", "mapping": { "date_column": "Buchung" } },
                {
                    "label": "American Express CSV export",
                    "mapping": {
                        "delimiter": ",",
                        "decimal_separator": ",",
                        "date_column": "Datum",
                        "amount_column": "Betrag",
                        "description_column": "Beschreibung",
                        "source_account": "liabilities:card:synthetic",
                        "amount_sign": "invert"
                    }
                }
            ]
        });
        let profile = amex_profile_from(&config).unwrap();
        assert_eq!(profile.source_account, "liabilities:card:synthetic");
        assert!(profile.invert);
        assert_eq!(profile.delimiter, b',');
        assert!(amex_profile_from(&json!({ "csv_mappings": [] })).is_err());
    }

    #[test]
    fn city_tokens_are_matched_conservatively_from_the_end() {
        // Synthetic descriptors only (the Musterstadt convention) — never
        // verbatim live transaction text in this public file.
        let cities: HashSet<String> = ["MUSTERSTADT", "BAD MUSTERSTADT", "BEISPIELHAUSEN"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            trailing_city("MARKT 12345678 MUSTERSTADT", &cities),
            Some("MUSTERSTADT")
        );
        // The longest match wins, and whitespace runs collapse first.
        assert_eq!(
            trailing_city("CAFE   BAD MUSTERSTADT", &cities),
            Some("BAD MUSTERSTADT")
        );
        assert_eq!(
            trailing_city("PAYPAL *SYNTHETIC GAME 4029357733", &cities),
            None
        );
        // A city as word prefix, not trailing token, never matches.
        assert_eq!(trailing_city("MUSTERSTADTER KINDL", &cities), None);
        assert!(plausible_city("BAD MUSTERSTADT"));
        assert!(!plausible_city("4029357733"));
        assert!(!plausible_city("A0"));
    }

    #[test]
    fn traveler_names_and_destinations_parse_the_trips_shapes() {
        assert_eq!(
            traveler_names(r#"["Synthetic Person", "  ", "Second Person"]"#),
            vec!["Synthetic Person".to_string(), "Second Person".to_string()]
        );
        assert!(traveler_names("[]").is_empty());
        assert!(traveler_names("not json").is_empty());

        // The first destination with coordinates wins; kinds map onto the
        // registry's vocabulary (airport → station, trips' default → station).
        let destinations = r#"[
            {"id": "d0", "name": "Nowhere", "kind": "city"},
            {"id": "d1", "name": "  Musterstadt ", "kind": "city",
             "latitude": 50.0, "longitude": 7.0},
            {"id": "d2", "name": "Later", "kind": "venue",
             "latitude": 51.0, "longitude": 8.0}
        ]"#;
        assert_eq!(
            first_located_destination(destinations),
            Some(("Musterstadt".into(), "city".into(), 50.0, 7.0))
        );
        let airport = r#"[{"id": "d3", "name": "Synthetic Airport", "kind": "airport",
                           "latitude": 50.1, "longitude": 7.1}]"#;
        assert_eq!(
            first_located_destination(airport).map(|d| d.1),
            Some("station".into())
        );
        assert_eq!(first_located_destination("[]"), None);
        assert_eq!(
            first_located_destination(r#"[{"id": "d4", "name": "No coords"}]"#),
            None
        );
    }

    #[test]
    fn frontmatter_coordinates_parse_both_vault_spellings() {
        let list_form = "---\ntype: museum\ncity: New York\ncoordinates:\n  - \"40.7033\"\n  - \"-73.9894\"\nicon: pin\n---\nbody\n";
        let (scalars, lists) = parse_frontmatter(list_form);
        assert_eq!(
            frontmatter_coordinates(&scalars, &lists),
            Some((40.7033, -73.9894))
        );
        assert_eq!(scalars.get("city").map(String::as_str), Some("New York"));

        let inline_form = "---\nicon: user-round\ncoordinates: 49.8920,8.6480\n---\n";
        let (scalars, lists) = parse_frontmatter(inline_form);
        assert_eq!(
            frontmatter_coordinates(&scalars, &lists),
            Some((49.892, 8.648))
        );

        let empty = "---\ncoordinates: []\n---\n";
        let (scalars, lists) = parse_frontmatter(empty);
        assert_eq!(frontmatter_coordinates(&scalars, &lists), None);

        let absent = "no frontmatter at all";
        let (scalars, lists) = parse_frontmatter(absent);
        assert_eq!(frontmatter_coordinates(&scalars, &lists), None);
    }

    #[test]
    fn haversine_matches_a_known_distance() {
        // Vienna Hauptbahnhof-ish to Vienna centre: a few kilometres.
        let d = haversine_km((48.1849, 16.3673), (48.2085, 16.3721));
        assert!(d > 2.0 && d < 4.0, "got {d}");
        // Vienna to Darmstadt: several hundred kilometres, never "nearby".
        let far = haversine_km((48.1849, 16.3673), (49.8920, 8.6480));
        assert!(far > 500.0, "got {far}");
    }
}

/// Postgres-backed backfill tests. Synthetic fixtures only (the Musterstadt
/// convention); source tables the backfills normally read cross-schema
/// (`finance.transaction_candidates`, `trips.plans`) are scratch copies inside
/// the test's own schema, which is what the schema parameters exist for.
#[cfg(test)]
mod pg_tests {
    use super::*;
    use crate::geocode::pg_tests::{stub, stub_with};
    use crate::store::pg_tests::open_test_store;
    use std::sync::atomic::Ordering;

    const SOURCE_ACCOUNT: &str = "liabilities:card:synthetic";
    const VENUE_ITEM: &str = r#"[{"osm_type":"node","osm_id":7001,"lat":"50.0002","lon":"7.0002","name":"Synthetic Market","display_name":"Synthetic Market, Musterstadt","addresstype":"shop","address":{"town":"Musterstadt","country_code":"de"}}]"#;
    const CITY_ITEM: &str = r#"[{"osm_type":"relation","osm_id":7002,"lat":"50.0","lon":"7.0","name":"Musterstadt","display_name":"Musterstadt, Germany","addresstype":"city","address":{"city":"Musterstadt","country_code":"de"}}]"#;

    fn transaction_places(store: &PlacesStore) -> Vec<(String, String, String)> {
        let mut conn = store.conn().unwrap();
        conn.query(
            &format!(
                "SELECT source_id, precision, source FROM {}.transaction_places
                 ORDER BY source_id",
                store.schema()
            ),
            &[],
        )
        .unwrap()
        .iter()
        .map(|row| (row.get(0), row.get(1), row.get(2)))
        .collect()
    }

    #[test]
    fn candidate_locations_wait_for_the_finance_migration() {
        let (store, schema) = open_test_store("cands_absent");
        {
            let mut conn = store.conn().unwrap();
            // The candidates table as it exists before finance's ALTER TABLE
            // adds the location columns.
            conn.batch_execute(&format!(
                "CREATE TABLE {}.transaction_candidates (
                     fingerprint TEXT PRIMARY KEY,
                     source_account TEXT NOT NULL
                 )",
                schema.0
            ))
            .unwrap();
        }
        // Port 1 refuses connections: proving no geocode is even attempted.
        let geocoder = Geocoder::with_url(&store, "http://127.0.0.1:1/search".into());
        let phase = amex_from_candidates(
            &store,
            &geocoder,
            &schema.0,
            SOURCE_ACCOUNT,
            &HashSet::new(),
            "2026-08-25",
        )
        .unwrap();
        assert!(phase.is_none(), "absent columns mean no candidate phase");
    }

    #[test]
    fn stored_candidate_locations_link_venues_and_fall_back_to_the_city() {
        let (store, schema) = open_test_store("cands");
        {
            let mut conn = store.conn().unwrap();
            conn.batch_execute(&format!(
                "CREATE TABLE {}.transaction_candidates (
                     fingerprint TEXT PRIMARY KEY,
                     source_account TEXT NOT NULL,
                     location_street TEXT,
                     location_postal_code TEXT,
                     location_city TEXT,
                     location_country TEXT
                 )",
                schema.0
            ))
            .unwrap();
            // The contract shape: Adresse verbatim with its embedded newline
            // (street on line 1, city on line 2), PLZ and Land beside it.
            // fp-cityonly is a whitespace-only Adresse with a filled Stadt: no
            // street exists, so the venue path must never see it (D1).
            conn.execute(
                &format!(
                    "INSERT INTO {}.transaction_candidates VALUES
                     ('fp-venue', $1, E'Beispielstr. 1\\nMusterstadt', '12345', NULL, 'Deutschland'),
                     ('fp-cityfb', $1, E'Unresolvable Weg 9\\nMusterstadt', '12345', NULL, 'Deutschland'),
                     ('fp-cityonly', $1, E'\\n', '12345', 'Musterstadt', 'Deutschland'),
                     ('fp-linked', $1, E'Beispielstr. 1\\nMusterstadt', '12345', NULL, 'Deutschland'),
                     ('fp-noloc', $1, NULL, NULL, NULL, NULL),
                     ('fp-other', 'liabilities:card:other', E'Beispielstr. 1\\nMusterstadt', NULL, NULL, NULL)",
                    schema.0
                ),
                &[&SOURCE_ACCOUNT],
            )
            .unwrap();
        }
        // fp-linked already carries a link (a previous run's), so this run
        // must leave it alone.
        let existing = Place {
            id: stable_id("place", "test:prelinked"),
            name: "Prelinked Venue".into(),
            kind: "venue".into(),
            address: None,
            city: Some("Musterstadt".into()),
            country_code: Some("DE".into()),
            latitude: Some(50.0),
            longitude: Some(7.0),
            source: "test".into(),
            external_ref: None,
        };
        store.upsert_place(&existing, "2026-08-25").unwrap();
        store
            .link_transaction(
                "fp-linked",
                &existing.id,
                "venue",
                9000,
                "test",
                "2026-08-25",
            )
            .unwrap();

        // The stub resolves the good street, misses the bad one, and knows the
        // city, exercising both halves of the venue-then-city path.
        let (url, hits) = stub_with(|request: &str| {
            if request.contains("street=Beispielstr") {
                VENUE_ITEM.to_string()
            } else if request.contains("street=") {
                "[]".to_string()
            } else {
                CITY_ITEM.to_string()
            }
        });
        let geocoder = Geocoder::with_url(&store, url);

        let (stats, covered) = amex_from_candidates(
            &store,
            &geocoder,
            &schema.0,
            SOURCE_ACCOUNT,
            &store.linked_source_ids().unwrap(),
            "2026-08-25",
        )
        .unwrap()
        .expect("the location columns exist");

        // fp-noloc has no location_street and fp-other is another account:
        // neither is a candidate of this phase.
        assert_eq!(covered.len(), 4);
        assert!(covered.contains("fp-venue") && covered.contains("fp-cityfb"));
        assert_eq!(stats.linked, 1);
        assert_eq!(
            stats.city_fallback, 2,
            "the venue miss and the street-less row each earn a city link"
        );
        assert_eq!(stats.skipped_linked, 1);
        assert_eq!(stats.not_found, 0);
        assert_eq!(
            transaction_places(&store),
            vec![
                (
                    "fp-cityfb".to_string(),
                    "city".to_string(),
                    "amex-city-fallback".to_string()
                ),
                (
                    "fp-cityonly".to_string(),
                    "city".to_string(),
                    "amex-city-fallback".to_string()
                ),
                (
                    "fp-linked".to_string(),
                    "venue".to_string(),
                    "test".to_string()
                ),
                (
                    "fp-venue".to_string(),
                    "venue".to_string(),
                    "amex-backfill".to_string()
                ),
            ]
        );
        let first_run_hits = hits.load(Ordering::SeqCst);
        assert_eq!(
            first_run_hits, 3,
            "venue hit + venue miss + city hit; the street-less row's city \
             query is served from the cache and never reaches the venue path"
        );

        // Idempotent: a re-run links nothing new and never re-asks the
        // provider (every row is now linked, every query cached).
        let (stats, _) = amex_from_candidates(
            &store,
            &geocoder,
            &schema.0,
            SOURCE_ACCOUNT,
            &store.linked_source_ids().unwrap(),
            "2026-08-25",
        )
        .unwrap()
        .unwrap();
        assert_eq!(stats.linked, 0);
        assert_eq!(stats.city_fallback, 0);
        assert_eq!(stats.skipped_linked, 4);
        assert_eq!(hits.load(Ordering::SeqCst), first_run_hits);
    }

    #[test]
    fn travelers_become_proposals_and_never_confirmed_rows() {
        let (store, schema) = open_test_store("travelers");
        {
            let mut conn = store.conn().unwrap();
            conn.batch_execute(&format!(
                "CREATE TABLE {}.plans (
                     id TEXT PRIMARY KEY,
                     travelers TEXT NOT NULL,
                     destinations TEXT NOT NULL,
                     date_start TEXT NOT NULL,
                     date_end TEXT NOT NULL,
                     status TEXT NOT NULL
                 )",
                schema.0
            ))
            .unwrap();
            conn.execute(
                &format!(
                    "INSERT INTO {}.plans VALUES
                     ('plan-1', '[\"Synthetic Person\", \"Second Person\"]',
                      '[{{\"id\": \"d1\", \"name\": \"Musterstadt\", \"kind\": \"city\",
                          \"latitude\": 50.0, \"longitude\": 7.0}}]',
                      '2026-09-01', '2026-09-05', 'saved'),
                     ('plan-2', '[\"Synthetic Person\"]',
                      '[{{\"id\": \"d2\", \"name\": \"Nowhere\", \"kind\": \"city\"}}]',
                      '2026-10-01', '2026-10-03', 'saved'),
                     ('plan-3', '[\"Synthetic Person\"]',
                      '[{{\"id\": \"d3\", \"name\": \"Musterstadt\", \"kind\": \"city\",
                          \"latitude\": 50.0, \"longitude\": 7.0}}]',
                      '2026-11-01', '2026-11-03', 'archived'),
                     ('plan-4', '[]',
                      '[{{\"id\": \"d4\", \"name\": \"Musterstadt\", \"kind\": \"city\",
                          \"latitude\": 50.0, \"longitude\": 7.0}}]',
                      '2026-12-01', '2026-12-03', 'saved')",
                    schema.0
                ),
                &[],
            )
            .unwrap();
        }

        let report = travelers_from(&store, &schema.0, "2026-08-25").unwrap();
        assert_eq!(report.plans, 3, "the archived plan is not read");
        assert_eq!(report.without_located_destination, 1);
        assert_eq!(report.pairs, 2);
        assert_eq!(report.proposals_written, 2);
        assert_eq!(report.places_created, 1);

        let proposed = store.person_places_in_state("proposed").unwrap();
        assert_eq!(proposed.len(), 2);
        for row in &proposed {
            assert_eq!(row.state, "proposed");
            assert_eq!(row.source, "trips-travelers");
            assert_eq!(row.date_start.as_deref(), Some("2026-09-01"));
            assert_eq!(row.date_end.as_deref(), Some("2026-09-05"));
            assert_eq!(row.confidence_bp, 5000);
        }
        // PLC-7: derivation must never produce a confirmed row.
        assert!(store
            .person_places_in_state("confirmed")
            .unwrap()
            .is_empty());

        // Idempotent by (person, plan): a re-run writes nothing new.
        let again = travelers_from(&store, &schema.0, "2026-08-25").unwrap();
        assert_eq!(again.proposals_written, 0);
        assert_eq!(again.proposals_existing, 2);
        assert_eq!(again.places_created, 0);
    }

    #[test]
    fn an_unnamed_coordinate_is_named_by_reverse_geocode_or_kept_bare() {
        let (store, _schema) = open_test_store("revname");
        let (url, _hits) = stub(
            r#"{"osm_type":"relation","osm_id":7003,"lat":"50.10","lon":"7.10","name":"Musterstadt","display_name":"Musterstadt, Germany","addresstype":"city","address":{"city":"Musterstadt","country_code":"de"}}"#,
        );
        let geocoder = Geocoder::with_url(&store, url);
        let (place_id, named) =
            resolve_unnamed_place(&store, &geocoder, (50.1234, 7.1234), "2026-08-25").unwrap();
        assert!(named);
        let place = store.place(&place_id).unwrap().unwrap();
        assert_eq!(place.name, "Musterstadt");
        assert_eq!(place.kind, "city");

        // When the provider knows nothing there, the row stays a bare
        // coordinate — reported precision, never a guessed name (D1).
        let (url, _hits) = stub(r#"{"error":"Unable to geocode"}"#);
        let geocoder = Geocoder::with_url(&store, url);
        let (place_id, named) =
            resolve_unnamed_place(&store, &geocoder, (10.9876, 9.8765), "2026-08-25").unwrap();
        assert!(!named);
        let place = store.place(&place_id).unwrap().unwrap();
        assert_eq!(place.name, "10.9876,9.8765");
        assert_eq!(place.kind, "address");
    }
}
