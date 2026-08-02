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

use crate::travel::{Journey, Leg, SplitResult, Station};
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

    fn fahrplan_payload(from_eva: &str, to_eva: &str, datetime: &str) -> Value {
        json!({
            "abfahrtsHalt": from_eva,
            "anfrageZeitpunkt": datetime,
            "ankunftsHalt": to_eva,
            "ankunftSuche": "ABFAHRT",
            "klasse": "KLASSE_2",
            "produktgattungen": ["ICE", "EC_IC", "IR", "REGIONAL", "SBAHN", "BUS", "SCHIFF", "UBAHN", "TRAM", "ANRUFPFLICHTIG"],
            "reisende": [{
                "typ": "ERWACHSENER",
                "ermaessigungen": [{"art": "KEINE_ERMAESSIGUNG", "klasse": "KLASSENLOS"}],
                "anzahl": 1,
                "alter": []
            }],
            "schnelleVerbindungen": true,
            "deutschlandTicketVorhanden": false
        })
    }

    /// Direct journey search between two EVA station codes.
    pub fn search_connections(&self, from_eva: &str, to_eva: &str, datetime: &str) -> Result<Vec<Journey>, HafasError> {
        let payload = Self::fahrplan_payload(from_eva, to_eva, datetime);

        let response = self.client.post(FAHRPLAN_URL)
            .header("User-Agent", BROWSER_UA)
            .header("Accept", "application/json")
            .header("Content-Type", "application/json; charset=UTF-8")
            .json(&payload)
            .send()
            .map_err(|e| HafasError::Request(e.to_string()))?;

        let status = response.status();
        let text = response.text().map_err(|e| HafasError::Request(e.to_string()))?;
        if !status.is_success() {
            return Err(HafasError::BadStatus { status: status.as_u16(), body: text });
        }

        let body: Value = serde_json::from_str(&text).map_err(|e| HafasError::Parse(e.to_string(), text))?;
        Ok(parse_journeys_from_response(&body))
    }

    /// Station name -> EVA id search (autocomplete-style).
    pub fn suggest_stations(&self, query: &str) -> Result<Vec<Station>, HafasError> {
        let response = self.client.get(ORTE_URL)
            .query(&[("suchbegriff", query), ("typ", "ALL"), ("limit", "10")])
            .header("User-Agent", BROWSER_UA)
            .header("Accept", "application/json")
            .send()
            .map_err(|e| HafasError::Request(e.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            return Err(HafasError::BadStatus { status: status.as_u16(), body: String::new() });
        }

        let list: Vec<Value> = response.json().map_err(|e| HafasError::Request(e.to_string()))?;
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
    pub fn search_split_tickets(&self, from_eva: &str, to_eva: &str, datetime: &str) -> Result<SplitResult, HafasError> {
        let payload = Self::fahrplan_payload(from_eva, to_eva, datetime);
        let response = self.client.post(FAHRPLAN_URL)
            .header("User-Agent", BROWSER_UA)
            .header("Accept", "application/json")
            .header("Content-Type", "application/json; charset=UTF-8")
            .json(&payload)
            .send()
            .map_err(|e| HafasError::Request(e.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            return Err(HafasError::BadStatus { status: status.as_u16(), body: String::new() });
        }

        let body: Value = response.json().map_err(|e| HafasError::Request(e.to_string()))?;
        let verbindungen = body.get("verbindungen").and_then(|v| v.as_array())
            .ok_or_else(|| HafasError::Other("no connections found in Vendo response".into()))?;
        if verbindungen.is_empty() {
            return Err(HafasError::Other("no connection found".into()));
        }
        let v = &verbindungen[0];
        let direct_price = v.get("angebotsPreis").and_then(|p| p.get("betrag")).and_then(|b| b.as_f64());

        let stops = extract_stops(v);
        let n = stops.len();
        if n < 2 {
            return Err(HafasError::Other("not enough stops to perform split-ticketing".into()));
        }

        let mut prices: HashMap<(usize, usize), f64> = HashMap::new();
        let mut segments_data: HashMap<(usize, usize), Journey> = HashMap::new();

        for i in 0..n {
            for j in (i + 1)..n {
                std::thread::sleep(std::time::Duration::from_millis(250));
                if let Ok(journeys) = self.search_connections(&stops[i].ext_id, &stops[j].ext_id, &stops[i].departure_iso) {
                    if let Some(first) = journeys.first() {
                        if let Some(price) = first.total_price {
                            prices.insert((i, j), price);
                            segments_data.insert((i, j), first.clone());
                        }
                    }
                }
            }
        }

        let (split_price, path) = cheapest_split(n, &prices).ok_or(HafasError::NoSplitFound)?;

        let segments: Vec<Journey> = path.into_iter()
            .filter_map(|pair| segments_data.get(&pair).cloned())
            .collect();

        let savings = direct_price.map(|p| p - split_price).unwrap_or(0.0);

        Ok(SplitResult { original_price: direct_price, split_price, savings, segments })
    }
}

struct Stop {
    ext_id: String,
    departure_iso: String,
}

fn extract_stops(v: &Value) -> Vec<Stop> {
    let mut stops = Vec::new();
    let Some(sections) = v.get("verbindungsAbschnitte").and_then(|s| s.as_array()) else {
        return stops;
    };
    for section in sections {
        let verkehrsmittel = section.get("verkehrsmittel").cloned().unwrap_or(Value::Null);
        let typ = verkehrsmittel.get("typ").and_then(|t| t.as_str()).unwrap_or("");
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
            let Some(ext_id) = halt.get("id").or_else(|| halt.get("extId")).and_then(|id| id.as_str()) else {
                continue;
            };
            if stops.iter().any(|s: &Stop| s.ext_id == ext_id) {
                continue;
            }
            let departure_iso = halt.get("abfahrt")
                .and_then(|a| a.get("sollzeit").or_else(|| a.get("istzeit")))
                .and_then(|t| t.as_str())
                .or_else(|| {
                    halt.get("ankunft")
                        .and_then(|a| a.get("sollzeit").or_else(|| a.get("istzeit")))
                        .and_then(|t| t.as_str())
                })
                .unwrap_or("")
                .to_string();
            stops.push(Stop { ext_id: ext_id.to_string(), departure_iso });
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
pub fn cheapest_split(n: usize, segment_prices: &HashMap<(usize, usize), f64>) -> Option<(f64, Vec<(usize, usize)>)> {
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
                let verkehrsmittel = section.get("verkehrsmittel").cloned().unwrap_or(Value::Null);
                let name = verkehrsmittel.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
                let number = verkehrsmittel.get("nummer").and_then(|n| n.as_str()).unwrap_or("").to_string();
                let category = verkehrsmittel.get("kategorie").and_then(|c| c.as_str()).unwrap_or("").to_string();

                let attributes = verkehrsmittel.get("zugattribute").and_then(|a| a.as_array());
                let is_regional = attributes
                    .map(|attrs| attrs.iter().any(|attr| attr.get("key").and_then(|k| k.as_str()) == Some("9G")))
                    .unwrap_or(false);

                if let Some(halts) = section.get("halte").and_then(|h| h.as_array()) {
                    if halts.len() >= 2 {
                        let origin_halt = &halts[0];
                        let dest_halt = halts.last().unwrap();

                        let origin_station = Station {
                            id: origin_halt.get("id").and_then(|id| id.as_str()).unwrap_or("").to_string(),
                            name: origin_halt.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string(),
                            latitude: None,
                            longitude: None,
                        };
                        let dest_station = Station {
                            id: dest_halt.get("id").and_then(|id| id.as_str()).unwrap_or("").to_string(),
                            name: dest_halt.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string(),
                            latitude: None,
                            longitude: None,
                        };

                        let departure_time = origin_halt.get("abfahrt")
                            .and_then(|a| a.get("sollzeit").or_else(|| a.get("istzeit")))
                            .and_then(|t| t.as_str())
                            .unwrap_or("")
                            .to_string();
                        let arrival_time = dest_halt.get("ankunft")
                            .and_then(|a| a.get("sollzeit").or_else(|| a.get("istzeit")))
                            .and_then(|t| t.as_str())
                            .unwrap_or("")
                            .to_string();

                        legs.push(Leg {
                            origin: origin_station,
                            destination: dest_station,
                            departure_time,
                            arrival_time,
                            train_name: name,
                            train_number: number,
                            train_category: category,
                            platform: section.get("gleis").and_then(|g| g.as_str()).map(|s| s.to_string()),
                            is_regional,
                        });
                    }
                }
            }
        }

        if !legs.is_empty() {
            let first_leg = &legs[0];
            let last_leg = legs.last().unwrap();
            let price = v.get("angebotsPreis").and_then(|p| p.get("betrag")).and_then(|b| b.as_f64());
            let duration_seconds = v.get("verbindungsDauerInSeconds").and_then(|d| d.as_u64()).unwrap_or(0);
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
                id: v.get("tripId").and_then(|id| id.as_str()).unwrap_or("").to_string(),
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
        assert_eq!(j.delay_risk_score, None, "ONNX prediction not ported -- always None");
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
        assert_eq!(journeys[0].id, "", "\"id\" is not a real field on this endpoint -- must not be read as one");
    }

    #[test]
    fn detects_regional_attribute_9g() {
        let body = fixture_journey(true);
        let journeys = parse_journeys_from_response(&body);
        assert!(journeys[0].legs[0].is_regional, "zugattribute key '9G' should set is_regional");
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
