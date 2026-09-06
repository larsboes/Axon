//! The date, and the two conversions either side of it.
//!
//! These lived in `src/server.rs`, which is a `[[bin]]` (`Cargo.toml`'s
//! `[[bin]] name = "finance-server"`). Nothing in the library and nothing in
//! `finance-cli` could name them, so a lib module that needed today's date had
//! the choice of taking a timestamp through every signature or writing a ninth
//! copy of Hinnant's algorithm. Moving them here is subtraction: `server.rs`
//! calls the same functions it used to define.
//!
//! UTC, with no timezone database. A billing date, a price observation date and
//! a proposal date are not precise to the hour, and a tz dependency would be
//! bought for a boundary case this capability does not have.

/// Today as an ISO date, from the wall clock, with no date dependency.
pub fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    civil_from_days(secs / 86_400)
}

/// Now as second-granular RFC3339.
///
/// The granularity is load-bearing rather than cosmetic: `finance_decision_events`
/// keys on `(decision_id, event, recorded_at)`, so a date-granular stamp would
/// turn a same-day corrected verdict into a constraint error.
pub fn now_timestamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);
    let seconds = secs.rem_euclid(86_400);
    format!(
        "{}T{:02}:{:02}:{:02}Z",
        civil_from_days(secs.div_euclid(86_400)),
        seconds / 3_600,
        seconds % 3_600 / 60,
        seconds % 60
    )
}

/// Days since the Unix epoch as an ISO date (Howard Hinnant's algorithm, public
/// domain).
pub fn civil_from_days(days: i64) -> String {
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

/// An ISO date as days since the Unix epoch, or `None` when the string is not a
/// real date. The inverse of [`civil_from_days`].
pub fn iso_day(value: &str) -> Option<i64> {
    if !valid_iso_date(value) {
        return None;
    }
    let year = value[0..4].parse::<i64>().ok()?;
    let month = value[5..7].parse::<i64>().ok()?;
    let day = value[8..10].parse::<i64>().ok()?;
    let adjusted_year = year - i64::from(month <= 2);
    let era = adjusted_year.div_euclid(400);
    let year_of_era = adjusted_year - era * 400;
    let shifted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    Some(era * 146_097 + day_of_era - 719_468)
}

/// Whether a string is a real calendar date in `YYYY-MM-DD`. February 30th is
/// refused, not clamped.
pub fn valid_iso_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return false;
    }
    if bytes
        .iter()
        .enumerate()
        .any(|(index, byte)| !matches!(index, 4 | 7) && !byte.is_ascii_digit())
    {
        return false;
    }

    let Ok(year) = value[0..4].parse::<u32>() else {
        return false;
    };
    let Ok(month) = value[5..7].parse::<u32>() else {
        return false;
    };
    let Ok(day) = value[8..10].parse::<u32>() else {
        return false;
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let last_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=last_day).contains(&day)
}

/// Whole days between two ISO dates, `None` when either is not a date.
pub fn days_between(from: &str, to: &str) -> Option<i64> {
    Some(iso_day(to)? - iso_day(from)?)
}

/// The `YYYY-MM` a date falls in, or `None` when it is not a date.
pub fn month_of(date: &str) -> Option<String> {
    valid_iso_date(date).then(|| date[0..7].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_date_conversion_matches_known_days() {
        assert_eq!(civil_from_days(0), "1970-01-01");
        assert_eq!(civil_from_days(19_723), "2024-01-01");
        // 2024 was a leap year; the day after 02-28 is 02-29, not 03-01.
        assert_eq!(civil_from_days(19_782), "2024-02-29");
        assert_eq!(civil_from_days(20_673), "2026-08-08");
        assert_eq!(iso_day("1970-01-01"), Some(0));
        assert_eq!(iso_day("2024-02-29"), Some(19_782));
        assert_eq!(iso_day("2026-08-08"), Some(20_673));
    }

    #[test]
    fn today_is_an_iso_date() {
        let value = today();
        assert_eq!(value.len(), 10);
        assert!(valid_iso_date(&value));
    }

    #[test]
    fn dates_are_calendar_dates_not_only_ten_characters() {
        assert!(valid_iso_date("2024-02-29"));
        assert!(valid_iso_date("2026-08-08"));
        assert!(!valid_iso_date("2026-02-29"));
        assert!(!valid_iso_date("2026-13-01"));
        assert!(!valid_iso_date("202610-01-08"));
    }

    #[test]
    fn a_timestamp_is_second_granular_so_a_corrected_verdict_is_a_second_row() {
        let value = now_timestamp();
        assert_eq!(value.len(), 20, "{value}");
        assert!(value.ends_with('Z'));
        assert!(valid_iso_date(&value[0..10]));
    }

    #[test]
    fn a_month_is_the_first_seven_characters_of_a_real_date() {
        assert_eq!(month_of("2026-09-05").as_deref(), Some("2026-09"));
        assert_eq!(month_of("2026-02-30"), None);
        assert_eq!(days_between("2026-09-01", "2026-09-05"), Some(4));
    }
}
