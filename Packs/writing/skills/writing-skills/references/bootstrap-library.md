# Bootstrapping a whole skill library

## Contents
- When this applies (vs. the default single-skill workflow)
- Phase 1: discover before writing
- Phase 2: author in parallel
- Phase 3: review and fix
- Authoring rules to bake into every sub-agent's brief

## When this applies
The rest of this skill authors or audits **one** `SKILL.md`. This reference is for a different job: an existing project has no skill library at all, and the ask is "build the set that lets someone else — a junior engineer, or a cheaper model — carry this project forward." That's 10+ skills, discovered from the repo itself, authored in parallel, reviewed adversarially. Don't reach for this on a single-skill request; it's real overhead, justified only when the deliverable is the whole library.

## Phase 1: discover before writing
Investigate like an incoming engineer, not an author: README/manifest/contributor docs, build system, how the test suite actually runs, CI config, docs directories, git history (what changed, what got reverted, what stalled), TODO/FIXME hotspots, issue-shaped artifacts, generated-data/deploy conventions, any project memory available. Then ask **at most 5 questions**, only for what the repo can't tell you — typically: the hardest live problem right now, unwritten discipline rules, who the audience is and what they don't know, which past failure cost the most time, what "beyond current state" means here. Fold answers into everything below; don't author a single file before this phase completes.

## Phase 2: author in parallel
One delegated agent per skill (`Agent`/worktree-isolated if they'd otherwise conflict on shared files). Adapt this taxonomy to what Phase 1 found — merge thin categories, split deep ones, add domain categories that weren't anticipated. Aim for 10-16 skills:

**Core** (every project needs these): change-control (how changes are classified/gated/reviewed, with the incident behind each non-negotiable) · debugging-playbook (symptom→triage table, the traps that cost real time) · failure-archaeology (investigation/dead-end/revert chronicle, so nobody re-fights a settled battle) · architecture-contract (load-bearing decisions + why, invariants, known weak points) · domain-reference (the field's theory as it applies here) · config-and-flags (every axis: options, defaults, prod-vs-experimental, add-one checklist) · build-and-env (recreate from scratch, known traps) · run-and-operate (command anatomy, artifact conventions) · diagnostics-and-tooling (measure-don't-eyeball, ship the actual scripts) · validation-and-qa (what counts as evidence, the golden inventory) · docs-and-writing (house style, templates).

**Advanced** (what makes a junior dangerous in the good way): a `<hardest-problem>-campaign` — an executable, decision-gated playbook for the single hardest live problem, with expected observations at every gate and known-wrong paths explicitly fenced off · a proof-and-analysis-toolkit (this domain's "prove it" methods, each with a worked example from the repo's own history) · a research-frontier map (open problems, why current approaches fail, the first three concrete steps in this repo).

## Phase 3: review and fix
After ALL skills exist, three parallel reviewers over the complete set, then one fixer:
- **Factual** — re-verify every flag/path/command/citation against the repo; flag anything invented or stale.
- **Doctrine** — contradictions with the project's own rules, or between two skills; overstated claims; anything that changes behavior without a gate.
- **Usability** — trigger-description quality, duplication (one home per fact, cross-reference elsewhere), self-containedness, scannability.

The fixer applies blocking + important findings. Close with: the skill inventory (one line each), what got spot-checked, what's still uncertain — don't claim more verification than actually happened.

## Authoring rules to bake into every sub-agent's brief
- Audience: zero-context mid-level engineer or a cheaper model. Imperative voice, copy-pasteable commands, jargon defined once, tables/checklists, an explicit "when NOT to use this — see sibling X" in every skill.
- **Ground truth only.** Verify every command/flag/path against the repo before writing it down — a wrong runbook is worse than none.
- Embed knowledge; never make a skill's load-bearing content depend on a private/user-specific path.
- End every skill with a **Provenance and maintenance** section: one-line re-verification commands for anything that can drift (matches this skill's own convention — see `SKILL.md`'s footer as the worked example).
- No oversell — unproven things stay labeled open/candidate; nothing may contradict the project's own manifest/rules or route around its change control.
