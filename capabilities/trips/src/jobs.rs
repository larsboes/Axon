//! The in-process job map behind `POST /api/plan-search`.
//!
//! Ported from `capabilities/interior/src/api.rs` (`Auftraege`), with its
//! reasons kept because they are the reasons here too:
//!
//! - **Nothing is evicted while it runs.** A running job still has its writer
//!   in `spawn_blocking`. Dropping the entry would leave that writer writing
//!   into nothing and would answer 404 for a search that ran for minutes.
//! - **The eleventh search is refused, not dropped.** Ten concurrent searches
//!   is a load nobody asked for; a refusal with a sentence is honest, a silent
//!   drop is not.
//! - **Elapsed time is computed on read.** A job that had to count its own
//!   runtime would need a second thread for nothing.
//!
//! Deliberately no table. A ranked option space is a list of proposals, not a
//! fact about a trip, so it may die with the process; what is worth keeping
//! becomes durable through `POST /api/plan-search/:id/adopt`, which writes one
//! `option_set` row. The result is a §6.2 derived aggregate, C1: it holds a
//! companion COUNT and never a register row, and it is never projected.

use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use serde::Serialize;

use crate::plan_search::PlanSearchResult;

/// How many finished jobs are kept. Ten, because the surface fetches the last
/// one and the ones before it are history.
pub const MAX_JOBS: usize = 10;

/// The wall-clock budget one search may spend, measured from job start.
///
/// Eight candidates at up to 5 s of station resolution plus 20 s of fare search
/// each is a 200 s worst case, which is above any number worth promising. The
/// job stops pricing when the budget is spent, still finishes `done`, and says
/// so: `degraded` carries the pricing-budget sentence and `priced` is below
/// `considered`. The same honesty `unscored_legs` already uses in transit.
pub const JOB_DEADLINE_S: u64 = 180;

/// What the eleventh caller is told.
pub const ALL_JOBS_BUSY: &str = "ten plan searches are already running — wait for one to finish";

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum JobState {
    Running {
        since_ms: u128,
    },
    /// Boxed because a finished result is far larger than the other two
    /// variants and clippy measures the enum by its widest one.
    Done {
        result: Box<PlanSearchResult>,
    },
    /// Serialized as `error`, which is the failure field every other body in
    /// this repository uses and the one the dashboard client unwraps.
    Failed {
        #[serde(rename = "error")]
        reason: String,
    },
}

#[derive(Default)]
pub struct Jobs {
    next: u64,
    entries: BTreeMap<u64, (Instant, JobState)>,
}

impl Jobs {
    /// Make room, take the next number — or refuse.
    ///
    /// Only a finished job is evicted, and the keys ascend, so the first
    /// evictable entry is also the oldest.
    pub fn open(&mut self) -> Result<u64, String> {
        while self.entries.len() >= MAX_JOBS {
            let Some(old) = self
                .entries
                .iter()
                .find(|(_, (_, state))| !matches!(state, JobState::Running { .. }))
                .map(|(id, _)| *id)
            else {
                return Err(ALL_JOBS_BUSY.to_string());
            };
            self.entries.remove(&old);
        }
        // Numbers start at 1, so a job id is never the falsy 0 in a URL or a
        // JSON body a browser reads.
        self.next += 1;
        let id = self.next;
        self.entries
            .insert(id, (Instant::now(), JobState::Running { since_ms: 0 }));
        Ok(id)
    }

    pub fn finish(&mut self, id: u64, outcome: Result<PlanSearchResult, String>) {
        if let Some((_, state)) = self.entries.get_mut(&id) {
            *state = match outcome {
                Ok(result) => JobState::Done {
                    result: Box::new(result),
                },
                Err(reason) => JobState::Failed { reason },
            };
        }
    }

    /// The state as a reader sees it, with a running job's elapsed time filled
    /// in at read time.
    pub fn read(&self, id: u64) -> Option<JobState> {
        let (started, state) = self.entries.get(&id)?;
        Some(match state {
            JobState::Running { .. } => JobState::Running {
                since_ms: started.elapsed().as_millis(),
            },
            finished => finished.clone(),
        })
    }

    /// How long job `id` has been running. `None` for an unknown job.
    pub fn started_at(&self, id: u64) -> Option<Instant> {
        self.entries.get(&id).map(|(started, _)| *started)
    }
}

pub fn jobs() -> &'static Mutex<Jobs> {
    static JOBS: OnceLock<Mutex<Jobs>> = OnceLock::new();
    JOBS.get_or_init(|| Mutex::new(Jobs::default()))
}

/// Start a search in the background and hand back its number immediately.
///
/// `spawn_blocking`, because the composition makes blocking HTTP calls that can
/// take minutes: on an async thread that would stall every other request this
/// process serves. The same reason `flight_when` and interior's search use it.
pub fn start<F>(work: F) -> Result<u64, String>
where
    F: FnOnce(Instant) -> Result<PlanSearchResult, String> + Send + 'static,
{
    let id = jobs().lock().expect("job map").open()?;
    let started = jobs()
        .lock()
        .expect("job map")
        .started_at(id)
        .unwrap_or_else(Instant::now);
    tokio::task::spawn_blocking(move || {
        let outcome = work(started);
        jobs().lock().expect("job map").finish(id, outcome);
    });
    Ok(id)
}

pub fn read(id: u64) -> Option<JobState> {
    jobs().lock().expect("job map").read(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn done() -> Result<PlanSearchResult, String> {
        Err("test".into())
    }

    /// Interior's `ein_laufender_auftrag_wird_nicht_verdraengt`, ported: the
    /// map fills with running jobs and none of them is thrown away to make
    /// room, because each still has a writer behind it.
    #[test]
    fn a_running_job_is_never_evicted() {
        let mut jobs = Jobs::default();
        let first = jobs.open().expect("the first job fits");
        for _ in 1..MAX_JOBS {
            jobs.open().expect("ten jobs fit");
        }
        assert!(
            jobs.read(first).is_some(),
            "the first running job was evicted to make room"
        );
        assert!(matches!(jobs.read(first), Some(JobState::Running { .. })));
    }

    /// The refusal is an error carrying a sentence, not a silently dropped
    /// search.
    #[test]
    fn the_eleventh_search_is_refused_while_ten_run() {
        let mut jobs = Jobs::default();
        for _ in 0..MAX_JOBS {
            jobs.open().expect("ten jobs fit");
        }
        let refused = jobs.open().expect_err("the eleventh must be refused");
        assert_eq!(refused, ALL_JOBS_BUSY);

        // Finishing one makes room again, and the finished one is what goes.
        jobs.finish(1, done());
        let eleventh = jobs.open().expect("a finished job makes room");
        assert!(jobs.read(1).is_none(), "the finished job should be evicted");
        assert!(jobs.read(eleventh).is_some());
    }

    /// A number a reader can tell apart from "no job": ids ascend from 1.
    #[test]
    fn job_numbers_start_at_one() {
        let mut jobs = Jobs::default();
        assert_eq!(jobs.open().unwrap(), 1);
        assert_eq!(jobs.open().unwrap(), 2);
    }
}
