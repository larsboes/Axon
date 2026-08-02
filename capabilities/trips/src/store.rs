use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use postgres::{Client, NoTls, Row};
use serde::{Deserialize, Serialize};
use serde_json::Value;

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PlaceKind {
    Address,
    Airport,
    City,
    #[default]
    Station,
    Venue,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum StageStatus {
    Booked,
    Completed,
    OptionSelected,
    #[default]
    Planning,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlanSource {
    pub kind: String,
    pub reference: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
    conn: Mutex<Client>,
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
        let store = Self {
            conn: Mutex::new(Client::connect(database_url, NoTls)?),
            schema: schema.to_string(),
        };
        store.init_schema()?;
        Ok(store)
    }

    fn init_schema(&self) -> Result<(), Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
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
                ADD COLUMN IF NOT EXISTS source_ref TEXT;
            CREATE UNIQUE INDEX IF NOT EXISTS idx_trip_plan_source
                ON {schema}.plans(source_kind, source_ref)
                WHERE source_kind IS NOT NULL AND source_ref IS NOT NULL;
            ALTER TABLE {schema}.plan_items
                DROP CONSTRAINT IF EXISTS plan_items_item_type_check;
            ALTER TABLE {schema}.plan_items
                ADD CONSTRAINT plan_items_item_type_check
                CHECK (item_type IN ('journey','transport','event','activity','place','stay','image','note'));
            ",
            schema = self.schema
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
        let mut conn = self.conn.lock().unwrap();
        let row = conn.query_one(
            &format!(
                "INSERT INTO {schema}.plans
                    (id,title,origin,destinations,date_start,date_end,interests,status,travelers,
                     transport_modes,stages,cover_image_url,source_kind,source_ref,created_at,updated_at)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,'draft',$8,$9,$10,$11,$12,$13,$14,$14)
                 RETURNING id,title,origin,destinations,date_start,date_end,interests,status,
                    travelers,transport_modes,stages,cover_image_url,source_kind,source_ref,
                    created_at,updated_at",
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
        let mut conn = self.conn.lock().unwrap();
        let rows = conn.query(
            &format!(
                "SELECT id,title,origin,destinations,date_start,date_end,interests,status,
                        travelers,transport_modes,stages,cover_image_url,source_kind,source_ref,
                        created_at,updated_at
                 FROM {}.plans WHERE status != 'archived'
                 ORDER BY updated_at DESC",
                self.schema
            ),
            &[],
        )?;
        rows.iter().map(row_to_plan).collect()
    }

    pub fn get_plan(&self, id: &str) -> Result<Option<PlanDetails>, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let Some(row) = conn.query_opt(
            &format!(
                "SELECT id,title,origin,destinations,date_start,date_end,interests,status,
                        travelers,transport_modes,stages,cover_image_url,source_kind,source_ref,
                        created_at,updated_at
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
        let mut conn = self.conn.lock().unwrap();
        let row = conn.query_one(
            &format!(
                "UPDATE {schema}.plans SET
                    title=$1,origin=$2,destinations=$3,date_start=$4,date_end=$5,interests=$6,
                    status=$7,travelers=$8,transport_modes=$9,stages=$10,cover_image_url=$11,
                    updated_at=$12
                 WHERE id=$13
                 RETURNING id,title,origin,destinations,date_start,date_end,interests,status,
                    travelers,transport_modes,stages,cover_image_url,source_kind,source_ref,
                    created_at,updated_at",
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
            ],
        )?;
        Ok(Some(row_to_plan(&row)?))
    }

    pub fn find_plan_by_source(
        &self,
        kind: &str,
        reference: &str,
    ) -> Result<Option<TripPlan>, Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let row = conn.query_opt(
            &format!(
                "SELECT id,title,origin,destinations,date_start,date_end,interests,status,
                        travelers,transport_modes,stages,cover_image_url,source_kind,source_ref,
                        created_at,updated_at
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
        if ![
            "journey",
            "transport",
            "event",
            "activity",
            "place",
            "stay",
            "image",
            "note",
        ]
        .contains(&input.item_type.as_str())
        {
            return Err(
                "item_type must be journey, transport, event, activity, place, stay, image, or note"
                    .into(),
            );
        }
        if input.external_id.trim().is_empty() || input.title.trim().is_empty() {
            return Err("external_id and title are required".into());
        }

        let id = generated_id("trip:item");
        let now = now_text();
        let payload = serde_json::to_string(&input.payload)?;
        let mut conn = self.conn.lock().unwrap();
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
        let mut conn = self.conn.lock().unwrap();
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
        let mut conn = self.conn.lock().unwrap();
        let count = conn.execute(
            &format!("DELETE FROM {}.plans WHERE id = $1", self.schema),
            &[&plan_id],
        )?;
        Ok(count > 0)
    }
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

    #[test]
    fn generated_ids_unique() {
        let first = generated_id("trip:plan");
        let second = generated_id("trip:plan");
        assert!(first.starts_with("trip:plan:"));
        assert_ne!(first, second);
    }

    #[test]
    fn schema_names_are_restricted() {
        assert!(validate_schema("trips_test").is_ok());
        assert!(validate_schema("trips; DROP SCHEMA public").is_err());
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
