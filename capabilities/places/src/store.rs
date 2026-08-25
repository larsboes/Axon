//! Persistence for the place registry, the permanent geocode cache, the
//! transaction links and the companion register (`README.md` schema section).
//!
//! Writes happen only inside this store's own schema (ISA anti-claim A2).
//! Cross-schema reads live in `layers.rs` and `backfill.rs` as plain SELECTs,
//! the correlation-join usage `capabilities/postgres/README.md` blessed.

use postgres::{Client, Row};

pub type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

pub struct PlacesStore {
    pool: axon_store::Pool,
    schema: String,
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

/// A schema name reaches SQL by interpolation, because Postgres has no bind
/// parameter for an identifier. Copied deliberately from `finance`/`trips`:
/// the validation is the reason interpolating it is safe. `pub(crate)` because
/// the cross-schema readers in `backfill.rs`/`layers.rs` take the source
/// schema as a parameter (tests point them at scratch schemas) and interpolate
/// it under the same rule.
pub(crate) fn validate_schema(schema: &str) -> Fallible<()> {
    let ok = !schema.is_empty()
        && schema.len() <= 63
        && schema
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if ok {
        Ok(())
    } else {
        Err(format!("invalid schema name: {schema:?}").into())
    }
}

impl PlacesStore {
    pub fn open(database_url: &str) -> Fallible<Self> {
        Self::open_in_schema(database_url, "places")
    }

    pub fn open_in_schema(database_url: &str, schema: &str) -> Fallible<Self> {
        validate_schema(schema)?;
        let pool = axon_store::open_pool(database_url, schema, |conn| {
            Self::run_migration(conn, schema)
        })?;
        Ok(Self {
            pool,
            schema: schema.to_string(),
        })
    }

    pub(crate) fn conn(&self) -> Fallible<axon_store::PooledClient> {
        Ok(self.pool.get()?)
    }

    pub fn schema(&self) -> &str {
        &self.schema
    }

    /// The cheapest statement that proves this store can reach its database —
    /// what `/ready` promises, rather than mere liveness (#126).
    pub fn ping(&self) -> Fallible<()> {
        let mut conn = self.conn()?;
        conn.query_one("SELECT 1", &[])?;
        Ok(())
    }

    fn run_migration(conn: &mut Client, schema: &str) -> Fallible<()> {
        conn.batch_execute(&format!(
            "
            CREATE SCHEMA IF NOT EXISTS {schema};

            CREATE TABLE IF NOT EXISTS {schema}.places (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                kind TEXT NOT NULL
                    CHECK (kind IN ('venue','city','station','address','region')),
                address TEXT,
                city TEXT,
                country_code TEXT,
                latitude DOUBLE PRECISION,
                longitude DOUBLE PRECISION,
                source TEXT NOT NULL,
                external_ref TEXT,
                created_at TEXT NOT NULL
            );
            CREATE UNIQUE INDEX IF NOT EXISTS idx_places_external_ref
                ON {schema}.places(external_ref) WHERE external_ref IS NOT NULL;
            CREATE INDEX IF NOT EXISTS idx_places_kind
                ON {schema}.places(kind, name);

            -- Permanent by design (README D3): a repeat query is served from
            -- here and never leaves the host again. No TTL column on purpose.
            CREATE TABLE IF NOT EXISTS {schema}.geocode_cache (
                query_hash TEXT PRIMARY KEY,
                provider TEXT NOT NULL,
                query TEXT NOT NULL,
                response JSONB,
                place_id TEXT,
                status TEXT NOT NULL CHECK (status IN ('hit','miss','error')),
                fetched_at TEXT NOT NULL
            );

            -- source_id is the finance journal's SHA-256 candidate fingerprint
            -- (capabilities/finance/src/import.rs), the one identity that
            -- survives projection rebuilds (README D2). One link per
            -- transaction: the PRIMARY KEY makes a venue link and a later city
            -- guess mutually exclusive without any code having to check.
            CREATE TABLE IF NOT EXISTS {schema}.transaction_places (
                source_id TEXT PRIMARY KEY,
                place_id TEXT NOT NULL REFERENCES {schema}.places(id),
                precision TEXT NOT NULL CHECK (precision IN ('venue','city')),
                confidence_bp SMALLINT NOT NULL,
                source TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_transaction_places_place
                ON {schema}.transaction_places(place_id);

            -- The companion register, PRD 8.2 / README D4. C2: never seeded
            -- into axon_demo, and no write path here sets 'confirmed' except
            -- review_person_place (ISA PLC-7).
            CREATE TABLE IF NOT EXISTS {schema}.person_places (
                id TEXT PRIMARY KEY,
                person TEXT NOT NULL,
                place_id TEXT NOT NULL REFERENCES {schema}.places(id),
                date_start TEXT,
                date_end TEXT,
                confidence_bp SMALLINT NOT NULL,
                source TEXT NOT NULL,
                state TEXT NOT NULL
                    CHECK (state IN ('proposed','confirmed','dismissed')),
                created_at TEXT NOT NULL,
                reviewed_at TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_person_places_state
                ON {schema}.person_places(state, person);
            "
        ))?;
        Ok(())
    }

    /// Insert a place, or recognise one already registered. Identity is the
    /// stable id (and the unique `external_ref` behind it), so every backfill
    /// re-run is a counted no-op. Returns whether a row was created.
    pub fn upsert_place(&self, place: &Place, today: &str) -> Fallible<bool> {
        let schema = &self.schema;
        let mut conn = self.conn()?;
        let inserted = conn.execute(
            &format!(
                "INSERT INTO {schema}.places
                    (id, name, kind, address, city, country_code, latitude, longitude,
                     source, external_ref, created_at)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
                 ON CONFLICT (id) DO NOTHING"
            ),
            &[
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
        let schema = &self.schema;
        let mut conn = self.conn()?;
        Ok(conn
            .query_opt(
                &format!(
                    "SELECT id, name, kind, address, city, country_code, latitude,
                            longitude, source, external_ref
                     FROM {schema}.places WHERE id = $1"
                ),
                &[&id],
            )?
            .map(|row| row_to_place(&row)))
    }

    pub fn place_by_external_ref(&self, external_ref: &str) -> Fallible<Option<Place>> {
        let schema = &self.schema;
        let mut conn = self.conn()?;
        Ok(conn
            .query_opt(
                &format!(
                    "SELECT id, name, kind, address, city, country_code, latitude,
                            longitude, source, external_ref
                     FROM {schema}.places WHERE external_ref = $1"
                ),
                &[&external_ref],
            )?
            .map(|row| row_to_place(&row)))
    }

    /// List/search the registry. Both filters optional; bounded so an
    /// unfiltered call stays an inventory rather than a dump.
    pub fn search_places(&self, q: Option<&str>, kind: Option<&str>) -> Fallible<Vec<Place>> {
        let schema = &self.schema;
        let mut conn = self.conn()?;
        let pattern = q.map(|q| format!("%{q}%"));
        Ok(conn
            .query(
                &format!(
                    "SELECT id, name, kind, address, city, country_code, latitude,
                            longitude, source, external_ref
                     FROM {schema}.places
                     WHERE ($1::text IS NULL OR name ILIKE $1 OR city ILIKE $1)
                       AND ($2::text IS NULL OR kind = $2)
                     ORDER BY name, id
                     LIMIT 500"
                ),
                &[&pattern, &kind],
            )?
            .iter()
            .map(row_to_place)
            .collect())
    }

    /// Every place that carries a coordinate, for haversine matching in code —
    /// the no-PostGIS decision in `README.md` ("Deliberately not built").
    pub fn places_with_coordinates(&self) -> Fallible<Vec<Place>> {
        let schema = &self.schema;
        let mut conn = self.conn()?;
        Ok(conn
            .query(
                &format!(
                    "SELECT id, name, kind, address, city, country_code, latitude,
                            longitude, source, external_ref
                     FROM {schema}.places
                     WHERE latitude IS NOT NULL AND longitude IS NOT NULL"
                ),
                &[],
            )?
            .iter()
            .map(row_to_place)
            .collect())
    }

    pub fn cache_get(&self, query_hash: &str) -> Fallible<Option<CacheEntry>> {
        let schema = &self.schema;
        let mut conn = self.conn()?;
        Ok(conn
            .query_opt(
                &format!(
                    "SELECT status, response::text, place_id
                     FROM {schema}.geocode_cache WHERE query_hash = $1"
                ),
                &[&query_hash],
            )?
            .map(|row| CacheEntry {
                status: row.get(0),
                response: row.get(1),
                place_id: row.get(2),
            }))
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
        let schema = &self.schema;
        let mut conn = self.conn()?;
        conn.execute(
            &format!(
                "INSERT INTO {schema}.geocode_cache
                    (query_hash, provider, query, response, place_id, status, fetched_at)
                 VALUES ($1,$2,$3,$4::text::jsonb,$5,$6,$7)
                 ON CONFLICT (query_hash) DO NOTHING"
            ),
            &[
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
        let schema = &self.schema;
        let mut conn = self.conn()?;
        let inserted = conn.execute(
            &format!(
                "INSERT INTO {schema}.transaction_places
                    (source_id, place_id, precision, confidence_bp, source, created_at)
                 VALUES ($1,$2,$3,$4,$5,$6)
                 ON CONFLICT (source_id) DO NOTHING"
            ),
            &[
                &source_id,
                &place_id,
                &precision,
                &confidence_bp,
                &source,
                &today,
            ],
        )?;
        Ok(inserted == 1)
    }

    pub fn linked_source_ids(&self) -> Fallible<std::collections::HashSet<String>> {
        let schema = &self.schema;
        let mut conn = self.conn()?;
        Ok(conn
            .query(
                &format!("SELECT source_id FROM {schema}.transaction_places"),
                &[],
            )?
            .iter()
            .map(|row| row.get(0))
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
        let schema = &self.schema;
        let mut conn = self.conn()?;
        let inserted = conn.execute(
            &format!(
                "INSERT INTO {schema}.person_places
                    (id, person, place_id, date_start, date_end, confidence_bp,
                     source, state, created_at)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,'proposed',$8)
                 ON CONFLICT (id) DO NOTHING"
            ),
            &[
                &id,
                &person,
                &place_id,
                &date_start,
                &date_end,
                &confidence_bp,
                &source,
                &today,
            ],
        )?;
        Ok(inserted == 1)
    }

    pub fn person_places_in_state(&self, state: &str) -> Fallible<Vec<PersonPlaceRow>> {
        let schema = &self.schema;
        let mut conn = self.conn()?;
        Ok(conn
            .query(
                &format!(
                    "SELECT id, person, place_id, date_start, date_end, confidence_bp,
                            source, state
                     FROM {schema}.person_places
                     WHERE state = $1
                     ORDER BY person, id"
                ),
                &[&state],
            )?
            .iter()
            .map(|row| PersonPlaceRow {
                id: row.get(0),
                person: row.get(1),
                place_id: row.get(2),
                date_start: row.get(3),
                date_end: row.get(4),
                confidence_bp: row.get(5),
                source: row.get(6),
                state: row.get(7),
            })
            .collect())
    }

    /// The one write path that can produce `state = 'confirmed'`, reached only
    /// from the explicit confirm/dismiss routes (ISA PLC-7). Returns false when
    /// the id does not exist.
    pub fn review_person_place(&self, id: &str, review: Review, now: &str) -> Fallible<bool> {
        let schema = &self.schema;
        let mut conn = self.conn()?;
        Ok(conn.execute(
            &format!(
                "UPDATE {schema}.person_places
                 SET state = $2, reviewed_at = $3
                 WHERE id = $1"
            ),
            &[&id, &review.as_str(), &now],
        )? == 1)
    }
}

fn row_to_place(row: &Row) -> Place {
    Place {
        id: row.get(0),
        name: row.get(1),
        kind: row.get(2),
        address: row.get(3),
        city: row.get(4),
        country_code: row.get(5),
        latitude: row.get(6),
        longitude: row.get(7),
        source: row.get(8),
        external_ref: row.get(9),
    }
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
    fn a_schema_name_that_could_carry_sql_is_refused() {
        assert!(validate_schema("places").is_ok());
        assert!(validate_schema("places_test_123").is_ok());
        assert!(validate_schema("places; DROP SCHEMA public").is_err());
        assert!(validate_schema("Places").is_err());
        assert!(validate_schema("").is_err());
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

/// Postgres-backed tests, isolated schema-per-test-per-process exactly like
/// `capabilities/tasks/src/store.rs` and transit's store tests. Named `pg_tests`
/// so the hermetic Bazel target can `--skip` them and the tagged
/// `postgres-integration` target can select them.
#[cfg(test)]
pub(crate) mod pg_tests {
    use super::*;
    use postgres::NoTls;

    pub(crate) fn test_database_url() -> String {
        static URL: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        URL.get_or_init(|| {
            std::env::var("PLACES_TEST_DATABASE_URL")
                .unwrap_or_else(|_| crate::config::Config::load().database_url)
        })
        .clone()
    }

    /// Drops the schema on the way out, including on unwind — the guard shape
    /// transit's store tests adopted after leaked test schemas reached a real
    /// pg_dumpall.
    pub(crate) struct TestSchema(pub String);

    impl Drop for TestSchema {
        fn drop(&mut self) {
            if let Ok(mut client) = Client::connect(&test_database_url(), NoTls) {
                let _ = client.batch_execute(&format!("DROP SCHEMA IF EXISTS {} CASCADE", self.0));
            }
        }
    }

    pub(crate) fn open_test_store(name: &str) -> (PlacesStore, TestSchema) {
        let schema = format!("places_test_{name}_{}", std::process::id());
        let store =
            PlacesStore::open_in_schema(&test_database_url(), &schema).unwrap_or_else(|e| {
                panic!(
                    "could not open test store (is capabilities/postgres running? see README): {e}"
                )
            });
        (store, TestSchema(schema))
    }

    #[test]
    fn ddl_is_idempotent_and_the_store_answers_its_own_ping() {
        let (store, schema) = open_test_store("ddl");
        store.ping().expect("a live store answers its own ping");
        // A second migration over the same schema must be a no-op, not an error.
        let mut conn = Client::connect(&test_database_url(), NoTls).unwrap();
        PlacesStore::run_migration(&mut conn, &schema.0).expect("re-running DDL is a no-op");
    }

    #[test]
    fn places_and_links_are_idempotent_by_stable_identity() {
        let (store, _schema) = open_test_store("idem");
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
        let (store, _schema) = open_test_store("register");
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
