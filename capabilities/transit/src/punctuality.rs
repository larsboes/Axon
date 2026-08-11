//! Filling `Journey.delay_risk_score` from `capabilities/punctuality`.
//!
//! Over HTTP, never by linking that crate: a capability depends on another's contract,
//! not its code (README.md#schemas-and-dependency-direction). This module is the whole dependency, and it is a
//! one-way one — punctuality knows nothing about transit.
//!
//! Absence degrades, it never fails. If punctuality is not running, or has never
//! ingested, or has no cell for this train at this hour, the score stays `None` and the
//! search returns exactly what it returned before this module existed. A journey search
//! that breaks because a statistics service is down would be a worse product than one
//! without statistics.

use crate::travel::{Journey, SplitResult};
use serde::{Deserialize, Serialize};

/// Where punctuality-server listens.
///
/// The literal mirrors `capabilities/punctuality/service.toml`'s `port`, which is one
/// duplication more than README.md#dynamic-paths-and-current-facts likes. It is here rather than hidden because the honest
/// fix is a spine mechanism — service-runner.sh exporting a declared `requires =`
/// sibling's port the way it already exports `AXON_PORT` for the capability itself — and
/// building that for a single consumer would be inventing a convention from one example.
/// The second capability that needs a sibling's URL is when that gets built.
const DEFAULT_BASE_URL: &str = "http://127.0.0.1:8085";

pub fn base_url() -> String {
    std::env::var("AXON_PUNCTUALITY_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string())
}

#[derive(Debug, Serialize)]
struct StopQuery {
    eva: String,
    train_type: String,
    hour: u8,
    weekend: bool,
}

#[derive(Debug, Serialize)]
struct LookupBody {
    stops: Vec<StopQuery>,
}

#[derive(Debug, Deserialize)]
struct StopStats {
    /// Share of this train type's stops at this station in this hour that were at least
    /// six minutes off schedule. Six is DB's own punctuality threshold.
    share_late_6: f32,
}

#[derive(Debug, Deserialize)]
struct LookupResponse {
    stats: Vec<Option<StopStats>>,
}

/// The EVA number inside a station id.
///
/// `travel::Station::id` carries two different formats depending on which HAFAS endpoint
/// produced it, despite the type's own comment claiming it is always an EVA code:
/// `/reiseloesung/orte` (suggest) answers a bare `"8000044"`, while a journey search
/// answers the full location string
/// `A=1@O=Bonn Hbf@X=7097136@Y=50732008@U=80@L=8000044@i=U×008015485@`, where the EVA is
/// the `L=` field. Reading `id` as an EVA works for one of those and silently finds
/// nothing for the other.
pub fn eva_of(station_id: &str) -> Option<String> {
    if !station_id.is_empty() && station_id.chars().all(|c| c.is_ascii_digit()) {
        return Some(station_id.to_string());
    }
    station_id
        .split('@')
        .find_map(|part| part.strip_prefix("L="))
        .filter(|eva| !eva.is_empty() && eva.chars().all(|c| c.is_ascii_digit()))
        .map(str::to_string)
}

/// Local hour and weekend flag from a HAFAS `YYYY-MM-DDTHH:MM:SS` timestamp.
///
/// Parsed by position rather than with a date library: HAFAS emits local time in a fixed
/// layout, and punctuality's own timestamps are local wall-clock too, so no conversion
/// is wanted on either side — only the same clock read the same way.
pub fn hour_and_weekend(iso: &str) -> Option<(u8, bool)> {
    let (date, time) = iso.split_once('T')?;
    let hour: u8 = time.get(0..2)?.parse().ok()?;
    let mut parts = date.split('-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: i64 = parts.next()?.parse().ok()?;
    let d: i64 = parts.next()?.parse().ok()?;
    if hour > 23 {
        return None;
    }
    // Sakamoto's algorithm: day of week without a calendar dependency, 0 = Sunday.
    const T: [i64; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if m < 3 { y - 1 } else { y };
    let idx = usize::try_from(m - 1).ok()?;
    let dow = (y + y / 4 - y / 100 + y / 400 + T.get(idx)? + d) % 7;
    Some((hour, dow == 0 || dow == 6))
}

/// Fills `delay_risk_score` on every journey, in one request.
///
/// The score is the share of stops of the arriving train's type, at the journey's
/// destination, in the arrival hour, that ran at least six minutes off schedule. It is
/// emphatically NOT the probability that the trip works out: it says nothing about
/// catching a transfer, and a journey whose first leg is late enough to miss a
/// connection arrives on a different train than the one this describes. Transfer risk is
/// a different quantity and this data cannot produce it.
pub fn enrich(journeys: &mut [Journey]) {
    if journeys.is_empty() {
        return;
    }
    // Position i in the request maps to position i in the response, so the queries and
    // the journeys they belong to are zipped rather than matched by content.
    let mut targets: Vec<usize> = Vec::new();
    let mut stops: Vec<StopQuery> = Vec::new();
    for (i, j) in journeys.iter().enumerate() {
        let Some(last) = j.legs.last() else { continue };
        let Some(eva) = eva_of(&j.end_station.id).or_else(|| eva_of(&last.destination.id)) else {
            continue;
        };
        let Some((hour, weekend)) = hour_and_weekend(&last.arrival_time) else {
            continue;
        };
        targets.push(i);
        stops.push(StopQuery {
            eva,
            train_type: last.train_category.clone(),
            hour,
            weekend,
        });
    }
    if stops.is_empty() {
        return;
    }

    let client = match reqwest::blocking::Client::builder()
        // Short on purpose: this is an enhancement on a localhost service. Waiting on it
        // would make a journey search slower than not having the number at all.
        .timeout(std::time::Duration::from_secs(3))
        .build()
    {
        Ok(c) => c,
        Err(_) => return,
    };
    let response = client
        .post(format!("{}/lookup", base_url()))
        .json(&LookupBody { stops })
        .send()
        .and_then(|r| r.error_for_status())
        .and_then(|r| r.json::<LookupResponse>());

    let Ok(response) = response else { return };
    for (idx, stat) in targets.into_iter().zip(response.stats) {
        if let Some(stat) = stat {
            journeys[idx].delay_risk_score = Some(stat.share_late_6 as f64);
        }
    }
}

/// Same enrichment for a split-ticket result's segments.
///
/// A segment now carries its journey beside the train-match verdict, so the
/// journeys are lifted out, enriched as a slice, and put back rather than
/// enriching in place: `enrich` takes `&mut [Journey]` and keeping it that way
/// leaves it usable by every caller that has no split-ticket in hand.
pub fn enrich_split(result: &mut SplitResult) {
    let mut journeys: Vec<_> = result.segments.iter().map(|s| s.journey.clone()).collect();
    enrich(&mut journeys);
    for (segment, journey) in result.segments.iter_mut().zip(journeys) {
        segment.journey = journey;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eva_comes_out_of_both_station_id_formats() {
        // The suggest endpoint's bare form.
        assert_eq!(eva_of("8000044").as_deref(), Some("8000044"));
        // The journey-search location string, verified against a live response.
        assert_eq!(
            eva_of("A=1@O=Bonn Hbf@X=7097136@Y=50732008@U=80@L=8000044@i=U×008015485@").as_deref(),
            Some("8000044")
        );
        assert_eq!(
            eva_of("A=1@O=Köln Hbf@X=6958730@Y=50943029@U=80@L=8000207@i=U×008015458@").as_deref(),
            Some("8000207")
        );
    }

    #[test]
    fn a_station_id_without_an_eva_yields_nothing() {
        // Better no lookup than a lookup on a wrong key: a wrong key returns a real
        // statistic for a different station, which is worse than no statistic.
        assert_eq!(eva_of(""), None);
        assert_eq!(eva_of("A=1@O=Somewhere@X=1@Y=2@"), None);
        assert_eq!(eva_of("A=1@O=X@L=@i=1@"), None);
        assert_eq!(eva_of("A=1@O=X@L=abc@i=1@"), None);
    }

    #[test]
    fn weekday_matches_the_calendar() {
        // 2026-07-28 is a Tuesday, 2026-08-01 a Saturday, 2026-08-02 a Sunday.
        assert_eq!(hour_and_weekend("2026-07-28T14:32:00"), Some((14, false)));
        assert_eq!(hour_and_weekend("2026-08-01T09:00:00"), Some((9, true)));
        assert_eq!(hour_and_weekend("2026-08-02T23:59:00"), Some((23, true)));
        // January and February take Sakamoto's previous-year branch.
        assert_eq!(hour_and_weekend("2026-01-01T00:00:00"), Some((0, false))); // Thursday
        assert_eq!(hour_and_weekend("2026-02-28T12:00:00"), Some((12, true))); // Saturday
                                                                               // A leap year's 29 February -- 2024-02-29 was a Thursday.
        assert_eq!(hour_and_weekend("2024-02-29T08:00:00"), Some((8, false)));
    }

    #[test]
    fn a_malformed_timestamp_yields_nothing() {
        assert_eq!(hour_and_weekend("2026-07-28"), None);
        assert_eq!(hour_and_weekend(""), None);
        assert_eq!(hour_and_weekend("2026-07-28T99:00:00"), None);
        assert_eq!(hour_and_weekend("not-a-date T12:00:00"), None);
    }

    #[test]
    fn enrichment_of_nothing_does_nothing() {
        // Guards the early return: an empty search must not open a connection.
        let mut none: Vec<Journey> = Vec::new();
        enrich(&mut none);
        assert!(none.is_empty());
    }
}
