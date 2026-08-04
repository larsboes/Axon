//! Postgres persistence, own `calendar` schema on the shared instance
//! (`capabilities/postgres`), same pattern as trips/scouting/transit.
//! Instants stay TEXT (naive local, lexicographically ordered — see README's
//! time model), structured fields stay JSON text, so the store needs no
//! postgres type features beyond what trips already uses.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use postgres::{Client, NoTls, Row};
use serde_json::Value;

use crate::correlate;
use crate::date;
use crate::google;
use crate::model::{
    Commitment, Context, Entry, ExportOptIn, NewContext, NewEntry, NewRhythm, Rhythm,
    UpdateContext, UpdateEntry, UpdateRhythm,
};
use crate::rhythm;

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

type StoreResult<T> = Result<T, Box<dyn std::error::Error>>;

fn generated_id(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}:{nanos:x}{sequence:04x}")
}

fn now_text() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .to_string()
}

fn validate_schema(schema: &str) -> StoreResult<()> {
    if !schema
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        return Err("schema must contain only ASCII letters, digits, or underscore".into());
    }
    Ok(())
}

pub struct CalendarStore {
    conn: Mutex<Client>,
    schema: String,
}

impl CalendarStore {
    pub fn open(database_url: &str) -> StoreResult<Self> {
        Self::open_in_schema(database_url, "calendar")
    }

    pub fn open_in_schema(database_url: &str, schema: &str) -> StoreResult<Self> {
        validate_schema(schema)?;
        let store = Self {
            conn: Mutex::new(Client::connect(database_url, NoTls)?),
            schema: schema.to_string(),
        };
        store.init_schema()?;
        Ok(store)
    }

    fn init_schema(&self) -> StoreResult<()> {
        let mut conn = self.conn.lock().unwrap();
        conn.batch_execute(&format!(
            "
            CREATE SCHEMA IF NOT EXISTS {schema};
            CREATE TABLE IF NOT EXISTS {schema}.rhythms (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                title TEXT NOT NULL,
                location TEXT,
                byweekday TEXT NOT NULL,
                start_time TEXT,
                end_time TEXT,
                valid_from TEXT NOT NULL,
                valid_until TEXT NOT NULL,
                active INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS {schema}.entries (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                commitment TEXT NOT NULL DEFAULT 'possible'
                    CHECK (commitment IN ('possible','planned','committed')),
                title TEXT NOT NULL,
                starts_at TEXT NOT NULL,
                ends_at TEXT NOT NULL,
                all_day INTEGER NOT NULL DEFAULT 0,
                location TEXT,
                notes TEXT,
                source TEXT NOT NULL DEFAULT 'manual',
                external_id TEXT,
                rhythm_id TEXT REFERENCES {schema}.rhythms(id) ON DELETE SET NULL,
                payload TEXT NOT NULL DEFAULT '{{}}',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            -- Migration for tables created before the commitment axis existed.
            -- 'possible' is the deliberate default: everything already in
            -- there arrived via scouting promotion, which was never an operator decision,
            -- and a bug in this direction hands out a free day
            -- rather than silently walling one off.
            ALTER TABLE {schema}.entries
                ADD COLUMN IF NOT EXISTS commitment TEXT NOT NULL DEFAULT 'possible';
            ALTER TABLE {schema}.entries
                DROP CONSTRAINT IF EXISTS entries_commitment_check;
            ALTER TABLE {schema}.entries
                ADD CONSTRAINT entries_commitment_check
                CHECK (commitment IN ('possible','planned','committed'));
            CREATE INDEX IF NOT EXISTS idx_calendar_entries_starts
                ON {schema}.entries(starts_at);
            CREATE UNIQUE INDEX IF NOT EXISTS idx_calendar_entries_external
                ON {schema}.entries(source, external_id)
                WHERE external_id IS NOT NULL;
            CREATE UNIQUE INDEX IF NOT EXISTS idx_calendar_entries_rhythm_slot
                ON {schema}.entries(rhythm_id, starts_at)
                WHERE rhythm_id IS NOT NULL;
            CREATE TABLE IF NOT EXISTS {schema}.contexts (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                title TEXT NOT NULL,
                details TEXT NOT NULL DEFAULT '',
                valid_from TEXT NOT NULL,
                valid_until TEXT NOT NULL,
                source TEXT NOT NULL DEFAULT 'manual',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_calendar_contexts_window
                ON {schema}.contexts(valid_from, valid_until);
            -- Which entries already became a trip, so materialising twice
            -- returns the plan that exists instead of making a second one.
            -- Same ledger shape as google_exports, and for the same reason:
            -- the fact belongs to calendar, the plan belongs to trips, and
            -- neither store reaches into the other.
            CREATE TABLE IF NOT EXISTS {schema}.trip_materializations (
                entry_id TEXT PRIMARY KEY
                    REFERENCES {schema}.entries(id) ON DELETE CASCADE,
                plan_id TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS {schema}.google_exports (
                entry_id TEXT PRIMARY KEY
                    REFERENCES {schema}.entries(id) ON DELETE CASCADE,
                google_calendar_id TEXT NOT NULL,
                google_event_id TEXT,
                pushed_at TEXT,
                created_at TEXT NOT NULL
            );
            ",
            schema = self.schema
        ))?;
        Ok(())
    }

    // ---- entries ----------------------------------------------------------

    /// Entries overlapping the day window [from, to) — both "YYYY-MM-DD",
    /// from inclusive, to exclusive. Day granularity is what the correlation
    /// layer asks in ("is 14 Aug feasible?"); finer slicing is presentation.
    pub fn list_entries(&self, from: &str, to: &str, kinds: &[String]) -> StoreResult<Vec<Entry>> {
        let (from_boundary, to_boundary) = day_window_bounds(from, to)?;
        let mut conn = self.conn.lock().unwrap();
        let rows = if kinds.is_empty() {
            conn.query(
                &format!(
                    "SELECT * FROM {schema}.entries \
                     WHERE starts_at < $1 AND ends_at > $2 \
                     ORDER BY starts_at",
                    schema = self.schema
                ),
                &[&to_boundary, &from_boundary],
            )?
        } else {
            conn.query(
                &format!(
                    "SELECT * FROM {schema}.entries \
                     WHERE starts_at < $1 AND ends_at > $2 \
                       AND kind = ANY($3) \
                     ORDER BY starts_at",
                    schema = self.schema
                ),
                &[&to_boundary, &from_boundary, &kinds],
            )?
        };
        rows.iter().map(entry_from_row).collect()
    }

    /// Provider drafts are not a `kind`: the source tells us where they came
    /// from and `possible` is the deliberately non-blocking adoption state.
    /// Keeping this predicate here stops each dashboard surface from inventing
    /// its own idea of what "waiting for review" means.
    pub fn list_google_drafts(&self, from: &str, to: &str) -> StoreResult<Vec<Entry>> {
        let (from_boundary, to_boundary) = day_window_bounds(from, to)?;
        let mut conn = self.conn.lock().unwrap();
        let rows = conn.query(
            &format!(
                "SELECT * FROM {schema}.entries \
                 WHERE starts_at < $1 AND ends_at > $2 \
                   AND source = $3 AND commitment = $4 \
                 ORDER BY starts_at",
                schema = self.schema
            ),
            &[&to_boundary, &from_boundary, &google::SOURCE, &"possible"],
        )?;
        rows.iter().map(entry_from_row).collect()
    }

    /// External contributions that still need an Axon decision, except Google
    /// imports. Google has its own inbox because its refresh/ownership rules
    /// are special; all other providers share the ordinary Calendar proposal
    /// flow. A user-created `possible` block is intentionally not a proposal.
    pub fn list_external_proposals(&self, from: &str, to: &str) -> StoreResult<Vec<Entry>> {
        let (from_boundary, to_boundary) = day_window_bounds(from, to)?;
        let mut conn = self.conn.lock().unwrap();
        let rows = conn.query(
            &format!(
                "SELECT * FROM {schema}.entries \
                 WHERE starts_at < $1 AND ends_at > $2 \
                   AND commitment = $3 AND external_id IS NOT NULL AND source <> $4 \
                 ORDER BY starts_at",
                schema = self.schema
            ),
            &[&to_boundary, &from_boundary, &"possible", &google::SOURCE],
        )?;
        let proposals: Vec<Entry> = rows
            .iter()
            .map(entry_from_row)
            .collect::<StoreResult<_>>()?;
        if proposals.is_empty() {
            return Ok(proposals);
        }

        // Anything the operator has already decided on, whatever key it came in
        // under. `(source, external_id)` only dedupes a source against itself,
        // so this is the one check that can tell the inbox an event is already
        // in the calendar. The window is the caller's: a twin that overlaps a
        // proposal *outside* the requested range is not read, which costs a
        // suppression only for a proposal the window already cuts in half.
        let adopted = conn.query(
            &format!(
                "SELECT * FROM {schema}.entries \
                 WHERE starts_at < $1 AND ends_at > $2 AND commitment <> $3",
                schema = self.schema
            ),
            &[&to_boundary, &from_boundary, &"possible"],
        )?;
        let adopted: Vec<Entry> = adopted
            .iter()
            .map(entry_from_row)
            .collect::<StoreResult<_>>()?;
        Ok(correlate::without_already_adopted(proposals, &adopted)?)
    }

    /// The entry a provider contributed under this key, if any. Phase E's
    /// conflict policy has to read before it writes — `upsert_external_entry`
    /// overwrites unconditionally, and "Axon wins" means deciding *not* to
    /// call it. Reads the same `(source, external_id)` pair that index is
    /// unique on, so there is at most one.
    pub fn get_entry_by_external(
        &self,
        source: &str,
        external_id: &str,
    ) -> StoreResult<Option<Entry>> {
        let mut conn = self.conn.lock().unwrap();
        let row = conn.query_opt(
            &format!(
                "SELECT * FROM {schema}.entries WHERE source = $1 AND external_id = $2",
                schema = self.schema
            ),
            &[&source, &external_id],
        )?;
        row.map(|r| entry_from_row(&r)).transpose()
    }

    pub fn get_entry(&self, id: &str) -> StoreResult<Option<Entry>> {
        let mut conn = self.conn.lock().unwrap();
        let row = conn.query_opt(
            &format!(
                "SELECT * FROM {schema}.entries WHERE id = $1",
                schema = self.schema
            ),
            &[&id],
        )?;
        row.map(|r| entry_from_row(&r)).transpose()
    }

    pub fn create_entry(&self, input: &NewEntry) -> StoreResult<Entry> {
        input.validate()?;
        let entry = entry_from_input(input);
        let mut conn = self.conn.lock().unwrap();
        conn.execute(
            &format!(
                "INSERT INTO {schema}.entries \
                 (id, kind, title, starts_at, ends_at, all_day, location, notes, \
                  source, external_id, rhythm_id, payload, created_at, updated_at, \
                  commitment) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)",
                schema = self.schema
            ),
            &[
                &entry.id,
                &entry.kind,
                &entry.title,
                &entry.starts_at,
                &entry.ends_at,
                &(entry.all_day as i32),
                &entry.location,
                &entry.notes,
                &entry.source,
                &entry.external_id,
                &entry.rhythm_id,
                &serde_json::to_string(&entry.payload)?,
                &entry.created_at,
                &entry.updated_at,
                &entry.commitment.as_str(),
            ],
        )?;
        Ok(entry)
    }

    /// Idempotent contribution boundary for Feed/Scouting/Google and future
    /// providers. A stable `(source, external_id)` updates the contributed
    /// entry in place while preserving its Axon id and original `created_at`.
    /// Manual CRUD stays on `create_entry` and never acquires upsert semantics.
    pub fn upsert_external_entry(&self, input: &NewEntry) -> StoreResult<Entry> {
        input.validate()?;
        let external_id = input
            .external_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .ok_or("external_id is required for external upsert")?;
        let mut entry = entry_from_input(input);
        entry.external_id = Some(external_id.to_string());

        let mut conn = self.conn.lock().unwrap();
        let row = conn.query_one(
            &format!(
                "INSERT INTO {schema}.entries \
                 (id, kind, title, starts_at, ends_at, all_day, location, notes, \
                  source, external_id, rhythm_id, payload, created_at, updated_at, \
                  commitment) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15) \
                 ON CONFLICT (source, external_id) WHERE external_id IS NOT NULL \
                 DO UPDATE SET kind=EXCLUDED.kind, title=EXCLUDED.title, \
                   starts_at=EXCLUDED.starts_at, ends_at=EXCLUDED.ends_at, \
                   all_day=EXCLUDED.all_day, location=EXCLUDED.location, \
                   notes=EXCLUDED.notes, rhythm_id=EXCLUDED.rhythm_id, \
                   payload=EXCLUDED.payload, updated_at=EXCLUDED.updated_at \
                 RETURNING *",
                // `commitment` is absent from that DO UPDATE list on purpose.
                // A provider re-running its import re-states what an event is,
                // never how committed the operator is to it: once they raise a promoted
                // event to planned or committed, the next `scout
                // --promote-calendar` must not quietly hand it back down.
                schema = self.schema
            ),
            &[
                &entry.id,
                &entry.kind,
                &entry.title,
                &entry.starts_at,
                &entry.ends_at,
                &(entry.all_day as i32),
                &entry.location,
                &entry.notes,
                &entry.source,
                &entry.external_id,
                &entry.rhythm_id,
                &serde_json::to_string(&entry.payload)?,
                &entry.created_at,
                &entry.updated_at,
                &entry.commitment.as_str(),
            ],
        )?;
        entry_from_row(&row)
    }

    /// Applies a patch. Any patch to a rhythm-linked entry detaches it from
    /// its rhythm (rhythm_id → NULL): re-materialization regenerates linked
    /// instances, so keeping the link would silently drop the override.
    pub fn update_entry(&self, id: &str, patch: &UpdateEntry) -> StoreResult<Option<Entry>> {
        let mut entry = match self.get_entry(id)? {
            Some(entry) => entry,
            None => return Ok(None),
        };
        if let Some(kind) = &patch.kind {
            entry.kind = kind.clone();
        }
        if let Some(title) = &patch.title {
            entry.title = title.trim().to_string();
        }
        if let Some(starts_at) = &patch.starts_at {
            entry.starts_at = starts_at.clone();
        }
        if let Some(ends_at) = &patch.ends_at {
            entry.ends_at = ends_at.clone();
        }
        if let Some(all_day) = patch.all_day {
            entry.all_day = all_day;
        }
        if let Some(location) = &patch.location {
            entry.location = location.clone();
        }
        if let Some(notes) = &patch.notes {
            entry.notes = notes.clone();
        }
        if let Some(commitment) = patch.commitment {
            entry.commitment = commitment;
        }
        if patch != &UpdateEntry::default() && entry.rhythm_id.is_some() {
            entry.rhythm_id = None;
        }
        entry_as_new(&entry).validate()?;
        entry.updated_at = now_text();
        let mut conn = self.conn.lock().unwrap();
        conn.execute(
            &format!(
                "UPDATE {schema}.entries SET kind=$2, title=$3, starts_at=$4, ends_at=$5, \
                 all_day=$6, location=$7, notes=$8, rhythm_id=$9, updated_at=$10, \
                 commitment=$11 WHERE id=$1",
                schema = self.schema
            ),
            &[
                &entry.id,
                &entry.kind,
                &entry.title,
                &entry.starts_at,
                &entry.ends_at,
                &(entry.all_day as i32),
                &entry.location,
                &entry.notes,
                &entry.rhythm_id,
                &entry.updated_at,
                &entry.commitment.as_str(),
            ],
        )?;
        Ok(Some(entry))
    }

    pub fn delete_entry(&self, id: &str) -> StoreResult<bool> {
        let mut conn = self.conn.lock().unwrap();
        let count = conn.execute(
            &format!(
                "DELETE FROM {schema}.entries WHERE id = $1",
                schema = self.schema
            ),
            &[&id],
        )?;
        Ok(count > 0)
    }

    // ---- bounded planning context ----------------------------------------

    /// Contexts overlapping the visible date window. Their end is inclusive
    /// because they describe a human horizon ("through September"), not a
    /// schedulable half-open instant.
    pub fn list_contexts(&self, from: &str, to: &str) -> StoreResult<Vec<Context>> {
        let from_day = date::parse_date(from).ok_or("from must be YYYY-MM-DD")?;
        let to_day = date::parse_date(to).ok_or("to must be YYYY-MM-DD")?;
        if to_day < from_day {
            return Err("to must be on or after from".into());
        }
        let mut conn = self.conn.lock().unwrap();
        let rows = conn.query(
            &format!(
                "SELECT * FROM {schema}.contexts \
                 WHERE valid_from < $1 AND valid_until >= $2 \
                 ORDER BY valid_from, title",
                schema = self.schema
            ),
            &[&to, &from],
        )?;
        rows.iter().map(context_from_row).collect()
    }

    pub fn get_context(&self, id: &str) -> StoreResult<Option<Context>> {
        let mut conn = self.conn.lock().unwrap();
        let row = conn.query_opt(
            &format!(
                "SELECT * FROM {schema}.contexts WHERE id = $1",
                schema = self.schema
            ),
            &[&id],
        )?;
        row.map(|row| context_from_row(&row)).transpose()
    }

    pub fn create_context(&self, input: &NewContext) -> StoreResult<Context> {
        input.validate()?;
        let context = context_from_input(input);
        let mut conn = self.conn.lock().unwrap();
        conn.execute(
            &format!(
                "INSERT INTO {schema}.contexts \
                 (id, kind, title, details, valid_from, valid_until, source, created_at, updated_at) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
                schema = self.schema
            ),
            &[
                &context.id,
                &context.kind,
                &context.title,
                &context.details,
                &context.valid_from,
                &context.valid_until,
                &context.source,
                &context.created_at,
                &context.updated_at,
            ],
        )?;
        Ok(context)
    }

    pub fn update_context(&self, id: &str, patch: &UpdateContext) -> StoreResult<Option<Context>> {
        let mut context = match self.get_context(id)? {
            Some(context) => context,
            None => return Ok(None),
        };
        if let Some(kind) = &patch.kind {
            context.kind = kind.clone();
        }
        if let Some(title) = &patch.title {
            context.title = title.trim().to_string();
        }
        if let Some(details) = &patch.details {
            context.details = details.trim().to_string();
        }
        if let Some(valid_from) = &patch.valid_from {
            context.valid_from = valid_from.clone();
        }
        if let Some(valid_until) = &patch.valid_until {
            context.valid_until = valid_until.clone();
        }
        context_as_new(&context).validate()?;
        context.updated_at = now_text();
        let mut conn = self.conn.lock().unwrap();
        conn.execute(
            &format!(
                "UPDATE {schema}.contexts SET kind=$2, title=$3, details=$4, \
                 valid_from=$5, valid_until=$6, updated_at=$7 WHERE id=$1",
                schema = self.schema
            ),
            &[
                &context.id,
                &context.kind,
                &context.title,
                &context.details,
                &context.valid_from,
                &context.valid_until,
                &context.updated_at,
            ],
        )?;
        Ok(Some(context))
    }

    pub fn delete_context(&self, id: &str) -> StoreResult<bool> {
        let mut conn = self.conn.lock().unwrap();
        let count = conn.execute(
            &format!(
                "DELETE FROM {schema}.contexts WHERE id = $1",
                schema = self.schema
            ),
            &[&id],
        )?;
        Ok(count > 0)
    }

    // ---- google export ledger (Phase E) ------------------------------------

    /// Opts one entry in to Google export.
    ///
    /// A row in `google_exports` *is* the opt-in — nothing exports by default
    /// because the table starts empty, and opting out is deleting the row.
    /// The table also carries the Google event id the first push returns,
    /// which is a genuinely new fact with nowhere to live on `entries`: without
    /// it, the second push would create a second Google event instead of
    /// updating the first.
    ///
    /// Refusals (an imported entry, a rhythm instance) come from
    /// `google::export_refusal` so the rule is stated once and unit-tested
    /// without a database.
    pub fn opt_in_export(&self, entry_id: &str, calendar_id: &str) -> StoreResult<ExportOptIn> {
        let entry = self
            .get_entry(entry_id)?
            .ok_or_else(|| format!("entry {entry_id} not found"))?;
        if let Some(reason) = google::export_refusal(&entry) {
            return Err(reason.into());
        }
        if calendar_id.trim().is_empty() {
            return Err("google_calendar_id is required".into());
        }
        let mut conn = self.conn.lock().unwrap();
        let row = conn.query_one(
            &format!(
                "INSERT INTO {schema}.google_exports \
                 (entry_id, google_calendar_id, google_event_id, pushed_at, created_at) \
                 VALUES ($1,$2,NULL,NULL,$3) \
                 ON CONFLICT (entry_id) DO UPDATE SET \
                   google_calendar_id = EXCLUDED.google_calendar_id \
                 RETURNING *",
                schema = self.schema
            ),
            &[&entry.id, &calendar_id.trim(), &now_text()],
        )?;
        export_from_row(&row)
    }

    pub fn opt_out_export(&self, entry_id: &str) -> StoreResult<bool> {
        let mut conn = self.conn.lock().unwrap();
        let count = conn.execute(
            &format!(
                "DELETE FROM {schema}.google_exports WHERE entry_id = $1",
                schema = self.schema
            ),
            &[&entry_id],
        )?;
        Ok(count > 0)
    }

    /// The plan an entry already belongs to, if any.
    pub fn trip_plan_for(&self, entry_id: &str) -> StoreResult<Option<String>> {
        let mut conn = self.conn.lock().unwrap();
        let rows = conn.query(
            &format!(
                "SELECT plan_id FROM {schema}.trip_materializations WHERE entry_id = $1",
                schema = self.schema
            ),
            &[&entry_id],
        )?;
        Ok(rows.first().map(|row| row.get(0)))
    }

    /// Records that these entries became one plan. Written only after trips
    /// confirms the plan exists, so a failed call leaves no claim behind.
    pub fn record_trip_materialization(
        &self,
        entry_ids: &[String],
        plan_id: &str,
    ) -> StoreResult<()> {
        let now = now_text();
        let mut conn = self.conn.lock().unwrap();
        for entry_id in entry_ids {
            conn.execute(
                &format!(
                    "INSERT INTO {schema}.trip_materializations (entry_id, plan_id, created_at) \
                     VALUES ($1,$2,$3) ON CONFLICT (entry_id) DO NOTHING",
                    schema = self.schema
                ),
                &[entry_id, &plan_id, &now],
            )?;
        }
        Ok(())
    }

    /// Drops ledger rows whose plan trips no longer has. Called only after
    /// trips has *said* the plan is gone, never on a failure to reach it.
    pub fn forget_trip_materialization(&self, plan_id: &str) -> StoreResult<u64> {
        let mut conn = self.conn.lock().unwrap();
        Ok(conn.execute(
            &format!(
                "DELETE FROM {schema}.trip_materializations WHERE plan_id = $1",
                schema = self.schema
            ),
            &[&plan_id],
        )?)
    }

    pub fn list_export_optins(&self) -> StoreResult<Vec<ExportOptIn>> {
        let mut conn = self.conn.lock().unwrap();
        let rows = conn.query(
            &format!(
                "SELECT * FROM {schema}.google_exports ORDER BY created_at",
                schema = self.schema
            ),
            &[],
        )?;
        rows.iter().map(export_from_row).collect()
    }

    /// Every opted-in entry with the entry itself, for the export run. The
    /// `ON DELETE CASCADE` on `entry_id` means a deleted entry takes its
    /// opt-in with it, so this join never sees an orphan.
    ///
    /// The ledger's columns are aliased: both tables carry a `created_at`, and
    /// `row.get("created_at")` on an unaliased join takes whichever came
    /// first, which would silently stamp the entry with the opt-in's date.
    pub fn export_queue(&self) -> StoreResult<Vec<(ExportOptIn, Entry)>> {
        let mut conn = self.conn.lock().unwrap();
        let rows = conn.query(
            &format!(
                "SELECT e.*, \
                        x.entry_id AS x_entry_id, \
                        x.google_calendar_id AS x_google_calendar_id, \
                        x.google_event_id AS x_google_event_id, \
                        x.pushed_at AS x_pushed_at, \
                        x.created_at AS x_created_at \
                 FROM {schema}.google_exports x \
                 JOIN {schema}.entries e ON e.id = x.entry_id \
                 ORDER BY e.starts_at",
                schema = self.schema
            ),
            &[],
        )?;
        rows.iter()
            .map(|row| {
                let optin = ExportOptIn {
                    entry_id: row.get("x_entry_id"),
                    google_calendar_id: row.get("x_google_calendar_id"),
                    google_event_id: row.get("x_google_event_id"),
                    pushed_at: row.get("x_pushed_at"),
                    created_at: row.get("x_created_at"),
                };
                Ok((optin, entry_from_row(row)?))
            })
            .collect()
    }

    /// Records what a push produced. `google_event_id` is what turns the next
    /// push into an update instead of a duplicate.
    pub fn record_export_push(
        &self,
        entry_id: &str,
        google_event_id: &str,
    ) -> StoreResult<Option<ExportOptIn>> {
        let mut conn = self.conn.lock().unwrap();
        let row = conn.query_opt(
            &format!(
                "UPDATE {schema}.google_exports \
                 SET google_event_id = $2, pushed_at = $3 WHERE entry_id = $1 RETURNING *",
                schema = self.schema
            ),
            &[&entry_id, &google_event_id, &now_text()],
        )?;
        row.map(|r| export_from_row(&r)).transpose()
    }

    // ---- rhythms ----------------------------------------------------------

    pub fn list_rhythms(&self) -> StoreResult<Vec<Rhythm>> {
        let mut conn = self.conn.lock().unwrap();
        let rows = conn.query(
            &format!(
                "SELECT * FROM {schema}.rhythms ORDER BY valid_from",
                schema = self.schema
            ),
            &[],
        )?;
        rows.iter().map(rhythm_from_row).collect()
    }

    pub fn get_rhythm(&self, id: &str) -> StoreResult<Option<Rhythm>> {
        let mut conn = self.conn.lock().unwrap();
        let row = conn.query_opt(
            &format!(
                "SELECT * FROM {schema}.rhythms WHERE id = $1",
                schema = self.schema
            ),
            &[&id],
        )?;
        row.map(|r| rhythm_from_row(&r)).transpose()
    }

    /// Creates the rhythm and materializes its future instances atomically.
    /// Returns the rhythm plus how many instances were created.
    pub fn create_rhythm(&self, input: &NewRhythm) -> StoreResult<(Rhythm, usize)> {
        input.validate()?;
        let rhythm = Rhythm {
            id: generated_id("cal:rhythm"),
            kind: input.kind.clone(),
            title: input.title.trim().to_string(),
            location: input.location.clone(),
            byweekday: input.byweekday.clone(),
            start_time: input.start_time.clone(),
            end_time: input.end_time.clone(),
            valid_from: input.valid_from.clone(),
            valid_until: input.valid_until.clone(),
            active: input.active,
            created_at: now_text(),
            updated_at: now_text(),
        };
        let mut conn = self.conn.lock().unwrap();
        let mut tx = conn.transaction()?;
        tx.execute(
            &format!(
                "INSERT INTO {schema}.rhythms \
                 (id, kind, title, location, byweekday, start_time, end_time, \
                  valid_from, valid_until, active, created_at, updated_at) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)",
                schema = self.schema
            ),
            &[
                &rhythm.id,
                &rhythm.kind,
                &rhythm.title,
                &rhythm.location,
                &rhythm.byweekday.join(","),
                &rhythm.start_time,
                &rhythm.end_time,
                &rhythm.valid_from,
                &rhythm.valid_until,
                &(rhythm.active as i32),
                &rhythm.created_at,
                &rhythm.updated_at,
            ],
        )?;
        let created = insert_instances(&mut tx, &self.schema, &rhythm)?;
        tx.commit()?;
        Ok((rhythm, created))
    }

    /// Applies a patch and re-materializes forward: future linked instances
    /// are deleted and regenerated from the new rule, past ones stay as
    /// history. Detached (user-overridden) instances are untouched — they no
    /// longer carry the rhythm_id.
    pub fn update_rhythm(
        &self,
        id: &str,
        patch: &UpdateRhythm,
    ) -> StoreResult<Option<(Rhythm, usize)>> {
        let mut rhythm = match self.get_rhythm(id)? {
            Some(rhythm) => rhythm,
            None => return Ok(None),
        };
        if let Some(kind) = &patch.kind {
            rhythm.kind = kind.clone();
        }
        if let Some(title) = &patch.title {
            rhythm.title = title.trim().to_string();
        }
        if let Some(location) = &patch.location {
            rhythm.location = Some(location.clone());
        }
        if let Some(byweekday) = &patch.byweekday {
            rhythm.byweekday = byweekday.clone();
        }
        if let Some(start_time) = &patch.start_time {
            rhythm.start_time = Some(start_time.clone());
        }
        if let Some(end_time) = &patch.end_time {
            rhythm.end_time = Some(end_time.clone());
        }
        if let Some(valid_from) = &patch.valid_from {
            rhythm.valid_from = valid_from.clone();
        }
        if let Some(valid_until) = &patch.valid_until {
            rhythm.valid_until = valid_until.clone();
        }
        if let Some(active) = patch.active {
            rhythm.active = active;
        }
        crate::model::validate_rhythm_fields(
            &rhythm.kind,
            &rhythm.title,
            &rhythm.byweekday,
            rhythm.start_time.as_deref(),
            rhythm.end_time.as_deref(),
            &rhythm.valid_from,
            &rhythm.valid_until,
        )?;
        rhythm.updated_at = now_text();
        let mut conn = self.conn.lock().unwrap();
        let mut tx = conn.transaction()?;
        tx.execute(
            &format!(
                "UPDATE {schema}.rhythms SET kind=$2, title=$3, location=$4, byweekday=$5, \
                 start_time=$6, end_time=$7, valid_from=$8, valid_until=$9, active=$10, \
                 updated_at=$11 WHERE id=$1",
                schema = self.schema
            ),
            &[
                &rhythm.id,
                &rhythm.kind,
                &rhythm.title,
                &rhythm.location,
                &rhythm.byweekday.join(","),
                &rhythm.start_time,
                &rhythm.end_time,
                &rhythm.valid_from,
                &rhythm.valid_until,
                &(rhythm.active as i32),
                &rhythm.updated_at,
            ],
        )?;
        let created = if rhythm.active {
            delete_future_instances(&mut tx, &self.schema, &rhythm.id)?;
            insert_instances(&mut tx, &self.schema, &rhythm)?
        } else {
            // Pausing a rhythm clears its future, keeps its history.
            delete_future_instances(&mut tx, &self.schema, &rhythm.id)?
        };
        tx.commit()?;
        Ok(Some((rhythm, created)))
    }

    /// Deletes the rhythm. With `delete_instances`, future generated instances
    /// go too; otherwise they stay (the FK sets their rhythm_id NULL) as
    /// ordinary manual-looking entries.
    pub fn delete_rhythm(&self, id: &str, delete_instances: bool) -> StoreResult<bool> {
        let mut conn = self.conn.lock().unwrap();
        let mut tx = conn.transaction()?;
        if delete_instances {
            delete_future_instances(&mut tx, &self.schema, id)?;
        }
        let count = tx.execute(
            &format!(
                "DELETE FROM {schema}.rhythms WHERE id = $1",
                schema = self.schema
            ),
            &[&id],
        )?;
        tx.commit()?;
        Ok(count > 0)
    }

    /// Explicit re-materialization (POST /api/rhythms/:id/materialize).
    /// Idempotent: the (rhythm_id, starts_at) slot unique index turns repeats
    /// into no-ops, and unlike update_rhythm it does not delete anything.
    pub fn materialize_rhythm(&self, id: &str) -> StoreResult<Option<usize>> {
        let rhythm = match self.get_rhythm(id)? {
            Some(rhythm) if rhythm.active => rhythm,
            Some(_) => return Ok(Some(0)),
            None => return Ok(None),
        };
        let mut conn = self.conn.lock().unwrap();
        let mut tx = conn.transaction()?;
        let created = insert_instances(&mut tx, &self.schema, &rhythm)?;
        tx.commit()?;
        Ok(Some(created))
    }
}

fn delete_future_instances(
    tx: &mut postgres::Transaction,
    schema: &str,
    rhythm_id: &str,
) -> StoreResult<usize> {
    let today = date::format_date(date::today_days());
    let count = tx.execute(
        &format!(
            "DELETE FROM {schema}.entries \
             WHERE rhythm_id = $1 AND substr(starts_at, 1, 10) >= $2",
            schema = schema
        ),
        &[&rhythm_id, &today],
    )?;
    Ok(count as usize)
}

fn insert_instances(
    tx: &mut postgres::Transaction,
    schema: &str,
    rhythm: &Rhythm,
) -> StoreResult<usize> {
    let now = now_text();
    let mut created = 0;
    for instance in rhythm::instance_entries(rhythm, date::today_days())? {
        let affected = tx.execute(
            &format!(
                "INSERT INTO {schema}.entries \
                 (id, kind, title, starts_at, ends_at, all_day, location, notes, \
                  source, external_id, rhythm_id, payload, created_at, updated_at) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14) \
                 ON CONFLICT (rhythm_id, starts_at) WHERE rhythm_id IS NOT NULL DO NOTHING",
                schema = schema
            ),
            &[
                &generated_id("cal:entry"),
                &instance.kind,
                &instance.title,
                &instance.starts_at,
                &instance.ends_at,
                &(instance.all_day as i32),
                &instance.location,
                &instance.notes,
                &instance.source,
                &instance.external_id,
                &instance.rhythm_id,
                &serde_json::to_string(&instance.payload)?,
                &now,
                &now,
            ],
        )?;
        created += affected as usize;
    }
    Ok(created)
}

fn entry_as_new(entry: &Entry) -> NewEntry {
    NewEntry {
        kind: entry.kind.clone(),
        commitment: entry.commitment,
        title: entry.title.clone(),
        starts_at: entry.starts_at.clone(),
        ends_at: entry.ends_at.clone(),
        all_day: entry.all_day,
        location: entry.location.clone(),
        notes: entry.notes.clone(),
        source: entry.source.clone(),
        external_id: entry.external_id.clone(),
        rhythm_id: entry.rhythm_id.clone(),
        payload: entry.payload.clone(),
    }
}

fn context_as_new(context: &Context) -> NewContext {
    NewContext {
        kind: context.kind.clone(),
        title: context.title.clone(),
        details: context.details.clone(),
        valid_from: context.valid_from.clone(),
        valid_until: context.valid_until.clone(),
        source: context.source.clone(),
    }
}

fn day_window_bounds(from: &str, to: &str) -> StoreResult<(String, String)> {
    let from_day = date::parse_date(from).ok_or("from must be YYYY-MM-DD")?;
    let to_day = date::parse_date(to).ok_or("to must be YYYY-MM-DD")?;
    if to_day < from_day {
        return Err("to must be on or after from".into());
    }
    Ok((format!("{from}T00:00:00"), format!("{to}T00:00:00")))
}

fn entry_from_input(input: &NewEntry) -> Entry {
    let now = now_text();
    Entry {
        id: generated_id("cal:entry"),
        kind: input.kind.clone(),
        commitment: input.commitment,
        title: input.title.trim().to_string(),
        starts_at: input.starts_at.clone(),
        ends_at: input.ends_at.clone(),
        all_day: input.all_day,
        location: input.location.clone(),
        notes: input.notes.clone(),
        source: input.source.clone(),
        external_id: input.external_id.clone(),
        rhythm_id: input.rhythm_id.clone(),
        payload: input.payload.clone(),
        created_at: now.clone(),
        updated_at: now,
    }
}

fn context_from_input(input: &NewContext) -> Context {
    let now = now_text();
    Context {
        id: generated_id("cal:context"),
        kind: input.kind.clone(),
        title: input.title.trim().to_string(),
        details: input.details.trim().to_string(),
        valid_from: input.valid_from.clone(),
        valid_until: input.valid_until.clone(),
        source: input.source.clone(),
        created_at: now.clone(),
        updated_at: now,
    }
}

fn entry_from_row(row: &Row) -> StoreResult<Entry> {
    let payload: String = row.get("payload");
    Ok(Entry {
        id: row.get("id"),
        kind: row.get("kind"),
        commitment: Commitment::from_db(row.get("commitment")),
        title: row.get("title"),
        starts_at: row.get("starts_at"),
        ends_at: row.get("ends_at"),
        all_day: row.get::<_, i32>("all_day") != 0,
        location: row.get("location"),
        notes: row.get("notes"),
        source: row.get("source"),
        external_id: row.get("external_id"),
        rhythm_id: row.get("rhythm_id"),
        payload: serde_json::from_str(&payload).unwrap_or(Value::Null),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn context_from_row(row: &Row) -> StoreResult<Context> {
    Ok(Context {
        id: row.get("id"),
        kind: row.get("kind"),
        title: row.get("title"),
        details: row.get("details"),
        valid_from: row.get("valid_from"),
        valid_until: row.get("valid_until"),
        source: row.get("source"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn export_from_row(row: &Row) -> StoreResult<ExportOptIn> {
    Ok(ExportOptIn {
        entry_id: row.get("entry_id"),
        google_calendar_id: row.get("google_calendar_id"),
        google_event_id: row.get("google_event_id"),
        pushed_at: row.get("pushed_at"),
        created_at: row.get("created_at"),
    })
}

fn rhythm_from_row(row: &Row) -> StoreResult<Rhythm> {
    let byweekday: String = row.get("byweekday");
    Ok(Rhythm {
        id: row.get("id"),
        kind: row.get("kind"),
        title: row.get("title"),
        location: row.get("location"),
        byweekday: byweekday
            .split(',')
            .map(|token| token.trim().to_string())
            .filter(|token| !token.is_empty())
            .collect(),
        start_time: row.get("start_time"),
        end_time: row.get("end_time"),
        valid_from: row.get("valid_from"),
        valid_until: row.get("valid_until"),
        active: row.get::<_, i32>("active") != 0,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_ids_unique() {
        let first = generated_id("cal:entry");
        let second = generated_id("cal:entry");
        assert!(first.starts_with("cal:entry:"));
        assert_ne!(first, second);
    }

    #[test]
    fn schema_names_are_restricted() {
        assert!(validate_schema("calendar_test").is_ok());
        assert!(validate_schema("calendar; DROP SCHEMA public").is_err());
    }

    #[test]
    fn day_window_uses_midnight_boundaries_for_timed_entries() {
        let (from, to) = day_window_bounds("2026-07-30", "2026-07-31").unwrap();
        assert_eq!(from, "2026-07-30T00:00:00");
        assert_eq!(to, "2026-07-31T00:00:00");

        let same_day_end = "2026-07-30T12:00:00";
        assert!(same_day_end > from.as_str());
    }
}
