# Skill Authoring — Best Practices (distilled + reconciled)

Combined from Anthropic's official authoring guide, the agentskills.io skill-creator spec, and local Claude Code/LifeOS conventions. Read the section you need; each is self-contained.

## Contents
- Conciseness — the context window is a shared good
- Discovery — writing the `description` (the single most important field)
- Degrees of freedom — matching specificity to task fragility
- Progressive disclosure — the three patterns
- Workflows & feedback loops
- Content guidelines — terminology, no time-bombs
- Scripts — solve don't punt
- Anti-patterns
- Evaluation & iteration (Claude A / Claude B)

## Conciseness — the context window is a shared good
Only Level-1 metadata is always loaded; SKILL.md loads on trigger and then competes with conversation history. Default assumption: **Claude is already smart.** Challenge every line — "does Claude need this, or can it infer it?" Cut explanations of well-known concepts (what a PDF is, how libraries work). Concise beats complete.

## Discovery — writing the `description`
Claude picks among 100+ skills using only `name` + `description`. Requirements: third person, ≤ 1024 chars, no XML tags, no first/second person. Content formula: **what it does + when to use it (triggers) + when not to (negative triggers)**.

- Good: `Extract text and tables from PDF files, fill forms, merge documents. Use when working with PDFs, forms, or document extraction. Do not use for image-only scans without OCR.`
- Bad: `Helps with documents` / `Processes data` / `Does stuff with files`

Include concrete trigger terms the user is likely to say. Negative triggers prevent the skill from firing on the wrong task. If a skill "never triggers," the description is almost always the cause.

## Degrees of freedom — match specificity to fragility
Think of Claude as a robot on a path:
- **Narrow bridge, cliffs both sides** → one safe way. Low freedom: exact script, "do not modify the command." Use for migrations, destructive ops, safety-critical sequences.
- **Preferred trail** → medium freedom: parameterized script or pseudocode; some variation OK.
- **Open field** → high freedom: describe the goal, trust the model. Use for code review, analysis where context decides.

Over-constraining an open-field task wastes tokens and makes the skill brittle; under-constraining a cliff task causes failures.

## Progressive disclosure — three patterns
1. **High-level guide + references.** SKILL.md is a table of contents; `references/FORMS.md`, `references/REFERENCE.md` load only when that feature is used.
2. **Domain organization.** Split reference files by domain (`reference/finance.md`, `reference/sales.md`) so a sales question never loads finance context. Offer a `grep` hint for search.
3. **Conditional details.** Show the common path inline; link the rare/advanced path ("For tracked changes see `references/redlining.md`").

Rules: keep SKILL.md < 500 lines; keep every reference **one level deep** from SKILL.md (Claude may only `head -100` a nested reference and miss content); give reference files > 100 lines a table of contents so a partial read still reveals scope.

## Workflows & feedback loops
For multi-step work, give a copy-able checklist Claude ticks off as it goes — it prevents skipped validation steps. Build **validator → fix → repeat** loops: after each change, run the check (script or a `references/style-guide.md` comparison) and only proceed when it passes. For batch/destructive work use **plan → validate-plan → execute → verify**: write an intermediate `changes.json`, validate it with a verbose script, then apply — errors are caught before they touch originals.

## Content guidelines
- **No time-bombs.** Don't write "before August 2025 use X." Put deprecated info in a collapsed "Old patterns" section; keep the main body current.
- **One term per concept.** Pick "field" (not field/box/element) and "extract" (not extract/pull/get) and hold it throughout. Consistency helps the model follow.

## Scripts — solve, don't punt
Pre-made scripts beat generated code: more reliable, fewer tokens, consistent. In each script: handle the error (create the missing file, fall back) instead of failing to Claude; document every constant (no `TIMEOUT = 47`); emit specific messages ("Field 'sig_date' not found. Available: ..."). State intent in SKILL.md: **"Run `x.py`"** (execute, the default) vs **"See `x.py` for the algorithm"** (read). Don't assume packages exist — list them and their install. Use `Server:tool_name` fully-qualified names for MCP tools.

## Anti-patterns
- Forward slashes only in paths (`scripts/x.py`), never backslashes.
- Don't offer many options ("use pypdf or pdfplumber or PyMuPDF or..."); give one default + an escape hatch.
- No human docs inside a skill (README/INSTALLATION/CHANGELOG).
- No deeply nested references; no vague names (`helper`, `utils`, `tools`).

## Evaluation & iteration (Claude A / Claude B)
Write evaluations **before** padding docs: run a representative task with no skill, capture the real failure, then write the minimum instruction that fixes it. Develop with two roles — **Claude A** helps author/refine the skill; **Claude B** (fresh instance, skill loaded) runs real tasks; observe B's misses and bring them back to A. Iterate on the observed behavior, not on imagined requirements. Metadata (`name`/`description`) is the highest-leverage thing to tune.
