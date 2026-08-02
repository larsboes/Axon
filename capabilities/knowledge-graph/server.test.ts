// capabilities/knowledge-graph/server.test.ts — bounded smoke test for the graph REST API.
//
// Bounded in three ways, each deliberate. It plants a four-node graph instead of reading
// graphify-out/graph.json, which is generated, gitignored, and several thousand nodes — a
// test against it would assert this machine's code, not this API's contract. It calls
// handleAPI directly instead of binding a port, so nothing here depends on the capability
// running or on the UI having been built. And it asserts the shape and the status each
// documented route promises, not the contents of any particular graph.
//
// One assertion per documented route plus the two failure paths a caller actually hits: no
// graph on disk, and an unknown node. That is the whole surface README.md and Axon#116
// claim, which is what makes this a closure condition rather than coverage for its own
// sake.
// Run: bun test capabilities/knowledge-graph/server.test.ts

import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { handleAPI } from "./server.ts";

// graphify's own output shape: nodes carry id/label/file_type/source_file/community, edges
// arrive as links with source/target. Both aliases the flatteners accept are exercised —
// `links` here, `from`/`to` in the edge-alias case below.
const GRAPH = {
  built_at_commit: "fixturesha",
  nodes: [
    { id: "a.ts", label: "a.ts", file_type: "code", source_file: "src/a.ts", community: 0 },
    { id: "b.ts", label: "b.ts", file_type: "code", source_file: "src/b.ts", community: 0 },
    { id: "c.md", label: "c.md", file_type: "doc", source_file: "docs/c.md", community: 1 },
    { id: "d.ts", label: "d.ts", file_type: "code", source_file: "src/d.ts", community: 1 },
  ],
  links: [
    { source: "a.ts", target: "b.ts", label: "imports" },
    { source: "b.ts", target: "c.md", label: "documents" },
    { source: "c.md", target: "d.ts", label: "references" },
  ],
};

let root = "";
const previousRoot = process.env.AXON_ROOT;

/** Point the server at a scratch AXON_ROOT holding `graph`, or none at all. */
function plant(graph: unknown | null): void {
  root = mkdtempSync(join(tmpdir(), "axon-kg-"));
  if (graph !== null) {
    mkdirSync(join(root, "graphify-out"), { recursive: true });
    writeFileSync(join(root, "graphify-out/graph.json"), JSON.stringify(graph));
  }
  process.env.AXON_ROOT = root;
}

const call = async (path: string): Promise<{ status: number; body: any }> => {
  const response = handleAPI(new URL(`http://127.0.0.1:4244${path}`));
  if (!response) return { status: 0, body: null }; // not an API route this server owns
  return { status: response.status, body: await response.json() };
};

beforeEach(() => plant(GRAPH));
afterEach(() => {
  rmSync(root, { recursive: true, force: true });
  if (previousRoot === undefined) delete process.env.AXON_ROOT;
  else process.env.AXON_ROOT = previousRoot;
});

describe("GET /api/graph", () => {
  test("returns the graph as graphify wrote it", async () => {
    const { status, body } = await call("/api/graph");
    expect(status).toBe(200);
    expect(body.nodes).toHaveLength(4);
    expect(body.links).toHaveLength(3);
  });
});

describe("GET /api/graph/stats", () => {
  test("counts nodes, edges and communities", async () => {
    const { status, body } = await call("/api/graph/stats");
    expect(status).toBe(200);
    expect(body).toMatchObject({ nodes: 4, edges: 3, communities: 2, built_at: "fixturesha" });
  });

  test("splits the corpus by file type rather than reporting one total twice", async () => {
    const { body } = await call("/api/graph/stats");
    expect(body.corpus_files).toBe(3);
    expect(body.doc_files).toBe(1);
  });
});

describe("GET /api/graph/search", () => {
  test("matches on label, and returns only edges between two matches", async () => {
    const { status, body } = await call("/api/graph/search?q=.ts");
    expect(status).toBe(200);
    expect(body.results).toBe(3);
    // a→b survives (both .ts); b→c and c→d each have one endpoint outside the match.
    expect(body.edges).toHaveLength(1);
  });

  test("matches on source file and on file type, not just the label", async () => {
    expect((await call("/api/graph/search?q=docs/")).body.results).toBe(1);
    expect((await call("/api/graph/search?q=doc")).body.results).toBe(1);
  });

  test("a missing q is a 400, not an empty result set that reads like 'nothing matched'", async () => {
    const { status, body } = await call("/api/graph/search");
    expect(status).toBe(400);
    expect(body.error).toContain("q");
  });
});

describe("GET /api/graph/community/:id", () => {
  test("returns that community's members and its internal edges only", async () => {
    const { status, body } = await call("/api/graph/community/0");
    expect(status).toBe(200);
    expect(body).toMatchObject({ community: 0, members: 2 });
    expect(body.edges).toHaveLength(1); // a→b; b→c leaves the community
  });

  test("a community nobody is in is an empty membership, not an error", async () => {
    const { status, body } = await call("/api/graph/community/99");
    expect(status).toBe(200);
    expect(body.members).toBe(0);
  });
});

describe("GET /api/graph/node/:id", () => {
  test("returns the node with its connections in both directions", async () => {
    const { status, body } = await call("/api/graph/node/b.ts");
    expect(status).toBe(200);
    expect(body.node).toMatchObject({ id: "b.ts", file_type: "code", group: "community-0" });
    expect(body.connections.map((c: any) => c.node.id).sort()).toEqual(["a.ts", "c.md"]);
    expect(body.connections.map((c: any) => c.relationship).sort()).toEqual(["documents", "imports"]);
  });

  test("an unknown node is a 404 naming what was asked for", async () => {
    const { status, body } = await call("/api/graph/node/nope.ts");
    expect(status).toBe(404);
    expect(body.error).toContain("nope.ts");
  });
});

describe("failure paths", () => {
  test("no graph on disk is a 404 that says how to produce one", async () => {
    rmSync(root, { recursive: true, force: true });
    plant(null);
    const { status, body } = await call("/api/graph/stats");
    expect(status).toBe(404);
    expect(body.error).toContain("graphify");
  });

  test("an unrouted /api path is left for the server to 404, not answered here", async () => {
    expect(handleAPI(new URL("http://127.0.0.1:4244/api/graph/invented"))).toBeNull();
  });

  test("edges written as from/to are read the same as source/target", async () => {
    // graphify has emitted both shapes; the flattener accepts either, and a smoke test
    // that only ever saw one would not notice the day the other stopped working.
    rmSync(root, { recursive: true, force: true });
    plant({ nodes: GRAPH.nodes, edges: [{ from: "a.ts", to: "d.ts", label: "calls" }] });
    const { body } = await call("/api/graph/node/a.ts");
    expect(body.connections).toEqual([
      { relationship: "calls", node: expect.objectContaining({ id: "d.ts" }) },
    ]);
  });
});
