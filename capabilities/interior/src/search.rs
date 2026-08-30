//! Positionssuche: haelt ein Layout fest und rastert einzelne Moebel durch, bis eine
//! Kombination ohne harten Verstoss dasteht.
//!
//! Warum das ein Modul ist und kein Skript: die Frage "wohin passt X noch" kam am 2026-08-30
//! viermal, und viermal wurde dafuer ein Wegwerf-Skript geschrieben, ausgefuehrt und geloescht.
//! Jedes hatte leicht andere Annahmen darueber, was eine Sperrflaeche ist, und das Ergebnis des
//! letzten ging beim Sessionende verloren.
//!
//! Zwei Dinge machen die Suche praktikabel:
//!   * Ein billiger Vorfilter vor der vollen Pruefung. Rechteck-Ueberlappung kostet
//!     Nanosekunden, eine Raeumungspruefung 0,18 ms — der Filter entscheidet ueber Sekunden
//!     gegen Minuten.
//!   * Parallelitaet ueber die Kandidaten. Sie sind voneinander unabhaengig; das ist der
//!     Grund, warum hier rayon steht und keine DataFrame-Bibliothek.

use crate::clearance::{check_layout, CheckResult};
use crate::geometry::point_in_polygon;
use crate::model::{footprint, Layout, Model, ModelError, PlacedItem, Rect};
use rayon::prelude::*;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct Spec {
    /// Ids, die bewegt werden duerfen. Alles andere im Layout bleibt, wo es steht.
    pub move_refs: Vec<String>,
    /// Rasterschritt in cm. Grob suchen, dann verfeinern.
    pub step: i32,
    /// Optional je Id: erlaubtes Band [x0, x1, y0, y1].
    pub bands: BTreeMap<String, [i32; 4]>,
    /// Abbruch nach so vielen Treffern. 0 = alle.
    pub limit: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Hit {
    pub places: BTreeMap<String, [i32; 2]>,
    pub soft: usize,
    /// Engste der geprueften Routen in cm — hoeher ist besser.
    pub bottleneck_cm: i32,
    /// Wie viele Zentimeter der bewegten Moebel an einer Wand anliegen.
    ///
    /// Der Raeumungspruefer kennt nur Abstaende, nicht Sinn: er haelt einen Esstisch mitten
    /// im Raum fuer genauso richtig wie einen an der Wand, solange die Laufwege stimmen. Die
    /// ersten Suchlaeufe am 2026-08-30 lieferten deshalb lauter formal fehlerfreie Layouts,
    /// die niemand so einrichten wuerde. Diese Zahl ist die fehlende Haelfte des Urteils.
    pub wandkontakt_cm: i32,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchReport {
    pub base: String,
    pub moved: Vec<String>,
    pub step: i32,
    pub candidates_after_filter: usize,
    pub fully_checked: usize,
    pub hits: Vec<Hit>,
    pub elapsed_ms: u128,
}

/// Zentimeter der Rechteckkanten, die unmittelbar an einer Wand liegen.
///
/// Geprueft wird knapp ausserhalb der Kante: liegt der Punkt nicht mehr im Wohnpolygon, ist
/// dort eine Wand. Freistehende Moebel bekommen 0 und sinken damit in der Rangfolge — nicht
/// weil sie regelwidrig waeren, sondern weil sie schlechter sind.
fn wandkontakt_cm(model: &Model, r: &Rect) -> i32 {
    let poly = &model.room.hauptraum.polygon;
    let outside = |x: f64, y: f64| !point_in_polygon([x, y], poly);
    let probe = 4.0;
    let step = 10;
    let mut n = 0;
    let mut x = r.x + 5;
    while x < r.right() - 5 {
        if outside(x as f64, r.y as f64 - probe) {
            n += step;
        }
        if outside(x as f64, r.bottom() as f64 + probe) {
            n += step;
        }
        x += step;
    }
    let mut y = r.y + 5;
    while y < r.bottom() - 5 {
        if outside(r.x as f64 - probe, y as f64) {
            n += step;
        }
        if outside(r.right() as f64 + probe, y as f64) {
            n += step;
        }
        y += step;
    }
    n
}

fn inside_room(model: &Model, r: &Rect) -> bool {
    let poly = &model.room.hauptraum.polygon;
    let xs = [r.x + 1, r.x + r.w / 2, r.right() - 1];
    let ys = [r.y + 1, r.y + r.d / 2, r.bottom() - 1];
    xs.iter().all(|x| {
        ys.iter()
            .all(|y| point_in_polygon([*x as f64, *y as f64], poly))
    })
}

/// Sperrflaechen aus dem Modell, nicht aus dem Kopf. Die Wegwerf-Skripte haben genau diese
/// Rechtecke jedes Mal neu abgetippt — hier stammen sie aus room.toml und rules.toml.
fn blockers(model: &Model, fixed: &[&PlacedItem]) -> Result<Vec<Rect>, ModelError> {
    let mut out = Vec::new();
    for it in fixed {
        let (w, d, _) = footprint(it, &model.catalogue)?;
        out.push(Rect {
            x: it.x,
            y: it.y,
            w,
            d,
        });
    }
    for o in &model.room.oeffnungen {
        if let Some(sp) = o.sperrflaeche {
            out.push(sp.rect());
        }
    }
    Ok(out)
}

pub fn search(model: &Model, base: &Layout, spec: &Spec) -> Result<SearchReport, ModelError> {
    let t0 = std::time::Instant::now();

    let fixed: Vec<&PlacedItem> = base
        .items
        .iter()
        .filter(|i| !spec.move_refs.contains(&i.reference))
        .collect();
    let movers: Vec<&PlacedItem> = spec
        .move_refs
        .iter()
        .map(|r| {
            base.items
                .iter()
                .find(|i| &i.reference == r)
                .ok_or_else(|| {
                    ModelError::Missing(format!(
                        "`{r}` steht nicht in \"{}\" — nichts zu bewegen",
                        base.name
                    ))
                })
        })
        .collect::<Result<_, _>>()?;

    let block = blockers(model, &fixed)?;

    // Alle geometrisch zulaessigen Kombinationen sammeln, bevor irgendetwas geprueft wird.
    let mut combos: Vec<Vec<(String, [i32; 2], Rect)>> = vec![Vec::new()];
    for m in &movers {
        let (w, d, _) = footprint(m, &model.catalogue)?;
        // Ohne Band die Ausdehnung des Wohnpolygons, NICHT eine gemerkte Zahl. Hier stand
        // [0, 420, 0, 590] — die Masse dieser einen Wohnung, fest im Code einer Bibliothek,
        // die keine Wohnung kennen darf.
        let b = spec.bands.get(&m.reference).copied().unwrap_or_else(|| {
            let poly = &model.room.hauptraum.polygon;
            [
                poly.iter().map(|q| q[0]).min().unwrap_or(0),
                poly.iter().map(|q| q[0]).max().unwrap_or(0),
                poly.iter().map(|q| q[1]).min().unwrap_or(0),
                poly.iter().map(|q| q[1]).max().unwrap_or(0),
            ]
        });
        let mut next = Vec::new();
        for prefix in &combos {
            let mut y = b[2];
            while y <= b[3] - d {
                let mut x = b[0];
                while x <= b[1] - w {
                    let r = Rect { x, y, w, d };
                    if inside_room(model, &r)
                        && !block.iter().any(|q| r.overlaps(q))
                        && !prefix.iter().any(|(_, _, pr)| r.overlaps(pr))
                    {
                        let mut c = prefix.clone();
                        c.push((m.reference.clone(), [x, y], r));
                        next.push(c);
                    }
                    x += spec.step;
                }
                y += spec.step;
            }
        }
        combos = next;
    }
    let candidates = combos.len();

    // Die volle Pruefung parallel. Jeder Kandidat ist unabhaengig; das Modell wird nur gelesen.
    let checked = std::sync::atomic::AtomicUsize::new(0);
    let mut hits: Vec<Hit> = combos
        .par_iter()
        .filter_map(|combo| {
            let mut items: Vec<PlacedItem> = fixed.iter().map(|i| (*i).clone()).collect();
            for (reference, pos, _) in combo {
                let src = movers.iter().find(|m| &m.reference == reference)?;
                items.push(PlacedItem {
                    x: pos[0],
                    y: pos[1],
                    ..(*src).clone()
                });
            }
            let l = Layout {
                name: format!("{} (Suche)", base.name),
                items,
                id: String::new(),
            };
            checked.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let r: CheckResult = check_layout(model, &l).ok()?;
            if !r.hard.is_empty() {
                return None;
            }
            let bottleneck = r
                .metrics
                .corridors
                .iter()
                .filter_map(|c| c.width_cm)
                .min()
                .unwrap_or(0);
            Some(Hit {
                places: combo.iter().map(|(k, p, _)| (k.clone(), *p)).collect(),
                soft: r.soft.len(),
                bottleneck_cm: bottleneck,
                wandkontakt_cm: combo
                    .iter()
                    .map(|(_, _, rect)| wandkontakt_cm(model, rect))
                    .sum(),
            })
        })
        .collect();

    // Rangfolge: erst keine Warnungen, dann moeglichst viel Wand im Ruecken, dann der breiteste
    // Engpass. Die mittlere Stufe ist die wichtigste — ohne sie gewinnen Layouts, die jede Regel
    // erfuellen und trotzdem aussehen wie ein Moebellager.
    hits.sort_by(|a, b| {
        a.soft
            .cmp(&b.soft)
            .then(b.wandkontakt_cm.cmp(&a.wandkontakt_cm))
            .then(b.bottleneck_cm.cmp(&a.bottleneck_cm))
    });
    if spec.limit > 0 {
        hits.truncate(spec.limit);
    }

    Ok(SearchReport {
        base: base.name.clone(),
        moved: spec.move_refs.clone(),
        step: spec.step,
        candidates_after_filter: candidates,
        fully_checked: checked.load(std::sync::atomic::Ordering::Relaxed),
        hits,
        elapsed_ms: t0.elapsed().as_millis(),
    })
}
