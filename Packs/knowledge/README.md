# knowledge

Knowledge work on the Obsidian vault, for any agent harness (Claude Code, pi, Codex).
The pack is the purpose; `obsidian` is the tool inside it. Eight skills collapsed to
three on 2026-08-22.

Vault-coupled, never vault-hardcoded: the vault root is read from the overlay
(`config/knowledge.toml`, `vault_root`) at runtime, so nothing personal is baked
into a skill.

## Skills

- **`obsidian`** — the single entry point for vault work. Resolves the vault,
  gates every write on `Projects/Soma/Vault-Contract.md`, routes to the syntax it
  needs, and carries the wiki layer (ingest · synthesize · lint) plus `recall`
  over past sessions. Syntax lives in `references/` — `markdown`, `bases`,
  `canvas`, `cli`, `wiki` — read at the point of need, never loaded up front.
- **`teach`** — one-to-one teaching at the measured edge of the student's
  understanding: probe with graded quizzes, plan a dependency path, teach one
  reasoning step at a time. Every write routes through `obsidian`. The graded
  quizzes come from the pack's `quiz` pi extension; without it (other harnesses,
  non-interactive runs) the skill falls back to numbered text questions.
- **`defuddle`** — clean markdown out of a web page. Not vault-coupled itself,
  but it is what the wiki's ingest path calls, and the Obsidian Web Clipper
  feeds the same `Clippings/` folder by another route.

## Why one skill instead of eight

`obsidian-markdown`, `obsidian-bases` and `json-canvas` were pure syntax
references — no procedure, no scripts, no decisions — and each carried a
description broad enough to fire on almost any vault sentence. When one of them
won the trigger instead of `obsidian-ops`, the write happened without the vault
contract ever loading, so the gate that enforces typing and the no-doubling law
was silently skipped. As `references/` they carry the same content at zero
trigger cost, behind the one skill that reads the contract first.

`llm-wiki` was folded and rewritten rather than moved: it prescribed a parallel
`Knowledge/<domain>/llm-wiki/` tree with its own `index.md`, `log.md`,
`sources/`, `entities/` and `concepts/` folders, which contradicts the contract's
placement table and no-doubling law on every point. The vault *is* the wiki.

## Attribution

The syntax references started from [kepano/obsidian-skills](https://github.com/kepano/obsidian-skills)
(MIT, see `LICENSE`) and are edited freely from there — flattened into one skill, frontmatter
dropped so they cannot win a trigger, and rewritten against this vault as they get touched. They
are inspiration and starting material, not a pinned vendor copy: there is no re-sync path and no
obligation to track upstream. `references/wiki.md` is not upstream at all.
