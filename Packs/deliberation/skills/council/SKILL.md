---
name: council
description: Convenes four to five subagents that hold different positions on one decision, runs them over rounds, and reports a transcript, a recommendation and the minority position. Three rounds for a decision that is expensive to reverse, one round for a sanity check. Carries five domain presets — architecture, investment, travel, security, product — and a rule for composing a council when none of them fits. Use when the request asks for a council, a debate, several perspectives, a weighing of options, or the case for and against a choice. Do not use for a pure attack on one proposal (use red-team), and do not use for a question that has one factual answer.
license: MIT
---

# council

A council is four to five subagents that hold different positions on one decision. Each round
runs them in parallel and feeds every member the rounds before it. The run ends with one
recommendation and the position that lost.

Council weighs options against each other. To attack a single proposal, use `red-team`.

Adapted from the LifeOS Council skill by Daniel Miessler (github.com/danielmiessler/LifeOS), MIT.

## Procedure

Copy this checklist and track progress:

```
- [ ] 1. State the decision in one sentence
- [ ] 2. Pick DEBATE or QUICK
- [ ] 3. Pick a preset, or compose a council
- [ ] 4. Check every option has an advocate
- [ ] 5. Collect the evidence the members need
- [ ] 6. Run the rounds
- [ ] 7. Check every round with the clerk
- [ ] 8. Write the synthesis
```

**1 — State the decision.** Write the question the council answers and the options on the table.
A question with one factual answer is not a council topic. Answer it directly and stop. When the
caller gives a topic rather than a decision, confirm your restatement with `ask` before composing
anyone: it is the one judgement here that you cannot check yourself, and a wrong restatement wastes
the whole run (`references/compose.md`).

**2 — Pick the mode.**

| Mode | Rounds | Use when |
|---|---|---|
| QUICK | 1 | The decision is cheap to reverse, or the caller wants a sanity check |
| DEBATE | 3 | The decision is expensive to reverse, or the options are close |

Default to QUICK. Escalate to DEBATE when the members disagree about facts, not about taste.

**3 — Pick a preset.** Read the one file that matches the topic. Do not read the others.

| Preset | Decides | File |
|---|---|---|
| architecture | Placement, boundaries, adopt against build | `references/presets/architecture.md` |
| investment | A change to a personal portfolio | `references/presets/investment.md` |
| travel | One trip option against another | `references/presets/travel.md` |
| security | Threat model, data classes, blast radius | `references/presets/security.md` |
| product | What to build next, and how much of it | `references/presets/product.md` |

If no preset fits, read `references/compose.md` and write the briefs there. If the caller names
the members, use those names and skip this step.

Never launch the same brief twice. Identical members agree, and agreement carries no information.

**4 — Check every option has an advocate.** Print the option-to-advocate table in the header; an
option with nobody arguing for it loses for the wrong reason. Add a fifth member who holds it
(`references/compose.md`), or take the option off the table. Step 8 reports `recompose` when a run
reaches here without one.

**5 — Collect the evidence.** Each preset names what its members demand. Gather it before round 1
and put it in every member prompt. A member that has to guess produces an unverified claim, and
step 7 discards it.

**6 — Run the rounds.** Read `references/rounds.md` for the per-round prompts and the subagent
type each member runs as. Launch every member of one round in a single message, one Agent call
each. Print each round before the next round starts.

**7 — Check every round with the clerk.** Launch `council-clerk` on the round text as soon as the
round is printed. It resolves every citation the members wrote and reports the claims the evidence
does not support. Apply its verdicts to the transcript before the next round runs, so round 2
challenges a corrected round 1. `references/rounds.md` carries the clerk prompt.

**8 — Write the synthesis.** Read `references/output-format.md`. The synthesis ends with one
recommendation and the minority position, named and attributed to the member who holds it. When the
recommendation turns on an `[unverified]` claim, `ask` once: settle it now, or leave it open and say
so.

## Evidence rule

A council member states a position. The evidence rule decides which of its claims survive.

- **The topic is a repository, a dataset or a document.** A member cites a file path, a line
  number, a command output or a quoted figure for every factual claim.
- **The topic is a choice about the world** — a trip, a portfolio, a purchase. A member cites the
  fare, the fact sheet, the schedule or the rule text that it read.
- A claim with no pointer is marked `[unverified]` in the transcript, at the claim.
- Keep the marked claim. Do not delete it and do not repair it. A load-bearing claim that nobody
  can check is itself a finding, and the synthesis reports it as one.
- A member marks its own claims, so a member that wants to win simply does not mark them. The
  `council-clerk` in step 7 makes the rule real: it re-reads every pointer, returns one verdict per
  claim, and overrides the member's own mark in both directions.

## Error handling

- **Every member agrees in round 1.** Say so, stop, and report the agreement. A debate with no
  disagreement is a receipt, not a deliberation. The verdict is `recompose`, not a recommendation.
- **An option has no advocate, noticed after round 1.** Stop and report `recompose`. Do not report a
  recommendation: that option lost because nobody argued for it, and a transcript that reads as a
  decision would hide the reason.
- **The clerk returns anything but `resolves`.** Apply the verdict per the table in
  `references/rounds.md`: the transcript keeps the claim alongside what the source actually says, and
  the synthesis reports it. A member whose evidence says less than it claimed is a finding.
- **The harness has no `council-*` subagent types.** They ship in this Pack's `agents/` directory,
  and a harness installs them only if it has native subagents (Claude Code) or the `pi-subagents`
  extension plus `tools/packs-pi deploy deliberation` (pi). Without them, fall back to
  `general-purpose` for every member and for the clerk, and paste the read-only contract into each
  prompt: read only, never edit, cite or mark, return the round text alone.
- **The run is on pi.** The member types exist, but their files carry no `model:` pin and inherit the
  session model, so every member runs on one tier. Say so in the header: a sceptic on the same model
  as the members it judges is still worth convening, and it is not the instrument the README describes.
- **A member argues a position it was not given.** Keep the argument and record the drift. Do not
  re-run the member to make it stay in role.
- **The caller wants an attack, not a comparison.** Stop and use `red-team`.
