//! One train's actual stops on one day, from the same published files the
//! aggregate is built from.
//!
//! `ingest` projects six of the sixteen columns in each monthly parquet and
//! folds the rest away into per-cell histograms. That is the right shape for
//! "how late is an ICE at this station at this hour", and it cannot answer "how
//! late was ICE 611 at Bonn Hbf on the 14th" at all, because the ride is gone by
//! the time the histogram exists.
//!
//! The columns to answer it are already on disk: `train_number`,
//! `train_line_ride_id`, `train_line_station_num`, and the planned/actual
//! arrival and departure times are decompressed on every ingest and discarded.
//! This reads them instead.
//!
//! Three honesty constraints come with the dataset and are part of the answer:
//!
//! - Publication lags a journey by up to five weeks, so a recent trip is simply
//!   not here yet. That is different from a train that did not run.
//! - Collection is 98.92% complete with named missing hours, so an absent row
//!   never means "did not run" either.
//! - The raw directory is a prunable cache. A month that was pruned reads as
//!   unavailable, and this says so rather than returning nothing.
//!
//! All three collapse into the same rule: an absent row is reported as absent,
//! with the reason, and never as an empty result that looks like a measurement.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use arrow::array::{Array, BooleanArray, Int32Array, StringArray};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::arrow::ProjectionMask;
use serde::Serialize;

/// One observed stop of one train.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Stop {
    pub eva: String,
    pub station_name: Option<String>,
    pub train_type: String,
    pub train_number: String,
    pub line_number: Option<String>,
    /// Which stop this is along the ride, so a caller can order them without
    /// trusting the file's row order.
    pub stop_index: Option<i32>,
    pub final_destination: Option<String>,
    pub arrival_planned: Option<String>,
    pub arrival_actual: Option<String>,
    pub departure_planned: Option<String>,
    pub departure_actual: Option<String>,
    pub delay_minutes: Option<i32>,
    pub canceled: bool,
}

/// What a ride query found, and what it could not look at.
#[derive(Debug, Clone, Serialize)]
pub struct RideAnswer {
    pub train_type: String,
    pub train_number: String,
    pub date: String,
    pub stops: Vec<Stop>,
    /// Why `stops` may be empty. `None` means the file was read and the train
    /// genuinely has no rows in it.
    pub unavailable: Option<String>,
    /// Always stated, because a reader who does not know the collection is
    /// incomplete will read an absent stop as a cancelled train.
    pub caveat: &'static str,
}

pub const CAVEAT: &str = "An absent stop is not a train that did not run: collection is 98.92% \
     complete with named missing hours, and publication lags a journey by up to five weeks.";

/// The month file a date belongs to, as `YYYY-MM`.
pub fn month_of(date: &str) -> Option<String> {
    if date.len() < 7 {
        return None;
    }
    let (year, rest) = date.split_at(4);
    let month = rest.strip_prefix('-')?.get(..2)?;
    if !year.chars().all(|c| c.is_ascii_digit()) || !month.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(format!("{year}-{month}"))
}

/// Normalises a train number for comparison. DB writes `ICE 611`, `ICE611` and
/// `611` for the same train depending on the column, and a string compare that
/// misses is indistinguishable from a train that did not run.
pub fn same_train_number(left: &str, right: &str) -> bool {
    let digits = |value: &str| -> String {
        value.chars().filter(char::is_ascii_digit).collect()
    };
    let left_digits = digits(left);
    !left_digits.is_empty() && left_digits == digits(right)
}

const COLUMNS: [&str; 13] = [
    "eva",
    "station_name",
    "train_type",
    "train_number",
    "line_number",
    "train_line_station_num",
    "final_destination_station",
    "arrival_planned_time",
    "arrival_change_time",
    "departure_planned_time",
    "departure_change_time",
    "delay_in_min",
    "is_canceled",
];

fn text(batch: &arrow::record_batch::RecordBatch, name: &str, row: usize) -> Option<String> {
    let index = batch.schema().index_of(name).ok()?;
    let column = batch.column(index).as_any().downcast_ref::<StringArray>()?;
    if column.is_null(row) {
        return None;
    }
    Some(column.value(row).to_string())
}

fn number(batch: &arrow::record_batch::RecordBatch, name: &str, row: usize) -> Option<i32> {
    let index = batch.schema().index_of(name).ok()?;
    let column = batch.column(index).as_any().downcast_ref::<Int32Array>()?;
    if column.is_null(row) {
        return None;
    }
    Some(column.value(row))
}

fn flag(batch: &arrow::record_batch::RecordBatch, name: &str, row: usize) -> bool {
    batch
        .schema()
        .index_of(name)
        .ok()
        .and_then(|i| batch.column(i).as_any().downcast_ref::<BooleanArray>())
        .is_some_and(|c| !c.is_null(row) && c.value(row))
}

/// Reads one train's stops for one date out of the cached month file.
pub fn find(
    raw_dir: &Path,
    train_type: &str,
    train_number: &str,
    date: &str,
    eva: Option<&str>,
) -> Result<RideAnswer, String> {
    let answer = |stops, unavailable| RideAnswer {
        train_type: train_type.to_string(),
        train_number: train_number.to_string(),
        date: date.to_string(),
        stops,
        unavailable,
        caveat: CAVEAT,
    };

    let Some(month) = month_of(date) else {
        return Err(format!("date must be YYYY-MM-DD, got {date:?}"));
    };
    let path: PathBuf = raw_dir.join(format!("data-{month}.parquet"));
    if !path.is_file() {
        // The cache is prunable, so this is an ordinary state, not an error.
        return Ok(answer(
            Vec::new(),
            Some(format!(
                "{month} is not in the local cache ({}). Run `punctuality ingest` to fetch it.",
                raw_dir.display()
            )),
        ));
    }

    let file = std::fs::File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let schema = builder.parquet_schema();
    let indices: Vec<usize> = (0..schema.num_columns())
        .filter(|i| COLUMNS.contains(&schema.column(*i).name()))
        .collect();
    let reader = builder
        .with_projection(ProjectionMask::roots(schema, indices))
        .with_batch_size(65_536)
        .build()
        .map_err(|e| format!("{}: {e}", path.display()))?;

    let mut stops = Vec::new();
    for batch in reader {
        let batch = batch.map_err(|e| format!("{}: {e}", path.display()))?;
        for row in 0..batch.num_rows() {
            let Some(row_type) = text(&batch, "train_type", row) else {
                continue;
            };
            if !row_type.eq_ignore_ascii_case(train_type) {
                continue;
            }
            let Some(row_number) = text(&batch, "train_number", row) else {
                continue;
            };
            if !same_train_number(&row_number, train_number) {
                continue;
            }
            // The day is decided by the planned times, not by `time`: a train
            // that departs before midnight and arrives after it belongs to the
            // day it was scheduled on.
            let arrival_planned = text(&batch, "arrival_planned_time", row);
            let departure_planned = text(&batch, "departure_planned_time", row);
            let on_date = [&arrival_planned, &departure_planned]
                .into_iter()
                .flatten()
                .any(|value| value.starts_with(date));
            if !on_date {
                continue;
            }
            let row_eva = text(&batch, "eva", row).unwrap_or_default();
            if let Some(wanted) = eva {
                if row_eva.trim_start_matches('0') != wanted.trim_start_matches('0') {
                    continue;
                }
            }
            stops.push(Stop {
                eva: row_eva,
                station_name: text(&batch, "station_name", row),
                train_type: row_type,
                train_number: row_number,
                line_number: text(&batch, "line_number", row),
                stop_index: number(&batch, "train_line_station_num", row),
                final_destination: text(&batch, "final_destination_station", row),
                arrival_planned,
                arrival_actual: text(&batch, "arrival_change_time", row),
                departure_planned,
                departure_actual: text(&batch, "departure_change_time", row),
                delay_minutes: number(&batch, "delay_in_min", row),
                canceled: flag(&batch, "is_canceled", row),
            });
        }
    }

    stops.sort_by(|a, b| {
        a.stop_index
            .cmp(&b.stop_index)
            .then_with(|| a.departure_planned.cmp(&b.departure_planned))
    });
    Ok(answer(stops, None))
}

/// Which months the cache actually holds, so a caller can see the window.
pub fn cached_months(raw_dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(raw_dir) else {
        return Vec::new();
    };
    let mut months: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            name.strip_prefix("data-")
                .and_then(|rest| rest.strip_suffix(".parquet"))
                .map(str::to_string)
        })
        .collect();
    months.sort();
    months
}

/// Counts stops per station, for a quick read of where a train actually stops.
pub fn stations_seen(stops: &[Stop]) -> HashMap<String, usize> {
    let mut seen = HashMap::new();
    for stop in stops {
        *seen
            .entry(stop.station_name.clone().unwrap_or_else(|| stop.eva.clone()))
            .or_insert(0) += 1;
    }
    seen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_date_maps_to_its_month_file() {
        assert_eq!(month_of("2026-03-14").as_deref(), Some("2026-03"));
        assert_eq!(month_of("2026-03").as_deref(), Some("2026-03"));
        assert_eq!(month_of("14.03.2026"), None);
        assert_eq!(month_of("nonsense"), None);
        assert_eq!(month_of("2026"), None);
    }

    /// DB writes the same train as `ICE 611`, `ICE611` and `611` depending on
    /// the column. A plain string compare that misses looks exactly like a
    /// train that did not run, which is the failure this dataset makes easy.
    #[test]
    fn a_train_number_matches_across_the_ways_db_writes_it() {
        assert!(same_train_number("ICE 611", "611"));
        assert!(same_train_number("611", "ICE611"));
        assert!(same_train_number(" 611 ", "611"));
        assert!(!same_train_number("611", "612"));
        // No digits on either side is not a match, it is an unanswerable
        // comparison, and treating it as equal would return every train.
        assert!(!same_train_number("ICE", "ICE"));
        assert!(!same_train_number("", ""));
    }

    #[test]
    fn a_pruned_month_is_reported_as_unavailable_not_as_no_train() {
        let empty = std::env::temp_dir().join(format!("axon-ride-test-{}", std::process::id()));
        std::fs::create_dir_all(&empty).unwrap();
        let answer = find(&empty, "ICE", "611", "2026-03-14", None).unwrap();
        assert!(answer.stops.is_empty());
        let reason = answer
            .unavailable
            .expect("a missing month file must say so rather than look like no data");
        assert!(reason.contains("2026-03"), "got: {reason}");
        assert!(reason.contains("ingest"), "the reason must name the fix");
        let _ = std::fs::remove_dir_all(&empty);
    }

    #[test]
    fn cached_months_reads_the_window_off_disk() {
        let dir = std::env::temp_dir().join(format!("axon-ride-months-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for month in ["2026-02", "2026-01"] {
            std::fs::write(dir.join(format!("data-{month}.parquet")), b"x").unwrap();
        }
        std::fs::write(dir.join("notes.txt"), b"x").unwrap();
        assert_eq!(cached_months(&dir), vec!["2026-01", "2026-02"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
