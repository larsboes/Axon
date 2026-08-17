---
name: axon
description: Operates Axon and guides work on the Axon repository through dynamic discovery, bounded self-context, capability APIs, architecture placement, focused branches, and proportionate verification. Use when operating Axon services or feeds, diagnosing Axon, planning or coding inside the Axon repository, deciding where Axon code belongs, or handling Axon branches and PRs. Do not use for unrelated repositories, direct Home Assistant or device control, or generic prose and skill authoring with no Axon decision.
---

# Axon

Route first, then load only the selected reference. Derive changing facts at runtime; never
substitute a remembered port, inventory, issue state, service status, or graph count.

## Resolve context

Run one bounded context command before reading broadly:

```bash
scripts/axon-context with [capability]
scripts/axon-context on [unit-or-path]
```

If the command cannot find the checkout, work from the Axon repository or set the existing
`AXON_ROOT` environment variable. Read `references/shared-failure-policy.md` when any expected
tool, overlay, service, graph, or remote is unavailable.

## Select one mode

### Work with Axon

Use this mode to discover or operate running capabilities, feeds, APIs, and health surfaces.

1. Read `references/with-discovery.md`.
2. For reads, writes, ingestion, or operator actions, also read
   `references/with-operations.md`.
3. Read `references/shared-data-boundaries.md` before handling personal, vault, or
   cross-capability data.

### Work on Axon

Use this mode to understand, review, plan, or change the repository.

1. Read `references/on-orientation.md`.
2. Load only the additional leaf matching the task:

| Task | Read |
| --- | --- |
| Architecture or ownership boundary | `references/shared-architecture.md` |
| Data, secrets, provenance, or trust | `references/shared-data-boundaries.md` |
| Decide where code or configuration belongs | `references/on-placement.md` |
| Dependencies, language, shell, or Bazel | `references/on-dependencies-and-build.md` |
| README, generated docs, manifests, Packs, or skills | `references/on-documentation.md` |
| Tests, gates, review, or completion claims | `references/on-verification.md` |
| Branch, commit, or PR | `references/on-changes.md` |

For symbol-level traversal, use Graphify only when a current graph and Graphify integration
are present. Treat it as a drill-down after bounded orientation, never as the bootstrap.

## Invariants

- Preserve unrelated worktree changes and resolve the exact target before editing.
- Keep public code and doctrine in Axon; keep private values and state in the active overlay.
- Never generate, reveal, move, or overwrite secrets without explicit authorization.
- Prefer manifests, registry output, self-model queries, and GitHub metadata over duplicated
  prose.
- Report failed or unavailable checks exactly; do not turn missing optional context into a
  successful validation claim.
