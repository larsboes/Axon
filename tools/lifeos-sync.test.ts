// tools/lifeos-sync.test.ts — the two pure cores of lifeos-sync: reading the
// overlay's declared file set, and merging the hook fragment into a live
// settings.json. Both have the same failure mode worth planting a fixture
// against — succeeding quietly while doing less than the report claims.
// Run: bun test tools/lifeos-sync.test.ts

import { describe, expect, test } from "bun:test";
import { mergeHooks, parseOverlayTargets } from "./lifeos-sync.ts";

describe("parseOverlayTargets", () => {
  const manifest = {
    file: [
      { what: "PULSE.toml", src: "config/lifeos/PULSE.toml", dst: "LIFEOS/PULSE/PULSE.toml", why: "instance sinks" },
      { what: "budgets.json", src: "config/lifeos/budgets.json", dst: "LIFEOS/TOOLS/budgets.json", why: "instance caps" },
    ],
  };

  test("src resolves against the overlay, dst against the config root", () => {
    expect(parseOverlayTargets(manifest, "/ov", "/cfg")).toEqual([
      { what: "PULSE.toml", kind: "link", src: "/ov/config/lifeos/PULSE.toml", dst: "/cfg/LIFEOS/PULSE/PULSE.toml", why: "instance sinks" },
      { what: "budgets.json", kind: "link", src: "/ov/config/lifeos/budgets.json", dst: "/cfg/LIFEOS/TOOLS/budgets.json", why: "instance caps" },
    ]);
  });

  test("an overlay declaring nothing is legal — Axon's own targets still deploy", () => {
    expect(parseOverlayTargets({}, "/ov", "/cfg")).toEqual([]);
  });

  test("a target missing a field throws instead of being skipped", () => {
    // The failure this guards: a dropped entry means a file silently stops being
    // synced while the report still ends with "in sync".
    for (const missing of ["what", "src", "dst", "why"]) {
      const broken = { file: [{ ...manifest.file[0], [missing]: undefined }] };
      expect(() => parseOverlayTargets(broken, "/ov", "/cfg")).toThrow(missing);
    }
  });

  test("a blank field is missing, not present", () => {
    expect(() => parseOverlayTargets({ file: [{ ...manifest.file[0], why: "   " }] }, "/ov", "/cfg")).toThrow("why");
  });

  test("the error names which entry, so a long manifest is debuggable", () => {
    const broken = { file: [manifest.file[0], { what: "x", src: "y", dst: "z" }] };
    expect(() => parseOverlayTargets(broken, "/ov", "/cfg")).toThrow("#2");
  });

  test("[[file]] as a non-array is rejected rather than coerced", () => {
    expect(() => parseOverlayTargets({ file: "PULSE.toml" }, "/ov", "/cfg")).toThrow("array of tables");
  });

  test("kind is never taken from the manifest — only links are declarable", () => {
    const sneaky = { file: [{ ...manifest.file[0], kind: "hooks" }] };
    expect(parseOverlayTargets(sneaky, "/ov", "/cfg")[0].kind).toBe("link");
  });
});

describe("mergeHooks", () => {
  const block = (matcher: string, command: string) => ({ matcher, hooks: [{ type: "command", command }] });

  test("a fragment block missing from live is added", () => {
    const { merged, added } = mergeHooks({}, { PostToolUse: [block("Edit", "bun hooks/ProseGate.hook.ts")] });
    expect(merged.PostToolUse).toHaveLength(1);
    expect(added).toEqual(["PostToolUse: ProseGate.hook.ts"]);
  });

  test("re-running adds nothing — the property that makes deploy idempotent", () => {
    const fragment = { PostToolUse: [block("Edit", "bun hooks/ProseGate.hook.ts")] };
    const { merged } = mergeHooks({}, fragment);
    expect(mergeHooks(merged, fragment).added).toEqual([]);
  });

  test("a hand-retimed block is left exactly as it is", () => {
    // Identity is matcher + commands, deliberately not the whole object, so a
    // timeout tuned by hand survives every later deploy.
    const live = { PostToolUse: [{ matcher: "Edit", hooks: [{ type: "command", command: "bun hooks/ProseGate.hook.ts", timeout: 90 }] }] };
    const { merged, added } = mergeHooks(live, { PostToolUse: [block("Edit", "bun hooks/ProseGate.hook.ts")] });
    expect(added).toEqual([]);
    expect(merged.PostToolUse[0].hooks[0].timeout).toBe(90);
  });

  test("live blocks on the same event are preserved, not replaced", () => {
    const live = { PostToolUse: [block("Write", "bun hooks/Other.hook.ts")] };
    const { merged } = mergeHooks(live, { PostToolUse: [block("Edit", "bun hooks/ProseGate.hook.ts")] });
    expect(merged.PostToolUse).toHaveLength(2);
  });

  test("events the fragment never mentions are untouched", () => {
    const live = { SessionStart: [block("*", "bun hooks/Startup.hook.ts")] };
    const { merged } = mergeHooks(live, { PostToolUse: [block("Edit", "bun x.ts")] });
    expect(merged.SessionStart).toEqual(live.SessionStart);
  });

  test("documentation keys explain the delta and never reach settings.json", () => {
    const { merged } = mergeHooks({}, { PostToolUse: [{ ...block("Edit", "bun x.ts"), _why: "explains the delta" }] });
    expect(merged.PostToolUse[0]).not.toHaveProperty("_why");
  });
});
