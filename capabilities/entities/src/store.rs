//! The entity tables, in the shared Axon database under the `entities` prefix.
//!
//! Five tables. `fields` is the registry of what each kind has, built-ins seeded on open.
//! `entities` is one row per person, organisation, place or self. `values` holds the
//! current value of each field with its source. `facts` holds dated place facts.
//! `external` maps an entity to its id in another system (Google, Obsidian), so a sync
//! updates rather than duplicates.

use std::collections::BTreeMap;
use std::fmt::Display;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde_json::Value;

use crate::model::{
    builtin_fields, check_field, check_one_of, check_value, is_day, Entity, Fact, FieldDef,
    FieldValue, FACT_PREDICATES, KINDS, SOURCES,
};

pub const PREFIX: &str = "entities";

/// Why a store call did not do what was asked.
#[derive(Debug, PartialEq)]
pub enum StoreError {
    /// The request broke a rule; the message names it.
    Invalid(String),
    NotFound(String),
    /// The entity changed since the caller read it.
    Stale {
        current_revision: u32,
    },
    Db(String),
}

impl Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(m) | Self::NotFound(m) | Self::Db(m) => f.write_str(m),
            Self::Stale { current_revision } => {
                write!(
                    f,
                    "the entity changed since you read it (now revision {current_revision})"
                )
            }
        }
    }
}

type Result<T> = std::result::Result<T, StoreError>;

fn db<E: Display>(error: E) -> StoreError {
    StoreError::Db(error.to_string())
}

/// A new id: a time part and a process-wide counter, so two calls in one nanosecond differ.
fn new_id(prefix: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!(
        "{prefix}:{nanos:x}{:04x}",
        COUNTER.fetch_add(1, Ordering::Relaxed) & 0xffff
    )
}

fn now() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
        .to_string()
}

/// A new dated fact, before it has an id.
#[derive(Debug, Clone, Default)]
pub struct NewFact {
    pub predicate: String,
    pub place: String,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub valid_from: Option<String>,
    pub valid_to: Option<String>,
    pub note: Option<String>,
    pub source: String,
}

/// The changes one PATCH asks for. `values` maps a key to a new value, or to `null` to clear.
#[derive(Debug, Clone, Default)]
pub struct Patch {
    pub name: Option<String>,
    /// `Some(None)` clears the note link.
    pub note_ref: Option<Option<String>>,
    pub values: BTreeMap<String, Value>,
    pub source: String,
    pub expected_revision: Option<u32>,
}

pub struct EntitiesStore {
    pool: axon_store::Pool,
    prefix: String,
}

impl EntitiesStore {
    pub fn open(database_path: &Path) -> std::result::Result<Self, Box<dyn std::error::Error>> {
        let pool =
            axon_store::open_pool(database_path, PREFIX, |conn| Self::migrate(conn, PREFIX))?;
        Ok(Self {
            pool,
            prefix: PREFIX.to_string(),
        })
    }

    fn conn(&self) -> Result<axon_store::PooledClient> {
        self.pool.get().map_err(db)
    }

    pub fn ping(&self) -> Result<()> {
        self.conn()?
            .query_row("SELECT 1", [], |row| row.get::<_, i64>(0))
            .map(|_| ())
            .map_err(db)
    }

    fn migrate(
        conn: &Connection,
        prefix: &str,
    ) -> std::result::Result<(), Box<dyn std::error::Error>> {
        conn.execute_batch(&format!(
            "
            CREATE TABLE IF NOT EXISTS {prefix}_fields (
                kind        TEXT NOT NULL,
                key         TEXT NOT NULL,
                label       TEXT NOT NULL,
                field_type  TEXT NOT NULL,
                options     TEXT NOT NULL DEFAULT '[]',
                data_class  TEXT NOT NULL,
                builtin     INTEGER NOT NULL DEFAULT 0,
                created_at  TEXT NOT NULL,
                PRIMARY KEY (kind, key)
            );
            CREATE TABLE IF NOT EXISTS {prefix}_entities (
                id          TEXT PRIMARY KEY,
                kind        TEXT NOT NULL,
                name        TEXT NOT NULL,
                note_ref    TEXT,
                revision    INTEGER NOT NULL,
                created_at  TEXT NOT NULL,
                updated_at  TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_{prefix}_entities_kind ON {prefix}_entities(kind, name);
            CREATE TABLE IF NOT EXISTS {prefix}_values (
                entity_id   TEXT NOT NULL REFERENCES {prefix}_entities(id) ON DELETE CASCADE,
                key         TEXT NOT NULL,
                value       TEXT NOT NULL,
                source      TEXT NOT NULL,
                updated_at  TEXT NOT NULL,
                PRIMARY KEY (entity_id, key)
            );
            CREATE TABLE IF NOT EXISTS {prefix}_facts (
                id          TEXT PRIMARY KEY,
                entity_id   TEXT NOT NULL REFERENCES {prefix}_entities(id) ON DELETE CASCADE,
                predicate   TEXT NOT NULL,
                place       TEXT NOT NULL,
                latitude    REAL,
                longitude   REAL,
                valid_from  TEXT,
                valid_to    TEXT,
                note        TEXT,
                source      TEXT NOT NULL,
                created_at  TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_{prefix}_facts_entity ON {prefix}_facts(entity_id);
            CREATE TABLE IF NOT EXISTS {prefix}_external (
                system      TEXT NOT NULL,
                external_id TEXT NOT NULL,
                entity_id   TEXT NOT NULL REFERENCES {prefix}_entities(id) ON DELETE CASCADE,
                etag        TEXT,
                synced_at   TEXT NOT NULL,
                PRIMARY KEY (system, external_id)
            );
            "
        ))?;
        // Built-ins are seeded, never overwritten: a label the operator edited stays edited.
        for def in builtin_fields() {
            conn.execute(
                &format!(
                    "INSERT OR IGNORE INTO {prefix}_fields
                        (kind, key, label, field_type, options, data_class, builtin, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7)"
                ),
                params![
                    def.kind,
                    def.key,
                    def.label,
                    def.field_type,
                    serde_json::to_string(&def.options)?,
                    def.data_class,
                    now()
                ],
            )?;
        }
        Ok(())
    }

    // ─── Fields ───

    pub fn fields(&self, kind: Option<&str>) -> Result<Vec<FieldDef>> {
        let conn = self.conn()?;
        let p = &self.prefix;
        let mut stmt = conn
            .prepare(&format!(
                "SELECT kind, key, label, field_type, options, data_class, builtin
                 FROM {p}_fields WHERE (?1 IS NULL OR kind = ?1) ORDER BY kind, builtin DESC, key"
            ))
            .map_err(db)?;
        let rows = stmt
            .query_map(params![kind], |row| {
                let options: String = row.get(4)?;
                Ok(FieldDef {
                    kind: row.get(0)?,
                    key: row.get(1)?,
                    label: row.get(2)?,
                    field_type: row.get(3)?,
                    options: serde_json::from_str(&options).unwrap_or_default(),
                    data_class: row.get(5)?,
                    builtin: row.get::<_, i64>(6)? == 1,
                })
            })
            .map_err(db)?;
        rows.collect::<std::result::Result<_, _>>().map_err(db)
    }

    fn field(&self, conn: &Connection, kind: &str, key: &str) -> Result<Option<FieldDef>> {
        let p = &self.prefix;
        conn.query_row(
            &format!(
                "SELECT label, field_type, options, data_class, builtin
                 FROM {p}_fields WHERE kind = ?1 AND key = ?2"
            ),
            params![kind, key],
            |row| {
                let options: String = row.get(2)?;
                Ok(FieldDef {
                    kind: kind.into(),
                    key: key.into(),
                    label: row.get(0)?,
                    field_type: row.get(1)?,
                    options: serde_json::from_str(&options).unwrap_or_default(),
                    data_class: row.get(3)?,
                    builtin: row.get::<_, i64>(4)? == 1,
                })
            },
        )
        .optional()
        .map_err(db)
    }

    /// Declares a field for a kind. A key that exists is refused: changing a field's type
    /// under stored values would make them unreadable, so that is a separate, later decision.
    pub fn declare_field(&self, mut def: FieldDef) -> Result<FieldDef> {
        def.builtin = false;
        def.label = def.label.trim().to_string();
        check_field(&def).map_err(StoreError::Invalid)?;
        let conn = self.conn()?;
        if self.field(&conn, &def.kind, &def.key)?.is_some() {
            return Err(StoreError::Invalid(format!(
                "{} already has a field {:?}",
                def.kind, def.key
            )));
        }
        let p = &self.prefix;
        conn.execute(
            &format!(
                "INSERT INTO {p}_fields
                    (kind, key, label, field_type, options, data_class, builtin, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7)"
            ),
            params![
                def.kind,
                def.key,
                def.label,
                def.field_type,
                serde_json::to_string(&def.options).map_err(db)?,
                def.data_class,
                now()
            ],
        )
        .map_err(db)?;
        Ok(def)
    }

    // ─── Entities ───

    fn read_entity(&self, conn: &Connection, row: &Row<'_>) -> rusqlite::Result<Entity> {
        Ok(Entity {
            id: row.get(0)?,
            kind: row.get(1)?,
            name: row.get(2)?,
            note_ref: row.get(3)?,
            revision: row.get(4)?,
            created_at: row.get(5)?,
            updated_at: row.get(6)?,
            values: BTreeMap::new(),
            facts: Vec::new(),
        })
        .and_then(|mut entity| {
            entity.values = self.values_of(conn, &entity.id)?;
            entity.facts = self.facts_of(conn, &entity.id)?;
            Ok(entity)
        })
    }

    fn values_of(
        &self,
        conn: &Connection,
        id: &str,
    ) -> rusqlite::Result<BTreeMap<String, FieldValue>> {
        let p = &self.prefix;
        let mut stmt = conn.prepare(&format!(
            "SELECT key, value, source, updated_at FROM {p}_values WHERE entity_id = ?1"
        ))?;
        let rows = stmt.query_map(params![id], |row| {
            let raw: String = row.get(1)?;
            Ok((
                row.get::<_, String>(0)?,
                FieldValue {
                    value: serde_json::from_str(&raw).unwrap_or(Value::Null),
                    source: row.get(2)?,
                    updated_at: row.get(3)?,
                },
            ))
        })?;
        rows.collect()
    }

    fn facts_of(&self, conn: &Connection, id: &str) -> rusqlite::Result<Vec<Fact>> {
        let p = &self.prefix;
        let mut stmt = conn.prepare(&format!(
            "SELECT id, entity_id, predicate, place, latitude, longitude, valid_from, valid_to,
                    note, source, created_at
             FROM {p}_facts WHERE entity_id = ?1
             ORDER BY COALESCE(valid_from, ''), created_at"
        ))?;
        let rows = stmt.query_map(params![id], |row| {
            Ok(Fact {
                id: row.get(0)?,
                entity_id: row.get(1)?,
                predicate: row.get(2)?,
                place: row.get(3)?,
                latitude: row.get(4)?,
                longitude: row.get(5)?,
                valid_from: row.get(6)?,
                valid_to: row.get(7)?,
                note: row.get(8)?,
                source: row.get(9)?,
                created_at: row.get(10)?,
            })
        })?;
        rows.collect()
    }

    const ENTITY_COLUMNS: &'static str =
        "id, kind, name, note_ref, revision, created_at, updated_at";

    pub fn get(&self, id: &str) -> Result<Option<Entity>> {
        let conn = self.conn()?;
        let p = &self.prefix;
        conn.query_row(
            &format!(
                "SELECT {} FROM {p}_entities WHERE id = ?1",
                Self::ENTITY_COLUMNS
            ),
            params![id],
            |row| self.read_entity(&conn, row),
        )
        .optional()
        .map_err(db)
    }

    /// Every entity of a kind (or all), name order. `q` matches the name, case-insensitive.
    pub fn list(&self, kind: Option<&str>, q: Option<&str>) -> Result<Vec<Entity>> {
        let conn = self.conn()?;
        let p = &self.prefix;
        let pattern = q.map(|q| format!("%{}%", q.trim().to_lowercase()));
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {} FROM {p}_entities
                 WHERE (?1 IS NULL OR kind = ?1) AND (?2 IS NULL OR lower(name) LIKE ?2)
                 ORDER BY lower(name)",
                Self::ENTITY_COLUMNS
            ))
            .map_err(db)?;
        let rows = stmt
            .query_map(params![kind, pattern], |row| self.read_entity(&conn, row))
            .map_err(db)?;
        rows.collect::<std::result::Result<_, _>>().map_err(db)
    }

    fn check_source(source: &str) -> Result<()> {
        check_one_of("source", source, SOURCES).map_err(StoreError::Invalid)
    }

    /// Checks every value against the kind's fields and returns them normalised. `null`
    /// passes through: it clears the field.
    fn checked_values(
        &self,
        conn: &Connection,
        kind: &str,
        values: &BTreeMap<String, Value>,
    ) -> Result<BTreeMap<String, Value>> {
        let mut out = BTreeMap::new();
        for (key, value) in values {
            let Some(def) = self.field(conn, kind, key)? else {
                return Err(StoreError::Invalid(format!(
                    "{kind} has no field {key:?}; declare it first (POST /api/fields)"
                )));
            };
            let checked = if value.is_null() {
                Value::Null
            } else {
                check_value(&def, value).map_err(StoreError::Invalid)?
            };
            out.insert(key.clone(), checked);
        }
        Ok(out)
    }

    fn write_values(
        &self,
        tx: &Connection,
        id: &str,
        values: &BTreeMap<String, Value>,
        source: &str,
        at: &str,
    ) -> Result<()> {
        let p = &self.prefix;
        for (key, value) in values {
            if value.is_null() {
                tx.execute(
                    &format!("DELETE FROM {p}_values WHERE entity_id = ?1 AND key = ?2"),
                    params![id, key],
                )
                .map_err(db)?;
            } else {
                tx.execute(
                    &format!(
                        "INSERT INTO {p}_values (entity_id, key, value, source, updated_at)
                         VALUES (?1, ?2, ?3, ?4, ?5)
                         ON CONFLICT (entity_id, key) DO UPDATE SET
                            value = excluded.value, source = excluded.source,
                            updated_at = excluded.updated_at"
                    ),
                    params![id, key, value.to_string(), source, at],
                )
                .map_err(db)?;
            }
        }
        Ok(())
    }

    pub fn create(
        &self,
        kind: &str,
        name: &str,
        note_ref: Option<&str>,
        values: &BTreeMap<String, Value>,
        source: &str,
    ) -> Result<Entity> {
        check_one_of("kind", kind, crate::model::KINDS).map_err(StoreError::Invalid)?;
        Self::check_source(source)?;
        let name = name.trim();
        if name.is_empty() {
            return Err(StoreError::Invalid("name must not be empty".into()));
        }
        let mut conn = self.conn()?;
        let values = self.checked_values(&conn, kind, values)?;
        let id = new_id("ent");
        let at = now();
        let tx = axon_store::write_transaction(&mut conn).map_err(db)?;
        let p = &self.prefix;
        tx.execute(
            &format!(
                "INSERT INTO {p}_entities (id, kind, name, note_ref, revision, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, 1, ?5, ?5)"
            ),
            params![id, kind, name, note_ref, at],
        )
        .map_err(db)?;
        self.write_values(&tx, &id, &values, source, &at)?;
        tx.commit().map_err(db)?;
        drop(conn);
        self.get(&id)?
            .ok_or_else(|| StoreError::Db("the new entity was not found".into()))
    }

    pub fn patch(&self, id: &str, patch: &Patch) -> Result<Entity> {
        Self::check_source(&patch.source)?;
        let Some(current) = self.get(id)? else {
            return Err(StoreError::NotFound(format!("no entity {id}")));
        };
        if let Some(expected) = patch.expected_revision {
            if expected != current.revision {
                return Err(StoreError::Stale {
                    current_revision: current.revision,
                });
            }
        }
        if let Some(name) = &patch.name {
            if name.trim().is_empty() {
                return Err(StoreError::Invalid("name must not be empty".into()));
            }
        }
        let mut conn = self.conn()?;
        let values = self.checked_values(&conn, &current.kind, &patch.values)?;
        let at = now();
        let tx = axon_store::write_transaction(&mut conn).map_err(db)?;
        let p = &self.prefix;
        // The revision is compared again inside the write lock, so two writers that both
        // read revision 3 cannot both land.
        let changed = tx
            .execute(
                &format!(
                    "UPDATE {p}_entities SET
                        name = COALESCE(?2, name),
                        note_ref = CASE WHEN ?3 THEN ?4 ELSE note_ref END,
                        revision = revision + 1,
                        updated_at = ?5
                     WHERE id = ?1 AND revision = ?6"
                ),
                params![
                    id,
                    patch.name.as_deref().map(str::trim),
                    patch.note_ref.is_some(),
                    patch.note_ref.clone().flatten(),
                    at,
                    current.revision
                ],
            )
            .map_err(db)?;
        if changed == 0 {
            let now_revision: u32 = tx
                .query_row(
                    &format!("SELECT revision FROM {p}_entities WHERE id = ?1"),
                    params![id],
                    |row| row.get(0),
                )
                .map_err(db)?;
            return Err(StoreError::Stale {
                current_revision: now_revision,
            });
        }
        self.write_values(&tx, id, &values, &patch.source, &at)?;
        tx.commit().map_err(db)?;
        drop(conn);
        self.get(id)?
            .ok_or_else(|| StoreError::NotFound(format!("no entity {id}")))
    }

    pub fn delete(&self, id: &str) -> Result<bool> {
        let conn = self.conn()?;
        let p = &self.prefix;
        conn.execute(
            &format!("DELETE FROM {p}_entities WHERE id = ?1"),
            params![id],
        )
        .map(|n| n > 0)
        .map_err(db)
    }

    // ─── Facts ───

    pub fn add_fact(&self, entity_id: &str, new: &NewFact) -> Result<Fact> {
        check_new_fact(new)?;
        if self.get(entity_id)?.is_none() {
            return Err(StoreError::NotFound(format!("no entity {entity_id}")));
        }
        let conn = self.conn()?;
        let id = new_id("fact");
        let p = &self.prefix;
        conn.execute(
            &format!(
                "INSERT INTO {p}_facts
                    (id, entity_id, predicate, place, latitude, longitude, valid_from, valid_to,
                     note, source, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"
            ),
            params![
                id,
                entity_id,
                new.predicate,
                new.place.trim(),
                new.latitude,
                new.longitude,
                new.valid_from,
                new.valid_to,
                new.note.as_deref().map(str::trim).filter(|n| !n.is_empty()),
                new.source,
                now()
            ],
        )
        .map_err(db)?;
        self.facts_of(&conn, entity_id)
            .map_err(db)?
            .into_iter()
            .find(|f| f.id == id)
            .ok_or_else(|| StoreError::Db("the new fact was not found".into()))
    }

    pub fn delete_fact(&self, entity_id: &str, fact_id: &str) -> Result<bool> {
        let conn = self.conn()?;
        let p = &self.prefix;
        conn.execute(
            &format!("DELETE FROM {p}_facts WHERE id = ?1 AND entity_id = ?2"),
            params![fact_id, entity_id],
        )
        .map(|n| n > 0)
        .map_err(db)
    }

    // ─── External ids ───

    pub fn external(&self, system: &str, external_id: &str) -> Result<Option<String>> {
        let conn = self.conn()?;
        let p = &self.prefix;
        conn.query_row(
            &format!("SELECT entity_id FROM {p}_external WHERE system = ?1 AND external_id = ?2"),
            params![system, external_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(db)
    }

    /// Every entity linked to `system`, for the name match that must skip them.
    pub fn linked_entity_ids(&self, system: &str) -> Result<std::collections::HashSet<String>> {
        let conn = self.conn()?;
        let p = &self.prefix;
        let mut stmt = conn
            .prepare(&format!(
                "SELECT entity_id FROM {p}_external WHERE system = ?1"
            ))
            .map_err(db)?;
        let rows = stmt
            .query_map(params![system], |row| row.get(0))
            .map_err(db)?;
        rows.collect::<std::result::Result<_, _>>().map_err(db)
    }

    pub fn link_external(
        &self,
        system: &str,
        external_id: &str,
        entity_id: &str,
        etag: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn()?;
        let p = &self.prefix;
        conn.execute(
            &format!(
                "INSERT INTO {p}_external (system, external_id, entity_id, etag, synced_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (system, external_id) DO UPDATE SET
                    entity_id = excluded.entity_id, etag = excluded.etag,
                    synced_at = excluded.synced_at"
            ),
            params![system, external_id, entity_id, etag, now()],
        )
        .map(|_| ())
        .map_err(db)
    }
}

/// Checks a new fact before anything is stored or sent: a known predicate and source, a
/// place, dates that are dates and in order, and both dates on an away period.
pub fn check_new_fact(new: &NewFact) -> Result<()> {
    check_one_of("predicate", &new.predicate, FACT_PREDICATES).map_err(StoreError::Invalid)?;
    check_one_of("source", &new.source, SOURCES).map_err(StoreError::Invalid)?;
    if new.place.trim().is_empty() {
        return Err(StoreError::Invalid("place must not be empty".into()));
    }
    for day in [&new.valid_from, &new.valid_to].into_iter().flatten() {
        if !is_day(day) {
            return Err(StoreError::Invalid(format!(
                "{day:?} is not a YYYY-MM-DD date"
            )));
        }
    }
    if let (Some(from), Some(to)) = (&new.valid_from, &new.valid_to) {
        if from > to {
            return Err(StoreError::Invalid(
                "valid_from must not be after valid_to".into(),
            ));
        }
    }
    if new.predicate == "away" && (new.valid_from.is_none() || new.valid_to.is_none()) {
        return Err(StoreError::Invalid(
            "an away period needs both valid_from and valid_to".into(),
        ));
    }
    Ok(())
}

/// Checks a kind name, for callers that filter by kind.
pub fn check_kind(kind: &str) -> Result<()> {
    check_one_of("kind", kind, KINDS).map_err(StoreError::Invalid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scratch(name: &str) -> (EntitiesStore, std::path::PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("axon-entities-store-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        (EntitiesStore::open(&dir.join("axon.db")).unwrap(), dir)
    }

    fn values(pairs: &[(&str, Value)]) -> BTreeMap<String, Value> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn a_person_is_created_with_checked_values_and_read_back() {
        let (store, dir) = scratch("create");
        let ron = store
            .create(
                "person",
                " Ron ",
                None,
                &values(&[
                    ("sleeping_option", json!("ask")),
                    ("emails", json!(["ron@example.org"])),
                ]),
                "operator",
            )
            .unwrap();
        assert_eq!(ron.name, "Ron");
        assert_eq!(ron.revision, 1);
        assert_eq!(ron.values["sleeping_option"].value, json!("ask"));
        assert_eq!(ron.values["emails"].source, "operator");
        assert_eq!(store.list(Some("person"), Some("ro")).unwrap().len(), 1);

        let refused = store.create(
            "person",
            "X",
            None,
            &values(&[("shoe_size", json!(44))]),
            "operator",
        );
        assert!(matches!(refused, Err(StoreError::Invalid(m)) if m.contains("declare it first")));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_patch_against_an_old_revision_is_stale_and_a_null_clears() {
        let (store, dir) = scratch("patch");
        let ron = store
            .create(
                "person",
                "Ron",
                None,
                &values(&[("sleeping_option", json!("ask"))]),
                "operator",
            )
            .unwrap();
        let patched = store
            .patch(
                &ron.id,
                &Patch {
                    values: values(&[
                        ("sleeping_option", Value::Null),
                        ("sleeping_note", json!("sofa")),
                    ]),
                    source: "operator".into(),
                    expected_revision: Some(1),
                    ..Patch::default()
                },
            )
            .unwrap();
        assert_eq!(patched.revision, 2);
        assert!(!patched.values.contains_key("sleeping_option"));
        assert_eq!(patched.values["sleeping_note"].value, json!("sofa"));

        let stale = store.patch(
            &ron.id,
            &Patch {
                name: Some("Ronald".into()),
                source: "operator".into(),
                expected_revision: Some(1),
                ..Patch::default()
            },
        );
        assert_eq!(
            stale,
            Err(StoreError::Stale {
                current_revision: 2
            })
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_declared_field_takes_values_and_a_duplicate_is_refused() {
        let (store, dir) = scratch("declare");
        let def = FieldDef {
            kind: "person".into(),
            key: "climbing_grade".into(),
            label: "Climbing grade".into(),
            field_type: "text".into(),
            options: vec![],
            data_class: "C2".into(),
            builtin: true, // ignored: a declared field is never built in
        };
        let stored = store.declare_field(def.clone()).unwrap();
        assert!(!stored.builtin);
        assert!(store.declare_field(def).is_err());
        let ron = store
            .create(
                "person",
                "Ron",
                None,
                &values(&[("climbing_grade", json!("6b"))]),
                "operator",
            )
            .unwrap();
        assert_eq!(ron.values["climbing_grade"].value, json!("6b"));
        assert_eq!(
            store.fields(Some("person")).unwrap().len(),
            builtin_fields().len() + 1
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn facts_are_checked_and_cascade_with_their_entity() {
        let (store, dir) = scratch("facts");
        let ron = store
            .create("person", "Ron", None, &BTreeMap::new(), "operator")
            .unwrap();
        let away = NewFact {
            predicate: "away".into(),
            place: "Lisbon".into(),
            valid_from: Some("2026-10-04".into()),
            valid_to: Some("2026-10-18".into()),
            source: "operator".into(),
            ..NewFact::default()
        };
        store.add_fact(&ron.id, &away).unwrap();
        let open_away = NewFact {
            valid_to: None,
            ..away.clone()
        };
        assert!(
            store.add_fact(&ron.id, &open_away).is_err(),
            "away needs both dates"
        );
        let backwards = NewFact {
            valid_from: Some("2026-10-20".into()),
            ..away.clone()
        };
        assert!(store.add_fact(&ron.id, &backwards).is_err());
        assert_eq!(store.get(&ron.id).unwrap().unwrap().facts.len(), 1);

        assert!(store.delete(&ron.id).unwrap());
        let conn = store.conn().unwrap();
        let left: i64 = conn
            .query_row("SELECT COUNT(*) FROM entities_facts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 0, "facts go with their entity");
        let _ = std::fs::remove_dir_all(dir);
    }
}
