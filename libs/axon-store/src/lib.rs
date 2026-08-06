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
//! ## Dependency rule
//!
//! Compiled into consumers by `#[path]` include (see `libs/axon-config/README.md`
//! for why), so it may only use crates **every** consumer already has: `postgres`,
//! which all seven store-owning capabilities depend on at the same version. Adding
//! any other dependency here silently changes a consumer's dependency resolution.

use std::collections::HashSet;
use std::error::Error;
use std::sync::{Mutex, OnceLock};

use postgres::Client;

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
