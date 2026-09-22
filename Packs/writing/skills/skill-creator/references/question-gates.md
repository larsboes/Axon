# Question gates — asking the user as pipeline machinery

How a skill asks its user questions: deliberately, batched, with concrete options, and with
the answers persisted as artifacts the workflow can verify. Idea-source: Deutsche Telekom's
better-code `one-modernizer` demo (MIT), rewritten; no text retained.

## Contents
- When to gate (and when not to)
- The resolution ladder — exhaust these before asking
- Question grammar
- Escape-hatch defaults ("just figure it out")
- Persist answers as artifacts
- The post-artifact validation gate
- Centralize the policy

## When to gate (and when not to)

Gate on **material ambiguity**: the answer would change what gets built (target stack, scope
boundary, destructive-action confirmation, which of two contradictory requirements wins). Do
not gate on questions with a conventional default, facts the skill can verify itself, or
preferences already recorded in project config — asking those is ceremony that trains the user
to ignore questions.

## The resolution ladder — exhaust these before asking

1. **Context.** The conversation, the request, the skill's own inputs.
2. **Source.** Verify against the live system — read the file, run the read-only probe, cite
   `file:line`. Docs and memory are stale by default.
3. **Ask** — only what survived 1 and 2, batched into one round.

A skill body states the ladder once, at the point where ambiguity is likely (usually the first
step), not on every step.

## Question grammar

- **Numbered questions, enumerated options.** Each question offers concrete options inline —
  never an open-ended "what do you want?". Put the recommended option first and mark it.
- **One round per gate.** Batch every question the phase needs into a single round; a phase
  that asks one question per message is an interview, not a gate.
- **Use the structured tool when the harness has one.** In Claude Code that is
  `AskUserQuestion` (≤4 questions per call, 2–4 options each, `multiSelect` where options
  aren't exclusive). Fall back to a numbered markdown list with lettered options elsewhere.
- **Lead with the decision, not the background.** One line of framing per question, maximum.

## Escape-hatch defaults ("just figure it out")

Every question block defines what happens if the user declines to answer or says "figure it
out": the skill proceeds on **named defaults**, and every assumption taken this way is
recorded in the output (an `Assumptions` section, or an unknowns file with `UNKNOWN:` prefixes
and a confidence level). Autonomy without silent guessing. A skill that cannot proceed safely
on defaults says so and stops instead of inventing facts.

## Persist answers as artifacts

Answers are workflow state, not vibes. For multi-phase skills:

- Write the answers to a file the pipeline owns (the reference pattern: `clarifications.json`
  in the workflow's state directory) at the moment they're given.
- Later phases **check for the artifact, not for a memory of the conversation** — a fresh
  session or a subagent picks up the same state.
- Where the harness supports hooks, a PreToolUse check on the state directory can block the
  next phase until the clarifications artifact exists — the question gate becomes mechanically
  enforced rather than advisory. Optional; the artifact convention alone carries most of the
  value.

## The post-artifact validation gate

After producing a substantial artifact the user will build on (a spec, a mapping, a plan),
gate once with three fixed questions before the next phase consumes it:

> Anything missing? Anything wrong? Any corrections?

Fixed wording, one round, then proceed. This catches the errors that are obvious to the user
and invisible to the model, at the cheapest possible moment.

## Centralize the policy

The rules above are policy, not per-skill content. State them **once** — in the pack's shared
reference (this file) or, for a plugin, an output style — and have skill bodies say only
"gate ambiguity per `references/question-gates.md`" plus their phase-specific question list.
Repeating the policy in every skill body is the duplication this pack exists to prevent.
