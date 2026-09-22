# Skill Validation Checklist

Final audit before a skill is done. Every item must pass.

## 1. Metadata & discovery
- [ ] `name` is 1–64 chars, lowercase/digits/single-hyphens, no reserved words (`anthropic`, `claude`).
- [ ] `name` exactly matches the parent directory name.
- [ ] `description` ≤ 1024 chars, no XML tags, third person (no I/me/my/we/our/you/your).
- [ ] `description` states what it does **and** "Use when..." **and** a negative trigger ("Do not use for...").
- [ ] `description` contains **no workflow summary** — triggers only, or the model follows the summary instead of the body.
- [ ] `when_to_use` (if present): combined with `description` stays ≤ 1536 chars, key use case first.
- [ ] `scripts/validate_metadata.py --file SKILL.md` prints SUCCESS.

## 2. Structure & paths
- [ ] Only `scripts/`, `references/`, `assets/` — each exactly one level deep.
- [ ] No `README.md` / `INSTALLATION.md` / `CHANGELOG.md` inside the skill.
- [ ] All paths in SKILL.md use forward slashes.
- [ ] References are one level deep from SKILL.md (no reference-to-reference chains).

## 3. Logic & instructions
- [ ] SKILL.md body < 500 lines.
- [ ] Instructions are third-person imperative ("Extract", "Run", "Validate").
- [ ] Degrees of freedom match task fragility (low for fragile/safety-critical, high for open-ended).
- [ ] Large schemas/rule-sets live in `references/` or `assets/`, read at point of need.
- [ ] Consistent domain terminology throughout.
- [ ] Reference files > 100 lines have a table of contents.

## 4. Scripts & determinism
- [ ] Scripts are tiny single-purpose CLIs that take arguments.
- [ ] Scripts solve errors (don't punt); no magic constants.
- [ ] Descriptive stdout on success, specific stderr on failure (self-correction loop).
- [ ] Execute-vs-read intent is explicit in SKILL.md.
- [ ] Required packages listed; no assumption they're pre-installed (esp. for API surface).

## 5. Error handling & evaluation
- [ ] SKILL.md has an "Error handling" section for common failure states.
- [ ] Validation/verification steps exist for critical or destructive operations.
- [ ] `evals/evals.json` exists with ≥ 2 realistic cases, one of them an edge/ambiguity case.
- [ ] Trigger tests include ≥ 1 **near-miss** should-NOT-trigger prompt (boundary-ambiguous, sibling-skill territory).
- [ ] A baseline (RED) run without the skill was captured before instructions were written or fattened.

## 6. Current-model rules
- [ ] No instruction tells the model to echo/transcribe/explain its internal reasoning (refusal risk — see `references/fable-5-authoring.md`).
- [ ] Every line survives the de-prescription test: a current model would get it wrong without the line (or it's marked "kept for small-model correctness").
- [ ] Ambiguity path defined: a question gate per `references/question-gates.md`, or named escape-hatch defaults that get documented in output.
- [ ] Long-running/autonomous skills carry a ground-progress-claims block.
- [ ] Optional `Boundaries (Will / Will not)` block, if present, agrees with the description's negative triggers.
