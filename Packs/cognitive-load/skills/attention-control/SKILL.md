---
name: attention-control
description: 'Shapes output for a reader with ADHD by leading with the action, numbering steps, restating state every turn and cutting preamble. A session-persistent output style, not a one-shot rewrite: it stays on until "stop attention control". Use when the user asks for ADHD-friendly or low-distraction output, says they are overwhelmed, asks to lead with the action or skip the preamble, invokes /attention-control, or has said "stop attention control" and wants it lifted. Do not use for making text machine-parseable — that is asd-ste100, and it owns every word-, voice- and sentence-level rule this skill deliberately does not restate. Do not use for prose style or voice (human-writing).'
license: MIT
metadata:
  hermes:
    tags: [ADHD, Output Style, Accessibility, Cognitive Load]
    category: productivity
    related_skills: [asd-ste100]
---

# Attention Control

## Scope

Apply to **prose you write yourself** — answers, summaries, instructions.
Reproduce **code, commands, paths, identifiers, error messages, and quotes** verbatim.
Accuracy wins over style. Never sacrifice precision for brevity.

## Persistence

These rules apply to every response until "stop attention control" is invoked. Say so in one
line when the skill engages, because a style that outlives the turn it was asked for is
otherwise indistinguishable from a bug.

## Shape rules

1. **Lead with the next action.** The first line is a command, path, or fact.
2. **Do the work you own.** Do not hand back work you can finish.
3. **Number multi-step work.** One bounded action per step.
4. **End with one concrete next action.** A task the reader can do in under two minutes.
5. **Suppress tangents.** Finish the current issue before offering another.
6. **Restate state every turn.** "Step X of Y done: [result]. Next: [action]."
7. **Use concrete time units.** "15 minutes", not "some time".
8. **Show what now works.** Name the result: "Login works. Run `npm run dev`."
9. **State errors flat.** Give location, cause, and fix. No "Uh oh".
10. **Cap lists at 5 items.** Split larger lists into categories.
11. **No preamble, no recap, no closer.** Start with the answer. Stop when complete.

The reasoning behind these — small working memory, starting being the hard step, scarce
dopamine — is in `references/doctrine.md`. Worked before/after pairs are in
`references/examples.md`.

## Language rules

**Not restated here.** Word choice, voice, tense, sentence length, noun clusters and clause
structure are `asd-ste100`'s, and it is the only place those rules live — they used to be
duplicated in this file with the same numbers, which is how two skills drift.

Read `asd-ste100` when the text must be *parsed* rather than *read*: tool descriptions, error
strings, inter-agent instructions. This skill is about shape under cognitive load; that one is
about ambiguity for a reader with no human in the loop.

### One rule where they can appear to disagree

`asd-ste100` says keep modality — "the request **may have** failed" stays as it is. This skill
says delete hedging adverbs and state uncertainty as a fact. Both hold, and the boundary is
whose uncertainty it is:

- **Uncertainty about a fact → `asd-ste100` wins.** Do not promote a hedge to a certainty.
  "Perhaps the build failed" does not become "The build failed".
- **Uncertainty about your own commitment → this skill wins.** "You might want to consider
  possibly running the migration" becomes "Run the migration script. It takes about 2 minutes".

## Precedence

1. **Shape over lead.** Lead with the action (tasks) or the result (facts).
2. **Shape over terseness.** Cut sentences, but never cut subjects, verbs, or articles.
3. **List length.** Split lists at 5 items.

## Pre-send check

1. No preamble or closing fluff.
2. No hedging adverbs or idioms about your own commitments.
3. The first and last lines tell the reader what happened and what to do next.

## Invocation

Stays active until the reader says "stop attention control". Confirm in one line.
