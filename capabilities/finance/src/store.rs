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

            CREATE INDEX IF NOT EXISTS idx_price_points_sub
                ON {schema}.price_points(subscription_id, valid_from);
            CREATE INDEX IF NOT EXISTS idx_state_changes_sub
                ON {schema}.state_changes(subscription_id, effective);
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
                        "SELECT valid_from, amount_cents, currency, cycle, reason
                         FROM {schema}.price_points
                         WHERE subscription_id = $1 ORDER BY valid_from"
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
                         WHERE subscription_id = $1 ORDER BY effective"
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
    pub fn append_price(&self, id: &str, price: &PricePoint, today: &str) -> Fallible<()> {
        let schema = self.schema.clone();
        let mut conn = self.conn()?;
        insert_price(&mut conn, &schema, id, price, today)?;
        touch(&mut conn, &schema, id, today)
    }

    /// Append a state change. Never updates: that is the guarantee.
    pub fn append_state(&self, id: &str, change: &StateChange, today: &str) -> Fallible<()> {
        let schema = self.schema.clone();
        let mut conn = self.conn()?;
        insert_state(&mut conn, &schema, id, change, today)?;
        touch(&mut conn, &schema, id, today)
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
}

fn insert_price(
    conn: &mut axon_store::PooledClient,
    schema: &str,
    id: &str,
    price: &PricePoint,
    today: &str,
) -> Fallible<()> {
    conn.execute(
        &format!(
            "INSERT INTO {schema}.price_points
                (subscription_id, valid_from, amount_cents, currency, cycle, reason, recorded_at)
             VALUES ($1,$2,$3,$4,$5,$6,$7)
             ON CONFLICT (subscription_id, valid_from, reason) DO NOTHING"
        ),
        &[
            &id,
            &price.valid_from,
            &price.amount_cents,
            &price.currency,
            &cycle_str(price.cycle),
            &price.reason,
            &today,
        ],
    )?;
    Ok(())
}

fn insert_state(
    conn: &mut axon_store::PooledClient,
    schema: &str,
    id: &str,
    change: &StateChange,
    today: &str,
) -> Fallible<()> {
    conn.execute(
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
    Ok(())
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
