// tools/check-rust-crate-roots.test.ts — the ambiguity rule, on synthetic targets.
//
// The first case is the 2026-07-29 failure verbatim: capabilities/comms included
// //libs/axon-config:src/lib.rs, rules_rust inferred the crate root by basename, chose the
// wrong lib.rs, and produced an rlib exposing none of comms' modules while reporting a
// successful build (Axon#46).
//
// Synthetic rather than a real query: the rule is what is under test, and a test that
// shells out to Bazel would only prove this checkout is currently clean.
// Run: bun test tools/check-rust-crate-roots.test.ts

import { describe, expect, test } from "bun:test";

import { crateRootViolations, type RustTarget } from "./check-rust-crate-roots.ts";

const target = (over: Partial<RustTarget> = {}): RustTarget => ({
  label: "//capabilities/example:example",
  kind: "rust_library",
  srcs: [],
  crateRoot: null,
  ...over,
});

describe("crateRootViolations", () => {
  test("a second lib.rs with no crate_root is the comms failure", () => {
    const t = target({
      srcs: ["//capabilities/comms:src/lib.rs", "//libs/axon-config:src/lib.rs"],
    });
    const [v] = crateRootViolations([t]);
    expect(v.ambiguous).toBe("lib.rs");
    expect(v.candidates).toHaveLength(2);
  });

  test("the same srcs pass once the target names its root", () => {
    const t = target({
      srcs: ["//capabilities/comms:src/lib.rs", "//libs/axon-config:src/lib.rs"],
      crateRoot: "//capabilities/comms:src/lib.rs",
    });
    expect(crateRootViolations([t])).toHaveLength(0);
  });

  test("two main.rs are ambiguous for the same reason", () => {
    const t = target({
      kind: "rust_binary",
      srcs: ["//capabilities/example:src/main.rs", "//vendored/thing:src/main.rs"],
    });
    expect(crateRootViolations([t])[0].ambiguous).toBe("main.rs");
  });

  test("one lib.rs beside one main.rs is not ambiguous", () => {
    // rules_rust picks lib.rs for a library and main.rs for a binary, which is what both
    // mean. Requiring crate_root here would be a rule with no failure behind it.
    const t = target({
      srcs: ["//capabilities/example:src/lib.rs", "//capabilities/example:src/main.rs"],
    });
    expect(crateRootViolations([t])).toHaveLength(0);
  });

  test("a server binary's srcs are read by basename, not by position", () => {
    // scout-server carries src/server.rs plus two shared libs. Only the two lib.rs files
    // collide; the crate_root it declares is neither of them.
    const t = target({
      kind: "rust_binary",
      label: "//capabilities/scouting:scout-server",
      srcs: [
        "//capabilities/scouting:src/server.rs",
        "//libs/axon-server:src/lib.rs",
        "//libs/axon-config:src/lib.rs",
      ],
    });
    expect(crateRootViolations([t])[0].candidates).toEqual([
      "//libs/axon-server:src/lib.rs",
      "//libs/axon-config:src/lib.rs",
    ]);
  });

  test("a test target inheriting its crate has no srcs and nothing to guess", () => {
    expect(crateRootViolations([target({ kind: "rust_test", srcs: [] })])).toHaveLength(0);
  });

  test("many sources with a single root are fine without crate_root", () => {
    const t = target({
      srcs: [
        "//libs/inference:src/lib.rs",
        "//libs/inference:src/router.rs",
        "//libs/inference:src/providers/mod.rs",
        "//libs/inference:src/providers/gemini.rs",
      ],
    });
    expect(crateRootViolations([t])).toHaveLength(0);
  });

  test("every ambiguous target is reported, not just the first", () => {
    const two = [
      target({ label: "//a:a", srcs: ["//a:src/lib.rs", "//libs/x:src/lib.rs"] }),
      target({ label: "//b:b", srcs: ["//b:src/lib.rs", "//libs/y:src/lib.rs"] }),
    ];
    expect(crateRootViolations(two).map((v) => v.target.label)).toEqual(["//a:a", "//b:b"]);
  });
});
