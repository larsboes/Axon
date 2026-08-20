/**
 * Turns the measured model into a Pascal scene.
 *
 * This is the only file that speaks metres. Everything upstream is centimetres, matching
 * room.yaml, and the conversion happens once, here, at the boundary.
 *
 * Two things are done the careful way rather than the obvious way:
 *
 * Wall identity is resolved by geometry, not by assuming an order. `create_room` returns wall
 * ids in polygon-edge order, but room.yaml's `waende` entries do not all run in the same
 * direction as the corresponding edge — `west` and `sued_hauptraum` are both reversed — so an
 * opening's position is computed from its real coordinates and projected onto the edge.
 *
 * Furniture is scaled to its true footprint. Pascal's catalogue is generic presets, and its
 * `double-bed` is 200 x 250 against a real 160 x 200, so an unscaled item would draw a room
 * that measures wrong. Anything with no measured size is not drawn at all.
 */

import { Pascal } from "./pascal.ts";
import {
  footprint,
  kindOf,
  loadFurniture,
  loadRoom,
  openingSpan,
  type FurnitureItem,
  type ItemKind,
  type Layout,
  type Pt,
  type Room,
} from "./model.ts";

const M = (cm: number) => cm / 100;

export interface BuildResult {
  levelId: string;
  zoneId: string;
  areaM2: number;
  walls: Record<string, string>;
  openings: string[];
  items: Array<{ ref: string; drawn: "asset" | "zone" }>;
  warnings: string[];
}

/** Preferred catalogue asset per kind; absent means the item is drawn as an exact zone. */
const ASSET_FOR: Partial<Record<ItemKind, string>> = {
  bed: "double-bed",
  couch: "sofa",
};

function edgesOf(poly: Pt[]): Array<{ a: Pt; b: Pt }> {
  return poly.map((a, i) => ({ a, b: poly[(i + 1) % poly.length] }));
}

/** Which polygon edge does this point sit on, and how far along it (0..1)? */
function projectOntoEdges(p: Pt, poly: Pt[]): { index: number; t: number } | null {
  const edges = edgesOf(poly);
  let bestIdx = -1;
  let bestT = 0;
  let bestDist = Infinity;
  edges.forEach((e, i) => {
    const dx = e.b[0] - e.a[0];
    const dy = e.b[1] - e.a[1];
    const len2 = dx * dx + dy * dy;
    if (!len2) return;
    let t = ((p[0] - e.a[0]) * dx + (p[1] - e.a[1]) * dy) / len2;
    t = Math.max(0, Math.min(1, t));
    const px = e.a[0] + t * dx;
    const py = e.a[1] + t * dy;
    const d = Math.hypot(p[0] - px, p[1] - py);
    if (d < bestDist) {
      bestDist = d;
      bestIdx = i;
      bestT = t;
    }
  });
  return bestDist <= 5 ? { index: bestIdx, t: bestT } : null;
}

/** Map room.yaml wall names onto polygon edges by matching endpoints, either direction. */
function wallNamesByEdge(room: Room): string[] {
  const edges = edgesOf(room.polygon);
  const same = (a: Pt, b: Pt) => Math.hypot(a[0] - b[0], a[1] - b[1]) < 1;
  return edges.map((e, i) => {
    for (const [name, w] of Object.entries(room.waende)) {
      if ((same(e.a, w.von) && same(e.b, w.bis)) || (same(e.a, w.bis) && same(e.b, w.von))) {
        return name;
      }
    }
    return `edge_${i}`;
  });
}

export async function buildScene(
  p: Pascal,
  opts: { layout?: Layout; wallHeightCm?: number } = {},
): Promise<BuildResult> {
  const [room, catalogue] = await Promise.all([loadRoom(), loadFurniture()]);
  const warnings: string[] = [];

  const height = opts.wallHeightCm ?? room.hoehe ?? 250;
  if (room.hoehe == null) {
    warnings.push(`room height is not measured (todo_aufmass: raumhoehe) — drawing walls at ${height} cm`);
  }

  const scene = await p.call("get_scene", {});
  const levelId = Object.keys(scene.nodes).find((k) => k.startsWith("level_"));
  if (!levelId) throw new Error("pascal returned no level to build on");

  const created = await p.call("create_room", {
    levelId,
    name: "Wohnraum",
    polygon: room.polygon.map(([x, y]) => [M(x), M(y)]),
    wallHeight: `${height}cm`,
    wallThickness: "12cm",
  });

  const names = wallNamesByEdge(room);
  const walls: Record<string, string> = {};
  names.forEach((n, i) => (walls[n] = created.wallIds[i]));

  // --- openings -----------------------------------------------------------
  const openings: string[] = [];
  for (const o of room.oeffnungen) {
    const span = openingSpan(room, o);
    if (!span) {
      warnings.push(`opening "${o.id}" names wall "${o.wand}", which room.yaml does not define`);
      continue;
    }
    const mid: Pt = [(span.a[0] + span.b[0]) / 2, (span.a[1] + span.b[1]) / 2];
    const hit = projectOntoEdges(mid, room.polygon);
    if (!hit) {
      warnings.push(`opening "${o.id}" does not land on any wall of the polygon — skipped`);
      continue;
    }
    const tool = o.typ === "fenster" ? "add_window" : "add_door";
    await p.call(tool, {
      wallId: created.wallIds[hit.index],
      t: hit.t,
      width: `${o.breite}cm`,
      height: o.typ === "fenster" ? "120cm" : "210cm",
    });
    openings.push(o.id);
  }

  // --- fixed furniture ----------------------------------------------------
  const items: BuildResult["items"] = [];
  for (const f of room.fixMoebel) {
    const w = f.x[1] - f.x[0];
    const d = f.y[1] - f.y[0];
    await drawZone(p, levelId, f.id, f.x[0], f.y[0], w, d);
    items.push({ ref: f.id, drawn: "zone" });
  }

  // --- layout -------------------------------------------------------------
  for (const placed of opts.layout?.items ?? []) {
    const cat = catalogue.get(placed.ref);
    let fp: { w: number; d: number; h: number | null };
    try {
      fp = footprint(placed, catalogue);
    } catch (e) {
      warnings.push((e as Error).message);
      continue;
    }
    const kind = kindOf(placed);
    const assetId = ASSET_FOR[kind];
    if (assetId && (await drawAsset(p, levelId, created.slabId ?? levelId, assetId, placed, fp, cat))) {
      items.push({ ref: placed.ref, drawn: "asset" });
    } else {
      await drawZone(p, levelId, placed.ref, placed.x, placed.y, fp.w, fp.d);
      items.push({ ref: placed.ref, drawn: "zone" });
    }
  }

  return {
    levelId,
    zoneId: created.zoneId,
    areaM2: created.areaSqMeters,
    walls,
    openings,
    items,
    warnings,
  };
}

/**
 * The scene graph, unwrapped.
 *
 * `export_json` answers `{ json: "<the whole scene, stringified>" }`, so writing its result
 * straight to disk produces a file whose only key is `json` and whose geometry is trapped
 * inside a string. Anything downstream would have to know that; this unwraps it once, here.
 */
export async function exportScene(p: Pascal): Promise<any> {
  const r = await p.call("export_json", {});
  const raw = typeof r === "string" ? r : typeof r?.json === "string" ? r.json : null;
  return raw ? JSON.parse(raw) : r;
}

/** Exact footprint as a labelled zone. Always dimensionally honest, even when plain. */
async function drawZone(
  p: Pascal,
  levelId: string,
  label: string,
  x: number,
  y: number,
  w: number,
  d: number,
) {
  await p.call("set_zone", {
    levelId,
    label: `${label} ${Math.round(w)}×${Math.round(d)}`,
    polygon: [
      [M(x), M(y)],
      [M(x + w), M(y)],
      [M(x + w), M(y + d)],
      [M(x), M(y + d)],
    ],
  });
}

/** Catalogue item scaled to the real footprint. Returns false if the asset is unavailable. */
async function drawAsset(
  p: Pascal,
  levelId: string,
  target: string,
  assetId: string,
  placed: { x: number; y: number; rot: number },
  fp: { w: number; d: number; h: number | null },
  cat?: FurnitureItem,
): Promise<boolean> {
  const found = await p.call("search_assets", { query: assetId, limit: 8 });
  const list = found?.items ?? found?.results ?? found;
  const asset = Array.isArray(list) ? list.find((a: any) => a.id === assetId) : null;
  if (!asset?.dimensions) return false;

  const placedItem = await p.call("place_item", {
    catalogItemId: assetId,
    targetNodeId: target,
    position: [M(placed.x + fp.w / 2), 0, M(placed.y + fp.d / 2)],
    rotation: `${placed.rot}°`,
  });
  const id = placedItem?.itemId;
  if (!id) return false;

  // Scale against the UNROTATED true size; rotation is applied by the node itself.
  const [aw, ah, ad] = asset.dimensions as [number, number, number];
  const trueW = placed.rot % 180 === 0 ? fp.w : fp.d;
  const trueD = placed.rot % 180 === 0 ? fp.d : fp.w;
  const trueH = fp.h;
  await p.call("apply_patch", {
    patches: [
      {
        op: "update",
        id,
        data: {
          scale: [
            M(trueW) / aw,
            trueH ? M(trueH) / ah : 1,
            M(trueD) / ad,
          ],
        },
      },
    ],
  });
  return true;
}
