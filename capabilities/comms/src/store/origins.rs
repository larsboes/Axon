//! Capture origins and collector-run projections.

use super::*;

impl Store {
    pub fn record_feed_origin(
        &self,
        feed_id: &str,
        source_id: &str,
        source_ref: &str,
        label: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut conn = self.conn()?;
        conn.execute(
            &format!(
                "INSERT INTO {schema}.feed_origins
                    (feed_id, source_id, source_ref, label, first_seen, last_seen)
                 VALUES ($1,$2,$3,$4,now(),now())
                 ON CONFLICT (feed_id, source_id, source_ref) DO UPDATE SET
                    label = COALESCE(excluded.label, {schema}.feed_origins.label),
                    last_seen = now()",
                schema = self.schema
            ),
            &[&feed_id, &source_id, &source_ref, &label],
        )?;
        Ok(())
    }

    pub fn feed_origins(
        &self,
        feed_id: &str,
    ) -> Result<Vec<FeedOrigin>, Box<dyn std::error::Error>> {
        let mut conn = self.conn()?;
        let rows = conn.query(
            &format!(
                "SELECT source_id, source_ref, label
                 FROM {}.feed_origins WHERE feed_id = $1 ORDER BY first_seen",
                self.schema
            ),
            &[&feed_id],
        )?;
        Ok(rows
            .iter()
            .map(|row| FeedOrigin {
                source_id: row.get(0),
                source_ref: row.get(1),
                label: row.get(2),
            })
            .collect())
    }

    pub fn list_origin_summaries(&self) -> Result<Vec<OriginSummary>, Box<dyn std::error::Error>> {
        let mut conn = self.conn()?;
        let rows = conn.query(
            &format!(
                "SELECT source_id, COUNT(DISTINCT feed_id), MIN(first_seen)::text, MAX(last_seen)::text
                 FROM {}.feed_origins
                 GROUP BY source_id
                 ORDER BY MAX(last_seen) DESC",
                self.schema
            ),
            &[],
        )?;
        Ok(rows
            .iter()
            .map(|r| OriginSummary {
                source_id: r.get(0),
                item_count: r.get(1),
                first_seen: r.get(2),
                last_seen: r.get(3),
            })
            .collect())
    }

    /// Which items arrived together, derived at read time from `feed_origins`
    /// alone (#84). A "run" is a cluster of arrivals for one source: ordered by
    /// `first_seen`, a gap longer than `RUN_GAP_MINUTES` starts a new one.
    ///
    /// Nothing is stored for this — no run id on the item, no batch table. A
    /// collector that fetches each URL can take a while, so the gap threshold
    /// is generous; two genuine runs of the same source inside half an hour
    /// read as one, which is the failure this trades for never having to
    /// migrate a grouping decision.
    pub fn list_feed_runs(&self, days: i32) -> Result<Vec<FeedRun>, Box<dyn std::error::Error>> {
        let mut conn = self.conn()?;
        let rows = conn.query(
            &format!(
                "WITH ordered AS (
                     SELECT o.feed_id, o.source_id, o.label, o.first_seen,
                            LAG(o.first_seen) OVER (
                                PARTITION BY o.source_id ORDER BY o.first_seen
                            ) AS prev_seen
                     FROM {schema}.feed_origins o
                     JOIN {schema}.feed_items f ON f.id = o.feed_id
                     WHERE f.day >= CURRENT_DATE - $1::int
                 ),
                 marked AS (
                     SELECT *,
                            CASE
                                WHEN prev_seen IS NULL
                                  OR first_seen - prev_seen > interval '{gap} minutes'
                                THEN 1 ELSE 0
                            END AS starts_run
                     FROM ordered
                 ),
                 runs AS (
                     SELECT feed_id, source_id, label, first_seen,
                            SUM(starts_run) OVER (
                                PARTITION BY source_id ORDER BY first_seen
                                ROWS UNBOUNDED PRECEDING
                            ) AS run_seq
                     FROM marked
                 )
                 SELECT feed_id,
                        source_id,
                        label,
                        source_id || '#' || run_seq::text AS run_key,
                        MIN(first_seen) OVER (PARTITION BY source_id, run_seq)::text AS run_started
                 FROM runs
                 ORDER BY run_started DESC, first_seen ASC",
                schema = self.schema,
                gap = RUN_GAP_MINUTES
            ),
            &[&days],
        )?;
        Ok(rows
            .iter()
            .map(|r| FeedRun {
                feed_id: r.get(0),
                source_id: r.get(1),
                label: r.get(2),
                run_key: r.get(3),
                run_started: r.get(4),
            })
            .collect())
    }
}
