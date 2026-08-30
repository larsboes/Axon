//! Das Inventar als Zeilen, in der einen geteilten SQLite-Datei.
//!
//! PRD Q58 (2026-08-30): **eine Item-Tabelle, zwei Konsumenten.** Ein Zelt und ein
//! Kleiderschrank sind dieselbe Zeilenform — ein Ding mit Massen, einem Preis, einer Herkunft
//! und einem Zustand. Was sich unterscheidet, ist die Platzierung, und die ist eine zweite
//! Tabelle, keine zweite Kopie. Der Tabellenname `interior_item` ist deshalb heute schon
//! falsch und die Tabelle richtig: wenn Ausruestung dazukommt, wird umbenannt, nicht geforkt.
//!
//! ## Was hier liegt und was nicht
//!
//! Q60 zieht die Grenze bei *hat die Zahl eine Begruendung, die mitwandern muss*. Ein Moebel
//! ist eine Zeile: Masse, Preis, welche Seite sich oeffnet. Ein Raum ist keine —
//! `room.toml` traegt datierte Korrekturkommentare, und drei davon sind das Protokoll eines
//! Bugs, der einen falschen Plan erzeugt hat. Dafuer hat eine Tabelle keine Spalte.
//!
//! Geometrie bleibt also Datei. Das Inventar wird Zeile.
//!
//! ## Drei Tabellen, und warum der Zustand eine eigene ist
//!
//! Ein Wunsch, der gekauft wird, und ein Moebel, das weggegeben wird, sind **zwei Zeilen und
//! nicht eine ueberschriebene**. Ein `state`-Feld auf dem Item wuerde beim ersten Kauf
//! vergessen, wann etwas ein Wunsch war — und genau diese Spanne ist das, was die Wunschliste
//! spaeter mit `finance` verbindet.

use crate::model::Seite;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// Alles, was ein Ding ausmacht — ob es schon da ist oder erst gewuenscht.
///
/// Die Feldnamen folgen den TOML-Dateien, aus denen die Zeilen stammen, statt sie zu
/// uebersetzen: ein zweites Vokabular fuer dieselbe Sache ist die teure Version dieses Fehlers.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Item {
    pub id: String,
    /// `piece` = ein Ding (besessen oder ein konkretes Produkt). `slot` = ein BEDARF mit
    /// Zielmassen und noch ohne Produkt. Der Unterschied ueberlebt den Import, weil er der
    /// Grund ist, warum eine Wunschliste mehr ist als eine Einkaufsliste.
    pub kind: Kind,
    pub label: String,
    pub b: Option<i32>,
    pub t: Option<i32>,
    pub h: Option<i32>,
    pub h_min: Option<i32>,
    pub b_aufgeklappt: Option<i32>,
    pub t_ausgeklappt: Option<i32>,
    pub laenge: Option<i32>,
    pub anzahl: Option<i32>,
    /// Benannte Zustaende eines Klappmoebels, z. B. `["zu", "ausgeklappt"]`.
    pub zustaende: Vec<String>,
    /// Welche Masse geschaetzt sind. Wandert unveraendert in jeden Pruefbericht.
    pub unsicher: Vec<String>,
    pub platzbedarf_zone: Option<i32>,
    pub platzbedarf_block: Option<i32>,
    /// In Cent, nicht Euro. `finance` rechnet in Cent, und die Wunschliste trifft dort auf ein
    /// Budget (PRD B29) — zwei Einheiten fuer denselben Betrag ist die Naht, an der das reisst.
    pub preis_cent: Option<i64>,
    pub kosten_min_cent: Option<i64>,
    pub kosten_max_cent: Option<i64>,
    pub link: Option<String>,
    pub artikelnummer: Option<String>,
    pub quelle: Option<String>,
    pub gemessen_am: Option<String>,
    /// `bring` / `weg` / … — was beim Umzug mit diesem Stueck passiert.
    pub mitnahme: Option<String>,
    /// Dringlichkeit eines Bedarfs: `pflicht`, `empfehlung`, `konzept`, `ersetzt`.
    pub prioritaet: Option<String>,
    /// Ein Slot, der aus einem anderen Eintrag hervorgeht.
    pub basiert_auf: Option<String>,
    /// Welche Eintraege dieses Produkt ueberfluessig macht. Eine Beziehung, als Liste
    /// gehalten und nicht als Tabelle, weil sie hoechstens zwei Eintraege lang ist und
    /// nichts darauf joint. Sobald B29 die Wunschliste auf das Budget trifft und daraus ein
    /// Join wird, wird es eine Tabelle.
    pub ersetzt: Vec<String>,
    pub varianten: Vec<String>,
    pub ziel: Option<String>,
    pub hinweis: Option<String>,
    pub begruendung: Option<String>,
    pub entscheidung_offen: Option<String>,

    // --- Was dieses Stueck an Platz verlangt (PRD Q61 / B26) ---
    //
    // Bis 2026-08-31 riet die Pruefung aus dem Namen: `bett*` war ein Bett, `schrank*` ein
    // Schrank, und mit der Vermutung kam jede Schwelle mit. Das ist einmal teuer danebengegangen
    // — `^couch` fing `couchtisch`, also wurde ein Couchtisch gegen die Regeln eines Sofas
    // geprueft, und gefunden wurde es erst, als ein echter Esstisch dazukam.
    //
    // Diese Felder sind der Ersatz. Sie stehen am STUECK, nicht an der Wohnung: `open_clear`
    // ist am eigenen Schrank gemessen und schlaegt die Faustregel, und wie viele Seiten ein
    // Bett braucht, ist eine Aussage ueber die Nutzung. Ein Feld, das ein Ding beschreibt, das
    // mir gehoert, hat nichts in einer Datei zu suchen, die eine Wohnung beschreibt, die ich
    // miete.
    //
    // Alle optional, und leer heisst: der Name entscheidet weiter. Ein Umschalten am selben Tag
    // fuer alle 42 Zeilen waere ein Stichtag, an dem sich Verdikte aendern, ohne dass jemand
    // die Zahlen dahinter geprueft hat.
    /// Welche Seite Tueren oder Schubladen braucht, in der Ausrichtung des Stuecks selbst.
    pub opens: Option<Seite>,
    /// Wie viel davor frei bleiben muss, in cm. Ohne `opens` gilt es fuer die beste Seite.
    pub open_clear: Option<i32>,
    /// Darf die sich oeffnende Seite an einer Wand liegen. `false` heisst: dort ist sie nutzlos.
    pub wall_ok: Option<bool>,
    /// Zweiter Zustand: Schlafsofa, Klapptisch. `dir` ist die Seite, `to` die Gesamttiefe
    /// AUSGEKLAPPT — nicht der Zuwachs, weil die Produktseite die Gesamttiefe nennt.
    pub expands_dir: Option<Seite>,
    pub expands_to: Option<i32>,
    /// Wie viele Seiten begehbar sein muessen. Ein Bett fuer eine Person braucht eine.
    pub access_sides: Option<i32>,
    /// Wie tief eine solche Seite sein muss, in cm.
    pub access_clear: Option<i32>,
}

impl Item {
    /// Wahr, wenn irgendein Mass geraten ist. Das Flag wandert in jeden Bericht, der auf
    /// diesem Moebel beruht — eine Schaetzung, die unterwegs zur Messung wird, ist der
    /// Fehler, gegen den dieses Feld existiert.
    pub fn is_uncertain(&self) -> bool {
        !self.unsicher.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Piece,
    Slot,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::Piece => "piece",
            Kind::Slot => "slot",
        }
    }
    fn parse(s: &str) -> Kind {
        if s == "slot" {
            Kind::Slot
        } else {
            Kind::Piece
        }
    }
}

/// Besitzt er es, will er es, oder ist es weg.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Owned,
    Wanted,
    Gone,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Owned => "owned",
            State::Wanted => "wanted",
            State::Gone => "gone",
        }
    }
    pub fn parse(s: &str) -> Option<State> {
        match s {
            "owned" => Some(State::Owned),
            "wanted" => Some(State::Wanted),
            "gone" => Some(State::Gone),
            _ => None,
        }
    }
}

/// Wo ein Stueck in einer Wohnung tatsaechlich steht.
///
/// **Nicht dasselbe wie ein Layout.** `flats/<id>/layouts/*.toml` sind Vorschlaege, ueber die
/// argumentiert wird — sie tragen datierte Begruendungen und gehoeren nach Q60 in Dateien.
/// Diese Tabelle haelt, wo etwas nach dem Einzug wirklich steht: eine Tatsache ohne Argument,
/// und damit eine Zeile.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Placement {
    pub item_id: String,
    pub flat: String,
    pub x: i32,
    pub y: i32,
    pub rot: i32,
}

pub struct Store {
    pool: axon_store::Pool,
    prefix: String,
}

type Fehler = Box<dyn std::error::Error>;

/// Der Praefix wird in DDL und jedes Statement interpoliert, also wird er geprueft statt
/// gebunden — dieselbe Vorsichtsmassnahme wie in `capabilities/trips/src/store.rs`.
fn validate_prefix(prefix: &str) -> Result<(), Fehler> {
    if !prefix
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
        || prefix.is_empty()
    {
        return Err("prefix must contain only ASCII letters, digits, or underscore".into());
    }
    Ok(())
}

impl Store {
    pub fn open(database_path: &Path) -> Result<Self, Fehler> {
        Self::open_with_prefix(database_path, "interior")
    }

    pub fn open_with_prefix(database_path: &Path, prefix: &str) -> Result<Self, Fehler> {
        validate_prefix(prefix)?;
        let pool = axon_store::open_pool(database_path, prefix, |conn| {
            Self::run_migration(conn, prefix)
        })?;
        Ok(Self {
            pool,
            prefix: prefix.to_string(),
        })
    }

    fn conn(&self) -> Result<axon_store::PooledClient, Fehler> {
        Ok(self.pool.get()?)
    }

    /// Eine Verbindung fuer eine Abfrage, die ueber den eigenen Praefix hinausgeht.
    ///
    /// Genau ein Konsument: `budget::monatssaldo` liest `finance_transaction_projection`. Das
    /// ist erlaubt und der Grund fuer die eine geteilte Datei; es ist nur nichts, das
    /// versehentlich passieren soll, deshalb hat es einen eigenen, benannten Weg.
    pub fn borrow_connection(&self) -> Result<axon_store::PooledClient, Fehler> {
        self.conn()
    }

    /// Die Tabellen, wie sie sind — nicht die Geschichte, die zu ihnen gefuehrt hat. Die Datei
    /// beginnt leer, also gibt es keine ALTER-Kette zu bewahren (libs/axon-store/README.md).
    fn run_migration(conn: &Connection, prefix: &str) -> Result<(), Fehler> {
        conn.execute_batch(&format!(
            "
            CREATE TABLE IF NOT EXISTS {prefix}_item (
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
                created_at         TEXT NOT NULL,
                updated_at         TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS {prefix}_item_state (
                id      INTEGER PRIMARY KEY AUTOINCREMENT,
                item_id TEXT NOT NULL REFERENCES {prefix}_item(id) ON DELETE CASCADE,
                state   TEXT NOT NULL CHECK (state IN ('owned','wanted','gone')),
                since   TEXT NOT NULL,
                note    TEXT
            );
            CREATE TABLE IF NOT EXISTS {prefix}_placement (
                id      INTEGER PRIMARY KEY AUTOINCREMENT,
                item_id TEXT NOT NULL REFERENCES {prefix}_item(id) ON DELETE CASCADE,
                flat    TEXT NOT NULL,
                x       INTEGER NOT NULL,
                y       INTEGER NOT NULL,
                rot     INTEGER NOT NULL DEFAULT 0,
                since   TEXT NOT NULL,
                UNIQUE (item_id, flat)
            );
            CREATE INDEX IF NOT EXISTS {prefix}_idx_state_item
                ON {prefix}_item_state(item_id, since DESC);
            CREATE INDEX IF NOT EXISTS {prefix}_idx_placement_flat
                ON {prefix}_placement(flat);
            ",
            prefix = prefix
        ))?;
        Ok(())
    }

    pub fn ping(&self) -> Result<(), Fehler> {
        let conn = self.conn()?;
        conn.query_row("SELECT 1", [], |row| row.get::<_, i64>(0))?;
        Ok(())
    }

    /// Anlegen oder aktualisieren. `created_at` ueberlebt ein Update — wann eine Zeile
    /// entstanden ist, ist eine andere Tatsache als wann sie zuletzt stimmte.
    pub fn upsert_item(&self, it: &Item) -> Result<(), Fehler> {
        let p = &self.prefix;
        let conn = self.conn()?;
        conn.execute(
            &format!(
                "INSERT INTO {p}_item (
                    id, kind, label, b, t, h, h_min, b_aufgeklappt, t_ausgeklappt, laenge,
                    anzahl, zustaende, unsicher, platzbedarf_zone, platzbedarf_block,
                    preis_cent, kosten_min_cent, kosten_max_cent, link, artikelnummer,
                    quelle, gemessen_am, mitnahme, prioritaet, basiert_auf, ersetzt,
                    varianten, ziel, hinweis, begruendung, entscheidung_offen,
                    opens, open_clear, wall_ok, expands_dir, expands_to,
                    access_sides, access_clear, created_at, updated_at
                 ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                    ?11, ?12, ?13, ?14, ?15,
                    ?16, ?17, ?18, ?19, ?20,
                    ?21, ?22, ?23, ?24, ?25, ?26,
                    ?27, ?28, ?29, ?30, ?31,
                    ?32, ?33, ?34, ?35, ?36, ?37, ?38, {now}, {now}
                 )
                 ON CONFLICT(id) DO UPDATE SET
                    kind=excluded.kind, label=excluded.label, b=excluded.b, t=excluded.t,
                    h=excluded.h, h_min=excluded.h_min,
                    b_aufgeklappt=excluded.b_aufgeklappt,
                    t_ausgeklappt=excluded.t_ausgeklappt, laenge=excluded.laenge,
                    anzahl=excluded.anzahl, zustaende=excluded.zustaende,
                    unsicher=excluded.unsicher,
                    platzbedarf_zone=excluded.platzbedarf_zone,
                    platzbedarf_block=excluded.platzbedarf_block,
                    preis_cent=excluded.preis_cent,
                    kosten_min_cent=excluded.kosten_min_cent,
                    kosten_max_cent=excluded.kosten_max_cent, link=excluded.link,
                    artikelnummer=excluded.artikelnummer, quelle=excluded.quelle,
                    gemessen_am=excluded.gemessen_am, mitnahme=excluded.mitnahme,
                    prioritaet=excluded.prioritaet, basiert_auf=excluded.basiert_auf,
                    ersetzt=excluded.ersetzt,
                    varianten=excluded.varianten, ziel=excluded.ziel,
                    hinweis=excluded.hinweis, begruendung=excluded.begruendung,
                    entscheidung_offen=excluded.entscheidung_offen,
                    opens=excluded.opens, open_clear=excluded.open_clear,
                    wall_ok=excluded.wall_ok, expands_dir=excluded.expands_dir,
                    expands_to=excluded.expands_to, access_sides=excluded.access_sides,
                    access_clear=excluded.access_clear,
                    updated_at={now}",
                p = p,
                now = axon_store::now_offset("'+0 seconds'")
            ),
            params![
                it.id,
                it.kind.as_str(),
                it.label,
                it.b,
                it.t,
                it.h,
                it.h_min,
                it.b_aufgeklappt,
                it.t_ausgeklappt,
                it.laenge,
                it.anzahl,
                serde_json::to_string(&it.zustaende)?,
                serde_json::to_string(&it.unsicher)?,
                it.platzbedarf_zone,
                it.platzbedarf_block,
                it.preis_cent,
                it.kosten_min_cent,
                it.kosten_max_cent,
                it.link,
                it.artikelnummer,
                it.quelle,
                it.gemessen_am,
                it.mitnahme,
                it.prioritaet,
                it.basiert_auf,
                serde_json::to_string(&it.ersetzt)?,
                serde_json::to_string(&it.varianten)?,
                it.ziel,
                it.hinweis,
                it.begruendung,
                it.entscheidung_offen,
                it.opens.map(|s| s.as_str()),
                it.open_clear,
                it.wall_ok,
                it.expands_dir.map(|s| s.as_str()),
                it.expands_to,
                it.access_sides,
                it.access_clear,
            ],
        )?;
        Ok(())
    }

    /// Einen Zustandswechsel festhalten. Schreibt NICHT, wenn der aktuelle Zustand schon
    /// derselbe ist: ein wiederholter Import darf keine Geschichte erfinden.
    pub fn record_state(
        &self,
        item_id: &str,
        state: State,
        note: Option<&str>,
    ) -> Result<bool, Fehler> {
        if self.current_state(item_id)? == Some(state) {
            return Ok(false);
        }
        let p = &self.prefix;
        let conn = self.conn()?;
        conn.execute(
            &format!(
                "INSERT INTO {p}_item_state (item_id, state, since, note)
                 VALUES (?1, ?2, {now}, ?3)",
                p = p,
                now = axon_store::now_offset("'+0 seconds'")
            ),
            params![item_id, state.as_str(), note],
        )?;
        Ok(true)
    }

    pub fn current_state(&self, item_id: &str) -> Result<Option<State>, Fehler> {
        let p = &self.prefix;
        let conn = self.conn()?;
        let raw: Option<String> = conn
            .query_row(
                &format!(
                    "SELECT state FROM {p}_item_state WHERE item_id = ?1
                     ORDER BY since DESC, id DESC LIMIT 1"
                ),
                params![item_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(raw.as_deref().and_then(State::parse))
    }

    pub fn place(&self, pl: &Placement) -> Result<(), Fehler> {
        let p = &self.prefix;
        let conn = self.conn()?;
        conn.execute(
            &format!(
                "INSERT INTO {p}_placement (item_id, flat, x, y, rot, since)
                 VALUES (?1, ?2, ?3, ?4, ?5, {now})
                 ON CONFLICT(item_id, flat) DO UPDATE SET
                    x=excluded.x, y=excluded.y, rot=excluded.rot, since={now}",
                p = p,
                now = axon_store::now_offset("'+0 seconds'")
            ),
            params![pl.item_id, pl.flat, pl.x, pl.y, pl.rot],
        )?;
        Ok(())
    }

    pub fn placements(&self, flat: &str) -> Result<Vec<Placement>, Fehler> {
        let p = &self.prefix;
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT item_id, flat, x, y, rot FROM {p}_placement WHERE flat = ?1 ORDER BY item_id"
        ))?;
        let rows = stmt.query_map(params![flat], |row| {
            Ok(Placement {
                item_id: row.get(0)?,
                flat: row.get(1)?,
                x: row.get(2)?,
                y: row.get(3)?,
                rot: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Jedes Item mit seinem aktuellen Zustand. Der Katalog, den die Pruefung liest.
    pub fn catalogue(&self) -> Result<BTreeMap<String, (Item, Option<State>)>, Fehler> {
        let p = &self.prefix;
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT i.id, i.kind, i.label, i.b, i.t, i.h, i.h_min, i.b_aufgeklappt,
                    i.t_ausgeklappt, i.laenge, i.anzahl, i.zustaende, i.unsicher,
                    i.platzbedarf_zone, i.platzbedarf_block, i.preis_cent, i.kosten_min_cent,
                    i.kosten_max_cent, i.link, i.artikelnummer, i.quelle, i.gemessen_am,
                    i.mitnahme, i.prioritaet, i.basiert_auf, i.ersetzt, i.varianten, i.ziel,
                    i.hinweis, i.begruendung, i.entscheidung_offen,
                    i.opens, i.open_clear, i.wall_ok, i.expands_dir, i.expands_to,
                    i.access_sides, i.access_clear,
                    (SELECT s.state FROM {p}_item_state s
                      WHERE s.item_id = i.id ORDER BY s.since DESC, s.id DESC LIMIT 1)
             FROM {p}_item i ORDER BY i.id"
        ))?;
        let rows = stmt.query_map([], |row| {
            let kind: String = row.get(1)?;
            let seite = |i: usize| -> rusqlite::Result<Option<Seite>> {
                Ok(row
                    .get::<_, Option<String>>(i)?
                    .as_deref()
                    .and_then(Seite::parse))
            };
            let state: Option<String> = row.get(38)?;
            Ok((
                Item {
                    id: row.get(0)?,
                    kind: Kind::parse(&kind),
                    label: row.get(2)?,
                    b: row.get(3)?,
                    t: row.get(4)?,
                    h: row.get(5)?,
                    h_min: row.get(6)?,
                    b_aufgeklappt: row.get(7)?,
                    t_ausgeklappt: row.get(8)?,
                    laenge: row.get(9)?,
                    anzahl: row.get(10)?,
                    zustaende: axon_store::json_column(row, 11)?,
                    unsicher: axon_store::json_column(row, 12)?,
                    platzbedarf_zone: row.get(13)?,
                    platzbedarf_block: row.get(14)?,
                    preis_cent: row.get(15)?,
                    kosten_min_cent: row.get(16)?,
                    kosten_max_cent: row.get(17)?,
                    link: row.get(18)?,
                    artikelnummer: row.get(19)?,
                    quelle: row.get(20)?,
                    gemessen_am: row.get(21)?,
                    mitnahme: row.get(22)?,
                    prioritaet: row.get(23)?,
                    basiert_auf: row.get(24)?,
                    ersetzt: axon_store::json_column(row, 25)?,
                    varianten: axon_store::json_column(row, 26)?,
                    ziel: row.get(27)?,
                    hinweis: row.get(28)?,
                    begruendung: row.get(29)?,
                    entscheidung_offen: row.get(30)?,
                    opens: seite(31)?,
                    open_clear: row.get(32)?,
                    wall_ok: row.get(33)?,
                    expands_dir: seite(34)?,
                    expands_to: row.get(35)?,
                    access_sides: row.get(36)?,
                    access_clear: row.get(37)?,
                },
                state.as_deref().and_then(State::parse),
            ))
        })?;
        let mut out = BTreeMap::new();
        for r in rows {
            let (item, state) = r?;
            out.insert(item.id.clone(), (item, state));
        }
        Ok(out)
    }
}
