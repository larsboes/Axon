---
project: axon
type: isa
phase: climbing
progress: 0
principal_stated_goal: "I want no new issues, I wanna get rid of all issues for axon and axon personal and only carry through normal ISAs etc."
---

# ISA · Axon

Repo-wide state of record. Open work that belongs to one capability or Pack lives in
that owner's own `ISA.md` (`Packs/travel/ISA.md`, `capabilities/learning/ISA.md`); this
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

- **C1** — `bazel test //...` before claiming the gates pass. `//:architecture_up_to_date_test`
  is the only check that catches a stale generated `ARCHITECTURE.md`.
- **C2** — `tools/self check` fails locally and that is pre-existing: it compares per-unit
  code counts only when a code graph is present, and this machine's graph is behind main.
  Verify in a `git worktree` of origin/main, not in place.
- **C3** — sweeps run `rg --no-ignore --hidden --follow`; plain `rg` honours `.gitignore`
  and hides most of axon-personal's `config/`.
- **C4** — `service-runner.sh status` prints the DECLARED image tag; `container list`
  prints the running one. Only the second answers what is actually running.

## Goal

Zero open issues in `larsboes/Axon` and `larsboes/axon-personal`, every live item they
carried standing as a claim in an ISA, and no automation able to open a new one.

## Features

### F0 · The tracker stops being the backlog

Why: two systems of record is the actual defect; closing the issues without moving the
doctrine and the automation would refill the tracker by Monday.

- [ ] ISC-1 — `gh issue list --state open` returns nothing in either repo. Falsifier:
  any open issue in `larsboes/Axon` or `larsboes/axon-personal`.
- [ ] ISC-2 — every closed issue's live content stands as a claim or a Not-yet-specified
  entry in an ISA, and its closing comment names the file. Falsifier: a closing comment
  with no destination, or a destination that does not contain the content.
- [ ] ISC-3 — the doctrine agrees with itself: `README.md`, `CONTRIBUTING.md`,
  `AGENTS.md` and `tools/doctor.ts` all name ISAs as the backlog. Falsifier:
  `rg -i "backlog is Issues|GitHub Issue"` still describes the backlog anywhere in those
  four files.
- [ ] ISC-4 — `upstream-watch` opens no issue: it writes the drift report to the job
  summary and exits non-zero when an entry is past its cooldown. Falsifier: the workflow
  file still calls `gh issue create`, or a green run hides real drift.
- [ ] ISC-5 — `bazel test //...` green on the landing commit, `ARCHITECTURE.md` included.
  Falsifier: any failing target.

### F1 · Demo seeding for Comms, Scouting and Transit

Why: three capabilities are missing from the published demo, so Feed, Scout and Travel
are hidden and the shell looks thinner than the system is.

- [ ] ISC-6 — the demo shows Comms, Scouting and Transit, every recorded value having
  come from a real server answering a real request, not a hand-written fixture.
  Falsifier: a recorded response no capability actually produced.
- [ ] ISC-7 — transit's `FAHRPLAN_URL` and `ORTE_URL` are env-overridable, so the seeder
  can point the real parser at a stub. Falsifier: the consts are still hardcoded.

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
  (quarry: `~/Developer/Inbox/labs/data-lab/projects/link-prediction`, read, not copied).
  Two things to know before consuming it: the snapshot uses the vault's old `Areas/`
  paths, so it predates the `Knowledge/` rename and the note count has roughly tripled;
  and precision matters more than recall, so the useful output is a short ranked list
  per note, not a score for every pair. No consumer yet — that is what keeps it here
  rather than in Features.
- **Operator installs past cooldown**, neither a security item: tailscale 1.98.9 →
  1.102.2, xberg 1.0.5 → 1.0.14.

## Test Strategy

| isc | type | check | threshold | tool | anchors_to |
| --- | --- | --- | --- | --- | --- |
| ISC-1 | command | `gh issue list --state open` in both repos | empty | gh | stated goal |
| ISC-2 | file | read each closing comment and its named ISA | content present | Read | "carry through ISAs" |
| ISC-3 | command | `rg -i "backlog is Issues"` over the four files | zero hits | rg | "no new issues" |
| ISC-4 | command | `rg "gh issue create" .github/workflows` | zero hits | rg | "no new issues" |
| ISC-5 | command | `bazel test //...` | all pass | bazel | C1 |
| ISC-6 | command | demo build, inspect recorded responses | three capabilities present | bash | F1 |
| ISC-7 | code inspect | read transit's URL consts | env-overridable | rg | F1 |
| ISC-8 | command | `tools/upstream-checker` | every warn bumped or held with a reason | bash | F2 |
| ISC-9 | command | `git log` for the postgres bump | its own commit | git | F2 |

## Anti-claims

- [ ] A1 — no new tracking surface replaces the tracker. Falsifier: a `TODO.md`,
  `PLAN.md`, `HANDOFF.md` or `ROADMAP.md` appears anywhere in the repo, gitignored ones
  included.
- [ ] A2 — no issue is closed whose content exists nowhere else. Falsifier: ISC-2 fails
  for any closed issue.
- [ ] A3 — the public repo keeps a path for outside bug reports. Falsifier:
  `.github/ISSUE_TEMPLATE/` is deleted or issues are disabled repo-wide.

## Decisions

- **2026-08-19 — the backlog moves from Issues to ISAs** (principal's call). Migrate
  first, then close; change the doctrine in all four places that state it; stop the one
  workflow that creates issues.
- **2026-08-19 — `upstream-watch` reports to the job summary and reds the run** rather
  than being deleted. Deleting it would leave drift findable only when someone looks.
- **2026-08-19 — `.github/ISSUE_TEMPLATE/` stays.** Axon is public and an external
  report still needs somewhere to land; what changed is that our own backlog is not there.

## Log

- 2026-08-19 · Scaffolded. Carries Axon issues #172, #174, #180 and the tracker
  retirement itself; #185 and #186 went to `Packs/travel/ISA.md`.
