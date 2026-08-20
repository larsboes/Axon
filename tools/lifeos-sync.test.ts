// tools/lifeos-sync.test.ts — the two pure cores of lifeos-sync: reading the
// overlay's declared file set, and merging the hook fragment into a live
// settings.json. Both have the same failure mode worth planting a fixture
// against — succeeding quietly while doing less than the report claims.
// Run: bun test tools/lifeos-sync.test.ts

import { describe, expect, test } from "bun:test";
import { mergeHooks, parseOverlayTargets, writebackPlan } from "./lifeos-sync.ts";

const entry = (hash: string, mtimeMs: number) => ({ hash, mtimeMs });

describe("parseOverlayTargets", () => {
  const manifest = {
    file: [
      { what: "PULSE.toml", src: "config/lifeos/PULSE.toml", dst: "LIFEOS/PULSE/PULSE.toml", why: "instance sinks" },
      { what: "budgets.json", src: "config/lifeos/budgets.json", dst: "LIFEOS/TOOLS/budgets.json", why: "instance caps" },
    ],
  };

  test("src resolves against the overlay, dst against the config root", () => {
    expect(parseOverlayTargets(manifest, "/ov", "/cfg")).toEqual([
      { what: "PULSE.toml", kind: "copy", src: "/ov/config/lifeos/PULSE.toml", dst: "/cfg/LIFEOS/PULSE/PULSE.toml", why: "instance sinks", writeback: false },
      { what: "budgets.json", kind: "copy", src: "/ov/config/lifeos/budgets.json", dst: "/cfg/LIFEOS/TOOLS/budgets.json", why: "instance caps", writeback: false },
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

  test("kind is never taken from the manifest — only copies are declarable", () => {
    const sneaky = { file: [{ ...manifest.file[0], kind: "hooks" }] };
    expect(parseOverlayTargets(sneaky, "/ov", "/cfg")[0].kind).toBe("copy");
  });

  test("writeback defaults off — a target is read-only unless it says otherwise", () => {
    expect(parseOverlayTargets(manifest, "/ov", "/cfg")[0].writeback).toBe(false);
  });

  test("writeback is honoured when declared", () => {
    const w = { file: [{ ...manifest.file[0], writeback: true }] };
    expect(parseOverlayTargets(w, "/ov", "/cfg")[0].writeback).toBe(true);
  });

  test("a non-boolean writeback throws rather than being coerced", () => {
    // `writeback = "yes"` is truthy in JS. Coercing it would silently turn a
    // read-only target into one that copies live content back into the overlay.
    for (const bad of ["yes", 1, "false"]) {
      expect(() => parseOverlayTargets({ file: [{ ...manifest.file[0], writeback: bad }] }, "/ov", "/cfg")).toThrow("writeback");
    }
  });
});

describe("writebackPlan", () => {
  test("a live file that is both newer and different is pulled back", () => {
    const src = new Map([["CURRENT.md", entry("a", 100)]]);
    const dst = new Map([["CURRENT.md", entry("b", 200)]]);
    expect(writebackPlan(src, dst)).toEqual(["CURRENT.md"]);
  });

  test("newer but identical is not an edit — a touch must not count", () => {
    const src = new Map([["CURRENT.md", entry("a", 100)]]);
    const dst = new Map([["CURRENT.md", entry("a", 999)]]);
    expect(writebackPlan(src, dst)).toEqual([]);
  });

  test("different but older never wins — a stale live copy must not overwrite a deliberate overlay edit", () => {
    const src = new Map([["CURRENT.md", entry("new", 500)]]);
    const dst = new Map([["CURRENT.md", entry("old", 100)]]);
    expect(writebackPlan(src, dst)).toEqual([]);
  });

  test("a file only the live tree has is pulled back — that is generator output", () => {
    const src = new Map([["CURRENT.md", entry("a", 100)]]);
    const dst = new Map([["CURRENT.md", entry("a", 100)], ["DERIVED/state.json", entry("x", 300)]]);
    expect(writebackPlan(src, dst)).toEqual(["DERIVED/state.json"]);
  });

  test("a file only the overlay has is left alone — deploy is what puts it live", () => {
    const src = new Map([["A.md", entry("a", 100)], ["B.md", entry("b", 100)]]);
    const dst = new Map([["A.md", entry("a", 100)]]);
    expect(writebackPlan(src, dst)).toEqual([]);
  });

  test("two identical trees plan nothing, so a re-run stays a no-op", () => {
    const t = () => new Map([["A.md", entry("a", 100)], ["B.md", entry("b", 200)]]);
    expect(writebackPlan(t(), t())).toEqual([]);
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

  // The subset bug, 2026-08-20. Identity used to be the whole block: matcher plus
  // every command joined. So a fragment block whose commands were a SUBSET of a
  // live block's never matched, and deploy appended it — registering hooks that
  // were already there a second time. Found on a live install where the fragment's
  // two-hook UserPromptSubmit block sat inside a live six-hook block; status called
  // both permanently missing.
  const multi = (matcher: string, ...commands: string[]) => ({
    matcher,
    hooks: commands.map((command) => ({ type: "command", command })),
  });
  const h = (name: string) => `bun hooks/${name}.hook.ts`;

  test("a fragment block already contained in a bigger live block adds nothing", () => {
    const live = { UserPromptSubmit: [multi("*", h("A"), h("B"), h("C"))] };
    const { merged, added } = mergeHooks(live, { UserPromptSubmit: [multi("*", h("C"), h("A"))] });
    expect(added).toEqual([]);
    expect(merged.UserPromptSubmit).toHaveLength(1);
  });

  test("order within a block is not identity — the same commands reordered are the same hooks", () => {
    const live = { PostToolUse: [multi("Edit", h("A"), h("B"))] };
    const { added } = mergeHooks(live, { PostToolUse: [multi("Edit", h("B"), h("A"))] });
    expect(added).toEqual([]);
  });

  test("a partly-new block contributes only its new hooks", () => {
    const live = { PostToolUse: [multi("Edit", h("A"), h("B"))] };
    const { merged, added } = mergeHooks(live, { PostToolUse: [multi("Edit", h("B"), h("New"))] });
    expect(added).toEqual(["PostToolUse: New.hook.ts"]);
    expect(merged.PostToolUse).toHaveLength(2);
    expect(merged.PostToolUse[1].hooks).toHaveLength(1);
    expect(merged.PostToolUse[0].hooks).toHaveLength(2);
  });

  test("the same command under a different matcher is a different hook", () => {
    const live = { PostToolUse: [block("Edit", h("A"))] };
    const { added } = mergeHooks(live, { PostToolUse: [block("Write", h("A"))] });
    expect(added).toEqual(["PostToolUse: A.hook.ts"]);
  });

  test("a partial add is still idempotent — deploying twice settles", () => {
    const live = { PostToolUse: [multi("Edit", h("A"))] };
    const fragment = { PostToolUse: [multi("Edit", h("A"), h("New"))] };
    const once = mergeHooks(live, fragment);
    expect(once.added).toEqual(["PostToolUse: New.hook.ts"]);
    expect(mergeHooks(once.merged, fragment).added).toEqual([]);
  });

  test("a hand-retimed hook inside a bigger block is not re-added or rewritten", () => {
    const live = { UserPromptSubmit: [{ matcher: "*", hooks: [
      { type: "command", command: h("A") },
      { type: "command", command: h("B"), timeout: 90 },
    ] }] };
    const { merged, added } = mergeHooks(live, { UserPromptSubmit: [multi("*", h("B"))] });
    expect(added).toEqual([]);
    expect(merged.UserPromptSubmit[0].hooks[1].timeout).toBe(90);
  });
});
