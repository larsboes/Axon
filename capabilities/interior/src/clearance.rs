//! Beurteilt ein Layout gegen `rules.toml`.
//!
//! Jede Schwelle kommt aus dem Modell. Nichts in dieser Datei entscheidet, wie viel Platz ein
//! Laufweg braucht — sie entscheidet nur, ob das Layout ihn hat.
//!
//! Schwere folgt `rules.toml`: `hart` blockiert, `weich` warnt. Ein Layout besteht nur, wenn
//! keine harte Regel verletzt ist. Es gibt absichtlich keinen Weg, ein Bestehen zu melden,
//! solange ein harter Verstoss offen ist — ein Pruefer, der weich durchwinkt, ist schlechter
//! als gar keiner.
//!
//! Der Satz oben stimmt seit 2026-08-31, und vorher war er eine Behauptung ueber Code, den
//! niemand nachgelesen hatte: die Schwere stand an allen 21 Ausgabestellen als Literal, und
//! `Regel { id, schwere, text }` wurde aus `rules.toml` geparst und von **nichts** gelesen.
//! Eine Wohnung konnte R3 auf `hart` setzen und bekam weiterhin eine Warnung. In genau der
//! Capability, die es gegen zwei Fassungen derselben Zahl gibt.
//!
//! ## Zwei Klassen von Kennungen, und die Grenze dazwischen
//!
//! `REGEL_IDS` sind **Hausregeln**: die Wohnung deklariert sie in `rules.toml` mit Schwere und
//! Text, und dieser Pruefer schlaegt beides ueber `rules.regel(id)` nach. Fehlt die Kennung
//! dort, ist das ein Fehler und kein textloser Verstoss.
//!
//! Die uebrigen Kennungen — `kollision`, `raumgrenze`, `zugang`, `laufweg` und die anderen —
//! sind **keine Hausregeln**, sondern Geometrie und Nutzbarkeit. Zwei Moebel koennen sich
//! nicht ueberlappen, und keine Wohnung kann das erlauben. Sie deklarieren zu lassen hiesse,
//! jede Wohnung eine Invariante wiederholen zu lassen, die sie nicht aendern kann, und die
//! erste vergessene waere eine still abgeschaltete Pruefung.

use crate::geometry::{
    clearance_field, free_depth_on_side, overlap_area, point_in_polygon, widest_path, Grid, Side,
    RES, SIDES,
};
use crate::model::{
    footprint, Catalogue, Layout, Model, ModelError, Opening, PlacedItem, Pt, Rect, Room, Rules,
    Seite,
};
use crate::store::Item;
use serde::Serialize;

pub use crate::model::Severity;

/// Die Hausregeln, die diese Maschine prueft.
///
/// Der Abgleich laeuft in beide Richtungen und beide sind noetig. Was hier steht, MUSS die
/// Wohnung deklarieren, sonst bricht die Pruefung ab — eine Kennung ohne Text ist ein Verstoss,
/// den niemand nachschlagen kann. Was die Wohnung deklariert und hier fehlt, meldet
/// `CheckResult::nicht_geprueft` als Luecke, statt es fallen zu lassen.
pub const REGEL_IDS: &[&str] = &["R1", "R2", "R3", "R4", "R5", "R6", "R7", "R8"];

#[derive(Debug, Clone, Serialize)]
pub struct Violation {
    pub rule: String,
    pub severity: Severity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
    pub message: String,
    /// Der Regeltext aus `rules.toml`, wo es einen gibt.
    ///
    /// Nur Hausregeln haben einen. `message` sagt, was dieses Layout falsch macht; `text` sagt,
    /// welche Regel das ueberhaupt zu einer Regel macht — und der stand bis 2026-08-31 in einer
    /// Datei, die kein Bericht las.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub measured: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<i32>,
}

/// Ein Verstoss gegen eine deklarierte Hausregel: Schwere und Text kommen aus der Wohnung.
fn haus_regel(
    rules: &Rules,
    id: &str,
    item: Option<String>,
    message: String,
    measured: Option<i32>,
    required: Option<i32>,
) -> Result<Violation, ModelError> {
    let regel = rules.regel(id)?;
    Ok(Violation {
        rule: regel.id.clone(),
        severity: regel.severity()?,
        item,
        message,
        text: Some(regel.text.clone()),
        measured,
        required,
    })
}

/// Legt einen Verstoss in `hart` oder `weich`, je nach seiner eigenen Schwere.
///
/// Die Sortierung folgt jetzt der Datei, also darf sie nicht mehr an der Aufrufstelle stehen:
/// `hard.push(..)` mit einem weichen Verstoss darin waere genau die Divergenz, die diese
/// Aenderung schliesst.
fn einsortieren(v: Violation, hard: &mut Vec<Violation>, soft: &mut Vec<Violation>) {
    match v.severity {
        Severity::Hart => hard.push(v),
        Severity::Weich => soft.push(v),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Corridor {
    pub from: String,
    pub to: String,
    pub width_cm: Option<i32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Uncertainty {
    pub reference: String,
    pub label: String,
    pub fields: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Metrics {
    pub room_area_m2: f64,
    pub occupied_area_m2: f64,
    pub free_area_m2: f64,
    pub corridors: Vec<Corridor>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckResult {
    pub layout: String,
    pub pass: bool,
    pub hard: Vec<Violation>,
    pub soft: Vec<Violation>,
    pub uncertainties: Vec<Uncertainty>,
    pub metrics: Metrics,
    /// Stuecke in diesem Layout, gegen die schon entschieden ist.
    ///
    /// Abgeleitet aus `prioritaet = "verworfen"` im Inventar, nicht am Layout deklariert: die
    /// Entscheidung faellt am Moebel und soll nicht in jeder Datei wiederholt werden, die es
    /// aufstellt. Zwei Layouts stellten am 2026-08-31 einen Tisch auf, der seit dem 30. verworfen
    /// war — eines davon war eines der vier durchgefallenen, und es fiel an genau diesem Tisch.
    /// Ein Verdikt ueber Moebel, die nicht kommen, beantwortet keine Frage, die noch offen ist.
    pub veraltet: Vec<String>,
    /// Regeln, die diese Wohnung deklariert und diese Maschine nicht prueft.
    ///
    /// Gehoert in das Ergebnis und nicht in ein Log, weil es das Verdikt qualifiziert:
    /// „bestanden" heisst ab hier „bestanden, gemessen an den Regeln, die gemessen wurden".
    /// Die reale Wohnung fuehrt zwei davon (R5, R6), und vor 2026-08-31 sagte das nichts.
    pub nicht_geprueft: Vec<UngeprueteRegel>,
}

/// Eine Regel, die auf dieses Layout nicht angewendet wurde, und warum nicht.
///
/// Zwei Gruende, gleiche Folge. **Nicht implementiert:** die Wohnung fuehrt die Regel, diese
/// Maschine kennt sie nicht. **Nicht anwendbar:** die Maschine kennt sie, aber es fehlt eine
/// Angabe, ohne die sie nichts messen kann.
///
/// Der zweite Fall ist der gefaehrlichere, weil er wie Bestehen aussieht. `kleiderschrank_bestand`
/// stand bis 2026-08-31 ohne gemessene Hoehe in drei Layouts im Lichtkorridor; R3 begrenzt dort
/// auf 140 cm, und `if let Some(h)` uebersprang das Stueck **stillschweigend**. Zwei Layouts
/// bestanden auf einer Regel, die fuer das entscheidende Moebel nie gelaufen war.
#[derive(Debug, Clone, Serialize)]
pub struct UngeprueteRegel {
    pub rule: String,
    pub text: String,
    /// Im Klartext, was fehlt. Wandert in den Bericht, damit die Luecke benannt ist.
    pub grund: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Bed,
    Desk,
    Couch,
    Wardrobe,
    CoffeeTable,
    Table,
    Shelf,
    Other,
}

/// Zwei Reihenfolgen sind hier wesentlich, beide teuer gelernt: `couch` darf `couchtisch` nicht
/// fangen — sonst gilt ein Couchtisch als Sofa —, und `coffee_table` muss vor `table` stehen,
/// weil ein Couchtisch 40 cm zum Sofa will und nicht 80 cm Stuhlausziehzone.
pub fn kind_of(p: &PlacedItem) -> Kind {
    if let Some(k) = p.kind.as_deref() {
        return match k {
            "bed" => Kind::Bed,
            "desk" => Kind::Desk,
            "couch" => Kind::Couch,
            "wardrobe" => Kind::Wardrobe,
            "coffee_table" => Kind::CoffeeTable,
            "table" => Kind::Table,
            "shelf" => Kind::Shelf,
            _ => Kind::Other,
        };
    }
    let r = p.reference.as_str();
    if r.starts_with("bett") {
        Kind::Bed
    } else if r.starts_with("schreibtisch") || r.starts_with("buerostuhl") {
        Kind::Desk
    } else if r.starts_with("couchtisch") || r.starts_with("beistelltisch") {
        Kind::CoffeeTable
    } else if r.starts_with("couch") || r.starts_with("sofa") {
        Kind::Couch
    } else if r.starts_with("kleiderschrank") || r.starts_with("schrank") {
        Kind::Wardrobe
    } else if r.starts_with("esstisch") || r.starts_with("klapptisch") {
        Kind::Table
    } else if r.starts_with("kallax")
        || r.contains("regal")
        || r.starts_with("lowboard")
        || r.starts_with("raumtrenner")
    {
        Kind::Shelf
    } else {
        Kind::Other
    }
}

struct Placed<'a> {
    it: &'a PlacedItem,
    kind: Kind,
    /// Der Katalogeintrag, falls es einen gibt. Ein Layout darf ein Stueck nennen, das seine
    /// Masse nur ueber `size:` mitbringt und im Inventar fehlt — dann gibt es nichts zu
    /// deklarieren, und der Name entscheidet.
    item: Option<&'a Item>,
    w: i32,
    d: i32,
    h: Option<i32>,
    rect: Rect,
}

struct Segment {
    a: Pt,
    b: Pt,
    normal: [f64; 2],
}

/// Loest eine Oeffnung in eine Strecke in Raumkoordinaten auf, samt der Normalen, die nach
/// INNEN zeigt. Die Normale wird probiert, nicht angenommen: `west` und `sued_hauptraum`
/// laufen entgegen der Kantenrichtung des Polygons.
fn opening_segment(room: &Room, o: &Opening) -> Option<Segment> {
    let (a, b) = room.opening_span(o)?;
    let len = (((b[0] - a[0]).pow(2) + (b[1] - a[1]).pow(2)) as f64)
        .sqrt()
        .max(1.0);
    let ux = (b[0] - a[0]) as f64 / len;
    let uy = (b[1] - a[1]) as f64 / len;
    let mid = [(a[0] + b[0]) as f64 / 2.0, (a[1] + b[1]) as f64 / 2.0];
    for n in [[-uy, ux], [uy, -ux]] {
        let probe = [mid[0] + n[0] * 20.0, mid[1] + n[1] * 20.0];
        if point_in_polygon(probe, &room.hauptraum.polygon) {
            return Some(Segment { a, b, normal: n });
        }
    }
    Some(Segment {
        a,
        b,
        normal: [0.0, 0.0],
    })
}

/// Die Rasterseite zu einer Modellseite. Zwei Aufzaehlungen fuer dieselben vier Richtungen:
/// `Seite` beschreibt das Modell und ist deutsch wie die Dateien, `Side` gehoert der Geometrie.
fn als_side(s: Seite) -> Side {
    match s {
        Seite::Nord => Side::North,
        Seite::Sued => Side::South,
        Seite::Ost => Side::East,
        Seite::West => Side::West,
    }
}

/// Was ein Stueck an Platz verlangt, wenn es es selbst sagt (PRD Q61 / B26).
///
/// Gibt `false` zurueck, wenn das Stueck gar nichts erklaert — dann entscheidet weiter der
/// Name, und der Aufrufer faellt auf `kind_of` zurueck. Das ist Absicht: 42 Zeilen an einem Tag
/// umzustellen waere ein Stichtag, an dem sich Verdikte aendern, ohne dass jemand die Zahlen
/// dahinter geprueft hat.
///
/// Die drei Pruefungen, und was jede von der Namensfassung uebernimmt:
///
/// * `open_clear` — der Platz vor Tueren und Schubladen. `wall_ok = false` bindet ihn an die
///   Seite, die `opens` nennt; sonst reicht die beste, weil ein Stueck gedreht werden kann.
/// * `access_sides` / `access_clear` — wie viele Seiten begehbar sein muessen und wie tief.
/// * `expands` — der zweite Zustand eines Klappmoebels. `to` ist die Gesamttiefe ausgeklappt,
///   also ist der zusaetzlich noetige Platz `to` minus der Tiefe in dieser Richtung.
fn eigene_ansprueche(p: &Placed, grid: &Grid, hard: &mut Vec<Violation>) -> bool {
    let Some(it) = p.item else { return false };
    let deklariert =
        it.open_clear.is_some() || it.access_sides.is_some() || it.expands_to.is_some();
    if !deklariert {
        return false;
    }
    let name = p.it.reference.as_str();

    if let Some(want) = it.open_clear {
        let seite = it.opens.map(|s| s.gedreht(p.it.rot));
        let bindend = it.wall_ok == Some(false);
        let (gemessen, wo) = match (seite, bindend) {
            (Some(s), true) => (
                free_depth_on_side(grid, &p.rect, als_side(s), want),
                s.as_str().to_string(),
            ),
            _ => (
                SIDES
                    .iter()
                    .map(|s| free_depth_on_side(grid, &p.rect, *s, want))
                    .max()
                    .unwrap_or(0),
                "seiner besten Seite".to_string(),
            ),
        };
        if gemessen < want {
            hard.push(Violation {
                rule: "oeffnen".into(),
                severity: Severity::Hart,
                item: Some(name.into()),
                message: format!(
                    "\"{name}\" hat {gemessen} cm zum Oeffnen an {wo}, braucht {want}"
                ),
                text: None,
                measured: Some(gemessen),
                required: Some(want),
            });
        }
    }

    if let Some(n) = it.access_sides {
        let want = it.access_clear.unwrap_or(0);
        let genug = SIDES
            .iter()
            .filter(|s| free_depth_on_side(grid, &p.rect, **s, want) >= want)
            .count() as i32;
        if genug < n {
            hard.push(Violation {
                rule: "zugang".into(),
                severity: Severity::Hart,
                item: Some(name.into()),
                message: format!(
                    "\"{name}\" ist von {genug} Seiten mit {want} cm erreichbar, braucht {n}"
                ),
                text: None,
                measured: Some(genug),
                required: Some(n),
            });
        }
    }

    if let (Some(dir), Some(to)) = (it.expands_dir, it.expands_to) {
        let seite = dir.gedreht(p.it.rot);
        let tiefe = match seite {
            Seite::Nord | Seite::Sued => p.d,
            Seite::Ost | Seite::West => p.w,
        };
        let zusatz = to - tiefe;
        if zusatz > 0 {
            let frei = free_depth_on_side(grid, &p.rect, als_side(seite), zusatz);
            if frei < zusatz {
                hard.push(Violation {
                    rule: "ausklappen".into(),
                    severity: Severity::Hart,
                    item: Some(name.into()),
                    message: format!(
                        "\"{name}\" hat {frei} cm nach {} zum Ausklappen, braucht {zusatz} (ausgeklappt {to} tief)",
                        seite.as_str()
                    ),
                    text: None,
                    measured: Some(frei),
                    required: Some(zusatz),
                });
            }
        }
    }
    true
}

/// Wie weit ein Wegpunkt vor der Sache liegt, zu der er gehoert. Ein Mensch steht VOR einer Tuer
/// und nicht in ihrem Rahmen; wird das kleiner, wird der Rahmen selbst zur Engstelle jeder Route.
/// Eine Zahl, an einer Stelle, fuer Oeffnungen und feste Einbauten gleichermassen.
const WEGPUNKT_ABSTAND: i32 = 45;

/// Die Flaeche, die neben einem festen Einbau frei bleiben muss, auf der Seite, die er nennt.
fn anlauf_rect(r: &Rect, seite: Seite, tiefe: i32) -> Rect {
    match seite {
        Seite::Nord => Rect {
            x: r.x,
            y: r.y - tiefe,
            w: r.w,
            d: tiefe,
        },
        Seite::Sued => Rect {
            x: r.x,
            y: r.bottom(),
            w: r.w,
            d: tiefe,
        },
        Seite::West => Rect {
            x: r.x - tiefe,
            y: r.y,
            w: tiefe,
            d: r.d,
        },
        Seite::Ost => Rect {
            x: r.right(),
            y: r.y,
            w: tiefe,
            d: r.d,
        },
    }
}

/// Wo ein Mensch steht, wenn er diesen Einbau benutzt.
fn anlauf_punkt(r: &Rect, seite: Seite) -> Pt {
    match seite {
        Seite::Nord => [r.x + r.w / 2, r.y - WEGPUNKT_ABSTAND],
        Seite::Sued => [r.x + r.w / 2, r.bottom() + WEGPUNKT_ABSTAND],
        Seite::West => [r.x - WEGPUNKT_ABSTAND, r.y + r.d / 2],
        Seite::Ost => [r.right() + WEGPUNKT_ABSTAND, r.y + r.d / 2],
    }
}

/// Das Rechteck, das ein Mensch vor einer Oeffnung braucht, aus ihrer eigenen Freihaltezone.
fn approach_rect(seg: &Segment, depth: i32) -> Rect {
    let d = depth as f64;
    let xs = [
        seg.a[0] as f64,
        seg.b[0] as f64,
        seg.a[0] as f64 + seg.normal[0] * d,
        seg.b[0] as f64 + seg.normal[0] * d,
    ];
    let ys = [
        seg.a[1] as f64,
        seg.b[1] as f64,
        seg.a[1] as f64 + seg.normal[1] * d,
        seg.b[1] as f64 + seg.normal[1] * d,
    ];
    let x = xs.iter().cloned().fold(f64::INFINITY, f64::min);
    let y = ys.iter().cloned().fold(f64::INFINITY, f64::min);
    let x1 = xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let y1 = ys.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    Rect {
        x: x as i32,
        y: y as i32,
        w: (x1 - x) as i32,
        d: (y1 - y) as i32,
    }
}

pub fn check_layout(
    model: &Model,
    layout: &Layout,
) -> Result<CheckResult, crate::model::ModelError> {
    let room = &model.room;
    let rules = &model.rules;
    let cat: &Catalogue = &model.catalogue;
    let mut hard: Vec<Violation> = Vec::new();
    let mut soft: Vec<Violation> = Vec::new();
    // Regeln, die laufen wollten und nicht konnten. Wird unten mit den gar nicht
    // implementierten zusammengelegt — fuer den Leser ist beides dasselbe: nicht gemessen.
    let mut ausgefallen: Vec<(String, String)> = Vec::new();

    let mut placed: Vec<Placed> = Vec::new();
    for it in &layout.items {
        let (w, d, h) = footprint(it, cat)?;
        placed.push(Placed {
            it,
            kind: kind_of(it),
            item: cat.get(&it.reference),
            w,
            d,
            h,
            rect: Rect {
                x: it.x,
                y: it.y,
                w,
                d,
            },
        });
    }

    // --- der Raum selbst -----------------------------------------------------
    let mut grid = Grid::new(&room.hauptraum.polygon);
    for f in &room.fix_moebel {
        grid.occupy(&Rect {
            x: f.x[0],
            y: f.y[0],
            w: f.x[1] - f.x[0],
            d: f.y[1] - f.y[0],
        });
    }
    // Tuerschwenk-Sperrflaechen werden hier absichtlich NICHT blockiert. Sie schraenken ein, wo
    // Moebel stehen duerfen, nicht wo ein Mensch geht — durch den Schwenkbereich der eigenen
    // Wohnungstuer laeuft man taeglich. Blockiert man sie, wird der Schwenkbogen selbst zur
    // Engstelle jeder Route und meldet 50 cm, egal wo die Moebel stehen. R2 prueft sie unten
    // direkt gegen die Moebelgrundflaechen — so, wie rules.toml sie formuliert.
    for p in &placed {
        grid.occupy(&p.rect);
    }

    // --- Raumgrenze und gegenseitige Ueberlappung ---------------------------
    for p in &placed {
        let corners = [
            [p.rect.x + 1, p.rect.y + 1],
            [p.rect.right() - 1, p.rect.y + 1],
            [p.rect.x + 1, p.rect.bottom() - 1],
            [p.rect.right() - 1, p.rect.bottom() - 1],
        ];
        if !corners
            .iter()
            .all(|c| point_in_polygon([c[0] as f64, c[1] as f64], &room.hauptraum.polygon))
        {
            hard.push(Violation {
                rule: "raumgrenze".into(),
                severity: Severity::Hart,
                item: Some(p.it.reference.clone()),
                message: format!(
                    "\"{}\" ragt aus dem Raum bei {},{} ({}×{} cm)",
                    p.it.reference, p.it.x, p.it.y, p.w, p.d
                ),
                text: None,
                measured: None,
                required: None,
            });
        }
    }
    for i in 0..placed.len() {
        for j in (i + 1)..placed.len() {
            let (a, b) = (&placed[i], &placed[j]);
            if a.rect.overlaps(&b.rect) {
                hard.push(Violation {
                    rule: "kollision".into(),
                    severity: Severity::Hart,
                    item: Some(format!("{} / {}", a.it.reference, b.it.reference)),
                    message: format!(
                        "\"{}\" und \"{}\" ueberlappen sich um {} dm²",
                        a.it.reference,
                        b.it.reference,
                        (overlap_area(&a.rect, &b.rect) as f64 / 100.0).round() as i32
                    ),
                    text: None,
                    measured: None,
                    required: None,
                });
            }
        }
    }

    // --- R1 / R2 / R7: Anlauf- und Sperrzonen -------------------------------
    for o in &room.oeffnungen {
        let Some(depth) = o.freihaltezone else {
            continue;
        };
        let Some(seg) = opening_segment(room, o) else {
            continue;
        };
        let zone = approach_rect(&seg, depth);
        for p in &placed {
            if p.rect.overlaps(&zone) {
                einsortieren(
                    haus_regel(
                        rules,
                        "R1",
                        Some(p.it.reference.clone()),
                        format!(
                            "\"{}\" ragt in die {} cm Anlaufzone von {}",
                            p.it.reference, depth, o.id
                        ),
                        None,
                        Some(depth),
                    )?,
                    &mut hard,
                    &mut soft,
                );
            }
        }
    }
    for o in &room.oeffnungen {
        let Some(sp) = o.sperrflaeche else { continue };
        let zone = sp.rect();
        for p in &placed {
            if p.rect.overlaps(&zone) {
                einsortieren(
                    haus_regel(
                        rules,
                        "R2",
                        Some(p.it.reference.clone()),
                        format!(
                            "\"{}\" steht im Tuerschwenkbereich von {}",
                            p.it.reference, o.id
                        ),
                        None,
                        None,
                    )?,
                    &mut hard,
                    &mut soft,
                );
            }
        }
    }
    // R7 galt bis 2026-08-30 fuer genau ein Moebel, das der Code beim Namen kannte. Sie gilt
    // jetzt fuer jeden festen Einbau, der eine Anlaufzone deklariert — und die Schwelle schlaegt
    // er ueber ihren Namen in rules.toml nach, statt sie mitzubringen.
    for f in &room.fix_moebel {
        let Some(zone_decl) = &f.anlaufzone else {
            continue;
        };
        let depth = rules.abstand(&zone_decl.abstand)?;
        let zone = anlauf_rect(&f.rect(), zone_decl.seite, depth);
        for p in &placed {
            if p.rect.overlaps(&zone) {
                einsortieren(
                    haus_regel(
                        rules,
                        "R7",
                        Some(p.it.reference.clone()),
                        format!(
                            "\"{}\" blockiert die {} cm Anlaufzone von {}",
                            p.it.reference, depth, f.id
                        ),
                        None,
                        Some(depth),
                    )?,
                    &mut hard,
                    &mut soft,
                );
            }
        }
    }

    // --- R3: der Lichtkorridor ----------------------------------------------
    // Anders als in der TypeScript-Vorlage sind diese vier Zahlen KEINE Abschrift aus einem
    // Prosatext mehr: rules.toml fuehrt `[lichtkorridor]` strukturiert, also kann die Regel
    // nicht mehr still von ihrer eigenen Beschreibung abweichen.
    let lc = &rules.lichtkorridor;
    let corridor = Rect {
        x: lc.x[0],
        y: lc.y[0],
        w: lc.x[1] - lc.x[0],
        d: lc.y[1] - lc.y[0],
    };
    for p in &placed {
        // Ohne gemessene Hoehe kann R3 hier nichts sagen — und ein Stueck, das gar nicht im
        // Korridor steht, braucht auch keine: gemeldet wird nur, wo die Regel wirklich gegriffen
        // haette.
        if p.h.is_none() && p.rect.overlaps(&corridor) {
            ausgefallen.push((
                "R3".to_string(),
                format!(
                    "\"{}\" steht im Lichtkorridor und hat keine gemessene Hoehe",
                    p.it.reference
                ),
            ));
        }
        if let Some(h) = p.h {
            if h > lc.max_hoehe && p.rect.overlaps(&corridor) {
                einsortieren(
                    haus_regel(
                        rules,
                        "R3",
                        Some(p.it.reference.clone()),
                        format!(
                            "\"{}\" ist {} cm hoch im Lichtkorridor (max {}); dieser Raum hat eine Fensterwand",
                            p.it.reference, h, lc.max_hoehe
                        ),
                        Some(h),
                        Some(lc.max_hoehe),
                    )?,
                    &mut hard,
                    &mut soft,
                );
            }
        }
    }

    // --- R4: Bettkopf weg von der Verglasung --------------------------------
    if let Some(glaz) = room
        .oeffnungen
        .iter()
        .find(|o| o.typ.as_deref().is_some_and(|t| t.starts_with("glastuer")))
    {
        if let Some(seg) = opening_segment(room, glaz) {
            let band = approach_rect(&seg, 20);
            for p in placed.iter().filter(|p| p.kind == Kind::Bed) {
                if p.rect.overlaps(&band) {
                    einsortieren(
                        haus_regel(
                            rules,
                            "R4",
                            Some(p.it.reference.clone()),
                            "Bett beruehrt die Verglasung (Kaelte, Zug, keine Privatsphaere zur Terrasse)".into(),
                            None,
                            None,
                        )?,
                        &mut hard,
                        &mut soft,
                    );
                }
            }
        }
    }

    // --- R5: Fensterlicht faellt seitlich auf den Schreibtisch ---------------
    //
    // "Weder frontal zur Verglasung (Blendung) noch mit dem Ruecken dazu (Reflexionen)."
    // Beides sind Aussagen ueber die Achse, auf der der Schreibtisch zur Verglasung steht:
    // sitzt man an einer Tiefenseite, liegt vorne und hinten auf der Tiefenachse. Zeigt die
    // Verglasung dorthin, ist sie im Gesicht oder im Ruecken; liegt sie auf der Breitenachse,
    // faellt das Licht seitlich ein — genau das, was die Regel verlangt.
    //
    // Ohne Verglasung im Raum ist nichts zu messen, und das wird gemeldet statt uebersprungen.
    {
        let glas = room
            .oeffnungen
            .iter()
            .find(|o| o.typ.as_deref().is_some_and(|t| t.starts_with("glastuer")));
        let desks = placed.iter().filter(|p| p.kind == Kind::Desk).count();
        match glas.and_then(|g| opening_segment(room, g)) {
            None if desks > 0 => ausgefallen.push((
                "R5".to_string(),
                "der Raum fuehrt keine Verglasung, gegen die ein Schreibtisch stehen koennte"
                    .to_string(),
            )),
            Some(seg) => {
                // Aus welcher Richtung das Licht kommt, sagt die NORMALE der Verglasung, nicht
                // der Abstand zwischen zwei Mittelpunkten. Der erste Entwurf verglich
                // Mittelpunkte und meldete daraufhin in allen 13 Layouts einen Verstoss — bei
                // einem Schreibtisch, der in allen 13 an derselben Stelle steht und dessen
                // Fenster in der Seitenwand sitzt. Eine Regel, die ueberall feuert, misst
                // nichts.
                //
                // Die Verglasung liegt in einer Wand; ihre Normale zeigt in den Raum. Faellt das
                // Licht entlang der Achse ein, auf der der Schreibtisch vorne und hinten hat,
                // steht er frontal oder mit dem Ruecken dazu. Steht die Normale quer dazu,
                // kommt es seitlich — und genau das verlangt die Regel.
                let licht_laeuft_in_y = seg.normal[1].abs() > seg.normal[0].abs();
                for p in placed.iter().filter(|p| p.kind == Kind::Desk) {
                    // `p.rect` ist bereits gedreht, also entscheidet es selbst, welche Achse
                    // gerade die Tiefe ist: an der Tiefenseite sitzt man.
                    let tiefe_laeuft_in_y = p.rect.d <= p.rect.w;
                    if licht_laeuft_in_y == tiefe_laeuft_in_y {
                        einsortieren(
                            haus_regel(
                                rules,
                                "R5",
                                Some(p.it.reference.clone()),
                                format!(
                                    "\"{}\" steht mit Front oder Ruecken zur Verglasung — Blendung bzw. Reflexionen; das Licht soll seitlich einfallen",
                                    p.it.reference
                                ),
                                None,
                                None,
                            )?,
                            &mut hard,
                            &mut soft,
                        );
                    }
                }
            }
            _ => {}
        }
    }

    // --- R6: der Blick vom Eingang faellt nicht zuerst aufs Bett -------------
    //
    // Ein Strahl von der Eingangsoeffnung nach innen, entlang ihrer Normalen. Was er zuerst
    // trifft, ist das, was man beim Eintreten sieht. Ist das ein Bett, ist die Regel verletzt.
    //
    // **Welche Oeffnung der Eingang ist, sagt die Wohnung** (`eingang = true`), nicht der Code.
    // `badtuer` und `eingangstuer` sind beide `typ = "tuer"`, und die nach dem Namen zu
    // unterscheiden waere genau der Fehler, den B26a mit der Kuechen-Anlaufzone geschlossen hat.
    // Sagt die Wohnung nichts, laeuft R6 nicht und sagt das.
    {
        let betten = placed.iter().filter(|p| p.kind == Kind::Bed).count();
        let eingang = room.oeffnungen.iter().find(|o| o.eingang == Some(true));
        match eingang.and_then(|e| opening_segment(room, e)) {
            None if betten > 0 => ausgefallen.push((
                "R6".to_string(),
                "keine Oeffnung ist als Eingang deklariert (`eingang = true` in room.toml)"
                    .to_string(),
            )),
            Some(seg) => {
                let mut x = (seg.a[0] + seg.b[0]) as f64 / 2.0;
                let mut y = (seg.a[1] + seg.b[1]) as f64 / 2.0;
                let mut gesehen: Option<&Placed> = None;
                // In Schritten von 5 cm, hoechstens die Diagonale eines sehr grossen Raums.
                for _ in 0..400 {
                    x += seg.normal[0] * 5.0;
                    y += seg.normal[1] * 5.0;
                    if !point_in_polygon([x, y], &room.hauptraum.polygon) {
                        break;
                    }
                    if let Some(hit) = placed.iter().find(|p| {
                        x >= p.rect.x as f64
                            && x < p.rect.right() as f64
                            && y >= p.rect.y as f64
                            && y < p.rect.bottom() as f64
                    }) {
                        gesehen = Some(hit);
                        break;
                    }
                }
                if let Some(p) = gesehen {
                    if p.kind == Kind::Bed {
                        einsortieren(
                            haus_regel(
                                rules,
                                "R6",
                                Some(p.it.reference.clone()),
                                format!(
                                    "vom Eingang aus faellt der Blick zuerst auf \"{}\"",
                                    p.it.reference
                                ),
                                None,
                                None,
                            )?,
                            &mut hard,
                            &mut soft,
                        );
                    }
                }
            }
            _ => {}
        }
    }

    // --- Zugangsabstaende ----------------------------------------------------
    for p in &placed {
        // Sagt das Stueck selbst, was es braucht, gilt das und sonst nichts. Beides zu pruefen
        // hiesse, ein Moebel gegen zwei Regelsaetze zu messen und den strengeren gewinnen zu
        // lassen — dann waere die Deklaration keine Ersetzung, sondern eine Verschaerfung.
        if eigene_ansprueche(p, &grid, &mut hard) {
            continue;
        }
        match p.kind {
            Kind::Bed => {
                let want = rules.abstand("bett_zugang_laengsseite")?;
                let second = rules.abstand("bett_zugang_zweite_seite")?;
                let long = if p.w >= p.d {
                    [Side::North, Side::South]
                } else {
                    [Side::West, Side::East]
                };
                let mut depths: Vec<i32> = long
                    .iter()
                    .map(|s| free_depth_on_side(&grid, &p.rect, *s, want))
                    .collect();
                depths.sort_unstable_by(|a, b| b.cmp(a));
                if depths[0] < want {
                    hard.push(Violation {
                        rule: "bett_zugang".into(),
                        severity: Severity::Hart,
                        item: Some(p.it.reference.clone()),
                        message: format!(
                            "Bett hat {} cm an seiner besten Laengsseite, braucht {}",
                            depths[0], want
                        ),
                        text: None,
                        measured: Some(depths[0]),
                        required: Some(want),
                    });
                }
                if depths[1] < second {
                    hard.push(Violation {
                        rule: "bett_zugang".into(),
                        severity: Severity::Hart,
                        item: Some(p.it.reference.clone()),
                        message: format!(
                            "Bett hat {} cm an seiner zweiten Laengsseite, braucht {}",
                            depths[1], second
                        ),
                        text: None,
                        measured: Some(depths[1]),
                        required: Some(second),
                    });
                }
            }
            Kind::Desk => {
                let want = rules.abstand("schreibtisch_stuhlzone")?;
                let best = SIDES
                    .iter()
                    .map(|s| free_depth_on_side(&grid, &p.rect, *s, want))
                    .max()
                    .unwrap_or(0);
                if best < want {
                    hard.push(Violation {
                        rule: "stuhlzone".into(), severity: Severity::Hart,
                        item: Some(p.it.reference.clone()),
                        message: format!("Schreibtisch hat {} cm fuer den Stuhl an seiner besten Seite, braucht {}", best, want),
                        text: None,
                        measured: Some(best), required: Some(want),
                    });
                }
            }
            Kind::Wardrobe => {
                let want = rules.abstand("schrank_tuer_oeffnen")?;
                let best = SIDES
                    .iter()
                    .map(|s| free_depth_on_side(&grid, &p.rect, *s, want))
                    .max()
                    .unwrap_or(0);
                if best < want {
                    hard.push(Violation {
                        rule: "schrank_tuer".into(),
                        severity: Severity::Hart,
                        item: Some(p.it.reference.clone()),
                        message: format!(
                            "Schrank hat {} cm zum Tueroeffnen, braucht {}",
                            best, want
                        ),
                        text: None,
                        measured: Some(best),
                        required: Some(want),
                    });
                }
            }
            // Das Schlafsofa wird zum Ausziehen nicht verschoben (Lars, 2026-08-30). Die
            // Ausklapptiefe ist damit dauerhaft belegte Flaeche und nichts, das man sich im
            // Gastfall borgt. Geprueft nur an den Laengsseiten: ausgezogen wird ueber die
            // lange Kante, und in genau eine Richtung, also reicht die bessere Seite.
            Kind::Couch => {
                if let Ok(want) = rules.abstand("couch_ausklapptiefe") {
                    let long = if p.w >= p.d {
                        [Side::North, Side::South]
                    } else {
                        [Side::West, Side::East]
                    };
                    let best = long
                        .iter()
                        .map(|s| free_depth_on_side(&grid, &p.rect, *s, want))
                        .max()
                        .unwrap_or(0);
                    if best < want {
                        // Bis 2026-08-31 hiess dieser Verstoss `couch_ausklappen`, waehrend
                        // jede rules.toml dieselbe Regel als R8 fuehrte. Zwei Namen fuer eine
                        // Regel, und der Bericht nannte den, den die Wohnung nicht kannte.
                        einsortieren(
                            haus_regel(
                                rules,
                                "R8",
                                Some(p.it.reference.clone()),
                                format!("Couch hat {} cm zum Ausklappen an ihrer besten Laengsseite, braucht {} — sie soll dafuer nicht verschoben werden", best, want),
                                Some(best),
                                Some(want),
                            )?,
                            &mut hard,
                            &mut soft,
                        );
                    }
                }
            }
            // Ein Esstisch, an dem niemand einen Stuhl herausziehen kann, ist ein Regal. Hart,
            // wenn keine Seite den Platz hat; weich, wenn nur eine ihn hat — das ist eine
            // Sitzplatzzahl und kein Verstoss: ein Wandklapptisch hat legitim eine Seite.
            Kind::Table => {
                let want = rules.abstand("esstisch_stuhl_ausziehen")?;
                let sides: Vec<i32> = SIDES
                    .iter()
                    .map(|s| free_depth_on_side(&grid, &p.rect, *s, want))
                    .collect();
                let best = *sides.iter().max().unwrap_or(&0);
                let seats = sides.iter().filter(|d| **d >= want).count();
                if seats == 0 {
                    hard.push(Violation {
                        rule: "stuhl_ausziehen".into(), severity: Severity::Hart,
                        item: Some(p.it.reference.clone()),
                        message: format!("Tisch hat {} cm zum Stuhlherausziehen an seiner besten Seite, braucht {}", best, want),
                        text: None,
                        measured: Some(best), required: Some(want),
                    });
                } else if seats < 2 {
                    soft.push(Violation {
                        rule: "stuhl_ausziehen".into(), severity: Severity::Weich,
                        item: Some(p.it.reference.clone()),
                        message: format!("Tisch bietet {} Sitzplatz — nur eine Seite hat die {} cm, die ein Stuhl braucht", seats, want),
                        text: None,
                        measured: Some(seats as i32), required: Some(2),
                    });
                }
            }
            Kind::CoffeeTable => {
                let want = rules.abstand("couchtisch_vor_sofa")?;
                for c in placed.iter().filter(|q| q.kind == Kind::Couch) {
                    let dx = (c.rect.x - p.rect.right())
                        .max(p.rect.x - c.rect.right())
                        .max(0);
                    let dy = (c.rect.y - p.rect.bottom())
                        .max(p.rect.y - c.rect.bottom())
                        .max(0);
                    let gap = (((dx * dx + dy * dy) as f64).sqrt()).round() as i32;
                    if gap < want {
                        soft.push(Violation {
                            rule: "couchtisch_abstand".into(),
                            severity: Severity::Weich,
                            item: Some(p.it.reference.clone()),
                            message: format!("Couchtisch steht {} cm vom Sofa, will {}", gap, want),
                            text: None,
                            measured: Some(gap),
                            required: Some(want),
                        });
                    }
                }
            }
            _ => {}
        }
    }

    // --- Laufwege ------------------------------------------------------------
    let field = clearance_field(&grid);
    let mut waypoints: Vec<(String, Pt)> = Vec::new();
    for o in &room.oeffnungen {
        if o.typ.as_deref() == Some("fenster") {
            continue;
        }
        let Some(seg) = opening_segment(room, o) else {
            continue;
        };
        let mid = [
            (seg.a[0] + seg.b[0]) as f64 / 2.0,
            (seg.a[1] + seg.b[1]) as f64 / 2.0,
        ];
        waypoints.push((
            o.id.clone(),
            [
                (mid[0] + seg.normal[0] * WEGPUNKT_ABSTAND as f64) as i32,
                (mid[1] + seg.normal[1] * WEGPUNKT_ABSTAND as f64) as i32,
            ],
        ));
    }
    for f in &room.fix_moebel {
        let Some(zone_decl) = &f.anlaufzone else {
            continue;
        };
        waypoints.push((f.id.clone(), anlauf_punkt(&f.rect(), zone_decl.seite)));
    }
    let find = |id: &str| waypoints.iter().find(|(n, _)| n == id).map(|(_, p)| *p);

    let mut corridors = Vec::new();
    for r in &room.routen {
        let (from, to) = (r.von.as_str(), r.nach.as_str());
        // Ein Wegpunkt, den es nicht gibt, ist ein Fehler und kein uebersprungener Weg. Bis
        // 2026-08-30 hat diese Stelle still weitergemacht: eine vertippte Oeffnung kostete eine
        // Route, und der Bericht sah danach aus wie einer ueber eine Wohnung mit weniger Wegen.
        let (Some(a), Some(b)) = (find(from), find(to)) else {
            let bekannt: Vec<&str> = waypoints.iter().map(|(n, _)| n.as_str()).collect();
            return Err(ModelError::Missing(format!(
                "Route {from} → {to}: kein Wegpunkt dieses Namens. Wegpunkte sind Oeffnungen \
                 ausser Fenstern und feste Moebel mit `anlaufzone`; vorhanden sind: {}",
                bekannt.join(", ")
            )));
        };
        let bottleneck = widest_path(&grid, &field, a, b);
        let width = bottleneck.map(|v| (v * 2.0).round() as i32);
        corridors.push(Corridor {
            from: from.into(),
            to: to.into(),
            width_cm: width,
        });
        match width {
            None => hard.push(Violation {
                rule: "laufweg".into(),
                severity: Severity::Hart,
                item: None,
                message: format!("gar kein begehbarer Weg von {} nach {}", from, to),
                text: None,
                measured: None,
                required: Some(rules.laufwege.haupt_min),
            }),
            Some(w) if w < rules.laufwege.haupt_min => hard.push(Violation {
                rule: "laufweg".into(),
                severity: Severity::Hart,
                item: None,
                message: format!(
                    "Route {} → {} schnuert auf {} cm ein, unter dem Minimum von {} cm",
                    from, to, w, rules.laufwege.haupt_min
                ),
                text: None,
                measured: Some(w),
                required: Some(rules.laufwege.haupt_min),
            }),
            Some(w) if w < rules.laufwege.haupt_soll => soft.push(Violation {
                rule: "laufweg".into(),
                severity: Severity::Weich,
                item: None,
                message: format!(
                    "Route {} → {} schnuert auf {} cm ein, unter dem Ziel von {} cm",
                    from, to, w, rules.laufwege.haupt_soll
                ),
                text: None,
                measured: Some(w),
                required: Some(rules.laufwege.haupt_soll),
            }),
            _ => {}
        }
    }

    let occupied: i32 = placed.iter().map(|p| p.w * p.d).sum();
    let refs: Vec<&str> = layout.items.iter().map(|i| i.reference.as_str()).collect();
    let uncertainties = model
        .catalogue
        .values()
        .filter(|i: &&Item| i.is_uncertain() && refs.contains(&i.id.as_str()))
        .map(|i| Uncertainty {
            reference: i.id.clone(),
            label: i.label.clone(),
            fields: i.unsicher.clone(),
        })
        .collect();

    Ok(CheckResult {
        layout: layout.name.clone(),
        pass: hard.is_empty(),
        hard,
        soft,
        uncertainties,
        metrics: Metrics {
            room_area_m2: room.area_m2(),
            occupied_area_m2: occupied as f64 / 10_000.0,
            free_area_m2: (grid.free_cells() as f64 * (RES * RES) as f64) / 10_000.0,
            corridors,
        },
        veraltet: placed
            .iter()
            .filter(|p| {
                p.item
                    .and_then(|i| i.prioritaet.as_deref())
                    .is_some_and(|s| s == "verworfen")
            })
            .map(|p| p.it.reference.clone())
            .collect(),
        nicht_geprueft: rules
            .nicht_geprueft(REGEL_IDS)
            .into_iter()
            .map(|r| UngeprueteRegel {
                rule: r.id.clone(),
                text: r.text.clone(),
                grund: "diese Maschine prueft sie nicht".to_string(),
            })
            .chain(ausgefallen.into_iter().map(|(id, grund)| UngeprueteRegel {
                rule: id.clone(),
                text: rules.regel(&id).map(|r| r.text.clone()).unwrap_or_default(),
                grund,
            }))
            .collect(),
    })
}
