// tools/self.test.ts — planted-fixture tests for the pure core of the self-model.
//
// Every case here is a failure this actually hit on 2026-07-30, not a hypothetical. The
// graph shapes are real: graphify emits an import specifier and its target file as two
// separate nodes, gives unqualified symbols one global node, and records foreign module
// names in the same field as internal paths. A rollup written against the naive
// assumption ("every node names a file, one node per file") over-counts every TS unit and
// invents coupling that does not exist.
// Run: bun test tools/self.test.ts

import { describe, expect, test } from "bun:test";
import {
  classifyPath,
  couplingFromBazel,
  couplingFromRustPath,
  mergeCoupling,
  rollUp,
  unitForPath,
} from "./lib/self-model.ts";

/** A virtual tree: only these paths "exist". */
const tree = (...paths: string[]) => {
  const set = new Set(paths);
  return (p: string) => set.has(p);
};

describe("classifyPath", () => {
  test("a path that exists verbatim is internal", () => {
    const c = classifyPath("capabilities/comms/src/main.rs", tree("capabilities/comms/src/main.rs"));
    expect(c.cls).toBe("internal");
    expect(c.path).toBe("capabilities/comms/src/main.rs");
  });

  test("existing Graphify memory is local, never internal", () => {
    const path = "graphify-out/memory/2026-08-02-query.md";
    expect(classifyPath(path, tree(), tree(path)).cls).toBe("local");
  });

  test("other ignored output below an internal root is local, not stale", () => {
    const path = "capabilities/comms/debug-cache.json";
    expect(classifyPath(path, tree(), tree(path)).cls).toBe("local");
  });

  test("tracked root documentation remains internal for the unmatched aggregate", () => {
    expect(classifyPath("README.md", tree("README.md"), tree("README.md"))).toMatchObject({
      cls: "internal",
      path: "README.md",
    });
  });

  test("an extension-stripped import specifier resolves to its real file", () => {
    // graphify records `dashboard/src/lib/api` for the node whose file is api.ts.
    const c = classifyPath("dashboard/src/lib/api", tree("dashboard/src/lib/api.ts"));
    expect(c.cls).toBe("internal");
    expect(c.path).toBe("dashboard/src/lib/api.ts");
  });

  test("a .svelte.ts module resolves, not just plain .ts", () => {
    const c = classifyPath(
      "dashboard/src/lib/capabilities.svelte",
      tree("dashboard/src/lib/capabilities.svelte.ts"),
    );
    expect(c.cls).toBe("internal");
    expect(c.path).toBe("dashboard/src/lib/capabilities.svelte.ts");
  });

  test("a bare package name is external, never stale", () => {
    expect(classifyPath("maplibre-gl", tree()).cls).toBe("external");
    expect(classifyPath("svelte", tree()).cls).toBe("external");
  });

  test("a $-alias is external even though it contains a slash", () => {
    expect(classifyPath("$app/navigation", tree()).cls).toBe("external");
  });

  test("an unresolvable path under a known root is stale — the one real defect", () => {
    const c = classifyPath("capabilities/deleted/src/gone.rs", tree());
    expect(c.cls).toBe("stale");
  });

  test("a null or empty source_file is its own class, not silently internal", () => {
    expect(classifyPath(null, tree()).cls).toBe("empty");
    expect(classifyPath("", tree()).cls).toBe("empty");
  });
});

describe("unitForPath", () => {
  test("maps each of the three nouns plus the spine directories", () => {
    expect(unitForPath("capabilities/comms/src/main.rs")).toEqual({ name: "comms", kind: "capability" });
    expect(unitForPath("libs/axon-config/src/lib.rs")).toEqual({ name: "axon-config", kind: "lib" });
    expect(unitForPath("Packs/writing/skills/x.md")).toEqual({ name: "writing", kind: "pack" });
    expect(unitForPath("dashboard/src/routes/+page.svelte")).toEqual({ name: "dashboard", kind: "spine" });
    expect(unitForPath("tools/doctor.ts")).toEqual({ name: "tools", kind: "spine" });
  });

  test("a root-level file belongs to no unit", () => {
    expect(unitForPath("README.md")).toBeNull();
    expect(unitForPath("axon.toml")).toBeNull();
  });
});

describe("rollUp", () => {
  test("local Graphify artifacts do not change the public rollup", () => {
    const tracked = tree("capabilities/comms/src/main.rs");
    const local = "graphify-out/memory/private-query.md";
    const baseline = rollUp(
      [{ id: "code", source_file: "capabilities/comms/src/main.rs" }],
      tracked,
      tracked,
    );
    const withLocal = rollUp(
      [
        { id: "code", source_file: "capabilities/comms/src/main.rs" },
        { id: "local", source_file: local },
      ],
      tracked,
      tree("capabilities/comms/src/main.rs", local),
    );

    expect(withLocal.units).toEqual(baseline.units);
    expect(withLocal.buckets).toEqual(baseline.buckets);
    expect(withLocal.admittedNodes).toBe(baseline.admittedNodes);
  });

  test("a specifier and its target file count as ONE file but TWO nodes", () => {
    // The 2026-07-30 finding: 16 files existed as two nodes each. Prefix matching still
    // put both in the right unit, so unit assignment was safe — but every per-unit file
    // count was inflated. Both numbers are reported so the gap stays visible.
    const r = rollUp(
      [
        { id: "a", source_file: "dashboard/src/lib/api" },
        { id: "b", source_file: "dashboard/src/lib/api.ts" },
      ],
      tree("dashboard/src/lib/api.ts"),
    );
    expect(r.units).toHaveLength(1);
    expect(r.units[0]).toMatchObject({ name: "dashboard", kind: "spine", files: 1, nodes: 2 });
  });

  test("external, empty, stale and unmatched land in named buckets, never in a unit", () => {
    const r = rollUp(
      [
        { id: "1", source_file: "svelte" },
        { id: "2", source_file: "$app/state" },
        { id: "3", source_file: null },
        { id: "4", source_file: "capabilities/gone/src/x.rs" },
        { id: "5", source_file: "README.md" },
        { id: "6", source_file: "capabilities/comms/src/main.rs" },
      ],
      tree("capabilities/comms/src/main.rs", "README.md"),
    );
    expect(r.buckets.external).toBe(2);
    expect(r.buckets.empty).toBe(1);
    expect(r.buckets.stale).toEqual(["capabilities/gone/src/x.rs"]);
    expect(r.buckets.unmatched).toEqual(["README.md"]);
    expect(r.units.map((u) => u.name)).toEqual(["comms"]);
  });

  test("units come back sorted, so two runs on one graph are byte-identical", () => {
    const nodes = [
      { id: "1", source_file: "capabilities/transit/a.rs" },
      { id: "2", source_file: "capabilities/comms/a.rs" },
      { id: "3", source_file: "libs/axon-config/a.rs" },
    ];
    const exists = tree(...nodes.map((n) => n.source_file!));
    expect(rollUp(nodes, exists).units.map((u) => u.name)).toEqual([
      "axon-config",
      "comms",
      "transit",
    ]);
  });
});

describe("couplingFromRustPath", () => {
  test("a #[path] include reaching another unit is coupling", () => {
    const edges = couplingFromRustPath(
      "capabilities/calendar/src/lib.rs",
      '#[path = "../../../libs/axon-config/src/lib.rs"]\npub(crate) mod axon_config;',
    );
    expect(edges).toHaveLength(1);
    expect(edges[0]).toMatchObject({ from: "calendar", to: "axon-config", kind: "rust-path" });
  });

  test("a #[path] include staying inside its own unit is not coupling", () => {
    const edges = couplingFromRustPath(
      "capabilities/calendar/src/lib.rs",
      '#[path = "./helpers/thing.rs"] mod thing;',
    );
    expect(edges).toEqual([]);
  });

  test("plain `use` statements are ignored — they name crates, not units", () => {
    // This is why graphify's import edges were unusable: `use std::sync::OnceLock` says
    // nothing about which unit owns anything.
    const edges = couplingFromRustPath(
      "capabilities/axon-status/src/main.rs",
      "use std::sync::OnceLock;\nuse axum::Router;\nuse serde_json::json;",
    );
    expect(edges).toEqual([]);
  });

  test("a doc comment naming another unit is not coupling", () => {
    // The near-miss that killed text-substring confirmation: axon-status/src/main.rs
    // mentions "scouting" only in a `//!` comment about retired port literals, and a
    // whole-file substring check accepted it as a real import.
    const edges = couplingFromRustPath(
      "capabilities/axon-status/src/main.rs",
      "//! also retired the hardcoded transit/scouting port literals this file used to carry",
    );
    expect(edges).toEqual([]);
  });
});

describe("couplingFromBazel", () => {
  test("a label naming another unit is coupling, in srcs or deps alike", () => {
    const edges = couplingFromBazel(
      "capabilities/axon-status/BUILD.bazel",
      'srcs = ["src/main.rs", "//libs/axon-server:src/lib.rs", "//libs/axon-config:src/lib.rs"],',
    );
    expect(edges.map((e) => e.to).sort()).toEqual(["axon-config", "axon-server"]);
  });

  test("external crate deps are not unit coupling", () => {
    const edges = couplingFromBazel(
      "capabilities/comms/BUILD.bazel",
      'deps = ["@crate_index_comms//:axum", "@crate_index_comms//:serde"],',
    );
    expect(edges).toEqual([]);
  });

  test("a self-reference is not coupling", () => {
    const edges = couplingFromBazel(
      "capabilities/transit/BUILD.bazel",
      'srcs = ["//capabilities/transit:src/lib.rs"],',
    );
    expect(edges).toEqual([]);
  });
});

describe("mergeCoupling", () => {
  test("both evidence kinds are kept per pair, so a one-sided pair stays visible", () => {
    // A pair backed by bazel-label alone is the expected shape for a transitive include
    // (axon-config reaches axon-status through axon-server's own #[path]) and for a real
    // Bazel `deps` entry (scouting -> transit). Keeping the kinds is what lets a reader
    // tell those apart from drift instead of guessing.
    const merged = mergeCoupling([
      { from: "calendar", to: "axon-config", kind: "rust-path", file: "a.rs", evidence: "x" },
      { from: "calendar", to: "axon-config", kind: "bazel-label", file: "BUILD.bazel", evidence: "y" },
      { from: "scouting", to: "transit", kind: "bazel-label", file: "BUILD.bazel", evidence: "z" },
    ]);
    expect(merged).toHaveLength(2);
    expect(merged[0]).toMatchObject({ from: "calendar", to: "axon-config", kinds: ["bazel-label", "rust-path"] });
    expect(merged[1]).toMatchObject({ from: "scouting", to: "transit", kinds: ["bazel-label"] });
  });

  test("output is sorted, so the committed artifact is stable", () => {
    const merged = mergeCoupling([
      { from: "trips", to: "axon-server", kind: "rust-path", file: "a", evidence: "x" },
      { from: "comms", to: "axon-server", kind: "rust-path", file: "b", evidence: "y" },
      { from: "comms", to: "axon-config", kind: "rust-path", file: "c", evidence: "z" },
    ]);
    expect(merged.map((m) => `${m.from}->${m.to}`)).toEqual([
      "comms->axon-config",
      "comms->axon-server",
      "trips->axon-server",
    ]);
  });
});
