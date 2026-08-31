//! Kommt das Stueck ueberhaupt herein und bis an seinen Platz?
//!
//! Jede andere Pruefung dieser Capability fragt, ob ein Moebel an einer Stelle **stehen** darf.
//! Keine fragte, ob es dorthin **gelangt**. Das ist keine akademische Luecke: ein Schrank, der
//! an seinem Platz jede Regel erfuellt und nicht durch die Wohnungstuer passt, ist gekauftes
//! Geld — und die Zahl, die das entscheidet, steht laengst in der Zeile (`b`, `t`) und in
//! `room.toml` (`breite` der Oeffnung). Zusammengerechnet hat sie nie jemand.
//!
//! ## Das Modell, und wo es bewusst zu streng ist
//!
//! Gesucht wird ein Weg im **Konfigurationsraum**: nicht wo das Moebel steht, sondern jede
//! Lage, die es einnehmen kann — Position und Drehung zugleich. Zwei Drehungen genuegen, denn
//! ein Rechteck sieht nach 180 Grad aus wie vorher; die Zustaende sind also
//! `(x, y, quer|laengs)`.
//!
//! Drei Vereinfachungen, alle in dieselbe Richtung, damit ein Ja belastbar ist und ein Nein
//! eine Warnung:
//!
//! 1. **Gedreht wird nur, wo der Umkreis frei ist.** Ein Mensch kippt einen Schrank auf die
//!    Ecke und dreht ihn auf der Stelle; diese Maschine verlangt den ganzen Kreis, den das
//!    Stueck beim Drehen ueberstreicht. Damit ist jedes gefundene Manoever wirklich moeglich,
//!    und ein „geht nicht" heisst „nicht ohne Kippen".
//! 2. **Hindernisse sind nur die festen Einbauten.** Am Einzugstag ist die Wohnung leer, und
//!    in welcher Reihenfolge die uebrigen Stuecke hineingehen, entscheidet nicht diese Datei.
//! 3. **Die Tuerhoehe wird nicht geprueft.** `room.toml` fuehrt Oeffnungen mit `breite` und
//!    ohne Hoehe. Eine Hoehe zu erfinden waere schlimmer als die Luecke zu nennen.
//!
//! Die Tuer selbst liegt in der Wand und damit ausserhalb des begehbaren Polygons. Der Weg
//! beginnt deshalb an der ersten Lage **innerhalb** des Raums vor der Oeffnung, und dass das
//! Stueck durch die Oeffnung passt, ist eine eigene Frage mit einer eigenen Antwort.

use crate::geometry::{Grid, RES};
use crate::model::{footprint, Layout, Model, ModelError, Rect};
use serde::Serialize;
use std::collections::VecDeque;

/// Passt das Stueck durch die Wohnungstuer?
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "art", rename_all = "snake_case")]
pub enum Tuerpass {
    /// Die schmale Seite passt, mit so vielen cm Luft.
    Passt { luft_cm: i32, tuer_cm: i32 },
    /// Die schmale Seite ist breiter als die Oeffnung.
    PasstNicht { fehlen_cm: i32, tuer_cm: i32 },
    /// Passt nicht am Stueck, kommt aber zerlegt herein — die Zeile sagt es (`zerlegbar`).
    ///
    /// Der Unterschied zu `Passt` ist keine Formalie: er sagt, dass hier jemand geschraubt
    /// hat, und dass die Aussage aus einer Deklaration stammt und nicht aus der Geometrie.
    ZerlegtGetragen { fehlen_cm: i32, tuer_cm: i32 },
    /// Keine Oeffnung dieser Wohnung ist als Eingang deklariert (`eingang = true`).
    KeinEingang,
}

/// Wie das Stueck an seinen Platz kommt — oder warum nicht.
#[derive(Debug, Clone, Serialize)]
pub struct Einbringung {
    pub reference: String,
    pub b: i32,
    pub t: i32,
    pub tuer: Tuerpass,
    /// Gibt es einen Weg von der Tuer bis zum Zielplatz.
    pub erreichbar: bool,
    /// Wie viele Rasterschritte der kuerzeste Weg braucht, Drehungen eingerechnet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schritte: Option<usize>,
    /// Ob der Weg das Stueck mindestens einmal drehen muss.
    pub dreht: bool,
    /// Im Klartext, was fehlt — leer, wenn nichts fehlt.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grund: Option<String>,
}

/// Die Lagen, die ein Rechteck einnehmen kann: Zelle und Ausrichtung.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Lage {
    c: i32,
    r: i32,
    quer: bool,
}

/// Das Raster der Wohnung ohne die beweglichen Moebel: Polygon minus feste Einbauten.
fn leerer_raum(model: &Model) -> Grid {
    let mut g = Grid::new(&model.room.hauptraum.polygon);
    for f in &model.room.fix_moebel {
        g.occupy(&f.rect());
    }
    g
}

/// Ist die Grundflaeche an dieser Lage vollstaendig frei?
fn frei(g: &Grid, l: Lage, w: i32, d: i32) -> bool {
    let (w, d) = if l.quer { (d, w) } else { (w, d) };
    let cw = (w as f64 / RES as f64).ceil() as i32;
    let cd = (d as f64 / RES as f64).ceil() as i32;
    if l.c < 0 || l.r < 0 || l.c + cw > g.cols as i32 || l.r + cd > g.rows as i32 {
        return false;
    }
    for r in l.r..(l.r + cd) {
        for c in l.c..(l.c + cw) {
            if g.free[g.idx(c as usize, r as usize)] == 0 {
                return false;
            }
        }
    }
    true
}

/// Darf an dieser Lage gedreht werden? Der ueberstrichene Kreis muss frei sein.
///
/// Genaehert durch sein umschriebenes Quadrat mit der Diagonalen als Kantenlaenge, um die
/// Mitte des Stuecks. Konservativ und absichtlich: was hier durchgeht, geht wirklich.
fn darf_drehen(g: &Grid, l: Lage, w: i32, d: i32) -> bool {
    let diag = (((w * w + d * d) as f64).sqrt().ceil()) as i32;
    let (bw, bd) = if l.quer { (d, w) } else { (w, d) };
    let mitte_x = l.c * RES + bw / 2;
    let mitte_y = l.r * RES + bd / 2;
    let kreis = Lage {
        c: (mitte_x - diag / 2).div_euclid(RES),
        r: (mitte_y - diag / 2).div_euclid(RES),
        quer: false,
    };
    frei(g, kreis, diag, diag)
}

/// Dieselbe Lage, um die eigene Mitte gedreht.
fn gedreht(l: Lage, w: i32, d: i32) -> Lage {
    let (bw, bd) = if l.quer { (d, w) } else { (w, d) };
    let mitte_x = l.c * RES + bw / 2;
    let mitte_y = l.r * RES + bd / 2;
    let (nw, nd) = (bd, bw);
    Lage {
        c: (mitte_x - nw / 2).div_euclid(RES),
        r: (mitte_y - nd / 2).div_euclid(RES),
        quer: !l.quer,
    }
}

/// Passt das Stueck durch die Wohnungstuer?
///
/// `zerlegbar` kommt aus der Zeile des Stuecks und nicht aus einer Vermutung ueber Betten.
pub fn durch_die_tuer(model: &Model, b: i32, t: i32, zerlegbar: bool) -> Tuerpass {
    let Some(e) = model
        .room
        .oeffnungen
        .iter()
        .find(|o| o.eingang == Some(true))
    else {
        return Tuerpass::KeinEingang;
    };
    // Die schmale Seite entscheidet: durch eine Tuer geht ein Schrank hochkant.
    let schmal = b.min(t);
    if schmal <= e.breite {
        Tuerpass::Passt {
            luft_cm: e.breite - schmal,
            tuer_cm: e.breite,
        }
    } else if zerlegbar {
        Tuerpass::ZerlegtGetragen {
            fehlen_cm: schmal - e.breite,
            tuer_cm: e.breite,
        }
    } else {
        Tuerpass::PasstNicht {
            fehlen_cm: schmal - e.breite,
            tuer_cm: e.breite,
        }
    }
}

/// Kommt ein Stueck dieser Groesse von der Tuer bis auf dieses Rechteck?
pub fn weg_zum_platz(
    model: &Model,
    b: i32,
    t: i32,
    ziel: Rect,
    zerlegbar: bool,
) -> Result<Einbringung, ModelError> {
    let tuer = durch_die_tuer(model, b, t, zerlegbar);
    let g = leerer_raum(model);

    let ziel_lage = Lage {
        c: ziel.x.div_euclid(RES),
        r: ziel.y.div_euclid(RES),
        quer: ziel.w != b,
    };

    // Startlagen: alles, was unmittelbar vor der Eingangsoeffnung im Raum liegt. Welche Zelle
    // das genau ist, haengt an der Wand, in der die Tuer sitzt — deshalb wird die ganze
    // Spanne der Oeffnung als moeglicher Eintritt genommen und nicht ein gewaehlter Punkt.
    let Some(e) = model
        .room
        .oeffnungen
        .iter()
        .find(|o| o.eingang == Some(true))
    else {
        return Ok(Einbringung {
            reference: String::new(),
            b,
            t,
            tuer,
            erreichbar: false,
            schritte: None,
            dreht: false,
            grund: Some(
                "keine Oeffnung ist als Eingang deklariert (`eingang = true` in room.toml)".into(),
            ),
        });
    };
    let Some(seg) = crate::clearance::opening_segment(&model.room, e) else {
        return Err(ModelError::Missing(format!(
            "Oeffnung `{}` liegt in keiner bekannten Wand",
            e.id
        )));
    };

    // Der Eintritt ist der Punkt eine Rasterzelle INNERHALB der Tuermitte. Startlagen sind
    // alle Lagen, deren Grundflaeche diesen Punkt ueberdeckt — also jede Art, wie das Stueck
    // in der Tuer stehen kann. Einen einzelnen Ankerpunkt zu waehlen hiesse, eine Haltung
    // vorzuschreiben, und die falsche Haltung waere ein Nein, das nur vom Raster kommt.
    let ex = ((seg.a[0] + seg.b[0]) as f64 / 2.0 + seg.normal[0] * RES as f64) as i32;
    let ey = ((seg.a[1] + seg.b[1]) as f64 / 2.0 + seg.normal[1] * RES as f64) as i32;
    let (ec, er) = (ex.div_euclid(RES), ey.div_euclid(RES));

    let mut start: Vec<Lage> = Vec::new();
    for quer in [false, true] {
        let (bw, bd) = if quer { (t, b) } else { (b, t) };
        let (cw, cd) = (
            (bw as f64 / RES as f64).ceil() as i32,
            (bd as f64 / RES as f64).ceil() as i32,
        );
        for r in (er - cd + 1)..=er {
            for c in (ec - cw + 1)..=ec {
                let l = Lage { c, r, quer };
                if frei(&g, l, b, t) {
                    start.push(l);
                }
            }
        }
    }

    if !frei(&g, ziel_lage, b, t) {
        return Ok(Einbringung {
            reference: String::new(),
            b,
            t,
            tuer,
            erreichbar: false,
            schritte: None,
            dreht: false,
            grund: Some(
                "der Zielplatz selbst ist im leeren Raum nicht frei — ein fester Einbau steht dort"
                    .into(),
            ),
        });
    }
    if start.is_empty() {
        return Ok(Einbringung {
            reference: String::new(),
            b,
            t,
            tuer,
            erreichbar: false,
            schritte: None,
            dreht: false,
            grund: Some(
                "direkt hinter der Tuer ist keine Lage frei, in der das Stueck steht".into(),
            ),
        });
    }

    // Breitensuche: vier Nachbarzellen plus die Drehung auf der Stelle. Alle Schritte kosten
    // gleich viel, also findet die erste Ankunft den kuerzesten Weg.
    let mut gesehen: std::collections::HashSet<Lage> = std::collections::HashSet::new();
    let mut queue: VecDeque<(Lage, usize, bool)> = VecDeque::new();
    for s in start {
        if gesehen.insert(s) {
            queue.push_back((s, 0, false));
        }
    }
    while let Some((l, n, dreht)) = queue.pop_front() {
        if l == ziel_lage {
            return Ok(Einbringung {
                reference: String::new(),
                b,
                t,
                tuer,
                erreichbar: true,
                schritte: Some(n),
                dreht,
                grund: None,
            });
        }
        for (dc, dr) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let n2 = Lage {
                c: l.c + dc,
                r: l.r + dr,
                quer: l.quer,
            };
            if frei(&g, n2, b, t) && gesehen.insert(n2) {
                queue.push_back((n2, n + 1, dreht));
            }
        }
        if darf_drehen(&g, l, b, t) {
            let n2 = gedreht(l, b, t);
            if frei(&g, n2, b, t) && gesehen.insert(n2) {
                queue.push_back((n2, n + 1, true));
            }
        }
    }

    Ok(Einbringung {
        reference: String::new(),
        b,
        t,
        tuer,
        erreichbar: false,
        schritte: None,
        dreht: false,
        grund: Some(
            "kein Weg von der Tuer bis zum Platz, ohne das Stueck zu kippen oder einen Einbau zu \
             beruehren"
                .into(),
        ),
    })
}

/// Dieselbe Frage fuer ein Stueck, das in einem Layout schon einen Platz hat.
pub fn einbringung(
    model: &Model,
    layout: &Layout,
    reference: &str,
) -> Result<Einbringung, ModelError> {
    let it = layout
        .items
        .iter()
        .find(|i| i.reference == reference)
        .ok_or_else(|| {
            ModelError::Missing(format!(
                "`{reference}` steht nicht in \"{}\" — nichts einzubringen",
                layout.name
            ))
        })?;
    let (w, d, _) = footprint(it, &model.catalogue)?;
    let zerlegbar = model
        .catalogue
        .get(reference)
        .and_then(|i| i.zerlegbar)
        .unwrap_or(false);
    let mut e = weg_zum_platz(
        model,
        w,
        d,
        Rect {
            x: it.x,
            y: it.y,
            w,
            d,
        },
        zerlegbar,
    )?;
    e.reference = reference.to_string();
    Ok(e)
}
