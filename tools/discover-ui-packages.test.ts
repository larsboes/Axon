// tools/discover-ui-packages.test.ts — planted-tree tests for UI-package discovery.
//
// The case that matters most is the last one: a capability that grows a UI tomorrow must
// enter the gate without anyone editing a list. That is the whole reason discovery reads
// package metadata instead of naming capabilities, and it is the property a reviewer
// cannot verify by reading the tool — only by planting a package it has never heard of and
// watching it appear.
//
// Every tree is a scratch directory, so no case depends on what this checkout happens to
// contain today.
// Run: bun test tools/discover-ui-packages.test.ts

import { afterEach, describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { discoverUiPackages, isViolation, type UiPackage } from "./discover-ui-packages.ts";

const scratches: string[] = [];
afterEach(() => {
  while (scratches.length) rmSync(scratches.pop()!, { recursive: true, force: true });
});

function newRoot(): string {
  const root = mkdtempSync(join(tmpdir(), "axon-ui-discovery-"));
  scratches.push(root);
  return root;
}

interface PlantOptions {
  scripts?: Record<string, string>;
  deps?: Record<string, string>;
  lockfile?: boolean;
  svelteConfig?: boolean;
}

/** Write a package.json (and by default its lockfile) at `root/dir`. */
function plant(root: string, dir: string, options: PlantOptions = {}): void {
  const { scripts = {}, deps = {}, lockfile = true, svelteConfig = false } = options;
  const abs = join(root, dir);
  mkdirSync(abs, { recursive: true });
  writeFileSync(
    join(abs, "package.json"),
    JSON.stringify({ name: dir.replace(/\//g, "-"), scripts, dependencies: deps }),
  );
  if (lockfile) writeFileSync(join(abs, "bun.lock"), "{}\n");
  if (svelteConfig) writeFileSync(join(abs, "svelte.config.js"), "export default {};\n");
}

const CHECKABLE: PlantOptions = {
  scripts: { check: "svelte-check --tsconfig ./tsconfig.json" },
  deps: { svelte: "^5.0.0" },
};

const find = (packages: UiPackage[], dir: string): UiPackage => {
  const hit = packages.find((p) => p.dir === dir);
  if (!hit) throw new Error(`${dir} was not discovered at all: ${packages.map((p) => p.dir).join(", ")}`);
  return hit;
};

describe("discoverUiPackages", () => {
  test("a package with a check script and a lockfile is checkable", () => {
    const root = newRoot();
    plant(root, "dashboard", CHECKABLE);
    expect(find(discoverUiPackages(root), "dashboard")).toMatchObject({ verdict: "checkable" });
  });

  test("the declared check command reaches the operator, so a typo is visible", () => {
    const root = newRoot();
    plant(root, "dashboard", CHECKABLE);
    expect(find(discoverUiPackages(root), "dashboard").reason).toContain("svelte-check");
  });

  test("a Svelte package with no check script fails rather than being skipped", () => {
    const root = newRoot();
    plant(root, "capabilities/mute/ui", { deps: { svelte: "^5.0.0" } });
    const pkg = find(discoverUiPackages(root), "capabilities/mute/ui");
    expect(pkg.verdict).toBe("no-check-script");
    expect(isViolation(pkg.verdict)).toBe(true);
  });

  test("svelte.config.js alone marks a package as a Svelte UI", () => {
    // A UI can carry svelte only through its adapter's dependency tree. The config file is
    // the framework's own declaration, and missing it would let that UI opt out silently.
    const root = newRoot();
    plant(root, "capabilities/adapter-only/ui", { svelteConfig: true });
    expect(find(discoverUiPackages(root), "capabilities/adapter-only/ui").verdict).toBe(
      "no-check-script",
    );
  });

  test("a check script with no committed lockfile fails", () => {
    const root = newRoot();
    plant(root, "capabilities/loose/ui", { ...CHECKABLE, lockfile: false });
    const pkg = find(discoverUiPackages(root), "capabilities/loose/ui");
    expect(pkg.verdict).toBe("no-lockfile");
    expect(pkg.reason).toContain("reproducible");
  });

  test("bun.lockb counts as a committed resolution", () => {
    // bun wrote the binary lockfile before 1.2. A repository still carrying one has a
    // reproducible install, and refusing it would be a version opinion this gate has no
    // business holding.
    const root = newRoot();
    plant(root, "capabilities/legacy/ui", { ...CHECKABLE, lockfile: false });
    writeFileSync(join(root, "capabilities/legacy/ui/bun.lockb"), "");
    expect(find(discoverUiPackages(root), "capabilities/legacy/ui").verdict).toBe("checkable");
  });

  test("a package that is neither checkable nor Svelte is skipped, not failed", () => {
    const root = newRoot();
    plant(root, "tools/fixture", { scripts: { build: "tsc" }, lockfile: false });
    const pkg = find(discoverUiPackages(root), "tools/fixture");
    expect(pkg.verdict).toBe("not-a-ui");
    expect(isViolation(pkg.verdict)).toBe(false);
  });

  test("a non-Svelte package that declares a check is still checked", () => {
    // "Discover Svelte UIs" is the motivating case, not the rule. A package saying it can
    // be checked is the declaration this gate acts on, whatever renders it.
    const root = newRoot();
    plant(root, "capabilities/plain/ui", { scripts: { check: "tsc --noEmit" } });
    expect(find(discoverUiPackages(root), "capabilities/plain/ui").verdict).toBe("checkable");
  });

  test("installed dependencies and generated trees are not packages of ours", () => {
    const root = newRoot();
    plant(root, "dashboard", CHECKABLE);
    plant(root, "dashboard/node_modules/svelte", CHECKABLE);
    plant(root, "dashboard/.svelte-kit/output", CHECKABLE);
    plant(root, "dashboard/dist/vendor", CHECKABLE);
    expect(discoverUiPackages(root).map((p) => p.dir)).toEqual(["dashboard"]);
  });

  test("unparseable package.json fails instead of vanishing from the sweep", () => {
    const root = newRoot();
    mkdirSync(join(root, "capabilities/broken/ui"), { recursive: true });
    writeFileSync(join(root, "capabilities/broken/ui/package.json"), "{ not json");
    expect(isViolation(find(discoverUiPackages(root), "capabilities/broken/ui").verdict)).toBe(true);
  });

  test("a lockfile Git cannot see fails, because CI clones instead of copying", () => {
    // The local-green/CI-red shape: an ignored lockfile is present on the machine that
    // wrote it and absent from every clone, so the install fails in CI and reads as
    // infrastructure noise rather than a missing commit.
    const root = newRoot();
    plant(root, "capabilities/ignored/ui", CHECKABLE);
    writeFileSync(join(root, ".gitignore"), "bun.lock\n");
    spawnSync("git", ["-C", root, "init", "-q"]);
    spawnSync("git", ["-C", root, "add", "-A"]);
    expect(find(discoverUiPackages(root), "capabilities/ignored/ui").verdict).toBe("untracked");
  });

  test("a capability that grows a UI enters the gate with no edit to this repository", () => {
    const root = newRoot();
    plant(root, "dashboard", CHECKABLE);
    const before = discoverUiPackages(root).filter((p) => p.verdict === "checkable");

    plant(root, "capabilities/invented-tomorrow/ui", CHECKABLE);
    const after = discoverUiPackages(root).filter((p) => p.verdict === "checkable");

    expect(after.length).toBe(before.length + 1);
    expect(after.map((p) => p.dir)).toContain("capabilities/invented-tomorrow/ui");
  });
});
