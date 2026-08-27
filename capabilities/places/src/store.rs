//! Persistence for the place registry, the permanent geocode cache, the
//! transaction links and the companion register (`README.md` schema section).
//!
//! Writes happen only under this store's own table prefix (ISA anti-claim A2).
//! Cross-capability reads live in `layers.rs` and `backfill.rs` as plain
//! SELECTs. PRD Q45 (2026-08-27) made those same-file joins across table
//! prefixes rather than across schemas; they are joins either way, which is the
//! property the shared instance existed for.

use std::path::Path;

use axon_store::QueryAll;
use rusqlite::{params, Connection, OptionalExtension, Row};

pub type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

pub struct PlacesStore {
    pool: axon_store::Pool,
    /// Prefixes this capability's tables in the one shared file (PRD Q45):
    /// `places` here means `places_places` and its three siblings.
    prefix: String,
}

/// A registry row. `external_ref` holds a stable foreign identity such as an
/// EVA code (`eva:8000207`) or an OSM id (`osm:node/123`), which is what makes
/// every backfill idempotent: the same source identity is the same place.
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub address: Option<String>,
    pub city: Option<String>,
    pub country_code: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub source: String,
    pub external_ref: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub status: String,
    pub response: Option<String>,
    pub place_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PersonPlaceRow {
    pub id: String,
    pub person: String,
    pub place_id: String,
    pub date_start: Option<String>,
    pub date_end: Option<String>,
    pub confidence_bp: i16,
    pub source: String,
    pub state: String,
}

/// The only two states the review route may write. `Proposed` is deliberately
/// absent: proposals are created by `propose_person_place`, which hardcodes it,
/// and nothing else in this crate can spell `confirmed` into the table
/// (ISA PLC-7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Review {
    Confirmed,
    Dismissed,
}

impl Review {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Confirmed => "confirmed",
            Self::Dismissed => "dismissed",
        }
    }
}

/// A table prefix reaches SQL by interpolation, because SQL has no bind
/// parameter for an identifier. Copied deliberately from `finance`/`trips`:
/// the validation is the reason interpolating it is safe. `pub(crate)` because
/// the cross-capability readers in `backfill.rs`/`layers.rs` take the source
/// prefix as a parameter (tests point them at scratch prefixes) and interpolate
/// it under the same rule. Same character rule it had as `validate_schema`.
pub(crate) fn validate_prefix(prefix: &str) -> Fallible<()> {
    let ok = !prefix.is_empty()
        && prefix.len() <= 63
        && prefix
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if ok {
        Ok(())
    } else {
        Err(format!("invalid table prefix: {prefix:?}").into())
    }
}

impl PlacesStore {
    pub fn open(database_path: &Path) -> Fallible<Self> {
        Self::open_with_prefix(database_path, "places")
    }

    pub fn open_with_prefix(database_path: &Path, prefix: &str) -> Fallible<Self> {
        validate_prefix(prefix)?;
        let pool = axon_store::open_pool(database_path, prefix, |conn| {
            Self::run_migration(conn, prefix)
        })?;
        Ok(Self {
            pool,
            prefix: prefix.to_string(),
        })
    }

    pub(crate) fn conn(&self) -> Fallible<axon_store::PooledClient> {
        Ok(self.pool.get()?)
    }

    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// The cheapest statement that proves this store can reach its database —
    /// what `/ready` promises, rather than mere liveness (#126).
    pub fn ping(&self) -> Fallible<()> {
        let conn = self.conn()?;
        conn.query_row("SELECT 1", [], |row| row.get::<_, i64>(0))?;
        Ok(())
    }

    /// The current shape of the four tables. `places` is declared before the two
    /// that reference it, because a batch executes in order.
    fn run_migration(conn: &Connection, prefix: &str) -> Fallible<()> {
        conn.execute_batch(&format!(
            "
            CREATE TABLE IF NOT EXISTS {prefix}_places (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                kind TEXT NOT NULL
                    CHECK (kind IN ('venue','city','station','address','region')),
                address TEXT,
                city TEXT,
                country_code TEXT,
                latitude REAL,
                longitude REAL,
                source TEXT NOT NULL,
                external_ref TEXT,
                created_at TEXT NOT NULL
            );
            -- Index names carry the prefix too: one file is one namespace now.
            CREATE UNIQUE INDEX IF NOT EXISTS idx_{prefix}_places_external_ref
                ON {prefix}_places(external_ref) WHERE external_ref IS NOT NULL;
            CREATE INDEX IF NOT EXISTS idx_{prefix}_places_kind
                ON {prefix}_places(kind, name);

            -- Permanent by design (README D3): a repeat query is served from
            -- here and never leaves the host again. No TTL column on purpose.
            --
            -- `response` was JSONB and is TEXT holding JSON here -- one of the two
            -- measured columns in the repo with no SQLite equivalent (PRD Q45).
            -- Nothing ever queried inside it: every reader took `response::text`
            -- and parsed it in Rust, so the JSONB was buying validation, not
            -- indexing. That validation now happens where the value is produced.
            CREATE TABLE IF NOT EXISTS {prefix}_geocode_cache (
                query_hash TEXT PRIMARY KEY,
                provider TEXT NOT NULL,
                query TEXT NOT NULL,
                response TEXT,
                place_id TEXT,
                status TEXT NOT NULL CHECK (status IN ('hit','miss','error')),
                fetched_at TEXT NOT NULL
            );

            -- source_id is the finance journal's SHA-256 candidate fingerprint
            -- (capabilities/finance/src/import.rs), the one identity that
            -- survives projection rebuilds (README D2). One link per
            -- transaction: the PRIMARY KEY makes a venue link and a later city
            -- guess mutually exclusive without any code having to check.
            CREATE TABLE IF NOT EXISTS {prefix}_transaction_places (
                source_id TEXT PRIMARY KEY,
                place_id TEXT NOT NULL REFERENCES {prefix}_places(id),
                precision TEXT NOT NULL CHECK (precision IN ('venue','city')),
                confidence_bp INTEGER NOT NULL,
                source TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_{prefix}_transaction_places_place
                ON {prefix}_transaction_places(place_id);

            -- The companion register, PRD 8.2 / README D4. C2: never seeded
            -- into axon_demo, and no write path here sets 'confirmed' except
            -- review_person_place (ISA PLC-7).
            CREATE TABLE IF NOT EXISTS {prefix}_person_places (
                id TEXT PRIMARY KEY,
                person TEXT NOT NULL,
                place_id TEXT NOT NULL REFERENCES {prefix}_places(id),
                date_start TEXT,
                date_end TEXT,
                confidence_bp INTEGER NOT NULL,
                source TEXT NOT NULL,
                state TEXT NOT NULL
                    CHECK (state IN ('proposed','confirmed','dismissed')),
                created_at TEXT NOT NULL,
                reviewed_at TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_{prefix}_person_places_state
                ON {prefix}_person_places(state, person);
            "
        ))?;
        Ok(())
    }

    /// Insert a place, or recognise one already registered. Identity is the
    /// stable id (and the unique `external_ref` behind it), so every backfill
    /// re-run is a counted no-op. Returns whether a row was created.
    pub fn upsert_place(&self, place: &Place, today: &str) -> Fallible<bool> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        let inserted = conn.execute(
            &format!(
                "INSERT INTO {prefix}_places
                    (id, name, kind, address, city, country_code, latitude, longitude,
                     source, external_ref, created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
                 ON CONFLICT (id) DO NOTHING"
            ),
            params![
                &place.id,
                &place.name,
                &place.kind,
                &place.address,
                &place.city,
                &place.country_code,
                &place.latitude,
                &place.longitude,
                &place.source,
                &place.external_ref,
                &today,
            ],
        )?;
        Ok(inserted == 1)
    }

    pub fn place(&self, id: &str) -> Fallible<Option<Place>> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        Ok(conn
            .query_row(
                &format!(
                    "SELECT id, name, kind, address, city, country_code, latitude,
                            longitude, source, external_ref
                     FROM {prefix}_places WHERE id = ?1"
                ),
                params![&id],
                row_to_place,
            )
            .optional()?)
    }

    pub fn place_by_external_ref(&self, external_ref: &str) -> Fallible<Option<Place>> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        Ok(conn
            .query_row(
                &format!(
                    "SELECT id, name, kind, address, city, country_code, latitude,
                            longitude, source, external_ref
                     FROM {prefix}_places WHERE external_ref = ?1"
                ),
                params![&external_ref],
                row_to_place,
            )
            .optional()?)
    }

    /// List/search the registry. Both filters optional; bounded so an
    /// unfiltered call stays an inventory rather than a dump.
    ///
    /// `LIKE` where Postgres had `ILIKE`: SQLite's LIKE ignores case already,
    /// but only over ASCII, so a needle typed with an upper-case umlaut misses
    /// a name spelled with a lower-case one.
    pub fn search_places(&self, q: Option<&str>, kind: Option<&str>) -> Fallible<Vec<Place>> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        let pattern = q.map(|q| format!("%{q}%"));
        Ok(conn.query_all(
            &format!(
                "SELECT id, name, kind, address, city, country_code, latitude,
                        longitude, source, external_ref
                 FROM {prefix}_places
                 WHERE (?1 IS NULL OR name LIKE ?1 OR city LIKE ?1)
                   AND (?2 IS NULL OR kind = ?2)
                 ORDER BY name, id
                 LIMIT 500"
            ),
            params![&pattern, &kind],
            row_to_place,
        )?)
    }

    /// Every place that carries a coordinate, for haversine matching in code —
    /// the no-PostGIS decision in `README.md` ("Deliberately not built").
    pub fn places_with_coordinates(&self) -> Fallible<Vec<Place>> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        Ok(conn.query_all(
            &format!(
                "SELECT id, name, kind, address, city, country_code, latitude,
                        longitude, source, external_ref
                 FROM {prefix}_places
                 WHERE latitude IS NOT NULL AND longitude IS NOT NULL"
            ),
            [],
            row_to_place,
        )?)
    }

    pub fn cache_get(&self, query_hash: &str) -> Fallible<Option<CacheEntry>> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        // No `response::text`: the column is TEXT, so the cast the JSONB column
        // needed has nothing left to do.
        Ok(conn
            .query_row(
                &format!(
                    "SELECT status, response, place_id
                     FROM {prefix}_geocode_cache WHERE query_hash = ?1"
                ),
                params![&query_hash],
                |row| {
                    Ok(CacheEntry {
                        status: row.get(0)?,
                        response: row.get(1)?,
                        place_id: row.get(2)?,
                    })
                },
            )
            .optional()?)
    }

    /// The cache is permanent: a hash already present is left untouched, so a
    /// second writer cannot turn a served answer back into an egress.
    #[allow(clippy::too_many_arguments)]
    pub fn cache_put(
        &self,
        query_hash: &str,
        provider: &str,
        query: &str,
        response: Option<&str>,
        place_id: Option<&str>,
        status: &str,
        fetched_at: &str,
    ) -> Fallible<()> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        // `$4::text::jsonb` loses its cast with the column type. That cast was
        // also the only thing rejecting a malformed body, so the caller's own
        // serializer is now the validator -- every writer holds a serde value.
        conn.execute(
            &format!(
                "INSERT INTO {prefix}_geocode_cache
                    (query_hash, provider, query, response, place_id, status, fetched_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)
                 ON CONFLICT (query_hash) DO NOTHING"
            ),
            params![
                &query_hash,
                &provider,
                &query,
                &response,
                &place_id,
                &status,
                &fetched_at,
            ],
        )?;
        Ok(())
    }

    /// Link a transaction to a place. Idempotent on `source_id`; a transaction
    /// already linked (at either precision) is left alone.
    pub fn link_transaction(
        &self,
        source_id: &str,
        place_id: &str,
        precision: &str,
        confidence_bp: i16,
        source: &str,
        today: &str,
    ) -> Fallible<bool> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        let inserted = conn.execute(
            &format!(
                "INSERT INTO {prefix}_transaction_places
                    (source_id, place_id, precision, confidence_bp, source, created_at)
                 VALUES (?1,?2,?3,?4,?5,?6)
                 ON CONFLICT (source_id) DO NOTHING"
            ),
            params![
                &source_id,
                &place_id,
                &precision,
                confidence_bp,
                &source,
                &today,
            ],
        )?;
        Ok(inserted == 1)
    }

    pub fn linked_source_ids(&self) -> Fallible<std::collections::HashSet<String>> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        Ok(conn
            .query_all(
                &format!("SELECT source_id FROM {prefix}_transaction_places"),
                [],
                |row| row.get::<_, String>(0),
            )?
            .into_iter()
            .collect())
    }

    /// Write a register proposal. The state is the literal `'proposed'` and
    /// nothing the caller passes can change that — derivation never writes a
    /// confirmed row (README D4, ISA PLC-7).
    #[allow(clippy::too_many_arguments)]
    pub fn propose_person_place(
        &self,
        id: &str,
        person: &str,
        place_id: &str,
        date_start: Option<&str>,
        date_end: Option<&str>,
        confidence_bp: i16,
        source: &str,
        today: &str,
    ) -> Fallible<bool> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        let inserted = conn.execute(
            &format!(
                "INSERT INTO {prefix}_person_places
                    (id, person, place_id, date_start, date_end, confidence_bp,
                     source, state, created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,'proposed',?8)
                 ON CONFLICT (id) DO NOTHING"
            ),
            params![
                &id,
                &person,
                &place_id,
                &date_start,
                &date_end,
                confidence_bp,
                &source,
                &today,
            ],
        )?;
        Ok(inserted == 1)
    }

    pub fn person_places_in_state(&self, state: &str) -> Fallible<Vec<PersonPlaceRow>> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        Ok(conn.query_all(
            &format!(
                "SELECT id, person, place_id, date_start, date_end, confidence_bp,
                        source, state
                 FROM {prefix}_person_places
                 WHERE state = ?1
                 ORDER BY person, id"
            ),
            params![&state],
            |row| {
                Ok(PersonPlaceRow {
                    id: row.get(0)?,
                    person: row.get(1)?,
                    place_id: row.get(2)?,
                    date_start: row.get(3)?,
                    date_end: row.get(4)?,
                    confidence_bp: row.get(5)?,
                    source: row.get(6)?,
                    state: row.get(7)?,
                })
            },
        )?)
    }

    /// The one write path that can produce `state = 'confirmed'`, reached only
    /// from the explicit confirm/dismiss routes (ISA PLC-7). Returns false when
    /// the id does not exist.
    pub fn review_person_place(&self, id: &str, review: Review, now: &str) -> Fallible<bool> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        Ok(conn.execute(
            &format!(
                "UPDATE {prefix}_person_places
                 SET state = ?2, reviewed_at = ?3
                 WHERE id = ?1"
            ),
            params![&id, review.as_str(), &now],
        )? == 1)
    }
}

fn row_to_place(row: &Row) -> rusqlite::Result<Place> {
    Ok(Place {
        id: row.get(0)?,
        name: row.get(1)?,
        kind: row.get(2)?,
        address: row.get(3)?,
        city: row.get(4)?,
        country_code: row.get(5)?,
        latitude: row.get(6)?,
        longitude: row.get(7)?,
        source: row.get(8)?,
        external_ref: row.get(9)?,
    })
}

/// A stable id from a stable source identity, so a re-run of any backfill on
/// any machine lands on the row it made before. Same purpose as finance's
/// path-derived subscription ids.
pub fn stable_id(prefix: &str, identity: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(identity.as_bytes());
    let digest = hash.finalize();
    let mut encoded = String::with_capacity(16);
    for byte in &digest[..8] {
        use std::fmt::Write;
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    format!("{prefix}_{encoded}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_table_prefix_that_could_carry_sql_is_refused() {
        assert!(validate_prefix("places").is_ok());
        assert!(validate_prefix("places_test_123").is_ok());
        assert!(validate_prefix("places; DROP TABLE places_places").is_err());
        assert!(validate_prefix("Places").is_err());
        assert!(validate_prefix("").is_err());
    }

    #[test]
    fn stable_ids_are_stable_and_distinct() {
        assert_eq!(
            stable_id("place", "eva:8000207"),
            stable_id("place", "eva:8000207")
        );
        assert_ne!(
            stable_id("place", "eva:8000207"),
            stable_id("place", "eva:1234567")
        );
        assert!(stable_id("place", "eva:8000207").starts_with("place_"));
    }
}

/// Database-backed tests, a temp file per test. `db_tests` is the one selector every
/// database-backed module in the workspace is named by: CI's hermetic job runs
/// `--skip db_tests::` and its store job runs `db_tests::`, so the module name IS the
/// suite membership. It was `pg_tests` until 2026-08-25 (PRD Q44) and `postgres_tests`
/// until PRD Q45 — the suite needs a temp file now, not a server.
#[cfg(test)]
pub(crate) mod db_tests {
    use super::*;

    /// A file per test, in a directory this process owns. It replaces the schema-drop
    /// guard: that guard existed because a leaked test schema reached a real
    /// `pg_dumpall`, and a temp file is neither shared nor backed up.
    pub(crate) fn test_database(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("places-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a writable temp directory");
        let path = dir.join(format!("{name}.db"));
        // The pid is recycled eventually; the file starts empty, which is what the
        // old DROP SCHEMA was buying.
        for tail in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{tail}", path.display()));
        }
        path
    }

    /// The store plus the path it opened, because the cross-capability readers in
    /// `layers.rs`/`backfill.rs` need to create neighbour tables in the same file.
    pub(crate) fn open_test_store(name: &str) -> (PlacesStore, std::path::PathBuf) {
        let path = test_database(name);
        let store = PlacesStore::open(&path)
            .unwrap_or_else(|e| panic!("could not open test store at {}: {e}", path.display()));
        (store, path)
    }

    #[test]
    fn ddl_is_idempotent_and_the_store_answers_its_own_ping() {
        let (store, path) = open_test_store("ddl");
        store.ping().expect("a live store answers its own ping");
        // A second migration over the same prefix must be a no-op, not an error.
        let conn = Connection::open(&path).unwrap();
        PlacesStore::run_migration(&conn, store.prefix()).expect("re-running DDL is a no-op");
    }

    /// The readiness handler turns exactly this into a 503, where the stateless
    /// liveness handler answers 200 (#126). It replaces "port 1 is unreachable":
    /// there is no port any more, so an unusable path is the failure a deployment
    /// can actually have.
    #[test]
    fn a_store_cannot_be_opened_against_an_unusable_path() {
        let blocker = std::env::temp_dir().join(format!("places-blocker-{}", std::process::id()));
        std::fs::write(&blocker, b"not a directory").unwrap();
        assert!(
            PlacesStore::open(&blocker.join("axon.db")).is_err(),
            "an unusable path opened anyway"
        );
    }

    /// The cache is permanent by design (README D3), and its `response` column
    /// carried JSON that Postgres validated as JSONB. TEXT does not validate, so
    /// the round trip is pinned here instead.
    #[test]
    fn the_geocode_cache_is_write_once_and_returns_its_json_body() {
        let (store, _path) = open_test_store("cache");
        assert!(store.cache_get("h1").unwrap().is_none());

        store
            .cache_put(
                "h1",
                "nominatim",
                "Bonn",
                Some(r#"{"lat":"50.7"}"#),
                None,
                "hit",
                "2026-08-28",
            )
            .unwrap();
        // A second write must not turn a served answer back into an egress.
        store
            .cache_put(
                "h1",
                "nominatim",
                "Bonn",
                Some(r#"{"lat":"0"}"#),
                None,
                "miss",
                "2026-08-28",
            )
            .unwrap();

        let entry = store.cache_get("h1").unwrap().unwrap();
        assert_eq!(entry.status, "hit");
        let body: serde_json::Value =
            serde_json::from_str(entry.response.as_deref().unwrap()).unwrap();
        assert_eq!(body["lat"], "50.7");
    }

    #[test]
    fn places_and_links_are_idempotent_by_stable_identity() {
        let (store, _path) = open_test_store("idem");
        let place = Place {
            id: stable_id("place", "eva:8000001"),
            name: "Synthetic Hbf".into(),
            kind: "station".into(),
            address: None,
            city: None,
            country_code: Some("DE".into()),
            latitude: Some(50.0),
            longitude: Some(7.0),
            source: "test".into(),
            external_ref: Some("eva:8000001".into()),
        };
        assert!(store.upsert_place(&place, "2026-08-25").unwrap());
        assert!(!store.upsert_place(&place, "2026-08-25").unwrap());
        assert_eq!(
            store
                .place_by_external_ref("eva:8000001")
                .unwrap()
                .unwrap()
                .id,
            place.id
        );
        assert!(store
            .link_transaction("fp-1", &place.id, "venue", 9000, "test", "2026-08-25")
            .unwrap());
        assert!(!store
            .link_transaction("fp-1", &place.id, "city", 6000, "test", "2026-08-25")
            .unwrap());
        assert_eq!(store.linked_source_ids().unwrap().len(), 1);
    }

    #[test]
    fn a_proposal_is_born_proposed_and_only_review_confirms_it() {
        let (store, _path) = open_test_store("register");
        let place = Place {
            id: stable_id("place", "test:city"),
            name: "Synthetic City".into(),
            kind: "city".into(),
            address: None,
            city: None,
            country_code: None,
            latitude: Some(48.0),
            longitude: Some(16.0),
            source: "test".into(),
            external_ref: None,
        };
        store.upsert_place(&place, "2026-08-25").unwrap();
        assert!(store
            .propose_person_place(
                "pp_test",
                "Synthetic Person",
                &place.id,
                None,
                None,
                5000,
                "vault-frontmatter",
                "2026-08-25",
            )
            .unwrap());
        // Idempotent by id.
        assert!(!store
            .propose_person_place(
                "pp_test",
                "Synthetic Person",
                &place.id,
                None,
                None,
                5000,
                "vault-frontmatter",
                "2026-08-25",
            )
            .unwrap());
        let proposed = store.person_places_in_state("proposed").unwrap();
        assert_eq!(proposed.len(), 1);
        assert_eq!(proposed[0].state, "proposed");

        assert!(store
            .review_person_place("pp_test", Review::Confirmed, "2026-08-25")
            .unwrap());
        assert_eq!(store.person_places_in_state("confirmed").unwrap().len(), 1);
        assert!(!store
            .review_person_place("pp_missing", Review::Dismissed, "2026-08-25")
            .unwrap());
    }
}
