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

Adapted from the Council skill in LifeOS by Daniel Miessler
(https://github.com/danielmiessler/LifeOS), MIT.

## Procedure

Copy this checklist and track progress:

```
- [ ] 1. State the decision in one sentence
- [ ] 2. Pick DEBATE or QUICK
- [ ] 3. Pick a preset, or compose a council
- [ ] 4. Collect the evidence the members need
- [ ] 5. Run the rounds
- [ ] 6. Write the synthesis
```

**1 — State the decision.** Write the question the council answers and the options on the table.
A question with one factual answer is not a council topic. Answer it directly and stop.

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

**4 — Collect the evidence.** Each preset names what its members demand. Gather it before round 1
and put it in every member prompt. A member that has to guess produces an unverified claim, and
step 6 discards it.

**5 — Run the rounds.** Read `references/rounds.md` for the per-round prompts. Launch every member
of one round in a single message, each as a separate Agent call with
`subagent_type: "general-purpose"`. Print each round before the next round starts.

**6 — Write the synthesis.** Read `references/output-format.md`. The synthesis ends with one
recommendation and the minority position, named and attributed to the member who holds it.

## Evidence rule

A council member states a position. The evidence rule decides which of its claims survive.

- **The topic is a repository, a dataset or a document.** A member cites a file path, a line
  number, a command output or a quoted figure for every factual claim.
- **The topic is a choice about the world** — a trip, a portfolio, a purchase. A member cites the
  fare, the fact sheet, the schedule or the rule text that it read.
- A claim with no pointer is marked `[unverified]` in the transcript, at the claim.
- Keep the marked claim. Do not delete it and do not repair it. A load-bearing claim that nobody
  can check is itself a finding, and the synthesis reports it as one.

## Error handling

- **Every member agrees in round 1.** Say so, stop, and report the agreement. A debate with no
  disagreement is a receipt, not a deliberation.
- **A member cites a file that does not exist.** Drop the claim. Name the member and the path in
  the synthesis.
- **A member argues a position it was not given.** Keep the argument and record the drift. Do not
  re-run the member to make it stay in role.
- **The caller wants an attack, not a comparison.** Stop and use `red-team`.
