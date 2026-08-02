# Claude Code skill extensions

## Contents
- Frontmatter fields beyond name/description
- Dynamic context injection (`!command`)
- String substitutions
- Skill stacking
- Discovery: nested skills, live-reload, hiding a skill you don't own
- Description budget: why a "good" description can still get truncated

## Frontmatter fields beyond name/description
All optional; add only the ones a skill actually needs.

| Field | Use for |
|-------|---------|
| `disable-model-invocation: true` | Side-effecting workflows only the human should trigger (deploy, commit, send-message). Hides the description from context too. |
| `user-invocable: false` | Background knowledge Claude should know but `/name` isn't a meaningful user action. |
| `allowed-tools` | Pre-approve tools while the skill is active (space/comma/YAML list). Does not restrict — every tool stays callable, this only skips the permission prompt for the listed ones. |
| `disallowed-tools` | Remove tools from the pool while active (e.g. no `AskUserQuestion` in an unattended loop). Clears on the next message. |
| `model` / `effort` | Override for the rest of the turn only, not persisted. `effort`: low/medium/high/xhigh/max. |
| `context: fork` + `agent:` | Run in an isolated subagent — no conversation history, CLAUDE.md still loads (except `Explore`/`Plan` agents, which skip it). Only makes sense when the skill body is a self-contained task, not "reference conventions." |
| `paths` | Glob patterns — auto-load only when working with matching files (monorepo package-scoped skills). |
| `arguments` | Named positional args (`arguments: [issue, branch]`) → `$issue`, `$branch` placeholders. |
| `hooks` | Lifecycle hooks scoped to this skill only. |

## Dynamic context injection (`!command`)
`` !`<command>` `` at line-start (or after whitespace) runs the shell command **before** Claude ever sees the skill — the output replaces the placeholder as plain text. This is preprocessing, not something Claude executes; it happens once, and the output is never re-scanned for further placeholders. Multi-line: a ` ```! ` fenced block instead of the inline form. Killable repo-wide via `disableSkillShellExecution` in settings.

Use for: pulling live state (`git diff`, `gh pr view`) into the prompt so the skill's instructions arrive already grounded in the current tree, instead of asking Claude to fetch it itself.

## String substitutions
`$ARGUMENTS` (full arg string) · `$ARGUMENTS[N]` / `$N` (positional) · `$name` (from the `arguments:` list) · `${CLAUDE_SESSION_ID}` · `${CLAUDE_EFFORT}` · `${CLAUDE_SKILL_DIR}` (the skill's own directory — use this, not a relative path, when a bash injection needs to find a bundled script regardless of install location) · `${CLAUDE_PROJECT_DIR}`. Escape a literal `$1` as `\$1`.

## Skill stacking
`/code-review /fix-issue 123` loads both skills in one message; trailing text becomes `$ARGUMENTS` for each. Expansion stops at the first token that isn't an inline user-invocable skill (a `context: fork` skill, or one whose own arguments look like a slash command) — that token and everything after become argument text instead. Cap: first skill + 5 more stacked after it.

## Discovery: nested skills, live-reload, hiding a skill you don't own
- **Monorepo nesting.** `.claude/skills/` loads from every parent directory up to repo root, and from nested directories on demand when Claude touches files there. A name collision resolves to `dir/path:name`; the unqualified name still triggers the project-root skill AND appends a note to also invoke any directory-qualified variant matching the files in play.
- **Live-reload.** Editing an existing `~/.claude/skills/**/SKILL.md` takes effect within the session. Adding a brand-new top-level skills directory needs a session restart to start being watched.
- **`skillOverrides` (settings, not frontmatter).** Use when you want to change a skill's visibility without editing its `SKILL.md` — e.g. a project-checked-in or MCP-provided skill. Four states: `on` / `name-only` (saves listing budget) / `user-invocable-only` (hidden from Claude, still typeable) / `off` (fully hidden). `/skills` menu writes this for you.

## Description budget: why a "good" description can still get truncated
Every skill's `name` + `description` load at startup regardless of trigger — that listing shares a fixed character budget (roughly 1% of the model's context window). When the install has enough skills to overflow it, the *least-invoked* skills lose their description text first (dropping to name-only), not the newest or the longest. A skill that passes every metadata check can still fail to trigger simply because it's rarely used and got budget-starved by more popular siblings. `/doctor` reports how many descriptions are currently shortened/dropped. If a skill in a large install seems to have "stopped working," check that before rewriting its description.

## Evals (skill-creator plugin)
If `skill-creator` is installed (`/plugin install skill-creator@claude-plugins-official`), it automates: `evals/evals.json` test cases → isolated per-case subagent runs → `grading.json` pass/fail → `benchmark.json` with-vs-without-skill comparison (pass rate, tokens, time) → description-tuning by measuring should/should-not-trigger hit rate. Without the plugin, do the same by hand: write 3+ realistic prompts, run with and without the skill in a fresh session each time, compare — see `references/evaluation.md`.
