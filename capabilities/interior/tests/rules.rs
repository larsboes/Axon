//! Die Wohnung deklariert ihre Regeln, und der Pruefer schlaegt sie nach.
//!
//! Bis 2026-08-31 tat er das nicht. `rules.toml` fuehrte `[[regeln]]` mit `id`, `schwere` und
//! `text`; `model::Rules` parste die Liste; **nichts las sie**. Die Schwere stand stattdessen an
//! allen 21 Ausgabestellen in `clearance.rs` als Literal, und der Modulkopf dort behauptete
//! trotzdem "Schwere folgt rules.toml". Eine Wohnung konnte R3 auf `hart` setzen und bekam
//! weiterhin eine Warnung.
//!
//! Jeder Test hier arbeitet auf einer KOPIE der Musterwohnung im Temp-Verzeichnis, weil er die
//! Regeln veraendern muss, um zu zeigen, dass sie wirken. Eine Datei zu editieren, die andere
//! Testbinaries lesen, waere die zweite Fassung des Fehlers, den D20 gerade geschlossen hat.

use interior::clearance::{check_layout, REGEL_IDS};
use interior::model::Model;
use std::path::{Path, PathBuf};
use std::sync::Once;

const FLAT: &str = "muster";

/// Eine Kopie der Fixture, in der eine Zeile ersetzt ist.
///
/// Jede Variante bekommt ihr eigenes Verzeichnis und ihren eigenen Wohnungsnamen, damit die
/// Varianten nebeneinander existieren koennen: `AXON_PERSONAL_ROOT` ist prozessweit, und ein
/// Test, der sie zwischen zwei Laeufen umsetzt, laeuft gegen jeden Nachbarn im selben Binary.
fn variante(name: &str, ersetze: &[(&str, &str)]) -> Model {
    let wurzel = einmalige_wurzel();
    let ziel = wurzel.join("data/interior/flats").join(name);
    if !ziel.exists() {
        let quelle = fixture().join("data/interior/flats").join(FLAT);
        kopiere(&quelle, &ziel);
        let regeln = ziel.join("rules.toml");
        let mut text = std::fs::read_to_string(&regeln).expect("rules.toml der Kopie");
        for (von, nach) in ersetze {
            assert!(
                text.contains(von),
                "{name}: `{von}` steht nicht in rules.toml — der Test prueft eine Zeile, die es nicht gibt"
            );
            text = text.replace(von, nach);
        }
        std::fs::write(&regeln, text).expect("rules.toml der Kopie schreiben");
    }
    Model::load(name).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/overlay")
}

/// Setzt die Umgebung genau einmal je Binary und fuellt das Inventar ueber denselben Import,
/// den `interior import` fuehrt — kein zweiter Weg, Zeilen anzulegen.
fn einmalige_wurzel() -> PathBuf {
    static ONCE: Once = Once::new();
    let wurzel = std::env::temp_dir().join(format!("interior-rules-{}", std::process::id()));
    ONCE.call_once(|| {
        let _ = std::fs::remove_dir_all(&wurzel);
        std::fs::create_dir_all(wurzel.join("data/interior/flats")).expect("Testwurzel");
        let db = wurzel.join("test.db");
        std::env::set_var("AXON_PERSONAL_ROOT", &wurzel);
        std::env::set_var("AXON_DB_PATH", &db);
        std::env::remove_var("AXON_INTERIOR_FLAT");
        let store = interior::store::Store::open(&db).expect("die Testdatenbank oeffnet");
        interior::import::inventory(&store, &fixture().join("data/interior/inventory"))
            .expect("das Musterinventar importiert");
    });
    wurzel
}

fn kopiere(von: &Path, nach: &Path) {
    std::fs::create_dir_all(nach).expect("Zielverzeichnis");
    for e in std::fs::read_dir(von).expect("Quellverzeichnis") {
        let e = e.expect("Eintrag");
        let ziel = nach.join(e.file_name());
        if e.file_type().expect("Typ").is_dir() {
            kopiere(&e.path(), &ziel);
        } else {
            std::fs::copy(e.path(), &ziel).expect("Datei kopieren");
        }
    }
}

/// Der Kern: dieselbe Geometrie, dieselbe Verletzung, **ein Wort in der Datei anders**.
///
/// `b-lichtkorridor` stellt ein zu hohes Stueck in den Lichtkorridor. R3 steht in der
/// Musterwohnung auf `weich`, also ist das eine Warnung und das Layout besteht. Steht R3 auf
/// `hart`, faellt dasselbe Layout durch. Aendert sich hier nichts, kommt die Schwere wieder
/// aus dem Code.
#[test]
fn die_schwere_einer_regel_kommt_aus_der_datei() {
    let weich = variante("r3-weich", &[]);
    let hart = variante(
        "r3-hart",
        &[(
            "id = \"R3\"\nschwere = \"weich\"",
            "id = \"R3\"\nschwere = \"hart\"",
        )],
    );

    let l = weich.load_layout("b-lichtkorridor").expect("Layout");
    let w = check_layout(&weich, &l).expect("Pruefung weich");
    assert!(w.pass, "R3 weich: das Layout besteht mit einer Warnung");
    assert_eq!(w.soft.iter().filter(|v| v.rule == "R3").count(), 1);
    assert_eq!(w.hard.iter().filter(|v| v.rule == "R3").count(), 0);

    let l = hart.load_layout("b-lichtkorridor").expect("Layout");
    let h = check_layout(&hart, &l).expect("Pruefung hart");
    assert!(
        !h.pass,
        "R3 hart: dasselbe Layout faellt durch — sonst steht die Schwere weiter im Code"
    );
    assert_eq!(h.hard.iter().filter(|v| v.rule == "R3").count(), 1);
    assert_eq!(h.soft.iter().filter(|v| v.rule == "R3").count(), 0);
}

/// Der Regeltext erreicht den Bericht.
///
/// Er stand seit jeher in `rules.toml` und kam in keinem Verdikt vor. `message` sagt, was
/// dieses Layout falsch macht; `text` sagt, welche Regel das ueberhaupt zu einer Regel macht.
#[test]
fn ein_verstoss_traegt_den_regeltext_der_wohnung() {
    let m = variante("text", &[]);
    let l = m.load_layout("c-terrassentuer").expect("Layout");
    let r = check_layout(&m, &l).expect("Pruefung");

    let v = r
        .hard
        .iter()
        .find(|v| v.rule == "R1")
        .expect("c-terrassentuer verletzt R1");
    assert_eq!(
        v.text.as_deref(),
        Some("Anlaufzone vor der gesamten Breite der Terrassentuer freihalten."),
        "der Text kommt woertlich aus rules.toml"
    );
}

/// Eine Invariante traegt keinen Regeltext, und das ist die Grenze zwischen den zwei Klassen.
///
/// Zwei Moebel koennen sich nicht ueberlappen, und keine Wohnung kann das erlauben. Sie
/// deklarieren zu lassen hiesse, jede Wohnung eine Invariante wiederholen zu lassen, die sie
/// nicht aendern kann — und die erste vergessene waere eine still abgeschaltete Pruefung.
#[test]
fn eine_invariante_hat_keinen_regeltext_und_braucht_keine_deklaration() {
    let m = variante("invariante", &[]);
    let l = m.load_layout("d-ausserhalb").expect("Layout");
    let r = check_layout(&m, &l).expect("Pruefung");

    let v = r
        .hard
        .iter()
        .find(|v| v.rule == "raumgrenze")
        .expect("d-ausserhalb stellt ein Moebel durch die Wand");
    assert!(
        v.text.is_none(),
        "`raumgrenze` ist Geometrie und keine Hausregel — es gibt keinen Text nachzuschlagen"
    );
    assert!(
        !REGEL_IDS.contains(&"raumgrenze"),
        "und es steht deshalb auch nicht in REGEL_IDS"
    );
}

/// Eine Kennung, die die Wohnung nicht fuehrt, ist ein FEHLER und kein textloser Verstoss.
///
/// Vorher haette der Pruefer `R1` ausgegeben und der Bericht haette eine Regel genannt, die
/// niemand nachschlagen kann. Die Fehlermeldung nennt, was stattdessen deklariert ist — der
/// gleiche Umgang, den B26a fuer einen unbekannten Wegpunkt eingefuehrt hat.
#[test]
fn eine_nicht_deklarierte_kennung_bricht_die_pruefung_ab() {
    let m = variante(
        "ohne-r1",
        &[(
            "[[regeln]]\nid = \"R1\"\nschwere = \"hart\"\ntext = \"Anlaufzone vor der gesamten Breite der Terrassentuer freihalten.\"\n",
            "",
        )],
    );
    let l = m.load_layout("c-terrassentuer").expect("Layout");
    let e = check_layout(&m, &l).expect_err("R1 fehlt, also gibt es nichts nachzuschlagen");
    let text = e.to_string();
    assert!(
        text.contains("R1"),
        "die Meldung nennt die fehlende Kennung: {text}"
    );
    assert!(
        text.contains("R2"),
        "und die, die es gibt, damit ein Tippfehler sofort sichtbar ist: {text}"
    );
}

/// Eine unbekannte Schwere ist kein stillschweigendes `weich`.
///
/// Eine Wohnung, die sich vertippt, bekaeme sonst eine Regel, die nie blockiert, und der
/// Bericht saehe aus wie einer ueber eine Wohnung, in der alles erlaubt ist.
#[test]
fn eine_unbekannte_schwere_ist_ein_fehler() {
    let m = variante(
        "schwere-tippfehler",
        &[(
            "id = \"R1\"\nschwere = \"hart\"",
            "id = \"R1\"\nschwere = \"haart\"",
        )],
    );
    let l = m.load_layout("c-terrassentuer").expect("Layout");
    let e = check_layout(&m, &l).expect_err("`haart` ist keine Schwere");
    assert!(e.to_string().contains("haart"), "{e}");
}

/// Was die Wohnung deklariert und diese Maschine nicht prueft, steht im Ergebnis.
///
/// Die reale Wohnung fuehrt R5 und R6 — Blendung am Schreibtisch, der Blick vom Eingang aufs
/// Bett. Beides sind Hausregeln, beides prueft hier niemand, und bis 2026-08-31 fielen sie
/// stumm heraus: ein Bericht ueber acht deklarierte Regeln sah aus wie einer ueber sechs
/// gepruefte. „Bestanden" heisst ab hier „bestanden, gemessen an den Regeln, die gemessen
/// wurden".
#[test]
fn deklarierte_regeln_ohne_pruefung_werden_gemeldet() {
    let m = variante("ungeprueft", &[]);
    let l = m.load_layout("a-frei").expect("Layout");
    let r = check_layout(&m, &l).expect("Pruefung");

    assert!(r.pass, "a-frei besteht");
    let offen: Vec<&str> = r.nicht_geprueft.iter().map(|u| u.rule.as_str()).collect();
    assert_eq!(
        offen,
        vec!["R5", "R6"],
        "ein bestandenes Layout sagt trotzdem, was an ihm nicht gemessen wurde"
    );
    assert!(
        r.nicht_geprueft[0].text.contains("Blendung"),
        "mit dem Text, damit der Bericht sagt, was ungeprueft blieb"
    );
}

/// Jede Kennung, die diese Maschine ausgeben kann, fuehrt die Musterwohnung auch.
///
/// Der Abgleich laeuft in beide Richtungen, und das ist die Richtung, die kein Layout prueft:
/// R2 und R4 feuert in der Fixture nichts, also faende erst die echte Wohnung eine fehlende
/// Deklaration — zur Laufzeit, an einem Verdikt.
#[test]
fn die_musterwohnung_deklariert_jede_regel_die_der_pruefer_kennt() {
    let m = variante("vollstaendig", &[]);
    for id in REGEL_IDS {
        m.rules
            .regel(id)
            .unwrap_or_else(|e| panic!("REGEL_IDS nennt `{id}`, die Musterwohnung nicht: {e}"));
    }
}
