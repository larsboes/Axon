//! Paritaet gegen die aufgezeichneten Verdikte der TypeScript-Vorlage — an der ECHTEN Wohnung.
//!
//! Die Baseline wurde am 2026-08-30 aus der TS-Engine gezogen, bevor sie geloescht wurde. Sie
//! ist kein zweiter Pruefer, sondern ein Protokoll: dieselben zehn Layouts, dieselben Regeln,
//! dieselben Zahlen. Weicht Rust ab, ist entweder die Portierung falsch oder eine Regel hat
//! sich absichtlich geaendert — und dann gehoert die Baseline mit einer Begruendung neu
//! aufgezeichnet, nicht der Test entschaerft.
//!
//! Warum sie im Overlay liegt und nicht hier: sie enthaelt die Korridorbreiten und Moebelmasse
//! EINER Wohnung. Das ist dieselbe Kategorie wie das Raummodell selbst, und dieses Repository
//! ist oeffentlich. Sie liegt deshalb neben der Wohnung, die sie beschreibt, unter
//! `<overlay>/data/interior/flats/<id>/ts-baseline.json`.
//!
//! Ohne `AXON_PERSONAL_ROOT` meldet dieser Test, warum er nichts getan hat, und kehrt zurueck —
//! der Zustand in CI und auf jedem Rechner, der die Dateien nicht haelt. Die Maschine selbst
//! ist davon unabhaengig geprueft: `tests/engine.rs` laeuft ueberall.
//!
//! Die Routenbreiten haengen an der Rasterweite (geometry::RES = 5 cm). Wer sie aendert,
//! aendert diese Zahlen.

use interior::clearance::check_layout;
use interior::model::{default_flat, Model};
use serde_json::Value;

/// Das Modell der aktiven Wohnung und ihre aufgezeichnete Vorlage, oder `None` mit einem Grund.
/// `AXON_PERSONAL_ROOT` wird gelesen und nie gesetzt: welches Overlay gemeint ist, entscheidet
/// die Umgebung, nicht der Test.
fn live() -> Option<(Model, Value)> {
    std::env::var_os("AXON_PERSONAL_ROOT")?;
    let flat = match default_flat() {
        Ok(f) => f,
        Err(e) => {
            eprintln!(
                "AXON_PERSONAL_ROOT ist gesetzt, aber keine Wohnung waehlbar ({e}); uebersprungen"
            );
            return None;
        }
    };
    let model = match Model::load(&flat) {
        Ok(m) => m,
        Err(e) => {
            eprintln!(
                "AXON_PERSONAL_ROOT ist gesetzt, aber das Modell laedt nicht ({e}); uebersprungen"
            );
            return None;
        }
    };
    let path = model.flat_dir.join("ts-baseline.json");
    if !path.is_file() {
        eprintln!("keine ts-baseline.json neben dieser Wohnung; uebersprungen");
        return None;
    }
    let text = std::fs::read_to_string(&path).expect("die Baseline muss lesbar sein");
    Some((
        model,
        serde_json::from_str(&text).expect("Baseline ist gueltiges JSON"),
    ))
}

macro_rules! live_or_skip {
    () => {
        match live() {
            Some(v) => v,
            None => {
                eprintln!("setze AXON_PERSONAL_ROOT, um die Paritaet gegen die echte Wohnung zu pruefen; uebersprungen");
                return;
            }
        }
    };
}

#[test]
fn jedes_layout_faellt_gleich_aus_wie_in_der_vorlage() {
    let (model, base) = live_or_skip!();
    let layouts = base["layouts"].as_object().expect("layouts");
    let mut geprueft = 0;
    let mut abweichungen: Vec<String> = Vec::new();

    for (name, want) in layouts {
        let layout = model
            .load_layout(name)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let got = check_layout(&model, &layout).unwrap_or_else(|e| panic!("{name}: {e}"));

        let want_pass = want["pass"].as_bool().unwrap();
        if got.pass != want_pass {
            abweichungen.push(format!("{name}: pass {} statt {}", got.pass, want_pass));
        }
        let want_hard = want["hard"].as_array().unwrap().len();
        let want_soft = want["soft"].as_array().unwrap().len();
        if got.hard.len() != want_hard {
            abweichungen.push(format!(
                "{name}: {} harte Verstoesse statt {} ({})",
                got.hard.len(),
                want_hard,
                got.hard
                    .iter()
                    .map(|v| v.rule.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if got.soft.len() != want_soft {
            abweichungen.push(format!(
                "{name}: {} weiche Warnungen statt {} ({})",
                got.soft.len(),
                want_soft,
                got.soft
                    .iter()
                    .map(|v| v.rule.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        // Die Korridorbreiten sind die empfindlichste Zahl im ganzen System: sie haengen an
        // Raster, Distanzfeld und Wegsuche zugleich. Stimmen sie, stimmt der Kern.
        let want_c = want["corridors"].as_array().unwrap();
        for (i, wc) in want_c.iter().enumerate() {
            let Some(gc) = got.metrics.corridors.get(i) else {
                abweichungen.push(format!("{name}: Korridor {i} fehlt"));
                continue;
            };
            let w = wc["widthCm"].as_i64().map(|v| v as i32);
            if gc.width_cm != w {
                abweichungen.push(format!(
                    "{name}: Route {} → {} misst {:?} cm statt {:?}",
                    gc.from, gc.to, gc.width_cm, w
                ));
            }
        }
        geprueft += 1;
    }

    assert!(geprueft == 10, "10 Layouts erwartet, {geprueft} geprueft");
    assert!(
        abweichungen.is_empty(),
        "Abweichungen zur Vorlage:\n  {}",
        abweichungen.join("\n  ")
    );
}

#[test]
fn der_katalog_ist_vollstaendig_uebernommen() {
    let (model, base) = live_or_skip!();
    let want = base["catalogue"].as_object().unwrap();
    let mut fehlend: Vec<&String> = want
        .keys()
        .filter(|k| !model.catalogue.contains_key(*k))
        .collect();
    fehlend.sort();
    assert!(fehlend.is_empty(), "im TOML-Inventar fehlen: {fehlend:?}");
    assert_eq!(
        model.catalogue.len(),
        want.len(),
        "Katalog hat {} Eintraege, die Vorlage {}",
        model.catalogue.len(),
        want.len()
    );
}

#[test]
fn die_masse_jedes_moebels_sind_unveraendert() {
    let (model, base) = live_or_skip!();
    let want = base["catalogue"].as_object().unwrap();
    let mut abw = Vec::new();
    for (id, w) in want {
        let Some(item) = model.catalogue.get(id) else {
            continue;
        };
        let wb = w["b"].as_i64().map(|v| v as i32);
        let wt = w["t"].as_i64().map(|v| v as i32);
        if item.b != wb || item.t != wt {
            abw.push(format!(
                "{id}: {:?}×{:?} statt {:?}×{:?}",
                item.b, item.t, wb, wt
            ));
        }
    }
    assert!(abw.is_empty(), "Masse weichen ab:\n  {}", abw.join("\n  "));
}
