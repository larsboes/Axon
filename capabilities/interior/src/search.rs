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
use crate::geometry::{point_in_polygon, RES};
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

/// Sagt dieses Stueck selbst, dass es frei stehen soll?
///
/// Die Wandsumme belohnt eine Wand im Ruecken, und ein Raumtrenner hat per Definition keine.
/// Bis 2026-08-31 sank er deshalb in der Rangfolge — bestraft dafuer, dass er seine Aufgabe
/// erfuellt. Er zaehlt jetzt gar nicht mit, statt mit 0 cm zu zaehlen: ein Regal quer im Raum
/// soll die Bewertung der uebrigen Stuecke weder heben noch senken.
///
/// Freiwillig wie jedes Feld aus PRD Q61. Ein Stueck ohne die Angabe wird weiter an der Wand
/// gemessen, also aendert sich kein bestehender Rang, bis jemand eine Zeile fuellt.
fn ist_raumtrenner(model: &Model, reference: &str) -> bool {
    model
        .catalogue
        .get(reference)
        .and_then(|i| i.raumtrenner)
        .unwrap_or(false)
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
                    .filter(|(reference, _, _)| !ist_raumtrenner(model, reference))
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

/// Wo die linke obere Ecke eines Stuecks liegen DARF, als Lauflaengen je Rasterzeile.
///
/// Die Frage, die eine Oberflaeche beim Ziehen stellt: harte Kanten, damit ein Moebel gar nicht
/// erst durch eine Wand geschoben werden kann, statt es hinterher als Verstoss zu melden.
///
/// **Warum das hier steht und nicht im Browser.** Der Hauptraum ist ein Sechseck, kein Rechteck
/// — an einer Ecke liegt das Bad. Auf ein umschliessendes Rechteck zu klemmen wuerde ein Bett in
/// der Kerbe abstellen, die nicht zum Raum gehoert. Richtig klemmen heisst `point_in_polygon`,
/// und eine zweite Fassung davon in JavaScript waere die Doppelung, gegen die diese Capability
/// existiert. Die Oberflaeche bekommt deshalb eine LISTE und rechnet nichts.
///
/// Hart sind Waende und was sich nicht bewegen laesst: feste Einbauten und Tuerschwenkbereiche,
/// dieselben `blockers`, gegen die auch die Suche prueft. **Andere Moebel sind NICHT hart** —
/// beim Umstellen muss man ein Stueck an einem anderen vorbeifuehren koennen, und eine
/// Ueberlappung meldet `kollision` ohnehin sichtbar. Eine Kante, die das Umstellen unmoeglich
/// macht, waere schlimmer als der Verstoss, den sie verhindert.
pub fn allowed_positions(
    model: &Model,
    base: &Layout,
    reference: &str,
    rot: i32,
) -> Result<AllowedPositions, ModelError> {
    let it = base
        .items
        .iter()
        .find(|i| i.reference == reference)
        .ok_or_else(|| {
            ModelError::Missing(format!("`{reference}` steht nicht in \"{}\"", base.name))
        })?;
    // Die Grundflaeche fuer die GEFRAGTE Drehung, nicht fuer die aktuelle.
    let gedreht = PlacedItem { rot, ..it.clone() };
    let (w, d, _) = footprint(&gedreht, &model.catalogue)?;

    // Was sich NICHT bewegen laesst: feste Einbauten und Tuerschwenkbereiche. Beides steht in
    // room.toml, beides wird hier gelesen statt abgetippt.
    let mut sperr: Vec<Rect> = model.room.fix_moebel.iter().map(|f| f.rect()).collect();
    sperr.extend(
        model
            .room
            .oeffnungen
            .iter()
            .filter_map(|o| o.sperrflaeche.map(|z| z.rect())),
    );

    let poly = &model.room.hauptraum.polygon;
    let (mut min_x, mut min_y) = (i32::MAX, i32::MAX);
    let (mut max_x, mut max_y) = (i32::MIN, i32::MIN);
    for p in poly {
        min_x = min_x.min(p[0]);
        min_y = min_y.min(p[1]);
        max_x = max_x.max(p[0]);
        max_y = max_y.max(p[1]);
    }

    let mut rows = Vec::new();
    let mut y = min_y;
    while y + d <= max_y {
        let mut runs: Vec<[i32; 2]> = Vec::new();
        let mut lauf: Option<i32> = None;
        let mut x = min_x;
        while x + w <= max_x {
            let r = Rect { x, y, w, d };
            let frei = inside_room(model, &r) && !sperr.iter().any(|b| r.overlaps(b));
            match (frei, lauf) {
                (true, None) => lauf = Some(x),
                (false, Some(start)) => {
                    runs.push([start, x - RES]);
                    lauf = None;
                }
                _ => {}
            }
            x += RES;
        }
        if let Some(start) = lauf {
            runs.push([start, x - RES]);
        }
        if !runs.is_empty() {
            rows.push(AllowedRow { y, x: runs });
        }
        y += RES;
    }

    Ok(AllowedPositions {
        reference: reference.to_string(),
        rot,
        w,
        d,
        step: RES,
        rows,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct AllowedRow {
    pub y: i32,
    /// Erlaubte x-Bereiche, jeweils [von, bis] einschliesslich.
    pub x: Vec<[i32; 2]>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AllowedPositions {
    pub reference: String,
    pub rot: i32,
    pub w: i32,
    pub d: i32,
    pub step: i32,
    pub rows: Vec<AllowedRow>,
}
