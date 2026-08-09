//! Persistence for subscriptions and their two append-only series.
//!
//! The schema is the model's guarantee made structural. There is no `price` column
//! on `subscriptions` and no `status` column either, because a column is a thing
//! that can be updated, and the entire point is that a price change appends. What
//! the current price *is* comes from `price_at()` over the series, not from a row
//! somebody has to remember to keep in sync.
//!
//! `total_cents` is likewise absent. A cached total is a second source of truth
//! that goes stale silently, and the series it summarises is already in memory by
//! the time anyone asks.

use postgres::{Client, Row};

use crate::analytics::{TransactionKind, TransactionRow};
use crate::import::{CandidateState, TransactionCandidate};
use crate::investment::{
    Holding, HoldingsCoverage, Quantity, ReviewedHoldingsSnapshot, ReviewedHoldingsSource,
};
use crate::obsidian::ScannedNote;
use crate::subscription::{BillingCycle, PricePoint, State, StateChange, Subscription};

type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

pub struct FinanceStore {
    pool: axon_store::Pool,
    schema: String,
}

/// A schema name reaches SQL by interpolation, because Postgres has no bind
/// parameter for an identifier. Copied deliberately from `trips`: the validation is
/// the reason interpolating it is safe, and dropping the check while keeping the
/// interpolation is how this becomes an injection.
fn validate_schema(schema: &str) -> Fallible<()> {
    let ok = !schema.is_empty()
        && schema.len() <= 63
        && schema
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if ok {
        Ok(())
    } else {
        Err(format!("invalid schema name: {schema:?}").into())
    }
}

impl FinanceStore {
    pub fn open(database_url: &str) -> Fallible<Self> {
        Self::open_in_schema(database_url, "finance")
    }

    pub fn open_in_schema(database_url: &str, schema: &str) -> Fallible<Self> {
        validate_schema(schema)?;
        let pool = axon_store::open_pool(database_url, schema, |conn| {
            Self::run_migration(conn, schema)
        })?;
        Ok(Self {
            pool,
            schema: schema.to_string(),
        })
    }

    fn conn(&self) -> Fallible<axon_store::PooledClient> {
        Ok(self.pool.get()?)
    }

    /// The cheapest statement that proves this store can reach its database, which
    /// is what the readiness surface promises rather than mere liveness (#126).
    pub fn ping(&self) -> Fallible<()> {
        let mut conn = self.conn()?;
        conn.query_one("SELECT 1", &[])?;
        Ok(())
    }

    fn run_migration(conn: &mut Client, schema: &str) -> Fallible<()> {
        conn.batch_execute(&format!(
            "
            CREATE SCHEMA IF NOT EXISTS {schema};

            CREATE TABLE IF NOT EXISTS {schema}.subscriptions (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                source_path TEXT NOT NULL UNIQUE,
                category TEXT,
                value_rating SMALLINT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            -- Append-only by intent, and by the absence of any code path that
            -- updates or deletes a row here. The (subscription, date, reason) key
            -- makes a re-import idempotent without making a genuine same-day
            -- correction impossible: a different reason is a different point.
            CREATE TABLE IF NOT EXISTS {schema}.price_points (
                id BIGSERIAL PRIMARY KEY,
                subscription_id TEXT NOT NULL
                    REFERENCES {schema}.subscriptions(id) ON DELETE CASCADE,
                valid_from TEXT NOT NULL,
                amount_cents BIGINT NOT NULL,
                currency TEXT NOT NULL DEFAULT 'EUR',
                cycle TEXT NOT NULL
                    CHECK (cycle IN ('weekly','monthly','quarterly','yearly','one_off')),
                reason TEXT NOT NULL DEFAULT '',
                recorded_at TEXT NOT NULL,
                UNIQUE (subscription_id, valid_from, reason)
            );
            -- Added after the table shipped, so it arrives as an ALTER rather than
            -- in the CREATE: a database that already holds price points must keep
            -- them. Nullable because most subscriptions have exactly one tier.
            ALTER TABLE {schema}.price_points ADD COLUMN IF NOT EXISTS plan TEXT;

            CREATE TABLE IF NOT EXISTS {schema}.state_changes (
                id BIGSERIAL PRIMARY KEY,
                subscription_id TEXT NOT NULL
                    REFERENCES {schema}.subscriptions(id) ON DELETE CASCADE,
                effective TEXT NOT NULL,
                state TEXT NOT NULL
                    CHECK (state IN ('considering','trial','active','paused','cancelled')),
                note TEXT NOT NULL DEFAULT '',
                recorded_at TEXT NOT NULL,
                UNIQUE (subscription_id, effective, state)
            );
            ALTER TABLE {schema}.state_changes
                DROP CONSTRAINT IF EXISTS state_changes_state_check;
            ALTER TABLE {schema}.state_changes
                ADD CONSTRAINT state_changes_state_check
                CHECK (state IN ('considering','trial','active','covered','paused','cancelled'));

            CREATE INDEX IF NOT EXISTS idx_price_points_sub
                ON {schema}.price_points(subscription_id, valid_from);
            CREATE INDEX IF NOT EXISTS idx_state_changes_sub
                ON {schema}.state_changes(subscription_id, effective);

            CREATE TABLE IF NOT EXISTS {schema}.transaction_candidates (
                id TEXT PRIMARY KEY,
                fingerprint TEXT NOT NULL UNIQUE,
                booked_at TEXT NOT NULL,
                description TEXT NOT NULL,
                amount_cents BIGINT NOT NULL,
                currency TEXT NOT NULL,
                source_account TEXT NOT NULL,
                source_reference TEXT,
                proposed_account TEXT NOT NULL,
                confidence_basis_points SMALLINT NOT NULL,
                state TEXT NOT NULL CHECK (state IN ('pending','confirmed','rejected','duplicate')),
                created_at TEXT NOT NULL,
                reviewed_at TEXT
            );
            ALTER TABLE {schema}.transaction_candidates
                DROP CONSTRAINT IF EXISTS transaction_candidates_state_check;
            ALTER TABLE {schema}.transaction_candidates
                ADD CONSTRAINT transaction_candidates_state_check
                CHECK (state IN ('pending','confirmed','rejected','duplicate'));
            CREATE INDEX IF NOT EXISTS idx_transaction_candidates_state
                ON {schema}.transaction_candidates(state, booked_at DESC);

            CREATE TABLE IF NOT EXISTS {schema}.transaction_projection (
                id TEXT PRIMARY KEY,
                booked_at TEXT NOT NULL,
                description TEXT NOT NULL,
                kind TEXT NOT NULL CHECK (kind IN ('income','expense','transfer')),
                account TEXT NOT NULL,
                category TEXT NOT NULL,
                amount_cents BIGINT NOT NULL CHECK (amount_cents >= 0),
                currency TEXT NOT NULL
            );
            ALTER TABLE {schema}.transaction_projection
                ADD COLUMN IF NOT EXISTS source_id TEXT;
            ALTER TABLE {schema}.transaction_projection
                ADD COLUMN IF NOT EXISTS purpose TEXT;
            ALTER TABLE {schema}.transaction_projection
                ADD COLUMN IF NOT EXISTS trip_id TEXT;
            ALTER TABLE {schema}.transaction_projection
                ADD COLUMN IF NOT EXISTS cash_amount_cents BIGINT NOT NULL DEFAULT 0
                    CHECK (cash_amount_cents >= 0);
            ALTER TABLE {schema}.transaction_projection
                ADD COLUMN IF NOT EXISTS shared_cents BIGINT NOT NULL DEFAULT 0
                    CHECK (shared_cents >= 0);
            ALTER TABLE {schema}.transaction_projection
                ADD COLUMN IF NOT EXISTS reimbursement_for TEXT;
            CREATE INDEX IF NOT EXISTS idx_transaction_projection_date
                ON {schema}.transaction_projection(booked_at DESC);

            CREATE TABLE IF NOT EXISTS {schema}.holding_projection_state (
                singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
                snapshot_id TEXT NOT NULL,
                reviewed_at TEXT NOT NULL,
                coverage TEXT NOT NULL DEFAULT 'complete'
            );
            ALTER TABLE {schema}.holding_projection_state
                ADD COLUMN IF NOT EXISTS coverage TEXT NOT NULL DEFAULT 'complete';

            CREATE TABLE IF NOT EXISTS {schema}.holding_projection (
                instrument TEXT PRIMARY KEY,
                quantity_mantissa BIGINT NOT NULL,
                quantity_scale INTEGER NOT NULL CHECK (quantity_scale BETWEEN 0 AND 12),
                price_mantissa BIGINT,
                price_scale INTEGER CHECK (price_scale BETWEEN 0 AND 12),
                currency TEXT NOT NULL,
                CHECK ((price_mantissa IS NULL) = (price_scale IS NULL))
            );

            CREATE TABLE IF NOT EXISTS {schema}.holding_projection_sources (
                source_key TEXT PRIMARY KEY,
                snapshot_id TEXT NOT NULL,
                reviewed_at TEXT NOT NULL,
                coverage TEXT NOT NULL DEFAULT 'complete'
            );
            ALTER TABLE {schema}.holding_projection_sources
                ADD COLUMN IF NOT EXISTS coverage TEXT NOT NULL DEFAULT 'complete';
            "
        ))?;
        Ok(())
    }

    /// Every subscription, each with its full series attached.
    ///
    /// Three queries rather than one join, because a join across two one-to-many
    /// series multiplies the rows and then has to be de-duplicated in memory
    /// anyway. Three ordered scans are simpler to read and simpler to be right.
    pub fn list(&self) -> Fallible<Vec<Subscription>> {
        let schema = &self.schema;
        let mut conn = self.conn()?;

        let rows = conn.query(
            &format!(
                "SELECT id, name, source_path, category, value_rating
                 FROM {schema}.subscriptions ORDER BY name"
            ),
            &[],
        )?;
        let mut subs: Vec<Subscription> = rows.iter().map(row_to_subscription).collect();

        for sub in &mut subs {
            sub.prices = conn
                .query(
                    &format!(
                        "SELECT valid_from, amount_cents, currency, cycle, plan, reason
                         FROM {schema}.price_points
                         WHERE subscription_id = $1 ORDER BY valid_from, id"
                    ),
                    &[&sub.id],
                )?
                .iter()
                .map(row_to_price)
                .collect();

            sub.states = conn
                .query(
                    &format!(
                        "SELECT effective, state, note
                         FROM {schema}.state_changes
                         WHERE subscription_id = $1 ORDER BY effective, id"
                    ),
                    &[&sub.id],
                )?
                .iter()
                .filter_map(row_to_state)
                .collect();
        }
        Ok(subs)
    }

    pub fn get(&self, id: &str) -> Fallible<Option<Subscription>> {
        Ok(self.list()?.into_iter().find(|s| s.id == id))
    }

    /// Import a scanned note, or recognise one already imported.
    ///
    /// Identity is the vault-relative path, so a second import of the same note is
    /// a no-op rather than a duplicate. Crucially it does **not** re-seed the
    /// series: the frontmatter's single cost figure was only ever a starting point,
    /// and re-applying it would silently discard every price change recorded since.
    ///
    /// Returns whether a new subscription was created.
    pub fn import_note(&self, note: &ScannedNote, today: &str) -> Fallible<(String, bool)> {
        let schema = &self.schema;
        let mut conn = self.conn()?;

        if let Some(row) = conn.query_opt(
            &format!("SELECT id FROM {schema}.subscriptions WHERE source_path = $1"),
            &[&note.source_path],
        )? {
            return Ok((row.get::<_, String>("id"), false));
        }

        let seed = crate::obsidian::seed_from_note(note, today);
        let id = format!("sub_{:016x}", fnv1a64(note.source_path.as_bytes()));

        conn.execute(
            &format!(
                "INSERT INTO {schema}.subscriptions
                    (id, name, source_path, category, value_rating, created_at, updated_at)
                 VALUES ($1,$2,$3,$4,$5,$6,$6)"
            ),
            &[
                &id,
                &seed.name,
                &seed.source_path,
                &seed.category,
                &seed.value_rating,
                &today,
            ],
        )?;

        for price in &seed.prices {
            insert_price(&mut conn, schema, &id, price, today)?;
        }
        for state in &seed.states {
            insert_state(&mut conn, schema, &id, state, today)?;
        }
        Ok((id, true))
    }

    /// Append a price point. Never updates: that is the guarantee.
    pub fn append_price(&self, id: &str, price: &PricePoint, today: &str) -> Fallible<bool> {
        let schema = self.schema.clone();
        let mut conn = self.conn()?;
        let created = insert_price(&mut conn, &schema, id, price, today)?;
        touch(&mut conn, &schema, id, today)?;
        Ok(created)
    }

    /// Append a state change. Never updates: that is the guarantee.
    pub fn append_state(&self, id: &str, change: &StateChange, today: &str) -> Fallible<bool> {
        let schema = self.schema.clone();
        let mut conn = self.conn()?;
        let created = insert_state(&mut conn, &schema, id, change, today)?;
        touch(&mut conn, &schema, id, today)?;
        Ok(created)
    }

    /// How many rows each series holds. Exists for the append-only regression test,
    /// which is otherwise reduced to trusting that no UPDATE was written.
    pub fn series_lengths(&self, id: &str) -> Fallible<(i64, i64)> {
        let schema = &self.schema;
        let mut conn = self.conn()?;
        let prices: i64 = conn
            .query_one(
                &format!("SELECT COUNT(*) FROM {schema}.price_points WHERE subscription_id = $1"),
                &[&id],
            )?
            .get(0);
        let states: i64 = conn
            .query_one(
                &format!("SELECT COUNT(*) FROM {schema}.state_changes WHERE subscription_id = $1"),
                &[&id],
            )?
            .get(0);
        Ok((prices, states))
    }

    /// Stage normalized candidates. The CSV bytes never reach this store; the
    /// fingerprint makes importing the same export again a counted no-op. A
    /// changed mapping may refresh suggestions while a candidate is still pending;
    /// reviewed candidates remain untouched.
    pub fn stage_candidates(
        &self,
        candidates: &[TransactionCandidate],
        today: &str,
    ) -> Fallible<(usize, usize)> {
        let schema = &self.schema;
        let mut conn = self.conn()?;
        let (mut created, mut existing) = (0, 0);
        for candidate in candidates {
            let confidence = i16::try_from(candidate.confidence_basis_points)?;
            let inserted = conn.execute(
                &format!(
                    "INSERT INTO {schema}.transaction_candidates
                        (id, fingerprint, booked_at, description, amount_cents, currency,
                         source_account, source_reference, proposed_account,
                         confidence_basis_points, state, created_at)
                     VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)
                     ON CONFLICT (fingerprint) DO NOTHING"
                ),
                &[
                    &candidate.id,
                    &candidate.fingerprint,
                    &candidate.booked_at,
                    &candidate.description,
                    &candidate.amount_cents,
                    &candidate.currency,
                    &candidate.source_account,
                    &candidate.source_reference,
                    &candidate.proposed_account,
                    &confidence,
                    &candidate.state.as_str(),
                    &today,
                ],
            )?;
            if inserted == 1 {
                created += 1;
            } else {
                conn.execute(
                    &format!(
                        "UPDATE {schema}.transaction_candidates
                         SET proposed_account = $2, confidence_basis_points = $3
                         WHERE fingerprint = $1 AND state = 'pending'"
                    ),
                    &[
                        &candidate.fingerprint,
                        &candidate.proposed_account,
                        &confidence,
                    ],
                )?;
                existing += 1;
            }
        }
        Ok((created, existing))
    }

    pub fn list_candidates(&self) -> Fallible<Vec<TransactionCandidate>> {
        let schema = &self.schema;
        let mut conn = self.conn()?;
        Ok(conn
            .query(
                &format!(
                    "SELECT id, fingerprint, booked_at, description, amount_cents,
                            currency, source_account, source_reference, proposed_account,
                            confidence_basis_points, state
                     FROM {schema}.transaction_candidates
                     ORDER BY booked_at DESC, id"
                ),
                &[],
            )?
            .iter()
            .filter_map(row_to_candidate)
            .collect())
    }

    pub fn candidate(&self, id: &str) -> Fallible<Option<TransactionCandidate>> {
        Ok(self
            .list_candidates()?
            .into_iter()
            .find(|candidate| candidate.id == id))
    }

    pub fn review_candidate(
        &self,
        id: &str,
        state: CandidateState,
        account: &str,
        today: &str,
    ) -> Fallible<bool> {
        let schema = &self.schema;
        let mut conn = self.conn()?;
        Ok(conn.execute(
            &format!(
                "UPDATE {schema}.transaction_candidates
                 SET state = $2, proposed_account = $3, reviewed_at = $4
                 WHERE id = $1"
            ),
            &[&id, &state.as_str(), &account, &today],
        )? == 1)
    }

    pub fn review_transfer_pair(
        &self,
        canonical_id: &str,
        duplicate_id: &str,
        canonical_account: &str,
        today: &str,
    ) -> Fallible<bool> {
        let schema = &self.schema;
        let mut conn = self.conn()?;
        let mut transaction = conn.transaction()?;
        let canonical = transaction.execute(
            &format!(
                "UPDATE {schema}.transaction_candidates
                 SET state = 'confirmed', proposed_account = $2, reviewed_at = $3
                 WHERE id = $1 AND state IN ('pending','confirmed')"
            ),
            &[&canonical_id, &canonical_account, &today],
        )?;
        let duplicate = transaction.execute(
            &format!(
                "UPDATE {schema}.transaction_candidates
                 SET state = 'duplicate', reviewed_at = $3
                 WHERE id = $1 AND id <> $2 AND state IN ('pending','duplicate')"
            ),
            &[&duplicate_id, &canonical_id, &today],
        )?;
        if canonical != 1 || duplicate != 1 {
            return Ok(false);
        }
        transaction.commit()?;
        Ok(true)
    }

    /// Replace the disposable index in one database transaction. The journal is
    /// canonical, so a half-rebuilt projection is never observable.
    pub fn replace_transaction_projection(&self, rows: &[TransactionRow]) -> Fallible<()> {
        let schema = &self.schema;
        let mut conn = self.conn()?;
        let mut transaction = conn.transaction()?;
        transaction.execute(&format!("DELETE FROM {schema}.transaction_projection"), &[])?;
        for row in rows {
            transaction.execute(
                &format!(
                    "INSERT INTO {schema}.transaction_projection
                        (id, booked_at, description, kind, account, category, amount_cents,
                         currency, source_id, purpose, trip_id, cash_amount_cents, shared_cents,
                         reimbursement_for)
                     VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)"
                ),
                &[
                    &row.id,
                    &row.date,
                    &row.description,
                    &row.kind.as_str(),
                    &row.account,
                    &row.category,
                    &row.amount_cents,
                    &row.currency,
                    &row.source_id,
                    &row.purpose.map(|purpose| purpose.as_str()),
                    &row.trip_id,
                    &row.cash_amount_cents,
                    &row.shared_cents,
                    &row.reimbursement_for,
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn transaction_projection(&self) -> Fallible<Vec<TransactionRow>> {
        let schema = &self.schema;
        let mut conn = self.conn()?;
        Ok(conn
            .query(
                &format!(
                    "SELECT id, booked_at, description, kind, account, category,
                            amount_cents, currency, source_id, purpose, trip_id,
                            cash_amount_cents, shared_cents, reimbursement_for
                     FROM {schema}.transaction_projection
                     ORDER BY booked_at DESC, id"
                ),
                &[],
            )?
            .iter()
            .filter_map(row_to_transaction)
            .collect())
    }

    /// Replace the disposable holdings index and its review marker together. The
    /// marker makes a reviewed empty portfolio distinguishable from no snapshot.
    pub fn replace_holding_projection(&self, snapshot: &ReviewedHoldingsSnapshot) -> Fallible<()> {
        let schema = &self.schema;
        let mut conn = self.conn()?;
        let mut transaction = conn.transaction()?;
        transaction.execute(&format!("DELETE FROM {schema}.holding_projection"), &[])?;
        transaction.execute(
            &format!("DELETE FROM {schema}.holding_projection_state"),
            &[],
        )?;
        transaction.execute(
            &format!("DELETE FROM {schema}.holding_projection_sources"),
            &[],
        )?;
        transaction.execute(
            &format!(
                "INSERT INTO {schema}.holding_projection_state
                    (singleton, snapshot_id, reviewed_at, coverage) VALUES (TRUE, $1, $2, $3)"
            ),
            &[
                &snapshot.snapshot_id,
                &snapshot.reviewed_at,
                &snapshot.coverage.as_str(),
            ],
        )?;
        for holding in &snapshot.holdings {
            let quantity_scale = i32::try_from(holding.quantity.scale)?;
            let (price_mantissa, price_scale) = holding
                .latest_unit_price
                .as_ref()
                .map(|price| {
                    Ok::<_, Box<dyn std::error::Error>>((
                        Some(price.mantissa),
                        Some(i32::try_from(price.scale)?),
                    ))
                })
                .transpose()?
                .unwrap_or((None, None));
            transaction.execute(
                &format!(
                    "INSERT INTO {schema}.holding_projection
                        (instrument, quantity_mantissa, quantity_scale,
                         price_mantissa, price_scale, currency)
                     VALUES ($1,$2,$3,$4,$5,$6)"
                ),
                &[
                    &holding.instrument,
                    &holding.quantity.mantissa,
                    &quantity_scale,
                    &price_mantissa,
                    &price_scale,
                    &holding.currency,
                ],
            )?;
        }
        for source in &snapshot.sources {
            transaction.execute(
                &format!(
                    "INSERT INTO {schema}.holding_projection_sources
                        (source_key, snapshot_id, reviewed_at, coverage) VALUES ($1,$2,$3,$4)"
                ),
                &[
                    &source.source_key,
                    &source.snapshot_id,
                    &source.reviewed_at,
                    &source.coverage.as_str(),
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn clear_holding_projection(&self) -> Fallible<()> {
        let schema = &self.schema;
        let mut conn = self.conn()?;
        let mut transaction = conn.transaction()?;
        transaction.execute(&format!("DELETE FROM {schema}.holding_projection"), &[])?;
        transaction.execute(
            &format!("DELETE FROM {schema}.holding_projection_state"),
            &[],
        )?;
        transaction.execute(
            &format!("DELETE FROM {schema}.holding_projection_sources"),
            &[],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn holding_projection(&self) -> Fallible<Option<ReviewedHoldingsSnapshot>> {
        let schema = &self.schema;
        let mut conn = self.conn()?;
        let Some(state) = conn.query_opt(
            &format!(
                "SELECT snapshot_id, reviewed_at, coverage
                 FROM {schema}.holding_projection_state WHERE singleton = TRUE"
            ),
            &[],
        )?
        else {
            return Ok(None);
        };
        let mut holdings = Vec::new();
        for row in conn.query(
            &format!(
                "SELECT instrument, quantity_mantissa, quantity_scale,
                        price_mantissa, price_scale, currency
                 FROM {schema}.holding_projection ORDER BY instrument"
            ),
            &[],
        )? {
            let price_mantissa: Option<i64> = row.get("price_mantissa");
            let price_scale: Option<i32> = row.get("price_scale");
            let latest_unit_price = match (price_mantissa, price_scale) {
                (Some(mantissa), Some(scale)) => Some(Quantity {
                    mantissa,
                    scale: scale.try_into()?,
                }),
                (None, None) => None,
                _ => return Err("holding projection price is incomplete".into()),
            };
            holdings.push(Holding {
                instrument: row.get("instrument"),
                quantity: Quantity {
                    mantissa: row.get("quantity_mantissa"),
                    scale: row.get::<_, i32>("quantity_scale").try_into()?,
                },
                latest_unit_price,
                currency: row.get("currency"),
            });
        }
        let sources = conn
            .query(
                &format!(
                    "SELECT source_key, snapshot_id, reviewed_at, coverage
                     FROM {schema}.holding_projection_sources ORDER BY source_key"
                ),
                &[],
            )?
            .into_iter()
            .map(|row| {
                let coverage: String = row.get("coverage");
                Ok(ReviewedHoldingsSource {
                    source_key: row.get("source_key"),
                    snapshot_id: row.get("snapshot_id"),
                    reviewed_at: row.get("reviewed_at"),
                    coverage: HoldingsCoverage::parse(&coverage)
                        .ok_or("holding projection source coverage is invalid")?,
                })
            })
            .collect::<Fallible<Vec<_>>>()?;
        let stored_coverage: String = state.get("coverage");
        let coverage = HoldingsCoverage::parse(&stored_coverage)
            .ok_or("holding projection coverage is invalid")?;
        Ok(Some(ReviewedHoldingsSnapshot {
            schema_version: if sources.is_empty() { 1 } else { 2 },
            snapshot_id: state.get("snapshot_id"),
            reviewed_at: state.get("reviewed_at"),
            coverage,
            holdings,
            sources,
        }))
    }
}

fn insert_price(
    conn: &mut axon_store::PooledClient,
    schema: &str,
    id: &str,
    price: &PricePoint,
    today: &str,
) -> Fallible<bool> {
    let inserted = conn.execute(
        &format!(
            "INSERT INTO {schema}.price_points
                (subscription_id, valid_from, amount_cents, currency, cycle, plan, reason, recorded_at)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
             ON CONFLICT (subscription_id, valid_from, reason) DO NOTHING"
        ),
        &[
            &id,
            &price.valid_from,
            &price.amount_cents,
            &price.currency,
            &cycle_str(price.cycle),
            &price.plan,
            &price.reason,
            &today,
        ],
    )?;
    Ok(inserted == 1)
}

fn insert_state(
    conn: &mut axon_store::PooledClient,
    schema: &str,
    id: &str,
    change: &StateChange,
    today: &str,
) -> Fallible<bool> {
    let inserted = conn.execute(
        &format!(
            "INSERT INTO {schema}.state_changes
                (subscription_id, effective, state, note, recorded_at)
             VALUES ($1,$2,$3,$4,$5)
             ON CONFLICT (subscription_id, effective, state) DO NOTHING"
        ),
        &[
            &id,
            &change.effective,
            &change.state.as_str(),
            &change.note,
            &today,
        ],
    )?;
    Ok(inserted == 1)
}

/// `updated_at` on the parent row is the one thing that does get updated, and it
/// describes the record rather than the money.
fn touch(conn: &mut axon_store::PooledClient, schema: &str, id: &str, today: &str) -> Fallible<()> {
    conn.execute(
        &format!("UPDATE {schema}.subscriptions SET updated_at = $2 WHERE id = $1"),
        &[&id, &today],
    )?;
    Ok(())
}

fn cycle_str(cycle: BillingCycle) -> &'static str {
    match cycle {
        BillingCycle::Weekly => "weekly",
        BillingCycle::Monthly => "monthly",
        BillingCycle::Quarterly => "quarterly",
        BillingCycle::Yearly => "yearly",
        BillingCycle::OneOff => "one_off",
    }
}

fn cycle_from_str(raw: &str) -> BillingCycle {
    match raw {
        "weekly" => BillingCycle::Weekly,
        "quarterly" => BillingCycle::Quarterly,
        "yearly" => BillingCycle::Yearly,
        "one_off" => BillingCycle::OneOff,
        _ => BillingCycle::Monthly,
    }
}

fn row_to_subscription(row: &Row) -> Subscription {
    Subscription {
        id: row.get("id"),
        name: row.get("name"),
        source_path: row.get("source_path"),
        category: row.get("category"),
        value_rating: row.get("value_rating"),
        prices: Vec::new(),
        states: Vec::new(),
    }
}

fn row_to_price(row: &Row) -> PricePoint {
    PricePoint {
        valid_from: row.get("valid_from"),
        amount_cents: row.get("amount_cents"),
        currency: row.get("currency"),
        cycle: cycle_from_str(row.get::<_, String>("cycle").as_str()),
        plan: row.get("plan"),
        reason: row.get("reason"),
    }
}

/// A state the enum no longer knows is dropped rather than guessed into the
/// nearest neighbour. The CHECK constraint makes it unreachable today; if a later
/// migration widens it, a stale binary reporting the wrong state is worse than one
/// reporting a shorter history.
fn row_to_state(row: &Row) -> Option<StateChange> {
    Some(StateChange {
        effective: row.get("effective"),
        state: State::parse(row.get::<_, String>("state").as_str())?,
        note: row.get("note"),
    })
}

fn row_to_candidate(row: &Row) -> Option<TransactionCandidate> {
    let confidence: i16 = row.get("confidence_basis_points");
    Some(TransactionCandidate {
        id: row.get("id"),
        fingerprint: row.get("fingerprint"),
        booked_at: row.get("booked_at"),
        description: row.get("description"),
        amount_cents: row.get("amount_cents"),
        currency: row.get("currency"),
        source_account: row.get("source_account"),
        source_reference: row.get("source_reference"),
        proposed_account: row.get("proposed_account"),
        confidence_basis_points: confidence.try_into().ok()?,
        state: CandidateState::parse(row.get::<_, String>("state").as_str())?,
    })
}

fn row_to_transaction(row: &Row) -> Option<TransactionRow> {
    Some(TransactionRow {
        id: row.get("id"),
        date: row.get("booked_at"),
        description: row.get("description"),
        kind: TransactionKind::parse(row.get::<_, String>("kind").as_str())?,
        account: row.get("account"),
        category: row.get("category"),
        amount_cents: row.get("amount_cents"),
        currency: row.get("currency"),
        source_id: row.get("source_id"),
        purpose: row
            .get::<_, Option<String>>("purpose")
            .as_deref()
            .and_then(crate::allocation::SpendingPurpose::parse),
        trip_id: row.get("trip_id"),
        cash_amount_cents: row.get("cash_amount_cents"),
        shared_cents: row.get("shared_cents"),
        reimbursement_for: row.get("reimbursement_for"),
    })
}

/// Stable id from the note's path, so the same note gets the same id on any
/// machine and a re-import after a database rebuild lands on the row it had.
fn fnv1a64(data: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in data {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_schema_name_that_could_carry_sql_is_refused() {
        assert!(validate_schema("finance").is_ok());
        assert!(validate_schema("finance_test_123").is_ok());
        assert!(validate_schema("finance; DROP SCHEMA public").is_err());
        assert!(validate_schema("Finance").is_err());
        assert!(validate_schema("").is_err());
    }

    #[test]
    fn cycles_round_trip_through_their_stored_spelling() {
        for cycle in [
            BillingCycle::Weekly,
            BillingCycle::Monthly,
            BillingCycle::Quarterly,
            BillingCycle::Yearly,
            BillingCycle::OneOff,
        ] {
            assert_eq!(cycle_from_str(cycle_str(cycle)), cycle);
        }
    }

    #[test]
    fn an_id_is_derived_from_the_path_so_it_survives_a_rebuild() {
        let a = fnv1a64(b"Atlas/Finance/Subscriptions/Example.md");
        let b = fnv1a64(b"Atlas/Finance/Subscriptions/Example.md");
        let c = fnv1a64(b"Atlas/Finance/Subscriptions/Other.md");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
