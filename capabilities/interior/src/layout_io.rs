//! Ein Layout zurueckschreiben, ohne seine Begruendung zu verlieren.
//!
//! ## Warum das eine eigene Datei ist
//!
//! Layouts sind Dateien und keine Zeilen (PRD Q60, bestaetigt 2026-08-31). Die Regel dort ist
//! *hat die Zahl eine Begruendung, die mitwandern muss* — und bei einem Layout hat sie das:
//! `h-esstisch-offen` traegt drei Absaetze darueber, warum die Klappe im Ostschlauch nicht
//! aufgeht, und genau diese Absaetze sind der Grund, aus dem der PINNTORP verworfen wurde. Eine
//! Tabelle hat dafuer keine Spalte, und das Overlay ist ein Git-Repository: als Datei hat jede
//! Umstellung eine Geschichte und einen Diff.
//!
//! Daraus folgt die einzige Anforderung an diesen Schreiber: **die Prosa ueberlebt.** Ein
//! Serialisierer, der die Datei aus der Struktur neu erzeugt, wirft jeden Kommentar weg — und
//! damit das Wertvollste an ihr.
//!
//! ## Warum das ohne `toml_edit` geht
//!
//! Gemessen statt vermutet, 2026-08-31: in **keinem** der Layouts steht ein Kommentar hinter dem
//! ersten `[[item]]`, und die ganze Datei kennt fuenf Schluessel — `name`, und je Eintrag `ref`,
//! `x`, `y`, `rot`, `size`. Damit reicht ein Schnitt am ersten `[[item]]`: der Kopf bleibt Zeichen
//! fuer Zeichen stehen, die Eintraege werden neu geschrieben. Eine Abhaengigkeit fuer
//! Kommentarerhalt waere Aufwand fuer ein Problem, das die Dateien nicht haben.
//!
//! Sollte je ein Kommentar zwischen den Eintraegen stehen, geht er verloren — deshalb prueft
//! `pruefe_kopf_genuegt` das vorher und verweigert die Arbeit, statt still zu kuerzen.

use crate::model::{Layout, Model, ModelError, PlacedItem};
use std::path::Path;

/// Der Kopf einer Layoutdatei: alles vor dem ersten `[[item]]`.
fn kopf(text: &str) -> &str {
    match text.find("[[item]]") {
        Some(i) => &text[..i],
        None => text,
    }
}

/// Steht hinter dem ersten `[[item]]` noch ein Kommentar, waere er nach dem Schreiben weg.
///
/// Dann bricht der Schreiber ab. Eine Datei stillschweigend zu kuerzen ist genau der
/// Datenverlust, gegen den `deny_unknown_fields` im Import steht.
fn pruefe_kopf_genuegt(text: &str, pfad: &Path) -> Result<(), ModelError> {
    let Some(i) = text.find("[[item]]") else {
        return Ok(());
    };
    for zeile in text[i..].lines() {
        let z = zeile.trim();
        if z.starts_with('#') {
            return Err(ModelError::Missing(format!(
                "{}: hinter dem ersten [[item]] steht ein Kommentar ({z:?}). \
                 Dieser Schreiber erhaelt nur den Kopf und wuerde ihn verlieren — \
                 verschiebe ihn nach oben oder schreib die Datei von Hand",
                pfad.display()
            )));
        }
    }
    Ok(())
}

/// Die Eintraege als TOML, in der Reihenfolge, in der sie stehen.
fn eintraege(items: &[PlacedItem]) -> String {
    let mut s = String::new();
    for it in items {
        s.push_str("[[item]]\n");
        s.push_str(&format!("ref = \"{}\"\n", it.reference));
        s.push_str(&format!("x = {}\ny = {}\nrot = {}\n", it.x, it.y, it.rot));
        if let Some(sz) = it.size {
            s.push_str(&format!("size = [{}, {}]\n", sz[0], sz[1]));
        }
        if let Some(k) = &it.kind {
            s.push_str(&format!("kind = \"{k}\"\n"));
        }
        s.push('\n');
    }
    s
}

/// Ein bestehendes Layout mit neuen Positionen zurueckschreiben. Der Kopf bleibt.
pub fn update(model: &Model, id: &str, items: &[PlacedItem]) -> Result<(), ModelError> {
    let pfad = model.layouts_dir().join(format!("{id}.toml"));
    let text = std::fs::read_to_string(&pfad).map_err(|source| ModelError::Read {
        path: pfad.clone(),
        source,
    })?;
    pruefe_kopf_genuegt(&text, &pfad)?;
    let neu = format!("{}{}", kopf(&text), eintraege(items));
    std::fs::write(&pfad, neu).map_err(|source| ModelError::Read { path: pfad, source })
}

/// Ein neues Layout anlegen. Der Kopf sagt, woher es kommt.
///
/// Wortlaut wie bei `interior search --out`: ein Layout, das eine Maschine gesetzt hat, soll
/// sich nicht als handverlesenes ausgeben.
pub fn create(model: &Model, id: &str, layout: &Layout, notiz: &str) -> Result<(), ModelError> {
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        || id.is_empty()
    {
        return Err(ModelError::Missing(format!(
            "`{id}` ist kein Layoutname — erlaubt sind Buchstaben, Ziffern, - und _"
        )));
    }
    let pfad = model.layouts_dir().join(format!("{id}.toml"));
    if pfad.exists() {
        return Err(ModelError::Missing(format!(
            "`{id}` gibt es schon; ein bestehendes Layout wird nicht ueberschrieben"
        )));
    }
    let kopf = format!(
        "# {notiz}\n#\n\
         # Von einer Maschine gesetzt, nicht von Hand. Jede Position hat die volle\n\
         # Raeumungspruefung durchlaufen; was sie ergeben hat, steht im Verdikt daneben und\n\
         # nicht hier. Wer diese Aufstellung behalten will, schreibt darueber, WARUM — dieser\n\
         # Kopf ist der Platz dafuer und ueberlebt jedes spaetere Verschieben.\n\n\
         name = \"{}\"\n\n",
        layout.name.replace('"', "'")
    );
    std::fs::write(&pfad, format!("{kopf}{}", eintraege(&layout.items)))
        .map_err(|source| ModelError::Read { path: pfad, source })
}

#[cfg(test)]
mod tests {
    use super::{eintraege, kopf, pruefe_kopf_genuegt};
    use crate::model::PlacedItem;
    use std::path::Path;

    const DATEI: &str = "# Variante K — Bett quer an der Nordwand.\n\
                         #\n\
                         # Drei Absaetze Begruendung, die kein Serialisierer kennt.\n\n\
                         name = \"K\"\n\n\
                         [[item]]\nref = \"bett\"\nx = 0\ny = 0\nrot = 0\n\n";

    #[test]
    fn der_kopf_ueberlebt_zeichen_fuer_zeichen() {
        let k = kopf(DATEI);
        assert!(k.contains("Drei Absaetze Begruendung"));
        assert!(k.contains("name = \"K\""));
        assert!(!k.contains("[[item]]"));
    }

    #[test]
    fn die_eintraege_werden_neu_geschrieben() {
        let items = vec![PlacedItem {
            reference: "bett".into(),
            x: 120,
            y: 40,
            rot: 90,
            size: None,
            kind: None,
        }];
        let s = eintraege(&items);
        assert!(s.contains("ref = \"bett\""));
        assert!(s.contains("x = 120"));
        assert!(s.contains("rot = 90"));
        assert!(!s.contains("size"), "kein size, wenn keins gesetzt ist");
    }

    /// Ein Kommentar zwischen den Eintraegen bricht ab, statt still verloren zu gehen.
    #[test]
    fn ein_kommentar_hinter_den_eintraegen_verhindert_das_schreiben() {
        let mit =
            format!("{DATEI}# ein spaeter Gedanke\n[[item]]\nref = \"x\"\nx = 1\ny = 1\nrot = 0\n");
        let e = pruefe_kopf_genuegt(&mit, Path::new("k.toml"))
            .expect_err("der Kommentar wuerde verloren gehen");
        assert!(e.to_string().contains("spaeter Gedanke"), "{e}");
    }

    #[test]
    fn ohne_spaete_kommentare_darf_geschrieben_werden() {
        assert!(pruefe_kopf_genuegt(DATEI, Path::new("k.toml")).is_ok());
    }
}
