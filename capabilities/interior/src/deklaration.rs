//! Wer wird noch am Namen gemessen, und was kostet es, das zu aendern?
//!
//! PRD Q61 hat die Namensheuristik abgeloest — `bett*` ist ein Bett, `schrank*` ein Schrank —,
//! und B26 hat sie **absichtlich stehen lassen**: wer nichts erklaert, wird weiter am Namen
//! gemessen. Der Grund war gut und ist es geblieben. 42 Zeilen an einem Tag umzustellen waere
//! ein Stichtag, an dem sich Verdikte aendern, ohne dass jemand die Zahlen dahinter geprueft
//! hat.
//!
//! Die Folge davon war trotzdem ein Stillstand: die Maschine ist seit dem 31. August klueger
//! als ihre Daten, und niemand konnte sehen, wie viel klueger. Diese Datei beantwortet das in
//! drei Zahlen je Eintrag:
//!
//! 1. **Erklaert er sich schon?** Sonst: wofuer haelt die Heuristik ihn, und welche Schwellen
//!    erbt er damit aus `rules.toml`.
//! 2. **Was waere die Deklaration?** Ein Vorschlag, der genau das aufschreibt, was die
//!    Maschine ohnehin schon annimmt — als TOML zum Einfuegen.
//! 3. **Was aendert sie?** Dieselbe Rechnung, die `POST /api/items/:id/impact` fuer eine
//!    beliebige Aenderung fuehrt: alle Layouts vorher, alle nachher, und die Liste derer, bei
//!    denen sich das Verdikt bewegt.
//!
//! **Der Vorschlag ist keine Empfehlung, und er ist nicht wirkungsfrei.** Die Namensfassung
//! prueft an einem Bett die beiden Laengsseiten, die deklarierte Fassung zaehlt jede Seite,
//! die tief genug ist; das ist nicht dieselbe Frage, und Punkt 3 sagt deshalb bei jedem
//! Eintrag, was sich bewegt. Wer die Zeile uebernimmt, uebernimmt eine Entscheidung — und das
//! ist der Grund, warum diese Datei nichts schreibt.

use crate::clearance::{check_layout, kind_of_name, Kind};
use crate::model::{Model, ModelError};
use crate::store::Item;
use serde::Serialize;

/// Die Zeilen, die ein Stueck ueber sich selbst sagen kann (PRD Q61).
#[derive(Debug, Clone, Default, Serialize)]
pub struct Vorschlag {
    pub open_clear: Option<i32>,
    pub access_sides: Option<i32>,
    pub access_clear: Option<i32>,
    pub expands_dir: Option<String>,
    pub expands_to: Option<i32>,
    /// Dieselben Felder als TOML, direkt zum Einfuegen in `inventory/*.toml`.
    pub toml: String,
}

/// Was sich an den Verdikten bewegt, wenn der Vorschlag uebernommen wird.
#[derive(Debug, Clone, Serialize)]
pub struct Folgen {
    pub layouts: usize,
    pub bestanden_vorher: usize,
    pub bestanden_nachher: usize,
    /// Die Layouts, deren Verstossliste sich aendert — auch wenn das Verdikt gleich bleibt.
    pub geaendert: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Stand {
    pub id: String,
    pub label: String,
    /// Erklaert dieses Stueck schon selbst, was es braucht?
    pub erklaert: bool,
    /// Wofuer die Namensheuristik es haelt, solange es das nicht tut.
    pub geraten_als: &'static str,
    /// Welche Schwellen aus `rules.toml` es damit erbt.
    pub geerbte_schwellen: Vec<(String, i32)>,
    /// In welchen Layouts es steht — so viele Verdikte haengen daran.
    pub in_layouts: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vorschlag: Option<Vorschlag>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folgen: Option<Folgen>,
}

fn kind_name(k: Kind) -> &'static str {
    match k {
        Kind::Bed => "Bett",
        Kind::Desk => "Schreibtisch",
        Kind::Couch => "Sofa",
        Kind::Wardrobe => "Schrank",
        Kind::CoffeeTable => "Couchtisch",
        Kind::Table => "Tisch",
        Kind::Shelf => "Regal",
        Kind::Other => "nichts davon",
    }
}

/// Erklaert dieses Stueck sich schon selbst? Dieselbe Bedingung wie in `clearance.rs`.
pub fn erklaert(i: &Item) -> bool {
    i.open_clear.is_some() || i.access_sides.is_some() || i.expands_to.is_some()
}

/// Die Zeilen, die aufschreiben, was die Heuristik ohnehin annimmt.
///
/// `None` fuer `Kind::CoffeeTable`, `Kind::Shelf` und `Kind::Other`: fuer die nimmt die
/// Heuristik nichts an, also gaebe es nichts abzuschreiben. Ein erfundener Vorschlag waere
/// dort eine neue Regel und keine Uebersetzung einer bestehenden.
pub fn vorschlag(model: &Model, i: &Item) -> Option<Vorschlag> {
    let r = &model.rules;
    let mut v = Vorschlag::default();
    match kind_of_name(&i.id) {
        Kind::Bed => {
            let laengs = r.abstand("bett_zugang_laengsseite").ok()?;
            let zweite = r.abstand("bett_zugang_zweite_seite").ok()?;
            v.access_clear = Some(laengs);
            // Verlangt die Wohnung an der zweiten Laengsseite nichts, ist das eine Aussage
            // ueber Einzelbelegung — eine Seite genuegt.
            v.access_sides = Some(if zweite > 0 { 2 } else { 1 });
        }
        Kind::Desk => {
            v.access_sides = Some(1);
            v.access_clear = Some(r.abstand("schreibtisch_stuhlzone").ok()?);
        }
        Kind::Wardrobe => v.open_clear = Some(r.abstand("schrank_tuer_oeffnen").ok()?),
        Kind::Table => {
            v.access_sides = Some(2);
            v.access_clear = Some(r.abstand("esstisch_stuhl_ausziehen").ok()?);
        }
        Kind::Couch => {
            // Die Ausklapptiefe kennt das Stueck selbst, wo sie gemessen wurde; sonst die
            // Norm aus der Wohnung, als Gesamttiefe und nicht als Zuwachs.
            let tiefe = i.t.unwrap_or(0);
            v.expands_to = Some(
                i.t_ausgeklappt
                    .unwrap_or(tiefe + r.abstand("couch_ausklapptiefe").ok()?),
            );
            v.expands_dir = Some("sued".into());
        }
        Kind::CoffeeTable | Kind::Shelf | Kind::Other => return None,
    }
    v.toml = toml_zeilen(&v);
    Some(v)
}

fn toml_zeilen(v: &Vorschlag) -> String {
    let mut z = Vec::new();
    if let Some(x) = v.open_clear {
        z.push(format!("open_clear = {x}"));
    }
    if let Some(x) = v.access_sides {
        z.push(format!("access_sides = {x}"));
    }
    if let Some(x) = v.access_clear {
        z.push(format!("access_clear = {x}"));
    }
    if let (Some(d), Some(t)) = (v.expands_dir.as_ref(), v.expands_to) {
        z.push(format!("expands = {{ dir = \"{d}\", to = {t} }}"));
    }
    z.join("\n")
}

fn angewandt(i: &Item, v: &Vorschlag) -> Item {
    let mut n = i.clone();
    n.open_clear = v.open_clear.or(n.open_clear);
    n.access_sides = v.access_sides.or(n.access_sides);
    n.access_clear = v.access_clear.or(n.access_clear);
    n.expands_to = v.expands_to.or(n.expands_to);
    n.expands_dir = v
        .expands_dir
        .as_deref()
        .and_then(crate::model::Seite::parse)
        .or(n.expands_dir);
    n
}

/// Ein Layout, sein Verdikt und die Kennungen dessen, was daran auffiel.
///
/// Nur die Kennungen und nicht die vollen Verstoesse: verglichen wird, ob sich das Ergebnis
/// bewegt, und dafuer ist der Text unerheblich — er ist fuer jede Instanz derselben Regel
/// derselbe.
#[derive(PartialEq)]
struct Verdikt {
    layout: String,
    pass: bool,
    hart: Vec<String>,
    weich: Vec<String>,
}

/// Alle Verdikte dieser Wohnung, als Vergleichsgrundlage.
fn verdikte(model: &Model) -> Result<Vec<Verdikt>, ModelError> {
    let mut out = Vec::new();
    for name in model.layout_names()? {
        let l = model.load_layout(&name)?;
        let r = check_layout(model, &l)?;
        out.push(Verdikt {
            layout: name,
            pass: r.pass,
            hart: r.hard.iter().map(|v| v.rule.clone()).collect(),
            weich: r.soft.iter().map(|v| v.rule.clone()).collect(),
        });
    }
    Ok(out)
}

/// Wie weit das Inventar der Maschine hinterherhaengt, Eintrag fuer Eintrag.
pub fn uebersicht(model: &Model) -> Result<Vec<Stand>, ModelError> {
    let vorher = verdikte(model)?;
    let mut in_layouts: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for name in model.layout_names()? {
        for it in &model.load_layout(&name)?.items {
            in_layouts
                .entry(it.reference.clone())
                .or_default()
                .push(name.clone());
        }
    }

    let mut out = Vec::new();
    for i in model.catalogue.values() {
        let k = kind_of_name(&i.id);
        let schon = erklaert(i);
        let v = if schon { None } else { vorschlag(model, i) };

        let folgen = match &v {
            None => None,
            Some(v) => {
                // Dieselbe Rechnung wie `POST /api/items/:id/impact`, nur fuer den Vorschlag
                // statt fuer eine getippte Aenderung.
                let mut probe = Model {
                    room: model.room.clone(),
                    rules: model.rules.clone(),
                    catalogue: model.catalogue.clone(),
                    states: model.states.clone(),
                    flat_dir: model.flat_dir.clone(),
                };
                probe.catalogue.insert(i.id.clone(), angewandt(i, v));
                let nachher = verdikte(&probe)?;
                let geaendert = vorher
                    .iter()
                    .zip(nachher.iter())
                    .filter(|(a, b)| a != b)
                    .map(|(a, _)| a.layout.clone())
                    .collect();
                Some(Folgen {
                    layouts: vorher.len(),
                    bestanden_vorher: vorher.iter().filter(|x| x.pass).count(),
                    bestanden_nachher: nachher.iter().filter(|x| x.pass).count(),
                    geaendert,
                })
            }
        };

        out.push(Stand {
            id: i.id.clone(),
            label: i.label.clone(),
            erklaert: schon,
            geraten_als: kind_name(k),
            geerbte_schwellen: geerbte_schwellen(model, k),
            in_layouts: in_layouts.get(&i.id).cloned().unwrap_or_default(),
            vorschlag: v,
            folgen,
        });
    }
    Ok(out)
}

/// Welche Schwellen aus `rules.toml` ein Stueck ueber seinen Namen erbt.
fn geerbte_schwellen(model: &Model, k: Kind) -> Vec<(String, i32)> {
    let namen: &[&str] = match k {
        Kind::Bed => &["bett_zugang_laengsseite", "bett_zugang_zweite_seite"],
        Kind::Desk => &["schreibtisch_stuhlzone"],
        Kind::Wardrobe => &["schrank_tuer_oeffnen"],
        Kind::Couch => &["couch_ausklapptiefe"],
        Kind::Table => &["esstisch_stuhl_ausziehen"],
        Kind::CoffeeTable => &["couchtisch_vor_sofa"],
        Kind::Shelf | Kind::Other => &[],
    };
    namen
        .iter()
        .filter_map(|n| model.rules.abstand(n).ok().map(|v| (n.to_string(), v)))
        .collect()
}
