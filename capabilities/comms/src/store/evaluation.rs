//! Stored feed evaluations and the travel-context snapshot they consume.

use super::*;

impl Store {
    /// Store the complete evaluation and its factors atomically. The factor
    /// table is normalized so future trip/deadline factors can be added without
    /// a schema migration or an opaque JSON payload.
    pub fn replace_feed_evaluation(
        &self,
        evaluation: &FeedEvaluation,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let mut conn = self.conn()?;
        let transaction = conn.transaction()?;
        let tier = provenance::ranking_tier(&evaluation.mode);
        let affected = transaction.execute(
            &format!(
                "INSERT INTO {prefix}_feed_evaluations
                    (feed_id, overall_score, explanation, mode, item_revision,
                     context_revision, evaluator_revision, tier, evaluated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,{now})
                 ON CONFLICT (feed_id) DO UPDATE SET
                    overall_score = excluded.overall_score,
                    explanation = excluded.explanation,
                    mode = excluded.mode,
                    item_revision = excluded.item_revision,
                    context_revision = excluded.context_revision,
                    evaluator_revision = excluded.evaluator_revision,
                    tier = excluded.tier,
                    evaluated_at = {now}
                 WHERE CASE excluded.tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END >=
                       CASE {prefix}_feed_evaluations.tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END",
                prefix = self.prefix,
                now = axon_store::NOW
            ),
            params![
                &evaluation.feed_id,
                evaluation.overall_score,
                &evaluation.explanation,
                &evaluation.mode,
                &evaluation.item_revision,
                &evaluation.context_revision,
                &evaluation.evaluator_revision,
                &tier,
            ],
        )?;
        if affected == 0 {
            return Ok(false);
        }
        transaction.execute(
            &format!(
                "DELETE FROM {}_feed_evaluation_factors WHERE feed_id = ?1",
                self.prefix
            ),
            params![&evaluation.feed_id],
        )?;
        for (position, factor) in evaluation.factors.iter().enumerate() {
            let context_json = factor
                .context
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?;
            transaction.execute(
                &format!(
                    "INSERT INTO {prefix}_feed_evaluation_factors
                        (feed_id, factor_key, label, score, weight, rationale, context_json, position)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                    prefix = self.prefix
                ),
                params![
                    &evaluation.feed_id,
                    &factor.key,
                    &factor.label,
                    factor.score,
                    factor.weight,
                    &factor.rationale,
                    &context_json,
                    position as i32,
                ],
            )?;
        }
        transaction.commit()?;
        Ok(true)
    }

    pub fn feed_evaluation(
        &self,
        feed_id: &str,
    ) -> Result<Option<FeedEvaluation>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        // `evaluated_at::text` loses its cast: the column is TEXT now.
        let evaluation = conn
            .query_row(
                &format!(
                    "SELECT overall_score, explanation, mode, item_revision,
                            context_revision, evaluator_revision, evaluated_at
                     FROM {}_feed_evaluations WHERE feed_id = ?1",
                    self.prefix
                ),
                params![&feed_id],
                |row| {
                    Ok((
                        row.get::<_, f64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, Option<String>>(6)?.unwrap_or_default(),
                    ))
                },
            )
            .optional()?;
        let Some((
            overall_score,
            explanation,
            mode,
            item_revision,
            context_revision,
            evaluator_revision,
            evaluated_at,
        )) = evaluation
        else {
            return Ok(None);
        };
        let factors = conn.query_all(
            &format!(
                "SELECT factor_key, label, score, weight, rationale, context_json
                 FROM {}_feed_evaluation_factors
                 WHERE feed_id = ?1 ORDER BY position",
                self.prefix
            ),
            params![&feed_id],
            |factor| {
                Ok(EvaluationFactor {
                    key: factor.get(0)?,
                    label: factor.get(1)?,
                    score: factor.get(2)?,
                    weight: factor.get(3)?,
                    rationale: factor.get(4)?,
                    context: factor.get::<_, Option<String>>(5)?.and_then(|value| {
                        serde_json::from_str::<EvaluationFactorContext>(&value).ok()
                    }),
                })
            },
        )?;
        Ok(Some(FeedEvaluation {
            feed_id: feed_id.to_string(),
            overall_score,
            explanation,
            mode,
            item_revision,
            context_revision,
            evaluator_revision,
            evaluated_at,
            factors,
        }))
    }

    pub fn evaluation_summary(&self) -> Result<EvaluationSummary, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        // `COUNT(*) FILTER (WHERE ...)` is supported verbatim; the `::bigint` casts
        // go because a SQLite integer is already 64-bit.
        Ok(conn.query_row(
            &format!(
                "SELECT COUNT(*),
                        COUNT(*) FILTER (WHERE mode = 'reranked'),
                        COUNT(*) FILTER (WHERE mode = 'semantic'),
                        COUNT(*) FILTER (WHERE mode = 'lexical'),
                        COUNT(*) FILTER (WHERE mode = 'unscored')
                 FROM {}_feed_evaluations",
                self.prefix
            ),
            [],
            |row| {
                Ok(EvaluationSummary {
                    evaluated: row.get(0)?,
                    reranked: row.get(1)?,
                    semantic: row.get(2)?,
                    lexical: row.get(3)?,
                    unscored: row.get(4)?,
                })
            },
        )?)
    }

    pub fn replace_travel_context_snapshot(
        &self,
        revision: &str,
        payload: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        conn.execute(
            &format!(
                "INSERT INTO {prefix}_feed_context_snapshots
                    (context_kind, revision, payload, refreshed_at)
                 VALUES ('travel',?1,?2,{now})
                 ON CONFLICT (context_kind) DO UPDATE SET
                    revision = excluded.revision,
                    payload = excluded.payload,
                    refreshed_at = {now}",
                prefix = self.prefix,
                now = axon_store::NOW
            ),
            params![&revision, &payload],
        )?;
        Ok(())
    }

    pub fn travel_context_snapshot(
        &self,
    ) -> Result<Option<TravelContextSnapshot>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        Ok(conn
            .query_row(
                &format!(
                    "SELECT revision, payload, refreshed_at
                     FROM {}_feed_context_snapshots WHERE context_kind = 'travel'",
                    self.prefix
                ),
                [],
                |row| {
                    Ok(TravelContextSnapshot {
                        revision: row.get(0)?,
                        payload: row.get(1)?,
                        refreshed_at: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    })
                },
            )
            .optional()?)
    }
}
