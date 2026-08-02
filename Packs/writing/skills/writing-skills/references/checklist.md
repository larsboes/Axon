# Skill Validation Checklist

Final audit before a skill is done. Every item must pass.

## 1. Metadata & discovery
- [ ] `name` is 1–64 chars, lowercase/digits/single-hyphens, no reserved words (`anthropic`, `claude`).
- [ ] `name` exactly matches the parent directory name.
- [ ] `description` ≤ 1024 chars, no XML tags, third person (no I/me/my/we/our/you/your).
- [ ] `description` states what it does **and** "Use when..." **and** a negative trigger ("Do not use for...").
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
- [ ] At least one evaluation exists (task that failed without the skill, passes with it).
