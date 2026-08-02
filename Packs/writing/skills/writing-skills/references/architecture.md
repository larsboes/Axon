# How Skills Load (architecture)

## Contents
- Three levels of loading
- Filesystem model / progressive disclosure
- Where skills run (surface differences)
- Runtime/network constraints
- Security

## Three levels of loading
| Level | Content | When loaded | Token cost |
|-------|---------|-------------|-----------|
| 1 Metadata | `name` + `description` (YAML frontmatter) | always, at startup, into system prompt | ~100 / skill |
| 2 Instructions | SKILL.md body | when the skill triggers | < ~5k |
| 3 Resources | `references/`, `scripts/`, `assets/` | only when Claude reads/runs them | ~unlimited |

Design implication: put the trigger in Level 1, the workflow in Level 2, and everything bulky in Level 3.

## Filesystem model / progressive disclosure
A skill is a directory on the execution VM. When triggered, Claude reads SKILL.md via bash; referenced files are read only when needed; **scripts are executed via bash and their source never enters context — only their stdout does.** This is why bundled reference docs and datasets have no context cost until opened, and why scripts are cheaper than regenerating code.

## Where skills run (surface differences)
- **Claude Code**: custom skills only, filesystem-based at `~/.claude/skills/<name>/` (personal) or `<project>/.claude/skills/<name>/`. Full network access (same as any local program). Install packages locally, never globally. This is the surface these skills target.
- **Claude API**: pre-built + custom (uploaded via `/v1/skills`), run inside the code-execution tool container. **No network, no runtime package install** — only pre-installed packages.
- **claude.ai**: pre-built + custom (uploaded as zip in Settings > Features). Network access varies by admin/user setting.

Skills do **not** sync across surfaces — upload/manage each separately.

## Runtime/network constraints (plan for the target surface)
If a skill must run on the API, it cannot fetch URLs or `pip install` at runtime — bundle everything and rely only on pre-installed packages. On Claude Code it can, but should keep installs local to the project.

## Security
Treat installing a skill like installing software — only from trusted sources. A malicious skill can direct Claude to misuse tools (file ops, bash, code exec), exfiltrate data, or act outside its stated purpose. Audit every bundled file (SKILL.md, scripts, assets) for unexpected network calls or file access. Skills that pull from external URLs are highest-risk: fetched content can carry injected instructions, and trusted dependencies can be compromised over time.
