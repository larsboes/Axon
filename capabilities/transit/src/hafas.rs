//! HAFAS client: talks to bahn.de's internal, undocumented journey-search API
//! (the same one bahn.de's own website calls from the browser -- no key, no
//! auth, no public docs). Two endpoints:
//!   - POST /web/api/angebote/fahrplan   (journey search; also the pairwise
//!     query the split-ticket solver uses)
//!   - GET  /web/api/reiseloesung/orte    (station name -> EVA id lookup)
//!
//! **Not ported from the original: ONNX delay-risk prediction.** The source
//! service loaded a `tract-onnx` model (`infra/data/model.onnx`) to fill
//! `Journey.delay_risk_score`. Axon has no such model artifact -- the
//! source monorepo's delay-training pipeline was rated quarry-for-patterns-only
//! in the original migration evaluation and has now been removed from Axon.
//! Carrying `tract-onnx` (a heavy dependency) plus hand-rolled date
//! math for a field that would only ever return a constant 0.15 fallback is
//! the exact "machinery with nothing behind it" anti-pattern this repo
//! already strips elsewhere (scouting's CV generator, its HTTP server). The
//! field stays in `travel::Journey` (always `None` *here*) so filling it later
//! is additive, not a schema break.
//!
//! It is filled now, one layer up: `transit::punctuality` asks
//! `capabilities/punctuality` for measured lateness and enriches the journeys
//! this module returns. Deliberately not done inside this module -- HAFAS
//! parsing should stay a pure function of the HAFAS response, and the tests
//! below assert exactly that by checking the field is still `None` on the way
//! out of here.

use crate::travel::{
    Journey, Leg, SplitConfidence, SplitResult, SplitSegment, Station, TrainMatch,
};
use reqwest::blocking::Client;
use serde_json::{json, Value};
use std::collections::HashMap;

// Spoofed real-browser UA sent to bahn.de -- kept deliberately, not
// genericized to a self-identifying "Axon-Transit/0.1" string. This mirrors
// `capabilities/scouting/src/adapters/meetup.rs`'s precedent, not the
// self-identifying-UA one (source.rs/cfp_conferences/luma): this endpoint is
// undocumented and ungated only because it looks like normal browser
// traffic. A self-identifying UA here would plausibly just get blocked
// outright rather than "politely identify us" -- there's no ToS/robots.txt
// contract to honor on an endpoint that was never meant to be called this
// way. Flagged here and in README Gotchas rather than hidden.
const BROWSER_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

const FAHRPLAN_URL: &str = "https://www.bahn.de/web/api/angebote/fahrplan";
const ORTE_URL: &str = "https://www.bahn.de/web/api/reiseloesung/orte";

#[derive(Debug, thiserror::Error)]
pub enum HafasError {
    #[error("request failed: {0}")]
    Request(String),
    #[error("HAFAS query failed with status {status}: {body}")]
    BadStatus { status: u16, body: String },
    #[error("could not parse response: {0} (body: {1})")]
    Parse(String, String),
    /// No cheaper combination exists for this route. An outcome, not a failure: the
    /// query was fine and the honest answer is "none". Its own variant rather than an
    /// Other(String) the caller has to string-match, because the HTTP layer has to map
    /// it to a different status than a broken upstream.
    #[error("no split-ticket combination is cheaper than the direct fare here")]
    NoSplitFound,
    #[error("{0}")]
    Other(String),
}

/// Fare context carried into every bahn.de query. The vendor's own pricing
/// engine applies the discount, so returned fares are discount-correct per
/// leg -- which is what the split solver needs, because BahnCard applies per
/// Fahrkarte (BB C.2 Nr. 2.1) and every split segment is its own Fahrkarte.
/// A Deutschlandticket additionally zeroes pure regional connections on the
/// vendor's side.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FareOptions {
    /// 25 or 50; anything else fails the payload builder loudly.
    pub bahncard: Option<u8>,
    pub first_class: bool,
    pub deutschland_ticket: bool,
}

pub struct HafasClient {
    client: Client,
}

impl Default for HafasClient {
    fn default() -> Self {
        Self::new()
    }
}

impl HafasClient {
    pub fn new() -> Self {
        // A bare `Client::new()` has NO request timeout -- a slow or hung
        // response from bahn.de's undocumented endpoint blocks the calling
        // thread forever (this is exactly what happened during the port's
        // own live smoke test: a `--search` call stalled past 600s with no
        // recovery). 15s is generous for a single journey-search POST; a
        // real network issue should fail fast and loud, not hang silently.
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .expect("reqwest client with a fixed timeout should always build");
        Self { client }
    }

    fn fahrplan_payload(
        from_eva: &str,
        to_eva: &str,
        datetime: &str,
        fare: &FareOptions,
    ) -> Result<Value, HafasError> {
        let klasse = if fare.first_class { "KLASSE_1" } else { "KLASSE_2" };
        let ermaessigung = match fare.bahncard {
            None => json!({"art": "KEINE_ERMAESSIGUNG", "klasse": "KLASSENLOS"}),
            // Enum strings verified against db-vendo-client's
            // format/loyalty-cards.js (fetched 2026-08-12): the ermaessigung
            // klasse is the CARD's class; one first_class knob drives both.
            Some(25) => json!({"art": "BAHNCARD25", "klasse": klasse}),
            Some(50) => json!({"art": "BAHNCARD50", "klasse": klasse}),
            Some(other) => {
                return Err(HafasError::Other(format!(
                    "bahncard must be 25 or 50, got {other}"
                )))
            }
        };
        Ok(json!({
            "abfahrtsHalt": from_eva,
            "anfrageZeitpunkt": datetime,
            "ankunftsHalt": to_eva,
            "ankunftSuche": "ABFAHRT",
            "klasse": klasse,
            "produktgattungen": ["ICE", "EC_IC", "IR", "REGIONAL", "SBAHN", "BUS", "SCHIFF", "UBAHN", "TRAM", "ANRUFPFLICHTIG"],
            "reisende": [{
                "typ": "ERWACHSENER",
                "ermaessigungen": [ermaessigung],
                "anzahl": 1,
                "alter": []
            }],
            "schnelleVerbindungen": true,
            "deutschlandTicketVorhanden": fare.deutschland_ticket
        }))
    }

    /// Direct journey search between two EVA station codes.
    pub fn search_connections(
        &self,
        from_eva: &str,
        to_eva: &str,
        datetime: &str,
        fare: &FareOptions,
    ) -> Result<Vec<Journey>, HafasError> {
        let payload = Self::fahrplan_payload(from_eva, to_eva, datetime, fare)?;

        let response = self
            .client
            .post(FAHRPLAN_URL)
            .header("User-Agent", BROWSER_UA)
            .header("Accept", "application/json")
            .header("Content-Type", "application/json; charset=UTF-8")
            .json(&payload)
            .send()
            .map_err(|e| HafasError::Request(e.to_string()))?;

        let status = response.status();
        let text = response
            .text()
            .map_err(|e| HafasError::Request(e.to_string()))?;
        if !status.is_success() {
            return Err(HafasError::BadStatus {
                status: status.as_u16(),
                body: text,
            });
        }

        let body: Value =
            serde_json::from_str(&text).map_err(|e| HafasError::Parse(e.to_string(), text))?;
        Ok(parse_journeys_from_response(&body))
    }

    /// Station name -> EVA id search (autocomplete-style).
    pub fn suggest_stations(&self, query: &str) -> Result<Vec<Station>, HafasError> {
        let response = self
            .client
            .get(ORTE_URL)
            .query(&[("suchbegriff", query), ("typ", "ALL"), ("limit", "10")])
            .header("User-Agent", BROWSER_UA)
            .header("Accept", "application/json")
            .send()
            .map_err(|e| HafasError::Request(e.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            return Err(HafasError::BadStatus {
                status: status.as_u16(),
                body: String::new(),
            });
        }

        let list: Vec<Value> = response
            .json()
            .map_err(|e| HafasError::Request(e.to_string()))?;
        Ok(parse_suggest_response(&list))
    }

    /// Cheapest split-ticket search: finds every reasonable intermediate stop
    /// on the direct connection, prices every pairwise segment, and picks the
    /// cheapest way to stitch consecutive segments together end to end.
    ///
    /// Deliberately sequential, not bounded-concurrent (the original used a
    /// `tokio::task::JoinSet` + `Semaphore(2)`) -- see README's "Known gaps":
    /// this is a personal, low-frequency CLI tool, and avoiding tokio as a
    /// dependency (matching scouting's no-async-runtime precedent) is worth
    /// more than shaving a few seconds off an occasional split-ticket search.
    /// The 250ms inter-request pause is preserved.
    pub fn search_split_tickets(
        &self,
        from_eva: &str,
        to_eva: &str,
        datetime: &str,
        fare: &FareOptions,
    ) -> Result<SplitResult, HafasError> {
        let payload = Self::fahrplan_payload(from_eva, to_eva, datetime, fare)?;
        let response = self
            .client
            .post(FAHRPLAN_URL)
            .header("User-Agent", BROWSER_UA)
            .header("Accept", "application/json")
            .header("Content-Type", "application/json; charset=UTF-8")
            .json(&payload)
            .send()
            .map_err(|e| HafasError::Request(e.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            return Err(HafasError::BadStatus {
                status: status.as_u16(),
                body: String::new(),
            });
        }

        let body: Value = response
            .json()
            .map_err(|e| HafasError::Request(e.to_string()))?;
        let verbindungen = body
            .get("verbindungen")
            .and_then(|v| v.as_array())
            .ok_or_else(|| HafasError::Other("no connections found in Vendo response".into()))?;
        if verbindungen.is_empty() {
            return Err(HafasError::Other("no connection found".into()));
        }
        let v = &verbindungen[0];
        let direct_price = v
            .get("angebotsPreis")
            .and_then(|p| p.get("betrag"))
            .and_then(|b| b.as_f64());

        let stops = extract_stops(v);
        let n = stops.len();
        if n < 2 {
            return Err(HafasError::Other(
                "not enough stops to perform split-ticketing".into(),
            ));
        }

        // Which train the traveller is actually on over each stop pair. Read from
        // the same direct-journey payload the stops came from, so classifying a
        // segment costs no extra request.
        let spans = extract_section_spans(v, &stops);

        let mut prices: HashMap<(usize, usize), f64> = HashMap::new();
        let mut segments_data: HashMap<(usize, usize), Journey> = HashMap::new();
        let mut queried_pairs = 0usize;
        let mut unpriced_pairs = 0usize;

        for i in 0..n {
            for j in (i + 1)..n {
                std::thread::sleep(std::time::Duration::from_millis(250));
                queried_pairs += 1;
                // A failed query used to disappear here, so the DP quietly ran
                // against a table with holes in it and the caller was told the
                // answer was the cheapest chain that exists. It is counted now.
                // Same fare context as the direct query: BahnCard applies per
                // Fahrkarte, so every candidate segment is priced the way it
                // would actually be bought.
                let priced = match self.search_connections(
                    &stops[i].ext_id,
                    &stops[j].ext_id,
                    &stops[i].departure_iso,
                    fare,
                ) {
                    Ok(journeys) => match journeys.first() {
                        Some(first) => first.total_price.map(|price| (price, first.clone())),
                        None => None,
                    },
                    Err(_) => None,
                };
                match priced {
                    Some((price, journey)) => {
                        prices.insert((i, j), price);
                        segments_data.insert((i, j), journey);
                    }
                    None => unpriced_pairs += 1,
                }
            }
        }

        let (split_price, path) = cheapest_split(n, &prices).ok_or(HafasError::NoSplitFound)?;

        let segments: Vec<SplitSegment> = path
            .into_iter()
            .filter_map(|(i, j)| {
                segments_data.get(&(i, j)).cloned().map(|journey| {
                    let expected_trains = expected_trains(&spans, i, j);
                    let train_match = classify_train_match(&expected_trains, &journey);
                    SplitSegment {
                        journey,
                        train_match,
                        expected_trains,
                    }
                })
            })
            .collect();

        let confidence = split_confidence(&segments, unpriced_pairs);

        Ok(SplitResult {
            original_price: direct_price,
            split_price,
            savings: direct_price.map(|p| p - split_price),
            segments,
            confidence,
            unpriced_pairs,
            queried_pairs,
        })
    }
}

struct Stop {
    ext_id: String,
    departure_iso: String,
}

/// A stop's scheduled and real-time value for one event, kept apart.
///
/// Both are read every time. Folding them with `or_else` on the way in is what
/// made a delay invisible: the field held whichever existed and nothing recorded
/// which one it was.
fn times_of(halt: &Value, event: &str) -> (Option<String>, Option<String>) {
    let read = |key: &str| {
        halt.get(event)
            .and_then(|e| e.get(key))
            .and_then(|t| t.as_str())
            .map(str::to_string)
    };
    // `echtzeit`, captured from a live response. The code this replaced fell
    // back to `istzeit`, which this endpoint does not serve at all, so the
    // fallback never once fired and every journey silently carried its
    // scheduled time as though it were the real one. The same shape as the
    // `id`/`tripId` bug in Gotchas: a wrong key name that fails as silence.
    (read("sollzeit"), read("echtzeit"))
}

/// Whether HAFAS marked something cancelled.
///
/// The flag appears under several names across the response depending on where
/// it sits, and a missing flag means not cancelled -- never unknown, because
/// HAFAS omits it for the ordinary case.
fn is_cancelled(node: &Value) -> bool {
    // Both captured from a real section. A missing flag means not cancelled:
    // bahn.de omits them for the ordinary case rather than sending false.
    ["originCancelled", "destinationCancelled"]
        .iter()
        .any(|key| node.get(*key).and_then(|v| v.as_bool()).unwrap_or(false))
}

/// One non-WALK section of the direct journey, expressed as the stop-index pair
/// it spans plus the train that covers it.
struct SectionSpan {
    from: usize,
    to: usize,
    train_number: String,
}

/// Maps each non-WALK section of the direct journey onto the stop indices
/// `extract_stops` produced, so a stop pair can be asked which trains it rides.
///
/// Done as a second pass rather than inside `extract_stops` because that function
/// deduplicates by station id: a transfer station is pushed once, while processing
/// the section that *arrives* there, and the train it later departs on is not known
/// at that moment. Matching by station id afterwards has no such ordering problem.
fn extract_section_spans(v: &Value, stops: &[Stop]) -> Vec<SectionSpan> {
    let index_of = |ext_id: &str| stops.iter().position(|s| s.ext_id == ext_id);
    let mut spans = Vec::new();
    let Some(sections) = v.get("verbindungsAbschnitte").and_then(|s| s.as_array()) else {
        return spans;
    };
    for section in sections {
        let verkehrsmittel = section.get("verkehrsmittel").cloned().unwrap_or(Value::Null);
        if verkehrsmittel.get("typ").and_then(|t| t.as_str()) == Some("WALK") {
            continue;
        }
        let Some(halte) = section.get("halte").and_then(|h| h.as_array()) else {
            continue;
        };
        if halte.len() < 2 {
            continue;
        }
        let halt_id = |halt: &Value| -> Option<String> {
            halt.get("id")
                .or_else(|| halt.get("extId"))
                .and_then(|id| id.as_str())
                .map(|s| s.to_string())
        };
        let (Some(from_id), Some(to_id)) = (halt_id(&halte[0]), halt_id(halte.last().unwrap()))
        else {
            continue;
        };
        let (Some(from), Some(to)) = (index_of(&from_id), index_of(&to_id)) else {
            continue;
        };
        let train_number = verkehrsmittel
            .get("nummer")
            .or_else(|| verkehrsmittel.get("linienNummer"))
            .and_then(|n| n.as_str())
            .unwrap_or("")
            .to_string();
        spans.push(SectionSpan {
            from,
            to,
            train_number,
        });
    }
    spans
}

/// The trains the direct journey uses between two stop indices, in route order.
fn expected_trains(spans: &[SectionSpan], i: usize, j: usize) -> Vec<String> {
    spans
        .iter()
        .filter(|s| s.from >= i && s.to <= j && !s.train_number.is_empty())
        .map(|s| s.train_number.clone())
        .collect()
}

/// Whether a separately-priced journey is the same ride as the planned one.
pub fn classify_train_match(expected: &[String], journey: &Journey) -> TrainMatch {
    let actual: Vec<&str> = journey
        .legs
        .iter()
        .map(|l| l.train_number.as_str())
        .filter(|n| !n.is_empty())
        .collect();
    if expected.is_empty() || actual.is_empty() {
        return TrainMatch::Unknown;
    }
    if expected.len() == actual.len() && expected.iter().zip(&actual).all(|(e, a)| e == a) {
        return TrainMatch::Exact;
    }
    if actual.iter().any(|a| expected.iter().any(|e| e == a)) {
        return TrainMatch::Partial;
    }
    TrainMatch::Different
}

/// One value a caller can gate on, taking the worst case across the chain: a
/// chain is only as buyable as its least trustworthy ticket.
pub fn split_confidence(segments: &[SplitSegment], unpriced_pairs: usize) -> SplitConfidence {
    if segments
        .iter()
        .any(|s| s.train_match == TrainMatch::Different)
    {
        return SplitConfidence::Low;
    }
    if unpriced_pairs > 0
        || segments
            .iter()
            .any(|s| s.train_match != TrainMatch::Exact)
    {
        return SplitConfidence::Partial;
    }
    SplitConfidence::Exact
}

fn extract_stops(v: &Value) -> Vec<Stop> {
    let mut stops = Vec::new();
    let Some(sections) = v.get("verbindungsAbschnitte").and_then(|s| s.as_array()) else {
        return stops;
    };
    for section in sections {
        let verkehrsmittel = section
            .get("verkehrsmittel")
            .cloned()
            .unwrap_or(Value::Null);
        let typ = verkehrsmittel
            .get("typ")
            .and_then(|t| t.as_str())
            .unwrap_or("");
        if typ == "WALK" {
            continue;
        }
        let Some(halte) = section.get("halte").and_then(|h| h.as_array()) else {
            continue;
        };
        if halte.len() < 2 {
            continue;
        }
        for halt in [&halte[0], halte.last().unwrap()] {
            let Some(ext_id) = halt
                .get("id")
                .or_else(|| halt.get("extId"))
                .and_then(|id| id.as_str())
            else {
                continue;
            };
            if stops.iter().any(|s: &Stop| s.ext_id == ext_id) {
                continue;
            }
            let departure_iso = halt
                .get("abfahrt")
                .and_then(|a| a.get("sollzeit").or_else(|| a.get("istzeit")))
                .and_then(|t| t.as_str())
                .or_else(|| {
                    halt.get("ankunft")
                        .and_then(|a| a.get("sollzeit").or_else(|| a.get("istzeit")))
                        .and_then(|t| t.as_str())
                })
                .unwrap_or("")
                .to_string();
            stops.push(Stop {
                ext_id: ext_id.to_string(),
                departure_iso,
            });
        }
    }
    stops
}

/// Pure DP core of the split-ticket solver: given `n` stops (0..n, in route
/// order) and known fares between some pairs, finds the cheapest way to
/// stitch together consecutive purchased segments from stop 0 to stop n-1.
/// No network, no HAFAS types -- just prices in, (total, path) out. This is
/// the actual algorithm the original evaluation flagged as "zero tests
/// despite being the riskiest code"; see the tests below.
pub fn cheapest_split(
    n: usize,
    segment_prices: &HashMap<(usize, usize), f64>,
) -> Option<(f64, Vec<(usize, usize)>)> {
    if n == 0 {
        return None;
    }
    let mut dp = vec![f64::INFINITY; n];
    dp[0] = 0.0;
    let mut parent: Vec<Option<usize>> = vec![None; n];

    for i in 1..n {
        for j in 0..i {
            if let Some(price) = segment_prices.get(&(j, i)) {
                let cost = dp[j] + price;
                if cost < dp[i] {
                    dp[i] = cost;
                    parent[i] = Some(j);
                }
            }
        }
    }

    if dp[n - 1] == f64::INFINITY {
        return None;
    }

    let mut path = Vec::new();
    let mut curr = n - 1;
    while let Some(prev) = parent[curr] {
        path.push((prev, curr));
        curr = prev;
    }
    path.reverse();
    Some((dp[n - 1], path))
}

/// Pure JSON -> `Journey` parser, extracted out of `search_connections` so it
/// can be unit tested against a fixture without any network call. This is
/// the hand-rolled parsing of an undocumented, reverse-engineered response
/// shape -- the other piece the original evaluation flagged as untested.
pub fn parse_journeys_from_response(body: &Value) -> Vec<Journey> {
    let mut journeys = Vec::new();
    let Some(verbindungen) = body.get("verbindungen").and_then(|v| v.as_array()) else {
        return journeys;
    };

    for v in verbindungen {
        let mut legs = Vec::new();
        if let Some(sections) = v.get("verbindungsAbschnitte").and_then(|s| s.as_array()) {
            for section in sections {
                let verkehrsmittel = section
                    .get("verkehrsmittel")
                    .cloned()
                    .unwrap_or(Value::Null);
                let name = verkehrsmittel
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                let number = verkehrsmittel
                    .get("nummer")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                let category = verkehrsmittel
                    .get("kategorie")
                    .and_then(|c| c.as_str())
                    .unwrap_or("")
                    .to_string();

                let attributes = verkehrsmittel
                    .get("zugattribute")
                    .and_then(|a| a.as_array());
                let is_regional = attributes
                    .map(|attrs| {
                        attrs
                            .iter()
                            .any(|attr| attr.get("key").and_then(|k| k.as_str()) == Some("9G"))
                    })
                    .unwrap_or(false);

                if let Some(halts) = section.get("halte").and_then(|h| h.as_array()) {
                    if halts.len() >= 2 {
                        let origin_halt = &halts[0];
                        let dest_halt = halts.last().unwrap();

                        let origin_station = Station {
                            id: origin_halt
                                .get("id")
                                .and_then(|id| id.as_str())
                                .unwrap_or("")
                                .to_string(),
                            name: origin_halt
                                .get("name")
                                .and_then(|n| n.as_str())
                                .unwrap_or("")
                                .to_string(),
                            latitude: None,
                            longitude: None,
                        };
                        let dest_station = Station {
                            id: dest_halt
                                .get("id")
                                .and_then(|id| id.as_str())
                                .unwrap_or("")
                                .to_string(),
                            name: dest_halt
                                .get("name")
                                .and_then(|n| n.as_str())
                                .unwrap_or("")
                                .to_string(),
                            latitude: None,
                            longitude: None,
                        };

                        let (scheduled_departure, realtime_departure) =
                            times_of(origin_halt, "abfahrt");
                        let (scheduled_arrival, realtime_arrival) =
                            times_of(dest_halt, "ankunft");
                        // Real-time wins for the primary field, because that is
                        // the time you have to be on the platform for.
                        let departure_time = realtime_departure
                            .clone()
                            .or_else(|| scheduled_departure.clone())
                            .unwrap_or_default();
                        let arrival_time = realtime_arrival
                            .clone()
                            .or_else(|| scheduled_arrival.clone())
                            .unwrap_or_default();
                        let cancelled = is_cancelled(section)
                            || is_cancelled(origin_halt)
                            || is_cancelled(dest_halt);

                        // Live halts carry the plain EVA number in `extId` and a
                        // composite lid in `id`; older fixtures only the latter.
                        // station-time rejects anything that is not a plain
                        // 7-digit id, so trying both is safe.
                        let ext_id_of = |halt: &Value| -> String {
                            halt.get("extId")
                                .or_else(|| halt.get("id"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string()
                        };
                        let departure_utc =
                            station_time::rfc3339_utc(&departure_time, &ext_id_of(origin_halt));
                        let arrival_utc =
                            station_time::rfc3339_utc(&arrival_time, &ext_id_of(dest_halt));

                        legs.push(Leg {
                            origin: origin_station,
                            destination: dest_station,
                            departure_time,
                            arrival_time,
                            departure_utc,
                            arrival_utc,
                            train_name: name,
                            train_number: number,
                            train_category: category,
                            platform: section
                                .get("gleis")
                                .and_then(|g| g.as_str())
                                .map(|s| s.to_string()),
                            is_regional,
                            scheduled_departure,
                            realtime_departure,
                            scheduled_arrival,
                            realtime_arrival,
                            cancelled,
                        });
                    }
                }
            }
        }

        if !legs.is_empty() {
            let first_leg = &legs[0];
            let last_leg = legs.last().unwrap();
            let price = v
                .get("angebotsPreis")
                .and_then(|p| p.get("betrag"))
                .and_then(|b| b.as_f64());
            let duration_seconds = v
                .get("verbindungsDauerInSeconds")
                .and_then(|d| d.as_u64())
                .unwrap_or(0);
            let total_duration_minutes = (duration_seconds / 60) as u32;

            journeys.push(Journey {
                // The real bahn.de response field is "tripId", not "id" --
                // found via live verification while wiring this adapter into
                // scouting (capabilities/postgres/README.md, Phase 2): every journey in a
                // real response was silently getting id="" and collapsing
                // into one upserted row downstream. The fixture this parser
                // was tested against used "id" too (same wrong assumption in
                // both places), so `cargo test` stayed green the whole time
                // -- a live call was the only thing that caught it.
                id: v
                    .get("tripId")
                    .and_then(|id| id.as_str())
                    .unwrap_or("")
                    .to_string(),
                start_station: first_leg.origin.clone(),
                end_station: last_leg.destination.clone(),
                total_duration_minutes,
                legs,
                total_price: price,
                delay_risk_score: None,
            });
        }
    }

    journeys
}

/// Pure JSON -> `Station` parser for the `/reiseloesung/orte` suggest
/// endpoint, extracted for the same fixture-testability reason.
pub fn parse_suggest_response(list: &[Value]) -> Vec<Station> {
    let mut stations = Vec::new();
    for item in list {
        if let (Some(ext_id), Some(name)) = (
            item.get("extId").and_then(|id| id.as_str()),
            item.get("name").and_then(|n| n.as_str()),
        ) {
            stations.push(Station {
                id: ext_id.to_string(),
                name: name.to_string(),
                latitude: item.get("lat").and_then(|l| l.as_f64()),
                longitude: item.get("lon").and_then(|l| l.as_f64()),
            });
        }
    }
    stations
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cheapest_split_prefers_two_hop_over_direct_when_cheaper() {
        let mut prices = HashMap::new();
        prices.insert((0, 2), 25.0); // direct
        prices.insert((0, 1), 10.0);
        prices.insert((1, 2), 10.0); // split total 20.0, cheaper than direct

        let (total, path) = cheapest_split(3, &prices).unwrap();
        assert!((total - 20.0).abs() < 1e-9);
        assert_eq!(path, vec![(0, 1), (1, 2)]);
    }

    #[test]
    fn cheapest_split_falls_back_to_direct_when_split_is_pricier() {
        let mut prices = HashMap::new();
        prices.insert((0, 2), 15.0);
        prices.insert((0, 1), 10.0);
        prices.insert((1, 2), 10.0); // split total 20.0, direct is cheaper

        let (total, path) = cheapest_split(3, &prices).unwrap();
        assert!((total - 15.0).abs() < 1e-9);
        assert_eq!(path, vec![(0, 2)]);
    }

    #[test]
    fn cheapest_split_returns_none_when_unreachable() {
        let mut prices = HashMap::new();
        prices.insert((0, 1), 10.0);
        // no price at all reaches stop 2 -- (1,2) and (0,2) both missing.
        let result = cheapest_split(3, &prices);
        assert!(result.is_none());
    }

    /// A delayed and partly cancelled connection, in the shape bahn.de serves.
    ///
    /// Every key here was captured from a real bahn.de response on 2026-08-11,
    /// not guessed. That mattered: the first version of this used `istzeit`,
    /// which the endpoint does not serve, so the field would have been
    /// structurally null forever while the test passed. The live capture showed
    /// `echtzeit`, on a train running 35 minutes late.
    #[test]
    fn a_delay_survives_the_parse_instead_of_being_folded_away() {
        let body = serde_json::json!({
            "verbindungen": [{
                "verbindungsAbschnitte": [{
                    "verkehrsmittel": {
                        "typ": "ZUG", "name": "ICE 1513",
                        "nummer": "1513", "kurzText": "ICE"
                    },
                    "halte": [
                        {
                            "id": "8000044", "name": "Bonn Hbf",
                            "abfahrt": {
                                "sollzeit": "2026-08-25T10:07:00",
                                "echtzeit": "2026-08-25T10:26:00"
                            }
                        },
                        {
                            "id": "8000105", "name": "Frankfurt(Main)Hbf",
                            "ankunft": {
                                "sollzeit": "2026-08-25T11:49:00",
                                "echtzeit": "2026-08-25T12:04:00"
                            }
                        }
                    ]
                }]
            }]
        });

        let journeys = parse_journeys_from_response(&body);
        assert_eq!(journeys.len(), 1);
        let leg = &journeys[0].legs[0];

        // Both times survive, and they are different: nineteen minutes late.
        assert_eq!(
            leg.scheduled_departure.as_deref(),
            Some("2026-08-25T10:07:00")
        );
        assert_eq!(
            leg.realtime_departure.as_deref(),
            Some("2026-08-25T10:26:00")
        );
        assert_eq!(leg.scheduled_arrival.as_deref(), Some("2026-08-25T11:49:00"));
        assert_eq!(leg.realtime_arrival.as_deref(), Some("2026-08-25T12:04:00"));

        // The primary field carries the time you have to be there for.
        assert_eq!(leg.departure_time, "2026-08-25T10:26:00");
        assert_eq!(leg.arrival_time, "2026-08-25T12:04:00");
        assert!(!leg.cancelled);
    }

    /// The fare context reaches the wire in the exact shape bahn.de's own
    /// clients send (enum strings from db-vendo-client's loyalty-cards
    /// formatter), the no-options default stays byte-compatible with what
    /// always worked, and a card that does not exist fails loudly instead of
    /// pricing as no card.
    #[test]
    fn the_fare_context_reaches_the_payload_and_rejects_fantasy_cards() {
        let fare = FareOptions {
            bahncard: Some(25),
            first_class: false,
            deutschland_ticket: true,
        };
        let p = HafasClient::fahrplan_payload("8000207", "8000105", "2026-09-01T08:00:00", &fare)
            .unwrap();
        assert_eq!(p["klasse"], "KLASSE_2");
        assert_eq!(
            p["reisende"][0]["ermaessigungen"][0],
            serde_json::json!({"art": "BAHNCARD25", "klasse": "KLASSE_2"})
        );
        assert_eq!(p["deutschlandTicketVorhanden"], true);

        let default = HafasClient::fahrplan_payload(
            "8000207",
            "8000105",
            "2026-09-01T08:00:00",
            &FareOptions::default(),
        )
        .unwrap();
        assert_eq!(
            default["reisende"][0]["ermaessigungen"][0],
            serde_json::json!({"art": "KEINE_ERMAESSIGUNG", "klasse": "KLASSENLOS"})
        );
        assert_eq!(default["deutschlandTicketVorhanden"], false);

        let fantasy = FareOptions {
            bahncard: Some(17),
            ..Default::default()
        };
        assert!(
            HafasClient::fahrplan_payload("8000207", "8000105", "2026-09-01T08:00:00", &fantasy)
                .is_err()
        );
    }

    /// A zone-crossing leg gets unambiguous UTC instants next to its naive
    /// station-local strings. Shape captured from a real Köln->London response
    /// on 2026-08-12: live halts carry the plain EVA in `extId` and a composite
    /// lid in `id`, and London's naive arrival is BST, one hour behind CEST --
    /// the naive strings must survive byte-identical while the UTC pair carries
    /// the true elapsed time.
    #[test]
    fn a_zone_crossing_leg_carries_utc_instants_next_to_its_local_times() {
        let body = serde_json::json!({
            "verbindungen": [{
                "verbindungsAbschnitte": [{
                    "verkehrsmittel": { "typ": "ZUG", "name": "ICE 316", "nummer": "316" },
                    "halte": [
                        { "id": "A=1@O=Köln Hbf@X=6958730@Y=50943029@L=8000207@",
                          "extId": "8000207", "name": "Köln Hbf",
                          "abfahrt": { "sollzeit": "2026-08-13T09:43:00" } },
                        { "id": "A=1@O=London St. Pancras@X=-126361@Y=51531922@L=7004428@",
                          "extId": "7004428", "name": "London St. Pancras International",
                          "ankunft": { "sollzeit": "2026-08-13T13:57:00" } }
                    ]
                }]
            }]
        });
        let leg = &parse_journeys_from_response(&body)[0].legs[0];

        // The naive station-local strings are untouched -- the dashboard
        // renders them as-is.
        assert_eq!(leg.departure_time, "2026-08-13T09:43:00");
        assert_eq!(leg.arrival_time, "2026-08-13T13:57:00");
        // The UTC pair is what arithmetic uses: 07:43Z -> 12:57Z is 5h14m,
        // where naive subtraction would have said 4h14m.
        assert_eq!(leg.departure_utc.as_deref(), Some("2026-08-13T07:43:00Z"));
        assert_eq!(leg.arrival_utc.as_deref(), Some("2026-08-13T12:57:00Z"));
    }

    /// A station whose UIC prefix station-time does not know yields absent UTC
    /// fields, never a zone guess.
    #[test]
    fn an_unknown_station_prefix_leaves_the_utc_fields_absent() {
        let body = serde_json::json!({
            "verbindungen": [{
                "verbindungsAbschnitte": [{
                    "verkehrsmittel": { "typ": "ZUG", "name": "X 1", "nummer": "1" },
                    "halte": [
                        { "id": "2000001", "name": "Somewhere in a multi-zone country",
                          "abfahrt": { "sollzeit": "2026-08-13T09:43:00" } },
                        { "id": "8000207", "extId": "8000207", "name": "Köln Hbf",
                          "ankunft": { "sollzeit": "2026-08-13T13:57:00" } }
                    ]
                }]
            }]
        });
        let leg = &parse_journeys_from_response(&body)[0].legs[0];
        assert_eq!(leg.departure_utc, None);
        assert_eq!(leg.arrival_utc.as_deref(), Some("2026-08-13T11:57:00Z"));
    }

    /// No real-time value is not the same as no delay, and must not read as
    /// "on time".
    #[test]
    fn a_connection_with_no_realtime_data_says_so_rather_than_claiming_punctuality() {
        let body = serde_json::json!({
            "verbindungen": [{
                "verbindungsAbschnitte": [{
                    "verkehrsmittel": { "typ": "ZUG", "name": "RB 66", "nummer": "66" },
                    "halte": [
                        { "id": "8000044", "name": "Bonn Hbf",
                          "abfahrt": { "sollzeit": "2026-08-25T10:07:00" } },
                        { "id": "8000262", "name": "Siegburg/Bonn",
                          "ankunft": { "sollzeit": "2026-08-25T10:31:00" } }
                    ]
                }]
            }]
        });
        let leg = &parse_journeys_from_response(&body)[0].legs[0];
        assert!(leg.realtime_departure.is_none());
        assert!(leg.realtime_arrival.is_none());
        // The planning field still falls back to the schedule.
        assert_eq!(leg.departure_time, "2026-08-25T10:07:00");
    }

    /// A cancelled train used to come back as an ordinary leg with times on it.
    #[test]
    fn a_cancelled_leg_is_marked_cancelled() {
        let body = serde_json::json!({
            "verbindungen": [{
                "verbindungsAbschnitte": [{
                    "verkehrsmittel": { "typ": "ZUG", "name": "ICE 1513", "nummer": "1513" },
                    "originCancelled": true,
                    "halte": [
                        { "id": "8000044", "name": "Bonn Hbf",
                          "abfahrt": { "sollzeit": "2026-08-25T10:07:00" } },
                        { "id": "8000105", "name": "Frankfurt(Main)Hbf",
                          "ankunft": { "sollzeit": "2026-08-25T11:49:00" } }
                    ]
                }]
            }]
        });
        assert!(parse_journeys_from_response(&body)[0].legs[0].cancelled);
    }

    fn journey_on(trains: &[&str]) -> Journey {
        let station = |name: &str| Station {
            id: "8000000".into(),
            name: name.into(),
            latitude: None,
            longitude: None,
        };
        Journey {
            id: "j".into(),
            start_station: station("A"),
            end_station: station("B"),
            legs: trains
                .iter()
                .map(|n| Leg {
                    origin: station("A"),
                    destination: station("B"),
                    departure_time: "2026-09-01T08:00:00".into(),
                    arrival_time: "2026-09-01T10:00:00".into(),
                    departure_utc: None,
                    arrival_utc: None,
                    train_name: format!("ICE {n}"),
                    train_number: (*n).into(),
                    train_category: "ICE".into(),
                    platform: None,
                    is_regional: false,
                    scheduled_departure: None,
                    realtime_departure: None,
                    scheduled_arrival: None,
                    realtime_arrival: None,
                    cancelled: false,
                })
                .collect(),
            total_duration_minutes: 120,
            total_price: Some(30.0),
            delay_risk_score: None,
        }
    }

    fn owned(trains: &[&str]) -> Vec<String> {
        trains.iter().map(|t| (*t).to_string()).collect()
    }

    /// The defect this classification exists for: each segment is priced by a
    /// fresh search that takes `journeys.first()`, and nothing made that journey
    /// the train the traveller is on.
    #[test]
    fn train_match_separates_the_same_ride_from_a_different_one() {
        assert_eq!(
            classify_train_match(&owned(&["691"]), &journey_on(&["691"])),
            TrainMatch::Exact
        );
        // Priced for a service the traveller will not be on. This is the case
        // that costs money, so it must not be reported as merely unknown.
        assert_eq!(
            classify_train_match(&owned(&["691"]), &journey_on(&["512"])),
            TrainMatch::Different
        );
        // Covers one of the two planned trains.
        assert_eq!(
            classify_train_match(&owned(&["691", "512"]), &journey_on(&["691"])),
            TrainMatch::Partial
        );
        // Same trains, wrong order is not the same ride.
        assert_eq!(
            classify_train_match(&owned(&["691", "512"]), &journey_on(&["512", "691"])),
            TrainMatch::Partial
        );
        // No train number on either side is not a verdict.
        assert_eq!(
            classify_train_match(&[], &journey_on(&["691"])),
            TrainMatch::Unknown
        );
        assert_eq!(
            classify_train_match(&owned(&["691"]), &journey_on(&[])),
            TrainMatch::Unknown
        );
    }

    fn segment(train_match: TrainMatch) -> SplitSegment {
        SplitSegment {
            journey: journey_on(&["691"]),
            train_match,
            expected_trains: owned(&["691"]),
        }
    }

    /// A chain is only as buyable as its least trustworthy ticket, and a hole in
    /// the price table means the search never saw every candidate split.
    #[test]
    fn chain_confidence_takes_the_worst_case() {
        assert_eq!(
            split_confidence(&[segment(TrainMatch::Exact), segment(TrainMatch::Exact)], 0),
            SplitConfidence::Exact
        );
        assert_eq!(
            split_confidence(&[segment(TrainMatch::Exact), segment(TrainMatch::Partial)], 0),
            SplitConfidence::Partial
        );
        assert_eq!(
            split_confidence(&[segment(TrainMatch::Exact), segment(TrainMatch::Unknown)], 0),
            SplitConfidence::Partial
        );
        // All exact, but a failed pairwise query means a cheaper split may exist
        // that was never priced.
        assert_eq!(
            split_confidence(&[segment(TrainMatch::Exact)], 1),
            SplitConfidence::Partial
        );
        // One wrong-train segment outranks every other signal.
        assert_eq!(
            split_confidence(
                &[segment(TrainMatch::Exact), segment(TrainMatch::Different)],
                0
            ),
            SplitConfidence::Low
        );
    }

    /// `savings: 0.0` could not be told apart from "the split saves nothing",
    /// and the dashboard rendered that as "Direct is cheapest".
    #[test]
    fn unknown_direct_fare_yields_no_savings_figure() {
        let direct: Option<f64> = None;
        assert_eq!(direct.map(|p: f64| p - 20.0), None);
        assert_eq!(Some(35.0f64).map(|p| p - 20.0), Some(15.0));
    }

    #[test]
    fn cheapest_split_chains_across_four_stops() {
        let mut prices = HashMap::new();
        prices.insert((0, 3), 40.0); // direct
        prices.insert((0, 1), 8.0);
        prices.insert((1, 2), 8.0);
        prices.insert((2, 3), 8.0); // 3-hop total 24.0, cheapest
        prices.insert((0, 2), 20.0);
        prices.insert((1, 3), 20.0);

        let (total, path) = cheapest_split(4, &prices).unwrap();
        assert!((total - 24.0).abs() < 1e-9);
        assert_eq!(path, vec![(0, 1), (1, 2), (2, 3)]);
    }

    fn fixture_journey(is_regional_attr: bool) -> Value {
        json!({
            "verbindungen": [{
                "tripId": "journey-1",
                "angebotsPreis": {"betrag": 39.90},
                "verbindungsDauerInSeconds": 5400,
                "verbindungsAbschnitte": [{
                    "verkehrsmittel": {
                        "name": "ICE 691",
                        "nummer": "691",
                        "kategorie": "ICE",
                        "zugattribute": if is_regional_attr {
                            json!([{"key": "9G"}])
                        } else {
                            json!([])
                        }
                    },
                    "gleis": "7",
                    "halte": [
                        {
                            "id": "8000105",
                            "name": "Frankfurt(Main)Hbf",
                            "abfahrt": {"sollzeit": "2026-07-15T08:30:00"}
                        },
                        {
                            "id": "8000261",
                            "name": "Mannheim Hbf",
                            "ankunft": {"sollzeit": "2026-07-15T09:15:00"}
                        }
                    ]
                }]
            }]
        })
    }

    #[test]
    fn parses_single_leg_journey() {
        let body = fixture_journey(false);
        let journeys = parse_journeys_from_response(&body);
        assert_eq!(journeys.len(), 1);
        let j = &journeys[0];
        assert_eq!(j.id, "journey-1");
        assert_eq!(j.start_station.name, "Frankfurt(Main)Hbf");
        assert_eq!(j.end_station.name, "Mannheim Hbf");
        assert_eq!(j.total_duration_minutes, 90);
        assert!((j.total_price.unwrap() - 39.90).abs() < 1e-9);
        assert_eq!(
            j.delay_risk_score, None,
            "ONNX prediction not ported -- always None"
        );
        assert_eq!(j.legs.len(), 1);
        assert_eq!(j.legs[0].train_name, "ICE 691");
        assert_eq!(j.legs[0].platform.as_deref(), Some("7"));
        assert!(!j.legs[0].is_regional);
    }

    /// Regression test for the "id" vs "tripId" field-name bug (found via
    /// live verification, see the doc comment on `id:` in
    /// `parse_journeys_from_response`) -- a response using the WRONG key
    /// ("id" instead of "tripId") must not silently look like a valid,
    /// present id.
    #[test]
    fn missing_trip_id_field_yields_empty_id_not_a_wrong_value() {
        let mut body = fixture_journey(false);
        // Simulate the bug this test guards against: a response shaped with
        // "id" instead of "tripId" (what the fixture -- and the parser --
        // both wrongly assumed before the live-verification fix).
        let conn = &mut body["verbindungen"][0];
        let obj = conn.as_object_mut().unwrap();
        let stray = obj.remove("tripId").unwrap();
        obj.insert("id".into(), stray);

        let journeys = parse_journeys_from_response(&body);
        assert_eq!(journeys.len(), 1);
        assert_eq!(
            journeys[0].id, "",
            "\"id\" is not a real field on this endpoint -- must not be read as one"
        );
    }

    #[test]
    fn detects_regional_attribute_9g() {
        let body = fixture_journey(true);
        let journeys = parse_journeys_from_response(&body);
        assert!(
            journeys[0].legs[0].is_regional,
            "zugattribute key '9G' should set is_regional"
        );
    }

    #[test]
    fn empty_verbindungen_yields_empty_journeys() {
        let body = json!({"verbindungen": []});
        assert!(parse_journeys_from_response(&body).is_empty());
    }

    #[test]
    fn missing_verbindungen_key_yields_empty_journeys() {
        let body = json!({});
        assert!(parse_journeys_from_response(&body).is_empty());
    }

    #[test]
    fn parses_suggest_stations() {
        let list = vec![
            json!({"extId": "8000105", "name": "Frankfurt(Main)Hbf", "lat": 50.1072, "lon": 8.6633}),
            json!({"extId": "8000261", "name": "Mannheim Hbf"}),
            json!({"name": "missing ext id, should be skipped"}),
        ];
        let stations = parse_suggest_response(&list);
        assert_eq!(stations.len(), 2);
        assert_eq!(stations[0].id, "8000105");
        assert!((stations[0].latitude.unwrap() - 50.1072).abs() < 1e-6);
        assert_eq!(stations[1].id, "8000261");
        assert_eq!(stations[1].latitude, None);
    }
}
