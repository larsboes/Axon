/**
 * The geometry the rules are actually about: empty space.
 *
 * Pascal's own `check_collisions` is documented as an axis-aligned bounding-box test between
 * item footprints, which answers "do two things overlap". Every rule in constraints.yaml asks
 * the opposite question — how much room is left to walk, reach, open a door, or let light
 * past — so none of them can be expressed as an overlap test.
 *
 * Everything here is in centimetres, on a raster of the room. Two primitives carry the work:
 * an exact Euclidean distance transform giving each free point its distance to the nearest
 * obstacle, and a widest-path search giving the narrowest point of the best route between
 * two places. A corridor of width W has clearance W/2 down its centre line, so the bottleneck
 * clearance doubled is the corridor width a person actually gets.
 */

import type { Pt } from "./model.ts";

export interface Rect {
  x: number;
  y: number;
  w: number;
  d: number;
}

export const rectOf = (x: number, y: number, w: number, d: number): Rect => ({ x, y, w, d });

export function rectsOverlap(a: Rect, b: Rect): boolean {
  return a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.d && b.y < a.y + a.d;
}

/** Area of the intersection, cm2. Zero when they do not touch. */
export function overlapArea(a: Rect, b: Rect): number {
  const w = Math.min(a.x + a.w, b.x + b.w) - Math.max(a.x, b.x);
  const d = Math.min(a.y + a.d, b.y + b.d) - Math.max(a.y, b.y);
  return w > 0 && d > 0 ? w * d : 0;
}

export function pointInPolygon([px, py]: Pt, poly: Pt[]): boolean {
  let inside = false;
  for (let i = 0, j = poly.length - 1; i < poly.length; j = i++) {
    const [xi, yi] = poly[i];
    const [xj, yj] = poly[j];
    const intersects = yi > py !== yj > py && px < ((xj - xi) * (py - yi)) / (yj - yi) + xi;
    if (intersects) inside = !inside;
  }
  return inside;
}

export class Grid {
  readonly res: number;
  readonly cols: number;
  readonly rows: number;
  /** true where a person can stand: inside the room polygon and not occupied. */
  readonly free: Uint8Array;

  constructor(poly: Pt[], res = 5) {
    this.res = res;
    const maxX = Math.max(...poly.map((p) => p[0]));
    const maxY = Math.max(...poly.map((p) => p[1]));
    this.cols = Math.ceil(maxX / res);
    this.rows = Math.ceil(maxY / res);
    this.free = new Uint8Array(this.cols * this.rows);
    for (let r = 0; r < this.rows; r++) {
      for (let c = 0; c < this.cols; c++) {
        const centre: Pt = [(c + 0.5) * res, (r + 0.5) * res];
        this.free[r * this.cols + c] = pointInPolygon(centre, poly) ? 1 : 0;
      }
    }
  }

  idx(c: number, r: number) {
    return r * this.cols + c;
  }

  /** Marks a rectangle occupied. Anything outside the polygon is already occupied. */
  block(rect: Rect) {
    const c0 = Math.max(0, Math.floor(rect.x / this.res));
    const c1 = Math.min(this.cols - 1, Math.ceil((rect.x + rect.w) / this.res) - 1);
    const r0 = Math.max(0, Math.floor(rect.y / this.res));
    const r1 = Math.min(this.rows - 1, Math.ceil((rect.y + rect.d) / this.res) - 1);
    for (let r = r0; r <= r1; r++) {
      for (let c = c0; c <= c1; c++) this.free[this.idx(c, r)] = 0;
    }
  }

  cellOf([x, y]: Pt): [number, number] {
    return [
      Math.min(this.cols - 1, Math.max(0, Math.floor(x / this.res))),
      Math.min(this.rows - 1, Math.max(0, Math.floor(y / this.res))),
    ];
  }

  freeArea(): number {
    let n = 0;
    for (const v of this.free) n += v;
    return n * this.res * this.res;
  }
}

/**
 * Nearest cell a person could actually stand in. Waypoints are derived from openings, and an
 * opening's own approach point can land inside its door-swing Sperrfläche, which would make
 * every route from it read as "blocked" when the room is in fact perfectly walkable.
 */
export function nearestFree(
  grid: Grid,
  p: Pt,
  field?: Float64Array,
  radiusCm = 70,
): Pt | null {
  const [c0, r0] = grid.cellOf(p);
  const rad = Math.ceil(radiusCm / grid.res);
  let best: Pt | null = null;
  let bestScore = -1;
  for (let dc = -rad; dc <= rad; dc++) {
    for (let dr = -rad; dr <= rad; dr++) {
      const c = c0 + dc;
      const r = r0 + dr;
      if (c < 0 || r < 0 || c >= grid.cols || r >= grid.rows) continue;
      const i = grid.idx(c, r);
      if (!grid.free[i]) continue;
      // Prefer the most open cell nearby, not merely the closest one. A door's own approach
      // point sits hard against the frame, so anchoring a route there makes the doorway
      // itself the bottleneck and every route reads as impassably narrow.
      const score = field ? field[i] : -Math.hypot(dc, dr);
      if (score > bestScore) {
        bestScore = score;
        best = [(c + 0.5) * grid.res, (r + 0.5) * grid.res];
      }
    }
  }
  return best;
}

/** Felzenszwalb & Huttenlocher, exact squared EDT on one dimension. */
function dt1d(f: Float64Array): Float64Array {
  const n = f.length;
  const d = new Float64Array(n);
  const v = new Int32Array(n);
  const z = new Float64Array(n + 1);
  let k = 0;
  v[0] = 0;
  z[0] = -Infinity;
  z[1] = Infinity;
  for (let q = 1; q < n; q++) {
    let s = (f[q] + q * q - (f[v[k]] + v[k] * v[k])) / (2 * q - 2 * v[k]);
    while (s <= z[k]) {
      k--;
      s = (f[q] + q * q - (f[v[k]] + v[k] * v[k])) / (2 * q - 2 * v[k]);
    }
    k++;
    v[k] = q;
    z[k] = s;
    z[k + 1] = Infinity;
  }
  k = 0;
  for (let q = 0; q < n; q++) {
    while (z[k + 1] < q) k++;
    d[q] = (q - v[k]) * (q - v[k]) + f[v[k]];
  }
  return d;
}

/**
 * Distance in cm from each free cell to the nearest occupied cell or wall. Exact, not a
 * chamfer approximation, because 74 cm and 76 cm are opposite verdicts against a 75 cm floor.
 */
export function clearanceField(grid: Grid): Float64Array {
  const { cols, rows } = grid;
  const INF = 1e12;
  const f = new Float64Array(cols * rows);
  for (let i = 0; i < f.length; i++) f[i] = grid.free[i] ? INF : 0;

  const col = new Float64Array(rows);
  for (let c = 0; c < cols; c++) {
    for (let r = 0; r < rows; r++) col[r] = f[r * cols + c];
    const d = dt1d(col);
    for (let r = 0; r < rows; r++) f[r * cols + c] = d[r];
  }
  const row = new Float64Array(cols);
  for (let r = 0; r < rows; r++) {
    for (let c = 0; c < cols; c++) row[c] = f[r * cols + c];
    const d = dt1d(row);
    for (let c = 0; c < cols; c++) f[r * cols + c] = d[c];
  }
  for (let i = 0; i < f.length; i++) f[i] = Math.sqrt(f[i]) * grid.res;
  return f;
}

/**
 * The widest route from a to b: maximises the narrowest clearance along the path. Returns the
 * bottleneck clearance in cm, or null when no free route exists at all. Doubling it gives the
 * usable corridor width.
 */
export function widestPath(
  grid: Grid,
  field: Float64Array,
  from: Pt,
  to: Pt,
): number | null {
  const { cols, rows } = grid;
  const [c0, r0] = grid.cellOf(from);
  const [c1, r1] = grid.cellOf(to);
  const start = grid.idx(c0, r0);
  const goal = grid.idx(c1, r1);
  if (!grid.free[start] || !grid.free[goal]) return null;

  const best = new Float64Array(cols * rows); // best bottleneck reaching each cell
  const seen = new Uint8Array(cols * rows);
  best[start] = field[start];

  // Small grid (~10k cells); a linear scan for the max beats a heap in both clarity and speed.
  for (;;) {
    let cur = -1;
    let curVal = -1;
    for (let i = 0; i < best.length; i++) {
      if (!seen[i] && best[i] > curVal) {
        curVal = best[i];
        cur = i;
      }
    }
    if (cur < 0 || curVal <= 0) return null;
    if (cur === goal) return curVal;
    seen[cur] = 1;
    const c = cur % cols;
    const r = (cur - c) / cols;
    for (const [dc, dr] of [[1, 0], [-1, 0], [0, 1], [0, -1]] as const) {
      const nc = c + dc;
      const nr = r + dr;
      if (nc < 0 || nr < 0 || nc >= cols || nr >= rows) continue;
      const ni = grid.idx(nc, nr);
      if (seen[ni] || !grid.free[ni]) continue;
      const bottleneck = Math.min(curVal, field[ni]);
      if (bottleneck > best[ni]) best[ni] = bottleneck;
    }
  }
}
