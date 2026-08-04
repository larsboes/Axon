# Axon Pack

One progressively disclosed skill teaches assistants both how to operate Axon and how to work on
the Axon repository. It routes first, derives current context from Axon's own tools, then loads
only the doctrine and workflow needed for the selected task.

## Why this shape: one router over dynamic Axon context

`axon` replaces the narrower `axon-operate` skill. The operating workflow survives through
registry-backed `axapi`; repository work gains bounded orientation, placement, documentation,
verification, and issue/PR branches without loading the whole doctrine into every session.

Static references contain only slow-changing boundaries and decision tests. Capability inventory,
ports, health, graph size, repository state, and GitHub metadata are queried at runtime through
`axon-context`, `tools/self`, the capability registry, doctor, and GitHub. Graphify remains optional
symbol-level drill-down.

Inside this repository, `scripts/axon-context` and `scripts/axapi` are stable entrypoints that
delegate to the Pack-owned implementations. Installed Pack copies continue to use their local
scripts directly, so the workflow stays portable across harnesses.

The Pack is the harness-neutral source. Harness adapters may materialize or link it, but they may
not fork its workflow. Installed copies are deployment artifacts, never editing targets.

## Activation

Use the harness adapter appropriate to the assistant. Current adapter commands and drift behavior
come from `tools/packs.sh` and `tools/packs-codex`; assistant detection and broader deployment are
tracked separately from this skill migration.

## Skill

| Skill | What it does |
| --- | --- |
| `axon` | Operate capabilities or work on the repository through bounded, dynamically resolved context |

## Attribution

Self-authored. No external code or skill text adapted.
