# Claude Code skill extensions

Snapshot of code.claude.com/docs/en/skills as of 2026-08-09. Version-gated features note
their minimum Claude Code release.

## Contents
- Frontmatter fields beyond name/description
- Skill content lifecycle (persistence, dedupe, compaction)
- Dynamic context injection (`!command`)
- String substitutions
- Skill stacking
- Discovery: nested skills, symlinks, live-reload, skill-dir plugins
- Description budget and `when_to_use`
- Bundled run/verify skills
- Evals via the skill-creator plugin

## Frontmatter fields beyond name/description
All optional; add only what the skill needs.

| Field | Use for |
|-------|---------|
| `when_to_use` | Extra trigger context appended to the description in the listing. The combined `description` + `when_to_use` text is truncated at 1,536 chars — put the key use case first. |
| `disable-model-invocation: true` | Side-effecting workflows only the human should trigger. Hides the description from context. As of v2.1.196 also blocks scheduled tasks firing the skill. |
| `user-invocable: false` | Background knowledge that isn't a meaningful `/name` action. |
| `allowed-tools` | Pre-approve tools while active (does not restrict — only skips prompts). `${CLAUDE_PROJECT_DIR}` resolves inside rules as of v2.1.196. |
| `disallowed-tools` | Remove tools from the pool while active (e.g. no `AskUserQuestion` in an unattended loop). Clears on the next user message. |
| `model` / `effort` | Override for the rest of the turn only. `effort`: low/medium/high/xhigh/max. |
| `context: fork` + `agent:` | Run the body as a subagent's task in isolation. Only for self-contained task content — reference conventions forked alone return nothing useful. `Explore`/`Plan` agents skip CLAUDE.md. |
| `paths` | Glob patterns: auto-load only when working with matching files (monorepo scoping). |
| `arguments` | Named positional args (`arguments: [issue, branch]` → `$issue`, `$branch`). |
| `hooks` | Lifecycle hooks scoped to this skill only. |
| `shell` | `bash` (default) or `powershell` for `!command` blocks (Windows, gated by env var). |

## Skill content lifecycle (persistence, dedupe, compaction)
Invoked skill content enters the conversation **once and stays for the session** — write
standing instructions, not one-time steps; every line is a recurring token cost. Re-invoking
with identical rendered content adds a short already-loaded note instead of a second copy
(v2.1.202); changed arguments or dynamic-context output append the full new render.
Auto-compaction re-attaches the most recent invocation of each skill — first 5,000 tokens per
skill, 25,000-token shared budget, most-recently-invoked first — so older skills can drop
entirely; re-invoke after compaction if a large skill must stay authoritative.

## Dynamic context injection (`!command`)
`` !`<command>` `` at line start (or after whitespace) runs **before** the model sees the
skill; output replaces the placeholder as plain text, once, never re-scanned. Multi-line: a
` ```! ` fenced block. Use to arrive pre-grounded (`git diff`, `gh pr view`) instead of asking
the model to fetch. Killable via `disableSkillShellExecution` in settings.

## String substitutions
`$ARGUMENTS` · `$ARGUMENTS[N]` / `$N` · `$name` (from `arguments:`) · `${CLAUDE_SESSION_ID}` ·
`${CLAUDE_EFFORT}` (ultracode reports as `xhigh`) · `${CLAUDE_SKILL_DIR}` (the skill's own
directory — use for bundled scripts regardless of install location) · `${CLAUDE_PROJECT_DIR}`
(project root, v2.1.196+). Escape a literal `$1` as `\$1`.

## Skill stacking
`/a /b 123` loads both skills; trailing text becomes `$ARGUMENTS` for each. First skill plus
up to 5 more (v2.1.199+). Expansion stops at the first token that isn't an inline
user-invocable skill — forked skills and slash-looking arguments end the run there.

## Discovery: nested skills, symlinks, live-reload, skill-dir plugins
- **Precedence**: enterprise > personal (`~/.claude/skills/`) > project (`.claude/skills/`);
  any of them overrides a bundled skill of the same name. Plugin skills are namespaced
  (`plugin:skill`) and cannot collide.
- **Monorepo nesting**: `.claude/skills/` loads from every parent up to repo root and from
  nested dirs on demand. A name clash yields `dir/path:name`; invoking the unqualified name
  also instructs use of any directory-qualified variant matching the files in play (v2.1.203).
- **Symlinks**: a `<skill-name>` entry may be a symlink to a directory elsewhere; followed and
  deduped. This is how pack adapters link skills in.
- **Live-reload**: edits to existing SKILL.md files apply within the session; a brand-new
  top-level skills directory needs a restart.
- **Skill-dir plugins**: a `.claude-plugin/plugin.json` inside a skill folder makes it load as
  a plugin, able to bundle agents, hooks, and MCP servers (project dirs need workspace trust).
- **`skillOverrides` (settings)**: per-skill visibility without editing files you don't own —
  `on` / `name-only` / `user-invocable-only` / `off` (v2.1.199: `off` hides from remote/SDK
  callers too).

## Description budget and `when_to_use`
Every skill's name + description load at startup under a shared character budget (~1% of the
context window; raise via `skillListingBudgetFraction` or `SLASH_COMMAND_TOOL_CHAR_BUDGET`).
On overflow, the least-invoked skills lose their descriptions first — a rarely-used skill can
silently drop to name-only and "stop triggering". `/doctor` reports shortened/dropped entries.
Per-entry combined description+when_to_use text caps at 1,536 chars
(`skillListingMaxDescChars`). In large installs, prefer trimming and `name-only` overrides for
low-priority skills over raising the budget.

## Bundled run/verify skills
`/run` and `/verify` launch and check the app; `/run-skill-generator` records a per-project
launch recipe as `.claude/skills/run-<name>/` that both then follow (v2.1.145+). When authoring
a skill for a project with non-trivial launch steps, generate the run skill instead of
re-explaining the launch in prose.

## Evals via the skill-creator plugin
`/plugin install skill-creator@claude-plugins-official` automates the eval loop: `evals/evals.json`
test cases → isolated per-case subagent runs → `grading.json` → `benchmark.json`
(with/without-skill pass rate, tokens, time) → blind A/B between versions → description tuning
via should/should-not-trigger hit rates. Without the plugin, run the same loop by hand:
`references/evaluation.md`.
