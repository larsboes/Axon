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
        let conn = self.conn()?;
        conn.execute(
            &format!(
                "INSERT INTO {prefix}_source_state (source_name, last_run_at, cursor)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT (source_name) DO UPDATE SET
                     last_run_at = excluded.last_run_at,
                     cursor = COALESCE(excluded.cursor, {prefix}_source_state.cursor)",
                prefix = self.prefix
            ),
            params![&source_name, &now, &cursor],
        )?;
        Ok(())
    }

    pub fn get_source_state(
        &self,
        source_name: &str,
    ) -> Result<Option<SourceState>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        Ok(conn
            .query_row(
                &format!(
                    "SELECT source_name, last_run_at, cursor, last_success_at, last_failure_at,
                            last_error, considered_count, new_count, consecutive_failures
                     FROM {}_source_state WHERE source_name = ?1",
                    self.prefix
                ),
                params![&source_name],
                |r| {
                    Ok(SourceState {
                        source_name: r.get(0)?,
                        last_run_at: r.get(1)?,
                        cursor: r.get(2)?,
                        last_success_at: r.get(3)?,
                        last_failure_at: r.get(4)?,
                        last_error: r.get(5)?,
                        considered_count: r.get(6)?,
                        new_count: r.get(7)?,
                        consecutive_failures: r.get(8)?,
                    })
                },
            )
            .optional()?)
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
        let conn = self.conn()?;
        conn.execute(
            &format!(
                "INSERT INTO {prefix}_source_state
                     (source_name, last_run_at, last_success_at, considered_count,
                      new_count, consecutive_failures)
                 VALUES (?1, ?2, ?2, ?3, ?4, 0)
                 ON CONFLICT (source_name) DO UPDATE SET
                     last_run_at = excluded.last_run_at,
                     last_success_at = excluded.last_success_at,
                     considered_count = excluded.considered_count,
                     new_count = excluded.new_count,
                     consecutive_failures = 0,
                     last_error = NULL",
                prefix = self.prefix
            ),
            params![&source_name, &now, considered, new_items],
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
        let conn = self.conn()?;
        Ok(conn.query_row(
            &format!(
                "INSERT INTO {prefix}_source_state
                     (source_name, last_run_at, last_failure_at, last_error, consecutive_failures)
                 VALUES (?1, ?2, ?2, ?3, 1)
                 ON CONFLICT (source_name) DO UPDATE SET
                     last_run_at = excluded.last_run_at,
                     last_failure_at = excluded.last_failure_at,
                     last_error = excluded.last_error,
                     consecutive_failures = {prefix}_source_state.consecutive_failures + 1
                 RETURNING consecutive_failures",
                prefix = self.prefix
            ),
            params![&source_name, &now, &error_class],
            |row| row.get(0),
        )?)
    }

    /// Whether the store's clock currently sits inside a quiet window, given
    /// `[start, end)` in UTC hours. Asked of the store rather than computed in
    /// Rust: its clock is the one every other timestamp here comes from, and
    /// comms carries no date library to disagree with it. UTC, because the
    /// Postgres instance this replaced ran in UTC and `EXTRACT(HOUR FROM now())`
    /// therefore answered the UTC hour -- `strftime('%H','now')` answers the
    /// same one.
    pub fn within_quiet_hours(
        &self,
        start_hour: u32,
        end_hour: u32,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        if start_hour == end_hour {
            return Ok(false);
        }
        let conn = self.conn()?;
        let hour: i32 =
            conn.query_row("SELECT CAST(strftime('%H','now') AS INTEGER)", [], |row| {
                row.get(0)
            })?;
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
