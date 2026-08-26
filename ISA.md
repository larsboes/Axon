---
project: axon
type: isa
phase: climbing
progress: 75
principal_stated_goal: "I want no new issues, I wanna get rid of all issues for axon and axon personal and only carry through normal ISAs etc."
---

# ISA · Axon

Repo-wide state of record. Open work that belongs to one capability or Pack lives in
that owner's own `ISA.md` (`Packs/travel/ISA.md`, `capabilities/places/ISA.md`); this
file holds what is repo-wide or has no other owner.

## Problem

The backlog lived in GitHub Issues, and a tracker is a second system of record that
never says what *done* means. An issue carries a description and a lifecycle; it does
not carry a falsifier, so nothing about closing one proves anything was verified. The
ISA already is the artifact that states done as testable claims, and running both means
maintaining two surfaces that disagree the moment either is edited.

## Vision

One surface. Open work is a claim in an ISA with a probe that would falsify it; the
tracker holds nothing, and no automation creates entries in it.

## Out of Scope

- Disabling the issue tracker for the outside world. Axon is public; an external bug
  report still needs somewhere to land. What changes is that *our* backlog is not there.
- Rewriting closed-issue history. Closed issues stay readable as the record of what was
  once tracked.

## Principles

- **A work item exists because the principal decided it does.** Nothing auto-creates one.
- **Doctrine lives where the tool reads it.** A rule in the README that `tools/doctor`
  contradicts is not a rule.
- **Migrate before closing.** Content that outlives the issue moves into the owning ISA
  or the owning README first; the closing comment names where it went.

## Constraints

- **C1** — `cargo test --workspace --locked -- --skip postgres_tests::` and the
  `tools/check-*.sh` gates before claiming the gates pass.
  `tools/check-architecture-fresh.sh` is the only check that catches a stale generated
  `ARCHITECTURE.md`. Was `bazel test //...` until PRD Q44 retired Bazel (2026-08-25).
- **C2** — `tools/self check` fails locally and that is pre-existing: it compares per-unit
  code counts only when a code graph is present, and this machine's graph is behind main.
  Verify in a `git worktree` of origin/main, not in place.
- **C3** — sweeps run `rg --no-ignore --hidden --follow`; plain `rg` honours `.gitignore`
  and hides most of the private overlay's `config/`.
- **C4** — `service-runner.sh status` prints the DECLARED image tag; `container list`
  prints the running one. Only the second answers what is actually running.

## Goal

Zero open issues in this repository and in the private overlay, every live item they
carried standing as a claim in an ISA, and no automation able to open a new one.

## Features

### F0 · The tracker stops being the backlog

Why: two systems of record is the actual defect; closing the issues without moving the
doctrine and the automation would refill the tracker by Monday.

- [x] ISC-1 — `gh issue list --state open` returns nothing in either repo. Evidence: 0 and 0, 2026-08-20. Falsifier:
  any open issue in this repository or the private overlay.
- [x] ISC-2 — every closed issue's live content stands as a claim or a Not-yet-specified
  entry in an ISA, and its closing comment names the file. Falsifier: a closing comment
  with no destination, or a destination that does not contain the content.
- [x] ISC-3 — the doctrine agrees with itself: `README.md`, `CONTRIBUTING.md`,
  `AGENTS.md` and `tools/doctor.ts` all name ISAs as the backlog. Falsifier:
  `rg -i "backlog is Issues|GitHub Issue"` still describes the backlog anywhere in those
  four files.
- [x] ISC-4 — `upstream-watch` opens no issue: it writes the drift report to the job
  summary and exits non-zero when an entry is past its cooldown. Falsifier: the workflow
  file still calls `gh issue create`, or a green run hides real drift.
- [x] ISC-5 — `bazel test //...` green on the landing commit, `ARCHITECTURE.md` included. Evidence: 68/68 pass, plus `//:architecture_up_to_date_test` forced uncached, 4050fe4.
  Falsifier: any failing target.

### F1 · Demo seeding for Comms, Scouting and Transit

Why: three capabilities are missing from the published demo, so Feed, Scout and Travel
are hidden and the shell looks thinner than the system is.

- [x] ISC-6 — the demo shows Comms, Scouting and Transit, every recorded value having come
  from a real server answering a real request, not a hand-written fixture. Evidence, one
  full `tools/demo-up all` on 2026-08-20: `seeded comms: 7 items ingested from the demo
  origin (3 kept, 1 dismissed)` · `seeded scouting: 7 opportunities discovered through the
  rss adapter (7 persisted)` · `recorded transit: 2 paths` · 41 fixtures, then
  `tools/demo-site` wrote 52 reference pages and the hygiene gate passed over 251 files.
  Spot-checked: comms' titles are its extractor's output from the served HTML, scouting's
  rows carry `source: rss:demo-origin`, transit's journey is the origin's ICE 331 with
  `reliability: null` because punctuality is absent — the declared degradation.
- [x] ISC-7 — transit's endpoints are env-overridable, so the demo can point the real parser
  at a stub. Three, not the two this claim named: `AXON_TRANSIT_DBNAV_FAHRPLAN_URL` had to
  join them once dbnav became the default, or the default backend would have been the one
  path a stub cannot reach. Evidence: `every_endpoint_can_be_pointed_at_a_stub_and_otherwise_is_bahn_de`,
  plus a live CLI search against `tools/demo-origin` returning three parsed journeys on both
  backends.

Correction carried over: `demo.toml`'s `[absent.comms]` blames `sources/rss.rs`, which
is *scouting's* file, not Comms'. That reason was written from a bad reading of an `ls`
whose error was silenced.

### F2 · Upstream drift

Why: pins drift, and a bump is a deliberate audited act, never an auto-pull.

- [ ] ISC-8 — every entry `tools/upstream-checker` reports as `warn` is either bumped
  through the audit gate or has a written reason it is held. Falsifier: a `warn` entry
  with neither. (2026-08-19: 76 entries · 50 ok · 17 n/a · 9 warn · 0 fail, every warn
  inside its cooldown hold, where waiting is the action.)
- [ ] ISC-9 — the postgres 17.9 → 17.10 image decision is made on its own, not ridden
  along with another change. Falsifier: the bump appears in a commit about something else.

## Not yet specified

- **knowledge-graph link prediction over the vault.** `knowledge-graph` serves the code
  graph; the vault has one and nothing reads it. A completed run exists over 679 notes
  and 9,001 candidate pairs with structure, text and metadata features engineered
  (the quarry is a completed link-prediction notebook in a private lab checkout, read rather
  than copied; the overlay records where).
  Two things to know before consuming it: the snapshot uses the vault's old `Areas/`
  paths, so it predates the `Knowledge/` rename and the note count has roughly tripled;
  and precision matters more than recall, so the useful output is a short ranked list
  per note, not a score for every pair. No consumer yet — that is what keeps it here
  rather than in Features.
- **Operator installs past cooldown**, neither a security item: tailscale 1.98.9 →
  1.102.2, xberg 1.0.5 → 1.0.14.
- **The remaining four database-URL call sites.** calendar, finance, tasks and trips each
  hand-roll the same `std::env::var("AXON_<CAP>_DATABASE_URL")` two-liner that
  `axon_config::database_url_override` now owns. They work, so this is deduplication rather
  than a defect, and the repo's rule is that shared logic moves into the lib. Not swept in
  the run that added the helper, deliberately: that run was about the demo.

## Test Strategy

| isc | type | check | threshold | tool | anchors_to |
| --- | --- | --- | --- | --- | --- |
| ISC-1 | command | `gh issue list --state open`, here and in the overlay | empty | gh | stated goal |
| ISC-2 | file | read each closing comment and its named ISA | content present | Read | "carry through ISAs" |
| ISC-3 | command | `rg -i "backlog is Issues"` over the four files | zero hits | rg | "no new issues" |
| ISC-4 | command | `rg "gh issue create" .github/workflows` | zero hits | rg | "no new issues" |
| ISC-5 | command | `cargo test --workspace --locked -- --skip postgres_tests::` | all pass | cargo | C1 |
| ISC-6 | command | demo build, inspect recorded responses | three capabilities present | bash | F1 |
| ISC-7 | code inspect | read transit's URL consts | env-overridable | rg | F1 |
| ISC-8 | command | `tools/upstream-checker` | every warn bumped or held with a reason | bash | F2 |
| ISC-9 | command | `git log` for the postgres bump | its own commit | git | F2 |

## Anti-claims

- [x] A1 — no new tracking surface replaces the tracker. Falsifier: a `TODO.md`,
  `PLAN.md`, `HANDOFF.md` or `ROADMAP.md` appears anywhere in the repo, gitignored ones
  included.
- [x] A2 — no issue is closed whose content exists nowhere else. Falsifier: ISC-2 fails
  for any closed issue.
- [x] A3 — the public repo keeps a path for outside bug reports. Falsifier:
  `.github/ISSUE_TEMPLATE/` is deleted or issues are disabled repo-wide.

## Decisions

- **2026-08-19 — the backlog moves from Issues to ISAs** (principal's call). Migrate
  first, then close; change the doctrine in all four places that state it; stop the one
  workflow that creates issues.
- **2026-08-19 — `upstream-watch` reports to the job summary and reds the run** rather
  than being deleted. Deleting it would leave drift findable only when someone looks.
- **2026-08-20 — three capabilities never read their database variable, and the demo is
  what found it.** `tools/demo-up`'s whole mechanism is one exported
  `AXON_<CAP>_DATABASE_URL` per capability. comms, scouting and transit ignored it, went
  from their config file to `postgres_conn_from_shared_env`, and — the demo overlay having
  no `postgres.env` — landed on a fallback naming the real database, `dbname=axon
  password=axon`. The only thing between a demo seeding run and the live store was that the
  real password is not the word `axon`. Fixed with one shared
  `axon_config::database_url_override`, used by those three. The four that hand-roll the
  same two lines (calendar, finance, tasks, trips) are left alone and recorded below.
- **2026-08-20 — `upstream-checker` published the checkout's absolute path.** Its `--json`
  `manifest` field was `$AXON_ROOT/upstreams.toml`, which axon-status serves and the demo
  records. `tools/check-site-payload` refused to publish over it, which is the job that
  gate has. Now repo-relative.
- **2026-08-19 — `.github/ISSUE_TEMPLATE/` stays.** Axon is public and an external
  report still needs somewhere to land; what changed is that our own backlog is not there.

## Log

- 2026-08-19 · Scaffolded. Carries Axon issues #172, #174, #180 and the tracker
  retirement itself; #185 and #186 went to `Packs/travel/ISA.md`.
