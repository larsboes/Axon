# axon pack

The skill for operating and changing Axon itself. One skill, **`axon`**, with two modes it
routes between before reading anything else:

- **Work with Axon** — discover and operate running capabilities, feeds, APIs and health
  surfaces. Every changing fact (port, service state, graph count, issue state) is derived at
  runtime through `scripts/axon-context` and `scripts/axapi`, never remembered.
- **Work on Axon** — understand, review, plan or change the repository. Orientation first, then
  exactly one leaf for the task at hand: architecture, placement, dependencies, documentation,
  verification, issues, or data boundaries.

Everything it knows about this machine it asks for. The registry owns service identity, ports,
health paths and dependencies, so the skill carries no endpoint table that could go stale, and
no path, host or key of its own.

## Why it is a pack

It lived only in `~/.claude/skills/axon` until 2026-08-09, which meant the skill that operates
the entire estate was the one thing in that estate under no version control at all. A LifeOS
reinstall would have kept it (the installer never overwrites), but a machine failure would not.
It is public-safe by construction — 16 files, no private value in any of them — so it belongs in
Axon beside the other packs rather than in the private overlay.

## Activate
```bash
"$AXON_ROOT/tools/packs.sh" link axon   # → ~/.claude/skills/axon
```
