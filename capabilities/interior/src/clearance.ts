/**
 * Judges a layout against model/constraints.yaml.
 *
 * Every threshold is read from the model. Nothing in this file decides how much space a
 * walkway needs; it decides only whether the layout has it.
 *
 * Severity follows constraints.yaml: `hart` blocks, `weich` warns. A layout passes only when
 * no hard rule is violated — there is deliberately no way to report an overall pass with a
 * hard violation outstanding, because a checker that soft-fails is worse than no checker.
 */

import {
  footprint,
  kindOf,
  loadConstraints,
  loadFurniture,
  loadRoom,
  openingSpan,
  uncertainties,
  type Constraints,
  type FurnitureItem,
  type Layout,
  type Opening,
  type Pt,
  type Room,
  type Uncertainty,
} from "./model.ts";
import {
  Grid,
  clearanceField,
  nearestFree,
  overlapArea,
  pointInPolygon,
  rectOf,
  rectsOverlap,
  widestPath,
  type Rect,
} from "./geometry.ts";

export interface Violation {
  rule: string;
  severity: "hart" | "weich";
  item?: string;
  message: string;
  measured?: number;
  required?: number;
}

export interface CheckResult {
  layout: string;
  pass: boolean;
  hard: Violation[];
  soft: Violation[];
  uncertainties: Uncertainty[];
  metrics: {
    roomAreaM2: number;
    occupiedAreaM2: number;
    freeAreaM2: number;
    corridors: Array<{ from: string; to: string; widthCm: number | null }>;
  };
}

/** Resolve an opening (wall + offsets along it) into a segment in room coordinates. */
function openingSegment(room: Room, o: Opening): { a: Pt; b: Pt; normal: Pt } | null {
  const span = openingSpan(room, o);
  if (!span) return null;
  const { a, b } = span;
  const len = Math.hypot(b[0] - a[0], b[1] - a[1]) || 1;
  const ux = (b[0] - a[0]) / len;
  const uy = (b[1] - a[1]) / len;
  // Inward normal: try both perpendiculars, keep the one landing inside the room.
  const mid: Pt = [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2];
  for (const n of [[-uy, ux], [uy, -ux]] as Pt[]) {
    const probe: Pt = [mid[0] + n[0] * 20, mid[1] + n[1] * 20];
    if (pointInPolygon(probe, room.polygon)) return { a, b, normal: n };
  }
  return { a, b, normal: [0, 0] };
}

/** The rectangle a person needs in front of an opening, derived from its own freihaltezone. */
function approachRect(seg: { a: Pt; b: Pt; normal: Pt }, depth: number): Rect {
  const xs = [seg.a[0], seg.b[0], seg.a[0] + seg.normal[0] * depth, seg.b[0] + seg.normal[0] * depth];
  const ys = [seg.a[1], seg.b[1], seg.a[1] + seg.normal[1] * depth, seg.b[1] + seg.normal[1] * depth];
  const x = Math.min(...xs);
  const y = Math.min(...ys);
  return rectOf(x, y, Math.max(...xs) - x, Math.max(...ys) - y);
}

/** A person needs somewhere to stand, and roughly 60 cm of length to stand in. */
const STANDING_RUN_CM = 60;

/**
 * How deep the usable free space runs off one side of a rectangle, capped at `want`.
 *
 * The test is a contiguous run, not a percentage and not every cell. A desk at the head end of
 * a bed blocks part of that long side without making the bed unreachable; what decides access
 * is whether there is a continuous stretch long enough to stand in at the required depth.
 * Requiring the entire strip reported a perfectly usable bed as having zero access.
 */
function freeDepthOnSide(
  grid: Grid,
  r: Rect,
  side: "n" | "s" | "e" | "w",
  want: number,
  minRun = STANDING_RUN_CM,
): number {
  const step = grid.res;
  for (let d = step; d <= want; d += step) {
    const strip =
      side === "n" ? rectOf(r.x, r.y - d, r.w, step)
      : side === "s" ? rectOf(r.x, r.y + r.d + d - step, r.w, step)
      : side === "w" ? rectOf(r.x - d, r.y, step, r.d)
      : rectOf(r.x + r.w + d - step, r.y, step, r.d);
    if (longestFreeRun(grid, strip, side === "n" || side === "s") < minRun) return d - step;
  }
  return want;
}

/** Longest uninterrupted stretch of standable cells along a strip, in cm. */
function longestFreeRun(grid: Grid, s: Rect, alongX: boolean): number {
  const c0 = Math.floor(s.x / grid.res);
  const c1 = Math.ceil((s.x + s.w) / grid.res) - 1;
  const r0 = Math.floor(s.y / grid.res);
  const r1 = Math.ceil((s.y + s.d) / grid.res) - 1;
  let best = 0;
  let run = 0;
  const outer = alongX ? [c0, c1] : [r0, r1];
  for (let i = outer[0]; i <= outer[1]; i++) {
    const c = alongX ? i : c0;
    const r = alongX ? r0 : i;
    const ok = c >= 0 && r >= 0 && c < grid.cols && r < grid.rows && grid.free[grid.idx(c, r)];
    run = ok ? run + grid.res : 0;
    if (run > best) best = run;
  }
  return best;
}

export async function checkLayout(layout: Layout): Promise<CheckResult> {
  const [room, catalogue, rules] = await Promise.all([loadRoom(), loadFurniture(), loadConstraints()]);
  const hard: Violation[] = [];
  const soft: Violation[] = [];

  const placed = layout.items.map((it) => {
    const fp = footprint(it, catalogue);
    return { it, kind: kindOf(it), fp, rect: rectOf(it.x, it.y, fp.w, fp.d) };
  });

  // --- the room itself -----------------------------------------------------
  const grid = new Grid(room.polygon, 5);
  for (const f of room.fixMoebel) {
    grid.block(rectOf(f.x[0], f.y[0], f.x[1] - f.x[0], f.y[1] - f.y[0]));
  }
  // Door-swing Sperrflächen are deliberately NOT blocked here. They constrain where furniture
  // may stand, not where a person may walk — you walk through your own front door's swing arc
  // every day. Blocking them made the arc itself the bottleneck of every route in the flat,
  // reporting an identical 50 cm no matter where the furniture went. R2 tests them directly
  // against item footprints below, which is the rule as constraints.yaml actually states it.
  for (const p of placed) grid.block(p.rect);

  // --- containment and mutual overlap -------------------------------------
  for (const p of placed) {
    const corners: Pt[] = [
      [p.rect.x + 1, p.rect.y + 1],
      [p.rect.x + p.rect.w - 1, p.rect.y + 1],
      [p.rect.x + 1, p.rect.y + p.rect.d - 1],
      [p.rect.x + p.rect.w - 1, p.rect.y + p.rect.d - 1],
    ];
    if (!corners.every((c) => pointInPolygon(c, room.polygon))) {
      hard.push({
        rule: "raumgrenze", severity: "hart", item: p.it.ref,
        message: `"${p.it.ref}" sticks out of the room at ${p.it.x},${p.it.y} (${p.fp.w}×${p.fp.d} cm)`,
      });
    }
  }
  for (let i = 0; i < placed.length; i++) {
    for (let j = i + 1; j < placed.length; j++) {
      const a = placed[i];
      const b = placed[j];
      if (rectsOverlap(a.rect, b.rect)) {
        hard.push({
          rule: "kollision", severity: "hart", item: `${a.it.ref} / ${b.it.ref}`,
          message: `"${a.it.ref}" and "${b.it.ref}" overlap by ${Math.round(overlapArea(a.rect, b.rect) / 100)} dm²`,
        });
      }
    }
  }

  // --- R1 / R7 / R2: approach and blocking zones ---------------------------
  for (const o of room.oeffnungen) {
    if (!o.freihaltezone) continue;
    const seg = openingSegment(room, o);
    if (!seg) continue;
    const zone = approachRect(seg, o.freihaltezone);
    for (const p of placed) {
      if (rectsOverlap(p.rect, zone)) {
        hard.push({
          rule: "R1", severity: "hart", item: p.it.ref,
          required: o.freihaltezone,
          message: `"${p.it.ref}" intrudes on the ${o.freihaltezone} cm approach to ${o.id}`,
        });
      }
    }
  }
  for (const o of room.oeffnungen) {
    if (!o.sperrflaeche) continue;
    const zone = rectOf(o.sperrflaeche.x[0], o.sperrflaeche.y[0],
      o.sperrflaeche.x[1] - o.sperrflaeche.x[0], o.sperrflaeche.y[1] - o.sperrflaeche.y[0]);
    for (const p of placed) {
      if (rectsOverlap(p.rect, zone)) {
        hard.push({
          rule: "R2", severity: "hart", item: p.it.ref,
          message: `"${p.it.ref}" sits in the door-swing zone of ${o.id}`,
        });
      }
    }
  }
  const kitchen = room.fixMoebel.find((f) => f.id === "kuechenzeile");
  if (kitchen) {
    const depth = rules.abstaende.vor_kuechenzeile;
    const zone = rectOf(kitchen.x[0], kitchen.y[0] - depth, kitchen.x[1] - kitchen.x[0], depth);
    for (const p of placed) {
      if (rectsOverlap(p.rect, zone)) {
        hard.push({
          rule: "R7", severity: "hart", item: p.it.ref, required: depth,
          message: `"${p.it.ref}" blocks the ${depth} cm approach to the kitchen run`,
        });
      }
    }
  }

  // --- R3: the light corridor ---------------------------------------------
  // R3 is the one rule constraints.yaml states only as prose ("kein Moebel hoeher als 140 cm
  // im Streifen x 0..150 zwischen y 60..350"), so unlike every other threshold here these
  // four numbers cannot be read structurally and are transcribed. If R3's text changes, this
  // has to change with it.
  const lightCorridor = rectOf(0, 60, 150, 350 - 60);
  const LIGHT_MAX_H = 140;
  for (const p of placed) {
    if (p.fp.h != null && p.fp.h > LIGHT_MAX_H && rectsOverlap(p.rect, lightCorridor)) {
      soft.push({
        rule: "R3", severity: "weich", item: p.it.ref,
        measured: p.fp.h, required: LIGHT_MAX_H,
        message: `"${p.it.ref}" is ${p.fp.h} cm tall in the light corridor (max ${LIGHT_MAX_H}); this room has one window wall`,
      });
    }
  }

  // --- R4: bed head off the glazing ---------------------------------------
  const glazing = room.oeffnungen.find((o) => o.typ?.startsWith("glastuer"));
  if (glazing) {
    const seg = openingSegment(room, glazing);
    if (seg) {
      const band = approachRect(seg, 20);
      for (const p of placed.filter((p) => p.kind === "bed")) {
        if (rectsOverlap(p.rect, band)) {
          soft.push({
            rule: "R4", severity: "weich", item: p.it.ref,
            message: `bed touches the glazing (cold, draught, no privacy to the terrace)`,
          });
        }
      }
    }
  }

  // --- access distances ----------------------------------------------------
  for (const p of placed) {
    if (p.kind === "bed") {
      const long = p.fp.w >= p.fp.d ? (["n", "s"] as const) : (["w", "e"] as const);
      const want = rules.abstaende.bett_zugang_laengsseite;
      const depths = long.map((s) => freeDepthOnSide(grid, p.rect, s, want)).sort((a, b) => b - a);
      const [first, second] = depths;
      if (first < rules.abstaende.bett_zugang_laengsseite) {
        hard.push({
          rule: "bett_zugang", severity: "hart", item: p.it.ref,
          measured: first, required: rules.abstaende.bett_zugang_laengsseite,
          message: `bed has ${first} cm on its best long side, needs ${rules.abstaende.bett_zugang_laengsseite}`,
        });
      }
      if (second < rules.abstaende.bett_zugang_zweite_seite) {
        hard.push({
          rule: "bett_zugang", severity: "hart", item: p.it.ref,
          measured: second, required: rules.abstaende.bett_zugang_zweite_seite,
          message: `bed has ${second} cm on its second long side, needs ${rules.abstaende.bett_zugang_zweite_seite}`,
        });
      }
    }
    if (p.kind === "desk") {
      const want = rules.abstaende.schreibtisch_stuhlzone;
      const best = Math.max(...(["n", "s", "e", "w"] as const).map((s) => freeDepthOnSide(grid, p.rect, s, want)));
      if (best < want) {
        hard.push({
          rule: "stuhlzone", severity: "hart", item: p.it.ref, measured: best, required: want,
          message: `desk has ${best} cm for the chair on its best side, needs ${want}`,
        });
      }
    }
    if (p.kind === "wardrobe") {
      const want = rules.abstaende.schrank_tuer_oeffnen;
      const best = Math.max(...(["n", "s", "e", "w"] as const).map((s) => freeDepthOnSide(grid, p.rect, s, want)));
      if (best < want) {
        hard.push({
          rule: "schrank_tuer", severity: "hart", item: p.it.ref, measured: best, required: want,
          message: `wardrobe has ${best} cm to open its doors, needs ${want}`,
        });
      }
    }

    // A dining table nobody can pull a chair out at is a shelf. constraints.yaml has carried
    // esstisch_stuhl_ausziehen since the model was written and nothing read it until 2026-08-20,
    // so any layout with a table passed a rule that never ran. Hard when no side has the room at
    // all; soft when only one does, because that is a seat count rather than a violation — a
    // wall-mounted folding table legitimately has exactly one approach side.
    if (p.kind === "table") {
      const want = rules.abstaende.esstisch_stuhl_ausziehen;
      const sides = (["n", "s", "e", "w"] as const).map((s) => freeDepthOnSide(grid, p.rect, s, want));
      const best = Math.max(...sides);
      const seats = sides.filter((d) => d >= want).length;
      if (seats === 0) {
        hard.push({
          rule: "stuhl_ausziehen", severity: "hart", item: p.it.ref, measured: best, required: want,
          message: `table has ${best} cm to pull a chair out on its best side, needs ${want}`,
        });
      } else if (seats < 2) {
        soft.push({
          rule: "stuhl_ausziehen", severity: "weich", item: p.it.ref, measured: seats, required: 2,
          message: `table can seat ${seats} — only one side has the ${want} cm a chair needs`,
        });
      }
    }

    // Same story for couchtisch_vor_sofa, unread until the same day. Too close to the sofa and
    // you cannot get past your own knees.
    if (p.kind === "coffee_table") {
      const want = rules.abstaende.couchtisch_vor_sofa;
      for (const c of placed.filter((q) => q.kind === "couch")) {
        const dx = Math.max(c.rect.x - (p.rect.x + p.rect.w), p.rect.x - (c.rect.x + c.rect.w), 0);
        const dy = Math.max(c.rect.y - (p.rect.y + p.rect.d), p.rect.y - (c.rect.y + c.rect.d), 0);
        const gap = Math.round(Math.hypot(dx, dy));
        if (gap < want) {
          soft.push({
            rule: "couchtisch_abstand", severity: "weich", item: p.it.ref, measured: gap, required: want,
            message: `coffee table sits ${gap} cm from the couch, wants ${want}`,
          });
        }
      }
    }
  }

  // --- laufwege: the routes that have to stay walkable ----------------------
  const field = clearanceField(grid);
  const waypoints = new Map<string, Pt>();
  for (const o of room.oeffnungen) {
    if (o.typ === "fenster") continue;
    const seg = openingSegment(room, o);
    if (!seg) continue;
    const mid: Pt = [(seg.a[0] + seg.b[0]) / 2, (seg.a[1] + seg.b[1]) / 2];
    waypoints.set(o.id, [mid[0] + seg.normal[0] * 45, mid[1] + seg.normal[1] * 45]);
  }
  if (kitchen) {
    waypoints.set("kuechenzeile", [(kitchen.x[0] + kitchen.x[1]) / 2, kitchen.y[0] - 45]);
  }

  const routes: Array<[string, string]> = [
    ["eingangstuer", "terrassentuer"],
    ["eingangstuer", "kuechenzeile"],
    ["eingangstuer", "badtuer"],
  ];
  const corridors: CheckResult["metrics"]["corridors"] = [];
  for (const [from, to] of routes) {
    const a = waypoints.get(from);
    const b = waypoints.get(to);
    if (!a || !b) continue;
    const from_ = nearestFree(grid, a, field);
    const to_ = nearestFree(grid, b, field);
    const bottleneck = from_ && to_ ? widestPath(grid, field, from_, to_) : null;
    const width = bottleneck == null ? null : Math.round(bottleneck * 2);
    corridors.push({ from, to, widthCm: width });
    if (width == null) {
      hard.push({
        rule: "laufweg", severity: "hart",
        message: `no walkable route at all from ${from} to ${to}`,
        required: rules.laufwege.haupt_min,
      });
    } else if (width < rules.laufwege.haupt_min) {
      hard.push({
        rule: "laufweg", severity: "hart", measured: width, required: rules.laufwege.haupt_min,
        message: `route ${from} → ${to} narrows to ${width} cm, below the ${rules.laufwege.haupt_min} cm minimum`,
      });
    } else if (width < rules.laufwege.haupt_soll) {
      soft.push({
        rule: "laufweg", severity: "weich", measured: width, required: rules.laufwege.haupt_soll,
        message: `route ${from} → ${to} narrows to ${width} cm, under the ${rules.laufwege.haupt_soll} cm target`,
      });
    }
  }

  const occupied = placed.reduce((n, p) => n + p.fp.w * p.fp.d, 0);
  return {
    layout: layout.name,
    pass: hard.length === 0,
    hard,
    soft,
    uncertainties: uncertainties(catalogue, layout.items.map((i) => i.ref)),
    metrics: {
      roomAreaM2: room.areaM2,
      occupiedAreaM2: occupied / 10_000,
      freeAreaM2: grid.freeArea() / 10_000,
      corridors,
    },
  };
}
