#!/usr/bin/env bun
// tools/demo-record.ts — capture what the seeded capabilities answer (#168).
//
// GETs every path declared in demo.toml against the live capabilities and writes the
// bodies under demo/fixtures, plus an index.json the demo bundle's fetch shim reads.
//
// WHY AN INDEX RATHER THAN A NAMING CONVENTION. The browser could compute a fixture
// filename from a request path if both sides implemented the same rule — and then the demo
// would break, silently and only in production, the first time the two implementations
// disagreed about how to encode a query string. The index makes it one map written once and
// read once: a path is either in it or it is not, and "not" is a hard error the shim shows
// rather than an empty page.
//
// Nothing here is committed. Fixtures are a build product of the seeder plus the servers, and
// a tracked copy would let a stale recording get published behind an API that had moved.
//
//   tools/demo-record                 record into demo/fixtures
//   tools/demo-record --out <dir>     record elsewhere
//   tools/demo-record --check         request everything, write nothing, report sizes

import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";

import {
  absentPrefixes,
  AXON_ROOT,
  fixtureFile,
  loadManifest,
  resolvePath,
  routes,
  type DemoCapability,
} from "./lib/demo-endpoints.ts";

interface Recorded {
  browserPath: string;
  file: string;
  bytes: number;
  /** Rows, where the body is a list. Logged so a CI run shows the demo is not empty —
   *  a fixture set that records six 200s of `[]` is a green build and a blank demo. */
  rows: number | null;
}

function countRows(body: unknown): number | null {
  if (Array.isArray(body)) return body.length;
  if (body && typeof body === "object") {
    // The one-key envelope every list endpoint here uses: {tasks: []}, {backups: []}.
    const values = Object.values(body as Record<string, unknown>);
    const arrays = values.filter(Array.isArray);
    if (arrays.length === 1 && values.length <= 3) return (arrays[0] as unknown[]).length;
  }
  return null;
}

async function fetchJson(url: string): Promise<unknown> {
  const res = await fetch(url);
  const text = await res.text();
  if (!res.ok) throw new Error(`GET ${url} → ${res.status}: ${text.slice(0, 300)}`);
  if (!text) return null;
  try {
    return JSON.parse(text);
  } catch {
    throw new Error(`GET ${url} → 200 but the body is not JSON: ${text.slice(0, 200)}`);
  }
}

/** Paths from an `expand` rule: one per id in an already-recorded list body. Keeps generated
 *  ids out of demo.toml, which would otherwise have to be rewritten every time the seed
 *  changes — and a manifest that has to track generated state is not a manifest. */
function expanded(cap: DemoCapability, bodies: Map<string, unknown>): string[] {
  const out: string[] = [];
  for (const rule of cap.expand) {
    const body = bodies.get(rule.from);
    if (body === undefined) {
      throw new Error(
        `demo.toml: [capability.${cap.name}] expands '${rule.from}', which is not one of its paths`,
      );
    }
    const list = Array.isArray(body) ? body : [];
    for (const row of list) {
      const id = (row as Record<string, unknown>)?.[rule.id_field];
      if (typeof id !== "string" || id === "") continue;
      out.push(rule.into.replace("{id}", encodeURIComponent(id)));
    }
  }
  return out;
}

async function main(): Promise<void> {
  const args = process.argv.slice(2);
  if (args.includes("-h") || args.includes("--help")) {
    console.log("tools/demo-record [--out <dir>] [--check]");
    return;
  }
  const check = args.includes("--check");
  const outIdx = args.indexOf("--out");
  const manifest = loadManifest();
  const outDir = outIdx >= 0 ? args[outIdx + 1] : join(AXON_ROOT, manifest.fixturesDir);

  const table = routes();
  const recorded: Recorded[] = [];
  const index: Record<string, string> = {};

  if (!check) {
    // Cleared, not merged. A fixture left behind from a previous manifest is a route the
    // shim will happily serve and nothing will ever regenerate.
    rmSync(outDir, { recursive: true, force: true });
    mkdirSync(outDir, { recursive: true });
  }

  for (const cap of manifest.capabilities) {
    const bodies = new Map<string, unknown>();
    for (const browserPath of cap.paths) {
      bodies.set(browserPath, await fetchJson(resolvePath(browserPath, table).url));
    }
    for (const browserPath of expanded(cap, bodies)) {
      bodies.set(browserPath, await fetchJson(resolvePath(browserPath, table).url));
    }

    for (const [browserPath, body] of bodies) {
      const file = fixtureFile(browserPath);
      const json = `${JSON.stringify(body, null, 2)}\n`;
      if (!check) {
        const dest = join(outDir, file);
        mkdirSync(dirname(dest), { recursive: true });
        writeFileSync(dest, json);
      }
      index[browserPath] = file;
      recorded.push({ browserPath, file, bytes: json.length, rows: countRows(body) });
    }

    const forCap = recorded.filter((r) => r.browserPath.startsWith(`/${cap.name}`) || cap.paths.includes(r.browserPath));
    const empty = forCap.filter((r) => r.rows === 0);
    console.log(
      `recorded ${cap.name}: ${forCap.length} paths` +
        (empty.length > 0 ? ` — ${empty.length} returned an empty list: ${empty.map((e) => e.browserPath).join(", ")}` : ""),
    );
  }

  const meta = {
    // Read by the shim to render the banner, and by tools/demo-site to build the nav.
    generator: "tools/demo-record.ts",
    seed: manifest.seed,
    anchor: manifest.anchor,
    label: manifest.label,
    absent: manifest.absent,
    // Which capability owns which URL prefix — the enabled set, plus the absent ones read
    // straight from their manifests. The shim needs the absent ones most: a request to
    // /comms/feed has to be answerable with "Comms is not in this demo, here is why" instead
    // of the generic "no fixture", and it cannot tell those apart without knowing that
    // /comms is a capability at all. The registry alone would not say so, because under the
    // demo overlay Comms is not enabled and therefore not in it.
    prefixes: [
      ...table.map((r) => ({ capability: r.capability, prefix: r.prefix })),
      ...absentPrefixes(Object.keys(manifest.absent)),
    ],
    routes: index,
  };
  if (check) {
    const total = recorded.reduce((n, r) => n + r.bytes, 0);
    console.log(`demo-record: ${recorded.length} paths, ${total} bytes (nothing written)`);
    return;
  }
  writeFileSync(join(outDir, "index.json"), `${JSON.stringify(meta, null, 2)}\n`);
  console.log(`wrote ${recorded.length} fixtures + index.json to ${outDir}`);
}

if (import.meta.main) {
  main().catch((err) => {
    console.error(`demo-record: ${err instanceof Error ? err.message : String(err)}`);
    process.exit(1);
  });
}
