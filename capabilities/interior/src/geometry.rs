//! Raster, Distanzfeld, Wegsuche.
//!
//! Alles in Zentimetern. Zwei Primitive tragen die Arbeit:
//!   * eine exakte euklidische Distanztransformation, die jedem freien Punkt seinen Abstand
//!     zum naechsten belegten Punkt gibt, und
//!   * eine Maximum-Bottleneck-Suche, die die engste Stelle des breitesten Weges liefert.
//!
//! Ein Korridor der Breite W hat auf seiner Mittellinie die Freiheit W/2 — die verdoppelte
//! Engstelle ist deshalb die Breite, die ein Mensch tatsaechlich bekommt.

use crate::model::{Pt, Rect};
use std::collections::BinaryHeap;

/// Rasterweite in cm. 5 cm ist die Aufloesung, in der die Vorlaeuferimplementierung gemessen
/// hat; die aufgezeichneten Routenbreiten der Baseline gelten fuer genau diesen Wert. Wer ihn
/// aendert, aendert die Zahlen und muss die Baseline neu begruenden. Sie liegt im Overlay
/// neben der Wohnung, die sie beschreibt — siehe tests/live_parity.rs.
pub const RES: i32 = 5;

pub fn polygon_area_m2(poly: &[Pt]) -> f64 {
    let mut a = 0i64;
    for i in 0..poly.len() {
        let j = (i + 1) % poly.len();
        a += poly[i][0] as i64 * poly[j][1] as i64 - poly[j][0] as i64 * poly[i][1] as i64;
    }
    (a.abs() as f64 / 2.0) / 10_000.0
}

pub fn point_in_polygon(p: [f64; 2], poly: &[Pt]) -> bool {
    let mut inside = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let (xi, yi) = (poly[i][0] as f64, poly[i][1] as f64);
        let (xj, yj) = (poly[j][0] as f64, poly[j][1] as f64);
        if (yi > p[1]) != (yj > p[1]) && p[0] < (xj - xi) * (p[1] - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

pub struct Grid {
    pub cols: usize,
    pub rows: usize,
    /// 1, wo ein Mensch stehen kann: im Wohnpolygon und nicht belegt.
    pub free: Vec<u8>,
}

impl Grid {
    pub fn new(poly: &[Pt]) -> Grid {
        let max_x = poly.iter().map(|p| p[0]).max().unwrap_or(0);
        let max_y = poly.iter().map(|p| p[1]).max().unwrap_or(0);
        let cols = (max_x as f64 / RES as f64).ceil() as usize;
        let rows = (max_y as f64 / RES as f64).ceil() as usize;
        let mut free = vec![0u8; cols * rows];
        for r in 0..rows {
            for c in 0..cols {
                let centre = [(c as f64 + 0.5) * RES as f64, (r as f64 + 0.5) * RES as f64];
                free[r * cols + c] = point_in_polygon(centre, poly) as u8;
            }
        }
        Grid { cols, rows, free }
    }

    #[inline]
    pub fn idx(&self, c: usize, r: usize) -> usize {
        r * self.cols + c
    }

    pub fn cell_of(&self, p: Pt) -> (usize, usize) {
        let c = (p[0] / RES).clamp(0, self.cols as i32 - 1) as usize;
        let r = (p[1] / RES).clamp(0, self.rows as i32 - 1) as usize;
        (c, r)
    }

    pub fn occupy(&mut self, rect: &Rect) {
        let c0 = (rect.x / RES).max(0) as usize;
        let c1 = (((rect.right() as f64) / RES as f64).ceil() as i32 - 1).min(self.cols as i32 - 1);
        let r0 = (rect.y / RES).max(0) as usize;
        let r1 =
            (((rect.bottom() as f64) / RES as f64).ceil() as i32 - 1).min(self.rows as i32 - 1);
        if c1 < 0 || r1 < 0 {
            return;
        }
        for r in r0..=(r1 as usize) {
            for c in c0..=(c1 as usize) {
                let i = self.idx(c, r);
                self.free[i] = 0;
            }
        }
    }

    pub fn free_cells(&self) -> usize {
        self.free.iter().filter(|v| **v == 1).count()
    }
}

/// Exakte 1D-Distanztransformation (Felzenszwalb/Huttenlocher) — die Bausteine der 2D-Variante.
fn edt_1d(f: &[f64], out: &mut [f64]) {
    let n = f.len();
    let mut v = vec![0usize; n];
    let mut z = vec![0.0f64; n + 1];
    let mut k = 0usize;
    z[0] = f64::NEG_INFINITY;
    z[1] = f64::INFINITY;
    for q in 1..n {
        loop {
            let p = v[k];
            let s = ((f[q] + (q * q) as f64) - (f[p] + (p * p) as f64))
                / (2.0 * q as f64 - 2.0 * p as f64);
            if s <= z[k] {
                if k == 0 {
                    break;
                }
                k -= 1;
            } else {
                k += 1;
                v[k] = q;
                z[k] = s;
                z[k + 1] = f64::INFINITY;
                break;
            }
        }
        if k == 0 && z[1].is_infinite() && v[0] != q {
            // Der Rueckwaertslauf ist bis zur Wurzel gelaufen: q loest die bisherige Parabel ab.
            v[0] = q;
            z[0] = f64::NEG_INFINITY;
            z[1] = f64::INFINITY;
        }
    }
    let mut k = 0usize;
    for (q, cell) in out.iter_mut().enumerate().take(n) {
        while z[k + 1] < q as f64 {
            k += 1;
        }
        let p = v[k];
        let d = q as f64 - p as f64;
        *cell = d * d + f[p];
    }
}

/// Abstand in cm von jeder freien Zelle zur naechsten belegten Zelle oder Wand. Exakt, nicht
/// genaehert: eine Chamfer-Naeherung verschiebt Korridorbreiten um mehrere Zentimeter, und
/// genau an diesen Zentimetern entscheidet sich hier Bestehen oder Verstoss.
pub fn clearance_field(grid: &Grid) -> Vec<f64> {
    const INF: f64 = 1e12;
    let (cols, rows) = (grid.cols, grid.rows);
    let mut f: Vec<f64> = grid
        .free
        .iter()
        .map(|v| if *v == 1 { INF } else { 0.0 })
        .collect();

    let mut col = vec![0.0f64; rows];
    let mut dst = vec![0.0f64; rows];
    for c in 0..cols {
        for r in 0..rows {
            col[r] = f[r * cols + c];
        }
        edt_1d(&col, &mut dst);
        for r in 0..rows {
            f[r * cols + c] = dst[r];
        }
    }
    let mut row = vec![0.0f64; cols];
    let mut dst = vec![0.0f64; cols];
    for r in 0..rows {
        row.copy_from_slice(&f[r * cols..(r + 1) * cols]);
        edt_1d(&row, &mut dst);
        f[r * cols..(r + 1) * cols].copy_from_slice(&dst);
    }
    for v in f.iter_mut() {
        *v = v.sqrt() * RES as f64;
    }
    f
}

/// Naechste Zelle, in der ein Mensch tatsaechlich stehen koennte. Wegpunkte kommen aus
/// Oeffnungen und liegen deshalb per Definition in einer Wand.
/// Radius 70 cm wie in der Vorlage. Kleiner gewaehlt findet die Suche neben einer breiten
/// Tuer keine offene Zelle mehr, und der Tuerrahmen wird zur Engstelle jeder Route.
pub fn nearest_free(grid: &Grid, field: Option<&[f64]>, p: Pt, radius_cm: i32) -> Option<usize> {
    let radius_cells = (radius_cm as f64 / RES as f64).ceil() as i32;
    let (c0, r0) = grid.cell_of(p);
    let mut best: Option<(f64, usize)> = None;
    for dr in -radius_cells..=radius_cells {
        for dc in -radius_cells..=radius_cells {
            let c = c0 as i32 + dc;
            let r = r0 as i32 + dr;
            if c < 0 || r < 0 || c >= grid.cols as i32 || r >= grid.rows as i32 {
                continue;
            }
            let i = grid.idx(c as usize, r as usize);
            if grid.free[i] == 0 {
                continue;
            }
            // Die offenste Zelle in der Naehe, nicht die naechstgelegene: die Anlaufzone einer
            // Tuer ist der Ort, an dem man steht, nicht ihr Rahmen.
            let score = match field {
                Some(f) => f[i],
                None => -(((dc * dc + dr * dr) as f64).sqrt()),
            };
            if best.is_none_or(|(s, _)| score > s) {
                best = Some((score, i));
            }
        }
    }
    best.map(|(_, i)| i)
}

#[derive(PartialEq)]
struct HeapEntry(f64, usize);
impl Eq for HeapEntry {}
impl Ord for HeapEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0
            .partial_cmp(&other.0)
            .unwrap_or(std::cmp::Ordering::Equal)
    }
}
impl PartialOrd for HeapEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Engste Stelle des breitesten Weges, in cm — hoeher ist besser.
///
/// Max-Heap, nicht linearer Scan. Die TypeScript-Vorlage suchte das Maximum bei jeder Entnahme
/// mit einem Durchlauf ueber alle ~10.000 Zellen; das ist O(N²) je Route und war am 2026-08-30
/// mit 67,7 ms pro Layoutpruefung messbar. Mit Heap: 2,7 ms. Der Kommentar dort behauptete,
/// der Scan schlage einen Heap "in both clarity and speed" — nachgemessen hat das nie jemand.
pub fn widest_path(grid: &Grid, field: &[f64], from: Pt, to: Pt) -> Option<f64> {
    let start = nearest_free(grid, Some(field), from, 70)?;
    let goal = nearest_free(grid, Some(field), to, 70)?;

    let n = grid.cols * grid.rows;
    let mut best = vec![0.0f64; n];
    let mut seen = vec![0u8; n];
    let mut heap: BinaryHeap<HeapEntry> = BinaryHeap::new();
    best[start] = field[start];
    heap.push(HeapEntry(field[start], start));

    while let Some(HeapEntry(cur_val, cur)) = heap.pop() {
        if seen[cur] == 1 {
            continue; // veralteter Eintrag; ein BinaryHeap kennt kein decrease-key
        }
        if cur_val <= 0.0 {
            return None;
        }
        if cur == goal {
            return Some(cur_val);
        }
        seen[cur] = 1;
        let c = (cur % grid.cols) as i32;
        let r = (cur / grid.cols) as i32;
        for (dc, dr) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let (nc, nr) = (c + dc, r + dr);
            if nc < 0 || nr < 0 || nc >= grid.cols as i32 || nr >= grid.rows as i32 {
                continue;
            }
            let ni = grid.idx(nc as usize, nr as usize);
            if seen[ni] == 1 || grid.free[ni] == 0 {
                continue;
            }
            let bottleneck = cur_val.min(field[ni]);
            if bottleneck > best[ni] {
                best[ni] = bottleneck;
                heap.push(HeapEntry(bottleneck, ni));
            }
        }
    }
    None
}

/// Ein Mensch braucht einen Platz zum Stehen, und ungefaehr 60 cm Laenge, um darin zu stehen.
pub const STANDING_RUN_CM: i32 = 60;

/// Freie Tiefe an einer Seite eines Rechtecks, gedeckelt auf `want`.
///
/// Geprueft wird ein ZUSAMMENHAENGENDER Lauf, nicht jede Zelle und kein Prozentsatz. Ein
/// Schreibtisch am Kopfende eines Betts blockiert einen Teil dieser Laengsseite, ohne das Bett
/// unerreichbar zu machen; was den Zugang entscheidet, ist ein durchgehender Streifen, der
/// lang genug zum Stehen ist. Die Forderung "ganzer Streifen frei" meldete ein voellig
/// benutzbares Bett als zugangslos.
pub fn free_depth_on_side(grid: &Grid, r: &Rect, side: Side, want: i32) -> i32 {
    let step = RES;
    let mut d = step;
    while d <= want {
        let strip = match side {
            Side::North => Rect {
                x: r.x,
                y: r.y - d,
                w: r.w,
                d: step,
            },
            Side::South => Rect {
                x: r.x,
                y: r.bottom() + d - step,
                w: r.w,
                d: step,
            },
            Side::West => Rect {
                x: r.x - d,
                y: r.y,
                w: step,
                d: r.d,
            },
            Side::East => Rect {
                x: r.right() + d - step,
                y: r.y,
                w: step,
                d: r.d,
            },
        };
        let along_x = matches!(side, Side::North | Side::South);
        if longest_free_run(grid, &strip, along_x) < STANDING_RUN_CM {
            return d - step;
        }
        d += step;
    }
    want
}

/// Laengste ununterbrochene Strecke stehbarer Zellen entlang eines Streifens, in cm.
pub fn longest_free_run(grid: &Grid, s: &Rect, along_x: bool) -> i32 {
    let floor_div = |v: i32| (v as f64 / RES as f64).floor() as i32;
    let ceil_div = |v: i32| (v as f64 / RES as f64).ceil() as i32;
    let c0 = floor_div(s.x);
    let c1 = ceil_div(s.right()) - 1;
    let r0 = floor_div(s.y);
    let r1 = ceil_div(s.bottom()) - 1;
    let (lo, hi) = if along_x { (c0, c1) } else { (r0, r1) };
    let mut best = 0;
    let mut run = 0;
    for i in lo..=hi {
        let (c, r) = if along_x { (i, r0) } else { (c0, i) };
        let ok = c >= 0
            && r >= 0
            && c < grid.cols as i32
            && r < grid.rows as i32
            && grid.free[grid.idx(c as usize, r as usize)] == 1;
        run = if ok { run + RES } else { 0 };
        if run > best {
            best = run;
        }
    }
    best
}

pub fn overlap_area(a: &Rect, b: &Rect) -> i32 {
    let w = (a.right().min(b.right()) - a.x.max(b.x)).max(0);
    let h = (a.bottom().min(b.bottom()) - a.y.max(b.y)).max(0);
    w * h
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    North,
    South,
    East,
    West,
}

pub const SIDES: [Side; 4] = [Side::North, Side::South, Side::East, Side::West];
