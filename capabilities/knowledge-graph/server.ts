#!/usr/bin/env bun
// server.ts — serves the knowledge-graph capability: static Svelte UI (adapter-static)
// plus REST API over graphify-out/graph.json.
//
// Based on tools/panel-server.ts (heartbeat injection, backlink, security model)
// with API routes added for programmatic graph access.
//
// usage: bun capabilities/knowledge-graph/server.ts
// env:   AXON_PORT   the port to bind (exported by tools/service-runner.sh from the manifest)
//        AXON_ROOT   repo root (defaults to cwd)
//
// API endpoints:
//   GET  /api/graph              — full graph (nodes + edges)
//   GET  /api/graph/stats        — node/edge/community counts
//   GET  /api/graph/search?q=    — search nodes by label or file type
//   GET  /api/graph/community/:id — nodes and edges in one community
//   GET  /api/graph/node/:id     — one node with its connections
//
// Importing this file starts nothing: binding the port, resolving the built UI and every
// exit live in main(), behind `import.meta.main`. That is what lets server.test.ts call
// handleAPI against a planted graph without a port, a build, or a running service — a
// module that exits the process on import cannot be tested at all, and the UI's dist/ is
// generated and untracked, so on a fresh clone the old import path would have killed the
// test runner before its first assertion (Axon#116).

import { existsSync, readFileSync, statSync } from "node:fs";
import { join, normalize, resolve, sep } from "node:path";

const HEARTBEAT_INTERVAL_MS = 30_000;

const axonRoot = (): string => process.env.AXON_ROOT ?? process.cwd();
const distDirFor = (root: string): string => resolve(join(root, "capabilities/knowledge-graph/ui/dist"));

// ── Graph data ──────────────────────────────────────────────────────────────

// Keyed on path as well as mtime: the path is derived from AXON_ROOT, so a cache keyed on
// mtime alone can hand one root's graph to another whose file happens to share a timestamp.
let graphCache: { path: string; data: any; mtime: number } | null = null;

export function loadGraph(): any {
  const graphJsonPath = join(axonRoot(), "graphify-out/graph.json");
  if (!existsSync(graphJsonPath)) return null;
  const mtime = statSync(graphJsonPath).mtimeMs;
  if (graphCache && graphCache.path === graphJsonPath && graphCache.mtime === mtime) return graphCache.data;
  const raw = readFileSync(graphJsonPath, "utf-8");
  const data = JSON.parse(raw);
  graphCache = { path: graphJsonPath, data, mtime };
  return data;
}

interface FlatNode {
  id: string;
  label: string;
  file_type: string;
  source_file: string;
  community: number;
  group: string;
}

interface FlatEdge {
  from: string;
  to: string;
  label?: string;
}

export function flattenNodes(graph: any): FlatNode[] {
  const nodes = graph.nodes ?? [];
  return nodes.map((n: any) => ({
    id: String(n.id ?? n.label ?? ""),
    label: n.label ?? n.id ?? "",
    file_type: n.file_type ?? "unknown",
    source_file: n.source_file ?? "",
    community: n.community ?? -1,
    group: n.community !== undefined ? `community-${n.community}` : "uncategorized",
  }));
}

export function flattenEdges(graph: any): FlatEdge[] {
  const links = graph.links ?? graph.edges ?? [];
  return links.map((e: any) => ({
    from: String(e.source ?? e.from ?? ""),
    to: String(e.target ?? e.to ?? ""),
    label: e.label ?? e.relationship ?? "",
  }));
}

function getCommunities(nodes: FlatNode[]): number[] {
  const set = new Set<number>();
  for (const n of nodes) {
    if (n.community >= 0) set.add(n.community);
  }
  return [...set].sort((a, b) => a - b);
}

// ── MIME types (from panel-server.ts) ──────────────────────────────────────

const CONTENT_TYPES: Record<string, string> = {
  ".css": "text/css; charset=utf-8",
  ".html": "text/html; charset=utf-8",
  ".ico": "image/x-icon",
  ".jpg": "image/jpeg",
  ".jpeg": "image/jpeg",
  ".js": "text/javascript; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".map": "application/json; charset=utf-8",
  ".png": "image/png",
  ".svg": "image/svg+xml",
  ".txt": "text/plain; charset=utf-8",
  ".webmanifest": "application/manifest+json",
  ".webp": "image/webp",
  ".woff": "font/woff",
  ".woff2": "font/woff2",
};

const contentType = (path: string): string => {
  const dot = path.lastIndexOf(".");
  return (dot === -1 ? undefined : CONTENT_TYPES[path.slice(dot)]) ?? "application/octet-stream";
};

// ── Heartbeat injection (from panel-server.ts) ──────────────────────────────

const HEARTBEAT = `<script>
(function () {
  var ping = function () {
    if (document.visibilityState !== "visible") return;
    if (navigator.sendBeacon) navigator.sendBeacon("/__axon/ping");
    else fetch("/__axon/ping", { method: "POST", keepalive: true }).catch(function () {});
  };
  ping();
  setInterval(ping, ${HEARTBEAT_INTERVAL_MS});
  document.addEventListener("visibilitychange", ping);
})();
</script>`;

const shellPort = process.env.AXON_SHELL_PORT ?? "";
const BACKLINK = /^\d+$/.test(shellPort)
  ? `<a id="axon-back" href="#" title="Zurück zu Axon">← Axon</a>
<style>
#axon-back {
  position: fixed; left: 1rem; bottom: 1rem; z-index: 2147483647;
  padding: .4rem .7rem; border-radius: 999px;
  font: 500 12px/1 ui-sans-serif, system-ui, sans-serif; text-decoration: none;
  color: #e6f6ff; background: rgba(15, 23, 42, .82); border: 1px solid rgba(56, 189, 248, .45);
  backdrop-filter: blur(6px); opacity: .55; transition: opacity .15s ease;
}
#axon-back:hover, #axon-back:focus-visible { opacity: 1; }
@media print { #axon-back { display: none; } }
</style>
<script>
document.getElementById("axon-back").href =
  location.protocol + "//" + location.hostname + ":${shellPort}/";
</script>`
  : "";

// ── Static file resolution (from panel-server.ts) ──────────────────────────

function resolveFile(urlPath: string, distDir: string): string | null {
  let decoded: string;
  try {
    decoded = decodeURIComponent(urlPath);
  } catch {
    return null;
  }
  const candidate = normalize(join(distDir, decoded));
  if (candidate !== distDir && !candidate.startsWith(distDir + sep)) return null;

  for (const path of [candidate, `${candidate}.html`, join(candidate, "index.html")]) {
    if (existsSync(path) && statSync(path).isFile()) return path;
  }
  return null;
}

function precompressed(path: string, accept: string): { path: string; encoding: string } | null {
  if (path.endsWith(".html")) return null;
  for (const [encoding, ext] of [["br", ".br"], ["gzip", ".gz"]] as const) {
    if (accept.includes(encoding) && existsSync(path + ext)) return { path: path + ext, encoding };
  }
  return null;
}

// ── API handlers ────────────────────────────────────────────────────────────

export function handleAPI(url: URL): Response | null {
  const path = url.pathname;

  // /api/graph — full graph
  if (path === "/api/graph") {
    const graph = loadGraph();
    if (!graph) return new Response(JSON.stringify({ error: "graph not found — run tools/graphify.sh first" }), {
      status: 404,
      headers: { "content-type": "application/json" },
    });
    return new Response(JSON.stringify(graph), {
      headers: { "content-type": "application/json", "cache-control": "no-cache" },
    });
  }

  // /api/graph/stats — summary counts
  if (path === "/api/graph/stats") {
    const graph = loadGraph();
    if (!graph) return json404("graph not found — run tools/graphify.sh first");
    const nodes = flattenNodes(graph);
    const edges = flattenEdges(graph);
    const communities = getCommunities(nodes);
    return jsonOk({
      nodes: nodes.length,
      edges: edges.length,
      communities: communities.length,
      built_at: graph.built_at_commit ?? "unknown",
      corpus_files: nodes.filter(n => n.file_type === "code").length,
      doc_files: nodes.filter(n => n.file_type !== "code").length,
    });
  }

  // /api/graph/search?q=<term>
  if (path === "/api/graph/search") {
    const q = (url.searchParams.get("q") ?? "").toLowerCase().trim();
    if (!q) return json400("missing 'q' query parameter");
    const graph = loadGraph();
    if (!graph) return json404("graph not found");
    const nodes = flattenNodes(graph).filter(n =>
      n.label.toLowerCase().includes(q) ||
      n.source_file.toLowerCase().includes(q) ||
      n.file_type.toLowerCase().includes(q)
    );
    const edges = flattenEdges(graph);
    // Return matching nodes with their edges filtered to those connecting matching nodes
    const nodeIds = new Set(nodes.map(n => n.id));
    const matchingEdges = edges.filter(e => nodeIds.has(e.from) && nodeIds.has(e.to));
    return jsonOk({
      query: q,
      results: nodes.length,
      nodes,
      edges: matchingEdges,
    });
  }

  // /api/graph/community/:id
  const communityMatch = path.match(/^\/api\/graph\/community\/(\d+)$/);
  if (communityMatch) {
    const communityId = parseInt(communityMatch[1], 10);
    const graph = loadGraph();
    if (!graph) return json404("graph not found");
    const nodes = flattenNodes(graph).filter(n => n.community === communityId);
    const edges = flattenEdges(graph);
    const nodeIds = new Set(nodes.map(n => n.id));
    const communityEdges = edges.filter(e => nodeIds.has(e.from) && nodeIds.has(e.to));
    return jsonOk({
      community: communityId,
      members: nodes.length,
      nodes,
      edges: communityEdges,
    });
  }

  // /api/graph/node/:id
  const nodeMatch = path.match(/^\/api\/graph\/node\/(.+)$/);
  if (nodeMatch) {
    const nodeId = decodeURIComponent(nodeMatch[1]);
    const graph = loadGraph();
    if (!graph) return json404("graph not found");
    const nodes = flattenNodes(graph);
    const node = nodes.find(n => n.id === nodeId);
    if (!node) return json404(`node '${nodeId}' not found`);
    const edges = flattenEdges(graph);
    const connections = edges.filter(e => e.from === nodeId || e.to === nodeId).map(e => {
      const otherId = e.from === nodeId ? e.to : e.from;
      const other = nodes.find(n => n.id === otherId);
      return {
        relationship: e.label || "connected",
        node: other || { id: otherId, label: otherId },
      };
    });
    return jsonOk({ node, connections });
  }

  return null; // not an API route
}

function jsonOk(data: any): Response {
  return new Response(JSON.stringify(data), {
    headers: { "content-type": "application/json", "cache-control": "no-cache" },
  });
}

function json400(msg: string): Response {
  return new Response(JSON.stringify({ error: msg }), {
    status: 400,
    headers: { "content-type": "application/json" },
  });
}

function json404(msg: string): Response {
  return new Response(JSON.stringify({ error: msg }), {
    status: 404,
    headers: { "content-type": "application/json" },
  });
}

// ── Server ──────────────────────────────────────────────────────────────────

function main(): void {
  const port = Number(process.env.AXON_PORT ?? 4244);
  if (!Number.isInteger(port) || port <= 0) {
    console.error(`knowledge-graph: AXON_PORT must be a port number, got ${JSON.stringify(process.env.AXON_PORT)}`);
    process.exit(1);
  }

  const distDir = distDirFor(axonRoot());
  if (!existsSync(join(distDir, "index.html"))) {
    console.error(`knowledge-graph: ${distDir} has no index.html — has the UI been built?`);
    console.error("  cd capabilities/knowledge-graph/ui && bun install && bun run build");
    process.exit(1);
  }

  // Idle detection (same as panel-server.ts)
  let lastSignal = Date.now();

  Bun.serve({
    hostname: "127.0.0.1",
    port,
    async fetch(request) {
      const url = new URL(request.url);

      // Liveness pair (from panel-server.ts)
      if (url.pathname === "/__axon/ping") {
        lastSignal = Date.now();
        return new Response(null, { status: 204 });
      }
      if (url.pathname === "/__axon/idle") {
        return Response.json({
          idle_seconds: Math.floor((Date.now() - lastSignal) / 1000),
          last_signal: new Date(lastSignal).toISOString(),
        });
      }

      // API routes
      if (url.pathname.startsWith("/api/")) {
        const apiResponse = handleAPI(url);
        if (apiResponse) return apiResponse;
        return json404("unknown API endpoint");
      }

      // Static file serving (from panel-server.ts)
      const file = resolveFile(url.pathname, distDir) ?? join(distDir, "index.html");

      if (file.endsWith(".html")) {
        const html = await Bun.file(file).text();
        const withHeartbeat = html.includes("</head>")
          ? html.replace("</head>", `${HEARTBEAT}</head>`)
          : html + HEARTBEAT;
        const injected = withHeartbeat.includes("</body>")
          ? withHeartbeat.replace("</body>", `${BACKLINK}</body>`)
          : withHeartbeat + BACKLINK;
        return new Response(injected, {
          headers: { "content-type": "text/html; charset=utf-8", "cache-control": "no-cache" },
        });
      }

      const encoded = precompressed(file, request.headers.get("accept-encoding") ?? "");
      return new Response(Bun.file(encoded?.path ?? file), {
        headers: {
          "content-type": contentType(file),
          ...(encoded ? { "content-encoding": encoded.encoding } : {}),
          "cache-control": file.includes(`${sep}immutable${sep}`)
            ? "public, max-age=31536000, immutable"
            : "no-cache",
        },
      });
    },
  });

  console.log(`knowledge-graph: serving UI + API on 127.0.0.1:${port}`);
}

if (import.meta.main) main();
