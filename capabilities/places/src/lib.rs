//! places: canonical place registry, cached geocoder, and the map layers.
//!
//! Contract: `README.md` here (decisions D1-D4), `ISA.md` here (PLC-1..8).
//! Writes stay inside the `places` schema; reads of `finance.*`, `trips.*`,
//! `transit.*` and `punctuality.*` are read-only SELECTs, the correlation-join
//! usage `capabilities/store/README.md` chose one database for.

pub mod backfill;
pub mod config;
pub mod geocode;
pub mod layers;
pub mod store;

/// Today as an ISO date from the wall clock, UTC. Same algorithm and the same
/// no-date-dependency reasoning as `capabilities/finance/src/server.rs`
/// (Howard Hinnant's civil-from-days, public domain).
pub fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    civil_from_days(secs / 86_400)
}

fn civil_from_days(days: i64) -> String {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    format!("{year:04}-{m:02}-{d:02}")
}

/// The inverse of [`civil_from_days`], for counting days between two ISO dates.
///
/// Same algorithm, same source, same no-date-dependency reasoning; it lives
/// here rather than in `store.rs` so the pair stays in one place. `None` for
/// anything that is not `YYYY-MM-DD`.
pub fn days_from_civil(iso: &str) -> Option<i64> {
    let mut parts = iso.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: i64 = parts.next()?.parse().ok()?;
    let day: i64 = parts.next()?.get(..2)?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_date_conversion_matches_known_days() {
        assert_eq!(civil_from_days(0), "1970-01-01");
        assert_eq!(civil_from_days(19_782), "2024-02-29");
        assert_eq!(civil_from_days(20_673), "2026-08-08");
    }

    #[test]
    fn the_two_directions_are_inverses() {
        for day in [0i64, 19_782, 20_673, -1, 25_000] {
            assert_eq!(days_from_civil(&civil_from_days(day)), Some(day));
        }
        assert_eq!(days_from_civil("2026-13-01"), None);
        assert_eq!(days_from_civil("not a date"), None);
    }
}
