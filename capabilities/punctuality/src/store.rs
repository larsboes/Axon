//! Postgres persistence. Own schema (`punctuality`) inside the one shared instance,
//! same arrangement scouting and transit already use.
//!
//! Only the aggregate lands here — roughly 400k summary rows, not the ~120M stop
//! records they were computed from. That is a backup decision as much as a storage one:
//! `capabilities/postgres` is covered by `pg_dumpall`, so anything in this database is
//! in every backup forever, while the raw parquet is a cache that upstream can hand back
//! at any time.

use crate::ingest::CellKey;
use crate::stats::Cell;
use postgres::Client;
use std::collections::HashMap;

pub struct Store {
    /// Pooled like its six siblings, though this capability is the one that gains
    /// least from it: it opens a store twice per process, not once per request. It
    /// is here because the shared axon-store crate owns both migration and pooling, so
    /// a consumer that wants its migration half carries its pool half too — and
    /// carrying the dependency while hand-rolling a second connection strategy
    /// beside it would be the worse of the two outcomes.
    pool: axon_store::Pool,
    schema: String,
}

pub type Coverage = (String, String, i32);

/// The dataset writes EVA numbers zero-padded to eight digits (`08000044`); HAFAS, and
/// therefore `capabilities/transit`, returns them unpadded (`8000044`). Joining the two
/// without this returns zero rows and looks exactly like "we have no data for that
/// station" — which is how it presented the first time.
pub fn normalize_eva(eva: &str) -> String {
    let digits = eva.trim_start_matches('0');
    format!("{digits:0>8}")
}

#[derive(Debug, Clone)]
pub struct StatRow {
    pub eva: String,
    pub station_name: Option<String>,
    pub train_type: String,
    pub hour: i16,
    pub weekend: bool,
    pub n: i64,
    pub canceled: i64,
    pub mean_delay: f32,
    pub p50: i16,
    pub p90: i16,
    pub share_late_6: f32,
    pub cancel_rate: f32,
}

impl Store {
    pub fn open(database_url: &str) -> Result<Self, Box<dyn std::error::Error>> {
        Self::open_with_schema(database_url, "punctuality")
    }

    /// `schema` is either the literal `"punctuality"` or a test-generated name built
    /// from a static prefix plus this process's pid — never user input. Postgres has no
    /// parametrized-identifier syntax for DDL, so schema-qualified names are built with
    /// `format!` below; that is safe because of where the name comes from, not because
    /// interpolating into SQL is safe in general.
    pub fn open_with_schema(
        database_url: &str,
        schema: &str,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        // A pool checkout, not a connect, and the migration runs once per process
        // per (database, schema) rather than once per open. Both halves of the
        // Store::open problem -- libs/axon-store/README.md has the numbers.
        let pool = axon_store::open_pool(database_url, schema, |client| {
            Self::init_schema(client, schema)
        })?;
        Ok(Self {
            pool,
            schema: schema.to_string(),
        })
    }

    /// A connection from the shared pool.
    ///
    /// Held across a whole transaction by `replace_stats`, which is the intended
    /// use of a checkout rather than a problem with one: the connection is this
    /// caller's until it is dropped, and r2d2 hands the next caller a different one.
    fn conn(&self) -> Result<axon_store::PooledClient, Box<dyn std::error::Error>> {
        Ok(self.pool.get()?)
    }

    fn init_schema(client: &mut Client, schema: &str) -> Result<(), Box<dyn std::error::Error>> {
        client.batch_execute(&format!(
            "
            CREATE SCHEMA IF NOT EXISTS {schema};

            CREATE TABLE IF NOT EXISTS {schema}.stations (
                eva          TEXT PRIMARY KEY,
                station_name TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS {schema}.stop_stats (
                eva           TEXT     NOT NULL,
                train_type    TEXT     NOT NULL,
                hour          SMALLINT NOT NULL,
                weekend       BOOLEAN  NOT NULL,
                n             BIGINT   NOT NULL,
                canceled      BIGINT   NOT NULL,
                mean_delay    REAL     NOT NULL,
                p50           SMALLINT NOT NULL,
                p90           SMALLINT NOT NULL,
                share_late_6  REAL     NOT NULL,
                cancel_rate   REAL     NOT NULL,
                PRIMARY KEY (eva, train_type, hour, weekend)
            );

            -- What window produced the current contents. Without it, a table of numbers
            -- says nothing about which months it covers, and a partial re-ingest would
            -- be indistinguishable from a complete one.
            CREATE TABLE IF NOT EXISTS {schema}.ingest_runs (
                id           BIGSERIAL PRIMARY KEY,
                ran_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
                from_month   TEXT NOT NULL,
                to_month     TEXT NOT NULL,
                months       INT  NOT NULL,
                rows_read    BIGINT NOT NULL,
                rows_skipped BIGINT NOT NULL,
                cells        INT  NOT NULL
            );
            "
        ))?;
        Ok(())
    }

    /// Replaces the aggregate wholesale inside one transaction.
    ///
    /// Replace rather than upsert, because the rows are a *function* of the ingested
    /// window: narrowing the window and merging into what was there would leave rows
    /// from months no longer covered, and nothing in the table would show it.
    pub fn replace_stats(
        &mut self,
        cells: &HashMap<CellKey, Cell>,
        stations: &HashMap<String, String>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let schema = self.schema.clone();
        let mut conn = self.conn()?;
        let mut tx = conn.transaction()?;
        tx.execute(&format!("DELETE FROM {schema}.stop_stats"), &[])?;

        let insert = tx.prepare(&format!(
            "INSERT INTO {schema}.stop_stats
               (eva, train_type, hour, weekend, n, canceled, mean_delay, p50, p90, share_late_6, cancel_rate)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)"
        ))?;
        for (key, cell) in cells {
            tx.execute(
                &insert,
                &[
                    &key.eva,
                    &key.train_type,
                    &(key.hour as i16),
                    &key.weekend,
                    &(cell.n as i64),
                    &(cell.canceled as i64),
                    &(cell.mean() as f32),
                    &(cell.quantile(0.5) as i16),
                    &(cell.quantile(0.9) as i16),
                    &(cell.share_at_least(6) as f32),
                    &(cell.cancel_rate() as f32),
                ],
            )?;
        }

        let station_insert = tx.prepare(&format!(
            "INSERT INTO {schema}.stations (eva, station_name) VALUES ($1,$2)
             ON CONFLICT (eva) DO UPDATE SET station_name = EXCLUDED.station_name"
        ))?;
        for (eva, name) in stations {
            tx.execute(&station_insert, &[eva, name])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn record_run(
        &mut self,
        from: &str,
        to: &str,
        months: i32,
        rows: i64,
        skipped: i64,
        cells: i32,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let schema = &self.schema;
        self.conn()?.execute(
            &format!(
                "INSERT INTO {schema}.ingest_runs (from_month, to_month, months, rows_read, rows_skipped, cells)
                 VALUES ($1,$2,$3,$4,$5,$6)"
            ),
            &[&from, &to, &months, &rows, &skipped, &cells],
        )?;
        Ok(())
    }

    /// Statistics for one station, optionally narrowed to a train type, ordered by hour.
    /// `min_n` drops cells too thin to mean anything — a 100% late rate over three
    /// observations is noise wearing a percentage sign.
    pub fn station_stats(
        &mut self,
        eva: &str,
        train_type: Option<&str>,
        min_n: i64,
    ) -> Result<Vec<StatRow>, Box<dyn std::error::Error>> {
        let schema = &self.schema;
        let eva = normalize_eva(eva);
        let sql = format!(
            "SELECT s.eva, st.station_name, s.train_type, s.hour, s.weekend, s.n, s.canceled,
                    s.mean_delay, s.p50, s.p90, s.share_late_6, s.cancel_rate
             FROM {schema}.stop_stats s
             LEFT JOIN {schema}.stations st ON st.eva = s.eva
             WHERE s.eva = $1 AND ($2::text IS NULL OR s.train_type = $2) AND s.n >= $3
             ORDER BY s.train_type, s.weekend, s.hour"
        );
        let rows = self.conn()?.query(&sql, &[&eva, &train_type, &min_n])?;
        Ok(rows.iter().map(row_to_stat).collect())
    }

    /// One cell, or `None` when it does not exist or is thinner than `min_n`.
    ///
    /// `None` deliberately does not degrade to a neighbouring hour or to the station's
    /// average. Both would answer a question nobody asked and would be indistinguishable
    /// from a real reading downstream.
    pub fn stop_stats(
        &mut self,
        eva: &str,
        train_type: &str,
        hour: i16,
        weekend: bool,
        min_n: i64,
    ) -> Result<Option<StatRow>, Box<dyn std::error::Error>> {
        let schema = &self.schema;
        let eva = normalize_eva(eva);
        let rows = self.conn()?.query(
            &format!(
                "SELECT s.eva, st.station_name, s.train_type, s.hour, s.weekend, s.n, s.canceled,
                        s.mean_delay, s.p50, s.p90, s.share_late_6, s.cancel_rate
                 FROM {schema}.stop_stats s
                 LEFT JOIN {schema}.stations st ON st.eva = s.eva
                 WHERE s.eva = $1 AND s.train_type = $2 AND s.hour = $3 AND s.weekend = $4
                   AND s.n >= $5"
            ),
            &[&eva, &train_type, &hour, &weekend, &min_n],
        )?;
        Ok(rows.first().map(row_to_stat))
    }

    /// The window the current aggregate covers, from the most recent ingest run.
    /// `None` means nothing has been ingested — which a caller must be able to tell
    /// apart from "this train is never late".
    pub fn coverage(&mut self) -> Result<Option<Coverage>, Box<dyn std::error::Error>> {
        let schema = &self.schema;
        let rows = self.conn()?.query(
            &format!(
                "SELECT from_month, to_month, cells FROM {schema}.ingest_runs
                 ORDER BY id DESC LIMIT 1"
            ),
            &[],
        )?;
        Ok(rows.first().map(|r| (r.get(0), r.get(1), r.get(2))))
    }

    /// EVA numbers whose station name contains `needle`, case-insensitively.
    pub fn find_stations(
        &mut self,
        needle: &str,
    ) -> Result<Vec<(String, String)>, Box<dyn std::error::Error>> {
        let schema = &self.schema;
        let rows = self.conn()?.query(
            &format!(
                "SELECT eva, station_name FROM {schema}.stations
                 WHERE station_name ILIKE '%' || $1 || '%' ORDER BY station_name LIMIT 25"
            ),
            &[&needle],
        )?;
        Ok(rows.iter().map(|r| (r.get(0), r.get(1))).collect())
    }
}

fn row_to_stat(r: &postgres::Row) -> StatRow {
    StatRow {
        eva: r.get(0),
        station_name: r.get(1),
        train_type: r.get(2),
        hour: r.get(3),
        weekend: r.get(4),
        n: r.get(5),
        canceled: r.get(6),
        mean_delay: r.get(7),
        p50: r.get(8),
        p90: r.get(9),
        share_late_6: r.get(10),
        cancel_rate: r.get(11),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eva_normalization_bridges_the_dataset_and_hafas() {
        // The two forms of the same station must land on one key.
        assert_eq!(normalize_eva("8000044"), "08000044");
        assert_eq!(normalize_eva("08000044"), "08000044");
        assert_eq!(normalize_eva("8000044"), normalize_eva("08000044"));
    }

    #[test]
    fn normalization_does_not_truncate_a_longer_id() {
        assert_eq!(normalize_eva("123456789"), "123456789");
    }

    #[test]
    fn an_all_zero_id_does_not_become_empty() {
        // trim_start_matches would eat the whole string; the pad has to put it back.
        assert_eq!(normalize_eva("0"), "00000000");
    }
}
