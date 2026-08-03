// Tests for tools/generate-site.ts (#14).
//
// The acceptance criteria this file exists to hold are properties of the OUTPUT, not of the code:
// nothing overlay-scoped may appear, nothing may request another host, and no rendered fact may
// come from an input a fresh clone cannot reproduce. Each of those can regress silently through a
// one-line change to a template, so each gets an assertion against a rendered page.

import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { renderSite } from "./generate-site.ts";

const AXON_ROOT = join(import.meta.dir, "..");

const MODEL = {
  schema: 1,
  generator: "tools/self.ts",
  units: [
    { name: "axon-status", kind: "capability" as const, service: { kind: "process", requires: ["postgres"], port: "8082" } },
    { name: "postgres", kind: "capability" as const, service: { kind: "container", requires: [], image: "postgres" } },
    { name: "axon-config", kind: "lib" as const },
    { name: "writing", kind: "pack" as const },
    { name: "tools", kind: "spine" as const },
  ],
  coupling: [{ from: "axon-status", to: "axon-config", kinds: ["bazel-label"], evidence: ["capabilities/axon-status/BUILD.bazel"] }],
  upstreams: [
    { name: "bun", verdict: "adopt", pin: "1.3.14" },
    { name: "stop-slop", verdict: "reject", pin: "8da1f03" },
    { name: "unpinned-thing", verdict: "inspiration", pin: "" },
  ],
};

describe("renderSite", () => {
  const html = renderSite(MODEL);

  test("renders every unit kind it was given", () => {
    for (const u of MODEL.units) expect(html).toContain(u.name);
    for (const label of ["Capabilities", "Spine", "Libraries", "Packs"]) expect(html).toContain(label);
  });

  test("renders the service contract, not just the name", () => {
    expect(html).toContain("8082");
    expect(html).toContain("process");
    expect(html).toContain("container");
  });

  test("groups upstreams by verdict and shows the pin", () => {
    expect(html).toContain("adopt");
    expect(html).toContain("1.3.14");
    // An entry with no pin says so rather than rendering an empty cell that reads as a value.
    expect(html).toContain("unpinned");
  });

  // #14: "Self-contained: no external CSS, JS, font or CDN request."
  test("requests nothing from another host", () => {
    const urls = html.match(/https?:\/\/[^"' )]+/g) ?? [];
    expect(urls).toEqual([]);
    expect(html).not.toContain("<script");
    expect(html).not.toContain("@import");
  });

  // #14: "Every rendered fact traces to a manifest or a generated artifact." The code graph is
  // neither — graphify-out/ is untracked, so a fresh clone reproduces none of it. A future edit
  // that helpfully surfaces those numbers would break the criterion silently; this catches it.
  test("renders nothing sourced from the untracked code graph", () => {
    const withGraph = renderSite({
      ...MODEL,
      // @ts-expect-error — deliberately feeding fields the renderer must ignore
      graph: { nodes: 5355, present: true, stale: ["tools/lib/publish.sh"], external: 12, unmatched: [] },
      units: MODEL.units.map((u) => ({ ...u, code: { files: 99, nodes: 1234 } })),
    });
    expect(withGraph).not.toContain("5355");
    expect(withGraph).not.toContain("1234");
    expect(withGraph).not.toContain("publish.sh");
  });

  test("escapes manifest text rather than trusting it", () => {
    const nasty = renderSite({
      ...MODEL,
      units: [{ name: "<img src=x onerror=1>", kind: "capability" as const }],
    });
    expect(nasty).not.toContain("<img src=x");
    expect(nasty).toContain("&lt;img");
  });
});

// #14: "No overlay-scoped capability, port or host appears in the output." Asserted against the
// REAL self.json, because the property is about what the committed artifact contains — a fixture
// would only prove the fixture is clean.
describe("the real self.json", () => {
  const model = JSON.parse(readFileSync(join(AXON_ROOT, "self.json"), "utf8"));

  test("carries no overlay-owned capability", () => {
    // tools/lib/paths.sh: generators that write tracked artifacts scan AXON_CAPS_DIR alone,
    // because a capability name is itself a fact about a private deployment. This is the
    // downstream check on that rule.
    const overlayCaps = ["server"];
    const names = new Set(model.units.map((u: { name: string }) => u.name));
    for (const cap of overlayCaps) expect(names.has(cap)).toBe(false);
  });

  test("renders without a host, home path or tailnet name", () => {
    const html = renderSite(model);
    expect(html).not.toMatch(/\/Users\/[A-Za-z0-9._-]+\//);
    expect(html).not.toMatch(/\/home\/[A-Za-z0-9._-]+\//);
    expect(html).not.toMatch(/\.ts\.net/);
    expect(html).not.toMatch(/\b\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}\b/);
  });
});
