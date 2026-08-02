# knowledge-graph

Interactive browser view and REST API for the Axon code-dependency graph built by
[graphify](https://github.com/safishamsi/graphify).

## What it serves

**Browser view** — an interactive vis-network graph of the full codebase, accessible
from the dashboard's Projects page. Search nodes, filter by community, inspect
relationships.

**REST API** — the same graph data programmatically, for AI agents and scripts:

| Endpoint | Description |
|---|---|
| `GET /api/graph` | Full graph (nodes + edges) |
| `GET /api/graph/stats` | Summary stats (node/edge/community counts) |
| `GET /api/graph/search?q=<term>` | Search nodes by label or file type |
| `GET /api/graph/community/<id>` | Nodes and edges in one community |
| `GET /api/graph/node/<id>` | One node with its connections |

## How it connects to graphify

`tools/graphify.sh` builds `graphify-out/graph.json`, `graph.html`, and
`GRAPH_REPORT.md` at the repo root. This capability reads `graph.json` live on every
API call — no data duplication, always the current graph.

Rebuild the graph with:

```bash
tools/graphify.sh
```

## Run

```bash
tools/service-runner.sh start knowledge-graph   # :4244, or open its panel in the dashboard
```

Directly, for development:

```bash
cd capabilities/knowledge-graph/ui
bun install
bun run build                         # build the Svelte static UI
cd ../..
bun capabilities/knowledge-graph/server.ts  # serve on AXON_PORT (default 4244)
```

## Ownership boundary

This capability owns the serving layer only — the UI, the API, and the service
manifest. graphify owns the extraction, community detection and graph data format.
The `graphify-out/` directory is gitignored and machine-specific (absolute paths in
node IDs); this capability reads it at runtime and never commits it.

Future: cross-reference `self.json` to annotate nodes with code size per unit,
service health, and upstream pins.
