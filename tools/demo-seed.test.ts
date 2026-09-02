// Tests for tools/demo-seed.ts.
//
// One property, and it is a security one. `activeOverlay` asks paths.sh where the overlay is
// by running bash, and the answer decides whether demo-seed is allowed to write at all. It
// used to build that bash program by interpolating the checkout path into the `-c` string, so
// a checkout directory whose name contained `$(` or `"` was executed rather than read
// (CodeQL js/shell-command-injection-from-environment, tools/demo-seed.ts). The path is now a
// positional argument.
//
// The test runs the real function against a real hostile directory name rather than asserting
// something about the source text, because the shape of the argv is not the property -- the
// property is that the injected command does not run.

import { describe, expect, test } from "bun:test";
import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { activeOverlay } from "./demo-seed.ts";

describe("activeOverlay", () => {
  test("reads a paths.sh whose directory name is a shell command", () => {
    const root = mkdtempSync(join(tmpdir(), "axon-demo-seed-"));
    try {
      const marker = join(root, "INJECTED");
      // Every metacharacter that mattered, in one directory name: command substitution,
      // a quote to break out of the old `source "..."`, and a backtick.
      const hostile = join(root, `d"$(touch '${marker}')\`touch '${marker}'\`x`);
      mkdirSync(hostile, { recursive: true });
      const pathsSh = join(hostile, "paths.sh");
      // A stand-in for tools/lib/paths.sh: `source`d for one variable, which is all
      // activeOverlay reads out of it.
      writeFileSync(pathsSh, 'AXON_OVERLAY_ROOT="/tmp/axon-demo-seed-overlay"\n');

      expect(activeOverlay(pathsSh)).toBe("/tmp/axon-demo-seed-overlay");
      expect(existsSync(marker)).toBe(false);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  test("fails loudly when paths.sh is not there, rather than reporting no overlay", () => {
    expect(() => activeOverlay(join(tmpdir(), "axon-demo-seed-absent", "paths.sh"))).toThrow();
  });
});
