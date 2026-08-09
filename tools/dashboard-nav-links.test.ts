// Every internal link must survive a base path (#170).
//
// This is a source scan rather than a behavioural test, because the bug it catches is
// invisible in the only environment anyone develops in. `paths.base` is empty on a real
// machine, so `href="/calendar"` and `href={link("/calendar")}` are byte-identical there and
// stay identical through every test, every dev session and every code review. They differ
// only on the published demo, which is served from a subdirectory — where the raw form sent
// every click to the domain root and GitHub answered with its own 404.
//
// A reviewer cannot be expected to hold that in their head across 37 call sites. A grep can.
//
// It lives in tools/ rather than beside the dashboard because SvelteKit's generated tsconfig
// type-checks everything under `src/`, and `bun run check` then fails on `bun:test` and
// `import.meta.dir`, which are Bun's and not the browser's. Every other Bun test in this
// repository is here too, and the root `bun test` finds it either way.

import { describe, expect, test } from "bun:test";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { extname, join } from "node:path";

const SRC = join(import.meta.dir, "../dashboard/src");

function sources(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const full = join(dir, name);
    if (statSync(full).isDirectory()) return sources(full);
    return [".svelte", ".ts"].includes(extname(name)) && !name.endsWith(".test.ts") ? [full] : [];
  });
}

/** `//` is a protocol-relative URL, not an app route — the one leading slash that is fine. */
const RAW_HREF = /href="\/(?!\/)/g;
const RAW_GOTO = /goto\(\s*["'`]\//g;
/** A function handing back an app path. These are the sneaky ones: the literal is nowhere
 *  near the `href` that consumes it, so wrapping at the call site misses them. */
const RAW_RETURN = /return\s+["'`]\/(?!\/)/g;

const relative = (file: string) => file.slice(SRC.length + 1);

describe("internal links are base-aware", () => {
  const files = sources(SRC);

  test("there are sources to scan, so a passing run means something", () => {
    expect(files.length).toBeGreaterThan(30);
  });

  test("no component writes a raw absolute href", () => {
    const offenders = files
      .map((file) => [relative(file), [...readFileSync(file, "utf8").matchAll(RAW_HREF)].length] as const)
      .filter(([, n]) => n > 0);
    // Named in the failure, because "some file somewhere" is not actionable.
    expect(offenders.map(([file, n]) => `${file} (${n})`)).toEqual([]);
  });

  test("no component navigates to a raw absolute path", () => {
    const offenders = files
      .map((file) => [relative(file), [...readFileSync(file, "utf8").matchAll(RAW_GOTO)].length] as const)
      .filter(([, n]) => n > 0);
    expect(offenders.map(([file, n]) => `${file} (${n})`)).toEqual([]);
  });

  test("no helper returns a raw absolute path", () => {
    const offenders = files
      .map((file) => [relative(file), [...readFileSync(file, "utf8").matchAll(RAW_RETURN)].length] as const)
      .filter(([, n]) => n > 0);
    expect(offenders.map(([file, n]) => `${file} (${n})`)).toEqual([]);
  });

  // Both halves of the contract, exercised directly now that nav.ts imports no Vite alias.
  test("link() is transparent until a base path is configured", async () => {
    const { link, setBase } = await import("../dashboard/src/lib/nav.ts");
    setBase("");
    expect(link("/calendar")).toBe("/calendar");
    expect(link("/")).toBe("/");
  });

  test("link() prefixes every path once the demo's base is configured", async () => {
    const { link, setBase } = await import("../dashboard/src/lib/nav.ts");
    setBase("/Axon");
    expect(link("/calendar")).toBe("/Axon/calendar");
    expect(link("/feed/abc?source=calendar")).toBe("/Axon/feed/abc?source=calendar");
    setBase(""); // leave the module as found, for whatever imports it next
  });

  // Only the root layout may read the Vite alias, and among plain modules only it. A .svelte
  // file is always compiled by Vite and can never appear in a plain-bun import graph, so the
  // rule that matters is about `.ts`: the moment a second one imports `$app/paths`, every
  // Bun test that transitively reaches it fails to resolve — with an error naming a file
  // nowhere near the change, which is exactly how this was found.
  test("no plain module outside the root layout imports $app/paths", () => {
    const readers = files
      .filter((f) => f.endsWith(".ts"))
      .filter((f) => readFileSync(f, "utf8").includes('from "$app/paths"'));
    expect(readers.map(relative)).toEqual(["routes/+layout.ts"]);
  });
});
