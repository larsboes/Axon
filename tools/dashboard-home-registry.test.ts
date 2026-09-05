// The Home decision registry's contract, checked from disk rather than through Vite.
//
// `registry.ts` discovers kinds with `import.meta.glob`, which only exists inside a Vite
// build. Everything the glob relies on is nevertheless a fact about files — a filename
// matching a key, a `view` string matching a component, a band appearing in PRD §8.1's
// table, an acyclic dependency graph — and each of those is what silently breaks when a
// stream adds a kind. So this reads the directories.
//
// The import scan is the one nobody would think to write and everyone needs: `eager: true`
// puts every kind's transitive imports into Home's statically reachable graph, and
// vite.config.ts fails the build at a 500 kB eager chunk or on a leaked lazy vendor. The
// failure lands on whoever adds the offending kind and reads as a build error nowhere near
// their change.

import { describe, expect, test } from "bun:test";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

const HOME = join(import.meta.dir, "../dashboard/src/lib/home");
const KINDS = join(HOME, "kinds");
const ROWS = join(HOME, "rows");

const kindFiles = readdirSync(KINDS).filter((name) => name.endsWith(".ts"));
const rowFiles = readdirSync(ROWS).filter((name) => name.endsWith(".svelte"));

const kinds = await Promise.all(
  kindFiles.map(async (name) => {
    const module = (await import(join(KINDS, name))) as { default: Record<string, unknown> };
    return { file: name, key: name.replace(/\.ts$/, ""), kind: module.default };
  }),
);

/** PRD §8.1's own bands, plus 610 for the trip retrospective in §8.2. */
const PRD_BANDS = new Set([10_000, 900, 800, 700, 640, 630, 620, 610, 600, 550, 540, 500, 490]);

/** Every file under home/, so the import scan covers a helper a kind pulls in too. */
function sourcesUnder(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) return sourcesUnder(full);
    return /\.(ts|svelte)$/.test(entry.name) ? [full] : [];
  });
}

describe("the registry finds every kind by file, not by name", () => {
  test("there are kinds to check, so a passing run means something", () => {
    expect(kindFiles.length).toBeGreaterThanOrEqual(8);
    expect(rowFiles.length).toBeGreaterThanOrEqual(8);
  });

  test("every kind file exports a default whose key is its filename", () => {
    const mismatched = kinds
      .filter(({ key, kind }) => kind?.key !== key)
      .map(({ file, kind }) => `${file} declares key "${String(kind?.key)}"`);
    expect(mismatched).toEqual([]);
  });

  test("every kind names a row component that exists", () => {
    const missing = kinds
      .filter(({ kind }) => !existsSync(join(ROWS, `${String(kind.view)}.svelte`)))
      .map(({ file, kind }) => `${file} -> rows/${String(kind.view)}.svelte`);
    expect(missing).toEqual([]);
  });

  test("every band appears in the PRD table", () => {
    // Bands are NOT unique. PRD:2213 gives 640 to the purchase decision and PRD:3115
    // routes a doubled month of metered spend to the same band, so a band is a rank and
    // not a slot. A kind on a band the table does not name has to say which row it
    // extends, in its own file.
    const unlisted = kinds
      .filter(({ kind }) => !PRD_BANDS.has(kind.band as number))
      .filter(({ file }) => !readFileSync(join(KINDS, file), "utf8").includes("PRD"))
      .map(({ file, kind }) => `${file} at band ${String(kind.band)}`);
    expect(unlisted).toEqual([]);
  });

  test("every kind implements the whole attention contract", () => {
    const required = [
      "load", "rows", "id", "urgency", "href",
      "whyHere", "startOrDueAt", "candidateStatus", "dataClass", "processingRoute",
    ];
    const incomplete = kinds.flatMap(({ file, kind }) =>
      required.filter((name) => typeof kind[name] !== "function").map((name) => `${file}: ${name}`),
    );
    expect(incomplete).toEqual([]);
  });

  test("every dependsOn names a real kind and the graph is acyclic", () => {
    const byKey = new Map(kinds.map(({ key, kind }) => [key, kind]));
    const unknown = kinds.flatMap(({ file, kind }) =>
      ((kind.dependsOn as string[] | undefined) ?? [])
        .filter((dependency) => !byKey.has(dependency))
        .map((dependency) => `${file} -> ${dependency}`),
    );
    expect(unknown).toEqual([]);

    const state = new Map<string, "open" | "closed">();
    const walk = (key: string, trail: string[]): string[] => {
      if (state.get(key) === "closed") return [];
      if (state.get(key) === "open") return [[...trail, key].join(" -> ")];
      state.set(key, "open");
      const found = ((byKey.get(key)?.dependsOn as string[] | undefined) ?? []).flatMap(
        (next) => walk(next, [...trail, key]),
      );
      state.set(key, "closed");
      return found;
    };
    expect(kinds.flatMap(({ key }) => walk(key, []))).toEqual([]);
  });

  test("the opportunity kind still declares the calendar it reads", () => {
    // Not decoration: its gate drops an opportunity already in the diary and both of its
    // rank adjustments compare it against calendar entries and contexts. Without the
    // declaration its rank would depend on when calendar happened to answer.
    const opportunity = kinds.find(({ key }) => key === "opportunity");
    expect(opportunity?.kind.dependsOn).toEqual(["calendar"]);
  });
});

describe("nothing under home/ drags a lazy vendor into Home's eager chunk", () => {
  test("no file names maplibre, mermaid or vega", () => {
    const offenders = sourcesUnder(HOME)
      .filter((file) => /from\s+["'][^"']*(maplibre|mermaid|vega)/.test(readFileSync(file, "utf8")))
      .map((file) => file.slice(HOME.length + 1));
    expect(offenders).toEqual([]);
  });

  test("no kind reaches nav or api through the $lib alias", () => {
    // A kind must stay importable by plain `bun test`, which cannot resolve a Vite alias.
    const offenders = kindFiles
      .filter((name) => readFileSync(join(KINDS, name), "utf8").includes('from "$lib/'))
      .map((name) => `kinds/${name}`);
    expect(offenders).toEqual([]);
  });
});
