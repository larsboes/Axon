//! Postgres-backed persistence for `transit.trips`/`transit.trip_legs` --
//! the "own trips/trip_legs tables" half of Phase 2 (defined in
//! `capabilities/postgres/README.md`) --
//! plus `transit.trip_sessions` (Phase 3: fuzzy/triggered trip-search
//! sessions). Same shared local instance as `capabilities/scouting::store`
//! (own schema, same sync `postgres` client, same schema-per-capability
//! convention -- see `capabilities/postgres/README.md`). A recorded journey
//! lands in two places on an "auto" (adapter-driven) run: here, as the
//! detailed structured record; and in `scouting.opportunities` (via
//! `scouting::adapters::transit_fare`), as the scored/dismissable view. A
//! "manual" (`transit search`/`split` CLI) run only writes here -- there's
//! no adapter/pipeline involved in a one-shot CLI query. A "session" run
//! (`transit plan`) writes here AND into a `trip_sessions` row it owns --
//! different code path from the background scan (correlation driving query
//! #2 vs #1), same underlying store.
//!
//! Phase 3 settled two deliberately-deferred column questions:
//! `candidate_destinations` and
//! `date_window` are now real columns, but on the *session* row
//! (`trip_sessions.candidates` / `date_start`/`date_end`), not on `trips` --
//! a trip is one concrete journey, the candidate set and the date window
//! describe the user-intent session that found it, not the journey itself.
//! `trips` gains a nullable `session_id` FK so a session-scoped journey is
//! traceable back to the session that surfaced it, but a manual/auto trip
//! keeps `session_id = NULL` (the prior shape unchanged). `trips.status`
//! still exists (from Phase 2's schema sketch); nothing reads/sets it
//! back yet -- same honest "scaffolding only" gap
//! `scouting::store`'s `source_state.cursor` already carries.

use crate::travel::Journey;
use postgres::Client;

pub struct TransitStore {
    /// Shared with every other store in this process on the same database, so
    /// opening one is a checkout rather than a connect.
    pool: axon_store::Pool,
    schema: String,
}

impl TransitStore {
    pub fn open(database_url: &str) -> Result<Self, Box<dyn std::error::Error>> {
        Self::open_with_schema(database_url, "transit")
    }

    /// `schema` is always either the literal `"transit"` (production, via
    /// `open()`) or a test-generated name (see `tests`) -- never user input.
    /// See `scouting::store`'s identical note for why `format!`-built DDL is
    /// safe specifically in that narrow case.
    fn open_with_schema(database_url: &str, schema: &str) -> Result<Self, Box<dyn std::error::Error>> {
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

    /// A connection from the shared pool, for the duration of one statement.
    ///
    /// A `Result` where this used to be `self.conn.lock().unwrap()`: that unwrap
    /// could only fail on a poisoned mutex, whereas a checkout can genuinely fail
    /// when the database is down or every connection is busy.
    fn conn(&self) -> Result<axon_store::PooledClient, Box<dyn std::error::Error>> {
        Ok(self.pool.get()?)
    }

    fn init_schema(client: &mut Client, schema: &str) -> Result<(), Box<dyn std::error::Error>> {
        // Phase 2's `CREATE TABLE` baked the `trigger_reason` CHECK inline
        // (`CHECK (trigger_reason IN ('manual','auto'))`), auto-named
        // `trips_trigger_reason_check` by Postgres. Phase 3 adds `'session'`
        // to that set. `CREATE TABLE IF NOT EXISTS` won't touch an existing
        // table's constraints, so we migrate with a targeted DROP + re-ADD
        // of that named constraint (see the inline comment below).
        client.batch_execute(&format!(
            "
            CREATE SCHEMA IF NOT EXISTS {schema};

            CREATE TABLE IF NOT EXISTS {schema}.trips (
                id TEXT PRIMARY KEY,
                status TEXT NOT NULL DEFAULT 'new' CHECK (status IN ('new','dismissed','saved')),
                origin_eva TEXT NOT NULL,
                destination_eva TEXT NOT NULL,
                trigger_reason TEXT NOT NULL CHECK (trigger_reason IN ('manual','auto')),
                total_duration_minutes INTEGER,
                total_price DOUBLE PRECISION,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS {schema}.trip_legs (
                trip_id TEXT NOT NULL REFERENCES {schema}.trips(id) ON DELETE CASCADE,
                leg_index INTEGER NOT NULL,
                origin_eva TEXT NOT NULL,
                origin_name TEXT NOT NULL,
                destination_eva TEXT NOT NULL,
                destination_name TEXT NOT NULL,
                departure_time TEXT NOT NULL,
                arrival_time TEXT NOT NULL,
                train_name TEXT NOT NULL,
                train_number TEXT NOT NULL,
                train_category TEXT NOT NULL,
                platform TEXT,
                is_regional BOOLEAN NOT NULL,
                PRIMARY KEY (trip_id, leg_index)
            );

            -- Phase 3: the fuzzy-trip-search session itself. Created before
            -- the trips.session_id FK column so the reference resolves.
            CREATE TABLE IF NOT EXISTS {schema}.trip_sessions (
                id TEXT PRIMARY KEY,
                origin_eva TEXT NOT NULL,
                intent TEXT NOT NULL,
                candidates TEXT NOT NULL,
                date_start TEXT NOT NULL,
                date_end TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'new' CHECK (status IN ('new','dismissed','saved')),
                created_at TEXT NOT NULL
            );

            -- Phase 3: session-scoped journey ownership. Nullable so
            -- existing manual/auto trips keep their shape unchanged.
            ALTER TABLE {schema}.trips
                ADD COLUMN IF NOT EXISTS session_id TEXT REFERENCES {schema}.trip_sessions(id) ON DELETE SET NULL;

            -- Phase 3: migrate the trigger_reason CHECK to allow 'session'. Phase 2's
            -- inline `CHECK (trigger_reason IN ('manual','auto'))` was auto-
            -- named `trips_trigger_reason_check` by Postgres (the deterministic
            -- `<table>_<column>_check` convention), so a targeted DROP +
            -- re-ADD is sufficient -- no catalog-walking DO block needed.
            -- Idempotent: on a fresh install there's nothing to drop, the IF
            -- EXISTS no-ops, and the named constraint is added once. No
            -- existing row violates: Phase 2 only ever wrote manual/auto.
            ALTER TABLE {schema}.trips DROP CONSTRAINT IF EXISTS trips_trigger_reason_check;
            ALTER TABLE {schema}.trips
                ADD CONSTRAINT trips_trigger_reason_check
                CHECK (trigger_reason IN ('manual','auto','session'));

            CREATE INDEX IF NOT EXISTS idx_trips_status ON {schema}.trips(status);
            CREATE INDEX IF NOT EXISTS idx_trips_session ON {schema}.trips(session_id);
            "
        ))?;
        Ok(())
    }

    pub const VALID_TRIGGER_REASONS: [&'static str; 3] = ["manual", "auto", "session"];

    /// Records a found journey: upserts one `trips` row (keyed on the
    /// journey's own HAFAS-assigned id, same trust-the-source-id pattern
    /// `scouting::store` uses for `Opportunity.id`) and replaces its
    /// `trip_legs` wholesale inside one transaction -- a re-recorded journey
    /// (same id, re-run search) gets a clean, consistent leg set rather than
    /// a delete/insert race outside a transaction. Returns `Ok(true)` if
    /// this was a new trip id, `Ok(false)` if it updated an existing one.
    /// `session_id` is `None` for a manual/auto trip (Phase 2 shape); a
    /// session-scoped recording passes `Some(id)`.
    pub fn record_journey(
        &self,
        journey: &Journey,
        origin_eva: &str,
        destination_eva: &str,
        trigger_reason: &str,
        session_id: Option<&str>,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        if !Self::VALID_TRIGGER_REASONS.contains(&trigger_reason) {
            return Err(format!(
                "invalid trigger_reason '{trigger_reason}' -- must be one of: {}",
                Self::VALID_TRIGGER_REASONS.join(", ")
            )
            .into());
        }
        let mut conn = self.conn()?;
        let mut tx = conn.transaction()?;

        let existing = tx.query_opt(
            &format!("SELECT id FROM {}.trips WHERE id = $1", self.schema),
            &[&journey.id],
        )?;
        let is_new = existing.is_none();

        // `session_id` is only carried in the INSERT's value list -- the
        // ON CONFLICT branch deliberately does NOT touch it. A journey re-
        // found by a *different* session would otherwise retroactively
        // re-tag a prior session's trip; we record the destination/
        // duration/price refresh but leave ownership stable. Same
        // "don't clobber a prior decision on re-fetch" principle as
        // `scouting::store`'s status-preserving `ON CONFLICT`.
        tx.execute(
            &format!(
                "INSERT INTO {schema}.trips (id, origin_eva, destination_eva, trigger_reason,
                    total_duration_minutes, total_price, created_at, session_id)
                VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
                ON CONFLICT (id) DO UPDATE SET
                    origin_eva = excluded.origin_eva,
                    destination_eva = excluded.destination_eva,
                    trigger_reason = excluded.trigger_reason,
                    total_duration_minutes = excluded.total_duration_minutes,
                    total_price = excluded.total_price",
                schema = self.schema
            ),
            &[
                &journey.id,
                &origin_eva,
                &destination_eva,
                &trigger_reason,
                &(journey.total_duration_minutes as i32),
                &journey.total_price,
                &chrono_now(),
                &session_id,
            ],
        )?;

        insert_legs(&mut tx, &self.schema, &journey.id, &journey.legs)?;

        tx.commit()?;
        Ok(is_new)
    }

    /// Reads a trip back with its legs, in leg order -- verification/test
    /// helper; no CLI command consumes this yet (see module doc).
    pub fn get_trip(&self, id: &str) -> Result<Option<(TripRow, Vec<TripLegRow>)>, Box<dyn std::error::Error>> {
        let mut conn = self.conn()?;
        let trip_row = conn.query_opt(
            &format!(
                "SELECT id, status, origin_eva, destination_eva, trigger_reason,
                    total_duration_minutes, total_price, created_at, session_id
                 FROM {}.trips WHERE id = $1",
                self.schema
            ),
            &[&id],
        )?;
        let Some(t) = trip_row else { return Ok(None) };
        let trip = TripRow {
            id: t.try_get(0)?,
            status: t.try_get(1)?,
            origin_eva: t.try_get(2)?,
            destination_eva: t.try_get(3)?,
            trigger_reason: t.try_get(4)?,
            total_duration_minutes: t.try_get::<_, Option<i32>>(5)?.map(|n| n as u32),
            total_price: t.try_get(6)?,
            created_at: t.try_get(7)?,
            session_id: t.try_get::<_, Option<String>>(8)?,
        };

        let leg_rows = conn.query(
            &format!(
                "SELECT origin_eva, origin_name, destination_eva, destination_name,
                    departure_time, arrival_time, train_name, train_number, train_category,
                    platform, is_regional
                 FROM {}.trip_legs WHERE trip_id = $1 ORDER BY leg_index",
                self.schema
            ),
            &[&id],
        )?;
        let mut legs = Vec::new();
        for r in leg_rows {
            legs.push(TripLegRow {
                origin_eva: r.try_get(0)?,
                origin_name: r.try_get(1)?,
                destination_eva: r.try_get(2)?,
                destination_name: r.try_get(3)?,
                departure_time: r.try_get(4)?,
                arrival_time: r.try_get(5)?,
                train_name: r.try_get(6)?,
                train_number: r.try_get(7)?,
                train_category: r.try_get(8)?,
                platform: r.try_get(9)?,
                is_regional: r.try_get(10)?,
            });
        }
        Ok(Some((trip, legs)))
    }

    pub fn count(&self) -> Result<i64, Box<dyn std::error::Error>> {
        let mut conn = self.conn()?;
        let row = conn.query_one(&format!("SELECT COUNT(*) FROM {}.trips", self.schema), &[])?;
        Ok(row.try_get(0)?)
    }

    // ── Phase 3: fuzzy/triggered trip-search sessions ───────────────────
    //
    // A session owns one user-intent ("Valencia or Copenhagen, in September,
    // open"), its resolved candidate destination set, and a date window. The
    // `transit plan` CLI builds one, fans `search_connections` out across
    // (candidate x sampled date) and records every found journey into the
    // existing `trips`/`trip_legs` tables tagged `trigger_reason = "session"`
    // with `session_id` set back to this row. Same store, different query
    // path than the background scan -- see `capabilities/postgres/README.md`
    // driving query #2 and the module doc above.

    /// Upserts a session row keyed on `id`. `id` is expected to be the
    /// stable id from `stable_session_id` (origin + sorted candidates +
    /// date window + intent hash) so re-running the *same* plan updates
    /// the same session with fresh fare data instead of creating a new
    /// one every time -- the entire point of "triggered" vs "one-shot".
    /// Returns `Ok(true)` if this was a new session.
    pub fn upsert_session(
        &self,
        id: &str,
        origin_eva: &str,
        intent: &str,
        candidates: &[CandidateDest],
        date_start: &str,
        date_end: &str,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let cands_json = serde_json::to_string(candidates)?;
        let mut conn = self.conn()?;
        let existing = conn.query_opt(
            &format!("SELECT id FROM {}.trip_sessions WHERE id = $1", self.schema),
            &[&id],
        )?;
        let is_new = existing.is_none();
        conn.execute(
            &format!(
                "INSERT INTO {schema}.trip_sessions
                    (id, origin_eva, intent, candidates, date_start, date_end, created_at)
                 VALUES ($1,$2,$3,$4,$5,$6,$7)
                 ON CONFLICT (id) DO UPDATE SET
                    intent = excluded.intent,
                    candidates = excluded.candidates,
                    date_start = excluded.date_start,
                    date_end = excluded.date_end",
                schema = self.schema
            ),
            &[
                &id,
                &origin_eva,
                &intent,
                &cands_json,
                &date_start,
                &date_end,
                &chrono_now(),
            ],
        )?;
        Ok(is_new)
    }

    /// Reads a session back. `candidates` is JSON-deserialized from the
    /// stored TEXT column. Returns `Ok(None)` if the id isn't a session.
    pub fn get_session(&self, id: &str) -> Result<Option<SessionRow>, Box<dyn std::error::Error>> {
        let mut conn = self.conn()?;
        let row = conn.query_opt(
            &format!(
                "SELECT id, origin_eva, intent, candidates, date_start, date_end, status, created_at
                 FROM {}.trip_sessions WHERE id = $1",
                self.schema
            ),
            &[&id],
        )?;
        let Some(r) = row else { return Ok(None) };
        let cands_json: String = r.try_get(3)?;
        let candidates: Vec<CandidateDest> = serde_json::from_str(&cands_json).unwrap_or_default();
        Ok(Some(SessionRow {
            id: r.try_get(0)?,
            origin_eva: r.try_get(1)?,
            intent: r.try_get(2)?,
            candidates,
            date_start: r.try_get(4)?,
            date_end: r.try_get(5)?,
            status: r.try_get(6)?,
            created_at: r.try_get(7)?,
        }))
    }

    /// Lists every trip owned by a session, ranked cheapest-first (NULL
    /// prices last), then by shortest duration as a tiebreaker -- the
    /// ranking a "I feel like a trip, what's cheap?" query actually wants.
    /// Each entry carries its leg set, same shape as `get_trip` returns.
    pub fn list_session_trips(
        &self,
        session_id: &str,
    ) -> Result<Vec<(TripRow, Vec<TripLegRow>)>, Box<dyn std::error::Error>> {
        let mut conn = self.conn()?;
        let trips = conn.query(
            &format!(
                "SELECT id, status, origin_eva, destination_eva, trigger_reason,
                    total_duration_minutes, total_price, created_at, session_id
                 FROM {}.trips WHERE session_id = $1
                 ORDER BY total_price ASC NULLS LAST, total_duration_minutes ASC NULLS LAST",
                self.schema
            ),
            &[&session_id],
        )?;
        let mut out = Vec::with_capacity(trips.len());
        for t in trips {
            let id: String = t.try_get(0)?;
            let trip = TripRow {
                id: id.clone(),
                status: t.try_get(1)?,
                origin_eva: t.try_get(2)?,
                destination_eva: t.try_get(3)?,
                trigger_reason: t.try_get(4)?,
                total_duration_minutes: t.try_get::<_, Option<i32>>(5)?.map(|n| n as u32),
                total_price: t.try_get(6)?,
                created_at: t.try_get(7)?,
                session_id: t.try_get::<_, Option<String>>(8)?,
            };
            let leg_rows = conn.query(
                &format!(
                    "SELECT origin_eva, origin_name, destination_eva, destination_name,
                        departure_time, arrival_time, train_name, train_number, train_category,
                        platform, is_regional
                     FROM {}.trip_legs WHERE trip_id = $1 ORDER BY leg_index",
                    self.schema
                ),
                &[&id],
            )?;
            let mut legs = Vec::with_capacity(leg_rows.len());
            for r in leg_rows {
                legs.push(TripLegRow {
                    origin_eva: r.try_get(0)?,
                    origin_name: r.try_get(1)?,
                    destination_eva: r.try_get(2)?,
                    destination_name: r.try_get(3)?,
                    departure_time: r.try_get(4)?,
                    arrival_time: r.try_get(5)?,
                    train_name: r.try_get(6)?,
                    train_number: r.try_get(7)?,
                    train_category: r.try_get(8)?,
                    platform: r.try_get(9)?,
                    is_regional: r.try_get(10)?,
                });
            }
            out.push((trip, legs));
        }
        Ok(out)
    }
}

/// Shared `trip_legs` insertion -- used by every `record_journey*` path so
/// the leg-replace (delete-then-reinsert-wholesale) discipline is identical
/// whether the recording came from a manual CLI call, the `transit_fare`
/// adapter, or a session run. Caller holds the live transaction.
fn insert_legs(
    tx: &mut postgres::Transaction,
    schema: &str,
    journey_id: &str,
    legs: &[crate::travel::Leg],
) -> Result<(), Box<dyn std::error::Error>> {
    tx.execute(
        &format!("DELETE FROM {}.trip_legs WHERE trip_id = $1", schema),
        &[&journey_id],
    )?;
    for (i, leg) in legs.iter().enumerate() {
        tx.execute(
            &format!(
                "INSERT INTO {schema}.trip_legs (trip_id, leg_index, origin_eva, origin_name,
                    destination_eva, destination_name, departure_time, arrival_time,
                    train_name, train_number, train_category, platform, is_regional)
                VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)",
                schema = schema
            ),
            &[
                &journey_id,
                &(i as i32),
                &leg.origin.id,
                &leg.origin.name,
                &leg.destination.id,
                &leg.destination.name,
                &leg.departure_time,
                &leg.arrival_time,
                &leg.train_name,
                &leg.train_number,
                &leg.train_category,
                &leg.platform,
                &leg.is_regional,
            ],
        )?;
    }
    Ok(())
}

/// A resolved candidate destination for a session -- the EVA id plus the
/// human-readable station name `HafasClient::suggest_stations` returned for
/// it. Stored serialized as JSON in `trip_sessions.candidates`; round-trips
/// via `serde_json`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct CandidateDest {
    pub eva: String,
    pub name: String,
}

/// One fuzzy-trip-search session, read back from the store. `candidates`
/// has been JSON-parsed; an unparseable column yields an empty Vec rather
/// than an error (a session created by a buggy/partial run shouldn't
/// brick `get_session`).
#[derive(Debug, Clone, PartialEq)]
pub struct SessionRow {
    pub id: String,
    pub origin_eva: String,
    pub intent: String,
    pub candidates: Vec<CandidateDest>,
    pub date_start: String,
    pub date_end: String,
    pub status: String,
    pub created_at: String,
}

/// Deterministic session id from the inputs that define the session's
/// shape: origin, the *sorted* candidate destination set, the date window,
/// and the (trimmed, lowercased) intent text. Same inputs -> same id, so
/// re-running `transit plan --destinations Valencia,Copenhagen --date-from
/// 2026-09-01 --date-to 2026-09-30` updates the *same* session row with
/// fresh fare data rather than accumulating duplicate sessions. Same
/// "stable id from identity-bearing inputs" pattern `Opportunity::id` and
/// `Opportunity::fingerprint` already use; no random/uuid dependency
/// needed (and we wouldn't want one here -- a stable id is the feature,
/// not a bug).
pub fn stable_session_id(
    origin_eva: &str,
    candidates: &[CandidateDest],
    date_start: &str,
    date_end: &str,
    intent: &str,
) -> String {
    let mut evas: Vec<&str> = candidates.iter().map(|c| c.eva.as_str()).collect();
    evas.sort_unstable();
    evas.dedup();
    let stem = format!(
        "{origin_eva}|{}|{date_start}|{date_end}|{}",
        evas.join(","),
        intent.trim().to_lowercase()
    );
    let mut h = 0u64;
    for b in stem.as_bytes() {
        h = h.wrapping_mul(31).wrapping_add(*b as u64);
    }
    format!("trip:session:{:016x}", h)
}

#[derive(Debug, Clone, PartialEq)]
pub struct TripRow {
    pub id: String,
    pub status: String,
    pub origin_eva: String,
    pub destination_eva: String,
    pub trigger_reason: String,
    pub total_duration_minutes: Option<u32>,
    pub total_price: Option<f64>,
    pub created_at: String,
    /// Phase 3: which `trip_sessions` row owns this trip, if any -- `None`
    /// for a manual/`transit_fare`-adapter trip (Phase 2 shape).
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TripLegRow {
    pub origin_eva: String,
    pub origin_name: String,
    pub destination_eva: String,
    pub destination_name: String,
    pub departure_time: String,
    pub arrival_time: String,
    pub train_name: String,
    pub train_number: String,
    pub train_category: String,
    pub platform: Option<String>,
    pub is_regional: bool,
}

fn chrono_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}

#[cfg(test)]
mod unit_tests {
    use super::{stable_session_id, CandidateDest};

    #[test]
    fn stable_session_id_is_deterministic_and_input_orderindependent() {
        let cands = vec![
            CandidateDest {
                eva: "8300003".into(),
                name: "Barcelona".into(),
            },
            CandidateDest {
                eva: "8600206".into(),
                name: "Valencia".into(),
            },
        ];
        let id_a = stable_session_id(
            "8000044",
            &cands,
            "2026-09-01",
            "2026-09-30",
            "Valencia or Barcelona",
        );
        // Candidate order should NOT change the id (the helper sorts first).
        let reversed = vec![
            CandidateDest {
                eva: "8600206".into(),
                name: "Valencia".into(),
            },
            CandidateDest {
                eva: "8300003".into(),
                name: "Barcelona".into(),
            },
        ];
        let id_b = stable_session_id(
            "8000044",
            &reversed,
            "2026-09-01",
            "2026-09-30",
            "Valencia or Barcelona",
        );
        assert_eq!(id_a, id_b, "candidate order must not change the session id");
        // Intent is trimmed + lowercased so whitespace/case differences collapse.
        let id_c = stable_session_id(
            "8000044",
            &cands,
            "2026-09-01",
            "2026-09-30",
            "  VALENCIA or barcelona  ",
        );
        assert_eq!(
            id_a, id_c,
            "intent should be trimmed+lowercased before hashing"
        );
        // A different window or origin or candidate set changes the id.
        assert_ne!(
            id_a,
            stable_session_id(
                "8000044",
                &cands,
                "2026-10-01",
                "2026-09-30",
                "Valencia or Barcelona"
            )
        );
        assert_ne!(
            id_a,
            stable_session_id(
                "8000050",
                &cands,
                "2026-09-01",
                "2026-09-30",
                "Valencia or Barcelona"
            )
        );
        let only_val = vec![CandidateDest {
            eva: "8600206".into(),
            name: "Valencia".into(),
        }];
        assert_ne!(
            id_a,
            stable_session_id(
                "8000044",
                &only_val,
                "2026-09-01",
                "2026-09-30",
                "Valencia or Barcelona"
            )
        );
        assert!(
            id_a.starts_with("trip:session:"),
            "session ids should be namespaced"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::travel::{Leg, Station};
    use postgres::NoTls;

    // Same schema-per-test isolation pattern as scouting::store::tests --
    // see that module's doc comment for the full rationale.
    /// The same connection the binaries use, so a rotated Postgres password
    /// can't leave the tests behind. Resolved once: the config tests mutate
    /// process-global env while these run alongside them, and every store test
    /// must agree on one database.
    fn test_database_url() -> String {
        static URL: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        URL.get_or_init(|| {
            std::env::var("TRANSIT_TEST_DATABASE_URL")
                .unwrap_or_else(|_| crate::config::Config::load().database_url)
        })
        .clone()
    }

    fn open_test_store(name: &str) -> (TransitStore, TestSchema) {
        let schema = format!("transit_test_{name}_{}", std::process::id());
        let store = TransitStore::open_with_schema(&test_database_url(), &schema)
            .unwrap_or_else(|e| panic!("could not open test store (is capabilities/postgres running? see README): {e}"));
        (store, TestSchema(schema))
    }

    /// Drops the schema when it goes out of scope, including on unwind.
    ///
    /// The tests used to call drop_test_schema() as their last statement, which cleans up
    /// exactly when nothing goes wrong. A failing assertion panics first, the drop never
    /// runs, and the schema stays behind -- four of them were sitting in the shared
    /// database on 2026-07-28, from two long-finished processes, and every one would have
    /// gone into the next pg_dumpall. A guard runs on the way out either way.
    struct TestSchema(String);

    impl Drop for TestSchema {
        fn drop(&mut self) {
            drop_test_schema(&self.0);
        }
    }

    /// So a test that needs the schema name for raw SQL can still say `{schema}`.
    impl std::fmt::Display for TestSchema {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(&self.0)
        }
    }

    fn drop_test_schema(schema: &str) {
        if let Ok(mut client) = Client::connect(&test_database_url(), NoTls) {
            let _ = client.batch_execute(&format!("DROP SCHEMA IF EXISTS {schema} CASCADE"));
        }
    }

    fn mk_journey(id: &str) -> Journey {
        let bonn = Station { id: "8000044".into(), name: "Bonn Hbf".into(), latitude: None, longitude: None };
        let berlin = Station { id: "8098160".into(), name: "Berlin Hbf".into(), latitude: None, longitude: None };
        Journey {
            id: id.into(),
            start_station: bonn.clone(),
            end_station: berlin.clone(),
            legs: vec![Leg {
                origin: bonn,
                destination: berlin,
                departure_time: "2026-08-01T08:00:00".into(),
                arrival_time: "2026-08-01T12:00:00".into(),
                train_name: "ICE 691".into(),
                train_number: "691".into(),
                train_category: "ICE".into(),
                platform: Some("3".into()),
                is_regional: false,
            }],
            total_duration_minutes: 240,
            total_price: Some(79.90),
            delay_risk_score: None,
        }
    }

    #[test]
    fn record_journey_is_idempotent_and_round_trips() {
        let (store, _schema) = open_test_store("idempotent");

        let journey = mk_journey("journey:test:1");
        let is_new1 = store.record_journey(&journey, "8000044", "8098160", "auto", None).unwrap();
        assert!(is_new1, "first record should be new");
        let is_new2 = store.record_journey(&journey, "8000044", "8098160", "auto", None).unwrap();
        assert!(!is_new2, "re-recording the same journey id should not be new");

        assert_eq!(store.count().unwrap(), 1);

        let (trip, legs) = store.get_trip("journey:test:1").unwrap().expect("trip should exist");
        assert_eq!(trip.origin_eva, "8000044");
        assert_eq!(trip.destination_eva, "8098160");
        assert_eq!(trip.trigger_reason, "auto");
        assert_eq!(trip.status, "new", "freshly recorded trip defaults to 'new'");
        assert_eq!(trip.total_duration_minutes, Some(240));
        assert!((trip.total_price.unwrap() - 79.90).abs() < 1e-9);

        assert_eq!(legs.len(), 1);
        assert_eq!(legs[0].train_number, "691");
        assert_eq!(legs[0].platform.as_deref(), Some("3"));
        assert!(!legs[0].is_regional);

    }

    #[test]
    fn record_journey_replaces_legs_on_update() {
        let (store, _schema) = open_test_store("replace_legs");

        let mut journey = mk_journey("journey:test:2");
        store.record_journey(&journey, "8000044", "8098160", "manual", None).unwrap();

        // Re-recording with a different leg set should leave exactly the new
        // legs, not the old ones appended alongside them.
        journey.legs.push(journey.legs[0].clone());
        journey.total_duration_minutes = 300;
        store.record_journey(&journey, "8000044", "8098160", "manual", None).unwrap();

        let (trip, legs) = store.get_trip("journey:test:2").unwrap().unwrap();
        assert_eq!(trip.total_duration_minutes, Some(300));
        assert_eq!(legs.len(), 2, "leg set should be fully replaced, not appended to");

    }

    #[test]
    fn record_journey_rejects_invalid_trigger_reason() {
        let (store, _schema) = open_test_store("invalid_trigger");
        let journey = mk_journey("journey:test:3");
        let result = store.record_journey(&journey, "8000044", "8098160", "scheduled", None);
        assert!(result.is_err(), "an unrecognized trigger_reason must error, not silently accept");
    }

    // ── Phase 3: trip sessions ───────────────────────────────────────────

    fn mk_session_journey(id: &str, dest_eva: &str, dest_name: &str, price: Option<f64>, dur: u32) -> Journey {
        let bonn = Station { id: "8000044".into(), name: "Bonn Hbf".into(), latitude: None, longitude: None };
        let dest = Station { id: dest_eva.into(), name: dest_name.into(), latitude: None, longitude: None };
        Journey {
            id: id.into(),
            start_station: bonn.clone(),
            end_station: dest.clone(),
            legs: vec![Leg {
                origin: bonn,
                destination: dest,
                departure_time: "2026-09-10T08:00:00".into(),
                arrival_time: "2026-09-10T14:00:00".into(),
                train_name: "ICE 691".into(),
                train_number: "691".into(),
                train_category: "ICE".into(),
                platform: Some("3".into()),
                is_regional: false,
            }],
            total_duration_minutes: dur,
            total_price: price,
            delay_risk_score: None,
        }
    }

    #[test]
    fn upsert_session_is_idempotent_and_round_trips() {
        let (store, _schema) = open_test_store("session_upsert");
        let cands = vec![
            CandidateDest { eva: "8600206".into(), name: "Valencia".into() },
            CandidateDest { eva: "8300003".into(), name: "Barcelona".into() },
        ];
        let id = stable_session_id("8000044", &cands, "2026-09-01", "2026-09-30", "Valencia or Barcelona");
        let is_new1 = store.upsert_session(&id, "8000044", "Valencia or Barcelona", &cands, "2026-09-01", "2026-09-30").unwrap();
        assert!(is_new1, "first upsert of a session should be new");
        // Tweaked intent (same shape) should UPDATE, not create a second row.
        let is_new2 = store.upsert_session(&id, "8000044", "Valencia or Barcelona, open to nearby", &cands, "2026-09-01", "2026-09-30").unwrap();
        assert!(!is_new2, "re-upserting the same session id should update, not insert");

        let s = store.get_session(&id).unwrap().expect("session should exist");
        assert_eq!(s.origin_eva, "8000044");
        assert_eq!(s.intent, "Valencia or Barcelona, open to nearby");
        assert_eq!(s.date_start, "2026-09-01");
        assert_eq!(s.date_end, "2026-09-30");
        assert_eq!(s.status, "new");
        assert_eq!(s.candidates.len(), 2);
        assert!(s.candidates.iter().any(|c| c.eva == "8600206" && c.name == "Valencia"));
        assert!(s.candidates.iter().any(|c| c.eva == "8300003" && c.name == "Barcelona"));
    }

    #[test]
    fn session_journeys_are_tagged_owned_and_ranked_by_price() {
        let (store, _schema) = open_test_store("session_rank");
        let cands = vec![ CandidateDest { eva: "8600206".into(), name: "Valencia".into() } ];
        let sid = stable_session_id("8000044", &cands, "2026-09-01", "2026-09-30", "Valencia in Sept");
        store.upsert_session(&sid, "8000044", "Valencia in Sept", &cands, "2026-09-01", "2026-09-30").unwrap();

        // Three found journeys -- record them session-scoped, prices mixed
        // so the ranking order is provably cheapest-first, not insert order.
        store.record_journey(&mk_session_journey("j:expensive", "8600206", "Valencia", Some(129.50), 360), "8000044", "8600206", "session", Some(&sid)).unwrap();
        store.record_journey(&mk_session_journey("j:cheap", "8600206", "Valencia", Some(49.90), 380), "8000044", "8600206", "session", Some(&sid)).unwrap();
        store.record_journey(&mk_session_journey("j:mid", "8600206", "Valencia", Some(89.00), 340), "8000044", "8600206", "session", Some(&sid)).unwrap();

        let trips = store.list_session_trips(&sid).unwrap();
        assert_eq!(trips.len(), 3, "all three session journeys should be owned by the session");
        // Cheapest-first.
        assert_eq!(trips[0].0.id, "j:cheap");
        assert!((trips[0].0.total_price.unwrap() - 49.90).abs() < 1e-9);
        assert_eq!(trips[1].0.id, "j:mid");
        assert_eq!(trips[2].0.id, "j:expensive");
        // Each carries trigger_reason='session' and the session_id back.
        assert_eq!(trips[0].0.trigger_reason, "session");
        assert_eq!(trips[0].0.session_id.as_deref(), Some(sid.as_str()));
        // Legs round-trip too.
        assert_eq!(trips[0].1.len(), 1);
        assert_eq!(trips[0].1[0].destination_name, "Valencia");
    }

    #[test]
    fn replanning_session_refreshes_fares_without_losing_status() {
        // A re-found journey (same id, fresh price) under the SAME session:
        // the price/duration refresh, but a hand-set 'saved' status survives
        // -- same invariant scouting::store::upsert_preserves_status guards.
        let (store, schema) = open_test_store("session_refresh");
        let cands = vec![ CandidateDest { eva: "8600206".into(), name: "Valencia".into() } ];
        let sid = stable_session_id("8000044", &cands, "2026-09-01", "2026-09-30", "Valencia");
        store.upsert_session(&sid, "8000044", "Valencia", &cands, "2026-09-01", "2026-09-30").unwrap();
        let j = mk_session_journey("j:refresh", "8600206", "Valencia", Some(70.00), 360);
        store.record_journey(&j, "8000044", "8600206", "session", Some(&sid)).unwrap();

        // Hand-mark saved out-of-band (there's no CLI for it yet -- direct SQL
        // is the test seam; same pattern scouting::store::tests uses to test
        // the status-preservation path before a CLI exists).
        {
            let mut conn = Client::connect(&test_database_url(), NoTls).unwrap();
            conn.execute(
                &format!("UPDATE {}.trips SET status='saved' WHERE id='j:refresh'", schema),
                &[],
            ).unwrap();
        }
        let (trip, _) = store.get_trip("j:refresh").unwrap().unwrap();
        assert_eq!(trip.status, "saved");

        // Re-plan: same journey id, cheaper fare now.
        let mut j2 = j.clone();
        j2.total_price = Some(55.00);
        store.record_journey(&j2, "8000044", "8600206", "session", Some(&sid)).unwrap();

        let (trip2, _) = store.get_trip("j:refresh").unwrap().unwrap();
        assert_eq!(trip2.status, "saved", "status must survive a fare refresh");
        assert!((trip2.total_price.unwrap() - 55.00).abs() < 1e-9, "price should refresh");
    }

    #[test]
    fn session_scope_isolates_trips_from_manual_and_auto() {
        // A manual/auto trip (session_id=None) should NOT show up in a
        // session's list even if it shares origin/destination with session
        // trips -- the session_id filter is what keeps query #2's results
        // separate from #1's background-scan results.
        let (store, _schema) = open_test_store("session_isolation");
        let cands = vec![ CandidateDest { eva: "8600206".into(), name: "Valencia".into() } ];
        let sid = stable_session_id("8000044", &cands, "2026-09-01", "2026-09-30", "Valencia");
        store.upsert_session(&sid, "8000044", "Valencia", &cands, "2026-09-01", "2026-09-30").unwrap();

        store.record_journey(&mk_session_journey("j:manual", "8600206", "Valencia", Some(70.00), 360), "8000044", "8600206", "manual", None).unwrap();
        store.record_journey(&mk_session_journey("j:session", "8600206", "Valencia", Some(70.00), 360), "8000044", "8600206", "session", Some(&sid)).unwrap();

        let trips = store.list_session_trips(&sid).unwrap();
        assert_eq!(trips.len(), 1, "manual trip must not leak into the session list");
        assert_eq!(trips[0].0.id, "j:session");
        assert!(store.get_trip("j:manual").unwrap().unwrap().0.session_id.is_none());
    }
}
