//! "When could I go": the fuzzy-timeframe answer (travel PRD X4).
//!
//! Inverts the calendar loop. Booked stages already sync INTO the calendar;
//! this reads committed time back OUT, prices every day of a span with the
//! flexible-date grid, and answers with days ranked cheapest-first among the
//! days that are actually free -- instead of demanding exact dates before any
//! price exists. Date flexibility is the measured 40-54% axis; the calendar
//! is what makes the cheap day bookable rather than theoretical.
//!
//! Pure logic only: chunking a span into grid windows, expanding calendar
//! entries into busy days, merging and ranking. The HTTP glue lives in the
//! server, so every rule here is testable without a network.

use serde::{Deserialize, Serialize};

use crate::kiwi::GridDay;

/// One calendar entry, reduced to what ranking needs.
#[derive(Debug, Clone, Deserialize)]
pub struct CalendarSpan {
    pub starts_at: Option<String>,
    pub ends_at: Option<String>,
    pub title: Option<String>,
    pub commitment: Option<String>,
}

/// How occupied a day is, from the calendar's own commitment vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DayLoad {
    /// No calendar entry touches the day.
    Free,
    /// Only `planned` entries touch it -- soft, movable.
    Planned,
    /// A `committed` entry touches it.
    Committed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WhenDay {
    pub date: String,
    pub price: f64,
    pub price_formatted: String,
    pub booking_url: String,
    pub has_hidden_ground_transfer: bool,
    pub load: DayLoad,
    /// Titles of the calendar entries touching the day, so "committed" names
    /// what it would collide with.
    pub collisions: Vec<String>,
}

/// Grid-search centers covering `[from, to]` with the ±`flex` reach of one
/// flexible-date search each. Dates are day-resolution ISO strings; arithmetic
/// runs on day numbers to stay clear of any clock.
pub fn window_centers(from_day: i64, to_day: i64, flex: i64) -> Vec<i64> {
    if to_day < from_day || flex < 1 {
        return Vec::new();
    }
    let width = 2 * flex + 1;
    let mut centers = Vec::new();
    let mut start = from_day;
    while start <= to_day {
        centers.push((start + (start + width - 1).min(to_day)) / 2);
        start += width;
    }
    centers
}

/// The days of `[from, to]` each entry touches, with the strongest commitment
/// winning per day. Entries without a start are ignored; an entry without an
/// end occupies its start day.
pub fn day_loads(
    from: &str,
    to: &str,
    entries: &[CalendarSpan],
) -> Vec<(String, DayLoad, Vec<String>)> {
    let mut days: Vec<(String, DayLoad, Vec<String>)> = Vec::new();
    let mut day = from.to_string();
    while day.as_str() <= to {
        let mut load = DayLoad::Free;
        let mut collisions = Vec::new();
        for entry in entries {
            let Some(start) = entry.starts_at.as_deref().map(|s| &s[..10.min(s.len())]) else {
                continue;
            };
            let end = entry
                .ends_at
                .as_deref()
                .map(|s| &s[..10.min(s.len())])
                .unwrap_or(start);
            if day.as_str() < start || day.as_str() > end {
                continue;
            }
            let entry_load = match entry.commitment.as_deref() {
                Some("committed") => DayLoad::Committed,
                _ => DayLoad::Planned,
            };
            if let Some(title) = &entry.title {
                collisions.push(title.clone());
            }
            load = load.max(entry_load);
        }
        days.push((day.clone(), load, collisions));
        day = next_day(&day);
    }
    days
}

/// Grid days joined with day loads, ranked: free days cheapest-first, then
/// planned, committed last -- within each band by price. Days the grid did not
/// price are absent, which is itself the honest answer for them.
pub fn rank(grid: Vec<GridDay>, loads: &[(String, DayLoad, Vec<String>)]) -> Vec<WhenDay> {
    let mut days: Vec<WhenDay> = grid
        .into_iter()
        .map(|g| {
            let (load, collisions) = loads
                .iter()
                .find(|(date, _, _)| *date == g.date)
                .map(|(_, l, c)| (*l, c.clone()))
                .unwrap_or((DayLoad::Free, Vec::new()));
            WhenDay {
                date: g.date,
                price: g.price,
                price_formatted: g.price_formatted,
                booking_url: g.booking_url,
                has_hidden_ground_transfer: g.has_hidden_ground_transfer,
                load,
                collisions,
            }
        })
        .collect();
    days.sort_by(|a, b| {
        a.load
            .cmp(&b.load)
            .then(a.price.total_cmp(&b.price))
            .then(a.date.cmp(&b.date))
    });
    days
}

/// ISO date + 1 day, without a calendar dependency for the two month shapes
/// that matter here (Gregorian lengths incl. leap years).
fn next_day(iso: &str) -> String {
    let mut parts = iso.split('-');
    let (Some(y), Some(m), Some(d)) = (parts.next(), parts.next(), parts.next()) else {
        return iso.to_string();
    };
    let (y, m, d): (i32, u32, u32) = match (y.parse(), m.parse(), d.parse()) {
        (Ok(y), Ok(m), Ok(d)) => (y, m, d),
        _ => return iso.to_string(),
    };
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let len = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => 31,
    };
    let (y, m, d) = if d < len {
        (y, m, d + 1)
    } else if m < 12 {
        (y, m + 1, 1)
    } else {
        (y + 1, 1, 1)
    };
    format!("{y:04}-{m:02}-{d:02}")
}

/// Days since an arbitrary fixed epoch for `window_centers` arithmetic.
pub fn day_number(iso: &str) -> Option<i64> {
    let mut parts = iso.split('-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: i64 = parts.next()?.parse().ok()?;
    let d: i64 = parts.next()?.parse().ok()?;
    // Howard Hinnant's days-from-civil, the standard branchless form.
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146097 + doe)
}

/// `day_number`'s inverse.
pub fn iso_of_day_number(z: i64) -> String {
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = (mp + 2) % 12 + 1;
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid_day(date: &str, price: f64) -> GridDay {
        GridDay {
            date: date.into(),
            price,
            price_formatted: format!("{price} EUR"),
            option_id: "o".into(),
            booking_url: "https://example.org".into(),
            has_hidden_ground_transfer: false,
        }
    }

    #[test]
    fn a_span_is_covered_by_grid_windows_without_gaps() {
        let from = day_number("2026-09-01").unwrap();
        let to = day_number("2026-09-30").unwrap();
        let centers = window_centers(from, to, 10);
        // 30 days / 21-day windows = 2 searches.
        assert_eq!(centers.len(), 2);
        // Every span day is within ±10 of some center.
        for day in from..=to {
            assert!(
                centers.iter().any(|c| (day - c).abs() <= 10),
                "day {day} uncovered"
            );
        }
        assert_eq!(iso_of_day_number(centers[0]), "2026-09-11");
    }

    #[test]
    fn committed_days_rank_behind_free_days_regardless_of_price() {
        let entries = [
            CalendarSpan {
                starts_at: Some("2026-09-10T09:00:00".into()),
                ends_at: Some("2026-09-11T18:00:00".into()),
                title: Some("Kolloquium".into()),
                commitment: Some("committed".into()),
            },
            CalendarSpan {
                starts_at: Some("2026-09-20T00:00:00".into()),
                ends_at: None,
                title: Some("Maybe-BBQ".into()),
                commitment: Some("planned".into()),
            },
        ];
        let loads = day_loads("2026-09-09", "2026-09-21", &entries);
        let ranked = rank(
            vec![
                grid_day("2026-09-10", 19.99), // cheapest but committed
                grid_day("2026-09-20", 29.99), // planned
                grid_day("2026-09-09", 49.99), // free
            ],
            &loads,
        );
        assert_eq!(ranked[0].date, "2026-09-09");
        assert_eq!(ranked[0].load, DayLoad::Free);
        assert_eq!(ranked[1].date, "2026-09-20");
        assert_eq!(ranked[1].load, DayLoad::Planned);
        assert_eq!(ranked[2].date, "2026-09-10");
        assert_eq!(ranked[2].load, DayLoad::Committed);
        assert_eq!(ranked[2].collisions, vec!["Kolloquium".to_string()]);
    }

    #[test]
    fn month_and_year_boundaries_roll_correctly() {
        assert_eq!(next_day("2026-09-30"), "2026-10-01");
        assert_eq!(next_day("2026-12-31"), "2027-01-01");
        assert_eq!(next_day("2028-02-28"), "2028-02-29"); // leap
        assert_eq!(next_day("2026-02-28"), "2026-03-01");
        let n = day_number("2026-08-12").unwrap();
        assert_eq!(iso_of_day_number(n), "2026-08-12");
    }
}
