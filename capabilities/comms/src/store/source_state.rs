//! Collector run health, failure streaks, and quiet-hour decisions.

use super::*;

impl Store {
    // -- source_state ----------------------------------------------------

    pub fn record_run(
        &self,
        source_name: &str,
        cursor: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let now = epoch_now();
        let mut conn = self.conn()?;
        conn.execute(
            &format!(
                "INSERT INTO {schema}.source_state (source_name, last_run_at, cursor)
                 VALUES ($1, $2, $3)
                 ON CONFLICT (source_name) DO UPDATE SET
                     last_run_at = excluded.last_run_at,
                     cursor = COALESCE(excluded.cursor, {schema}.source_state.cursor)",
                schema = self.schema
            ),
            &[&source_name, &now, &cursor],
        )?;
        Ok(())
    }

    pub fn get_source_state(
        &self,
        source_name: &str,
    ) -> Result<Option<SourceState>, Box<dyn std::error::Error>> {
        let mut conn = self.conn()?;
        let row = conn.query_opt(
            &format!(
                "SELECT source_name, last_run_at, cursor, last_success_at, last_failure_at,
                        last_error, considered_count, new_count, consecutive_failures
                 FROM {}.source_state WHERE source_name = $1",
                self.schema
            ),
            &[&source_name],
        )?;
        Ok(row.map(|r| SourceState {
            source_name: r.get(0),
            last_run_at: r.get(1),
            cursor: r.get(2),
            last_success_at: r.get(3),
            last_failure_at: r.get(4),
            last_error: r.get(5),
            considered_count: r.get(6),
            new_count: r.get(7),
            consecutive_failures: r.get(8),
        }))
    }

    /// Record a completed pass. Success clears the failure streak; the counts
    /// describe the pass that just ran, not a running total, because "how much
    /// did the last run see" is the question a stale schedule raises.
    pub fn record_sweep_success(
        &self,
        source_name: &str,
        considered: i64,
        new_items: i64,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let now = epoch_now();
        let mut conn = self.conn()?;
        conn.execute(
            &format!(
                "INSERT INTO {schema}.source_state
                     (source_name, last_run_at, last_success_at, considered_count,
                      new_count, consecutive_failures)
                 VALUES ($1, $2, $2, $3, $4, 0)
                 ON CONFLICT (source_name) DO UPDATE SET
                     last_run_at = excluded.last_run_at,
                     last_success_at = excluded.last_success_at,
                     considered_count = excluded.considered_count,
                     new_count = excluded.new_count,
                     consecutive_failures = 0,
                     last_error = NULL",
                schema = self.schema
            ),
            &[&source_name, &now, &considered, &new_items],
        )?;
        Ok(())
    }

    /// `error_class` is a short stable label — `auth`, `quota`, `network`,
    /// `store`. Never a provider message: those quote request URLs and, for
    /// mail, occasionally the subject that failed.
    pub fn record_sweep_failure(
        &self,
        source_name: &str,
        error_class: &str,
    ) -> Result<i32, Box<dyn std::error::Error>> {
        let now = epoch_now();
        let mut conn = self.conn()?;
        let row = conn.query_one(
            &format!(
                "INSERT INTO {schema}.source_state
                     (source_name, last_run_at, last_failure_at, last_error, consecutive_failures)
                 VALUES ($1, $2, $2, $3, 1)
                 ON CONFLICT (source_name) DO UPDATE SET
                     last_run_at = excluded.last_run_at,
                     last_failure_at = excluded.last_failure_at,
                     last_error = excluded.last_error,
                     consecutive_failures = {schema}.source_state.consecutive_failures + 1
                 RETURNING consecutive_failures",
                schema = self.schema
            ),
            &[&source_name, &now, &error_class],
        )?;
        Ok(row.get(0))
    }

    /// Whether the store's clock currently sits inside a quiet window, given
    /// `[start, end)` in local hours. Asked of Postgres rather than computed in
    /// Rust: the store's clock is the one every other timestamp here comes
    /// from, and comms carries no date library to disagree with it.
    pub fn within_quiet_hours(
        &self,
        start_hour: u32,
        end_hour: u32,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        if start_hour == end_hour {
            return Ok(false);
        }
        let mut conn = self.conn()?;
        let row = conn.query_one("SELECT EXTRACT(HOUR FROM now())::INTEGER", &[])?;
        let hour: i32 = row.get(0);
        let (start, end) = (start_hour as i32, end_hour as i32);
        // A window that wraps midnight (22→7) is the normal case, so it is the
        // one spelled out rather than the one left to fall through.
        Ok(if start < end {
            hour >= start && hour < end
        } else {
            hour >= start || hour < end
        })
    }
}
