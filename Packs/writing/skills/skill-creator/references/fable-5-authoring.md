# Authoring for current models (Claude 5 era)

Rules that did not exist when most skills were written. Source: Anthropic's model-specific
prompting guides (prompting-claude-fable-5, claude-prompting-best-practices) and the Claude 5
context-engineering results, snapshot 2026-08-09.

## Contents
- De-prescription — the default direction is cutting
- Never instruct reasoning-echo (refusal risk)
- Ground progress claims (long-running skills)
- Damping blocks worth copying
- Cross-model floor

## De-prescription — the default direction is cutting

Skills written for prior models are often too prescriptive for current ones and can *degrade*
output; Anthropic removed over 80% of Claude Code's own system prompt without measurable loss.
Audit every line of an existing skill with one test: **would a current model get this right
with the line deleted?** If yes, delete it. Concretely:

- Replace blanket defaults ("always run X first") with targeted conditions ("run X when Y").
- Remove undertrigger workarounds ("if in doubt, use the tool") — they now cause overtriggering.
- Cut intensifiers (`MANDATORY`, `CRITICAL`, a rule restated three times). Stating a rule once
  is the instruction; repetition adds volume, not constraint.
- Cut choreography of the model's reasoning (numbered thinking steps for open-ended work).
  Keep the four legitimate HOW classes: safety gates, verified gotchas, exact tool invocations,
  output-format contracts.
- Prove the cut with an eval, not a feeling: run the before/after A/B from
  `references/evaluation.md`. If pass rate holds or rises with lines removed, the lines were
  cost, not value.

## Never instruct reasoning-echo (refusal risk)

Instructions telling the model to echo, transcribe, or explain its internal reasoning as
response text ("show your thinking", "reproduce your chain of thought") can trigger the
`reasoning_extraction` refusal category on Fable-class models, causing fallback or refusal.
When auditing an older skill, grep for show-your-thinking phrasing and replace it with:

- a structured *output* contract (the deliverable carries conclusions and evidence, not the
  reasoning transcript), or
- a progress/report tool where the harness provides one.

Asking for justification of a *result* ("state the evidence for each finding") is fine; asking
for the reasoning *process* is the risk.

## Ground progress claims (long-running skills)

For any skill that drives a long or autonomous run, include a grounding block — in Anthropic's
testing this nearly eliminated fabricated status reports:

> Before reporting progress, audit each claim against a tool result from this session. Only
> report work you can point to evidence for; if something is not yet verified, say so
> explicitly. If tests fail, say so with the output; if a step was skipped, say that.

## Damping blocks worth copying

Current models at high effort over-gather and over-build. If a skill's task is bounded, borrow
the applicable damper (adapted from the official guides):

- **Scope**: "Don't add features, refactor, or introduce abstractions beyond what the task
  requires. Do the simplest thing that works well; only validate at system boundaries."
- **Decision commitment**: "Choose an approach and commit. Revisit only on new information
  that directly contradicts it."
- **Temp files**: "If you create temporary scripts or files for iteration, remove them at the
  end of the task."
- **Acting vs assessing**: "When the user is describing a problem or asking a question, the
  deliverable is the assessment. Don't apply a fix until asked."

## Cross-model floor

Pack skills deploy beyond one harness and one model (Claude Code today, Codex via
`packs-codex`, weaker models in subagents). The de-prescription floor is: **the skill must
still be correct on the weakest model that will run it.** Where the strong model needs no
instruction but the weak one does, keep the instruction and mark why ("kept for small-model
correctness"), so the next de-prescription pass doesn't delete it blind. Test with every model
tier the skill actually deploys to (`references/evaluation.md`).
