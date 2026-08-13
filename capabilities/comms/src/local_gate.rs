//! Cross-process admission control for the local inference server.
//!
//! ## Why a lock and not a semaphore
//!
//! On 2026-08-05 four concurrent prefills pushed oMLX past its hard memory
//! watermark and it aborted all four. Two of them were feed digests. Nothing in
//! Axon knew the others were running, because the competing consumers are
//! separate processes: comms-server, graphify, the Pulse DA, anything else that
//! resolves the summarization role. An in-process semaphore bounds one of them
//! and lets the rest collide exactly as before.
//!
//! Postgres advisory locks are the mechanism because comms already has
//! Postgres, so this adds no dependency and no daemon. Two properties earn it
//! over a lock file: the lock is held by a *session*, so a process that dies
//! mid-request releases it without leaving a stale file behind; and waiting is
//! done by the database rather than by a polling loop.
//!
//! ## Why the cap is one
//!
//! oMLX already batches and schedules; what it does not do is refuse work it
//! cannot fit, which is how a fourth request turns three healthy ones into four
//! aborted ones. One at a time is the setting that cannot produce that. It
//! costs throughput only when two consumers want the GPU at once, and in that
//! situation the alternative was not parallelism — it was a memory abort.
//!
//! ## Why the cap is one *per backend*
//!
//! The first version held one lock for the whole machine, which was right while
//! oMLX was the only local server. It stopped being right the moment Apple
//! Foundation Models joined it: AFM runs on the Neural Engine at zero Metal
//! cost and shares no memory pool with oMLX, so an AFM digest that waits for an
//! oMLX prefill to finish waits ninety seconds for a resource it was never
//! going to touch, and is then told the machine is busy. The resource being
//! protected is a *backend's* memory, not the machine's, so the lock is keyed
//! by backend name: `foundation-models` and `omlx` contend with themselves and
//! with every other process resolving the same backend, and with nobody else.
//!
//! ## What a caller sees when the wait runs out
//!
//! `Outcome::CapacityAborted`, which is retryable and reported to a reader as a
//! machine condition. A background drain picks the item up on its next pass; an
//! operator pressing Regenerate is told the machine is busy rather than left
//! watching a spinner for the length of somebody else's transcript.

use std::sync::Arc;
use std::time::{Duration, Instant};

use postgres::{Client, NoTls};

use crate::summarize::{Admission, LocalGate};

/// The high half of every key this module takes. Advisory-lock keys share one
/// namespace per database, so a recognisable prefix keeps these entries legible
/// in `pg_locks` and out of the way of any other subsystem's small round number.
const LOCAL_INFERENCE_NAMESPACE: i64 = 0x41_58_4F_4E; // "AXON"

/// Which lock a backend takes. Any two processes must derive the same number
/// from the same backend name or they do not contend, and the whole point is
/// that they contend — so the hash is written out here rather than taken from
/// `DefaultHasher`, whose output std explicitly does not promise to keep stable
/// across releases. Two binaries built a month apart must still collide.
///
/// FNV-1a, 32-bit. The low half identifies the backend, the high half is the
/// namespace above, so the whole key stays positive and readable as
/// `AXON:<backend hash>`.
pub(crate) fn lock_key(backend_name: &str) -> i64 {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in backend_name.as_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    (LOCAL_INFERENCE_NAMESPACE << 32) | i64::from(hash)
}

/// How long a caller waits before being told the machine is busy.
///
/// Sized against the work, not the impatience: a sectioned digest of a long
/// transcript takes 12-20s on this machine, and a queue two deep is normal
/// during a drain. Much shorter and a drain would spend its passes reporting
/// contention; much longer and an operator's press hangs behind a backlog.
const WAIT_TIMEOUT: Duration = Duration::from_secs(90);

/// One Postgres advisory lock, held for the duration of a local request to one
/// backend.
pub struct AdvisoryGate {
    database_url: String,
    /// The resolved role's `backend_name`, not the model. Two roles on the same
    /// backend share one memory pool and must contend; the same model served by
    /// two backends does not.
    backend_name: String,
}

impl AdvisoryGate {
    pub fn new(database_url: impl Into<String>, backend_name: impl Into<String>) -> Self {
        Self {
            database_url: database_url.into(),
            backend_name: backend_name.into(),
        }
    }

    /// Wrap in the `Arc` that `Target::gate` wants.
    pub fn shared(
        database_url: impl Into<String>,
        backend_name: impl Into<String>,
    ) -> Arc<dyn LocalGate> {
        Arc::new(Self::new(database_url, backend_name))
    }
}

impl LocalGate for AdvisoryGate {
    fn acquire(&self) -> Result<Admission, String> {
        // A dedicated connection, not the Store's. The lock is session-scoped,
        // so it must outlive `acquire` and die with the request -- and holding
        // the Store's mutex for the length of a model call would stall every
        // unrelated query in this process for twenty seconds at a time.
        let mut client = Client::connect(&self.database_url, NoTls)
            .map_err(|error| format!("local inference gate: no database session ({error})"))?;

        // `pg_try_advisory_lock` rather than the blocking form: the blocking one
        // waits inside the server with no deadline this side can enforce, and a
        // caller that waits forever is indistinguishable from a hung model.
        let key = lock_key(&self.backend_name);
        let deadline = Instant::now() + WAIT_TIMEOUT;
        loop {
            let got: bool = client
                .query_one("SELECT pg_try_advisory_lock($1)", &[&key])
                .map_err(|error| format!("local inference gate: {error}"))?
                .get(0);
            if got {
                break;
            }
            if Instant::now() >= deadline {
                // Names the backend: "the machine is busy" sent a reader
                // looking at the wrong server the first time two of them ran.
                return Err(format!(
                    "another local inference request held {} for more than {}s",
                    self.backend_name,
                    WAIT_TIMEOUT.as_secs()
                ));
            }
            std::thread::sleep(Duration::from_millis(250));
        }

        // Dropping the client ends the session, which releases the lock even if
        // this process is killed between here and the unlock. The explicit
        // unlock is for the ordinary path, so the connection is not the only
        // thing standing between a finished request and the next one.
        Ok(Admission::new(move || {
            let _ = client.execute("SELECT pg_advisory_unlock($1)", &[&key]);
            drop(client);
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// The contract `complete` relies on: whatever a gate hands back releases
    /// itself when it goes out of scope, on every path out of the request.
    #[test]
    fn an_admission_releases_itself_on_drop() {
        static RELEASED: AtomicUsize = AtomicUsize::new(0);
        {
            let _held = Admission::new(|| {
                RELEASED.fetch_add(1, Ordering::SeqCst);
            });
            assert_eq!(RELEASED.load(Ordering::SeqCst), 0, "not while it is held");
        }
        assert_eq!(RELEASED.load(Ordering::SeqCst), 1, "released by drop");
    }

    /// Early returns are the reason drop was chosen over an explicit release.
    #[test]
    fn an_admission_releases_on_an_early_return() {
        static RELEASED: AtomicUsize = AtomicUsize::new(0);
        fn bail() -> Result<(), &'static str> {
            let _held = Admission::new(|| {
                RELEASED.fetch_add(1, Ordering::SeqCst);
            });
            Err("something went wrong before the end of the function")
        }
        assert!(bail().is_err());
        assert_eq!(RELEASED.load(Ordering::SeqCst), 1);
    }

    /// A gate with nothing to clean up must still be a valid permit, so a
    /// counting implementation does not have to invent a no-op closure.
    #[test]
    fn a_free_admission_is_a_permit_too() {
        let _held = Admission::free();
    }

    /// The namespace is what keeps these entries recognisable in `pg_locks` and
    /// clear of whatever else reaches for an advisory lock in this database.
    /// Pinned to its derivation rather than to a copy of itself.
    #[test]
    fn every_key_sits_in_the_axon_namespace() {
        assert_eq!(
            LOCAL_INFERENCE_NAMESPACE,
            i64::from(u32::from_be_bytes(*b"AXON")),
            "the namespace must stay byte-identical across every process"
        );
        for backend in ["omlx", "foundation-models", "ollama", ""] {
            let key = lock_key(backend);
            assert_eq!(
                key >> 32,
                LOCAL_INFERENCE_NAMESPACE,
                "{backend} landed outside the namespace"
            );
            assert!(
                key > 0,
                "a negative key still works but reads as a mistake in pg_locks"
            );
        }
    }

    /// The whole point of C15. Two loopback backends share no memory pool, so
    /// they must not share a lock: an AFM digest queuing ninety seconds behind
    /// an oMLX prefill is a `CapacityAborted` for a resource it never touched.
    #[test]
    fn two_backends_do_not_contend_with_each_other() {
        assert_ne!(
            lock_key("foundation-models"),
            lock_key("omlx"),
            "AFM and oMLX must take different locks"
        );
        // Every pair, so a hash collision between two names actually in
        // inference.json fails here rather than in production as a mystery wait.
        let backends = [
            "omlx",
            "foundation-models",
            "ollama",
            "llama-cpp",
            "lmstudio",
        ];
        for (index, left) in backends.iter().enumerate() {
            for right in &backends[index + 1..] {
                assert_ne!(
                    lock_key(left),
                    lock_key(right),
                    "{left} collides with {right}"
                );
            }
        }
    }

    /// And the other half: the same backend named twice must contend, or the
    /// gate guards nothing while looking exactly like one that works. Two
    /// processes derive this independently, so it has to be a pure function of
    /// the name and nothing else.
    #[test]
    fn one_backend_always_derives_the_same_key() {
        assert_eq!(lock_key("omlx"), lock_key("omlx"));
        assert_eq!(
            AdvisoryGate::new("postgres://a", "omlx").backend_name,
            "omlx"
        );
        // Pinned literals: a refactor of the hash that changes these silently
        // splits a running process from a restarted one, and both would report
        // healthy while sharing no lock at all.
        assert_eq!(lock_key("omlx"), 0x41_58_4F_4E_EA_68_17_05);
        assert_eq!(lock_key("foundation-models"), 0x41_58_4F_4E_54_17_33_81);
    }

    /// The same connection the binaries use — see `store.rs`'s note on why a
    /// second hardcoded default here is what once left these tests passing
    /// against nothing.
    fn test_database_url() -> String {
        std::env::var("COMMS_TEST_DATABASE_URL")
            .unwrap_or_else(|_| crate::config::Config::load().database_url)
    }

    /// C15 against real Postgres, not the key arithmetic: two loopback
    /// backends must hold admission **at the same time**, and a second caller
    /// on one of them must not. Advisory locks are database-wide rather than
    /// schema-scoped, so this needs no test schema — but it does need the same
    /// live database every other store test needs.
    ///
    /// The self-contention half is checked on a *third* backend name unique to
    /// this test, so a concurrently running comms-server holding the real
    /// `omlx` lock cannot turn this green or red by accident.
    #[test]
    fn afm_and_omlx_hold_admission_at_the_same_time() {
        let url = test_database_url();
        let afm = AdvisoryGate::new(&url, "foundation-models");
        let omlx = AdvisoryGate::new(&url, "omlx-gate-test");

        let held_afm = afm.acquire().unwrap_or_else(|error| {
            panic!(
                "could not take the AFM gate: {error} — needs capabilities/postgres running \
                 and AXON_PERSONAL_ROOT exported (or COMMS_TEST_DATABASE_URL set)"
            )
        });
        let held_omlx = omlx
            .acquire()
            .expect("a second backend must not queue behind the first");

        // Same backend, different gate object: this is the cross-process case,
        // and it must still be refused. `pg_try_advisory_lock` is per session,
        // and each gate opens its own, so this is a genuine second holder.
        let mut second_conn = Client::connect(&url, NoTls).expect("a second session");
        let got_again: bool = second_conn
            .query_one(
                "SELECT pg_try_advisory_lock($1)",
                &[&lock_key("omlx-gate-test")],
            )
            .expect("try_advisory_lock answers")
            .get(0);
        assert!(
            !got_again,
            "one backend must still admit only one request at a time"
        );

        drop(held_afm);
        drop(held_omlx);

        // And it is released, not leaked: the next caller gets straight in.
        let after: bool = second_conn
            .query_one(
                "SELECT pg_try_advisory_lock($1)",
                &[&lock_key("omlx-gate-test")],
            )
            .expect("try_advisory_lock answers")
            .get(0);
        assert!(after, "the lock survived the request that held it");
        let _ = second_conn.execute(
            "SELECT pg_advisory_unlock($1)",
            &[&lock_key("omlx-gate-test")],
        );
    }
}
