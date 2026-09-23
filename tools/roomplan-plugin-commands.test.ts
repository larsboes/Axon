// The RoomPlan plugin's command list, as a build gate.
//
// The defect this closes was live in the worktree and invisible in review: `build.rs` declared
// six commands while the Swift plugin implemented eleven. `tauri_plugin::Builder::new(COMMANDS)`
// is the generator input for `permissions/`, so the five the list omitted — `currentCapture`,
// `acceptPending`, `rejectPending`, `importLegacyReference`, `previewReference` — were listed in
// `permissions/default.toml` by hand and would have been dropped the moment anyone regenerated.
// The UI calls all five, so the failure mode is a working feature answering "not allowed".
//
// It lives in tools/ for the reason tools/dashboard-home-bands.test.ts:8-9 gives: SvelteKit
// type-checks everything under dashboard/src/, and `bun:test` is Bun's, not the browser's.
//
// The second half is the one that keeps this honest: every assertion is preceded by a
// non-empty check. A regex that matches nothing because a file moved would otherwise pass
// vacuously — the "a gate that cannot see its subject must not answer none" failure this
// repository has recorded more than once.
//
// PENDING COMMIT: all three files are currently untracked, so this test cannot run in a fresh
// checkout yet. It skips there and says so rather than failing, because a red gate for work
// nobody has committed is a gate people learn to ignore. It runs — and gates — the moment the
// plugin lands, which is the commit that needs it. Do not commit this file without
// `plugins/roomplan/`, `dashboard/src/lib/roomplan.ts` and `dashboard/src-tauri/`.

import { describe, expect, test } from "bun:test";
import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";

const REPO = resolve(import.meta.dir, "..");
const BUILD_RS = "plugins/roomplan/build.rs";
const SWIFT = "plugins/roomplan/ios/Sources/RoomPlanPlugin/RoomPlanPlugin.swift";
const TS = "dashboard/src/lib/roomplan.ts";

const missing = [BUILD_RS, SWIFT, TS].filter((file) => !existsSync(resolve(REPO, file)));
if (missing.length > 0) {
  console.log(
    `NOTE: reduced coverage — the RoomPlan plugin is not in this checkout, so its command list is ungated: ${missing.join(", ")}`,
  );
}

const read = (relative: string) => readFileSync(resolve(REPO, relative), "utf8");

/** The commands `tauri_plugin::Builder::new(...)` is told about. */
function declaredCommands(): string[] {
  const source = read(BUILD_RS);
  const block = source.match(/const COMMANDS[^=]*=\s*&\[([\s\S]*?)\];/);
  if (!block) throw new Error(`${BUILD_RS}: no COMMANDS array found`);
  return [...block[1].matchAll(/"([A-Za-z]+)"/g)].map((match) => match[1]);
}

/** The commands the native plugin actually implements. */
function swiftHandlers(): string[] {
  const source = read(SWIFT);
  return [...source.matchAll(/@objc (?:public )?(?:override )?func (\w+)\(_ invoke: Invoke\)/g)].map(
    (match) => match[1],
  );
}

/** The commands the web layer actually invokes. */
function invokedCommands(): string[] {
  const source = read(TS);
  return [
    ...source.matchAll(/invoke(?:<[^>]*>)?\(\s*"plugin:roomplan\|([A-Za-z]+)"/g),
  ].map((match) => match[1]);
}

describe.skipIf(missing.length > 0)("the RoomPlan plugin command list matches its native implementation", () => {
  test("the three sides are non-empty, so no assertion below can pass vacuously", () => {
    expect(declaredCommands().length).toBeGreaterThan(0);
    expect(swiftHandlers().length).toBeGreaterThan(0);
    expect(invokedCommands().length).toBeGreaterThan(0);
  });

  test("build.rs declares every Swift handler, and no command Swift does not implement", () => {
    const declared = new Set(declaredCommands());
    const handlers = new Set(swiftHandlers());

    const missing = [...handlers].filter((command) => !declared.has(command)).sort();
    const phantom = [...declared].filter((command) => !handlers.has(command)).sort();

    expect({ missing, phantom }).toEqual({ missing: [], phantom: [] });
  });

  test("every command the web layer invokes is declared", () => {
    const declared = new Set(declaredCommands());
    const undeclared = [...new Set(invokedCommands())]
      .filter((command) => !declared.has(command))
      .sort();

    expect(undeclared).toEqual([]);
  });
});
