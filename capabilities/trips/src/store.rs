use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use postgres::{Client, Row};
use serde::{Deserialize, Serialize};
use serde_json::Value;

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlaceKind {
    Address,
    Airport,
    City,
    #[default]
    Station,
    Venue,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct PlaceRef {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub kind: PlaceKind,
    #[serde(default)]
    pub address: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TransportMode {
    Bike,
    Bus,
    Car,
    Ferry,
    Flight,
    Train,
    Walk,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StageStatus {
    Booked,
    Completed,
    OptionSelected,
    #[default]
    Planning,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct TripStage {
    pub id: String,
    pub sequence: usize,
    pub origin: PlaceRef,
    pub destination: PlaceRef,
    pub date: Option<String>,
    #[serde(default)]
    pub transport_modes: Vec<TransportMode>,
    #[serde(default)]
    pub travelers: Vec<String>,
    #[serde(default)]
    pub status: StageStatus,
    #[serde(default)]
    pub selected_option_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct PlanSource {
    pub kind: String,
    pub reference: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct CreatePlan {
    pub title: String,
    pub origin: PlaceRef,
    pub destinations: Vec<PlaceRef>,
    pub date_start: String,
    pub date_end: String,
    #[serde(default)]
    pub interests: String,
    #[serde(default)]
    pub travelers: Vec<String>,
    #[serde(default)]
    pub transport_modes: Vec<TransportMode>,
    #[serde(default)]
    pub stages: Vec<TripStage>,
    #[serde(default)]
    pub cover_image_url: Option<String>,
    #[serde(default)]
    pub source: Option<PlanSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct UpdatePlan {
    pub title: Option<String>,
    pub origin: Option<PlaceRef>,
    pub destinations: Option<Vec<PlaceRef>>,
    pub date_start: Option<String>,
    pub date_end: Option<String>,
    pub interests: Option<String>,
    pub status: Option<String>,
    pub travelers: Option<Vec<String>>,
    pub transport_modes: Option<Vec<TransportMode>>,
    pub stages: Option<Vec<TripStage>>,
    pub cover_image_url: Option<String>,
    /// What the trip is meant to cost, in minor units. Finance keeps every
    /// actual cent and already tags postings with an `axon-trip-id`; this is the
    /// intention those actuals get compared against, which had no home at all.
    pub budget_cents: Option<i64>,
    pub currency: Option<String>,
    /// The `updated_at` the caller believes it is editing. Omitted keeps the old
    /// last-write-wins behaviour, so nothing that already works breaks.
    ///
    /// `stages` is accepted wholesale, so changing one stage means reading the
    /// plan, editing the array and writing it back. A browser holds that read for
    /// milliseconds. An agent holds it across turns while it calls transit and
    /// reasons, and every stage another writer changed in between is silently
    /// reverted. Same shape as comms' 409-on-hash-mismatch and calendar
    /// rejecting a changed Google revision.
    pub expected_updated_at: Option<String>,
}

/// A plan revision the caller did not expect is a lost update waiting to happen.
///
/// The error text carries `stale_plan` so a caller can branch on it without
/// parsing prose, and names both revisions so it can tell "someone else wrote"
/// apart from "I sent the wrong id".
fn check_expected_revision(
    expected: Option<&str>,
    actual: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    match expected {
        Some(expected) if expected != actual => Err(format!(
            "stale_plan: expected_updated_at {expected} but the plan is at {actual}; \
             re-read it and re-apply your change"
        )
        .into()),
        _ => Ok(()),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TripPlan {
    pub id: String,
    pub title: String,
    pub origin: PlaceRef,
    pub destinations: Vec<PlaceRef>,
    pub date_start: String,
    pub date_end: String,
    pub interests: String,
    pub status: String,
    pub travelers: Vec<String>,
    pub transport_modes: Vec<TransportMode>,
    pub stages: Vec<TripStage>,
    pub cover_image_url: Option<String>,
    pub source: Option<PlanSource>,
    pub created_at: String,
    pub updated_at: String,
    /// What this trip is meant to cost, in minor units, beside what it actually
    /// did. Finance keeps every actual cent and already tags postings with an
    /// `axon-trip-id`; the intention had no home at all, so the two halves of
    /// "did I overspend" were one HTTP call apart and never compared.
    #[serde(default)]
    pub budget_cents: Option<i64>,
    #[serde(default)]
    pub currency: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct CreatePlanItem {
    pub item_type: String,
    pub day: Option<String>,
    pub external_id: String,
    pub title: String,
    #[serde(default)]
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlanItem {
    pub id: String,
    pub plan_id: String,
    pub item_type: String,
    pub day: Option<String>,
    pub external_id: String,
    pub title: String,
    pub payload: Value,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlanDetails {
    #[serde(flatten)]
    pub plan: TripPlan,
    pub items: Vec<PlanItem>,
}

pub struct TripsStore {
    /// Shared with every other store in this process on the same database, so
    /// opening one is a checkout rather than a connect.
    pool: axon_store::Pool,
    schema: String,
}

fn generated_id(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}:{nanos:x}{sequence:04x}")
}

fn now_text() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    seconds.to_string()
}

fn generated_stages(input: &CreatePlan) -> Vec<TripStage> {
    let mut previous = input.origin.clone();
    input
        .destinations
        .iter()
        .enumerate()
        .map(|(sequence, destination)| {
            let stage = TripStage {
                id: format!("stage:{}", sequence + 1),
                sequence,
                origin: previous.clone(),
                destination: destination.clone(),
                date: Some(input.date_start.clone()),
                transport_modes: input.transport_modes.clone(),
                travelers: input.travelers.clone(),
                status: StageStatus::Planning,
                selected_option_id: None,
            };
            previous = destination.clone();
            stage
        })
        .collect()
}

fn validate_plan_fields(
    title: &str,
    destinations: &[PlaceRef],
    date_start: &str,
    date_end: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if title.trim().is_empty() {
        return Err("title is required".into());
    }
    if destinations.is_empty() || destinations.len() > 4 {
        return Err("choose between one and four destinations".into());
    }
    if date_start > date_end {
        return Err("date_start must be before or equal to date_end".into());
    }
    Ok(())
}

fn validate_schema(schema: &str) -> Result<(), Box<dyn std::error::Error>> {
    if !schema
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        return Err("schema must contain only ASCII letters, digits, or underscore".into());
    }
    Ok(())
}

impl TripsStore {
    pub fn open(database_url: &str) -> Result<Self, Box<dyn std::error::Error>> {
        Self::open_in_schema(database_url, "trips")
    }

    pub fn open_in_schema(
        database_url: &str,
        schema: &str,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        validate_schema(schema)?;
        // A pool checkout, not a connect, and the migration runs once per process
        // per (database, schema) rather than once per open. Both halves of the
        // Store::open problem -- libs/axon-store/README.md has the numbers.
        let pool = axon_store::open_pool(database_url, schema, |conn| {
            Self::run_migration(conn, schema)
        })?;
        Ok(Self {
            pool,
            schema: schema.to_string(),
        })
    }

    /// A connection from the shared pool, for the duration of one statement.
    ///
    /// A `Result` where this used to be `self.conn.lock().unwrap()`: that unwrap
    /// could only fail on a poisoned mutex, whereas a checkout can genuinely fail
    /// when the database is down or every connection is busy.
    fn conn(&self) -> Result<axon_store::PooledClient, Box<dyn std::error::Error>> {
        Ok(self.pool.get()?)
    }

    /// The cheapest statement that proves this store can actually reach its database.
    ///
    /// A checkout from the pool is not enough on its own — the point is to fail exactly when a
    /// real query would, which is what the readiness surface promises its caller (#126).
    pub fn ping(&self) -> Result<(), Box<dyn std::error::Error>> {
        let mut conn = self.conn()?;
        conn.query_one("SELECT 1", &[])?;
        Ok(())
    }

    fn run_migration(conn: &mut Client, schema: &str) -> Result<(), Box<dyn std::error::Error>> {
        conn.batch_execute(&format!(
            "
            CREATE SCHEMA IF NOT EXISTS {schema};
            CREATE TABLE IF NOT EXISTS {schema}.plans (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                origin TEXT NOT NULL,
                destinations TEXT NOT NULL,
                date_start TEXT NOT NULL,
                date_end TEXT NOT NULL,
                interests TEXT NOT NULL DEFAULT '',
                status TEXT NOT NULL DEFAULT 'draft'
                    CHECK (status IN ('draft','saved','archived')),
                travelers TEXT NOT NULL DEFAULT '[]',
                transport_modes TEXT NOT NULL DEFAULT '[]',
                stages TEXT NOT NULL DEFAULT '[]',
                cover_image_url TEXT,
                source_kind TEXT,
                source_ref TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS {schema}.plan_items (
                id TEXT PRIMARY KEY,
                plan_id TEXT NOT NULL REFERENCES {schema}.plans(id) ON DELETE CASCADE,
                item_type TEXT NOT NULL
                    CHECK (item_type IN ('journey','transport','event','activity','place','stay','image','note')),
                day TEXT,
                external_id TEXT NOT NULL,
                title TEXT NOT NULL,
                payload TEXT NOT NULL,
                created_at TEXT NOT NULL,
                UNIQUE (plan_id, item_type, external_id)
            );
            CREATE INDEX IF NOT EXISTS idx_trip_plan_updated
                ON {schema}.plans(updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_trip_item_plan
                ON {schema}.plan_items(plan_id, day, created_at);
            ALTER TABLE {schema}.plans
                ADD COLUMN IF NOT EXISTS travelers TEXT NOT NULL DEFAULT '[]',
                ADD COLUMN IF NOT EXISTS transport_modes TEXT NOT NULL DEFAULT '[]',
                ADD COLUMN IF NOT EXISTS stages TEXT NOT NULL DEFAULT '[]',
                ADD COLUMN IF NOT EXISTS cover_image_url TEXT,
                ADD COLUMN IF NOT EXISTS source_kind TEXT,
                ADD COLUMN IF NOT EXISTS source_ref TEXT,
                ADD COLUMN IF NOT EXISTS budget_cents BIGINT,
                ADD COLUMN IF NOT EXISTS currency TEXT;
            CREATE UNIQUE INDEX IF NOT EXISTS idx_trip_plan_source
                ON {schema}.plans(source_kind, source_ref)
                WHERE source_kind IS NOT NULL AND source_ref IS NOT NULL;
            ALTER TABLE {schema}.plan_items
                DROP CONSTRAINT IF EXISTS plan_items_item_type_check;
            ALTER TABLE {schema}.plan_items
                ADD CONSTRAINT plan_items_item_type_check
                CHECK (item_type IN ('journey','transport','event','activity','place','stay','image','note','option_set','booking','outcome'));
            ",
            schema = schema
        ))?;
        Ok(())
    }

    pub fn create_plan(&self, input: &CreatePlan) -> Result<TripPlan, Box<dyn std::error::Error>> {
        validate_plan_fields(
            &input.title,
            &input.destinations,
            &input.date_start,
            &input.date_end,
        )?;

        let id = generated_id("trip:plan");
        let now = now_text();
        let origin = serde_json::to_string(&input.origin)?;
        let destinations = serde_json::to_string(&input.destinations)?;
        let travelers = serde_json::to_string(&input.travelers)?;
        let transport_modes = serde_json::to_string(&input.transport_modes)?;
        let stages = serde_json::to_string(&if input.stages.is_empty() {
            generated_stages(input)
        } else {
            input.stages.clone()
        })?;
        let source_kind = input.source.as_ref().map(|source| source.kind.as_str());
        let source_ref = input
            .source
            .as_ref()
            .map(|source| source.reference.as_str());
        let mut conn = self.conn()?;
        let row = conn.query_one(
            &format!(
                "INSERT INTO {schema}.plans
                    (id,title,origin,destinations,date_start,date_end,interests,status,travelers,
                     transport_modes,stages,cover_image_url,source_kind,source_ref,created_at,updated_at)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,'draft',$8,$9,$10,$11,$12,$13,$14,$14)
                 RETURNING id,title,origin,destinations,date_start,date_end,interests,status,
                    travelers,transport_modes,stages,cover_image_url,source_kind,source_ref,
                    created_at,updated_at,budget_cents,currency",
                schema = self.schema
            ),
            &[
                &id,
                &input.title.trim(),
                &origin,
                &destinations,
                &input.date_start,
                &input.date_end,
                &input.interests.trim(),
                &travelers,
                &transport_modes,
                &stages,
                &input.cover_image_url,
                &source_kind,
                &source_ref,
                &now,
            ],
        )?;
        row_to_plan(&row)
    }

    pub fn list_plans(&self) -> Result<Vec<TripPlan>, Box<dyn std::error::Error>> {
        let mut conn = self.conn()?;
        let rows = conn.query(
            &format!(
                "SELECT id,title,origin,destinations,date_start,date_end,interests,status,
                        travelers,transport_modes,stages,cover_image_url,source_kind,source_ref,
                        created_at,updated_at,budget_cents,currency
                 FROM {}.plans WHERE status != 'archived'
                 ORDER BY updated_at DESC",
                self.schema
            ),
            &[],
        )?;
        rows.iter().map(row_to_plan).collect()
    }

    pub fn get_plan(&self, id: &str) -> Result<Option<PlanDetails>, Box<dyn std::error::Error>> {
        let mut conn = self.conn()?;
        let Some(row) = conn.query_opt(
            &format!(
                "SELECT id,title,origin,destinations,date_start,date_end,interests,status,
                        travelers,transport_modes,stages,cover_image_url,source_kind,source_ref,
                        created_at,updated_at,budget_cents,currency
                 FROM {}.plans WHERE id = $1",
                self.schema
            ),
            &[&id],
        )?
        else {
            return Ok(None);
        };
        let plan = row_to_plan(&row)?;
        let item_rows = conn.query(
            &format!(
                "SELECT id,plan_id,item_type,day,external_id,title,payload,created_at
                 FROM {}.plan_items WHERE plan_id = $1
                 ORDER BY day NULLS LAST, created_at",
                self.schema
            ),
            &[&id],
        )?;
        let items = item_rows
            .iter()
            .map(row_to_item)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Some(PlanDetails { plan, items }))
    }

    pub fn update_plan(
        &self,
        plan_id: &str,
        input: &UpdatePlan,
    ) -> Result<Option<TripPlan>, Box<dyn std::error::Error>> {
        let Some(details) = self.get_plan(plan_id)? else {
            return Ok(None);
        };
        let current = details.plan;
        check_expected_revision(input.expected_updated_at.as_deref(), &current.updated_at)?;
        let title = input.title.clone().unwrap_or(current.title);
        let origin = input.origin.clone().unwrap_or(current.origin);
        let destinations = input.destinations.clone().unwrap_or(current.destinations);
        let date_start = input.date_start.clone().unwrap_or(current.date_start);
        let date_end = input.date_end.clone().unwrap_or(current.date_end);
        let interests = input.interests.clone().unwrap_or(current.interests);
        let status = input.status.clone().unwrap_or(current.status);
        let travelers = input.travelers.clone().unwrap_or(current.travelers);
        let previous_transport_modes = current.transport_modes.clone();
        let transport_modes = input
            .transport_modes
            .clone()
            .unwrap_or(current.transport_modes);
        let route_changed =
            input.origin.is_some() || input.destinations.is_some() || input.date_start.is_some();
        let stages = input.stages.clone().unwrap_or_else(|| {
            if route_changed {
                generated_stages(&CreatePlan {
                    title: title.clone(),
                    origin: origin.clone(),
                    destinations: destinations.clone(),
                    date_start: date_start.clone(),
                    date_end: date_end.clone(),
                    interests: interests.clone(),
                    travelers: travelers.clone(),
                    transport_modes: transport_modes.clone(),
                    stages: Vec::new(),
                    cover_image_url: current.cover_image_url.clone(),
                    source: current.source.clone(),
                })
            } else if input.transport_modes.is_some() {
                propagate_default_transport_modes(
                    current.stages,
                    &previous_transport_modes,
                    &transport_modes,
                )
            } else {
                current.stages
            }
        });
        let cover_image_url = input.cover_image_url.clone().or(current.cover_image_url);
        let budget_cents = input.budget_cents.or(current.budget_cents);
        let currency = input.currency.clone().or(current.currency);
        if budget_cents.is_some_and(|cents| cents < 0) {
            return Err("budget_cents must not be negative".into());
        }

        validate_plan_fields(&title, &destinations, &date_start, &date_end)?;
        if !["draft", "saved", "archived"].contains(&status.as_str()) {
            return Err("status must be draft, saved, or archived".into());
        }

        let now = now_text();
        let origin_json = serde_json::to_string(&origin)?;
        let destinations_json = serde_json::to_string(&destinations)?;
        let travelers_json = serde_json::to_string(&travelers)?;
        let transport_modes_json = serde_json::to_string(&transport_modes)?;
        let stages_json = serde_json::to_string(&stages)?;
        let mut conn = self.conn()?;
        let row = conn.query_one(
            &format!(
                "UPDATE {schema}.plans SET
                    title=$1,origin=$2,destinations=$3,date_start=$4,date_end=$5,interests=$6,
                    status=$7,travelers=$8,transport_modes=$9,stages=$10,cover_image_url=$11,
                    updated_at=$12,budget_cents=$14,currency=$15
                 WHERE id=$13
                 RETURNING id,title,origin,destinations,date_start,date_end,interests,status,
                    travelers,transport_modes,stages,cover_image_url,source_kind,source_ref,
                    created_at,updated_at,budget_cents,currency",
                schema = self.schema
            ),
            &[
                &title.trim(),
                &origin_json,
                &destinations_json,
                &date_start,
                &date_end,
                &interests.trim(),
                &status,
                &travelers_json,
                &transport_modes_json,
                &stages_json,
                &cover_image_url,
                &now,
                &plan_id,
                &budget_cents,
                &currency,
            ],
        )?;
        Ok(Some(row_to_plan(&row)?))
    }

    /// Moves one item to a day, or clears it.
    ///
    /// `plan_items` has had a `day` column and an index on `(plan_id, day,
    /// created_at)` since the start, and no write path ever decided a value for
    /// it: a saved journey is stamped with the plan's `date_start` even when its
    /// stage runs on a different date, and every saved place gets `null` with
    /// nothing in the system able to fill it in. An index on a column nobody
    /// maintains sorts a multi-day trip into one heap.
    pub fn set_item_day(
        &self,
        plan_id: &str,
        item_id: &str,
        day: Option<&str>,
    ) -> Result<Option<PlanItem>, Box<dyn std::error::Error>> {
        if let Some(day) = day {
            // A date the index can order. Anything else silently sorts wrong.
            if day.len() != 10 || !day.as_bytes().iter().enumerate().all(|(i, b)| {
                if i == 4 || i == 7 {
                    *b == b'-'
                } else {
                    b.is_ascii_digit()
                }
            }) {
                return Err("day must be YYYY-MM-DD, or null to unset it".into());
            }
        }
        let mut conn = self.conn()?;
        let row = conn.query_opt(
            &format!(
                "UPDATE {schema}.plan_items SET day = $3
                 WHERE plan_id = $1 AND id = $2
                 RETURNING id,plan_id,item_type,day,external_id,title,payload,created_at",
                schema = self.schema
            ),
            &[&plan_id, &item_id, &day],
        )?;
        row.as_ref().map(row_to_item).transpose()
    }

    /// Records how a stage actually went, against the intent it was chosen under.
    ///
    /// `StageStatus::Completed` has existed since the start and nothing anywhere
    /// sets it: the past/upcoming split is pure date arithmetic, so the system
    /// has no memory of whether a connection was made, what it really cost, or
    /// whether the transfer was too tight. punctuality knows what every train
    /// does; nothing knows what these trips did.
    ///
    /// A stage with no `selected_option_id` is refused rather than recorded.
    /// With nothing chosen there is nothing to compare an actual against, and the
    /// row would be a hoard rather than a measurement.
    ///
    /// Stored as a plan item so it needs no new table and travels with the plan.
    /// The kill criterion is deliberately cheap to run: if two trips go by
    /// without this being filled in, the whole learning idea is answered and the
    /// endpoint goes away.
    pub fn record_outcome(
        &self,
        plan_id: &str,
        stage_id: &str,
        outcome: &Value,
    ) -> Result<PlanItem, Box<dyn std::error::Error>> {
        let Some(details) = self.get_plan(plan_id)? else {
            return Err("trip plan not found".into());
        };
        let stage = details
            .plan
            .stages
            .iter()
            .find(|stage| stage.id == stage_id)
            .ok_or_else(|| format!("no stage {stage_id} on {plan_id}"))?;
        let Some(selected) = stage.selected_option_id.as_deref() else {
            return Err(format!(
                "stage {stage_id} has no selected_option_id, so there is nothing to compare \
                 an outcome against"
            )
            .into());
        };

        let mut payload = outcome.clone();
        let object = payload
            .as_object_mut()
            .ok_or("outcome must be a JSON object")?;
        // The intent half, copied in at write time. Reading it back later from
        // the stage would compare an actual against whatever the plan says now,
        // which is not what was chosen.
        object.insert("stage_id".into(), Value::String(stage_id.to_string()));
        object.insert(
            "selected_option_id".into(),
            Value::String(selected.to_string()),
        );
        if let Some(date) = stage.date.clone() {
            object.insert("planned_date".into(), Value::String(date));
        }

        self.add_item(
            plan_id,
            &CreatePlanItem {
                item_type: "outcome".into(),
                day: stage.date.clone(),
                external_id: format!("outcome:{stage_id}"),
                title: format!("How {} → {} went", stage.origin.name, stage.destination.name),
                payload,
            },
        )
    }

    /// Where the operator actually goes, computed on read over the plans that
    /// already exist. No new table: a projection cannot drift from its source.
    ///
    /// `merge_candidates` is the part worth having on day one. The dashboard's
    /// place field slugifies typed text into `place:<slug>` with a null
    /// coordinate whenever the operator does not pick a suggested station, so the
    /// same city typed two ways is two places, and every coordinate-dependent
    /// behaviour downstream (the 75 km candidate match, the map, nearby places)
    /// degrades without saying so. This reports the collisions instead of
    /// guessing at a merge, because merging identities is not a read's decision.
    pub fn list_places(&self) -> Result<Value, Box<dyn std::error::Error>> {
        use std::collections::BTreeMap;

        let mut visits: BTreeMap<String, (String, usize, Option<String>, Option<String>, bool)> =
            BTreeMap::new();
        for plan in self.list_plans()? {
            let dated = std::iter::once(&plan.origin)
                .chain(plan.destinations.iter())
                .map(|place| (place, plan.date_start.clone()));
            for (place, date) in dated {
                let entry = visits.entry(place.id.clone()).or_insert_with(|| {
                    (
                        place.name.clone(),
                        0,
                        None,
                        None,
                        place.latitude.is_some() && place.longitude.is_some(),
                    )
                });
                entry.1 += 1;
                entry.2 = Some(match entry.2.take() {
                    Some(first) if first <= date => first,
                    _ => date.clone(),
                });
                entry.3 = Some(match entry.3.take() {
                    Some(last) if last >= date => last,
                    _ => date,
                });
            }
        }

        // Two ids whose names normalise to one string are one place typed twice.
        let mut by_name: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (id, (name, _, _, _, _)) in &visits {
            by_name
                .entry(normalize_place_name(name))
                .or_default()
                .push(id.clone());
        }
        let merge_candidates: Vec<Value> = by_name
            .iter()
            .filter(|(_, ids)| ids.len() > 1)
            .map(|(name, ids)| json_value(name, ids))
            .collect();

        let places: Vec<Value> = visits
            .iter()
            .map(|(id, (name, count, first, last, has_coordinate))| {
                serde_json::json!({
                    "id": id,
                    "name": name,
                    "visits": count,
                    "first": first,
                    "last": last,
                    "has_coordinate": has_coordinate,
                })
            })
            .collect();

        Ok(serde_json::json!({
            "places": places,
            "merge_candidates": merge_candidates,
        }))
    }

    pub fn find_plan_by_source(
        &self,
        kind: &str,
        reference: &str,
    ) -> Result<Option<TripPlan>, Box<dyn std::error::Error>> {
        let mut conn = self.conn()?;
        let row = conn.query_opt(
            &format!(
                "SELECT id,title,origin,destinations,date_start,date_end,interests,status,
                        travelers,transport_modes,stages,cover_image_url,source_kind,source_ref,
                        created_at,updated_at,budget_cents,currency
                 FROM {}.plans WHERE source_kind = $1 AND source_ref = $2",
                self.schema
            ),
            &[&kind, &reference],
        )?;
        row.as_ref().map(row_to_plan).transpose()
    }

    pub fn add_item(
        &self,
        plan_id: &str,
        input: &CreatePlanItem,
    ) -> Result<PlanItem, Box<dyn std::error::Error>> {
        if !ITEM_TYPES.contains(&input.item_type.as_str()) {
            return Err(format!(
                "item_type must be one of {}",
                ITEM_TYPES.join(", ")
            )
            .into());
        }
        if input.external_id.trim().is_empty() || input.title.trim().is_empty() {
            return Err("external_id and title are required".into());
        }
        validate_payload(&input.item_type, &input.payload)?;

        let id = generated_id("trip:item");
        let now = now_text();
        let payload = serde_json::to_string(&input.payload)?;
        let mut conn = self.conn()?;
        let row = conn.query_one(
            &format!(
                "INSERT INTO {schema}.plan_items
                    (id,plan_id,item_type,day,external_id,title,payload,created_at)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
                 ON CONFLICT (plan_id,item_type,external_id) DO UPDATE SET
                    day = excluded.day,
                    title = excluded.title,
                    payload = excluded.payload
                 RETURNING id,plan_id,item_type,day,external_id,title,payload,created_at",
                schema = self.schema
            ),
            &[
                &id,
                &plan_id,
                &input.item_type,
                &input.day,
                &input.external_id,
                &input.title.trim(),
                &payload,
                &now,
            ],
        )?;
        conn.execute(
            &format!(
                "UPDATE {}.plans SET updated_at = $1, status = 'saved' WHERE id = $2",
                self.schema
            ),
            &[&now, &plan_id],
        )?;
        row_to_item(&row)
    }

    pub fn delete_item(
        &self,
        plan_id: &str,
        item_id: &str,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let mut conn = self.conn()?;
        let count = conn.execute(
            &format!(
                "DELETE FROM {}.plan_items WHERE plan_id = $1 AND id = $2",
                self.schema
            ),
            &[&plan_id, &item_id],
        )?;
        Ok(count > 0)
    }

    pub fn delete_plan(&self, plan_id: &str) -> Result<bool, Box<dyn std::error::Error>> {
        let mut conn = self.conn()?;
        let count = conn.execute(
            &format!("DELETE FROM {}.plans WHERE id = $1", self.schema),
            &[&plan_id],
        )?;
        Ok(count > 0)
    }
}

/// Every accepted `item_type`. One list, so the check and the error message it
/// prints cannot drift apart.
pub const ITEM_TYPES: &[&str] = &[
    "journey",
    "transport",
    "event",
    "activity",
    "place",
    "stay",
    "image",
    "note",
    "option_set",
    "booking",
    "outcome",
];

/// The `item_type`s whose payload shape this capability is willing to promise,
/// and the fields a caller must send for each.
///
/// Deliberately short. `payload` is stored as JSON text so provider evidence can
/// be preserved without becoming part of the durable contract, and the freedom
/// that buys is real: `event` alone is written by three different producers with
/// three different shapes (a scouting opportunity, a whole `ScoredResult`, and a
/// calendar anchor). Declaring a shape for `event` would reject two of the three.
///
/// So a variant is declared only where there is exactly one shape to promise:
///
/// - `transport` has one producer and one shape, and is the item an agent most
///   needs to write, because "hold this connection in the plan" is the request.
/// - `option_set` is new here and has no existing producer, so its shape can be
///   fixed from the start. It records the fares that were offered and not taken,
///   which cannot be recovered later at yesterday's prices.
///
/// Every other type stays permissive, and that is a statement rather than an
/// omission: an unmodelled payload is accepted as-is.
/// - `booking` is what makes a stage's `booked` status mean something. Before it,
///   `booked` was a string the API accepted with nothing behind it: no order
///   reference, no fare name, no refundability, no cancellation deadline. It
///   deliberately records `traveler_name_present` as a boolean rather than the
///   name, because whose name is on a ticket is personal data this repo has no
///   reason to hold.
/// - `stay` is declared for its intended producer, accommodation search
///   results entered through the agent surface (in-repo, the demo seeder is
///   the one writer), so its shape is fixed from the start the way
///   `option_set`'s was: where the stay is and when. Coordinates are required
///   because the place matching downstream runs on them; provider fields such
///   as the booking URL, price and rating ride along unvalidated.
const DECLARED_PAYLOADS: &[(&str, &[&str])] = &[
    ("transport", &["mode", "journey"]),
    ("option_set", &["query", "options"]),
    ("booking", &["provider", "order_ref"]),
    ("stay", &["check_in", "check_out", "latitude", "longitude"]),
];

/// Rejects a declared variant that is missing a required field, naming the field.
///
/// A caller that guesses a payload shape used to get a 201 and a row nobody could
/// read back. Naming the field is the whole point: "invalid payload" sends the
/// caller back to the source, one field name sends it back to its own request.
fn validate_payload(
    item_type: &str,
    payload: &serde_json::Value,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some((_, required)) = DECLARED_PAYLOADS.iter().find(|(t, _)| *t == item_type) else {
        return Ok(());
    };
    let Some(object) = payload.as_object() else {
        return Err(format!("payload for item_type '{item_type}' must be an object").into());
    };
    for field in *required {
        if !object.contains_key(*field) {
            return Err(format!(
                "payload for item_type '{item_type}' requires the field '{field}' \
                 (required: {})",
                required.join(", ")
            )
            .into());
        }
    }
    Ok(())
}

/// Case- and whitespace-insensitive, and nothing more. Stripping punctuation or
/// folding umlauts would collapse places that really are different, and this
/// reports collisions for a human to judge rather than merging them itself.
fn normalize_place_name(name: &str) -> String {
    name.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn json_value(name: &str, ids: &[String]) -> Value {
    serde_json::json!({ "normalized_name": name, "ids": ids })
}

fn propagate_default_transport_modes(
    stages: Vec<TripStage>,
    previous: &[TransportMode],
    updated: &[TransportMode],
) -> Vec<TripStage> {
    stages
        .into_iter()
        .map(|mut stage| {
            // A stage equal to the former plan default has not been customized.
            // Let it follow a global edit while preserving stage-specific choices.
            if stage.transport_modes == previous {
                stage.transport_modes = updated.to_vec();
            }
            stage
        })
        .collect()
}

fn row_to_plan(row: &Row) -> Result<TripPlan, Box<dyn std::error::Error>> {
    let origin: String = row.try_get(2)?;
    let destinations: String = row.try_get(3)?;
    let travelers: String = row.try_get(8)?;
    let transport_modes: String = row.try_get(9)?;
    let stages: String = row.try_get(10)?;
    let source_kind: Option<String> = row.try_get(12)?;
    let source_ref: Option<String> = row.try_get(13)?;
    Ok(TripPlan {
        id: row.try_get(0)?,
        title: row.try_get(1)?,
        origin: serde_json::from_str(&origin)?,
        destinations: serde_json::from_str(&destinations)?,
        date_start: row.try_get(4)?,
        date_end: row.try_get(5)?,
        interests: row.try_get(6)?,
        status: row.try_get(7)?,
        travelers: serde_json::from_str(&travelers)?,
        transport_modes: serde_json::from_str(&transport_modes)?,
        stages: serde_json::from_str(&stages)?,
        cover_image_url: row.try_get(11)?,
        source: source_kind
            .zip(source_ref)
            .map(|(kind, reference)| PlanSource { kind, reference }),
        created_at: row.try_get(14)?,
        updated_at: row.try_get(15)?,
        budget_cents: row.try_get(16)?,
        currency: row.try_get(17)?,
    })
}

fn row_to_item(row: &Row) -> Result<PlanItem, Box<dyn std::error::Error>> {
    let payload: String = row.try_get(6)?;
    Ok(PlanItem {
        id: row.try_get(0)?,
        plan_id: row.try_get(1)?,
        item_type: row.try_get(2)?,
        day: row.try_get(3)?,
        external_id: row.try_get(4)?,
        title: row.try_get(5)?,
        payload: serde_json::from_str(&payload)?,
        created_at: row.try_get(7)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn generated_ids_unique() {
        let first = generated_id("trip:plan");
        let second = generated_id("trip:plan");
        assert!(first.starts_with("trip:plan:"));
        assert_ne!(first, second);
    }

    /// The lost-update guard. A browser holds a read for milliseconds; an agent
    /// holds it across turns while it calls transit and reasons, and `stages` is
    /// accepted wholesale, so a stale write silently reverts everything another
    /// writer changed in between.
    #[test]
    fn a_conditional_write_refuses_a_revision_it_did_not_expect() {
        // Omitted keeps last-write-wins, so nothing that already works breaks.
        assert!(check_expected_revision(None, "1786470000").is_ok());
        assert!(check_expected_revision(Some("1786470000"), "1786470000").is_ok());

        let stale = check_expected_revision(Some("1786470000"), "1786479999")
            .expect_err("a changed revision must be refused");
        let message = stale.to_string();
        // The prefix is what the server turns into 409 + code:stale_plan, so a
        // caller can branch without parsing prose.
        assert!(message.starts_with("stale_plan:"), "got: {message}");
        // Both revisions are named, so "someone else wrote" is distinguishable
        // from "I sent the wrong id".
        assert!(message.contains("1786470000") && message.contains("1786479999"));
    }

    /// Two ids whose names normalise to one string are one place typed twice.
    /// The dashboard mints `place:<slug>` with a null coordinate for any typed
    /// text, so this is how a plan quietly stops matching candidates within
    /// 75 km of itself.
    #[test]
    fn place_names_normalise_for_collision_detection_only() {
        assert_eq!(normalize_place_name("Bonn  Hbf"), "bonn hbf");
        assert_eq!(normalize_place_name("BONN HBF"), "bonn hbf");
        assert_eq!(normalize_place_name(" München "), "münchen");
        // Umlauts and punctuation are deliberately NOT folded: collapsing
        // Munchen and München would merge two places a human should judge, and
        // this detector reports collisions rather than deciding them.
        assert_ne!(
            normalize_place_name("Munchen"),
            normalize_place_name("München")
        );
        assert_ne!(
            normalize_place_name("Frankfurt(Main)Hbf"),
            normalize_place_name("Frankfurt Main Hbf")
        );
    }

    #[test]
    fn schema_names_are_restricted() {
        assert!(validate_schema("trips_test").is_ok());
        assert!(validate_schema("trips; DROP SCHEMA public").is_err());
    }

    /// The write paths the dashboard already uses, replayed field for field from
    /// `dashboard/src/routes/travel/+page.svelte`. A declared variant that
    /// rejected one of these would turn a working button into a 400 on deploy,
    /// which is the failure mode this test exists to catch.
    #[test]
    fn every_payload_the_dashboard_already_writes_still_validates() {
        let existing = [
            // saveJourney -- the one transport producer, and the shape now declared.
            ("transport", json!({ "mode": "train", "journey": { "id": "j:1" } })),
            // addTravelCandidate: a scouting opportunity.
            (
                "event",
                json!({ "opportunity_id": "o:1", "source": "luma", "url": "https://example.org" }),
            ),
            // saveEvent: the whole search result, shape not ours to fix.
            ("event", json!({ "title": "t", "score": 0.5, "date": "2026-09-01" })),
            // saveCalendarEvent: a calendar anchor.
            ("event", json!({ "calendar_entry_id": "e:1", "commitment": "planned" })),
            // savePlace.
            ("activity", json!({ "url": "https://example.org", "latitude": 50.7 })),
        ];
        for (item_type, payload) in existing {
            assert!(
                validate_payload(item_type, &payload).is_ok(),
                "existing dashboard write for '{item_type}' must keep working: {payload}"
            );
        }
    }

    /// A caller that guesses a declared payload's shape gets told which field it
    /// missed, not a 201 and an unreadable row.
    #[test]
    fn a_declared_variant_names_the_field_it_is_missing() {
        let missing_journey = validate_payload("transport", &json!({ "mode": "train" }))
            .expect_err("transport without a journey must be rejected");
        assert!(
            missing_journey.to_string().contains("journey"),
            "the error must name the missing field, got: {missing_journey}"
        );

        let missing_options = validate_payload(
            "option_set",
            &json!({ "query": { "from": "8000044", "to": "8000105" } }),
        )
        .expect_err("option_set without options must be rejected");
        assert!(missing_options.to_string().contains("options"));

        // A stay without coordinates is exactly the row A2's coordinate matcher
        // could never use, so the write is refused and names what it lacks.
        let missing_latitude = validate_payload(
            "stay",
            &json!({ "check_in": "2026-09-14", "check_out": "2026-09-16", "longitude": 11.57 }),
        )
        .expect_err("stay without latitude must be rejected");
        assert!(missing_latitude.to_string().contains("latitude"));

        // A payload that is not an object at all is a different mistake, and says so.
        let not_an_object = validate_payload("transport", &json!("a string"))
            .expect_err("a non-object payload must be rejected");
        assert!(not_an_object.to_string().contains("must be an object"));

        // Complete payloads pass.
        assert!(validate_payload(
            "option_set",
            &json!({
                "query": { "from": "8000044", "to": "8000105", "time": "2026-09-01T08:00:00" },
                "options": [{ "id": "j:1", "total_price": 36.47, "chosen": true }],
                "observed_at": "2026-08-11T12:00:00Z"
            })
        )
        .is_ok());

        // A stay as the agent surface writes it from an accommodation search
        // result: the declared fields plus provider evidence riding along.
        assert!(validate_payload(
            "stay",
            &json!({
                "check_in": "2026-09-14",
                "check_out": "2026-09-16",
                "latitude": 48.1371,
                "longitude": 11.5754,
                "provider": "booking.com",
                "url": "https://www.booking.com/hotel/de/example.html",
                "amount_cents": 12200,
                "currency": "EUR"
            })
        )
        .is_ok());
    }

    /// An unmodelled type accepts anything, deliberately: the alternative is
    /// inventing a shape for `note` that no producer agreed to.
    #[test]
    fn undeclared_types_stay_permissive() {
        for item_type in ITEM_TYPES {
            if DECLARED_PAYLOADS.iter().any(|(t, _)| t == item_type) {
                continue;
            }
            assert!(
                validate_payload(item_type, &json!({ "anything": [1, 2, 3] })).is_ok(),
                "'{item_type}' is not declared and must accept any object"
            );
        }
        // And the declared list is a subset of the accepted types, so a variant
        // can never be declared for a type the CHECK constraint would reject.
        for (declared, _) in DECLARED_PAYLOADS {
            assert!(
                ITEM_TYPES.contains(declared),
                "'{declared}' is declared but not an accepted item_type"
            );
        }
    }

    #[test]
    fn global_mode_edit_preserves_custom_stage_modes() {
        let place = |id: &str| PlaceRef {
            id: id.into(),
            name: id.into(),
            kind: PlaceKind::City,
            address: None,
            latitude: None,
            longitude: None,
        };
        let stage = |id: &str, modes: Vec<TransportMode>| TripStage {
            id: id.into(),
            sequence: 1,
            origin: place("origin"),
            destination: place("destination"),
            date: None,
            transport_modes: modes,
            travelers: Vec::new(),
            status: StageStatus::Planning,
            selected_option_id: None,
        };
        let stages = propagate_default_transport_modes(
            vec![
                stage("default", vec![TransportMode::Train]),
                stage("custom", vec![TransportMode::Flight]),
            ],
            &[TransportMode::Train],
            &[TransportMode::Train, TransportMode::Car],
        );
        assert_eq!(
            stages[0].transport_modes,
            vec![TransportMode::Train, TransportMode::Car]
        );
        assert_eq!(stages[1].transport_modes, vec![TransportMode::Flight]);
    }
}
