#!/usr/bin/env bun
// tools/generate-site.ts — render self.json as a self-contained page (#14).
//
// The repository has no landing surface: the only entry points are a 32 KB README and a generated
// ARCHITECTURE.md. This renders what Axon already knows about itself into one HTML file someone
// can read without cloning.
//
// Generated, never hand-maintained, for the same reason ARCHITECTURE.md is: a fact that is not in
// a manifest cannot appear here, so the page cannot drift from the tree.
//
// WHAT IS DELIBERATELY NOT RENDERED, and this is the interesting part:
//
//   graph.*        node counts and the stale list come from graphify-out/, which is untracked.
//                  A fresh clone cannot reproduce them, so publishing them would put a number on
//                  the page that nobody can check — the exact property #7 fixed for ports.
//   units[].code   same reason: per-unit file and node counts come from that graph.
//
// Everything that remains traces to a tracked manifest: service.toml for kind/port/requires,
// upstreams.toml for verdict/pin, and tools/self.ts's own coupling scan over BUILD.bazel and
// source imports.
//
// Self-contained by requirement (#14) and by house rule: inline CSS, no script, no font, no image,
// no request to any other host. The one interactive element (filtering by kind) is CSS-only.
//
//   tools/generate-site                 write site/index.html
//   tools/generate-site --out <dir>     write <dir>/index.html
//   tools/generate-site --check         render to memory, report size, write nothing

import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";

const AXON_ROOT = resolve(dirname(new URL(import.meta.url).pathname), "..");

interface Unit {
  name: string;
  kind: "capability" | "lib" | "pack" | "spine";
  service?: { kind: string; requires: string[]; port?: string; image?: string };
}
interface Coupling { from: string; to: string; kinds: string[]; evidence: string[] }
interface Upstream { name: string; verdict: string; pin: string }
interface SelfModel { schema: number; generator: string; units: Unit[]; coupling: Coupling[]; upstreams: Upstream[] }

// Minimal, total escaping. Every value below comes from a tracked manifest, but a manifest is
// still text somebody edits, and an unescaped `&` in a `why` field would be a silent corruption
// rather than a visible one.
const esc = (s: unknown): string =>
  String(s ?? "")
    .replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;").replace(/'/g, "&#39;");

const KIND_ORDER: Unit["kind"][] = ["capability", "spine", "lib", "pack"];
const KIND_LABEL: Record<string, string> = {
  capability: "Capabilities",
  spine: "Spine",
  lib: "Libraries",
  pack: "Packs",
};
const KIND_BLURB: Record<string, string> = {
  capability: "A service Axon runs, declared by one service.toml the shared runner interprets.",
  spine: "The parts every capability resolves against rather than reimplements.",
  lib: "Shared code with no service of its own.",
  pack: "Harness-neutral skills, deployed to an agent harness rather than run.",
};
// Preference order from upstreams.toml's own header comment, so the page groups them the way the
// manifest ranks them rather than alphabetically.
const VERDICT_ORDER = ["adopt", "contribute", "overlay", "fork", "build", "inspiration", "quarry", "reject"];

function renderUnits(units: Unit[]): string {
  return KIND_ORDER.map((kind) => {
    const rows = units.filter((u) => u.kind === kind).sort((a, b) => a.name.localeCompare(b.name));
    if (rows.length === 0) return "";
    const body = rows.map((u) => {
      const svc = u.service;
      const runs = svc ? esc(svc.kind) : "—";
      const port = svc?.port ? `<code>${esc(svc.port)}</code>` : "—";
      const needs = svc?.requires?.length
        ? svc.requires.map((r) => `<code>${esc(r)}</code>`).join(" ")
        : "—";
      return `<tr><th scope="row">${esc(u.name)}</th><td>${runs}</td><td>${port}</td><td>${needs}</td></tr>`;
    }).join("\n");
    return `
<section id="${kind}">
  <h2>${KIND_LABEL[kind]} <span class="count">${rows.length}</span></h2>
  <p class="blurb">${KIND_BLURB[kind]}</p>
  <div class="scroll">
    <table>
      <thead><tr><th scope="col">Name</th><th scope="col">Runs as</th><th scope="col">Port</th><th scope="col">Requires</th></tr></thead>
      <tbody>
${body}
      </tbody>
    </table>
  </div>
</section>`;
  }).join("\n");
}

function renderCoupling(coupling: Coupling[]): string {
  if (coupling.length === 0) return "";
  const rows = [...coupling]
    .sort((a, b) => a.from.localeCompare(b.from) || a.to.localeCompare(b.to))
    .map((c) => `<tr><th scope="row">${esc(c.from)}</th><td>→ ${esc(c.to)}</td><td>${c.kinds.map((k) => `<code>${esc(k)}</code>`).join(" ")}</td></tr>`)
    .join("\n");
  return `
<section id="coupling">
  <h2>Coupling <span class="count">${coupling.length}</span></h2>
  <p class="blurb">Which unit reads which, found in BUILD.bazel labels and source imports rather than declared by hand.</p>
  <div class="scroll">
    <table>
      <thead><tr><th scope="col">From</th><th scope="col">To</th><th scope="col">Seen in</th></tr></thead>
      <tbody>
${rows}
      </tbody>
    </table>
  </div>
</section>`;
}

function renderUpstreams(upstreams: Upstream[]): string {
  const groups = VERDICT_ORDER
    .map((v) => [v, upstreams.filter((u) => u.verdict === v).sort((a, b) => a.name.localeCompare(b.name))] as const)
    .filter(([, list]) => list.length > 0);
  const body = groups.map(([verdict, list]) => `
    <h3>${esc(verdict)} <span class="count">${list.length}</span></h3>
    <ul class="pins">
${list.map((u) => `      <li><span>${esc(u.name)}</span>${u.pin ? `<code>${esc(u.pin)}</code>` : `<em>unpinned</em>`}</li>`).join("\n")}
    </ul>`).join("\n");
  return `
<section id="upstreams">
  <h2>Upstreams <span class="count">${upstreams.length}</span></h2>
  <p class="blurb">Every external project Axon touches, with the verdict it was given and the exact ref consumed. No entry, no entry.</p>
${body}
</section>`;
}

export function renderSite(model: SelfModel): string {
  const counts = KIND_ORDER
    .map((k) => [k, model.units.filter((u) => u.kind === k).length] as const)
    .filter(([, n]) => n > 0);
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Axon — self-model</title>
<meta name="description" content="What Axon is: its capabilities, their contracts, and how they connect — generated from the repository's own manifests.">
<style>
  :root {
    color-scheme: light dark;
    --bg: #fbfbfa; --fg: #1a1a1a; --dim: #5b5b58; --line: #e2e1dd;
    --card: #ffffff; --accent: #2b5f8f; --code-bg: #f2f1ee;
  }
  @media (prefers-color-scheme: dark) {
    :root {
      --bg: #121312; --fg: #e8e7e3; --dim: #9a9a95; --line: #2a2b29;
      --card: #191a19; --accent: #7fb3dd; --code-bg: #202120;
    }
  }
  * { box-sizing: border-box; }
  body {
    margin: 0; padding: 0 1.25rem 5rem; background: var(--bg); color: var(--fg);
    font: 15px/1.6 ui-sans-serif, -apple-system, "Segoe UI", Roboto, sans-serif;
  }
  main { max-width: 62rem; margin: 0 auto; }
  header { padding: 4rem 0 2rem; border-bottom: 1px solid var(--line); }
  h1 { margin: 0 0 .4rem; font-size: 2.1rem; letter-spacing: -.02em; }
  .tagline { margin: 0; max-width: 44rem; color: var(--dim); font-size: 1.05rem; }
  .totals { display: flex; flex-wrap: wrap; gap: .5rem; margin: 1.6rem 0 0; padding: 0; list-style: none; }
  .totals li {
    padding: .35rem .7rem; border: 1px solid var(--line); border-radius: 999px;
    background: var(--card); font-size: .82rem; color: var(--dim);
  }
  .totals strong { color: var(--fg); font-variant-numeric: tabular-nums; }
  section { padding: 2.5rem 0 0; }
  h2 { margin: 0 0 .3rem; font-size: 1.3rem; letter-spacing: -.01em; }
  h3 { margin: 1.6rem 0 .5rem; font-size: .95rem; text-transform: uppercase; letter-spacing: .06em; color: var(--dim); }
  .count {
    margin-left: .4rem; padding: .1rem .45rem; border-radius: 4px; background: var(--code-bg);
    font-size: .75rem; font-weight: 400; color: var(--dim); vertical-align: middle;
    font-variant-numeric: tabular-nums;
  }
  .blurb { margin: 0 0 1rem; max-width: 46rem; color: var(--dim); font-size: .9rem; }
  /* Wide content scrolls inside its own box; the page body never scrolls sideways. */
  .scroll { overflow-x: auto; border: 1px solid var(--line); border-radius: 8px; background: var(--card); }
  table { width: 100%; border-collapse: collapse; font-size: .88rem; }
  th, td { padding: .55rem .8rem; text-align: left; border-bottom: 1px solid var(--line); vertical-align: top; }
  tbody tr:last-child th, tbody tr:last-child td { border-bottom: 0; }
  thead th { font-size: .72rem; text-transform: uppercase; letter-spacing: .06em; color: var(--dim); font-weight: 500; }
  tbody th { font-weight: 600; white-space: nowrap; }
  code {
    padding: .1rem .35rem; border-radius: 4px; background: var(--code-bg);
    font: .85em ui-monospace, SFMono-Regular, Menlo, monospace; overflow-wrap: anywhere;
  }
  .pins { margin: 0; padding: 0; list-style: none; display: grid; gap: .3rem;
          grid-template-columns: repeat(auto-fill, minmax(19rem, 1fr)); }
  .pins li {
    display: flex; align-items: baseline; justify-content: space-between; gap: .6rem;
    padding: .4rem .7rem; border: 1px solid var(--line); border-radius: 6px; background: var(--card);
    font-size: .85rem;
  }
  .pins em { color: var(--dim); font-size: .8rem; }
  footer { margin-top: 4rem; padding-top: 1.5rem; border-top: 1px solid var(--line); color: var(--dim); font-size: .82rem; }
  footer code { background: none; padding: 0; }
  a { color: var(--accent); }
</style>
</head>
<body>
<main>
<header>
  <h1>Axon</h1>
  <p class="tagline">One tree that knows what it runs. Every capability declares what it needs in a manifest; one shared runner decides how to satisfy it on this machine.</p>
  <ul class="totals">
${counts.map(([k, n]) => `    <li><strong>${n}</strong> ${esc(KIND_LABEL[k].toLowerCase())}</li>`).join("\n")}
    <li><strong>${model.upstreams.length}</strong> upstreams</li>
    <li><strong>${model.coupling.length}</strong> couplings</li>
  </ul>
</header>
${renderUnits(model.units)}
${renderCoupling(model.coupling)}
${renderUpstreams(model.upstreams)}
<footer>
  <p>Generated from <code>self.json</code> (schema ${esc(model.schema)}, written by <code>${esc(model.generator)}</code>) and rendered by <code>tools/generate-site.ts</code>. Nothing here is hand-maintained.</p>
  <p>Code-graph counts and the per-unit file counts in <code>self.json</code> are deliberately absent: they come from an untracked artifact, so a fresh clone cannot reproduce them, and a number nobody can check does not belong on a page like this.</p>
</footer>
</main>
</body>
</html>
`;
}

function main(): void {
  const args = process.argv.slice(2);
  if (args.includes("-h") || args.includes("--help")) {
    console.log("tools/generate-site [--out <dir>] [--check]");
    process.exit(0);
  }
  const outIdx = args.indexOf("--out");
  const outDir = outIdx >= 0 ? args[outIdx + 1] : join(AXON_ROOT, "site");
  const selfPath = join(AXON_ROOT, "self.json");
  if (!existsSync(selfPath)) {
    console.error(`generate-site: no self.json at ${selfPath} — run: tools/self generate`);
    process.exit(1);
  }
  const model = JSON.parse(readFileSync(selfPath, "utf8")) as SelfModel;
  // A schema bump means the shape this renderer reads may have moved under it. Fail rather than
  // publish a page built from assumptions about fields that no longer mean what they did.
  if (model.schema !== 1) {
    console.error(`generate-site: self.json is schema ${model.schema}, this renderer knows schema 1`);
    process.exit(1);
  }
  const html = renderSite(model);
  if (args.includes("--check")) {
    console.log(`generate-site: renders ${html.length} bytes from schema ${model.schema} (nothing written)`);
    return;
  }
  mkdirSync(outDir, { recursive: true });
  const dest = join(outDir, "index.html");
  writeFileSync(dest, html);
  console.log(`wrote ${dest} (${html.length} bytes)`);
}

if (import.meta.main) main();
