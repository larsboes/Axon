---
project: axon-traveler
type: isa
phase: climbing
progress: 65
principal_stated_goal: "Upgrade the Axon travel systems into a real hyper-personalised travel planning system, usable both by me directly and with an agent."
---

# ISA · traveler

Capability-scoped state of record. Repo-wide items stay in the root `ISA.md`;
the travel *search* stack's record stays in `Packs/travel/ISA.md`.

Live personal values stay out of this public file by rule. Everything below that
names a count was measured against the overlay's database on 2026-09-23; the
values themselves are not repeated here.

## Problem

The travel stack could search and could rank, and the ranking was the same for
every person and every trip. `capabilities/trips/src/plan_search.rs` generates
destination candidates from calendar windows, cities, events, climate and
presence, then weighs them with four `const` values. It declares a fifth factor,
`FACTOR_RETROSPECTIVE`, with a doc comment stating it is *"declared and NOT
computed in v1"*.

The personal signal was captured and read by nothing:

- `plan.interests` is a column on every plan, populated on all but one, and is
  echoed to the page that displays it and scored by nothing.
- `option_set` plan items record the options that were offered beside the one
  that was chosen. Twenty-eight exist on one upcoming trip. Nothing compares
  them.
- Thirteen real trips carry dates, destinations and companions. No code reads
  them for a pattern.
- The retrospective endpoint, its form and its published factor formula exist;
  the table has zero rows, with one retrospective pending past the window.

There is also no place for a hard constraint to live. `PlanSearchRequest`
accepts `origin`, `month`, `date_window`, `min_days`, `budget_cents`, `currency`,
`modes`, `interests` and `max_candidates` — and no weight and no limit of any
kind. A search cannot refuse a 05:40 departure or a 00:50 arrival.

## Vision

A search that answers with the traveller's own reasons: the limits they set are
obeyed and named when they cost something, the weights are theirs, and every
value the profile rests on can say whether it was stated, derived, proposed, or
never established at all. A profile that has never been written changes nothing —
so the upgrade is safe on the day it lands and on every day the capability is
down.

## Goal

The profile exists, is served, refuses what it cannot honour, and reports the
provenance of every field — and one real search reads it.

## Features

### F1 · The profile exists and says where every value came from

Why: a weight chosen by a person and a weight nobody has looked at behave
identically and must not read the same.

- [x] TRV-1 — an unstated profile is served rather than 404'd, and claims no
  weights: `TravelProfile::unstated()` returns `revision: 0`, and
  `stated_weights()` returns `None` so a consumer keeps its own defaults.
  Evidence: `an_unstated_profile_constrains_nothing_and_claims_no_weights`,
  `a_stated_weight_makes_the_profile_the_source`,
  `an_unstated_profile_is_served_rather_than_404d`. Falsifier: a `404`, or a
  `Some` from an unstated profile.
- [x] TRV-2 — `basis` covers every field in `BASIS_KEYS` exactly once, and a map
  that omits a field or invents one is refused rather than defaulted. Evidence:
  `every_declared_field_has_a_basis_entry_and_none_are_invented`,
  `a_basis_that_omits_a_field_is_refused_rather_than_defaulted`,
  `a_basis_naming_a_field_that_does_not_exist_is_refused`. Falsifier: a write
  whose `basis` is one key short and still returns 2xx.
- [x] TRV-3 — the manifest covers every served route, every write route declares
  its request schema, and every declared path is actually mounted. Evidence:
  `the_manifest_covers_every_served_route`,
  `every_write_route_declares_its_request_schema`,
  `every_declared_path_is_actually_served`. Falsifier: a mounted path missing
  from `ROUTES`.
- [x] TRV-4 — weights that do not sum to 1.0 are refused before the store is
  touched, and the refusal names the sum. Evidence:
  `weights_that_do_not_sum_to_one_are_refused_before_anything_is_written`, which
  also asserts the profile is still `stored: false` afterwards. Falsifier: a
  2xx, or a refusal that does not name the number.
- [x] TRV-5 — a conditional write against a stale revision is refused and changes
  nothing; the check happens inside the write transaction. Evidence:
  `a_write_against_a_stale_revision_is_refused_and_changes_nothing` (store) and
  `a_write_against_a_stale_revision_is_a_409_naming_the_current_one` (HTTP).
  Falsifier: a stored write, or a revision that moved anyway.

### F2 · It is a surface an agent can drive and a browser cannot reach sideways

- [x] TRV-6 — a foreign browser origin is refused on the **wired** router, not
  merely by an available predicate, and a request with no `Origin` still passes.
  Evidence: `a_foreign_browser_origin_is_refused_on_the_wired_router`, which
  drives the router the binary serves. Falsifier: a 200 for
  `Origin: https://evil.example` on any route added after the layer.
- [x] TRV-7 — a profile round-trips through HTTP and reports its revision, and
  `PUT` answers 201 on first write and 200 after. Evidence:
  `a_profile_round_trips_through_http_and_reports_its_revision`, plus a live
  `curl` against the running process. Falsifier: a field that does not survive
  the trip.

### F3 · The baseline comes from the rows, not from a guess

Why: the stored plans, their dates, companions and interests were captured and
read by nothing. The read is pure arithmetic over typed columns.

- [x] TRV-8 — the projection publishes what it counted and what it cannot say.
  Evidence: `notes` carries three limits and `basis` carries every plan id used,
  asserted by `the_baseline_counts_plans_and_a_not_taken_one_out`; the live
  response was checked against `<overlay>/data/traveler/evidence-2026-09-23.md`.
  Falsifier: a count with no `basis`, or a `notes` list shorter than the limits
  the fields have.
- [x] TRV-9 — lead time is measured only over plans whose row predates the trip.
  The first live run over a vault-imported history published a **negative**
  median — the row's age, not a booking lead time, rendered as a measurement.
  Evidence:
  `an_imported_trip_is_left_out_of_the_lead_time_rather_than_counted_negative`
  and `a_history_with_no_bookings_has_no_lead_time_rather_than_a_negative_one`;
  the figures are in `<overlay>/data/traveler/evidence-2026-09-23.md`.
  Falsifier: a negative figure in `lead_time_days`.
- [x] TRV-10 — a machine where `trips` has never run answers with absences rather
  than 500. Evidence:
  `a_machine_where_trips_has_never_run_reads_as_empty_rather_than_failing`,
  `plans_without_a_retrospectives_table_still_derive`, and the live HTTP case
  `the_derived_route_answers_with_no_trips_rather_than_failing`. Falsifier: a
  non-200 from the route on a fresh install.
- [x] TRV-11 — company is counted as a shape and never listed. The projection
  reads `travelers` only through `traveller_count`, which returns a number.
  Evidence: `company_counts_a_shape_and_never_a_name`; live output carries
  `{unrecorded: 3, pair: 5, group: 4}` and no name anywhere. Falsifier: a name in
  the response.

### F4 · A second grain of weights, and a migration that keeps the round trip

Why: `plan-search` ranks destinations, but the operator's trips are anchored — an
event or a person decides the city, and the question is which connection gets
there. The destination factors do not describe a journey, so ranking one needs
its own block.

- [x] TRV-12 — `journey` is a separate weight block, and a write that does not
  sum to 1.0 is refused naming **which** block and which four keys. Two blocks
  with different keys means "weights must sum to 1.0" alone does not tell a
  caller what to fix. Evidence: `the_journey_block_is_refused_with_its_own_keys_named`,
  `the_default_weights_are_normalised`, `both_weight_blocks_are_declared_in_basis`.
  Falsifier: a refusal that names neither the block nor its keys.
- [x] TRV-13 — the column arrives on a deployed table without losing a row.
  SQLite adds a column but cannot alter one, so this is an `ADD COLUMN` with a
  `{}` default that `#[serde(default)]` reads as usable weights. Evidence:
  `a_table_without_the_journey_column_gains_it_without_losing_a_row`, and the
  live column added on this machine with the existing row intact. Falsifier: a
  row lost, or a stored profile that cannot be read back.
- [x] TRV-14 — a row written before a block existed is both readable and
  writable. Found live: the journey column arrived and a `PUT` of the existing
  profile was refused, because its `basis` carried no `journey.*` entries and
  `validate` demands a provenance for every declared field. An absent entry is
  completed as `default` on read. Evidence:
  `a_row_written_before_a_block_existed_is_readable_and_writable_again`, and a
  live GET-then-PUT of the stored profile answering 200. Falsifier: a `400` on a
  body that came from `GET /api/profile`.
- [x] TRV-15 — an unstated journey block ranks nothing, so the block's arrival
  changed no search. Evidence: `an_unstated_journey_block_ranks_nothing`; live,
  the existing row reads every `journey.*` provenance as `default`. Falsifier: a
  consumer that ranks while `stated_journey_weights()` returns `None`.

## Not yet specified

In scope, too dim to state as a claim yet.

- **F3 · The vault proposes.** The design conversation settled "profile is the
  interface, the vault proposes, the operator confirms". `TELOS/Personal/Events
  Profile.md` is marked *"Draft profile, awaiting Lars's corrections"* and
  `scouting` already reads it for event scoring. The proposal route reads the
  TELOS notes, emits a per-field diff with the quoted span each change rests on,
  and writes nothing until the operator accepts. Gated on deciding which TELOS
  sections are admissible: `CURRENT_STATE` holds money, freedom, rhythms and
  relationships, which is a different kind of fact from an interest list.
- **F4 · The offered-versus-chosen half.** Twenty-eight `option_set` items exist
  on one plan and the chosen stay beside them. Comparing what was offered to what
  was picked is the sharpest personal signal in the store and needs no new input
  — but one plan is one observation, and a preference inferred from a single
  choice is a guess wearing a number.
- **F5 · `plan-search-v2` reads the profile.** The consumer. Without it this
  capability is a store with a contract and no reader, which is why it was the
  first item in the design conversation and not the last.
- **F6 · Companion patterns, pseudonymously.** `places_person_places` holds
  nineteen rows, all in state `proposed`, with the person's name in a plain
  column and the same individual appearing under two spellings. The chosen shape
  is that the profile learns *company* — solo, pair, group, family — resolved
  through the register's person ids, never names. `GET /api/plans` still serves
  `travelers`; the ISA that records that leak is `Packs/travel/ISA.md` L3, whose
  first precondition is met and whose other two are not.
- **Whether the profile should be one row or one per person.** The store is
  keyed by id and nothing resolves a "current" one, which is deliberate. The day
  a second profile is needed is the day that question is real.

## Test Strategy

| isc | type | check | threshold | tool | anchors_to |
| --- | --- | --- | --- | --- | --- |
| TRV-1 | command | `cargo test -p traveler`; read `/api/profile` with no row | `stored:false`, `revision:0`, weights `None` | cargo + curl | F1 |
| TRV-2 | command | `cargo test -p traveler model::` | every omission and invention refused | cargo | F1 |
| TRV-3 | command | `cargo test -p traveler route_manifest_tests::` | zero undeclared, zero schema-less bodies | cargo | F1 |
| TRV-4 | command | PUT with weights summing to 1.7 | 400 naming the sum; nothing written | curl + jq | F1 |
| TRV-5 | command | PUT twice, second with a wrong `expected_revision` | 409, `current_revision` correct | curl + jq | F1 |
| TRV-6 | command | GET with a foreign `Origin` on the wired router | 403, and 200 without one | cargo | F2 |
| TRV-7 | command | PUT then GET against the running process | every field survives | curl + jq | F2 |
| TRV-8 | command | `cargo test -p traveler derive::`; read `/api/profile/derived` | `basis` non-empty, `notes` complete | cargo + curl | F3 |
| TRV-9 | command | derive a history whose rows postdate their trips | lead time absent or positive, never negative | cargo | F3 |
| TRV-10 | command | derive against a database with no trips tables | 200, absences not zeros | cargo + curl | F3 |
| TRV-11 | command | grep the live derived body for a traveller name | 0 hits | curl | F3 |
| TRV-12 | command | PUT a profile whose journey weights sum to 1.3 | 400 naming the block and its keys | cargo + curl | F4 |
| TRV-13 | red-then-green | add the column to a hand-built old-shape table | row preserved, defaults readable | cargo | F4 |
| TRV-14 | command | GET `/api/profile` then PUT the same body back | 2xx, not 400 | curl | F4 |
| TRV-15 | command | read a row whose journey basis is absent | `stated_journey_weights()` is `None` | cargo | F4 |

## Anti-claims

- [x] A1 — no personal value lives in this repository. `config.rs` holds a port
  and a database path and nothing else; the profile is a row in the overlay's
  database. Falsifier: a name, an address or a station in tracked source.
- [x] A2 — a missing profile never fails a search. `stated_weights()` returns
  `None` and no consumer is required to call this at all. Falsifier: a consumer
  whose error path includes "traveler unavailable".
- [x] A3 — no stored profile reaches a model prompt. Nothing here calls a model;
  there is no inference dependency in `Cargo.toml`. Falsifier: a prompt builder
  or an inference role in this crate.
- [x] A4 — no write route is reachable from a foreign browser origin. Held by the
  guard on the wired router, asserted by driving the router rather than the
  predicate. Falsifier: TRV-6 going green against a route added below the layer.

## Decisions

- **2026-09-23 — the stated half, and why it declines every hard limit.** The
  operator set all five weights and declined `earliest_departure`,
  `latest_arrival`, `max_changes` and `min_transfer_buffer_min` — recorded as
  `stated` with a null value, not left `default`, because "no limit" is a
  decision and must not read as a field nobody has looked at. The values
  themselves are personal and live in the overlay; the figures from that write are
  in `<overlay>/data/traveler/evidence-2026-09-23.md`.

  The reasoning matters more than the values. On changes and transfer time: *it
  depends on the trip, predicted delays should be taken into account, and
  sometimes stops are useful — a toilet, buying something.* Three consequences:

  1. A hard transfer floor is the wrong instrument. The measured delay history
     `punctuality` already publishes is per station, per train type and per hour,
     and it is a better answer than one number applied to every connection — so
     the floor stays null and the risk is read per leg, which is what
     `journey reliability` already does.
  2. **A transfer is not purely a cost.** Every existing treatment counts a
     change as risk and time lost. A stop is also an opportunity, which no field
     in this profile can express — recorded here rather than lost, and not built.
  3. A constraint that depends on the trip cannot live on the profile. It belongs
     on the plan or the search request, which is where it already is
     (`PlanSearchRequest` takes the caller's own bounds per call).

  The window fields stay null for the same reason: with no limit, a search
  refuses nothing and the ranking carries the preference instead. That is the
  intended behaviour, not an unfinished profile.

- **2026-09-23 — the capability is `traveler`, and it owns the profile rather
  than the plans.** The four readers are `trips`, `transit`, `scouting` and
  `calendar`; a table in `trips` would give three of them a dependency on a peer,
  and `transit` is the lower layer. Recorded in `README.md` D1.
- **2026-09-23 — the weight keys are `plan_search`'s factor keys.** No
  translation table, because a second name for one factor is where the two
  drift. `README.md` D2.
- **2026-09-23 — `default` is a provenance value, not a missing entry.** The
  alternative makes a real state and a typo indistinguishable, in the one
  mechanism whose job is telling a decision from an accident. `README.md` D3.
- **2026-09-23 — no permissive CORS, following `places` rather than `trips`.**
  This serves personal state and will serve companion patterns. `README.md` D4.
- **2026-09-23 — the design conversation's four rulings.** Recorded because they
  shaped every claim above: the profile is the interface and the vault proposes
  (F3); the first thing personalisation changes is ranking and hard constraints
  (F5); the profile gets its own capability (D1); companions stay pseudonymous
  and the `travelers` leak closes (F6).

## Log

- 2026-09-23 · F4 added. The journey weight block, at a second grain from the
  destination weights, with the `ADD COLUMN` migration SQLite allows. The
  migration exposed a gap the tests then closed: a profile written before a block
  existed is readable but was not writable, because its basis carried no
  provenance for the new block's fields.
- 2026-09-23 · The stated half written. Every hard limit declined, with the
  reasoning recorded in Decisions — a transfer floor is the wrong instrument for
  a preference that depends on the trip, and a stop is not purely a cost. F3
  added the same day; the first live run over the real history published a
  negative lead time and two counts that under-report, both now stated in the
  response rather than discovered later. The attendance ground truth came from a
  narrowing pass over the past plans; the counts are in
  `<overlay>/data/traveler/evidence-2026-09-23.md`.
- 2026-09-23 · Scaffolded from the design conversation that settled the four
  rulings in Decisions. F1 and F2 shipped with 25 passing tests; F3–F6 recorded
  as not yet specified.
