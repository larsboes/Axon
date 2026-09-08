//! Die Ausruestungsspalten, und der Umbau, der sie in eine bestehende Datei bringt (B51).
//!
//! Der Grund fuer diesen Test steht in der Geschichte: die Packliste wurde in der Nacht vom
//! 2026-09-03 gegen genau diese Spalten gebaut, fand sie nicht, und wurde vor dem Merge
//! zurueckgenommen (`815750c`), weil eine halbe Form, die einmal in der Live-Datei steht,
//! bleibt. Was hier geprueft wird, ist deshalb nicht "gibt es die Spalten", sondern: **eine
//! Datei mit dem alten CHECK ueberlebt den Umbau, mit ihren Zeilen und ihrer Geschichte.**

use interior::store::{Item, Kind, State, Store};
use rusqlite::Connection;
use std::path::Path;

/// Eine Datei in genau der Form, die vor B51 auf der Platte lag: der enge CHECK, keine der
/// sieben Spalten. Von Hand geschrieben und nicht von `Store::open` erzeugt — sonst pruefte
/// der Test den Umbau gegen die Tabelle, die er selbst gerade neu gebaut hat.
fn alte_datei(pfad: &Path) {
    let conn = Connection::open(pfad).unwrap();
    conn.execute_batch(
        "CREATE TABLE interior_item (
            id                 TEXT PRIMARY KEY,
            kind               TEXT NOT NULL CHECK (kind IN ('piece','slot')),
            label              TEXT NOT NULL,
            b                  INTEGER,
            t                  INTEGER,
            h                  INTEGER,
            h_min              INTEGER,
            b_aufgeklappt      INTEGER,
            t_ausgeklappt      INTEGER,
            laenge             INTEGER,
            anzahl             INTEGER,
            zustaende          TEXT NOT NULL DEFAULT '[]',
            unsicher           TEXT NOT NULL DEFAULT '[]',
            platzbedarf_zone   INTEGER,
            platzbedarf_block  INTEGER,
            preis_cent         INTEGER,
            kosten_min_cent    INTEGER,
            kosten_max_cent    INTEGER,
            link               TEXT,
            artikelnummer      TEXT,
            quelle             TEXT,
            gemessen_am        TEXT,
            mitnahme           TEXT,
            prioritaet         TEXT,
            basiert_auf        TEXT,
            ersetzt            TEXT NOT NULL DEFAULT '[]',
            varianten          TEXT NOT NULL DEFAULT '[]',
            ziel               TEXT,
            hinweis            TEXT,
            begruendung        TEXT,
            entscheidung_offen TEXT,
            opens              TEXT,
            open_clear         INTEGER,
            wall_ok            INTEGER,
            expands_dir        TEXT,
            expands_to         INTEGER,
            access_sides       INTEGER,
            access_clear       INTEGER,
            raumtrenner        INTEGER,
            zerlegbar          INTEGER,
            bild               TEXT,
            created_at         TEXT NOT NULL,
            updated_at         TEXT NOT NULL
         );
         CREATE TABLE interior_item_state (
            id      INTEGER PRIMARY KEY AUTOINCREMENT,
            item_id TEXT NOT NULL REFERENCES interior_item(id) ON DELETE CASCADE,
            state   TEXT NOT NULL CHECK (state IN ('owned','wanted','gone')),
            since   TEXT NOT NULL,
            note    TEXT
         );
         CREATE TABLE interior_placement (
            id      INTEGER PRIMARY KEY AUTOINCREMENT,
            item_id TEXT NOT NULL REFERENCES interior_item(id) ON DELETE CASCADE,
            flat    TEXT NOT NULL,
            x       INTEGER NOT NULL,
            y       INTEGER NOT NULL,
            rot     INTEGER NOT NULL DEFAULT 0,
            since   TEXT NOT NULL,
            UNIQUE (item_id, flat)
         );
         INSERT INTO interior_item (id, kind, label, b, t, h, created_at, updated_at)
             VALUES ('regal', 'piece', 'Regal', 80, 30, 200, '2026-01-01', '2026-01-01');
         INSERT INTO interior_item_state (item_id, state, since, note)
             VALUES ('regal', 'owned', '2026-01-02', 'gekauft');
         INSERT INTO interior_placement (item_id, flat, x, y, since)
             VALUES ('regal', 'wohnung', 10, 20, '2026-01-03');",
    )
    .unwrap();
}

fn tempdatei(name: &str) -> std::path::PathBuf {
    let pfad = std::env::temp_dir().join(format!("interior-gear-{name}-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&pfad);
    pfad
}

#[test]
fn der_umbau_haelt_zeilen_zustand_und_platzierung() {
    let pfad = tempdatei("umbau");
    alte_datei(&pfad);

    let store = Store::open(&pfad).unwrap();

    // Die Zeile ist noch da, mit ihren Massen.
    let (regal, zustand) = store.item("regal").unwrap().expect("Regal ueberlebt");
    assert_eq!(
        (regal.label.as_str(), regal.b, regal.h),
        ("Regal", Some(80), Some(200))
    );
    assert_eq!(zustand, Some(State::Owned));
    // Und die Kindtabellen auch — ein DROP mit eingeschalteten Fremdschluesseln haette sie
    // mitgenommen, was der eigentliche Grund fuer diesen Test ist.
    assert_eq!(store.placements("wohnung").unwrap().len(), 1);
    assert_eq!(store.state_history("regal").unwrap().len(), 1);

    let _ = std::fs::remove_file(&pfad);
}

#[test]
fn nach_dem_umbau_darf_ausruestung_in_die_tabelle() {
    let pfad = tempdatei("gear");
    alte_datei(&pfad);
    let store = Store::open(&pfad).unwrap();

    let zelt = Item {
        id: "zelt".into(),
        kind: Kind::Gear,
        label: "Zelt".into(),
        weight_g: Some(1_850),
        category: Some("schlafen".into()),
        packable: Some(true),
        waterproof: Some(true),
        quick_dry: Some(false),
        pack_location: Some("rucksack".into()),
        trip_types: vec!["hiking".into()],
        ..Item::default()
    };
    store.upsert_item(&zelt).unwrap();

    let (gelesen, _) = store.item("zelt").unwrap().expect("Zelt");
    assert_eq!(gelesen.kind, Kind::Gear);
    assert_eq!(gelesen.weight_g, Some(1_850));
    assert_eq!(gelesen.category.as_deref(), Some("schlafen"));
    assert_eq!(
        (gelesen.packable, gelesen.waterproof, gelesen.quick_dry),
        (Some(true), Some(true), Some(false))
    );
    assert_eq!(gelesen.pack_location.as_deref(), Some("rucksack"));
    assert_eq!(gelesen.trip_types, vec!["hiking".to_string()]);

    let _ = std::fs::remove_file(&pfad);
}

#[test]
fn ein_zweiter_start_baut_nicht_noch_einmal_um() {
    // Der Umbau liest den gespeicherten DDL-Text und laeuft nur, wenn `gear` fehlt. Ohne diese
    // Bedingung kopierte jeder Start die ganze Tabelle — teuer, und jedes Mal eine Gelegenheit,
    // sie zu verlieren.
    let pfad = tempdatei("idempotent");
    alte_datei(&pfad);
    let store = Store::open(&pfad).unwrap();
    store
        .upsert_item(&Item {
            id: "zelt".into(),
            kind: Kind::Gear,
            label: "Zelt".into(),
            ..Item::default()
        })
        .unwrap();
    drop(store);

    let store = Store::open(&pfad).unwrap();
    assert_eq!(store.item_count().unwrap(), 2);
    assert!(store.item("zelt").unwrap().is_some());

    let _ = std::fs::remove_file(&pfad);
}
