//! Reading the monthly parquet and folding it into cells.
//!
//! Five columns out of seventeen are projected, which is most of why a laptop can chew
//! through ~120M stop records without a database: parquet is columnar, so the fourteen
//! unread columns are never decompressed.

use crate::stats::Cell;
use arrow::array::{Array, BooleanArray, Int32Array, StringArray, TimestampNanosecondArray};
use arrow::record_batch::RecordBatch;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::arrow::ProjectionMask;
use std::collections::HashMap;
use std::path::Path;

const COLUMNS: [&str; 5] = ["eva", "train_type", "delay_in_min", "is_canceled", "time"];

#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    #[error("opening {0}: {1}")]
    Open(String, #[source] std::io::Error),
    #[error("reading parquet {0}: {1}")]
    Parquet(String, #[source] parquet::errors::ParquetError),
    #[error("reading parquet {0}: {1}")]
    Arrow(String, #[source] arrow::error::ArrowError),
    #[error("{file}: column `{column}` is missing — upstream schema changed (see upstreams.toml [deutsche-bahn-data], which records one such break in 2026-05)")]
    MissingColumn { file: String, column: String },
    #[error("{file}: column `{column}` has type {actual}, expected {expected}")]
    WrongType { file: String, column: String, actual: String, expected: String },
}

/// A station/type/hour/weekend bucket.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CellKey {
    pub eva: String,
    pub train_type: String,
    pub hour: u8,
    pub weekend: bool,
}

/// What one ingest run produced, so the caller can report it rather than guess.
#[derive(Debug, Default, Clone, Copy)]
pub struct Counts {
    pub rows: u64,
    /// Rows dropped for having no usable delay reading and no cancellation flag.
    pub skipped: u64,
}

/// Local wall-clock hour and weekend flag from a nanosecond timestamp.
///
/// No timezone library, deliberately: upstream states every timestamp is already
/// Europe/Berlin local time as the DB API returned it, with no conversion applied. So
/// the value IS the local clock, and treating it as one is correct — converting it
/// again would shift every reading by an hour or two and quietly move the rush hour.
fn hour_and_weekend(nanos: i64) -> (u8, bool) {
    let secs = nanos.div_euclid(1_000_000_000);
    let hour = (secs.rem_euclid(86_400) / 3_600) as u8;
    // Epoch day 0 was a Thursday; +3 rotates that to a Monday-first week.
    let weekday = (secs.div_euclid(86_400) + 3).rem_euclid(7);
    (hour, weekday >= 5)
}

fn column<'a, T: 'static>(
    batch: &'a RecordBatch,
    file: &str,
    name: &str,
    expected: &str,
) -> Result<&'a T, IngestError> {
    let idx = batch
        .schema()
        .index_of(name)
        .map_err(|_| IngestError::MissingColumn { file: file.into(), column: name.into() })?;
    let col = batch.column(idx);
    col.as_any().downcast_ref::<T>().ok_or_else(|| IngestError::WrongType {
        file: file.into(),
        column: name.into(),
        actual: format!("{:?}", col.data_type()),
        expected: expected.into(),
    })
}

/// Folds one monthly file into `cells`, adding to whatever is already there.
pub fn fold_file(
    path: &Path,
    cells: &mut HashMap<CellKey, Cell>,
    stations: &mut HashMap<String, String>,
) -> Result<Counts, IngestError> {
    let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
    let file = std::fs::File::open(path).map_err(|e| IngestError::Open(name.clone(), e))?;

    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .map_err(|e| IngestError::Parquet(name.clone(), e))?;

    // station_name rides along so the CLI can resolve a name to an eva without a second
    // pass or a hardcoded station list.
    let mut wanted: Vec<&str> = COLUMNS.to_vec();
    wanted.push("station_name");
    let schema = builder.parquet_schema();
    let indices: Vec<usize> = (0..schema.num_columns())
        .filter(|i| wanted.contains(&schema.column(*i).name()))
        .collect();
    let mask = ProjectionMask::roots(schema, indices);

    let reader = builder
        .with_projection(mask)
        .with_batch_size(65_536)
        .build()
        .map_err(|e| IngestError::Parquet(name.clone(), e))?;

    let mut counts = Counts::default();
    for batch in reader {
        let batch = batch.map_err(|e| IngestError::Arrow(name.clone(), e))?;
        let eva = column::<StringArray>(&batch, &name, "eva", "Utf8")?;
        let train_type = column::<StringArray>(&batch, &name, "train_type", "Utf8")?;
        let delay = column::<Int32Array>(&batch, &name, "delay_in_min", "Int32")?;
        let canceled = column::<BooleanArray>(&batch, &name, "is_canceled", "Boolean")?;
        let time = column::<TimestampNanosecondArray>(&batch, &name, "time", "Timestamp(Nanosecond)")?;
        let station_name = column::<StringArray>(&batch, &name, "station_name", "Utf8")?;

        for row in 0..batch.num_rows() {
            counts.rows += 1;
            if eva.is_null(row) || train_type.is_null(row) || time.is_null(row) {
                counts.skipped += 1;
                continue;
            }
            let is_canceled = !canceled.is_null(row) && canceled.value(row);
            if delay.is_null(row) && !is_canceled {
                // Neither a delay reading nor a cancellation: nothing this row can say.
                counts.skipped += 1;
                continue;
            }

            let (hour, weekend) = hour_and_weekend(time.value(row));
            let key = CellKey {
                eva: eva.value(row).to_string(),
                train_type: train_type.value(row).to_string(),
                hour,
                weekend,
            };
            cells
                .entry(key)
                .or_default()
                .record(if delay.is_null(row) { 0 } else { delay.value(row) }, is_canceled);

            if !station_name.is_null(row) {
                stations
                    .entry(eva.value(row).to_string())
                    .or_insert_with(|| station_name.value(row).to_string());
            }
        }
    }
    Ok(counts)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400 * 1_000_000_000;
    const HOUR: i64 = 3_600 * 1_000_000_000;

    #[test]
    fn epoch_zero_is_thursday_midnight() {
        let (hour, weekend) = hour_and_weekend(0);
        assert_eq!(hour, 0);
        assert!(!weekend, "1970-01-01 was a Thursday");
    }

    #[test]
    fn the_weekend_is_saturday_and_sunday() {
        // Thursday +2 = Saturday, +3 = Sunday, +4 = Monday.
        assert!(!hour_and_weekend(DAY).1, "Friday");
        assert!(hour_and_weekend(2 * DAY).1, "Saturday");
        assert!(hour_and_weekend(3 * DAY).1, "Sunday");
        assert!(!hour_and_weekend(4 * DAY).1, "Monday");
    }

    #[test]
    fn hours_advance_and_wrap_within_the_day() {
        assert_eq!(hour_and_weekend(9 * HOUR).0, 9);
        assert_eq!(hour_and_weekend(23 * HOUR).0, 23);
        assert_eq!(hour_and_weekend(24 * HOUR).0, 0);
    }

    #[test]
    fn timestamps_before_the_epoch_do_not_wrap_backwards() {
        // div_euclid rather than plain division: truncating toward zero would put
        // 1969-12-31 23:00 in hour -1, and a negative index into a 24-hour bucket is a
        // panic waiting for the one row with a bad timestamp.
        let (hour, _) = hour_and_weekend(-HOUR);
        assert_eq!(hour, 23);
        let (hour, weekend) = hour_and_weekend(-DAY);
        assert_eq!(hour, 0);
        assert!(!weekend, "1969-12-31 was a Wednesday");
    }
}
