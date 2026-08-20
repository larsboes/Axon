/**
 * Rule-engine tests for the promoted capability.
 *
 * These run with NO overlay present. The model is written into a temp directory and pointed at
 * with INTERIOR_MODEL_DIR, mirroring the scratch-overlay pattern in libs/overlay/overlay.test.ts,
 * because a generic capability cannot depend on one deployment's private data to prove its rules
 * fire. The question "does the real flat still pass" is a question about that flat, not about
 * this engine, and it is answered by running `interior check` against the overlay.
 *
 * MODEL_DIR is resolved once at import time, so the fixture is written and the env var set
 * before anything from src/ is imported.
 */

import { describe, expect, test } from "bun:test";
import { mkdtempSync, mkdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

function scratchModel(): string {
  const dir = mkdtempSync(join(tmpdir(), "interior-model-"));
  mkdirSync(join(dir, "layouts"), { recursive: true });

  // Deliberately not anyone's flat: it exists to make the rules observable, and a rule that
  // only fires in one real room is not a rule. The dimensions are odd numbers on purpose so
  // the containment test below can look for them in src/ and find nothing.
  writeFileSync(
    join(dir, "room.yaml"),
    [
      "hauptraum:",
      "  polygon:",
      "    - [0, 0]",
      "    - [383, 0]",
      "    - [383, 471]",
      "    - [0, 471]",
      "  hoehe: 250",
      "oeffnungen:",
      "  - id: tuer",
      "    wand: ost",
      "    von: 300",
      "    bis: 400",
      "    breite: 100",
      "    typ: tuer",
      "",
    ].join("\n"),
  );

  writeFileSync(
    join(dir, "constraints.yaml"),
    [
      "laufwege:",
      "  haupt_soll: 90",
      "  haupt_min: 75",
      "  neben_min: 60",
      "abstaende:",
      "  bett_zugang_laengsseite: 60",
      "  bett_zugang_zweite_seite: 40",
      "  schreibtisch_stuhlzone: 100",
      "  esstisch_stuhl_ausziehen: 80",
      "  couchtisch_vor_sofa: 40",
      "  schrank_tuer_oeffnen: 90",
      "  vor_kuechenzeile: 100",
      "regeln: []",
      "",
    ].join("\n"),
  );

  writeFileSync(
    join(dir, "furniture.yaml"),
    [
      "vorhanden:",
      "  - id: couch_fixture",
      "    label: Couch",
      "    b: 200",
      "    t: 85",
      "    h: 70",
      "kandidaten:",
      "  - id: esstisch_fixture",
      "    label: Dining table",
      "    b: 80",
      "    t: 80",
      "    h: 75",
      "  - id: couchtisch_fixture",
      "    label: Coffee table",
      "    b: 90",
      "    t: 46",
      "    h: 45",
      "  - id: kleiderschrank_fixture",
      "    label: Wardrobe",
      "    b: 120",
      "    t: 60",
      "    h: 200",
      "",
    ].join("\n"),
  );

  return dir;
}

process.env.INTERIOR_MODEL_DIR = scratchModel();

const { checkLayout } = await import("../src/clearance.ts");
const { kindOf, loadRoom, polygonAreaM2 } = await import("../src/model.ts");
type Layout = Awaited<ReturnType<typeof import("../src/model.ts").loadLayout>>;
type PlacedItem = Layout["items"][number];

const layout = (name: string, ...items: PlacedItem[]) => ({ name, items });
const rules = (r: Awaited<ReturnType<typeof checkLayout>>, id: string) =>
  [...r.hard, ...r.soft].filter((v) => v.rule === id);

describe("the model loads without an overlay", () => {
  test("reads the fixture room and computes its area from the polygon", async () => {
    const room = await loadRoom();
    expect(room.areaM2).toBeCloseTo(18.04, 2);
    expect(polygonAreaM2([[0, 0], [383, 0], [383, 471], [0, 471]])).toBeCloseTo(18.04, 2);
  });
});

describe("item classification", () => {
  test("a Couchtisch is a coffee table, not a couch", () => {
    // `^couch` matched `couchtisch` before 2026-08-20, so a coffee table was handed the sofa
    // rules and never got its own.
    expect(kindOf({ ref: "couchtisch_fixture", x: 0, y: 0, rot: 0 })).toBe("coffee_table");
    expect(kindOf({ ref: "couch_fixture", x: 0, y: 0, rot: 0 })).toBe("couch");
    expect(kindOf({ ref: "esstisch_fixture", x: 0, y: 0, rot: 0 })).toBe("table");
  });
});

describe("table rules", () => {
  // esstisch_stuhl_ausziehen and couchtisch_vor_sofa were in the constraints schema from the
  // start and no code path read either until 2026-08-20, so a layout with a table passed rules
  // that never ran.
  test("a dining table standing free keeps its chair zones", async () => {
    const r = await checkLayout(layout("t", { ref: "esstisch_fixture", x: 160, y: 160, rot: 0 }));
    expect(rules(r, "stuhl_ausziehen")).toHaveLength(0);
  });

  test("a dining table with no chair room on any side is a hard violation", async () => {
    const r = await checkLayout(
      layout(
        "t",
        { ref: "esstisch_fixture", x: 0, y: 0, rot: 0 },
        { ref: "kleiderschrank_fixture", x: 80, y: 0, rot: 0, size: [100, 100] },
        { ref: "couch_fixture", x: 0, y: 80, rot: 0, size: [100, 100] },
      ),
    );
    const v = rules(r, "stuhl_ausziehen");
    expect(v.length).toBeGreaterThan(0);
    expect(v[0]!.severity).toBe("hart");
  });

  test("a table reachable from one side only reports a seat count, not a failure", async () => {
    const r = await checkLayout(
      layout(
        "t",
        { ref: "esstisch_fixture", x: 160, y: 0, rot: 0 },
        { ref: "kleiderschrank_fixture", x: 60, y: 0, rot: 0, size: [100, 100] },
        { ref: "couch_fixture", x: 240, y: 0, rot: 0, size: [100, 100] },
      ),
    );
    const v = rules(r, "stuhl_ausziehen");
    expect(v.length).toBeGreaterThan(0);
    expect(v[0]!.severity).toBe("weich");
  });

  test("a coffee table jammed against the couch warns", async () => {
    const r = await checkLayout(
      layout(
        "t",
        { ref: "couch_fixture", x: 100, y: 100, rot: 0 },
        { ref: "couchtisch_fixture", x: 100, y: 195, rot: 0 },
      ),
    );
    const v = rules(r, "couchtisch_abstand");
    expect(v.length).toBeGreaterThan(0);
    expect(v[0]!.measured).toBeLessThan(40);
  });
});

describe("wardrobe rules", () => {
  test("a wardrobe with no room to open its doors is a hard violation", async () => {
    const r = await checkLayout(
      layout(
        "t",
        { ref: "kleiderschrank_fixture", x: 0, y: 0, rot: 0 },
        { ref: "couchtisch_fixture", x: 120, y: 0, rot: 0, size: [100, 60] },
        { ref: "couch_fixture", x: 0, y: 60, rot: 0, size: [220, 100] },
      ),
    );
    const v = rules(r, "schrank_tuer");
    expect(v.length).toBeGreaterThan(0);
    expect(v[0]!.severity).toBe("hart");
  });
});

describe("the capability carries no house facts", () => {
  // This is the property that lets a generic capability live in a public repository while the
  // room it plans stays private. The earlier version of this test listed one real flat's
  // dimensions as the forbidden literals, which put those dimensions in the test file — fine in
  // a private overlay, wrong here. It now looks for the fixture's own odd numbers instead.
  const srcDir = join(import.meta.dir, "..", "src");
  const stripComments = (s: string) =>
    s.replace(/\/\*[\s\S]*?\*\//g, "").replace(/\/\/.*$/gm, "");

  test("no model dimension appears in src as a literal", async () => {
    const { readdir, readFile } = await import("node:fs/promises");
    const forbidden = [/\b383\b/, /\b471\b/, /\b18\.04\b/];
    for (const f of await readdir(srcDir)) {
      const code = stripComments(await readFile(join(srcDir, f), "utf8"));
      for (const re of forbidden) expect(code).not.toMatch(re);
    }
  });

  test("no source file writes to the three truth files", async () => {
    const { readdir, readFile } = await import("node:fs/promises");
    for (const f of await readdir(srcDir)) {
      const body = await readFile(join(srcDir, f), "utf8");
      expect(body).not.toMatch(/Bun\.write\s*\(\s*[^)]*(?:ROOM_YAML|FURNITURE_YAML|CONSTRAINTS_YAML)/);
      expect(body).not.toMatch(/writeFile\s*\(\s*[^)]*(?:ROOM_YAML|FURNITURE_YAML|CONSTRAINTS_YAML)/);
    }
  });
});
