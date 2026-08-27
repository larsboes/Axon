//! Task records and their provenance.
//!
//! The whole capability exists for one sentence in the Gmail router doctrine:
//! a mail that needs doing becomes *exactly one* owned record, and the mail
//! stops being the thing you track. "Exactly one" is a unique index here, not
//! a convention — a convention gets a duplicate the first time a sweep runs
//! twice, and then the inbox is authoritative again and nothing was gained.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use axon_store::QueryAll;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

pub const STATUSES: [&str; 3] = ["open", "done", "dropped"];

/// One thing to do, and where it came from.
///
/// `source_*` is not decoration. Without a way back to the mail, a task is a
/// sentence someone typed once, and the first question about it ("what did
/// they actually ask for?") has no answer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub status: String,
    pub due: Option<String>,
    pub note: Option<String>,
    /// Which capability observed this, e.g. `comms`. `None` for one typed by
    /// hand — those are tasks too, and refusing them would push the operator
    /// back into a second list.
    pub source_capability: Option<String>,
    pub source_id: Option<String>,
    pub source_url: Option<String>,
    /// Inherited from the source, never re-derived. A task promoted from a
    /// Private mail is Private: the subject travelled into the title.
    pub data_class: String,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewTask {
    pub title: String,
    #[serde(default)]
    pub due: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub source_capability: Option<String>,
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub source_url: Option<String>,
    #[serde(default)]
    pub data_class: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct TaskPatch {
    pub title: Option<String>,
    pub status: Option<String>,
    /// Present-but-null clears the field; absent leaves it. Without the double
    /// option, "remove the due date" is unexpressible and the caller has to
    /// invent a sentinel.
    #[serde(default, deserialize_with = "double_option")]
    pub due: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub note: Option<Option<String>>,
}

fn double_option<'de, D>(deserializer: D) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer).map(Some)
}

pub struct Store {
    /// Shared with every other store in this process on the same file, so
    /// opening one is a checkout rather than an open.
    pool: axon_store::Pool,
    /// Prefixes this capability's tables in the one shared file (PRD Q45):
    /// `tasks` here means the table `tasks_tasks`.
    prefix: String,
}

impl Store {
    pub fn open(database_path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        Self::open_with_prefix(database_path, "tasks")
    }

    pub fn open_with_prefix(
        database_path: &Path,
        prefix: &str,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        validate_prefix(prefix)?;
        // A pool checkout, and the migration runs once per process per (file,
        // prefix) rather than once per open -- libs/axon-store/README.md has why.
        let pool = axon_store::open_pool(database_path, prefix, |conn| {
            Self::run_migration(conn, prefix)
        })?;
        Ok(Self {
            pool,
            prefix: prefix.to_string(),
        })
    }

    /// A connection from the shared pool, for the duration of one statement.
    ///
    /// A `Result` where this used to be `self.conn.lock().unwrap()`: that unwrap
    /// could only fail on a poisoned mutex, whereas a checkout can genuinely fail
    /// when the file is unreachable or every connection is busy.
    fn conn(&self) -> Result<axon_store::PooledClient, Box<dyn std::error::Error>> {
        Ok(self.pool.get()?)
    }

    fn run_migration(conn: &Connection, prefix: &str) -> Result<(), Box<dyn std::error::Error>> {
        conn.execute_batch(&format!(
            "
            CREATE TABLE IF NOT EXISTS {prefix}_tasks (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'open'
                    CHECK (status IN ('open','done','dropped')),
                due TEXT,
                note TEXT,
                source_capability TEXT,
                source_id TEXT,
                source_url TEXT,
                data_class TEXT NOT NULL DEFAULT 'personal'
                    CHECK (data_class IN ('public','personal','vault')),
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                completed_at TEXT
            );

            -- The doctrine's 'exactly one owned record', enforced. Partial so
            -- hand-written tasks, which have no source, are not all collapsed
            -- into a single row by their shared NULL.
            CREATE UNIQUE INDEX IF NOT EXISTS {prefix}_tasks_one_per_source
                ON {prefix}_tasks (source_capability, source_id)
                WHERE source_capability IS NOT NULL AND source_id IS NOT NULL;
            ",
            prefix = prefix
        ))?;
        Ok(())
    }

    /// Create, or return what already exists for this source.
    ///
    /// The bool is "this is new". A promote that finds an existing row is the
    /// expected result of pressing the button twice, not an error — and it
    /// must not overwrite a title the operator has since corrected.
    pub fn create(&self, new: &NewTask) -> Result<(Task, bool), Box<dyn std::error::Error>> {
        let title = new.title.trim();
        if title.is_empty() {
            return Err("a task needs a title".into());
        }
        let data_class = new.data_class.as_deref().unwrap_or("personal");
        if !crate::content_item::valid(data_class) {
            return Err(format!("invalid data class '{data_class}'").into());
        }

        if let (Some(capability), Some(source_id)) =
            (new.source_capability.as_deref(), new.source_id.as_deref())
        {
            if let Some(existing) = self.find_by_source(capability, source_id)? {
                return Ok((existing, false));
            }
        }

        let id = task_id(new, title);
        let conn = self.conn()?;
        let task = conn.query_row(
            &format!(
                "INSERT INTO {prefix}_tasks
                     (id, title, status, due, note, source_capability, source_id,
                      source_url, data_class, created_at, updated_at)
                 VALUES (?1, ?2, 'open', ?3, ?4, ?5, ?6, ?7, ?8, {now}, {now})
                 RETURNING {columns}",
                prefix = self.prefix,
                now = axon_store::NOW,
                columns = COLUMNS
            ),
            params![
                &id,
                &title,
                &new.due,
                &new.note,
                &new.source_capability,
                &new.source_id,
                &new.source_url,
                &data_class,
            ],
            row_to_task,
        )?;
        Ok((task, true))
    }

    pub fn find_by_source(
        &self,
        capability: &str,
        source_id: &str,
    ) -> Result<Option<Task>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        let task = conn
            .query_row(
                &format!(
                    "SELECT {columns} FROM {prefix}_tasks
                     WHERE source_capability = ?1 AND source_id = ?2",
                    prefix = self.prefix,
                    columns = COLUMNS
                ),
                params![&capability, &source_id],
                row_to_task,
            )
            .optional()?;
        Ok(task)
    }

    pub fn get(&self, id: &str) -> Result<Option<Task>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        let task = conn
            .query_row(
                &format!(
                    "SELECT {columns} FROM {prefix}_tasks WHERE id = ?1",
                    prefix = self.prefix,
                    columns = COLUMNS
                ),
                params![&id],
                row_to_task,
            )
            .optional()?;
        Ok(task)
    }

    /// Open tasks first and by due date, because that is the order the list is
    /// read in. Anything already decided sorts after, newest first.
    pub fn list(&self, status: Option<&str>) -> Result<Vec<Task>, Box<dyn std::error::Error>> {
        if let Some(value) = status {
            if !STATUSES.contains(&value) {
                return Err(format!("invalid status '{value}'").into());
            }
        }
        let conn = self.conn()?;
        Ok(conn.query_all(
            &format!(
                "SELECT {columns} FROM {prefix}_tasks
                 WHERE ?1 IS NULL OR status = ?1
                 ORDER BY (status = 'open') DESC,
                          due NULLS LAST,
                          updated_at DESC",
                prefix = self.prefix,
                columns = COLUMNS
            ),
            params![&status],
            row_to_task,
        )?)
    }

    pub fn patch(
        &self,
        id: &str,
        patch: &TaskPatch,
    ) -> Result<Option<Task>, Box<dyn std::error::Error>> {
        if let Some(status) = patch.status.as_deref() {
            if !STATUSES.contains(&status) {
                return Err(format!("invalid status '{status}'").into());
            }
        }
        if let Some(title) = patch.title.as_deref() {
            if title.trim().is_empty() {
                return Err("a task needs a title".into());
            }
        }

        let conn = self.conn()?;
        // COALESCE for the plain fields, and the double option for the two
        // that can be cleared: `?n IS NULL` cannot distinguish "leave it" from
        // "clear it", so the flag decides which the caller meant.
        let task = conn
            .query_row(
                &format!(
                    "UPDATE {prefix}_tasks SET
                     title = COALESCE(?2, title),
                     status = COALESCE(?3, status),
                     due = CASE WHEN ?4 THEN ?5 ELSE due END,
                     note = CASE WHEN ?6 THEN ?7 ELSE note END,
                     updated_at = {now},
                     completed_at = CASE
                         WHEN COALESCE(?3, status) = 'done' AND completed_at IS NULL THEN {now}
                         WHEN COALESCE(?3, status) <> 'done' THEN NULL
                         ELSE completed_at END
                 WHERE id = ?1
                 RETURNING {columns}",
                    prefix = self.prefix,
                    now = axon_store::NOW,
                    columns = COLUMNS
                ),
                params![
                    &id,
                    &patch.title.as_deref().map(str::trim),
                    &patch.status,
                    &patch.due.is_some(),
                    &patch.due.clone().flatten(),
                    &patch.note.is_some(),
                    &patch.note.clone().flatten(),
                ],
                row_to_task,
            )
            .optional()?;
        Ok(task)
    }

    /// The cheapest statement that proves this store can actually reach its database.
    ///
    /// A checkout from the pool is not enough on its own — the point is to fail exactly when a
    /// real query would, which is what the readiness surface promises its caller (#126).
    pub fn ping(&self) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        conn.query_row("SELECT 1", [], |row| row.get::<_, i64>(0))?;
        Ok(())
    }

    pub fn counts(&self) -> Result<(i64, i64), Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        // `date('now')` is UTC, which is what `to_char(now(), 'YYYY-MM-DD')`
        // rendered here too -- the instance ran in UTC.
        Ok(conn.query_row(
            &format!(
                "SELECT COUNT(*) FILTER (WHERE status = 'open'),
                        COUNT(*) FILTER (WHERE status = 'open' AND due IS NOT NULL
                                         AND due <= date('now'))
                 FROM {prefix}_tasks",
                prefix = self.prefix
            ),
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?)
    }
}

const COLUMNS: &str = "id, title, status, due, note, source_capability, source_id,
     source_url, data_class, created_at, updated_at, completed_at";

fn row_to_task(r: &Row) -> rusqlite::Result<Task> {
    Ok(Task {
        id: r.get(0)?,
        title: r.get(1)?,
        status: r.get(2)?,
        due: r.get(3)?,
        note: r.get(4)?,
        source_capability: r.get(5)?,
        source_id: r.get(6)?,
        source_url: r.get(7)?,
        data_class: r.get(8)?,
        created_at: r.get(9)?,
        updated_at: r.get(10)?,
        completed_at: r.get(11)?,
    })
}

/// Derived from the source when there is one, so the id is stable across a
/// re-promote even if the unique index were ever dropped. Hand-written tasks
/// fall back to a clock-based id; they have no natural key to derive from.
fn task_id(new: &NewTask, title: &str) -> String {
    match (new.source_capability.as_deref(), new.source_id.as_deref()) {
        (Some(capability), Some(source_id)) => format!("{capability}:{source_id}"),
        _ => {
            let micros = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_micros())
                .unwrap_or(0);
            let slug: String = title
                .chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .take(12)
                .collect::<String>()
                .to_ascii_lowercase();
            format!("manual:{micros}:{slug}")
        }
    }
}

/// The prefix is interpolated into DDL and every statement, so it is checked
/// rather than trusted. Production passes the literal `tasks`; only a test
/// passes anything else.
fn validate_prefix(prefix: &str) -> Result<(), Box<dyn std::error::Error>> {
    let ok = !prefix.is_empty()
        && prefix
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
        && prefix.chars().next().is_some_and(|c| !c.is_ascii_digit());
    if ok {
        Ok(())
    } else {
        Err(format!("unsafe table prefix '{prefix}'").into())
    }
}

/// Database-backed; named for the selector CI splits on — see
/// `capabilities/scouting/src/store.rs` for why the name is the contract. It
/// was `postgres_tests` until PRD Q45; the suite needs a temp file now, not a
/// server.
#[cfg(test)]
mod db_tests {
    use super::*;

    /// A file per test, in a directory this process owns. No shared server, so
    /// two `cargo test` runs cannot truncate each other's rows mid-assertion,
    /// and a panicking test leaves nothing behind that a later run inherits.
    fn open_test_store(suffix: &str) -> Store {
        let dir = std::env::temp_dir().join(format!("tasks-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a writable temp directory");
        let path = dir.join(format!("{suffix}.db"));
        // The directory is named by pid, and a pid is recycled eventually. A
        // previous run's rows arriving in this one is the failure the old
        // TRUNCATE prevented, so the file starts empty.
        for tail in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{tail}", path.display()));
        }
        Store::open(&path)
            .unwrap_or_else(|e| panic!("could not open test store at {}: {e}", path.display()))
    }

    /// The readiness probe has to reach the database, not merely hold a pool handle —
    /// a check that passes without touching the file is the bug #126 is about.
    #[test]
    fn ping_reaches_the_database() {
        let store = open_test_store("ping");
        store.ping().expect("a live store answers its own ping");
    }

    #[test]
    fn a_store_cannot_be_opened_against_an_unusable_path() {
        // A file where a directory has to be. The readiness handler turns
        // exactly this into a 503, the way it did for a stopped container.
        let blocker = std::env::temp_dir().join(format!("tasks-blocker-{}", std::process::id()));
        std::fs::write(&blocker, b"not a directory").unwrap();
        let error = Store::open(&blocker.join("axon.db"));
        assert!(error.is_err(), "an unusable path opened anyway");
    }

    fn from_mail(source_id: &str, title: &str) -> NewTask {
        NewTask {
            title: title.into(),
            due: None,
            note: None,
            source_capability: Some("comms".into()),
            source_id: Some(source_id.into()),
            source_url: Some(format!("https://mail.google.com/mail/u/0/#all/{source_id}")),
            data_class: Some("personal".into()),
        }
    }

    /// The reason this capability exists. Promoting the same mail twice is the
    /// normal consequence of a button that can be pressed twice, and it has to
    /// yield one record — otherwise the inbox is authoritative again.
    #[test]
    fn one_mail_yields_exactly_one_task() {
        let store = open_test_store("one_per_source");

        let (first, created) = store
            .create(&from_mail("thread-1", "Reply to the landlord"))
            .unwrap();
        assert!(created);

        let (second, created_again) = store
            .create(&from_mail("thread-1", "Reply to the landlord"))
            .unwrap();
        assert!(
            !created_again,
            "a second promote must not create a second task"
        );
        assert_eq!(first.id, second.id);
        assert_eq!(store.list(None).unwrap().len(), 1);
    }

    /// A corrected title is the operator's, and a re-promote must not quietly
    /// restore the subject line they rejected.
    #[test]
    fn a_repromote_does_not_overwrite_an_edited_title() {
        let store = open_test_store("repromote");
        let (task, _) = store
            .create(&from_mail("thread-2", "Re: Fwd: RE: invoice??"))
            .unwrap();
        store
            .patch(
                &task.id,
                &TaskPatch {
                    title: Some("Pay the January invoice".into()),
                    ..Default::default()
                },
            )
            .unwrap();

        let (again, created) = store
            .create(&from_mail("thread-2", "Re: Fwd: RE: invoice??"))
            .unwrap();
        assert!(!created);
        assert_eq!(again.title, "Pay the January invoice");
    }

    /// Hand-written tasks share a NULL source. A plain unique index would
    /// collapse them all into one row, so the index is partial — and this is
    /// the test that fails if someone "simplifies" it.
    #[test]
    fn tasks_without_a_source_do_not_collide() {
        let store = open_test_store("manual");
        for title in ["Book the dentist", "Renew the passport", "Call Oma"] {
            let (_, created) = store
                .create(&NewTask {
                    title: title.into(),
                    due: None,
                    note: None,
                    source_capability: None,
                    source_id: None,
                    source_url: None,
                    data_class: None,
                })
                .unwrap();
            assert!(created, "{title} should be its own task");
        }
        assert_eq!(store.list(None).unwrap().len(), 3);
    }

    #[test]
    fn completing_stamps_a_time_and_reopening_clears_it() {
        let store = open_test_store("completion");
        let (task, _) = store
            .create(&from_mail("thread-3", "Send the form"))
            .unwrap();
        assert!(task.completed_at.is_none());

        let done = store
            .patch(
                &task.id,
                &TaskPatch {
                    status: Some("done".into()),
                    ..Default::default()
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(done.status, "done");
        let stamped = done.completed_at.clone().expect("done stamps a time");

        let reopened = store
            .patch(
                &task.id,
                &TaskPatch {
                    status: Some("open".into()),
                    ..Default::default()
                },
            )
            .unwrap()
            .unwrap();
        assert!(
            reopened.completed_at.is_none(),
            "reopening must clear the completion stamp, not keep {stamped}"
        );
    }

    /// Absent and present-but-null are different requests, and collapsing them
    /// makes "remove the due date" unexpressible.
    #[test]
    fn a_null_due_clears_it_but_an_absent_one_leaves_it() {
        let store = open_test_store("patch_due");
        let mut new = from_mail("thread-4", "Renew the Bahncard");
        new.due = Some("2026-09-01".into());
        let (task, _) = store.create(&new).unwrap();

        let untouched = store
            .patch(
                &task.id,
                &TaskPatch {
                    note: Some(Some("before the trip".into())),
                    ..Default::default()
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(untouched.due.as_deref(), Some("2026-09-01"));

        let cleared = store
            .patch(
                &task.id,
                &TaskPatch {
                    due: Some(None),
                    ..Default::default()
                },
            )
            .unwrap()
            .unwrap();
        assert!(cleared.due.is_none());
        assert_eq!(cleared.note.as_deref(), Some("before the trip"));
    }

    /// A task promoted from a Private mail carries the subject in its title,
    /// so it inherits the class rather than defaulting to Personal.
    #[test]
    fn the_data_class_is_inherited_not_defaulted() {
        let store = open_test_store("data_class");
        let mut new = from_mail("thread-5", "Tax assessment [number]");
        new.data_class = Some("vault".into());
        let (task, _) = store.create(&new).unwrap();
        assert_eq!(task.data_class, "vault");

        new.source_id = Some("thread-6".into());
        new.data_class = Some("nonsense".into());
        assert!(store.create(&new).is_err(), "an unknown class is refused");
    }

    #[test]
    fn open_tasks_sort_before_decided_ones_and_by_due_date() {
        let store = open_test_store("ordering");
        let mut late = from_mail("thread-7", "Later");
        late.due = Some("2026-12-01".into());
        let mut soon = from_mail("thread-8", "Sooner");
        soon.due = Some("2026-08-10".into());
        store.create(&late).unwrap();
        store.create(&soon).unwrap();
        let (undated, _) = store.create(&from_mail("thread-9", "Undated")).unwrap();
        store
            .patch(
                &undated.id,
                &TaskPatch {
                    status: Some("done".into()),
                    ..Default::default()
                },
            )
            .unwrap();

        let titles: Vec<String> = store
            .list(None)
            .unwrap()
            .into_iter()
            .map(|t| t.title)
            .collect();
        assert_eq!(titles, vec!["Sooner", "Later", "Undated"]);
        assert_eq!(store.list(Some("open")).unwrap().len(), 2);
        assert!(store.list(Some("bogus")).is_err());
    }
}
