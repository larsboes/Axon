---
name: writing-skills
description: Authors, structures, and audits Claude agent skills (SKILL.md + scripts/references/assets) to Anthropic best-practices, the agentskills.io spec, and local Claude Code conventions. Use when creating a new skill, refactoring one for progressive disclosure, or auditing a skill's metadata/structure. Do not use for README/user docs, non-agentic library code, or MCP-server implementation — for a runbook-style domain skill, model it on a sibling under `.claude/skills/`.
allowed-tools: Read, Write, Edit, Bash
---

# Writing Skills

Build a skill a zero-context mid-level engineer or Sonnet-class model can load and execute correctly on the first try. Discoverable metadata, lean body, deterministic scripts. Ground truth only: verify every path/command against disk before writing it — a wrong runbook is worse than none.

## Before you scaffold
Write a skill when the same instructions get pasted into chat repeatedly, or a CLAUDE.md section has grown into a procedure rather than a fact — a skill's body loads only when triggered, so it costs nothing until used. Don't write one for a single unrepeated task, or to store a fact with no procedure attached (that's a `references/` note inside an existing skill, or `CLAUDE.md` itself, not a new skill).

## Variables — instantiate before writing
| var | fill with | constraint |
|-----|-----------|------------|
| `<name>` | the skill id | `^[a-z0-9]+(-[a-z0-9]+)*$`, ≤64, == its directory basename, no `anthropic`/`claude` |
| `<desc>` | the trigger sentence | 3rd person, ≤1024, **what + "Use when …" + "Do not use for …"** |
| `<dir>` | `.claude/skills/<name>` (project) or `~/.claude/skills/<name>` (personal) | the two discovery roots |
| `<freedom>` | `low` \| `medium` \| `high` per step | fragile/safety → low · preferred-pattern → medium · open-ended → high |
| `<body_max>` | `500` | hard line ceiling for SKILL.md |
| `<ref_depth>` | `1` | references stay this many hops from SKILL.md |
| `<folders>` | `scripts references assets` | the only subdirs, each one level deep |

## Loading model — earn every token
| level | loaded | budget | holds |
|-------|--------|--------|-------|
| 1 metadata | always | ~100 tok/skill | `<name>`+`<desc>`, the trigger |
| 2 body | on trigger | `<<body_max>` lines | workflow + the few rules a smart model can't infer |
| 3 `<folders>` | on read/run | ~∞ (scripts: only stdout costs tokens) | schemas, long docs, deterministic tools |

Bulk lives in Level 3; SKILL.md is its table of contents. Full model + surface differences: `references/architecture.md`.

## Procedure
```
- [ ] 1. <name>+<desc> pass validate_metadata.py
- [ ] 2. <dir>/<folders> scaffolded, name==dir
- [ ] 3. body written: imperative, <ref_depth>-deep refs, <<body_max> lines
- [ ] 4. every fragile step → a scripts/ CLI (args in, stdout/stderr out)
- [ ] 5. every references/checklist.md item passes
```

**1 · Metadata — the highest-leverage field.** `<desc>` is how a model picks this skill from 100+. State what it does, the concrete trigger phrases a user would say, and the negative trigger + which sibling to use instead. Gate it:
`python3 scripts/validate_metadata.py --name "<name>" --description "<desc>" --dir "<dir>"`
→ SUCCESS proceeds; any NAME/DESC/STYLE/DIR/DISCOVERY line → fix that field only, re-run. Discovery patterns + worked examples: `references/best-practices.md` §Discovery. Restricting tools? add `allowed-tools:` (e.g. `Read, Write, Bash(git status:*)`).

**2 · Scaffold.** `<dir>/{<folders>}`, each one level deep. No README/INSTALLATION/CHANGELOG inside a skill — those are the tell of a human doc masquerading as one.

**3 · Body.** Start from `assets/SKILL.template.md`. Third-person imperative. Pick `<freedom>` per step: `low` → exact command + "do not modify"; `medium` → parameterized script; `high` → goal only, trust the model. Over `<body_max>` lines, or carrying a large schema? Move it to `references/` and command the read at point of need ("Read `references/api.md` to pick the endpoint"), never deeper than `<ref_depth>` — a nested reference gets `head`-previewed and half-read. Picking a workflow shape (sequential steps, MCP-tool coordination, iterative refinement, decision-tree tool selection, embedded domain expertise): `references/patterns.md`.

**4 · Scripts.** One fragile task (regex, parsing, multi-step, safety-critical) per CLI. Takes args. **Solves, never punts** — handle the error in the script, don't fail to Claude. No magic constants. Specific stdout on success, specific stderr on failure so the agent self-corrects. State intent in the body: "Run `x.py`" (execute) vs "See `x.py` for the algorithm" (read).

**5 · Audit.** Every item in `references/checklist.md`. Quality-critical skill? Evaluation-first: run the task with no skill, capture the exact failure, then write only enough to close it — never pad docs against imagined needs. Three-tier test framework (triggering / functional / performance-comparison) and the under/over-triggering fix table: `references/evaluation.md`.

**Claude Code frontmatter/runtime beyond the basics** (`context: fork`, dynamic `!command` injection, string substitutions, skill stacking, nested/monorepo discovery, `skillOverrides`): `references/claude-code-extensions.md`.

## When NOT to use
Not for README/user docs, library code, or MCP-server code. For a domain runbook, don't start from scratch — copy the closest sibling skill's shape.

## Bootstrapping a whole library
Different job from the rest of this skill: an existing project has *no* skill library yet, and the ask is the whole set (10+ skills), not one. Discover-then-parallel-author-then-adversarial-review workflow: `references/bootstrap-library.md`.

## Provenance and maintenance
Re-verify when the environment drifts:
- discovery roots: `ls -d ~/.claude/skills <project>/.claude/skills 2>/dev/null`
- validator still green on this skill: `python3 scripts/validate_metadata.py --file SKILL.md --dir "$(pwd)"`
- rule sources, distilled into `references/` (never cited live from the gitignored staging dir they arrived in) — re-distill if these change, and re-check the live doc first since Anthropic's guides are versioned and this is a point-in-time snapshot:
  - `references/best-practices.md`, `references/architecture.md`, `references/checklist.md`, the "Before you scaffold" heuristic above: Anthropic's [Agent Skills overview](https://platform.claude.com/docs/en/agents-and-tools/agent-skills/overview) + [authoring best practices](https://platform.claude.com/docs/en/agents-and-tools/agent-skills/best-practices), and the [agentskills.io](https://agentskills.io) `skill-creator` spec, absorbed as of 2026-07-09.
  - `references/claude-code-extensions.md`: Anthropic's [Extend Claude with skills](https://code.claude.com/docs/en/skills) doc, absorbed 2026-07-10.
  - `references/evaluation.md`: the testing chapter of a third-party PDF skill-authoring guide (unknown author/license — ideas only, no text retained verbatim) plus Anthropic's evaluation-driven-development section, absorbed 2026-07-10.
  - `references/bootstrap-library.md`: a one-shot "bootstrap a whole skill library" mega-prompt found alongside the PDF (unknown author/license — same idea-only treatment), absorbed 2026-07-10.
  - `references/patterns.md`: the "Patterns and troubleshooting" chapter of the same third-party PDF (idea-only, same treatment), absorbed 2026-07-10.
- **This distillation is a snapshot, not a live mirror.** For anything not covered here, or to confirm a rule still holds, read the source directly rather than assuming this file caught everything: [Agent Skills overview](https://platform.claude.com/docs/en/agents-and-tools/agent-skills/overview) · [authoring best practices](https://platform.claude.com/docs/en/agents-and-tools/agent-skills/best-practices) · [Extend Claude with skills](https://code.claude.com/docs/en/skills) · [agentskills.io](https://agentskills.io).
