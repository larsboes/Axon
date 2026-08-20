/**
 * Loads the measured room model. Read-only, always: `model/room.yaml`,
 * `model/furniture.yaml` and `model/constraints.yaml` are truth captured on site with a tape
 * measure, and nothing in this capability is entitled to correct them.
 *
 * Units are centimetres throughout, matching the model. Metres appear only at the Pascal
 * boundary, in scene.ts, and nowhere else.
 *
 * Coordinates follow room.yaml: origin at the inner north-west corner, x east, y south.
 */

import { CONSTRAINTS_YAML, FURNITURE_YAML, LAYOUTS_DIR, ROOM_YAML } from "./paths.ts";
import { join } from "node:path";

export type Pt = [number, number];

export interface Opening {
  id: string;
  wand: string;
  von: number;
  bis: number;
  breite: number;
  typ?: string;
  sperrflaeche?: { x: [number, number]; y: [number, number] };
  freihaltezone?: number;
  schwenk?: string;
  schwenk_nach_innen?: boolean;
}

export interface FixedFurniture {
  id: string;
  zone?: string;
  x: [number, number];
  y: [number, number];
  tiefe?: number;
  laenge?: number;
  status?: string;
}

export interface Room {
  polygon: Pt[];
  zonen: Record<string, { x: [number, number]; y: [number, number]; flaeche_m2: number }>;
  bad: { x: [number, number]; y: [number, number] };
  waende: Record<string, { von: Pt; bis: Pt; laenge: number; frei: boolean; notiz?: string }>;
  oeffnungen: Opening[];
  fixMoebel: FixedFurniture[];
  hoehe: number | null;
  todoAufmass: string[];
  /** Area in m2 implied by the polygon, computed rather than read, so it cannot drift. */
  areaM2: number;
}

/**
 * Where an opening physically sits, in room coordinates.
 *
 * `von`/`bis` are ABSOLUTE coordinates along whichever axis the wall runs, not offsets from
 * the wall's `von` endpoint. For nord/ost/west the two readings coincide, because those walls
 * start at 0 — only `badtuer` separates them, and read as an offset it lands at x 530..620 on
 * a wall that spans 260..420 and is 160 cm long. Absolute is the reading that works for all
 * four openings, and it is what the model means.
 */
export function openingSpan(room: Room, o: Opening): { a: Pt; b: Pt } | null {
  const wall = room.waende[o.wand];
  if (!wall) return null;
  const horizontal = Math.abs(wall.bis[0] - wall.von[0]) > Math.abs(wall.bis[1] - wall.von[1]);
  return horizontal
    ? { a: [o.von, wall.von[1]], b: [o.bis, wall.von[1]] }
    : { a: [wall.von[0], o.von], b: [wall.von[0], o.bis] };
}

export interface FurnitureItem {
  id: string;
  label: string;
  b?: number;
  t?: number;
  h?: number | [number, number];
  unsicher?: string[];
  status?: string;
  platzbedarf_zone?: number;
  platzbedarf_block?: number;
  hinweis?: string;
  /** "vorhanden" (owned), "kandidaten" (still a decision), "produkte" (a real product, real numbers). */
  group: "vorhanden" | "kandidaten" | "produkte";
}

export interface Constraints {
  laufwege: { haupt_soll: number; haupt_min: number; neben_min: number };
  abstaende: Record<string, number>;
  regeln: Array<{ id: string; schwere: "hart" | "weich"; text: string }>;
}

export interface PlacedItem {
  ref: string;
  x: number;
  y: number;
  /** Degrees clockwise. 0 means `b` runs along x. Only right angles are meaningful here. */
  rot: number;
  /** Optional footprint override, e.g. the 180x80 desk top on the existing frame. */
  size?: [number, number];
  kind?: ItemKind;
}

export type ItemKind =
  | "bed" | "desk" | "couch" | "wardrobe" | "table" | "coffee_table" | "shelf" | "other";

export interface Layout {
  name: string;
  items: PlacedItem[];
}

async function readYaml<T>(path: string): Promise<T> {
  const f = Bun.file(path);
  if (!(await f.exists())) throw new Error(`model file missing: ${path}`);
  return Bun.YAML.parse(await f.text()) as T;
}

/** Shoelace. The model states 22.6; the polygon says 22.70, and the polygon is the geometry. */
export function polygonAreaM2(poly: Pt[]): number {
  let a = 0;
  for (let i = 0; i < poly.length; i++) {
    const [x1, y1] = poly[i];
    const [x2, y2] = poly[(i + 1) % poly.length];
    a += x1 * y2 - x2 * y1;
  }
  return Math.abs(a) / 2 / 10_000;
}

/**
 * Two todo_aufmass entries contain a colon ("heizkoerper vorhanden? falls ja: wand, breite"),
 * so YAML reads them as maps rather than strings. Flatten them back to the line as written.
 */
function normaliseTodo(entry: unknown): string {
  if (typeof entry === "string") return entry;
  if (entry && typeof entry === "object") {
    return Object.entries(entry as Record<string, unknown>)
      .map(([k, v]) => (v == null ? k : `${k}: ${v}`))
      .join("; ");
  }
  return String(entry);
}

export async function loadRoom(): Promise<Room> {
  const r = await readYaml<any>(ROOM_YAML);
  const polygon = r.hauptraum.polygon as Pt[];
  return {
    polygon,
    zonen: r.zonen ?? {},
    bad: r.bad,
    waende: r.waende ?? {},
    oeffnungen: (r.oeffnungen ?? []) as Opening[],
    fixMoebel: (r.fix_moebel ?? []) as FixedFurniture[],
    hoehe: r.hauptraum.hoehe ?? null,
    todoAufmass: (r.todo_aufmass ?? []).map(normaliseTodo),
    areaM2: polygonAreaM2(polygon),
  };
}

export async function loadFurniture(): Promise<Map<string, FurnitureItem>> {
  const f = await readYaml<any>(FURNITURE_YAML);
  const out = new Map<string, FurnitureItem>();
  for (const group of ["vorhanden", "kandidaten", "produkte"] as const) {
    for (const item of f[group] ?? []) out.set(item.id, { ...item, group });
  }
  return out;
}

export async function loadConstraints(): Promise<Constraints> {
  return await readYaml<Constraints>(CONSTRAINTS_YAML);
}

export async function loadLayout(nameOrPath: string): Promise<Layout> {
  const path = nameOrPath.endsWith(".yaml") ? nameOrPath : join(LAYOUTS_DIR, `${nameOrPath}.yaml`);
  return await readYaml<Layout>(path);
}

/**
 * Ids carry their kind in this model (`bett_bestand`, `schreibtisch_gestell`), so the rules
 * that are kind-specific — bed access, chair zones — can find their targets without a second
 * copy of the catalogue. A layout may always override explicitly.
 */
const KIND_PREFIX: Array<[RegExp, ItemKind]> = [
  [/^bett/, "bed"],
  [/^schreibtisch|^buerostuhl/, "desk"],
  [/^couch(?!tisch)|^sofa/, "couch"],
  [/^kleiderschrank|^schrank/, "wardrobe"],
  // Two orderings matter here. `^couch` used to match `couchtisch` and classify a coffee table
  // as a sofa, which the lookahead above now stops; and coffee_table has to precede table,
  // because a Couchtisch wants 40 cm to the sofa, not 80 cm of chair pull-out.
  [/^couchtisch|^beistelltisch/, "coffee_table"],
  [/^esstisch|^klapptisch/, "table"],
  [/^kallax|regal|^lowboard|^raumtrenner/, "shelf"],
];

export function kindOf(item: PlacedItem): ItemKind {
  if (item.kind) return item.kind;
  for (const [re, kind] of KIND_PREFIX) if (re.test(item.ref)) return kind;
  return "other";
}

/** Footprint in cm after rotation, from the layout override or the catalogue. */
export function footprint(
  placed: PlacedItem,
  catalogue: Map<string, FurnitureItem>,
): { w: number; d: number; h: number | null } {
  const cat = catalogue.get(placed.ref);
  const b = placed.size?.[0] ?? cat?.b;
  const t = placed.size?.[1] ?? cat?.t;
  if (b == null || t == null) {
    throw new Error(`no footprint for "${placed.ref}" — not in furniture.yaml and no size: in the layout`);
  }
  const swap = Math.abs(((placed.rot % 180) + 180) % 180 - 90) < 45;
  const rawH = cat?.h;
  const h = Array.isArray(rawH) ? rawH[1] : (rawH ?? null);
  return { w: swap ? t : b, d: swap ? b : t, h };
}

export interface Uncertainty {
  ref: string;
  label: string;
  fields: string[];
  note?: string;
}

/**
 * Every dimension the model itself flags as estimated. Surfaced rather than defaulted,
 * because a checker fed a guessed couch returns a confident answer about a room nobody
 * measured.
 */
export function uncertainties(
  catalogue: Map<string, FurnitureItem>,
  refs?: string[],
): Uncertainty[] {
  const out: Uncertainty[] = [];
  for (const item of catalogue.values()) {
    if (refs && !refs.includes(item.id)) continue;
    if (!item.unsicher?.length) continue;
    out.push({ ref: item.id, label: item.label, fields: item.unsicher, note: item.status });
  }
  return out.sort((a, b) => a.ref.localeCompare(b.ref));
}
