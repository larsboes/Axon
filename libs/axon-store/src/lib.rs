//! Run a capability's schema migration once per process, not once per `Store::open`.
//!
//! ## The failure this exists to prevent
//!
//! Every capability's `Store::open` used to connect and then run its whole
//! migration: a hundred-odd `CREATE TABLE IF NOT EXISTS` and `ALTER TABLE ... ADD
//! COLUMN IF NOT EXISTS` statements in one `batch_execute`, which is one
//! transaction. A no-op `ADD COLUMN IF NOT EXISTS` still takes ACCESS EXCLUSIVE on
//! its table and holds it to commit, so a single opener accumulates that lock on
//! roughly fifteen tables at once. Two openers doing that concurrently each hold
//! what the other needs, and Postgres resolves the cycle by killing one session.
//!
//! That is not a rare race. `comms` alone opens a store in 43 HTTP handlers and
//! five timers, so it fired whenever two timers landed on the same tick. It
//! surfaced as `digest drain: db error: ERROR: deadlock detected` once a third
//! fifteen-minute timer joined the other two; the two that were already colliding
//! did it rarely enough to look like weather.
//!
//! ## Why once-per-process rather than a lock around it
//!
//! An advisory lock serialises the contention, it does not remove it. Every opener
//! still pays for a migration that has been a no-op since boot, and the openers
//! still queue behind each other. Running it once per (database, schema) removes
//! the contention instead: the second and every later `open` in a process do no DDL
//! at all, so there is nothing left to deadlock on and nothing to wait for.
//!
//! The cross-process gate stays anyway, because "once per process" says nothing
//! about two processes of the same capability starting together — a CLI run while
//! the server is booting, or a restart overlapping the process it replaces. That
//! window is real, short, and exactly what `pg_advisory_lock` covers.
//!
//! ## What this deliberately keeps
//!
//! The old design was self-healing by accident: any code path could open a store
//! against an empty database and it worked, which is what every test helper leans
//! on — they build a per-pid schema and call `open`. That property survives here.
//! The first `open` for a given (database, schema) still migrates, so no caller
//! inherits an assumption that someone else went first. A failed migration is not
//! recorded, so the next `open` retries rather than inheriting a half-built schema.
//!
//! ## The other half: one connection per request
//!
//! Migrating once fixed the deadlock and little of the latency, because the DDL was
//! never more than 2-3 ms of it. What every `Store::open` also did was open a fresh
//! Postgres session, measured against the live database rather than estimated:
//!
//! ```text
//! Client::connect      32-39 ms   (five runs)
//! pooled Store::open   0.20-0.30 ms
//! ```
//!
//! [`pool_for`] gives a process one pool per database URL, so `Store::open` becomes
//! a checkout. Call sites did not change for that: `open` still takes a URL and
//! still returns a `Store`, because a pool keyed by URL is the shape 43 handlers
//! were already asking for without knowing it.
//!
//! ## What this does not make fast, and the estimate that was wrong
//!
//! The originating issue read `/feed` at ~76 ms against `/health` at ~2 ms and
//! concluded roughly 50 ms per request was connection setup. That was an
//! overestimate: the connect is ~32 ms, and the rest of the gap is not fixed cost
//! at all. It scales with the response:
//!
//! ```text
//! /feed?days=1     57 KB    ~29-56 ms
//! /feed?days=7    107 KB    ~57 ms      (median of 15, was ~78 ms before pooling)
//! /feed?days=30   194 KB    ~97 ms
//! ```
//!
//! So the endpoint improved by about 21 ms and remains dominated by per-item work
//! and serialization. Worth writing down rather than quietly not mentioning: a pool
//! is the right fix for the connect, and it was never going to be the fix for the
//! other two thirds.
//!
//! ## Dependency rule
//!
//! Compiled into consumers by `#[path]` include (see `libs/axon-config/README.md`
//! for why), so it may only use crates **every** consumer already has: `postgres`,
//! `r2d2` and `r2d2_postgres`, which all seven store-owning capabilities depend on
//! at the same versions. Adding any other dependency here silently changes a
//! consumer's dependency resolution.

use std::collections::HashMap;
use std::collections::HashSet;
use std::error::Error;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use postgres::{Client, NoTls};
use r2d2_postgres::PostgresConnectionManager;

/// A pooled Postgres connection. Derefs to [`Client`], so a call site that used to
/// hold a `MutexGuard<Client>` reads the same.
pub type PooledClient = r2d2::PooledConnection<PostgresConnectionManager<NoTls>>;

/// Shared by every `Store` in a process that talks to the same database.
pub type Pool = r2d2::Pool<PostgresConnectionManager<NoTls>>;

/// Beyond this, a checkout gives up rather than hanging.
///
/// r2d2's default is 30 seconds. That is a sane default for a batch job and the
/// wrong one for an HTTP handler: with Postgres down, 30 seconds is long enough
/// that the dashboard looks hung rather than broken, and long enough for the
/// requests behind it to pile up. Five seconds is past any real contention on a
/// pool this size and short enough to read as a failure.
const CHECKOUT_TIMEOUT: Duration = Duration::from_secs(5);

/// r2d2's own default, named rather than inherited so it is visible next to the
/// two settings that are NOT defaults.
const MAX_CONNECTIONS: u32 = 10;

fn pools() -> &'static Mutex<HashMap<String, Pool>> {
    static POOLS: OnceLock<Mutex<HashMap<String, Pool>>> = OnceLock::new();
    POOLS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The pool for `database_url` in this process, built on first ask.
///
/// Keyed by URL rather than by capability: two capabilities pointing at the same
/// database should share, and a test schema is not a different database.
///
/// `min_idle(0)` is the setting that matters and is not a default. r2d2 otherwise
/// keeps `max_size` connections warm, which is right for a server and actively
/// wrong for the CLI half of these crates: `comms sweep` would open ten sessions to
/// run one query and close nine of them on exit. Zero means the pool grows to what
/// is actually used, so the one-shot path costs one connection and the server path
/// still reaches ten under load.
pub fn pool_for(database_url: &str) -> Result<Pool, Box<dyn Error>> {
    let mut pools = pools()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    if let Some(existing) = pools.get(database_url) {
        return Ok(existing.clone());
    }

    let manager = PostgresConnectionManager::new(database_url.parse()?, NoTls);
    let pool = r2d2::Pool::builder()
        .max_size(MAX_CONNECTIONS)
        .min_idle(Some(0))
        .connection_timeout(CHECKOUT_TIMEOUT)
        .build(manager)?;

    pools.insert(database_url.to_string(), pool.clone());
    Ok(pool)
}

/// Get a pool for `database_url` and make sure `schema` is migrated.
///
/// The checkout here is not only a probe, though it serves as one. `min_idle(0)`
/// means `build` establishes nothing, so without it a wrong URL or a stopped
/// database would be reported by the first query rather than by `open` — a
/// regression against the connect-per-open behaviour this replaces, and the kind
/// that turns "the database is down" into a confusing error deep in a handler.
pub fn open_pool(
    database_url: &str,
    schema: &str,
    ddl: impl FnOnce(&mut Client) -> Result<(), Box<dyn Error>>,
) -> Result<Pool, Box<dyn Error>> {
    let pool = pool_for(database_url)?;
    let mut conn = pool.get()?;
    migrate_once(&mut conn, database_url, schema, ddl)?;
    drop(conn);
    Ok(pool)
}

/// Serialises migrations across processes.
///
/// One key for every capability rather than one derived per schema. Two
/// capabilities migrating different schemas cannot deadlock each other anyway —
/// different tables, different locks — so a shared key costs them a few
/// milliseconds of waiting at boot and buys the property that matters: during a
/// restart, the outgoing process and the incoming one must agree on which number
/// to wait on, and a derived key stops agreeing the moment a schema name changes.
///
/// Deliberately distinct from the key in comms' `local_gate.rs`. They guard
/// unrelated things, and sharing a number would make each wait on the other.
const MIGRATION_LOCK_KEY: i64 = 0x41_58_4F_4E_5F_4D_49_47; // "AXON_MIG"

fn migrated() -> &'static Mutex<HashSet<String>> {
    static MIGRATED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    MIGRATED.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Runs `work` the first time this process sees a given (database, schema), and
/// never again for that pair.
///
/// Split out from [`migrate_once`] so the once-guard is testable without a
/// database: everything subtle here is about locking and retry, and none of it
/// needs Postgres to be wrong.
///
/// `work` running to an error is not recorded. A half-built schema that reports
/// itself as migrated is the one outcome worse than migrating twice.
pub fn once_per_target<E>(
    database_url: &str,
    schema: &str,
    work: impl FnOnce() -> Result<(), E>,
) -> Result<(), E> {
    // A unit separator, because a database URL and a schema name are both
    // arbitrary text: concatenated without one, ("a", "bc") and ("ab", "c") are
    // the same key.
    let key = format!("{database_url}\u{1f}{schema}");

    // Held across `work`, not merely across the lookup. Releasing between the
    // check and the migration lets two threads both miss and both migrate, which
    // is the precise race this exists to close.
    //
    // Poisoning is recovered rather than propagated. The key is inserted only on
    // success, so a panicking migration leaves the set correct, and refusing to
    // migrate for the remaining life of the process is a worse failure than the
    // one that poisoned it.
    let mut done = migrated()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    if done.contains(&key) {
        return Ok(());
    }

    work()?;
    done.insert(key);
    Ok(())
}

/// Migrates `schema` if this process has not already, holding the cross-process
/// advisory lock while it does.
///
/// The DDL itself stays in the capability — it is the one part of this that is
/// genuinely per-capability. What is shared is when it runs and what stops two
/// sessions running it at once.
pub fn migrate_once(
    client: &mut Client,
    database_url: &str,
    schema: &str,
    ddl: impl FnOnce(&mut Client) -> Result<(), Box<dyn Error>>,
) -> Result<(), Box<dyn Error>> {
    once_per_target(database_url, schema, || {
        // The blocking form, not `try`: an opener that cannot migrate yet should
        // wait for the one that is, not fail.
        client.execute("SELECT pg_advisory_lock($1)", &[&MIGRATION_LOCK_KEY])?;
        let result = ddl(client);
        // Released on both paths. A failed migration that kept the lock would hang
        // every future opener behind a session that has already given up.
        let _ = client.execute("SELECT pg_advisory_unlock($1)", &[&MIGRATION_LOCK_KEY]);
        result
    })
}

#[cfg(all(test, feature = "standalone-tests"))]
mod tests {
    use super::once_per_target;
    use std::cell::Cell;

    /// Distinct per test: the guard is process-global by design, so two tests
    /// sharing a (url, schema) would see each other's result and pass for the
    /// wrong reason.
    fn url(tag: &str) -> String {
        format!("postgres://test/{tag}")
    }

    #[test]
    fn runs_the_first_time_and_not_again() {
        let runs = Cell::new(0);
        let bump = || {
            runs.set(runs.get() + 1);
            Ok::<(), ()>(())
        };

        once_per_target(&url("repeat"), "s", bump).unwrap();
        once_per_target(&url("repeat"), "s", bump).unwrap();
        once_per_target(&url("repeat"), "s", bump).unwrap();

        assert_eq!(runs.get(), 1, "only the first call should do the work");
    }

    #[test]
    fn a_failure_is_retried_rather_than_recorded() {
        let runs = Cell::new(0);

        let first = once_per_target(&url("retry"), "s", || {
            runs.set(runs.get() + 1);
            Err::<(), &str>("migration blew up")
        });
        assert_eq!(first, Err("migration blew up"));

        once_per_target(&url("retry"), "s", || {
            runs.set(runs.get() + 1);
            Ok::<(), &str>(())
        })
        .unwrap();

        assert_eq!(
            runs.get(),
            2,
            "a failed migration must not mark the target as done"
        );
    }

    #[test]
    fn schemas_are_tracked_separately() {
        let runs = Cell::new(0);
        let bump = || {
            runs.set(runs.get() + 1);
            Ok::<(), ()>(())
        };

        once_per_target(&url("schemas"), "one", bump).unwrap();
        once_per_target(&url("schemas"), "two", bump).unwrap();
        once_per_target(&url("schemas"), "one", bump).unwrap();

        assert_eq!(runs.get(), 2, "each schema migrates on its own");
    }

    #[test]
    fn databases_are_tracked_separately() {
        let runs = Cell::new(0);
        let bump = || {
            runs.set(runs.get() + 1);
            Ok::<(), ()>(())
        };

        once_per_target(&url("db-a"), "same", bump).unwrap();
        once_per_target(&url("db-b"), "same", bump).unwrap();

        assert_eq!(runs.get(), 2, "the same schema in two databases is two targets");
    }

    /// The separator matters: without it ("…/ab", "c") and ("…/a", "bc") collide,
    /// and the second schema silently never migrates.
    #[test]
    fn a_shared_prefix_does_not_collide() {
        let runs = Cell::new(0);
        let bump = || {
            runs.set(runs.get() + 1);
            Ok::<(), ()>(())
        };

        once_per_target("postgres://test/collideab", "c", bump).unwrap();
        once_per_target("postgres://test/collidea", "bc", bump).unwrap();

        assert_eq!(runs.get(), 2, "the key must not be ambiguous at the join");
    }
}
