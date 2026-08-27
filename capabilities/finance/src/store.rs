//! Persistence for subscriptions and their two append-only series.
//!
//! The table shape is the model's guarantee made structural. There is no `price`
//! column on `subscriptions` and no `status` column either, because a column is a
//! thing that can be updated, and the entire point is that a price change appends.
//! What the current price *is* comes from `price_at()` over the series, not from a
//! row somebody has to remember to keep in sync.
//!
//! `total_cents` is likewise absent. A cached total is a second source of truth
//! that goes stale silently, and the series it summarises is already in memory by
//! the time anyone asks.

use std::path::Path;

use axon_store::QueryAll;
use rusqlite::{params, Connection, OptionalExtension, Row};

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
    /// Prefixes this capability's tables in the one shared file (PRD Q45):
    /// `finance` here means `finance_subscriptions` and its seven siblings.
    prefix: String,
}

/// A table prefix reaches SQL by interpolation, because SQL has no bind parameter
/// for an identifier. Copied deliberately from `trips`: the validation is the reason
/// interpolating it is safe, and dropping the check while keeping the interpolation
/// is how this becomes an injection.
fn validate_prefix(prefix: &str) -> Fallible<()> {
    let ok = !prefix.is_empty()
        && prefix.len() <= 63
        && prefix
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if ok {
        Ok(())
    } else {
        Err(format!("invalid table prefix: {prefix:?}").into())
    }
}

impl FinanceStore {
    pub fn open(database_path: &Path) -> Fallible<Self> {
        Self::open_with_prefix(database_path, "finance")
    }

    pub fn open_with_prefix(database_path: &Path, prefix: &str) -> Fallible<Self> {
        validate_prefix(prefix)?;
        let pool = axon_store::open_pool(database_path, prefix, |conn| {
            Self::run_migration(conn, prefix)
        })?;
        Ok(Self {
            pool,
            prefix: prefix.to_string(),
        })
    }

    fn conn(&self) -> Fallible<axon_store::PooledClient> {
        Ok(self.pool.get()?)
    }

    /// The cheapest statement that proves this store can reach its database, which
    /// is what the readiness surface promises rather than mere liveness (#126).
    pub fn ping(&self) -> Fallible<()> {
        let conn = self.conn()?;
        conn.query_row("SELECT 1", [], |row| row.get::<_, i64>(0))?;
        Ok(())
    }

    /// The current shape of the eight tables, not the history that produced them.
    ///
    /// Fourteen `ALTER TABLE ... ADD COLUMN IF NOT EXISTS` statements and two
    /// DROP/ADD CONSTRAINT pairs are folded into their `CREATE TABLE`s. SQLite has
    /// neither form -- no conditional ADD COLUMN, no alterable constraint -- and no
    /// deployed SQLite file predates this migration, so there is no history for the
    /// replay to describe. Each folded column keeps the nullability the ALTER gave
    /// it, because the code that reads it was written against that.
    ///
    /// `subscriptions` is declared before the two series that reference it, because
    /// a batch executes in order.
    fn run_migration(conn: &Connection, prefix: &str) -> Fallible<()> {
        conn.execute_batch(&format!(
            "
            CREATE TABLE IF NOT EXISTS {prefix}_subscriptions (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                source_path TEXT NOT NULL UNIQUE,
                category TEXT,
                value_rating INTEGER,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            -- Append-only by intent, and by the absence of any code path that
            -- updates or deletes a row here. The (subscription, date, reason) key
            -- makes a re-import idempotent without making a genuine same-day
            -- correction impossible: a different reason is a different point.
            --
            -- AUTOINCREMENT rather than the bare rowid alias BIGSERIAL would map to:
            -- `list` orders by (valid_from, id), so two points on one day keep the
            -- order they were appended in, and a plain rowid is reused after the
            -- highest row is deleted.
            --
            -- `plan` is nullable because most subscriptions have exactly one tier.
            CREATE TABLE IF NOT EXISTS {prefix}_price_points (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                subscription_id TEXT NOT NULL
                    REFERENCES {prefix}_subscriptions(id) ON DELETE CASCADE,
                valid_from TEXT NOT NULL,
                amount_cents INTEGER NOT NULL,
                currency TEXT NOT NULL DEFAULT 'EUR',
                cycle TEXT NOT NULL
                    CHECK (cycle IN ('weekly','monthly','quarterly','yearly','one_off')),
                reason TEXT NOT NULL DEFAULT '',
                recorded_at TEXT NOT NULL,
                plan TEXT,
                UNIQUE (subscription_id, valid_from, reason)
            );

            -- The CHECK is the widened one: 'covered' arrived as a DROP/ADD
            -- CONSTRAINT pair under Postgres and is simply in the list here.
            CREATE TABLE IF NOT EXISTS {prefix}_state_changes (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                subscription_id TEXT NOT NULL
                    REFERENCES {prefix}_subscriptions(id) ON DELETE CASCADE,
                effective TEXT NOT NULL,
                state TEXT NOT NULL
                    CHECK (state IN ('considering','trial','active','covered','paused','cancelled')),
                note TEXT NOT NULL DEFAULT '',
                recorded_at TEXT NOT NULL,
                UNIQUE (subscription_id, effective, state)
            );

            -- Index names carry the prefix too: one file is one namespace now.
            CREATE INDEX IF NOT EXISTS idx_{prefix}_price_points_sub
                ON {prefix}_price_points(subscription_id, valid_from);
            CREATE INDEX IF NOT EXISTS idx_{prefix}_state_changes_sub
                ON {prefix}_state_changes(subscription_id, effective);

            -- The nullable raw location columns are preserved from the export for
            -- the places capability, which reads candidates to link spend to venues
            -- (capabilities/places/README.md, D1/D2).
            CREATE TABLE IF NOT EXISTS {prefix}_transaction_candidates (
                id TEXT PRIMARY KEY,
                fingerprint TEXT NOT NULL UNIQUE,
                booked_at TEXT NOT NULL,
                description TEXT NOT NULL,
                amount_cents INTEGER NOT NULL,
                currency TEXT NOT NULL,
                source_account TEXT NOT NULL,
                source_reference TEXT,
                proposed_account TEXT NOT NULL,
                confidence_basis_points INTEGER NOT NULL,
                state TEXT NOT NULL
                    CHECK (state IN ('pending','confirmed','rejected','duplicate')),
                created_at TEXT NOT NULL,
                reviewed_at TEXT,
                location_street TEXT,
                location_postal_code TEXT,
                location_city TEXT,
                location_country TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_{prefix}_transaction_candidates_state
                ON {prefix}_transaction_candidates(state, booked_at DESC);

            CREATE TABLE IF NOT EXISTS {prefix}_transaction_projection (
                id TEXT PRIMARY KEY,
                booked_at TEXT NOT NULL,
                description TEXT NOT NULL,
                kind TEXT NOT NULL CHECK (kind IN ('income','expense','transfer')),
                account TEXT NOT NULL,
                category TEXT NOT NULL,
                amount_cents INTEGER NOT NULL CHECK (amount_cents >= 0),
                currency TEXT NOT NULL,
                source_id TEXT,
                purpose TEXT,
                trip_id TEXT,
                cash_amount_cents INTEGER NOT NULL DEFAULT 0
                    CHECK (cash_amount_cents >= 0),
                shared_cents INTEGER NOT NULL DEFAULT 0
                    CHECK (shared_cents >= 0),
                reimbursement_for TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_{prefix}_transaction_projection_date
                ON {prefix}_transaction_projection(booked_at DESC);

            -- `singleton` is INTEGER because SQLite has no boolean type; the CHECK
            -- is what keeps the table to one row, exactly as it did as a BOOLEAN.
            CREATE TABLE IF NOT EXISTS {prefix}_holding_projection_state (
                singleton INTEGER PRIMARY KEY DEFAULT 1 CHECK (singleton = 1),
                snapshot_id TEXT NOT NULL,
                reviewed_at TEXT NOT NULL,
                coverage TEXT NOT NULL DEFAULT 'complete'
            );

            CREATE TABLE IF NOT EXISTS {prefix}_holding_projection (
                instrument TEXT PRIMARY KEY,
                quantity_mantissa INTEGER NOT NULL,
                quantity_scale INTEGER NOT NULL CHECK (quantity_scale BETWEEN 0 AND 12),
                price_mantissa INTEGER,
                price_scale INTEGER CHECK (price_scale BETWEEN 0 AND 12),
                currency TEXT NOT NULL,
                CHECK ((price_mantissa IS NULL) = (price_scale IS NULL))
            );

            CREATE TABLE IF NOT EXISTS {prefix}_holding_projection_sources (
                source_key TEXT PRIMARY KEY,
                snapshot_id TEXT NOT NULL,
                reviewed_at TEXT NOT NULL,
                coverage TEXT NOT NULL DEFAULT 'complete'
            );
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
        let prefix = &self.prefix;
        let conn = self.conn()?;

        let mut subs: Vec<Subscription> = conn.query_all(
            &format!(
                "SELECT id, name, source_path, category, value_rating
                 FROM {prefix}_subscriptions ORDER BY name"
            ),
            [],
            row_to_subscription,
        )?;

        for sub in &mut subs {
            sub.prices = conn.query_all(
                &format!(
                    "SELECT valid_from, amount_cents, currency, cycle, plan, reason
                     FROM {prefix}_price_points
                     WHERE subscription_id = ?1 ORDER BY valid_from, id"
                ),
                params![&sub.id],
                row_to_price,
            )?;

            sub.states = conn
                .query_all(
                    &format!(
                        "SELECT effective, state, note
                         FROM {prefix}_state_changes
                         WHERE subscription_id = ?1 ORDER BY effective, id"
                    ),
                    params![&sub.id],
                    row_to_state,
                )?
                .into_iter()
                .flatten()
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
        let prefix = &self.prefix;
        let conn = self.conn()?;

        if let Some(existing) = conn
            .query_row(
                &format!("SELECT id FROM {prefix}_subscriptions WHERE source_path = ?1"),
                params![&note.source_path],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        {
            return Ok((existing, false));
        }

        let seed = crate::obsidian::seed_from_note(note, today);
        let id = format!("sub_{:016x}", fnv1a64(note.source_path.as_bytes()));

        conn.execute(
            &format!(
                "INSERT INTO {prefix}_subscriptions
                    (id, name, source_path, category, value_rating, created_at, updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?6)"
            ),
            params![
                &id,
                &seed.name,
                &seed.source_path,
                &seed.category,
                &seed.value_rating,
                &today,
            ],
        )?;

        for price in &seed.prices {
            insert_price(&conn, prefix, &id, price, today)?;
        }
        for state in &seed.states {
            insert_state(&conn, prefix, &id, state, today)?;
        }
        Ok((id, true))
    }

    /// Append a price point. Never updates: that is the guarantee.
    pub fn append_price(&self, id: &str, price: &PricePoint, today: &str) -> Fallible<bool> {
        let conn = self.conn()?;
        let created = insert_price(&conn, &self.prefix, id, price, today)?;
        touch(&conn, &self.prefix, id, today)?;
        Ok(created)
    }

    /// Append a state change. Never updates: that is the guarantee.
    pub fn append_state(&self, id: &str, change: &StateChange, today: &str) -> Fallible<bool> {
        let conn = self.conn()?;
        let created = insert_state(&conn, &self.prefix, id, change, today)?;
        touch(&conn, &self.prefix, id, today)?;
        Ok(created)
    }

    /// How many rows each series holds. Exists for the append-only regression test,
    /// which is otherwise reduced to trusting that no UPDATE was written.
    pub fn series_lengths(&self, id: &str) -> Fallible<(i64, i64)> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        let prices: i64 = conn.query_row(
            &format!("SELECT COUNT(*) FROM {prefix}_price_points WHERE subscription_id = ?1"),
            params![&id],
            |row| row.get(0),
        )?;
        let states: i64 = conn.query_row(
            &format!("SELECT COUNT(*) FROM {prefix}_state_changes WHERE subscription_id = ?1"),
            params![&id],
            |row| row.get(0),
        )?;
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
        let prefix = &self.prefix;
        let conn = self.conn()?;
        let (mut created, mut existing) = (0, 0);
        for candidate in candidates {
            let confidence = i16::try_from(candidate.confidence_basis_points)?;
            let inserted = conn.execute(
                &format!(
                    "INSERT INTO {prefix}_transaction_candidates
                        (id, fingerprint, booked_at, description, amount_cents, currency,
                         source_account, source_reference, proposed_account,
                         confidence_basis_points, state, created_at,
                         location_street, location_postal_code, location_city,
                         location_country)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)
                     ON CONFLICT (fingerprint) DO NOTHING"
                ),
                params![
                    &candidate.id,
                    &candidate.fingerprint,
                    &candidate.booked_at,
                    &candidate.description,
                    candidate.amount_cents,
                    &candidate.currency,
                    &candidate.source_account,
                    &candidate.source_reference,
                    &candidate.proposed_account,
                    confidence,
                    candidate.state.as_str(),
                    &today,
                    &candidate.location_street,
                    &candidate.location_postal_code,
                    &candidate.location_city,
                    &candidate.location_country,
                ],
            )?;
            if inserted == 1 {
                created += 1;
            } else {
                // COALESCE, not plain assignment: a re-import through a mapping
                // that gained location columns fills them in, while one through
                // a mapping without them never erases what an earlier import
                // captured.
                conn.execute(
                    &format!(
                        "UPDATE {prefix}_transaction_candidates
                         SET proposed_account = ?2, confidence_basis_points = ?3,
                             location_street = COALESCE(?4, location_street),
                             location_postal_code = COALESCE(?5, location_postal_code),
                             location_city = COALESCE(?6, location_city),
                             location_country = COALESCE(?7, location_country)
                         WHERE fingerprint = ?1 AND state = 'pending'"
                    ),
                    params![
                        &candidate.fingerprint,
                        &candidate.proposed_account,
                        confidence,
                        &candidate.location_street,
                        &candidate.location_postal_code,
                        &candidate.location_city,
                        &candidate.location_country,
                    ],
                )?;
                existing += 1;
            }
        }
        Ok((created, existing))
    }

    pub fn list_candidates(&self) -> Fallible<Vec<TransactionCandidate>> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        Ok(conn
            .query_all(
                &format!(
                    "SELECT id, fingerprint, booked_at, description, amount_cents,
                            currency, source_account, source_reference, proposed_account,
                            confidence_basis_points, state, location_street,
                            location_postal_code, location_city, location_country
                     FROM {prefix}_transaction_candidates
                     ORDER BY booked_at DESC, id"
                ),
                [],
                row_to_candidate,
            )?
            .into_iter()
            .flatten()
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
        let prefix = &self.prefix;
        let conn = self.conn()?;
        Ok(conn.execute(
            &format!(
                "UPDATE {prefix}_transaction_candidates
                 SET state = ?2, proposed_account = ?3, reviewed_at = ?4
                 WHERE id = ?1"
            ),
            params![&id, state.as_str(), &account, &today],
        )? == 1)
    }

    pub fn review_transfer_pair(
        &self,
        canonical_id: &str,
        duplicate_id: &str,
        canonical_account: &str,
        today: &str,
    ) -> Fallible<bool> {
        let prefix = &self.prefix;
        let mut conn = self.conn()?;
        let transaction = conn.transaction()?;
        let canonical = transaction.execute(
            &format!(
                "UPDATE {prefix}_transaction_candidates
                 SET state = 'confirmed', proposed_account = ?2, reviewed_at = ?3
                 WHERE id = ?1 AND state IN ('pending','confirmed')"
            ),
            params![&canonical_id, &canonical_account, &today],
        )?;
        let duplicate = transaction.execute(
            &format!(
                "UPDATE {prefix}_transaction_candidates
                 SET state = 'duplicate', reviewed_at = ?3
                 WHERE id = ?1 AND id <> ?2 AND state IN ('pending','duplicate')"
            ),
            params![&duplicate_id, &canonical_id, &today],
        )?;
        if canonical != 1 || duplicate != 1 {
            // Dropped without committing, so `Transaction`'s own Drop rolls back:
            // half a transfer pair is not a state this table may be left in.
            return Ok(false);
        }
        transaction.commit()?;
        Ok(true)
    }

    /// Replace the disposable index in one database transaction. The journal is
    /// canonical, so a half-rebuilt projection is never observable.
    pub fn replace_transaction_projection(&self, rows: &[TransactionRow]) -> Fallible<()> {
        let prefix = &self.prefix;
        let mut conn = self.conn()?;
        let transaction = conn.transaction()?;
        transaction.execute(&format!("DELETE FROM {prefix}_transaction_projection"), [])?;
        {
            let mut insert = transaction.prepare(&format!(
                "INSERT INTO {prefix}_transaction_projection
                    (id, booked_at, description, kind, account, category, amount_cents,
                     currency, source_id, purpose, trip_id, cash_amount_cents, shared_cents,
                     reimbursement_for)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)"
            ))?;
            for row in rows {
                insert.execute(params![
                    &row.id,
                    &row.date,
                    &row.description,
                    row.kind.as_str(),
                    &row.account,
                    &row.category,
                    row.amount_cents,
                    &row.currency,
                    &row.source_id,
                    &row.purpose.map(|purpose| purpose.as_str()),
                    &row.trip_id,
                    row.cash_amount_cents,
                    row.shared_cents,
                    &row.reimbursement_for,
                ])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn transaction_projection(&self) -> Fallible<Vec<TransactionRow>> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        Ok(conn
            .query_all(
                &format!(
                    "SELECT id, booked_at, description, kind, account, category,
                            amount_cents, currency, source_id, purpose, trip_id,
                            cash_amount_cents, shared_cents, reimbursement_for
                     FROM {prefix}_transaction_projection
                     ORDER BY booked_at DESC, id"
                ),
                [],
                row_to_transaction,
            )?
            .into_iter()
            .flatten()
            .collect())
    }

    /// Replace the disposable holdings index and its review marker together. The
    /// marker makes a reviewed empty portfolio distinguishable from no snapshot.
    pub fn replace_holding_projection(&self, snapshot: &ReviewedHoldingsSnapshot) -> Fallible<()> {
        let prefix = &self.prefix;
        let mut conn = self.conn()?;
        let transaction = conn.transaction()?;
        transaction.execute(&format!("DELETE FROM {prefix}_holding_projection"), [])?;
        transaction.execute(
            &format!("DELETE FROM {prefix}_holding_projection_state"),
            [],
        )?;
        transaction.execute(
            &format!("DELETE FROM {prefix}_holding_projection_sources"),
            [],
        )?;
        transaction.execute(
            &format!(
                "INSERT INTO {prefix}_holding_projection_state
                    (singleton, snapshot_id, reviewed_at, coverage) VALUES (1, ?1, ?2, ?3)"
            ),
            params![
                &snapshot.snapshot_id,
                &snapshot.reviewed_at,
                snapshot.coverage.as_str(),
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
                    "INSERT INTO {prefix}_holding_projection
                        (instrument, quantity_mantissa, quantity_scale,
                         price_mantissa, price_scale, currency)
                     VALUES (?1,?2,?3,?4,?5,?6)"
                ),
                params![
                    &holding.instrument,
                    holding.quantity.mantissa,
                    quantity_scale,
                    price_mantissa,
                    price_scale,
                    &holding.currency,
                ],
            )?;
        }
        for source in &snapshot.sources {
            transaction.execute(
                &format!(
                    "INSERT INTO {prefix}_holding_projection_sources
                        (source_key, snapshot_id, reviewed_at, coverage) VALUES (?1,?2,?3,?4)"
                ),
                params![
                    &source.source_key,
                    &source.snapshot_id,
                    &source.reviewed_at,
                    source.coverage.as_str(),
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn clear_holding_projection(&self) -> Fallible<()> {
        let prefix = &self.prefix;
        let mut conn = self.conn()?;
        let transaction = conn.transaction()?;
        transaction.execute(&format!("DELETE FROM {prefix}_holding_projection"), [])?;
        transaction.execute(
            &format!("DELETE FROM {prefix}_holding_projection_state"),
            [],
        )?;
        transaction.execute(
            &format!("DELETE FROM {prefix}_holding_projection_sources"),
            [],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn holding_projection(&self) -> Fallible<Option<ReviewedHoldingsSnapshot>> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        let Some((snapshot_id, reviewed_at, stored_coverage)) = conn
            .query_row(
                &format!(
                    "SELECT snapshot_id, reviewed_at, coverage
                     FROM {prefix}_holding_projection_state WHERE singleton = 1"
                ),
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?
        else {
            return Ok(None);
        };
        let mut holdings = Vec::new();
        for (
            instrument,
            quantity_mantissa,
            quantity_scale,
            price_mantissa,
            price_scale,
            currency,
        ) in conn.query_all(
            &format!(
                "SELECT instrument, quantity_mantissa, quantity_scale,
                            price_mantissa, price_scale, currency
                     FROM {prefix}_holding_projection ORDER BY instrument"
            ),
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i32>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, Option<i32>>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
        )? {
            let latest_unit_price = match (price_mantissa, price_scale) {
                (Some(mantissa), Some(scale)) => Some(Quantity {
                    mantissa,
                    scale: scale.try_into()?,
                }),
                (None, None) => None,
                _ => return Err("holding projection price is incomplete".into()),
            };
            holdings.push(Holding {
                instrument,
                quantity: Quantity {
                    mantissa: quantity_mantissa,
                    scale: quantity_scale.try_into()?,
                },
                latest_unit_price,
                currency,
            });
        }
        let sources = conn
            .query_all(
                &format!(
                    "SELECT source_key, snapshot_id, reviewed_at, coverage
                     FROM {prefix}_holding_projection_sources ORDER BY source_key"
                ),
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )?
            .into_iter()
            .map(|(source_key, snapshot_id, reviewed_at, coverage)| {
                Ok(ReviewedHoldingsSource {
                    source_key,
                    snapshot_id,
                    reviewed_at,
                    coverage: HoldingsCoverage::parse(&coverage)
                        .ok_or("holding projection source coverage is invalid")?,
                })
            })
            .collect::<Fallible<Vec<_>>>()?;
        let coverage = HoldingsCoverage::parse(&stored_coverage)
            .ok_or("holding projection coverage is invalid")?;
        Ok(Some(ReviewedHoldingsSnapshot {
            schema_version: if sources.is_empty() { 1 } else { 2 },
            snapshot_id,
            reviewed_at,
            coverage,
            holdings,
            sources,
        }))
    }
}

fn insert_price(
    conn: &Connection,
    prefix: &str,
    id: &str,
    price: &PricePoint,
    today: &str,
) -> Fallible<bool> {
    let inserted = conn.execute(
        &format!(
            "INSERT INTO {prefix}_price_points
                (subscription_id, valid_from, amount_cents, currency, cycle, plan, reason, recorded_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
             ON CONFLICT (subscription_id, valid_from, reason) DO NOTHING"
        ),
        params![
            &id,
            &price.valid_from,
            price.amount_cents,
            &price.currency,
            cycle_str(price.cycle),
            &price.plan,
            &price.reason,
            &today,
        ],
    )?;
    Ok(inserted == 1)
}

fn insert_state(
    conn: &Connection,
    prefix: &str,
    id: &str,
    change: &StateChange,
    today: &str,
) -> Fallible<bool> {
    let inserted = conn.execute(
        &format!(
            "INSERT INTO {prefix}_state_changes
                (subscription_id, effective, state, note, recorded_at)
             VALUES (?1,?2,?3,?4,?5)
             ON CONFLICT (subscription_id, effective, state) DO NOTHING"
        ),
        params![
            &id,
            &change.effective,
            change.state.as_str(),
            &change.note,
            &today,
        ],
    )?;
    Ok(inserted == 1)
}

/// `updated_at` on the parent row is the one thing that does get updated, and it
/// describes the record rather than the money.
fn touch(conn: &Connection, prefix: &str, id: &str, today: &str) -> Fallible<()> {
    conn.execute(
        &format!("UPDATE {prefix}_subscriptions SET updated_at = ?2 WHERE id = ?1"),
        params![&id, &today],
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

fn row_to_subscription(row: &Row) -> rusqlite::Result<Subscription> {
    Ok(Subscription {
        id: row.get("id")?,
        name: row.get("name")?,
        source_path: row.get("source_path")?,
        category: row.get("category")?,
        value_rating: row.get("value_rating")?,
        prices: Vec::new(),
        states: Vec::new(),
    })
}

fn row_to_price(row: &Row) -> rusqlite::Result<PricePoint> {
    Ok(PricePoint {
        valid_from: row.get("valid_from")?,
        amount_cents: row.get("amount_cents")?,
        currency: row.get("currency")?,
        cycle: cycle_from_str(row.get::<_, String>("cycle")?.as_str()),
        plan: row.get("plan")?,
        reason: row.get("reason")?,
    })
}

/// A state the enum no longer knows is dropped rather than guessed into the
/// nearest neighbour. The CHECK constraint makes it unreachable today; if a later
/// migration widens it, a stale binary reporting the wrong state is worse than one
/// reporting a shorter history.
fn row_to_state(row: &Row) -> rusqlite::Result<Option<StateChange>> {
    let Some(state) = State::parse(row.get::<_, String>("state")?.as_str()) else {
        return Ok(None);
    };
    Ok(Some(StateChange {
        effective: row.get("effective")?,
        state,
        note: row.get("note")?,
    }))
}

/// `Ok(None)` for a row this binary cannot represent, which the caller drops.
/// Distinct from `Err`: a column that will not convert is a broken read, while a
/// state string the enum does not know is a row from a newer writer.
fn row_to_candidate(row: &Row) -> rusqlite::Result<Option<TransactionCandidate>> {
    let confidence: i16 = row.get("confidence_basis_points")?;
    let (Ok(confidence_basis_points), Some(state)) = (
        confidence.try_into(),
        CandidateState::parse(row.get::<_, String>("state")?.as_str()),
    ) else {
        return Ok(None);
    };
    Ok(Some(TransactionCandidate {
        id: row.get("id")?,
        fingerprint: row.get("fingerprint")?,
        booked_at: row.get("booked_at")?,
        description: row.get("description")?,
        amount_cents: row.get("amount_cents")?,
        currency: row.get("currency")?,
        source_account: row.get("source_account")?,
        source_reference: row.get("source_reference")?,
        proposed_account: row.get("proposed_account")?,
        confidence_basis_points,
        state,
        location_street: row.get("location_street")?,
        location_postal_code: row.get("location_postal_code")?,
        location_city: row.get("location_city")?,
        location_country: row.get("location_country")?,
    }))
}

fn row_to_transaction(row: &Row) -> rusqlite::Result<Option<TransactionRow>> {
    let Some(kind) = TransactionKind::parse(row.get::<_, String>("kind")?.as_str()) else {
        return Ok(None);
    };
    Ok(Some(TransactionRow {
        id: row.get("id")?,
        date: row.get("booked_at")?,
        description: row.get("description")?,
        kind,
        account: row.get("account")?,
        category: row.get("category")?,
        amount_cents: row.get("amount_cents")?,
        currency: row.get("currency")?,
        source_id: row.get("source_id")?,
        purpose: row
            .get::<_, Option<String>>("purpose")?
            .as_deref()
            .and_then(crate::allocation::SpendingPurpose::parse),
        trip_id: row.get("trip_id")?,
        cash_amount_cents: row.get("cash_amount_cents")?,
        shared_cents: row.get("shared_cents")?,
        reimbursement_for: row.get("reimbursement_for")?,
    }))
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
    fn a_table_prefix_that_could_carry_sql_is_refused() {
        assert!(validate_prefix("finance").is_ok());
        assert!(validate_prefix("finance_test_123").is_ok());
        assert!(validate_prefix("finance; DROP TABLE finance_subscriptions").is_err());
        assert!(validate_prefix("Finance").is_err());
        assert!(validate_prefix("").is_err());
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
