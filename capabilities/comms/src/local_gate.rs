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

/// Identifies this lock among all advisory locks in the database. Arbitrary but
/// fixed: any two processes must choose the same number or they do not contend,
/// and the whole point is that they contend.
///
/// Advisory-lock keys share one namespace per database, so this is deliberately
/// not a small round number that another subsystem might reach for.
const LOCAL_INFERENCE_LOCK_KEY: i64 = 0x41_58_4F_4E_5F_47_50_55; // "AXON_GPU"

/// How long a caller waits before being told the machine is busy.
///
/// Sized against the work, not the impatience: a sectioned digest of a long
/// transcript takes 12-20s on this machine, and a queue two deep is normal
/// during a drain. Much shorter and a drain would spend its passes reporting
/// contention; much longer and an operator's press hangs behind a backlog.
const WAIT_TIMEOUT: Duration = Duration::from_secs(90);

/// One Postgres advisory lock, held for the duration of a local request.
pub struct AdvisoryGate {
    database_url: String,
}

impl AdvisoryGate {
    pub fn new(database_url: impl Into<String>) -> Self {
        Self {
            database_url: database_url.into(),
        }
    }

    /// Wrap in the `Arc` that `Target::gate` wants.
    pub fn shared(database_url: impl Into<String>) -> Arc<dyn LocalGate> {
        Arc::new(Self::new(database_url))
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
        let deadline = Instant::now() + WAIT_TIMEOUT;
        loop {
            let got: bool = client
                .query_one(
                    "SELECT pg_try_advisory_lock($1)",
                    &[&LOCAL_INFERENCE_LOCK_KEY],
                )
                .map_err(|error| format!("local inference gate: {error}"))?
                .get(0);
            if got {
                break;
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "another local inference request held the machine for more than {}s",
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
            let _ = client.execute(
                "SELECT pg_advisory_unlock($1)",
                &[&LOCAL_INFERENCE_LOCK_KEY],
            );
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

    /// The key is what makes two processes contend. Pinned to its derivation
    /// rather than to a copy of itself, so this fails if someone retypes the
    /// literal wrong — a gate whose processes each picked a different number
    /// guards nothing, and looks exactly like one that works.
    #[test]
    fn the_lock_key_is_the_ascii_of_axon_gpu() {
        assert_eq!(
            LOCAL_INFERENCE_LOCK_KEY,
            i64::from_be_bytes(*b"AXON_GPU"),
            "the advisory key must stay byte-identical across every process"
        );
        const {
            assert!(
                LOCAL_INFERENCE_LOCK_KEY > 0,
                "a negative key still works but reads as a mistake in pg_locks"
            );
        }
    }
}
