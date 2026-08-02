// tools/self.ts — Axon's self-model: what this repo contains, what is wired to what,
// where each upstream stands, and how much code each unit holds. One committed artifact
// (self.json) plus a query surface over it.
//
// Two consumers, by design. An agent starting work here reads self.json whole — it is
// deliberately kept small enough for that — instead of rediscovering the same structure
// by hand every session. A human runs `tools/self status` or `tools/self explain comms`.
// The dashboard panel (Axon#66) is a third view over the same file, not a second source.
//
// What is committed vs fused on read is the load-bearing distinction:
//
//   committed   structure, provenance, code rollup, coupling map — all derived from
//               tracked files, so two runs on an unchanged tree are byte-identical and
//               the artifact survives a fresh clone.
//   fused       live process health (axon-status owns it) and open issue counts (the
//               tracker owns them). Copying either into a committed file would give one
//               fact two homes and make the file lie the moment a process stops or an
//               issue is triaged.
//
// TypeScript rather than bash under the tools-doctor-typescript-not-bash precedent: this
// parses upstreams.toml via Bun.TOML and does set arithmetic over a 3,965-node graph,
// neither of which tools/lib/toml.sh's single-line grep/sed contract can express. The
// pure logic lives in tools/lib/self-model.ts and is tested by tools/self.test.ts; this
// file owns all I/O.
//
//   tools/self generate          # regenerate self.json from the working tree
//   tools/self status            # one row per unit (add --online for open issue counts)
//   tools/self explain <unit>    # wiring, code size, provenance for one unit
//   tools/self coupling          # what is compiled into what, with evidence
//   tools/self check             # is the committed self.json still current?
//   tools/self -h                # this help
//
// Exit 0 = fine, 1 = stale (check) or unknown unit (explain).

import { existsSync, readFileSync, readdirSync } from "node:fs";
import { resolve } from "node:path";
import {
  couplingFromBazel,
  couplingFromRustPath,
  mergeCoupling,
  rollUp,
  type SourceCoupling,
} from "./lib/self-model.ts";

const HELP = `tools/self — Axon's self-model: structure, coupling, provenance, code size.

  tools/self generate          regenerate self.json from the working tree
  tools/self status            one row per unit (--online adds open issue counts)
  tools/self explain <unit>    wiring, code size, provenance for one unit
  tools/self coupling          what is compiled into what, with evidence
  tools/self check             is the committed self.json still current? (exit 1 if not)

  --json                       machine-readable output for status/explain/coupling
`;

const AXON_ROOT = resolve(import.meta.dir, "..");
const SELF_JSON = `${AXON_ROOT}/self.json`;

/** The artifact's shape. Bump `schema` when a consumer would need to care. */
interface SelfModel {
  schema: 1;
  /** Deliberately NOT a timestamp: a generated-at field would make every run differ. */
  generator: string;
  units: Array<{
    name: string;
    kind: string;
    /** Present for anything with a service.toml. */
    service?: { kind: string; port?: string; requires: string[]; image?: string };
    code?: { files: number; nodes: number };
  }>;
  /** Compile-time coupling: what is pulled into what. Distinct from service `requires`. */
  coupling: Array<{ from: string; to: string; kinds: string[]; evidence: string[] }>;
  upstreams: Array<{ name: string; verdict: string; pin: string }>;
  /** Honest accounting of what the code graph could not attribute. */
  graph: { present: boolean; nodes: number; external: number; stale: string[]; unmatched: string[] };
}

function readText(path: string): string | null {
  try {
    return readFileSync(path, "utf8");
  } catch {
    return null;
  }
}

/** The registry is the one TOML reader for service manifests (README.md#one-manifest-per-concern). */
function readRegistry(): Array<Record<string, unknown>> {
  const proc = Bun.spawnSync({
    cmd: [`${AXON_ROOT}/tools/capability.sh`, "registry"],
    stdout: "pipe",
    stderr: "pipe",
  });
  if (proc.exitCode !== 0) return [];
  try {
    return JSON.parse(proc.stdout.toString());
  } catch {
    return [];
  }
}

function readUpstreams(): SelfModel["upstreams"] {
  const text = readText(`${AXON_ROOT}/upstreams.toml`);
  if (!text) return [];
  const parsed = Bun.TOML.parse(text) as Record<string, { verdict?: string; pin?: string }>;
  return Object.entries(parsed)
    .map(([name, v]) => ({ name, verdict: v?.verdict ?? "", pin: v?.pin ?? "" }))
    .sort((a, b) => a.name.localeCompare(b.name));
}

/**
 * Walk tracked sources for ground-truth coupling.
 *
 * `git ls-files` rather than a filesystem glob: an untracked scratch file is not part of
 * what this repo IS, and including it would make the committed artifact depend on
 * whatever happens to be lying in the working tree.
 */
function readCoupling(): SourceCoupling[] {
  const proc = Bun.spawnSync({
    cmd: ["git", "-C", AXON_ROOT, "ls-files", "*.rs", "BUILD.bazel", "*/BUILD.bazel"],
    stdout: "pipe",
  });
  const files = proc.stdout.toString().split("\n").filter(Boolean);
  const edges: SourceCoupling[] = [];
  for (const file of files) {
    const text = readText(`${AXON_ROOT}/${file}`);
    if (text === null) continue;
    if (file.endsWith(".rs")) edges.push(...couplingFromRustPath(file, text));
    if (file.endsWith("BUILD.bazel")) edges.push(...couplingFromBazel(file, text));
  }
  return edges;
}

/** Public-safe graph input boundary: only Git-tracked paths may become internal metadata. */
function readTrackedPaths(): Set<string> {
  const proc = Bun.spawnSync({
    cmd: ["git", "-C", AXON_ROOT, "ls-files", "-z"],
    stdout: "pipe",
  });
  if (proc.exitCode !== 0) return new Set();
  return new Set(proc.stdout.toString().split("\0").filter(Boolean));
}

function build(): SelfModel {
  const registry = readRegistry();
  const graphText = readText(`${AXON_ROOT}/graphify-out/graph.json`);
  const trackedPaths = readTrackedPaths();
  const tracked = (p: string) => trackedPaths.has(p);
  const exists = (p: string) => existsSync(`${AXON_ROOT}/${p}`);

  let rollupUnits: Array<{ name: string; kind: string; files: number; nodes: number }> = [];
  let graph: SelfModel["graph"] = { present: false, nodes: 0, external: 0, stale: [], unmatched: [] };
  if (graphText) {
    const parsed = JSON.parse(graphText);
    const r = rollUp(parsed.nodes ?? [], tracked, exists);
    rollupUnits = r.units;
    graph = {
      present: true,
      nodes: r.admittedNodes,
      external: r.buckets.external,
      stale: r.buckets.stale,
      unmatched: r.buckets.unmatched,
    };
  }

  // The unit inventory comes from the tracked tree, never from the code graph.
  //
  // Deriving it from the graph made the whole artifact depend on git-ignored
  // graphify-out/: on a fresh clone the unit list silently collapsed from 31 to the ~12
  // capabilities that happen to own a service.toml, dropping every Pack and lib. What
  // Axon *contains* is a fact about tracked files, so it is read from them; the graph
  // only ever contributes `code` counts on top.
  const kindByUnit = new Map<string, string>();
  const addDirs = (parent: string, kind: string) => {
    try {
      for (const name of readdirSync(`${AXON_ROOT}/${parent}`, { withFileTypes: true })) {
        if (name.isDirectory()) kindByUnit.set(name.name, kind);
      }
    } catch {
      // A missing top-level directory is not an error: a minimal install has no Packs.
    }
  };
  addDirs("capabilities", "capability");
  addDirs("libs", "lib");
  addDirs("Packs", "pack");
  for (const spine of ["dashboard", "tools", "schemas"]) {
    if (existsSync(`${AXON_ROOT}/${spine}`)) kindByUnit.set(spine, "spine");
  }

  const codeByUnit = new Map(rollupUnits.map((u) => [u.name, u]));
  const names = new Set<string>(kindByUnit.keys());
  // Overlay capabilities are runtime-visible but never generation-visible (Axon#225):
  // self.json is tracked and public, and a capability name is itself a fact about a
  // private deployment. The registry labels its own rows, so this filter does not have
  // to guess from a path.
  const publicRegistry = registry.filter((r) => String(r.scope ?? "") !== "overlay-capability");
  for (const row of publicRegistry) names.add(String(row.name));

  const units: SelfModel["units"] = [...names].sort().map((name) => {
    const reg = publicRegistry.find((r) => String(r.name) === name);
    const code = codeByUnit.get(name);
    const kind = kindByUnit.get(name) ?? (reg ? "capability" : "unknown");
    const out: SelfModel["units"][number] = { name, kind };
    if (reg) {
      out.service = {
        kind: String(reg.kind ?? ""),
        requires: (reg.requires as string[]) ?? [],
      };
      if (reg.port) out.service.port = String(reg.port);
      if (reg.image) out.service.image = String(reg.image);
    }
    if (code) out.code = { files: code.files, nodes: code.nodes };
    return out;
  });

  return {
    schema: 1,
    generator: "tools/self.ts",
    units,
    coupling: mergeCoupling(readCoupling()),
    upstreams: readUpstreams(),
    graph,
  };
}

/** Stable stringify via sorted construction above — key order is insertion order. */
function serialize(model: SelfModel): string {
  return JSON.stringify(model, null, 2) + "\n";
}

function loadCommitted(): SelfModel | null {
  const text = readText(SELF_JSON);
  return text ? (JSON.parse(text) as SelfModel) : null;
}

/**
 * Drop the layers that only a machine with a built code graph can reproduce.
 *
 * `self.json` is committed, but its per-unit `code` counts and its `graph` block are
 * derived from `graphify-out/`, which is git-ignored because graphify slugifies node ids
 * from the absolute scan path. A fresh clone — CI, or another machine before it runs
 * `tools/graphify.sh` — therefore cannot regenerate those two layers, and a naive
 * full-text comparison reports the file stale when nothing is actually wrong.
 *
 * That is the same defect class this run gated against elsewhere: a committed artifact
 * depending on an input others cannot see. The honest resolution is not to hide the
 * mismatch but to narrow the claim — compare everything reproducible here, and say which
 * scope was checked. Structure, coupling and provenance all come from tracked files, so
 * they are gateable anywhere; the code layer is gated wherever a graph exists.
 */
function stripLocalLayers(model: SelfModel): Omit<SelfModel, "graph"> & { units: SelfUnitLike[] } {
  const { graph: _graph, ...rest } = model;
  return { ...rest, units: model.units.map(({ code: _code, ...u }) => u) };
}
type SelfUnitLike = Omit<SelfModel["units"][number], "code">;

/** Open issues per unit, joined on the `<unit>:` title prefix the tracker already uses. */
function openIssuesByUnit(): { counts: Map<string, number>; unmatched: number } | null {
  // No --repo: gh resolves it from this checkout's remote, the same way the
  // `git -C AXON_ROOT` calls above resolve theirs. It was hardcoded to one
  // owner/name, which is a deployment fact in public code and would have gone
  // on querying that name after a rename — answering from whatever repository
  // happened to hold it rather than from this one.
  const proc = Bun.spawnSync({
    cmd: ["gh", "issue", "list", "--state", "open", "--limit", "200", "--json", "title"],
    cwd: AXON_ROOT,
    stdout: "pipe",
    stderr: "pipe",
  });
  if (proc.exitCode !== 0) return null;
  let rows: Array<{ title: string }>;
  try {
    rows = JSON.parse(proc.stdout.toString());
  } catch {
    return null;
  }
  const counts = new Map<string, number>();
  let unmatched = 0;
  for (const { title } of rows) {
    const m = title.match(/^([A-Za-z0-9._-]+):/);
    if (m) counts.set(m[1], (counts.get(m[1]) ?? 0) + 1);
    else unmatched += 1;
  }
  return { counts, unmatched };
}

const args = process.argv.slice(2);
const wantJson = args.includes("--json");
const online = args.includes("--online");
const cmd = args.find((a) => !a.startsWith("-")) ?? "status";

if (args.includes("-h") || args.includes("--help")) {
  console.log(HELP);
  process.exit(0);
}

if (cmd === "generate") {
  const out = serialize(build());
  await Bun.write(SELF_JSON, out);
  console.log(`wrote ${SELF_JSON} (${out.length} bytes)`);
  process.exit(0);
}

if (cmd === "check") {
  const committedText = readText(SELF_JSON);
  if (!committedText) {
    console.error("self.json is missing. Run: tools/self generate");
    process.exit(1);
  }
  const fresh = build();
  // Full comparison only where a code graph exists to reproduce those layers; elsewhere
  // (a fresh clone, CI) narrow the claim to the tracked-file layers rather than reporting
  // a mismatch nobody can fix without running graphify. See stripLocalLayers.
  const scope = fresh.graph.present ? "full" : "structure, coupling and provenance";
  const left = fresh.graph.present ? serialize(fresh) : JSON.stringify(stripLocalLayers(fresh), null, 2);
  const right = fresh.graph.present
    ? committedText
    : JSON.stringify(stripLocalLayers(JSON.parse(committedText) as SelfModel), null, 2);

  if (left === right) {
    console.log(
      fresh.graph.present
        ? "self.json is current."
        : `self.json is current (${scope}; no code graph here, so per-unit code counts were not compared).`,
    );
    process.exit(0);
  }
  console.error(`self.json is stale (${scope} differ). Run: tools/self generate`);
  process.exit(1);
}

const model = loadCommitted() ?? build();

if (cmd === "coupling") {
  if (wantJson) {
    console.log(JSON.stringify(model.coupling, null, 2));
    process.exit(0);
  }
  console.log(`Compile-time coupling — what is pulled into what (${model.coupling.length} pairs).`);
  console.log("Distinct from service `requires`, which is what must be RUNNING.\n");
  for (const e of model.coupling) {
    console.log(`  ${e.from.padEnd(14)} -> ${e.to.padEnd(14)} [${e.kinds.join("+")}]`);
  }
  process.exit(0);
}

if (cmd === "explain") {
  const name = args.find((a) => !a.startsWith("-") && a !== "explain");
  const unit = model.units.find((u) => u.name === name);
  if (!unit) {
    console.error(`unknown unit '${name}'. Known: ${model.units.map((u) => u.name).join(", ")}`);
    process.exit(1);
  }
  if (wantJson) {
    console.log(JSON.stringify(unit, null, 2));
    process.exit(0);
  }
  console.log(`${unit.name} (${unit.kind})`);
  if (unit.code) console.log(`  code       ${unit.code.files} files, ${unit.code.nodes} graph nodes`);
  if (unit.service) {
    console.log(`  service    kind=${unit.service.kind}${unit.service.port ? ` port=${unit.service.port}` : ""}`);
    console.log(`  requires   ${unit.service.requires.length ? unit.service.requires.join(", ") : "—"} (must be running)`);
  }
  const out = model.coupling.filter((c) => c.from === unit.name).map((c) => c.to);
  const inc = model.coupling.filter((c) => c.to === unit.name).map((c) => c.from);
  console.log(`  compiles in ${out.length ? out.join(", ") : "—"}`);
  console.log(`  used by     ${inc.length ? inc.join(", ") : "—"}`);
  process.exit(0);
}

// Default: status
const work = online ? openIssuesByUnit() : null;
if (wantJson) {
  console.log(JSON.stringify({ ...model, work: work ? Object.fromEntries(work.counts) : null }, null, 2));
  process.exit(0);
}
console.log(`Axon self-model — ${model.units.length} units, ${model.coupling.length} coupling pairs`);
console.log(
  model.graph.present
    ? `Code graph: ${model.graph.nodes} nodes, ${model.graph.external} external, ${model.graph.stale.length} stale\n`
    : "Code graph: absent (run tools/graphify.sh)\n",
);
const header = `  ${"unit".padEnd(18)}${"kind".padEnd(12)}${"files".padStart(6)}${"port".padStart(7)}${"requires".padStart(12)}`;
console.log(header + (work ? "   open" : ""));
for (const u of model.units) {
  let row = `  ${u.name.padEnd(18)}${u.kind.padEnd(12)}`;
  row += String(u.code?.files ?? "—").padStart(6);
  row += String(u.service?.port ?? "—").padStart(7);
  row += String(u.service?.requires.length ? u.service.requires.join(",") : "—").padStart(12);
  if (work) row += String(work.counts.get(u.name) ?? 0).padStart(7);
  console.log(row);
}
if (work) console.log(`\n  ${work.unmatched} open issues match no unit prefix.`);
if (model.graph.stale.length) {
  console.log(`\n  ⚠ ${model.graph.stale.length} graph paths no longer exist — run tools/graphify.sh`);
}
