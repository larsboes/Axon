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
        let mut transaction = conn.transaction()?;
        let tier = provenance::ranking_tier(&evaluation.mode);
        let affected = transaction.execute(
            &format!(
                "INSERT INTO {schema}.feed_evaluations
                    (feed_id, overall_score, explanation, mode, item_revision,
                     context_revision, evaluator_revision, tier, evaluated_at)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,now())
                 ON CONFLICT (feed_id) DO UPDATE SET
                    overall_score = excluded.overall_score,
                    explanation = excluded.explanation,
                    mode = excluded.mode,
                    item_revision = excluded.item_revision,
                    context_revision = excluded.context_revision,
                    evaluator_revision = excluded.evaluator_revision,
                    tier = excluded.tier,
                    evaluated_at = now()
                 WHERE CASE excluded.tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END >=
                       CASE feed_evaluations.tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END",
                schema = self.schema
            ),
            &[
                &evaluation.feed_id,
                &evaluation.overall_score,
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
                "DELETE FROM {}.feed_evaluation_factors WHERE feed_id = $1",
                self.schema
            ),
            &[&evaluation.feed_id],
        )?;
        for (position, factor) in evaluation.factors.iter().enumerate() {
            let context_json = factor
                .context
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?;
            transaction.execute(
                &format!(
                    "INSERT INTO {schema}.feed_evaluation_factors
                        (feed_id, factor_key, label, score, weight, rationale, context_json, position)
                     VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
                    schema = self.schema
                ),
                &[
                    &evaluation.feed_id,
                    &factor.key,
                    &factor.label,
                    &factor.score,
                    &factor.weight,
                    &factor.rationale,
                    &context_json,
                    &(position as i32),
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
        let mut conn = self.conn()?;
        let evaluation = conn.query_opt(
            &format!(
                "SELECT overall_score, explanation, mode, item_revision,
                        context_revision, evaluator_revision, evaluated_at::text
                 FROM {}.feed_evaluations WHERE feed_id = $1",
                self.schema
            ),
            &[&feed_id],
        )?;
        let Some(row) = evaluation else {
            return Ok(None);
        };
        let factor_rows = conn.query(
            &format!(
                "SELECT factor_key, label, score, weight, rationale, context_json
                 FROM {}.feed_evaluation_factors
                 WHERE feed_id = $1 ORDER BY position",
                self.schema
            ),
            &[&feed_id],
        )?;
        let factors = factor_rows
            .iter()
            .map(|factor| EvaluationFactor {
                key: factor.get(0),
                label: factor.get(1),
                score: factor.get(2),
                weight: factor.get(3),
                rationale: factor.get(4),
                context: factor
                    .get::<_, Option<String>>(5)
                    .and_then(|value| serde_json::from_str::<EvaluationFactorContext>(&value).ok()),
            })
            .collect();
        Ok(Some(FeedEvaluation {
            feed_id: feed_id.to_string(),
            overall_score: row.get(0),
            explanation: row.get(1),
            mode: row.get(2),
            item_revision: row.get(3),
            context_revision: row.get(4),
            evaluator_revision: row.get(5),
            evaluated_at: row.get::<_, Option<String>>(6).unwrap_or_default(),
            factors,
        }))
    }

    pub fn evaluation_summary(&self) -> Result<EvaluationSummary, Box<dyn std::error::Error>> {
        let mut conn = self.conn()?;
        let row = conn.query_one(
            &format!(
                "SELECT COUNT(*)::bigint,
                        COUNT(*) FILTER (WHERE mode = 'reranked')::bigint,
                        COUNT(*) FILTER (WHERE mode = 'semantic')::bigint,
                        COUNT(*) FILTER (WHERE mode = 'lexical')::bigint,
                        COUNT(*) FILTER (WHERE mode = 'unscored')::bigint
                 FROM {}.feed_evaluations",
                self.schema
            ),
            &[],
        )?;
        Ok(EvaluationSummary {
            evaluated: row.get(0),
            reranked: row.get(1),
            semantic: row.get(2),
            lexical: row.get(3),
            unscored: row.get(4),
        })
    }

    pub fn replace_travel_context_snapshot(
        &self,
        revision: &str,
        payload: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut conn = self.conn()?;
        conn.execute(
            &format!(
                "INSERT INTO {schema}.feed_context_snapshots
                    (context_kind, revision, payload, refreshed_at)
                 VALUES ('travel',$1,$2,now())
                 ON CONFLICT (context_kind) DO UPDATE SET
                    revision = excluded.revision,
                    payload = excluded.payload,
                    refreshed_at = now()",
                schema = self.schema
            ),
            &[&revision, &payload],
        )?;
        Ok(())
    }

    pub fn travel_context_snapshot(
        &self,
    ) -> Result<Option<TravelContextSnapshot>, Box<dyn std::error::Error>> {
        let mut conn = self.conn()?;
        let row = conn.query_opt(
            &format!(
                "SELECT revision, payload, refreshed_at::text
                 FROM {}.feed_context_snapshots WHERE context_kind = 'travel'",
                self.schema
            ),
            &[],
        )?;
        Ok(row.map(|row| TravelContextSnapshot {
            revision: row.get(0),
            payload: row.get(1),
            refreshed_at: row.get::<_, Option<String>>(2).unwrap_or_default(),
        }))
    }
}
