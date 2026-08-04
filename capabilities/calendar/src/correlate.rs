//! Phase C: does a candidate date survive the operator's real calendar?
//!
//! Pure over a slice of entries — the caller hands in the window, this module
//! never queries and never reads another capability's store. Verdicts are
//! soft (README's "No hard filter on conflicts"): the layer explains, the
//! operator decides. Nothing here filters a candidate out.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::date;
use crate::model::{Commitment, Entry};

/// The three verdicts of the README's Correlation contract, ordered by how
/// much they cost the operator. `Ord` is what makes a day's verdict the worst
/// of its overlapping entries, and a window's verdict the worst of its days.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Feasibility {
    Free,
    NeedsTravelDay,
    Conflicts,
}

/// The most an entry at this commitment is allowed to cost, whatever it is.
///
/// This is the whole reason the axis exists. An Impact Lab in Atlanta that
/// scouting merely *found* used to reach `Conflicts` through `kind = "event"`
/// and quietly delete two days from August. A thing you have not decided on
/// cannot cost you anything, so `Possible` caps at `Free`.
fn commitment_ceiling(commitment: Commitment) -> Feasibility {
    match commitment {
        Commitment::Possible => Feasibility::Free,
        Commitment::Planned => Feasibility::NeedsTravelDay,
        Commitment::Committed => Feasibility::Conflicts,
    }
}

/// What one entry does to a candidate that overlaps it.
///
/// Two axes, and the cheaper one wins: `kind` says how much this *could* cost,
/// `commitment` caps how much it is *allowed* to. Taking the min is what keeps
/// the enabling kinds honest — a planned `travel_ok` is still free, because
/// planning to be up for a trip was never a cost.
///
/// Deliberately total on the kind side: a kind this layer has never seen
/// returns `Free`. That is the other half of "kinds are data, not a
/// constraint" — a day-planning kind added later lands without a migration
/// *and* without silently blocking travel on every day it covers.
pub fn impact(kind: &str, commitment: Commitment) -> Feasibility {
    kind_ceiling(kind).min(commitment_ceiling(commitment))
}

/// What this kind costs when it is actually happening.
fn kind_ceiling(kind: &str) -> Feasibility {
    match kind {
        // You are somewhere else, or already booked into something concrete
        // on that day. The contract's free clause ("or only travel_ok/event")
        // is the candidate's *own* already-promoted entry, which never reaches
        // this function — see `Verdict::already_in_calendar`. Any *other*
        // committed event is a real clash: you cannot attend both.
        "away" | "event" | "nightlife" => Feasibility::Conflicts,
        // Movable, so still offered with the cost named: on-site work can go
        // remote, a busy block can be dropped.
        "work_onsite" | "busy" => Feasibility::NeedsTravelDay,
        // work_remote is location-flexible by definition, travel_ok is the
        // explicit yes. Both stay in the evidence so the UI can mention them;
        // neither raises the verdict. Unknown kinds land here too — including
        // the imports that used to be spelled `kind = "draft"`, which are now
        // ordinary kinds at `Commitment::Possible`.
        _ => Feasibility::Free,
    }
}

/// A date range to correlate.
///
/// `id` is the caller's own key, echoed back on the verdict and used to
/// recognize the candidate's own already-promoted entry. Either key works: the
/// provider's (what promotion writes to `external_id`) or the scouting
/// opportunity's (what promotion records in `payload.opportunity_id`). Both
/// have to, because each side holds a different one for good reason — see
/// `is_same_thing`.
///
/// Calendar does not read the scouting store; the caller hands the dates in,
/// the same contribution boundary `PUT /api/entries/external` uses.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Candidate {
    pub id: String,
    pub starts_at: String,
    #[serde(default)]
    pub ends_at: Option<String>,
}

/// One entry that overlapped, with what it did to the verdict. The UI reads
/// this to say "you have on-site work that day" instead of showing a badge.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Evidence {
    pub entry_id: String,
    pub kind: String,
    /// Carried so the UI can say *why* something is weightless — "you noted
    /// this, you didn't book it" reads very differently from "this is free".
    pub commitment: Commitment,
    pub title: String,
    pub starts_at: String,
    pub ends_at: String,
    pub all_day: bool,
    pub impact: Feasibility,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Verdict {
    pub id: String,
    pub verdict: Feasibility,
    /// The range actually checked, after normalization — a caller that sent a
    /// provider timestamp can see what it was read as.
    pub starts_at: String,
    pub ends_at: String,
    /// An entry with this candidate's `external_id` already exists (Phase A's
    /// "in Kalender übernehmen" promoted it). It is excluded from the verdict:
    /// correlating an opportunity against itself would make every saved
    /// opportunity conflict with itself.
    pub already_in_calendar: bool,
    /// Strongest impact first. Includes neutral overlaps — they explain the
    /// day even when they do not drive the verdict.
    pub evidence: Vec<Evidence>,
}

/// A maximal run of days where travel is possible at all: every day whose
/// verdict is not `conflicts`. The soft cost is named, not filtered —
/// `days_needing_travel_day` lists the days inside the run that would cost a
/// remote day or a dropped block, and `verdict` is the worst day in the run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FeasibleWindow {
    pub starts_on: String,
    /// Exclusive, like every other end in this capability.
    pub ends_before: String,
    /// Every day in the run, in order — the shape transit's `plan --dates`
    /// takes, so the hand-off needs no date arithmetic on the way over.
    pub days: Vec<String>,
    pub verdict: Feasibility,
    pub days_needing_travel_day: Vec<String>,
}

const MINUTES_PER_DAY: i64 = 1440;

/// Provider instants arrive in whatever shape the source emits — Luma's
/// `2026-07-10T18:00:00.000Z`, euro-hackathons' `2026-10-23T00:00:00+00:00`,
/// transit's naive `2026-08-01T08:00:00`. The zone designator and any
/// fractional seconds are dropped and the wall clock is read as local: that is
/// the single-home-timezone call the README's time model already makes, and
/// nothing here converts anything. Phase E (Google sync) is where offsets
/// become real.
fn normalize_instant(text: &str) -> Option<String> {
    let text = text.trim();
    let Some((day, time)) = text.split_once('T') else {
        // Date-only: strict, exactly as the store stores it.
        return date::parse_date(text).map(|_| text.to_string());
    };
    // A '-' can only be an offset sign here — the date half is already split
    // off — so cutting at the first of these leaves the bare wall time.
    let cut = time
        .find(|c: char| matches!(c, 'Z' | 'z' | '+' | '-' | '.'))
        .unwrap_or(time.len());
    let normalized = format!("{day}T{}", &time[..cut]);
    date::parse_instant(&normalized).map(|_| normalized)
}

struct Resolved {
    start: i64,
    end: i64,
    starts_at: String,
    ends_at: String,
}

/// Resolves a candidate to a half-open `[start, end)` minute range.
///
/// A missing end, or an end at or before the start (a provider that emits one
/// instant twice, which euro-hackathons does for single-day entries), becomes
/// the whole start day. An event without an end is a day, not an instant —
/// and an empty range would overlap nothing and report `free` for a day the
/// operator is demonstrably away.
fn resolve(candidate: &Candidate) -> Result<Resolved, String> {
    let starts_at = normalize_instant(&candidate.starts_at).ok_or_else(|| {
        format!(
            "candidate {}: starts_at is not a date or local time: {}",
            candidate.id, candidate.starts_at
        )
    })?;
    let start = date::instant_minutes(&starts_at)
        .ok_or_else(|| format!("candidate {}: unreadable starts_at", candidate.id))?;

    let end = match candidate
        .ends_at
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        Some(raw) => {
            let ends_at = normalize_instant(raw).ok_or_else(|| {
                format!(
                    "candidate {}: ends_at is not a date or local time: {raw}",
                    candidate.id
                )
            })?;
            let end = date::instant_minutes(&ends_at)
                .ok_or_else(|| format!("candidate {}: unreadable ends_at", candidate.id))?;
            (end > start).then_some((end, ends_at))
        }
        None => None,
    };

    Ok(match end {
        Some((end, ends_at)) => Resolved {
            start,
            end,
            starts_at,
            ends_at,
        },
        None => {
            let day = start.div_euclid(MINUTES_PER_DAY);
            Resolved {
                start: day * MINUTES_PER_DAY,
                end: (day + 1) * MINUTES_PER_DAY,
                starts_at: date::format_date(day),
                ends_at: date::format_date(day + 1),
            }
        }
    })
}

/// An entry's `[start, end)` in minutes. An unreadable instant is an error,
/// not a skip: the store validates every write, so a row that fails here is
/// corruption, and a verdict computed as if the entry were not there would be
/// confidently wrong about the operator's own time.
fn entry_range(entry: &Entry) -> Result<(i64, i64), String> {
    let start = date::instant_minutes(&entry.starts_at)
        .ok_or_else(|| format!("entry {}: unreadable starts_at {}", entry.id, entry.starts_at))?;
    let end = date::instant_minutes(&entry.ends_at)
        .ok_or_else(|| format!("entry {}: unreadable ends_at {}", entry.id, entry.ends_at))?;
    Ok((start, end))
}

/// Half-open overlap. Ends are exclusive everywhere in this capability, so an
/// entry ending 2026-08-15 does not touch an event on 2026-08-15.
fn overlaps(a: (i64, i64), b: (i64, i64)) -> bool {
    a.0 < b.1 && b.0 < a.1
}

/// Is this entry the candidate itself, already promoted?
///
/// Two ids can name the same thing, and both are legitimate. `external_id` is
/// the *provider's* key — the bare `evt-…` Luma id — deliberately so, because
/// a second path to the same event (an ICS import rather than the JSON scrape)
/// produces that same key and must dedupe onto the same row. But the caller
/// asking for a verdict is Feed's Entdecken view, and what it holds is the
/// *scouting opportunity* id, `evt:luma:evt-…`. Matching only `external_id`
/// meant a saved opportunity came back `already_in_calendar: false` and
/// `conflicts` — with itself, because its own promoted entry is `kind = event`.
/// Verified against live data 2026-07-30: the bare id matched, the namespaced
/// one did not.
///
/// Promotion writes the opportunity id into `payload.opportunity_id`, so the
/// link exists; this reads it. Neither side had to give up its own key.
fn is_same_thing(entry: &Entry, candidate_id: &str) -> bool {
    if entry.external_id.as_deref() == Some(candidate_id) {
        return true;
    }
    entry
        .payload
        .get("opportunity_id")
        .and_then(Value::as_str)
        == Some(candidate_id)
}

/// The verdict for one candidate against the entries of a window that covers
/// it. Entries outside the candidate's range are ignored, so passing a wider
/// window than needed is free.
pub fn verdict_for(candidate: &Candidate, entries: &[Entry]) -> Result<Verdict, String> {
    let range = resolve(candidate)?;
    let mut verdict = Feasibility::Free;
    let mut already_in_calendar = false;
    let mut evidence = Vec::new();

    for entry in entries {
        if is_same_thing(entry, &candidate.id) {
            already_in_calendar = true;
            continue;
        }
        let entry_range = entry_range(entry)?;
        if !overlaps((range.start, range.end), entry_range) {
            continue;
        }
        let entry_impact = impact(&entry.kind, entry.commitment);
        verdict = verdict.max(entry_impact);
        evidence.push(Evidence {
            entry_id: entry.id.clone(),
            kind: entry.kind.clone(),
            commitment: entry.commitment,
            title: entry.title.clone(),
            starts_at: entry.starts_at.clone(),
            ends_at: entry.ends_at.clone(),
            all_day: entry.all_day,
            impact: entry_impact,
        });
    }

    evidence.sort_by(|a, b| {
        b.impact
            .cmp(&a.impact)
            .then_with(|| a.starts_at.cmp(&b.starts_at))
    });

    Ok(Verdict {
        id: candidate.id.clone(),
        verdict,
        starts_at: range.starts_at,
        ends_at: range.ends_at,
        already_in_calendar,
        evidence,
    })
}

/// Verdicts for one caller-supplied batch, in the caller's order.
///
/// The HTTP edge reads one Calendar window for the whole batch and delegates
/// here. Keeping the loop beside `verdict_for` makes the served batch contract
/// testable without a database or an Axum router.
pub fn verdicts_for(candidates: &[Candidate], entries: &[Entry]) -> Result<Vec<Verdict>, String> {
    candidates
        .iter()
        .map(|candidate| verdict_for(candidate, entries))
        .collect()
}

/// The day window a candidate set needs read out of the store, as the
/// `from`/`to` pair `CalendarStore::list_entries` takes (from inclusive, to
/// exclusive). `None` for an empty set — no candidates, no query.
pub fn query_window(candidates: &[Candidate]) -> Result<Option<(String, String)>, String> {
    let mut bounds: Option<(i64, i64)> = None;
    for candidate in candidates {
        let range = resolve(candidate)?;
        let first = range.start.div_euclid(MINUTES_PER_DAY);
        // One day past the exclusive end: an end mid-day still needs that
        // day's entries, and over-fetching a day costs nothing because
        // `verdict_for` filters on the exact range anyway.
        let last = range.end.div_euclid(MINUTES_PER_DAY) + 1;
        bounds = Some(match bounds {
            Some((lo, hi)) => (lo.min(first), hi.max(last)),
            None => (first, last),
        });
    }
    Ok(bounds.map(|(lo, hi)| (date::format_date(lo), date::format_date(hi))))
}

/// Groups `[from_day, to_day)` into the runs of days where travel is possible
/// at all. `min_days` drops runs shorter than that — transit's fare search
/// asks for at least a weekend, and searching a single feasible Tuesday
/// between two `away` blocks is a wasted HAFAS call.
pub fn feasible_windows(
    from_day: i64,
    to_day: i64,
    entries: &[Entry],
    min_days: usize,
) -> Result<Vec<FeasibleWindow>, String> {
    let mut windows = Vec::new();
    let mut run: Vec<(String, Feasibility)> = Vec::new();

    for day in from_day..to_day {
        let range = (day * MINUTES_PER_DAY, (day + 1) * MINUTES_PER_DAY);
        let mut impact = Feasibility::Free;
        for entry in entries {
            if overlaps(range, entry_range(entry)?) {
                impact = impact.max(self::impact(&entry.kind, entry.commitment));
            }
        }
        if impact == Feasibility::Conflicts {
            close_run(&mut windows, &mut run, day, min_days);
        } else {
            run.push((date::format_date(day), impact));
        }
    }
    close_run(&mut windows, &mut run, to_day, min_days);
    Ok(windows)
}

fn close_run(
    windows: &mut Vec<FeasibleWindow>,
    run: &mut Vec<(String, Feasibility)>,
    ends_before_day: i64,
    min_days: usize,
) {
    if run.len() >= min_days.max(1) {
        windows.push(FeasibleWindow {
            starts_on: run[0].0.clone(),
            ends_before: date::format_date(ends_before_day),
            days: run.iter().map(|(day, _)| day.clone()).collect(),
            verdict: run
                .iter()
                .map(|(_, impact)| *impact)
                .max()
                .unwrap_or(Feasibility::Free),
            days_needing_travel_day: run
                .iter()
                .filter(|(_, impact)| *impact == Feasibility::NeedsTravelDay)
                .map(|(day, _)| day.clone())
                .collect(),
        });
    }
    run.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    /// A real entry in the calendar. Defaults to `Committed` because that is
    /// what every test written before the axis existed meant by "there is an
    /// away block on the 14th" — use `entry_at` to vary the commitment.
    fn entry(kind: &str, starts_at: &str, ends_at: &str) -> Entry {
        entry_at(kind, Commitment::Committed, starts_at, ends_at)
    }

    fn entry_at(kind: &str, commitment: Commitment, starts_at: &str, ends_at: &str) -> Entry {
        let all_day = !starts_at.contains('T');
        Entry {
            id: format!("cal:entry:{kind}-{starts_at}"),
            kind: kind.into(),
            commitment,
            title: format!("{kind} block"),
            starts_at: starts_at.into(),
            ends_at: ends_at.into(),
            all_day,
            location: None,
            notes: None,
            source: "manual".into(),
            external_id: None,
            rhythm_id: None,
            payload: Value::Null,
            created_at: "0".into(),
            updated_at: "0".into(),
        }
    }

    fn candidate(starts_at: &str, ends_at: Option<&str>) -> Candidate {
        Candidate {
            id: "opp:munich-meetup".into(),
            starts_at: starts_at.into(),
            ends_at: ends_at.map(str::to_string),
        }
    }

    fn verdict(candidate: &Candidate, entries: &[Entry]) -> Feasibility {
        verdict_for(candidate, entries).unwrap().verdict
    }

    #[test]
    fn empty_calendar_is_free() {
        let day = candidate("2026-08-14", Some("2026-08-15"));
        assert_eq!(verdict(&day, &[]), Feasibility::Free);
    }

    #[test]
    fn travel_ok_and_remote_work_stay_free_but_are_still_evidence() {
        let day = candidate("2026-08-14", Some("2026-08-15"));
        let entries = [
            entry("travel_ok", "2026-08-10", "2026-08-20"),
            entry("work_remote", "2026-08-14T09:00", "2026-08-14T17:00"),
        ];
        let result = verdict_for(&day, &entries).unwrap();
        assert_eq!(result.verdict, Feasibility::Free);
        assert_eq!(
            result.evidence.len(),
            2,
            "a free verdict still explains the day"
        );
    }

    #[test]
    fn onsite_work_and_busy_need_a_travel_day() {
        let day = candidate("2026-08-14", Some("2026-08-15"));
        assert_eq!(
            verdict(&day, &[entry("work_onsite", "2026-08-14", "2026-08-15")]),
            Feasibility::NeedsTravelDay
        );
        assert_eq!(
            verdict(&day, &[entry("busy", "2026-08-14T09:00", "2026-08-14T10:00")]),
            Feasibility::NeedsTravelDay
        );
    }

    #[test]
    fn away_and_a_confirmed_event_conflict() {
        let day = candidate("2026-08-14", Some("2026-08-15"));
        assert_eq!(
            verdict(&day, &[entry("away", "2026-08-12", "2026-08-16")]),
            Feasibility::Conflicts
        );
        assert_eq!(
            verdict(&day, &[entry("event", "2026-08-14T18:00", "2026-08-14T22:00")]),
            Feasibility::Conflicts
        );
    }

    #[test]
    fn the_worst_overlapping_entry_wins_and_leads_the_evidence() {
        let day = candidate("2026-08-14", Some("2026-08-15"));
        let entries = [
            entry("work_remote", "2026-08-14T08:00", "2026-08-14T09:00"),
            entry("away", "2026-08-14", "2026-08-15"),
            entry("busy", "2026-08-14T10:00", "2026-08-14T11:00"),
        ];
        let result = verdict_for(&day, &entries).unwrap();
        assert_eq!(result.verdict, Feasibility::Conflicts);
        assert_eq!(result.evidence[0].kind, "away");
        assert_eq!(result.evidence.len(), 3);
    }

    #[test]
    fn an_unconfirmed_google_draft_explains_a_day_without_blocking_it() {
        // Phase E's whole safety property, at the layer that would enforce a
        // block: an all-day import from Google must not make the day
        // infeasible before the operator has looked at it.
        let day = candidate("2026-08-14", Some("2026-08-15"));
        let mut draft = entry("draft", "2026-08-14", "2026-08-15");
        draft.source = "google".into();
        draft.external_id = Some("3q7l9v1c8m2p4t6y0x5z".into());

        let result = verdict_for(&day, &[draft.clone()]).unwrap();
        assert_eq!(result.verdict, Feasibility::Free);
        assert_eq!(result.evidence.len(), 1, "it still explains the day");

        // ...and the moment the operator confirms it into an `event`, it does.
        let mut confirmed = draft;
        confirmed.kind = "event".into();
        assert_eq!(verdict(&day, &[confirmed]), Feasibility::Conflicts);
    }

    #[test]
    fn an_unknown_kind_is_neutral_never_a_conflict() {
        let day = candidate("2026-08-14", Some("2026-08-15"));
        // Phase F's day-planning blocks land as kinds this layer never saw.
        let entries = [entry("deep_work", "2026-08-14", "2026-08-15")];
        let result = verdict_for(&day, &entries).unwrap();
        assert_eq!(result.verdict, Feasibility::Free);
        assert_eq!(result.evidence[0].impact, Feasibility::Free);
        assert_eq!(
            impact("something_invented_in_2027", Commitment::Committed),
            Feasibility::Free
        );
    }

    /// The full 3 x 7 matrix, written out rather than derived, so that a
    /// change to either ceiling has to be argued cell by cell.
    #[test]
    fn impact_is_the_cheaper_of_the_two_ceilings() {
        use Commitment::{Committed, Planned, Possible};
        use Feasibility::{Conflicts, Free, NeedsTravelDay as Ntd};

        let cases: &[(&str, Feasibility, Feasibility, Feasibility)] = &[
            // kind            possible  planned  committed
            ("busy", Free, Ntd, Ntd),
            ("work_onsite", Free, Ntd, Ntd),
            ("work_remote", Free, Free, Free),
            ("away", Free, Ntd, Conflicts),
            ("event", Free, Ntd, Conflicts),
            ("travel_ok", Free, Free, Free),
            ("something_invented_in_2027", Free, Free, Free),
        ];

        for (kind, possible, planned, committed) in cases {
            assert_eq!(impact(kind, Possible), *possible, "{kind} @ possible");
            assert_eq!(impact(kind, Planned), *planned, "{kind} @ planned");
            assert_eq!(impact(kind, Committed), *committed, "{kind} @ committed");
        }
    }

    /// The cell a commitment-first rule gets wrong: planning to be available
    /// is not a cost, so it must not raise the verdict.
    #[test]
    fn planning_to_be_available_costs_nothing() {
        assert_eq!(impact("travel_ok", Commitment::Planned), Feasibility::Free);
        assert_eq!(
            impact("work_remote", Commitment::Planned),
            Feasibility::Free
        );
    }

    /// The Atlanta regression. An event scouting merely found, on a day you
    /// are otherwise free, must leave the day free — it used to reach
    /// `Conflicts` through `kind = "event"` and delete the day from August.
    #[test]
    fn a_merely_discovered_event_never_blocks_a_day() {
        let day = candidate("2026-08-04", Some("2026-08-05"));
        let found = [entry_at(
            "event",
            Commitment::Possible,
            "2026-08-04T23:00:00",
            "2026-08-05T03:00:00",
        )];
        assert_eq!(verdict(&day, &found), Feasibility::Free);

        // ...and the same event, once he has actually registered, does block.
        let booked = [entry_at(
            "event",
            Commitment::Committed,
            "2026-08-04T23:00:00",
            "2026-08-05T03:00:00",
        )];
        assert_eq!(verdict(&day, &booked), Feasibility::Conflicts);
    }

    #[test]
    fn ends_are_exclusive_at_the_day_boundary() {
        let day = candidate("2026-08-15", Some("2026-08-16"));
        // Ends 2026-08-15, so it covers the 14th and stops.
        let entries = [entry("away", "2026-08-13", "2026-08-15")];
        assert_eq!(verdict(&day, &entries), Feasibility::Free);

        let starts_that_day = [entry("away", "2026-08-15", "2026-08-16")];
        assert_eq!(verdict(&day, &starts_that_day), Feasibility::Conflicts);
    }

    #[test]
    fn ends_are_exclusive_at_the_midnight_boundary_too() {
        // The case tuple comparison gets wrong: an all-day entry's start reads
        // as "no time", which sorts *before* an explicit T00:00 end.
        let all_day = candidate("2026-08-14", Some("2026-08-15"));
        let ends_at_midnight = [entry("away", "2026-08-13T22:00", "2026-08-14T00:00")];
        assert_eq!(verdict(&all_day, &ends_at_midnight), Feasibility::Free);

        let starts_at_midnight = [entry("away", "2026-08-14T00:00", "2026-08-14T01:00")];
        assert_eq!(verdict(&all_day, &starts_at_midnight), Feasibility::Conflicts);
    }

    #[test]
    fn all_day_entries_overlap_timed_candidates_and_the_reverse() {
        let evening = candidate("2026-08-14T18:00", Some("2026-08-14T22:00"));
        let all_day_block = [entry("work_onsite", "2026-08-14", "2026-08-15")];
        assert_eq!(verdict(&evening, &all_day_block), Feasibility::NeedsTravelDay);

        // A timed block that ends before the candidate starts does not.
        let morning_block = [entry("work_onsite", "2026-08-14T09:00", "2026-08-14T17:00")];
        assert_eq!(verdict(&evening, &morning_block), Feasibility::Free);

        // ...and the all-day candidate sees that same morning block.
        let whole_day = candidate("2026-08-14", Some("2026-08-15"));
        assert_eq!(verdict(&whole_day, &morning_block), Feasibility::NeedsTravelDay);
    }

    #[test]
    fn the_candidates_own_promoted_entry_never_conflicts_with_it() {
        let day = candidate("2026-08-14", Some("2026-08-15"));
        let mut promoted = entry("event", "2026-08-14", "2026-08-15");
        promoted.source = "scouting".into();
        promoted.external_id = Some("opp:munich-meetup".into());

        let result = verdict_for(&day, &[promoted.clone()]).unwrap();
        assert_eq!(result.verdict, Feasibility::Free);
        assert!(result.already_in_calendar);
        assert!(result.evidence.is_empty());

        // A *different* confirmed event on the same day is still a conflict.
        let mut other = promoted;
        other.external_id = Some("opp:some-other-thing".into());
        let result = verdict_for(&day, &[other]).unwrap();
        assert_eq!(result.verdict, Feasibility::Conflicts);
        assert!(!result.already_in_calendar);
    }

    #[test]
    fn a_promoted_entry_is_recognised_by_its_opportunity_id_too() {
        // The seam between scouting's promotion and this layer, found live on
        // 2026-07-30. Promotion stores the *provider's* key in `external_id`
        // (bare `evt-…`) so an ICS import of the same event dedupes onto the
        // same row — but Feed's Entdecken view asks with the *opportunity* id
        // (`evt:luma:evt-…`). Matching only `external_id` made a saved
        // opportunity conflict with itself.
        let mut promoted = entry("event", "2026-08-14", "2026-08-15");
        promoted.source = "luma".into();
        promoted.external_id = Some("evt-E8mj424DVKBXFb4".into());
        promoted.payload = json!({ "opportunity_id": "evt:luma:evt-E8mj424DVKBXFb4" });

        for asked_as in ["evt-E8mj424DVKBXFb4", "evt:luma:evt-E8mj424DVKBXFb4"] {
            let mut c = candidate("2026-08-14", Some("2026-08-15"));
            c.id = asked_as.into();
            let result = verdict_for(&c, &[promoted.clone()]).unwrap();
            assert!(
                result.already_in_calendar,
                "asked as {asked_as}: the entry that IS this candidate was not recognised"
            );
            assert_eq!(result.verdict, Feasibility::Free);
        }

        // A different Luma event on the same day is still a real clash.
        let mut other = candidate("2026-08-14", Some("2026-08-15"));
        other.id = "evt:luma:evt-SOMETHING-ELSE".into();
        let result = verdict_for(&other, &[promoted]).unwrap();
        assert!(!result.already_in_calendar);
        assert_eq!(result.verdict, Feasibility::Conflicts);
    }

    #[test]
    fn provider_timestamps_are_read_as_local_wall_time() {
        // Luma, euro-hackathons and transit_fare all emit a different shape;
        // every one of them means 18:00 on the operator's own clock here.
        let expected = date::instant_minutes("2026-08-14T18:00").unwrap();
        for raw in [
            "2026-08-14T18:00:00.000Z",
            "2026-08-14T18:00:00+02:00",
            "2026-08-14T18:00:00-04:00",
            "2026-08-14T18:00:00",
            "2026-08-14T18:00",
        ] {
            let normalized = normalize_instant(raw).expect(raw);
            assert_eq!(
                date::instant_minutes(&normalized),
                Some(expected),
                "{raw} normalized to {normalized}, which is not 18:00 local"
            );
        }
        assert_eq!(
            normalize_instant("2026-08-14").as_deref(),
            Some("2026-08-14")
        );
        assert!(normalize_instant("2026-08-14 18:00").is_none());
        assert!(normalize_instant("next tuesday").is_none());
    }

    #[test]
    fn a_candidate_without_an_end_is_the_whole_start_day() {
        let evening = candidate("2026-08-14T18:00:00Z", None);
        let result = verdict_for(&evening, &[]).unwrap();
        assert_eq!(result.starts_at, "2026-08-14");
        assert_eq!(result.ends_at, "2026-08-15");

        // ...and so is one whose provider repeated the same instant.
        let degenerate = candidate("2026-08-14T18:00:00", Some("2026-08-14T18:00:00"));
        let result = verdict_for(&degenerate, &[]).unwrap();
        assert_eq!(result.starts_at, "2026-08-14");
        assert_eq!(result.ends_at, "2026-08-15");
    }

    #[test]
    fn a_broken_instant_is_loud() {
        assert!(verdict_for(&candidate("whenever", None), &[]).is_err());
        let corrupt = entry("busy", "2026-08-14", "not-a-date");
        let day = candidate("2026-08-14", Some("2026-08-15"));
        let error = verdict_for(&day, &[corrupt]).unwrap_err();
        assert!(error.contains("unreadable ends_at"), "{error}");
    }

    #[test]
    fn query_window_spans_every_candidate() {
        let candidates = [
            Candidate {
                id: "a".into(),
                starts_at: "2026-08-14".into(),
                ends_at: Some("2026-08-16".into()),
            },
            Candidate {
                id: "b".into(),
                starts_at: "2026-09-02T18:00:00Z".into(),
                ends_at: None,
            },
        ];
        let (from, to) = query_window(&candidates).unwrap().unwrap();
        assert_eq!(from, "2026-08-14");
        assert_eq!(to, "2026-09-04");
        assert_eq!(query_window(&[]).unwrap(), None);
    }

    #[test]
    fn a_batch_preserves_candidate_order_and_returns_all_three_verdicts() {
        let candidates = [
            Candidate {
                id: "free".into(),
                starts_at: "2026-08-10".into(),
                ends_at: None,
            },
            Candidate {
                id: "needs-travel-day".into(),
                starts_at: "2026-08-11".into(),
                ends_at: None,
            },
            Candidate {
                id: "conflicts".into(),
                starts_at: "2026-08-12".into(),
                ends_at: None,
            },
        ];
        let entries = [
            entry("work_remote", "2026-08-10", "2026-08-11"),
            entry("work_onsite", "2026-08-11", "2026-08-12"),
            entry("away", "2026-08-12", "2026-08-13"),
        ];

        let verdicts = verdicts_for(&candidates, &entries).unwrap();
        assert_eq!(
            verdicts
                .iter()
                .map(|verdict| (verdict.id.as_str(), verdict.verdict))
                .collect::<Vec<_>>(),
            [
                ("free", Feasibility::Free),
                ("needs-travel-day", Feasibility::NeedsTravelDay),
                ("conflicts", Feasibility::Conflicts),
            ]
        );
    }

    fn days(from: &str, to: &str) -> (i64, i64) {
        (date::parse_date(from).unwrap(), date::parse_date(to).unwrap())
    }

    #[test]
    fn windows_break_on_conflicts_and_carry_the_soft_cost() {
        let (from, to) = days("2026-08-10", "2026-08-17");
        let entries = [
            entry("away", "2026-08-12", "2026-08-14"),
            entry("work_onsite", "2026-08-11", "2026-08-12"),
        ];
        let windows = feasible_windows(from, to, &entries, 1).unwrap();
        assert_eq!(windows.len(), 2);

        assert_eq!(windows[0].starts_on, "2026-08-10");
        assert_eq!(windows[0].ends_before, "2026-08-12");
        assert_eq!(windows[0].days, ["2026-08-10", "2026-08-11"]);
        assert_eq!(windows[0].verdict, Feasibility::NeedsTravelDay);
        assert_eq!(windows[0].days_needing_travel_day, ["2026-08-11"]);

        // The away block ends 2026-08-14, exclusive, so the 14th is feasible.
        assert_eq!(windows[1].starts_on, "2026-08-14");
        assert_eq!(windows[1].ends_before, "2026-08-17");
        assert_eq!(windows[1].verdict, Feasibility::Free);
        assert!(windows[1].days_needing_travel_day.is_empty());
    }

    #[test]
    fn min_days_drops_runs_too_short_to_search() {
        let (from, to) = days("2026-08-10", "2026-08-20");
        let entries = [
            entry("away", "2026-08-11", "2026-08-15"),
            entry("away", "2026-08-16", "2026-08-18"),
        ];
        let all = feasible_windows(from, to, &entries, 1).unwrap();
        assert_eq!(all.len(), 3, "10th, 15th, and 18th-19th");

        let long_enough = feasible_windows(from, to, &entries, 2).unwrap();
        assert_eq!(long_enough.len(), 1);
        assert_eq!(long_enough[0].days, ["2026-08-18", "2026-08-19"]);
    }

    #[test]
    fn a_fully_blocked_span_yields_no_windows() {
        let (from, to) = days("2026-08-10", "2026-08-12");
        let entries = [entry("away", "2026-08-01", "2026-09-01")];
        assert!(feasible_windows(from, to, &entries, 1).unwrap().is_empty());
    }

    #[test]
    fn verdicts_serialize_as_the_contracts_wire_names() {
        let json = serde_json::to_string(&[
            Feasibility::Free,
            Feasibility::NeedsTravelDay,
            Feasibility::Conflicts,
        ])
        .unwrap();
        assert_eq!(json, r#"["free","needs-travel-day","conflicts"]"#);
    }
}


// ---------------------------------------------------------------------------
// Phase D: which entries belong to one journey
// ---------------------------------------------------------------------------

/// A run of entries in the same place, close enough in time to be one trip
/// rather than two.
///
/// Deliberately *not* a `trips.plan`: this module stays pure over `&[Entry]`
/// and never writes anywhere. Turning a draft into a real plan is an explicit
/// act somewhere else, which is also what keeps a draft cheap enough to
/// recompute on every request instead of storing and invalidating.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TripDraft {
    /// The clustering key, as the source recorded it.
    pub place: String,
    pub starts_on: String,
    /// Exclusive, like every other end in this capability.
    pub ends_before: String,
    pub entry_ids: Vec<String>,
    pub titles: Vec<String>,
    /// The strongest commitment among the members. A cluster of things you
    /// merely found is a possible trip; one booked ticket among them makes the
    /// whole journey real, because you are going either way.
    pub commitment: Commitment,
}

/// An entry that cannot be clustered, and why. Reported rather than dropped:
/// 13 of 62 events in the last live Luma sweep had no usable place, and a
/// silent filter is indistinguishable from a bug the first time something you
/// wanted goes missing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Unclustered {
    pub entry_id: String,
    pub title: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TripDrafts {
    pub drafts: Vec<TripDraft>,
    pub unclustered: Vec<Unclustered>,
}

/// Where an entry is, as the sources actually record it.
///
/// Luma writes the city into `payload.city` and the street into `location`, so
/// `location` alone would cluster "Theresienstraße 6" separately from
/// "München". City first, `location` as the fallback for entries that carry
/// only that.
fn place_of(entry: &Entry) -> Option<String> {
    let city = entry
        .payload
        .get("city")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|city| !city.is_empty());
    let fallback = entry.location.as_deref().map(str::trim).filter(|l| !l.is_empty());
    city.or(fallback).map(|place| place.to_string())
}

/// Groups entries into draft trips.
///
/// `max_gap_days` is how far apart two things in the same place can be and
/// still be one journey — the issue's own example is two Munich events five
/// days apart. `home` is the place that is never a trip; passing `None` means
/// every place clusters, including the one you live in, which is visible
/// rather than silently wrong.
pub fn cluster_trips(
    entries: &[Entry],
    max_gap_days: i64,
    home: Option<&str>,
) -> Result<TripDrafts, String> {
    let mut by_place: std::collections::BTreeMap<String, Vec<&Entry>> = Default::default();
    let mut unclustered = Vec::new();

    for entry in entries {
        if entry.kind != "event" && entry.kind != "nightlife" {
            continue;
        }
        match place_of(entry) {
            None => unclustered.push(Unclustered {
                entry_id: entry.id.clone(),
                title: entry.title.clone(),
                reason: "no place recorded, so it cannot be grouped with anything".into(),
            }),
            Some(place) => {
                if home.is_some_and(|home| home.eq_ignore_ascii_case(&place)) {
                    unclustered.push(Unclustered {
                        entry_id: entry.id.clone(),
                        title: entry.title.clone(),
                        reason: format!("{place} is home, which is not a trip"),
                    });
                    continue;
                }
                by_place.entry(place).or_default().push(entry);
            }
        }
    }

    let mut drafts = Vec::new();
    for (place, mut members) in by_place {
        members.sort_by(|a, b| a.starts_at.cmp(&b.starts_at));
        let mut run: Vec<&Entry> = Vec::new();
        let mut run_end: Option<i64> = None;

        for entry in members {
            let (start, end) = entry_days(entry)?;
            let split = run_end.is_some_and(|previous_end| start - previous_end > max_gap_days);
            if split {
                push_trip_run(&mut drafts, &mut unclustered, &place, &run)?;
                run.clear();
                run_end = None;
            }
            run_end = Some(run_end.map_or(end, |previous| previous.max(end)));
            run.push(entry);
        }
        if !run.is_empty() {
            push_trip_run(&mut drafts, &mut unclustered, &place, &run)?;
        }
    }

    drafts.sort_by(|a, b| a.starts_on.cmp(&b.starts_on).then(a.place.cmp(&b.place)));
    Ok(TripDrafts { drafts, unclustered })
}

/// A journey is evidence from multiple calendar anchors. A remote one-off is
/// still valuable, but it belongs in the ordinary Calendar proposal inbox;
/// rendering it as a trip would pressure the operator to create a plan merely
/// to acknowledge one event.
fn push_trip_run(
    drafts: &mut Vec<TripDraft>,
    unclustered: &mut Vec<Unclustered>,
    place: &str,
    run: &[&Entry],
) -> Result<(), String> {
    if run.len() >= 2 {
        drafts.push(draft_from(place, run)?);
    } else if let Some(entry) = run.first() {
        unclustered.push(Unclustered {
            entry_id: entry.id.clone(),
            title: entry.title.clone(),
            reason: format!("only one event in {place}; review it as a calendar event, not a trip"),
        });
    }
    Ok(())
}

/// First and last day an entry touches, as day numbers. The stored end is
/// exclusive, so a same-day entry ends on the day it starts.
fn entry_days(entry: &Entry) -> Result<(i64, i64), String> {
    let starts = date::parse_date(&entry.starts_at[..10.min(entry.starts_at.len())])
        .ok_or_else(|| format!("{}: unreadable starts_at", entry.id))?;
    let ends_raw = date::parse_date(&entry.ends_at[..10.min(entry.ends_at.len())])
        .ok_or_else(|| format!("{}: unreadable ends_at", entry.id))?;
    let ends = if entry.all_day { ends_raw - 1 } else { ends_raw };
    Ok((starts, ends.max(starts)))
}

fn draft_from(place: &str, run: &[&Entry]) -> Result<TripDraft, String> {
    let mut first = i64::MAX;
    let mut last = i64::MIN;
    for entry in run {
        let (start, end) = entry_days(entry)?;
        first = first.min(start);
        last = last.max(end);
    }
    Ok(TripDraft {
        place: place.to_string(),
        starts_on: date::format_date(first),
        ends_before: date::format_date(last + 1),
        entry_ids: run.iter().map(|e| e.id.clone()).collect(),
        titles: run.iter().map(|e| e.title.clone()).collect(),
        commitment: run
            .iter()
            .map(|e| e.commitment)
            .max()
            .unwrap_or(Commitment::Possible),
    })
}


#[cfg(test)]
mod cluster_tests {
    use super::*;
    use serde_json::json;

    fn event(id: &str, place: &str, starts_at: &str, ends_at: &str, c: Commitment) -> Entry {
        Entry {
            id: id.into(),
            kind: "event".into(),
            commitment: c,
            title: format!("{place} {id}"),
            starts_at: starts_at.into(),
            ends_at: ends_at.into(),
            all_day: !starts_at.contains('T'),
            location: None,
            notes: None,
            source: "luma".into(),
            external_id: None,
            rhythm_id: None,
            payload: json!({ "city": place }),
            created_at: "0".into(),
            updated_at: "0".into(),
        }
    }

    /// The issue's own example: two events in Munich five days apart are one
    /// trip, not two.
    #[test]
    fn two_events_in_one_place_within_the_gap_are_one_trip() {
        let entries = [
            event("a", "München", "2026-08-12T09:00:00", "2026-08-12T17:00:00", Commitment::Possible),
            event("b", "München", "2026-08-17T18:00:00", "2026-08-17T21:00:00", Commitment::Possible),
        ];
        let out = cluster_trips(&entries, 5, None).unwrap();
        assert_eq!(out.drafts.len(), 1, "five days apart is still one journey");
        assert_eq!(out.drafts[0].place, "München");
        assert_eq!(out.drafts[0].starts_on, "2026-08-12");
        assert_eq!(out.drafts[0].ends_before, "2026-08-18");
        assert_eq!(out.drafts[0].entry_ids, vec!["a", "b"]);
    }

    /// ...and the negation, which is what makes the gap mean anything.
    #[test]
    fn a_wider_gap_leaves_two_single_events_for_calendar_review() {
        let entries = [
            event("a", "München", "2026-08-12T09:00:00", "2026-08-12T17:00:00", Commitment::Possible),
            event("b", "München", "2026-08-20T18:00:00", "2026-08-20T21:00:00", Commitment::Possible),
        ];
        let out = cluster_trips(&entries, 5, None).unwrap();
        assert!(out.drafts.is_empty(), "one event is not a trip");
        assert_eq!(out.unclustered.len(), 2, "each isolated event stays a calendar proposal");
        assert!(out.unclustered.iter().all(|entry| entry.reason.contains("calendar event")));
    }

    #[test]
    fn different_places_never_merge_however_close_in_time() {
        let entries = [
            event("a", "München", "2026-08-12T09:00:00", "2026-08-12T17:00:00", Commitment::Possible),
            event("b", "Hamburg", "2026-08-13T09:00:00", "2026-08-13T17:00:00", Commitment::Possible),
        ];
        let out = cluster_trips(&entries, 5, None).unwrap();
        assert!(out.drafts.is_empty());
        assert_eq!(out.unclustered.len(), 2);
    }

    /// 13 of 62 events in the last live Luma sweep had no usable place.
    #[test]
    fn an_event_without_a_place_is_reported_not_dropped() {
        let mut placeless = event("x", "", "2026-08-12T09:00:00", "2026-08-12T17:00:00", Commitment::Possible);
        placeless.payload = json!({});
        let out = cluster_trips(&[placeless], 5, None).unwrap();
        assert!(out.drafts.is_empty());
        assert_eq!(out.unclustered.len(), 1);
        assert!(out.unclustered[0].reason.contains("no place"));
    }

    #[test]
    fn home_is_not_a_trip_and_says_so() {
        let entries = [event("a", "Example City", "2026-08-12T19:00:00", "2026-08-12T21:00:00", Commitment::Possible)];
        let out = cluster_trips(&entries, 5, Some("example city")).unwrap();
        assert!(out.drafts.is_empty(), "matching is case-insensitive");
        assert!(out.unclustered[0].reason.contains("home"));
    }

    /// One booked ticket makes the whole journey real: you are going anyway.
    #[test]
    fn a_draft_carries_the_strongest_commitment_in_it() {
        let entries = [
            event("a", "München", "2026-08-12T09:00:00", "2026-08-12T17:00:00", Commitment::Possible),
            event("b", "München", "2026-08-13T09:00:00", "2026-08-13T17:00:00", Commitment::Committed),
        ];
        let out = cluster_trips(&entries, 5, None).unwrap();
        assert_eq!(out.drafts[0].commitment, Commitment::Committed);
    }

    /// An all-day entry's stored end is exclusive, so it must not stretch the
    /// trip a day past where it actually ends.
    #[test]
    fn an_all_day_events_exclusive_end_does_not_lengthen_the_trip() {
        let entries = [
            event("a", "München", "2026-08-12", "2026-08-13", Commitment::Possible),
            event("b", "München", "2026-08-12T18:00:00", "2026-08-12T21:00:00", Commitment::Possible),
        ];
        let out = cluster_trips(&entries, 5, None).unwrap();
        assert_eq!(out.drafts[0].starts_on, "2026-08-12");
        assert_eq!(out.drafts[0].ends_before, "2026-08-13");
    }

    #[test]
    fn one_remote_event_is_a_calendar_proposal_not_a_trip() {
        let entries = [event("a", "München", "2026-08-12T09:00:00", "2026-08-12T17:00:00", Commitment::Possible)];
        let out = cluster_trips(&entries, 5, None).unwrap();
        assert!(out.drafts.is_empty());
        assert_eq!(out.unclustered.len(), 1);
        assert!(out.unclustered[0].reason.contains("calendar event"));
    }

    #[test]
    fn only_events_cluster_work_and_away_blocks_are_not_trips() {
        let mut work = event("w", "München", "2026-08-12T09:00:00", "2026-08-12T17:00:00", Commitment::Committed);
        work.kind = "work_onsite".into();
        let out = cluster_trips(&[work], 5, None).unwrap();
        assert!(out.drafts.is_empty() && out.unclustered.is_empty());
    }
}
