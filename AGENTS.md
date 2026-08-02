# Axon agent bootstrap

Use the `axon` skill for every task that operates Axon or changes this repository. Route through
its working-with or working-on workflow and load only the references selected there. Use Graphify
only as optional symbol-level drill-down after bounded Axon context is available.

Before editing, run `git status --short --branch` and `tools/doctor`. Preserve unrelated worktree
changes. Derive capabilities, ports, health, issue state and architecture from Axon tools and live
metadata rather than remembered prose.

Keep public code and doctrine in Axon. Keep private values, machine state and secrets in the active
overlay or Vaultwarden. Never generate, expose or change a secret without explicit authorization.

For planned GitHub work, load the `axon` skill's issue workflow. Start from clean, current `main`;
use one issue branch and one focused draft PR; show scope and completed validation for approval;
merge only after approval; then continue from fresh `main`. Preserve and report failed or
unavailable checks exactly.

## graphify

This project has a knowledge graph at graphify-out/ with god nodes, community structure, and cross-file relationships.

When the user types `/graphify`, use the installed graphify skill or instructions before doing anything else.

Rules:
- For codebase questions, first run `graphify query "<question>"` when graphify-out/graph.json exists. Use `graphify path "<A>" "<B>"` for relationships and `graphify explain "<concept>"` for focused concepts. These return a scoped subgraph, usually much smaller than GRAPH_REPORT.md or raw grep output.
- Dirty graphify-out/ files are expected after hooks or incremental updates; dirty graph files are not a reason to skip graphify. Only skip graphify if the task is about stale or incorrect graph output, or the user explicitly says not to use it.
- If graphify-out/wiki/index.md exists, use it for broad navigation instead of raw source browsing.
- Read graphify-out/GRAPH_REPORT.md only for broad architecture review or when query/path/explain do not surface enough context.
- After modifying code, run `graphify update .` to keep the graph current (AST-only, no API cost).
