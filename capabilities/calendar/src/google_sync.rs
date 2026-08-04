//! Phase E's impure half: the credential, the Google Calendar API calls, and
//! the two runs that tie them to the store.
//!
//! Every *decision* lives in `google.rs` and is unit-tested against recorded
//! payloads with no token and no socket. What is left here is transport and
//! sequencing, deliberately thin.
//!
//! **Credentials.** Read from a plain `KEY=value` file in the private overlay,
//! the same shape and the same three keys `capabilities/comms` uses:
//! `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET`, `GOOGLE_REFRESH_TOKEN`. No
//! value is ever read from this repo, and none is ever logged — not in an
//! error, not in a report, not in a failed-refresh body (Google puts token
//! material in some of those).
//!
//! **When the credential is absent, this fails loudly.** Not a no-op, not an
//! empty result, not a placeholder event: a named error saying which key is
//! missing, which file it belongs in, and which setup step produces it. A
//! silent import that quietly does nothing is the failure mode that makes an
//! operator trust an empty calendar.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::{Config, GoogleConfig};
use crate::date;
use crate::google::{self, Action, GoogleEvent};
use crate::model::{Entry, ExportOptIn, NewEntry};
use crate::store::CalendarStore;
use crate::zone::HomeTimezone;

const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const CALENDAR_API: &str = "https://www.googleapis.com/calendar/v3/calendars";

/// The OAuth scopes this capability needs. Import alone is satisfied by the
/// readonly scope; `calendar.events` covers reading *and* writing events and
/// is therefore the one grant that serves both runs.
///
/// `capabilities/comms/auth/get-refresh-token.ts` already requests
/// `calendar.events`, so an operator who has run that bootstrap can point
/// `google.env_path` at `comms.env` instead of minting a second grant.
pub const SCOPE_READONLY: &str = "https://www.googleapis.com/auth/calendar.readonly";
pub const SCOPE_EVENTS: &str = "https://www.googleapis.com/auth/calendar.events";

const REQUIRED_KEYS: [&str; 3] = [
    "GOOGLE_CLIENT_ID",
    "GOOGLE_CLIENT_SECRET",
    "GOOGLE_REFRESH_TOKEN",
];

type SyncResult<T> = Result<T, String>;

/// A deliberately bounded review window.  Google can hold a lifetime of
/// history and recurring instances; a review surface that quietly turns into
/// a bulk import is neither useful nor safe.
pub const MAX_REVIEW_DAYS: i64 = 90;

// ---- resolved, checked settings -------------------------------------------

/// The two values Phase E refuses to guess, resolved once so both runs fail
/// the same way and fail before any network call.
#[derive(Debug, Clone)]
pub struct Settings {
    pub tz: HomeTimezone,
    pub calendar_id: String,
    pub google: GoogleConfig,
}

impl Settings {
    pub fn resolve(config: &Config) -> SyncResult<Self> {
        let path = crate::config::config_path();
        let name = config.home_timezone.as_deref().ok_or_else(|| {
            format!(
                "home_timezone is not configured. Google events carry a real UTC offset and this \
                 capability stores naive local wall time, so a sync cannot convert between them \
                 without knowing the operator's zone. Declare AXON_HOME_TIMEZONE in the overlay's \
                 config/deployment.env — scouting resolves the same value — or set \
                 \"home_timezone\" in {path:?} for this capability alone (README § Time model)."
            )
        })?;
        let tz = HomeTimezone::parse(name)?;
        let calendar_id = config
            .google
            .calendar_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| {
                format!(
                    "google.calendar_id is not configured. Set \"google\": {{\"calendar_id\": \
                     \"primary\"}} in {path:?}, or the \
                     …@group.calendar.google.com id of a secondary calendar. There is no default: \
                     guessing would sync the wrong calendar."
                )
            })?
            .to_string();
        Ok(Self {
            tz,
            calendar_id,
            google: config.google.clone(),
        })
    }
}

// ---- credentials ----------------------------------------------------------

struct CachedToken {
    /// The credential file this token came from. Keyed rather than global: a
    /// cache shared across two credential files would hand one calendar's
    /// token to the other's account, and the failure would look like a
    /// permissions problem rather than a caching one.
    source: std::path::PathBuf,
    token: String,
    expires_at: u64,
}

/// In-process only, never persisted. Refreshed within 60s of expiry.
static TOKEN_CACHE: OnceLock<Mutex<Option<CachedToken>>> = OnceLock::new();

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Reads one `KEY=value` from a plain env file. The value is returned but
/// never logged, and the error deliberately names only the key and the path.
fn read_env_key(env_path: &Path, key: &str) -> SyncResult<String> {
    let body = std::fs::read_to_string(env_path).map_err(|error| {
        format!(
            "cannot read the Google credential file {env_path:?}: {error}. Phase E expects \
             {} in that file. It lives in the private overlay and is never part of this repo; \
             see README § Phases > E for the steps that produce it.",
            REQUIRED_KEYS.join(", ")
        )
    })?;
    body.lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix(&format!("{key}="))
                .map(|value| value.trim().to_string())
        })
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            format!(
                "{key} is missing or empty in {env_path:?}. Phase E needs all of {}.",
                REQUIRED_KEYS.join(", ")
            )
        })
}

fn http_client() -> SyncResult<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .user_agent(concat!("AxonCalendar/", env!("CARGO_PKG_VERSION")))
        .gzip(true)
        .build()
        .map_err(|error| format!("could not build an HTTP client: {error}"))
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default = "default_expires_in")]
    expires_in: u64,
}

fn default_expires_in() -> u64 {
    3600
}

/// A valid access token, refreshed through the OAuth refresh grant when the
/// cache is cold or near expiry.
///
/// The failed-refresh path prints the status and nothing else: Google's error
/// bodies can echo token material.
pub fn access_token(env_path: &Path) -> SyncResult<String> {
    let cache = TOKEN_CACHE.get_or_init(|| Mutex::new(None));
    if let Some(cached) = cache.lock().unwrap().as_ref() {
        if cached.source == env_path && cached.expires_at > now_secs() + 60 {
            return Ok(cached.token.clone());
        }
    }

    let client_id = read_env_key(env_path, "GOOGLE_CLIENT_ID")?;
    let client_secret = read_env_key(env_path, "GOOGLE_CLIENT_SECRET")?;
    let refresh_token = read_env_key(env_path, "GOOGLE_REFRESH_TOKEN")?;

    let response = http_client()?
        .post(TOKEN_URL)
        .form(&[
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
            ("refresh_token", refresh_token.as_str()),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .map_err(|error| format!("token refresh could not reach Google: {error}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "token refresh failed with HTTP {} — the refresh token in {env_path:?} is probably \
             revoked or was issued for different scopes (import needs {SCOPE_READONLY}, export \
             needs {SCOPE_EVENTS}). Re-run the consent step in README § Phases > E.",
            response.status()
        ));
    }

    let token: TokenResponse = response
        .json()
        .map_err(|error| format!("token refresh returned an unreadable body: {error}"))?;
    *cache.lock().unwrap() = Some(CachedToken {
        source: env_path.to_path_buf(),
        token: token.access_token.clone(),
        expires_at: now_secs() + token.expires_in,
    });
    Ok(token.access_token)
}

// ---- the API surface, behind a trait so a fixture can stand in -------------

/// The three Google calls Phase E makes. A trait so the import and export runs
/// can be exercised against recorded payloads: everything below this line is
/// transport, and transport is the one thing a test cannot have.
pub trait CalendarApi {
    fn list_events(&self, calendar_id: &str, from: &str, to: &str, max: usize)
        -> SyncResult<Vec<GoogleEvent>>;
    /// Returns the created event's id.
    fn insert_event(&self, calendar_id: &str, body: &Value) -> SyncResult<String>;
    fn patch_event(&self, calendar_id: &str, event_id: &str, body: &Value) -> SyncResult<String>;
}

/// The real one. Reads the token from the configured credential file on each
/// call (cached in process), and never logs it.
pub struct HttpCalendarApi<'a> {
    env_path: &'a Path,
}

impl<'a> HttpCalendarApi<'a> {
    pub fn new(env_path: &'a Path) -> Self {
        Self { env_path }
    }

    fn token(&self) -> SyncResult<String> {
        access_token(self.env_path)
    }
}

/// Google's own encoding for a path segment that may contain `@` and `.`
/// (a secondary calendar id is an address-shaped string).
fn encode_segment(segment: &str) -> String {
    segment
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

fn fail_for(status: reqwest::StatusCode, what: &str) -> String {
    let hint = match status.as_u16() {
        401 => " — the access token was rejected; the refresh token may be revoked",
        403 => " — the grant is missing the scope this call needs, or the calendar is not shared with this account",
        404 => " — no such calendar or event for this account; check google.calendar_id",
        429 => " — Google rate-limited this run; retry later",
        _ => "",
    };
    format!("{what} failed with HTTP {status}{hint}")
}

impl CalendarApi for HttpCalendarApi<'_> {
    fn list_events(
        &self,
        calendar_id: &str,
        from: &str,
        to: &str,
        max: usize,
    ) -> SyncResult<Vec<GoogleEvent>> {
        let token = self.token()?;
        let client = http_client()?;
        let url = format!("{CALENDAR_API}/{}/events", encode_segment(calendar_id));
        let mut events: Vec<GoogleEvent> = Vec::new();
        let mut page_token: Option<String> = None;

        while events.len() < max {
            let page_size = (max - events.len()).min(250).to_string();
            let mut query: Vec<(&str, String)> = vec![
                ("timeMin", from.to_string()),
                ("timeMax", to.to_string()),
                // Expands a recurring series into instances, each with its own
                // stable id — which is what dedupe needs (google::map_event).
                ("singleEvents", "true".into()),
                ("orderBy", "startTime".into()),
                ("maxResults", page_size),
            ];
            if let Some(cursor) = &page_token {
                query.push(("pageToken", cursor.clone()));
            }

            let response = client
                .get(&url)
                .bearer_auth(&token)
                .query(&query)
                .send()
                .map_err(|error| format!("events.list could not reach Google: {error}"))?;
            if !response.status().is_success() {
                return Err(fail_for(response.status(), "events.list"));
            }
            let page: google::EventsPage = response
                .json()
                .map_err(|error| format!("events.list returned an unreadable body: {error}"))?;

            let exhausted = page.next_page_token.is_none();
            events.extend(page.items);
            page_token = page.next_page_token;
            if exhausted {
                break;
            }
        }
        events.truncate(max);
        Ok(events)
    }

    fn insert_event(&self, calendar_id: &str, body: &Value) -> SyncResult<String> {
        let url = format!("{CALENDAR_API}/{}/events", encode_segment(calendar_id));
        let response = http_client()?
            .post(&url)
            .bearer_auth(self.token()?)
            .json(body)
            .send()
            .map_err(|error| format!("events.insert could not reach Google: {error}"))?;
        if !response.status().is_success() {
            return Err(fail_for(response.status(), "events.insert"));
        }
        created_id(response)
    }

    fn patch_event(&self, calendar_id: &str, event_id: &str, body: &Value) -> SyncResult<String> {
        let url = format!(
            "{CALENDAR_API}/{}/events/{}",
            encode_segment(calendar_id),
            encode_segment(event_id)
        );
        let response = http_client()?
            .patch(&url)
            .bearer_auth(self.token()?)
            .json(body)
            .send()
            .map_err(|error| format!("events.patch could not reach Google: {error}"))?;
        if !response.status().is_success() {
            return Err(fail_for(response.status(), "events.patch"));
        }
        created_id(response)
    }
}

fn created_id(response: reqwest::blocking::Response) -> SyncResult<String> {
    let event: GoogleEvent = response
        .json()
        .map_err(|error| format!("Google returned an unreadable event: {error}"))?;
    if event.id.trim().is_empty() {
        return Err("Google returned an event without an id".into());
    }
    Ok(event.id)
}

// ---- the store surface, likewise behind traits ----------------------------

/// The store operations an import needs.
///
/// A trait for one reason: "running an import twice produces one entry per
/// event" is the claim this phase most has to *show*, and showing it needs a
/// second run over the same fixtures — which a test can have, and a Postgres
/// integration test in a sandbox cannot reliably.
pub trait ImportStore {
    fn existing(&self, external_id: &str) -> SyncResult<Option<Entry>>;
    fn upsert(&self, entry: &NewEntry) -> SyncResult<Entry>;
    fn delete(&self, entry_id: &str) -> SyncResult<bool>;
}

/// The store operations an export needs.
pub trait ExportStore {
    fn queue(&self) -> SyncResult<Vec<(ExportOptIn, Entry)>>;
    fn record_push(&self, entry_id: &str, google_event_id: &str) -> SyncResult<()>;
}

impl ImportStore for CalendarStore {
    fn existing(&self, external_id: &str) -> SyncResult<Option<Entry>> {
        self.get_entry_by_external(google::SOURCE, external_id)
            .map_err(|error| format!("reading the existing entry failed: {error}"))
    }

    fn upsert(&self, entry: &NewEntry) -> SyncResult<Entry> {
        self.upsert_external_entry(entry)
            .map_err(|error| error.to_string())
    }

    fn delete(&self, entry_id: &str) -> SyncResult<bool> {
        self.delete_entry(entry_id)
            .map_err(|error| format!("deleting {entry_id}: {error}"))
    }
}

impl ExportStore for CalendarStore {
    fn queue(&self) -> SyncResult<Vec<(ExportOptIn, Entry)>> {
        self.export_queue()
            .map_err(|error| format!("reading the export queue failed: {error}"))
    }

    fn record_push(&self, entry_id: &str, google_event_id: &str) -> SyncResult<()> {
        self.record_export_push(entry_id, google_event_id)
            .map(|_| ())
            .map_err(|error| format!("recording the push for {entry_id}: {error}"))
    }
}

// ---- import ---------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct ImportOutcome {
    pub google_event_id: String,
    pub title: String,
    pub action: Action,
    /// The Axon entry, once there is one.
    pub entry_id: Option<String>,
    /// Set when Axon's version won: what Google now says, so the divergence is
    /// visible instead of silently dropped.
    pub google_says: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkippedEvent {
    pub google_event_id: String,
    pub reason: String,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct ImportReport {
    pub calendar_id: String,
    pub home_timezone: String,
    pub from: String,
    pub to: String,
    pub fetched: usize,
    pub created: usize,
    pub refreshed: usize,
    pub unchanged: usize,
    pub kept_axon_version: usize,
    pub dropped_drafts: usize,
    pub outcomes: Vec<ImportOutcome>,
    pub skipped: Vec<SkippedEvent>,
    pub dry_run: bool,
}

/// The status of one Google event in the review-before-import surface.  This
/// is intentionally more cautious than the unattended import: a person must
/// see why an event is not selectable before Axon writes even a draft.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReviewStatus {
    Importable,
    LikelyDuplicate,
    AlreadyInAxon,
    Cancelled,
    Invalid,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportCandidate {
    pub google_event_id: String,
    /// Google's revision marker. The selected-import endpoint requires this
    /// exact value again, so an event changed after review cannot slip in.
    pub google_updated: Option<String>,
    pub title: String,
    pub starts_at: Option<String>,
    pub ends_at: Option<String>,
    pub all_day: Option<bool>,
    pub location: Option<String>,
    pub html_link: Option<String>,
    pub recurring_event_id: Option<String>,
    pub status: ReviewStatus,
    pub reason: Option<String>,
    /// An opaque label joining candidates which have identical normalized
    /// title and interval. It is a review hint, never an automatic deletion.
    pub duplicate_group: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportPreview {
    pub calendar_id: String,
    pub home_timezone: String,
    pub from: String,
    pub to: String,
    pub fetched: usize,
    /// The configured cap was reached, so this is not a complete review.
    pub at_event_limit: bool,
    pub candidates: Vec<ImportCandidate>,
}

/// One explicit choice from an `ImportPreview`.
#[derive(Debug, Clone, Deserialize)]
pub struct SelectedGoogleEvent {
    pub google_event_id: String,
    pub google_updated: Option<String>,
}

fn review_window(from: &str, to: &str) -> SyncResult<(String, String)> {
    let from_days = date::parse_date(from)
        .ok_or_else(|| "from must be a valid YYYY-MM-DD date".to_string())?;
    let to_days = date::parse_date(to)
        .ok_or_else(|| "to must be a valid YYYY-MM-DD date".to_string())?;
    let days = to_days - from_days;
    if days <= 0 {
        return Err("to must be after from".into());
    }
    if days > MAX_REVIEW_DAYS {
        return Err(format!(
            "Google import review is limited to {MAX_REVIEW_DAYS} days; narrow the date range before reviewing"
        ));
    }
    Ok((format!("{from}T00:00:00Z"), format!("{to}T00:00:00Z")))
}

fn normalized_title(title: &str) -> String {
    let mut normalized = String::with_capacity(title.len());
    for character in title.chars() {
        if character.is_alphanumeric() {
            normalized.extend(character.to_lowercase());
        } else {
            normalized.push(' ');
        }
    }
    normalized.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn duplicate_key(candidate: &ImportCandidate) -> Option<String> {
    if candidate.status != ReviewStatus::Importable {
        return None;
    }
    Some(format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{}",
        normalized_title(&candidate.title),
        candidate.starts_at.as_deref().unwrap_or_default(),
        candidate.ends_at.as_deref().unwrap_or_default(),
        candidate.all_day.unwrap_or(false),
    ))
}

fn candidate_for(event: &GoogleEvent, existing: Option<&Entry>, tz: &HomeTimezone) -> ImportCandidate {
    let mut candidate = ImportCandidate {
        google_event_id: event.id.clone(),
        google_updated: event.updated.clone(),
        title: event
            .summary
            .as_deref()
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .unwrap_or(google::UNTITLED)
            .to_string(),
        starts_at: None,
        ends_at: None,
        all_day: None,
        location: event.location.clone(),
        html_link: event.html_link.clone(),
        recurring_event_id: event.recurring_event_id.clone(),
        status: ReviewStatus::Invalid,
        reason: None,
        duplicate_group: None,
    };

    match google::decide(event, existing) {
        Action::Skip | Action::DropDraft | Action::KeepCancelledAxonVersion => {
            candidate.status = ReviewStatus::Cancelled;
            candidate.reason = Some("Cancelled in Google".into());
        }
        Action::RefreshDraft | Action::KeepAxonVersion => {
            candidate.status = ReviewStatus::AlreadyInAxon;
            candidate.reason = Some(match google::decide(event, existing) {
                Action::RefreshDraft => "Already imported as an Axon draft",
                Action::KeepAxonVersion => "Already adopted in Axon; Axon keeps its version",
                _ => unreachable!("handled above"),
            }
            .into());
        }
        Action::Create => match google::map_event(event, tz) {
            Ok(mapped) => {
                candidate.title = mapped.title;
                candidate.starts_at = Some(mapped.starts_at);
                candidate.ends_at = Some(mapped.ends_at);
                candidate.all_day = Some(mapped.all_day);
                candidate.location = mapped.location;
                candidate.status = ReviewStatus::Importable;
            }
            Err(reason) => candidate.reason = Some(reason),
        },
    }
    candidate
}

fn review_events(
    store: &dyn ImportStore,
    events: &[GoogleEvent],
    tz: &HomeTimezone,
) -> SyncResult<Vec<ImportCandidate>> {
    let mut candidates = Vec::with_capacity(events.len());
    for event in events {
        candidates.push(candidate_for(event, store.existing(event.id.trim())?.as_ref(), tz));
    }

    let mut grouped: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, candidate) in candidates.iter().enumerate() {
        if let Some(key) = duplicate_key(candidate) {
            grouped.entry(key).or_default().push(index);
        }
    }
    let mut group_number = 0usize;
    for indexes in grouped.values().filter(|indexes| indexes.len() > 1) {
        group_number += 1;
        let group = format!("duplicate-{group_number}");
        for index in indexes {
            candidates[*index].status = ReviewStatus::LikelyDuplicate;
            candidates[*index].duplicate_group = Some(group.clone());
            candidates[*index].reason = Some(
                "Same normalized title and exact time range as another Google event".into(),
            );
        }
    }
    Ok(candidates)
}

/// Fetches a bounded Google slice and classifies it for an operator.  It is
/// read-only: no preview row or imported entry is stored locally.
pub fn preview(
    store: &dyn ImportStore,
    api: &dyn CalendarApi,
    settings: &Settings,
    from: &str,
    to: &str,
) -> SyncResult<ImportPreview> {
    let (time_min, time_max) = review_window(from, to)?;
    let events = api.list_events(
        &settings.calendar_id,
        &time_min,
        &time_max,
        settings.google.max_events,
    )?;
    let candidates = review_events(store, &events, &settings.tz)?;
    Ok(ImportPreview {
        calendar_id: settings.calendar_id.clone(),
        home_timezone: settings.tz.name().to_string(),
        from: time_min,
        to: time_max,
        fetched: events.len(),
        at_event_limit: events.len() == settings.google.max_events,
        candidates,
    })
}

/// Imports only the events explicitly selected from a preview.  The provider
/// is queried again and every `updated` marker must still match, preventing a
/// stale preview from becoming an unintended write.
pub fn import_selected(
    store: &dyn ImportStore,
    api: &dyn CalendarApi,
    settings: &Settings,
    from: &str,
    to: &str,
    selected: &[SelectedGoogleEvent],
) -> SyncResult<ImportReport> {
    if selected.is_empty() {
        return Err("select at least one Google event to import".into());
    }
    if selected.len() > 100 {
        return Err("select at most 100 events per reviewed import".into());
    }
    let (time_min, time_max) = review_window(from, to)?;
    let mut expected: HashMap<String, Option<String>> = HashMap::new();
    for selection in selected {
        let id = selection.google_event_id.trim();
        if id.is_empty() || expected.contains_key(id) {
            return Err("every selected Google event needs one unique id".into());
        }
        expected.insert(id.to_string(), selection.google_updated.clone());
    }

    let events = api.list_events(
        &settings.calendar_id,
        &time_min,
        &time_max,
        settings.google.max_events,
    )?;
    let by_id: HashMap<&str, &GoogleEvent> = events.iter().map(|event| (event.id.as_str(), event)).collect();
    for (id, updated) in &expected {
        let Some(event) = by_id.get(id.as_str()) else {
            return Err(format!("Google event {id} is no longer in this review window; review again"));
        };
        if event.updated != *updated {
            return Err(format!("Google event {id} changed since the preview; review it again"));
        }
    }

    let reviewed = review_events(store, &events, &settings.tz)?;
    let candidates: HashMap<&str, &ImportCandidate> = reviewed
        .iter()
        .map(|candidate| (candidate.google_event_id.as_str(), candidate))
        .collect();
    let mut duplicate_groups = HashSet::new();
    for id in expected.keys() {
        let candidate = candidates
            .get(id.as_str())
            .ok_or_else(|| format!("Google event {id} could not be reviewed"))?;
        if !matches!(candidate.status, ReviewStatus::Importable | ReviewStatus::LikelyDuplicate) {
            return Err(format!("Google event {id} is no longer importable; review again"));
        }
        if let Some(group) = &candidate.duplicate_group {
            if !duplicate_groups.insert(group) {
                return Err("choose at most one event from each likely-duplicate group".into());
            }
        }
    }

    let chosen: Vec<GoogleEvent> = selected
        .iter()
        .filter_map(|selection| by_id.get(selection.google_event_id.trim()).copied().cloned())
        .collect();
    import_events(store, &chosen, settings, &time_min, &time_max, false)
}

/// `timeMin`/`timeMax` for the import, as the UTC-marked RFC 3339 Google
/// wants. Day granularity: the window only bounds the fetch, and the entries
/// inside it are converted individually.
pub fn import_window(today: i64, days_back: i64, days_ahead: i64) -> (String, String) {
    (
        format!("{}T00:00:00Z", date::format_date(today - days_back.max(0))),
        format!("{}T00:00:00Z", date::format_date(today + days_ahead.max(1))),
    )
}

/// Pulls the configured Google calendar into drafts.
///
/// Idempotent by construction: the dedupe key is `(source = "google",
/// external_id = <Google event id>)`, which the store's partial unique index
/// enforces, so a second run over an unchanged calendar writes nothing at all.
/// `google::decide` is what keeps a confirmed entry safe from the second run.
pub fn import(
    store: &dyn ImportStore,
    api: &dyn CalendarApi,
    settings: &Settings,
    dry_run: bool,
) -> SyncResult<ImportReport> {
    let (from, to) = import_window(
        date::today_days(),
        settings.google.import_days_back,
        settings.google.import_days_ahead,
    );
    let events = api.list_events(
        &settings.calendar_id,
        &from,
        &to,
        settings.google.max_events,
    )?;
    import_events(store, &events, settings, &from, &to, dry_run)
}

/// Applies a known provider slice. Kept separate from `import` so a reviewed
/// selection goes through exactly the same conflict policy and upsert path.
fn import_events(
    store: &dyn ImportStore,
    events: &[GoogleEvent],
    settings: &Settings,
    from: &str,
    to: &str,
    dry_run: bool,
) -> SyncResult<ImportReport> {
    let mut report = ImportReport {
        calendar_id: settings.calendar_id.clone(),
        home_timezone: settings.tz.name().to_string(),
        from: from.to_string(),
        to: to.to_string(),
        fetched: events.len(),
        dry_run,
        ..Default::default()
    };

    for event in events {
        let existing = store.existing(event.id.trim())?;

        match google::decide(event, existing.as_ref()) {
            Action::Skip => {}
            Action::KeepCancelledAxonVersion => {
                report.kept_axon_version += 1;
                report.outcomes.push(ImportOutcome {
                    google_event_id: event.id.clone(),
                    title: existing.as_ref().map(|e| e.title.clone()).unwrap_or_default(),
                    action: Action::KeepCancelledAxonVersion,
                    entry_id: existing.as_ref().map(|e| e.id.clone()),
                    google_says: Some(serde_json::json!({ "status": "cancelled" })),
                });
            }
            Action::KeepAxonVersion => {
                let existing = existing.expect("KeepAxonVersion implies an entry");
                report.kept_axon_version += 1;
                report.outcomes.push(ImportOutcome {
                    google_event_id: event.id.clone(),
                    title: existing.title.clone(),
                    action: Action::KeepAxonVersion,
                    entry_id: Some(existing.id),
                    google_says: Some(serde_json::to_value(event).unwrap_or(Value::Null)),
                });
            }
            Action::DropDraft => {
                let draft = existing.expect("DropDraft implies an entry");
                if !dry_run {
                    store.delete(&draft.id)?;
                }
                report.dropped_drafts += 1;
                report.outcomes.push(ImportOutcome {
                    google_event_id: event.id.clone(),
                    title: draft.title.clone(),
                    action: Action::DropDraft,
                    entry_id: Some(draft.id),
                    google_says: Some(serde_json::json!({ "status": "cancelled" })),
                });
            }
            action @ (Action::Create | Action::RefreshDraft) => {
                let candidate = match google::map_event(event, &settings.tz) {
                    Ok(candidate) => candidate,
                    Err(reason) => {
                        report.skipped.push(SkippedEvent {
                            google_event_id: event.id.clone(),
                            reason,
                        });
                        continue;
                    }
                };
                if let Some(existing) = &existing {
                    if !google::differs(&candidate, existing) {
                        report.unchanged += 1;
                        continue;
                    }
                }
                let entry_id = if dry_run {
                    existing.as_ref().map(|e| e.id.clone())
                } else {
                    match store.upsert(&candidate) {
                        Ok(entry) => Some(entry.id),
                        Err(error) => {
                            report.skipped.push(SkippedEvent {
                                google_event_id: event.id.clone(),
                                reason: format!("calendar rejected the entry: {error}"),
                            });
                            continue;
                        }
                    }
                };
                match action {
                    Action::Create => report.created += 1,
                    _ => report.refreshed += 1,
                }
                report.outcomes.push(ImportOutcome {
                    google_event_id: event.id.clone(),
                    title: candidate.title,
                    action,
                    entry_id,
                    google_says: None,
                });
            }
        }
    }
    Ok(report)
}

// ---- export ---------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct ExportOutcome {
    pub entry_id: String,
    pub title: String,
    /// `inserted` on the first push, `patched` afterwards.
    pub operation: &'static str,
    pub google_event_id: Option<String>,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct ExportReport {
    pub calendar_id: String,
    pub home_timezone: String,
    pub opted_in: usize,
    pub inserted: usize,
    pub patched: usize,
    pub pushed: Vec<ExportOutcome>,
    pub skipped: Vec<SkippedEvent>,
    pub dry_run: bool,
}

/// Pushes the opted-in entries, and only those.
///
/// The queue is the `google_exports` table, which is empty until the operator
/// opts an entry in one at a time — there is no "export everything" path and
/// no default-on flag. An entry that already carries a Google event id is
/// patched rather than inserted, so a second run updates rather than
/// duplicating.
pub fn export(
    store: &dyn ExportStore,
    api: &dyn CalendarApi,
    settings: &Settings,
    dry_run: bool,
) -> SyncResult<ExportReport> {
    let queue = store.queue()?;

    let mut report = ExportReport {
        calendar_id: settings.calendar_id.clone(),
        home_timezone: settings.tz.name().to_string(),
        opted_in: queue.len(),
        dry_run,
        ..Default::default()
    };

    for (optin, entry) in queue {
        let body = match google::export_body(&entry, &settings.tz) {
            Ok(body) => body,
            Err(reason) => {
                report.skipped.push(SkippedEvent {
                    google_event_id: optin.google_event_id.clone().unwrap_or_default(),
                    reason: format!("{}: {reason}", entry.id),
                });
                continue;
            }
        };
        // The ledger's calendar id wins over the current config: an entry
        // opted in against one calendar must not silently move to another.
        let calendar_id = &optin.google_calendar_id;
        let existing_event = optin.google_event_id.as_deref().filter(|id| !id.is_empty());
        let operation = if existing_event.is_some() {
            "patched"
        } else {
            "inserted"
        };

        if dry_run {
            report.pushed.push(ExportOutcome {
                entry_id: entry.id,
                title: entry.title,
                operation,
                google_event_id: existing_event.map(str::to_string),
            });
            continue;
        }

        let pushed = match existing_event {
            Some(event_id) => api.patch_event(calendar_id, event_id, &body),
            None => api.insert_event(calendar_id, &body),
        };
        match pushed {
            Ok(google_event_id) => {
                store.record_push(&entry.id, &google_event_id)?;
                match operation {
                    "patched" => report.patched += 1,
                    _ => report.inserted += 1,
                }
                report.pushed.push(ExportOutcome {
                    entry_id: entry.id,
                    title: entry.title,
                    operation,
                    google_event_id: Some(google_event_id),
                });
            }
            Err(reason) => report.skipped.push(SkippedEvent {
                google_event_id: existing_event.unwrap_or_default().to_string(),
                reason: format!("{}: {reason}", entry.id),
            }),
        }
    }
    Ok(report)
}

/// Entries a UI would offer an export toggle for — everything `export_refusal`
/// does not veto. Exposed so the refusal rule is asked once, here and in the
/// store, rather than reimplemented in a dashboard.
pub fn exportable(entries: &[Entry]) -> Vec<&Entry> {
    entries
        .iter()
        .filter(|entry| google::export_refusal(entry).is_none())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::google::{EventTime, EventsPage};
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    /// A recorded `events.list` page — one timed event, one all-day event,
    /// one expanded recurring instance.
    const PAGE: &str = r#"{
      "items": [
        {
          "id": "3q7l9v1c8m2p4t6y0x5z",
          "status": "confirmed",
          "summary": "Team sync",
          "location": "Example City",
          "start": { "dateTime": "2026-08-14T10:00:00+02:00", "timeZone": "Europe/Berlin" },
          "end":   { "dateTime": "2026-08-14T11:00:00+02:00", "timeZone": "Europe/Berlin" }
        },
        {
          "id": "7h2k4n6q8s0u2w4y6a8c",
          "status": "confirmed",
          "summary": "Urlaub",
          "start": { "date": "2026-08-17" },
          "end":   { "date": "2026-08-22" }
        },
        {
          "id": "base9x7v5t3r1p_20260819T063000Z",
          "status": "confirmed",
          "summary": "Standup",
          "start": { "dateTime": "2026-08-19T08:30:00+02:00" },
          "end":   { "dateTime": "2026-08-19T08:45:00+02:00" }
        }
      ]
    }"#;

    struct FixtureApi {
        events: Vec<GoogleEvent>,
        pushes: RefCell<Vec<(String, Option<String>, Value)>>,
    }

    impl FixtureApi {
        fn new(events: Vec<GoogleEvent>) -> Self {
            Self {
                events,
                pushes: RefCell::new(Vec::new()),
            }
        }
    }

    impl CalendarApi for FixtureApi {
        fn list_events(&self, _: &str, _: &str, _: &str, max: usize) -> SyncResult<Vec<GoogleEvent>> {
            Ok(self.events.iter().take(max).cloned().collect())
        }
        fn insert_event(&self, calendar_id: &str, body: &Value) -> SyncResult<String> {
            self.pushes
                .borrow_mut()
                .push((calendar_id.into(), None, body.clone()));
            Ok(format!("g-inserted-{}", self.pushes.borrow().len()))
        }
        fn patch_event(&self, calendar_id: &str, event_id: &str, body: &Value) -> SyncResult<String> {
            self.pushes
                .borrow_mut()
                .push((calendar_id.into(), Some(event_id.into()), body.clone()));
            Ok(event_id.to_string())
        }
    }

    /// In-memory stand-in with the one invariant the real table enforces:
    /// `(source, external_id)` is unique, so an upsert replaces rather than
    /// appends. Without that the idempotency test would prove nothing.
    #[derive(Default)]
    struct FakeStore {
        by_external: RefCell<BTreeMap<String, Entry>>,
        next_id: RefCell<usize>,
        exports: RefCell<Vec<(ExportOptIn, Entry)>>,
        pushes_recorded: RefCell<Vec<(String, String)>>,
    }

    impl FakeStore {
        fn seed(&self, entry: Entry) {
            self.by_external
                .borrow_mut()
                .insert(entry.external_id.clone().unwrap(), entry);
        }
        fn count(&self) -> usize {
            self.by_external.borrow().len()
        }
        fn get(&self, external_id: &str) -> Entry {
            self.by_external.borrow().get(external_id).cloned().unwrap()
        }
    }

    impl ImportStore for FakeStore {
        fn existing(&self, external_id: &str) -> SyncResult<Option<Entry>> {
            Ok(self.by_external.borrow().get(external_id).cloned())
        }
        fn upsert(&self, input: &NewEntry) -> SyncResult<Entry> {
            let external_id = input.external_id.clone().unwrap();
            let mut map = self.by_external.borrow_mut();
            let id = match map.get(&external_id) {
                Some(existing) => existing.id.clone(),
                None => {
                    *self.next_id.borrow_mut() += 1;
                    format!("cal:entry:{}", self.next_id.borrow())
                }
            };
            let entry = Entry {
                id,
                kind: input.kind.clone(),
                commitment: input.commitment,
                title: input.title.clone(),
                starts_at: input.starts_at.clone(),
                ends_at: input.ends_at.clone(),
                all_day: input.all_day,
                location: input.location.clone(),
                notes: input.notes.clone(),
                source: input.source.clone(),
                external_id: Some(external_id.clone()),
                rhythm_id: None,
                payload: input.payload.clone(),
                created_at: "0".into(),
                updated_at: "1".into(),
            };
            map.insert(external_id, entry.clone());
            Ok(entry)
        }
        fn delete(&self, entry_id: &str) -> SyncResult<bool> {
            let mut map = self.by_external.borrow_mut();
            let key = map
                .iter()
                .find(|(_, entry)| entry.id == entry_id)
                .map(|(key, _)| key.clone());
            Ok(key.map(|key| map.remove(&key)).is_some())
        }
    }

    impl ExportStore for FakeStore {
        fn queue(&self) -> SyncResult<Vec<(ExportOptIn, Entry)>> {
            Ok(self.exports.borrow().clone())
        }
        fn record_push(&self, entry_id: &str, google_event_id: &str) -> SyncResult<()> {
            self.pushes_recorded
                .borrow_mut()
                .push((entry_id.into(), google_event_id.into()));
            Ok(())
        }
    }

    fn settings() -> Settings {
        Settings {
            tz: HomeTimezone::parse("Europe/Berlin").unwrap(),
            calendar_id: "primary".into(),
            google: GoogleConfig::default(),
        }
    }

    fn fixture_events() -> Vec<GoogleEvent> {
        serde_json::from_str::<EventsPage>(PAGE).unwrap().items
    }

    #[test]
    fn importing_twice_produces_one_entry_per_event() {
        let store = FakeStore::default();
        let api = FixtureApi::new(fixture_events());

        let first = import(&store, &api, &settings(), false).unwrap();
        assert_eq!(first.fetched, 3);
        assert_eq!(first.created, 3);
        assert_eq!(store.count(), 3);

        let second = import(&store, &api, &settings(), false).unwrap();
        assert_eq!(second.created, 0, "nothing new on a repeat run");
        assert_eq!(second.refreshed, 0, "and nothing rewritten either");
        assert_eq!(second.unchanged, 3);
        assert_eq!(store.count(), 3, "still one entry per Google event");
    }

    #[test]
    fn every_imported_event_arrives_as_a_neutral_draft() {
        let store = FakeStore::default();
        import(&store, &FixtureApi::new(fixture_events()), &settings(), false).unwrap();
        for entry in store.by_external.borrow().values() {
            assert_eq!(
                entry.commitment,
                google::IMPORT_COMMITMENT,
                "{} is not a draft",
                entry.title
            );
            assert_eq!(entry.source, "google");
            assert_eq!(
                crate::correlate::impact(&entry.kind, entry.commitment),
                crate::correlate::Feasibility::Free
            );
        }
    }

    #[test]
    fn a_moved_google_event_refreshes_a_draft_but_not_a_confirmed_entry() {
        let store = FakeStore::default();
        let mut events = fixture_events();
        import(&store, &FixtureApi::new(events.clone()), &settings(), false).unwrap();

        // The operator adopts one of the three.
        let mut confirmed = store.get("3q7l9v1c8m2p4t6y0x5z");
        confirmed.kind = "work_onsite".into();
        confirmed.commitment = crate::model::Commitment::Committed;
        confirmed.title = "Sprint-Review (verschoben)".into();
        store.seed(confirmed);

        // Google moves both that one and a still-draft one by an hour.
        events[0].start.date_time = Some("2026-08-14T14:00:00+02:00".into());
        events[0].end.date_time = Some("2026-08-14T15:00:00+02:00".into());
        events[2].start.date_time = Some("2026-08-19T09:30:00+02:00".into());
        events[2].end.date_time = Some("2026-08-19T09:45:00+02:00".into());

        let report = import(&store, &FixtureApi::new(events), &settings(), false).unwrap();
        assert_eq!(report.kept_axon_version, 1);
        assert_eq!(report.refreshed, 1);
        assert_eq!(report.unchanged, 1);

        let axon = store.get("3q7l9v1c8m2p4t6y0x5z");
        assert_eq!(axon.starts_at, "2026-08-14T10:00:00", "Axon wins the collision");
        assert_eq!(axon.title, "Sprint-Review (verschoben)");
        assert_eq!(axon.kind, "work_onsite");

        let draft = store.get("base9x7v5t3r1p_20260819T063000Z");
        assert_eq!(draft.starts_at, "2026-08-19T09:30:00", "an unadopted draft follows Google");

        // The divergence is reported, not swallowed.
        let kept = report
            .outcomes
            .iter()
            .find(|o| o.action == Action::KeepAxonVersion)
            .unwrap();
        assert_eq!(
            kept.google_says.as_ref().unwrap()["start"]["dateTime"],
            "2026-08-14T14:00:00+02:00"
        );
    }

    #[test]
    fn a_cancellation_removes_an_untouched_draft_and_spares_an_adopted_one() {
        let store = FakeStore::default();
        let events = fixture_events();
        import(&store, &FixtureApi::new(events.clone()), &settings(), false).unwrap();

        let mut adopted = store.get("7h2k4n6q8s0u2w4y6a8c");
        adopted.kind = "away".into();
        adopted.commitment = crate::model::Commitment::Committed;
        store.seed(adopted);

        let cancelled: Vec<GoogleEvent> = events
            .iter()
            .map(|event| GoogleEvent {
                id: event.id.clone(),
                status: Some("cancelled".into()),
                ..Default::default()
            })
            .collect();

        let report = import(&store, &FixtureApi::new(cancelled), &settings(), false).unwrap();
        assert_eq!(report.dropped_drafts, 2);
        assert_eq!(report.kept_axon_version, 1);
        assert_eq!(store.count(), 1);
        assert_eq!(store.get("7h2k4n6q8s0u2w4y6a8c").kind, "away");
    }

    #[test]
    fn a_dry_run_writes_nothing() {
        let store = FakeStore::default();
        let report = import(&store, &FixtureApi::new(fixture_events()), &settings(), true).unwrap();
        assert_eq!(report.created, 3);
        assert!(report.dry_run);
        assert_eq!(store.count(), 0, "a dry run reports what it would do, and does not");
    }

    #[test]
    fn review_is_read_only_and_groups_only_exact_title_and_time_twins() {
        let store = FakeStore::default();
        let mut events = fixture_events();
        let mut copy = events[0].clone();
        copy.id = "same-meeting-different-google-id".into();
        copy.summary = Some("TEAM  sync".into());
        events.push(copy);

        let preview = preview(
            &store,
            &FixtureApi::new(events),
            &settings(),
            "2026-08-01",
            "2026-08-31",
        )
        .unwrap();
        assert_eq!(store.count(), 0, "review must never write a draft");
        let twins: Vec<&ImportCandidate> = preview
            .candidates
            .iter()
            .filter(|candidate| candidate.duplicate_group.is_some())
            .collect();
        assert_eq!(twins.len(), 2);
        assert!(twins.iter().all(|candidate| candidate.status == ReviewStatus::LikelyDuplicate));
        assert_eq!(twins[0].duplicate_group, twins[1].duplicate_group);
    }

    #[test]
    fn selected_import_writes_only_the_explicit_current_choice() {
        let store = FakeStore::default();
        let events = fixture_events();
        let preview = preview(
            &store,
            &FixtureApi::new(events.clone()),
            &settings(),
            "2026-08-01",
            "2026-08-31",
        )
        .unwrap();
        let selected = preview
            .candidates
            .iter()
            .find(|candidate| candidate.title == "Urlaub")
            .unwrap();
        let report = import_selected(
            &store,
            &FixtureApi::new(events),
            &settings(),
            "2026-08-01",
            "2026-08-31",
            &[SelectedGoogleEvent {
                google_event_id: selected.google_event_id.clone(),
                google_updated: selected.google_updated.clone(),
            }],
        )
        .unwrap();

        assert_eq!(report.created, 1);
        assert_eq!(store.count(), 1);
        assert_eq!(store.get(&selected.google_event_id).title, "Urlaub");
    }

    #[test]
    fn selected_import_refuses_two_members_of_one_duplicate_group() {
        let store = FakeStore::default();
        let mut events = fixture_events();
        let mut copy = events[0].clone();
        copy.id = "second-team-sync".into();
        events.push(copy);

        let error = import_selected(
            &store,
            &FixtureApi::new(events),
            &settings(),
            "2026-08-01",
            "2026-08-31",
            &[
                SelectedGoogleEvent {
                    google_event_id: "3q7l9v1c8m2p4t6y0x5z".into(),
                    google_updated: None,
                },
                SelectedGoogleEvent {
                    google_event_id: "second-team-sync".into(),
                    google_updated: None,
                },
            ],
        )
        .unwrap_err();

        assert!(error.contains("at most one"), "{error}");
        assert_eq!(store.count(), 0);
    }

    #[test]
    fn selected_import_rejects_a_google_revision_that_changed_after_review() {
        let store = FakeStore::default();
        let mut reviewed_events = fixture_events();
        reviewed_events[0].updated = Some("2026-08-01T10:00:00Z".into());
        let mut current_events = reviewed_events.clone();
        current_events[0].updated = Some("2026-08-01T11:00:00Z".into());

        let error = import_selected(
            &store,
            &FixtureApi::new(current_events),
            &settings(),
            "2026-08-01",
            "2026-08-31",
            &[SelectedGoogleEvent {
                google_event_id: reviewed_events[0].id.clone(),
                google_updated: reviewed_events[0].updated.clone(),
            }],
        )
        .unwrap_err();

        assert!(error.contains("changed since the preview"), "{error}");
        assert_eq!(store.count(), 0, "a stale preview must not write anything");
    }

    #[test]
    fn review_rejects_an_unbounded_window() {
        let error = review_window("2026-08-01", "2026-11-01").unwrap_err();
        assert!(error.contains("90 days"), "{error}");
    }

    #[test]
    fn an_unmappable_event_is_skipped_with_its_reason_and_the_rest_still_import() {
        let store = FakeStore::default();
        let mut events = fixture_events();
        events.push(GoogleEvent {
            id: "broken".into(),
            status: Some("confirmed".into()),
            summary: Some("Kein Ende".into()),
            start: EventTime {
                date_time: Some("2026-08-21T10:00:00+02:00".into()),
                ..Default::default()
            },
            ..Default::default()
        });

        let report = import(&store, &FixtureApi::new(events), &settings(), false).unwrap();
        assert_eq!(report.created, 3);
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(report.skipped[0].google_event_id, "broken");
        assert!(report.skipped[0].reason.contains("no end"), "{:?}", report.skipped[0]);
        assert_eq!(store.count(), 3);
    }

    #[test]
    fn nothing_exports_without_an_opt_in() {
        let store = FakeStore::default();
        let api = FixtureApi::new(vec![]);
        let report = export(&store, &api, &settings(), false).unwrap();
        assert_eq!(report.opted_in, 0);
        assert_eq!(report.inserted, 0);
        assert!(api.pushes.borrow().is_empty(), "an empty ledger pushes nothing");
    }

    #[test]
    fn an_opted_in_entry_inserts_once_then_patches() {
        let store = FakeStore::default();
        let entry = Entry {
            id: "cal:entry:42".into(),
            kind: "event".into(),
            commitment: crate::model::Commitment::Committed,
            title: "Vortrag".into(),
            starts_at: "2026-08-14T18:00:00".into(),
            ends_at: "2026-08-14T20:00:00".into(),
            all_day: false,
            location: Some("München".into()),
            notes: None,
            source: "manual".into(),
            external_id: None,
            rhythm_id: None,
            payload: Value::Null,
            created_at: "0".into(),
            updated_at: "0".into(),
        };
        let optin = ExportOptIn {
            entry_id: entry.id.clone(),
            google_calendar_id: "primary".into(),
            google_event_id: None,
            pushed_at: None,
            created_at: "0".into(),
        };
        *store.exports.borrow_mut() = vec![(optin.clone(), entry.clone())];

        let api = FixtureApi::new(vec![]);
        let first = export(&store, &api, &settings(), false).unwrap();
        assert_eq!(first.inserted, 1);
        assert_eq!(first.patched, 0);
        assert_eq!(
            store.pushes_recorded.borrow()[0],
            ("cal:entry:42".into(), "g-inserted-1".into())
        );
        assert_eq!(
            api.pushes.borrow()[0].2["start"]["dateTime"],
            "2026-08-14T18:00:00+02:00"
        );

        // The ledger now knows the remote id, so the next run updates it.
        *store.exports.borrow_mut() = vec![(
            ExportOptIn {
                google_event_id: Some("g-inserted-1".into()),
                ..optin
            },
            entry,
        )];
        let second = export(&store, &api, &settings(), false).unwrap();
        assert_eq!(second.patched, 1);
        assert_eq!(second.inserted, 0);
        assert_eq!(api.pushes.borrow()[1].1.as_deref(), Some("g-inserted-1"));
    }

    #[test]
    fn an_export_run_pushes_to_the_calendar_the_entry_was_opted_in_against() {
        // Not the currently configured one: re-pointing the config must not
        // silently relocate an event that already lives somewhere else.
        let store = FakeStore::default();
        let entry = Entry {
            id: "cal:entry:7".into(),
            kind: "event".into(),
            commitment: crate::model::Commitment::Committed,
            title: "Woanders".into(),
            starts_at: "2026-08-17".into(),
            ends_at: "2026-08-18".into(),
            all_day: true,
            location: None,
            notes: None,
            source: "manual".into(),
            external_id: None,
            rhythm_id: None,
            payload: Value::Null,
            created_at: "0".into(),
            updated_at: "0".into(),
        };
        *store.exports.borrow_mut() = vec![(
            ExportOptIn {
                entry_id: entry.id.clone(),
                google_calendar_id: "team@group.calendar.google.com".into(),
                google_event_id: None,
                pushed_at: None,
                created_at: "0".into(),
            },
            entry,
        )];

        let api = FixtureApi::new(vec![]);
        export(&store, &api, &settings(), false).unwrap();
        assert_eq!(api.pushes.borrow()[0].0, "team@group.calendar.google.com");
    }

    #[test]
    fn a_missing_credential_file_names_the_path_and_the_keys() {
        let missing = std::env::temp_dir().join("axon-calendar-does-not-exist.env");
        let error = read_env_key(&missing, "GOOGLE_CLIENT_ID").unwrap_err();
        assert!(error.contains("axon-calendar-does-not-exist.env"), "{error}");
        assert!(error.contains("GOOGLE_REFRESH_TOKEN"), "{error}");
        assert!(error.contains("never part of this repo"), "{error}");
    }

    #[test]
    fn an_empty_value_is_missing_not_present() {
        let dir = std::env::temp_dir().join(format!("axon-calendar-env-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("calendar.env");
        std::fs::write(
            &path,
            "GOOGLE_CLIENT_ID=1234.apps.example\nGOOGLE_CLIENT_SECRET=\n",
        )
        .unwrap();

        assert_eq!(
            read_env_key(&path, "GOOGLE_CLIENT_ID").unwrap(),
            "1234.apps.example"
        );
        let empty = read_env_key(&path, "GOOGLE_CLIENT_SECRET").unwrap_err();
        assert!(empty.contains("missing or empty"), "{empty}");
        let absent = read_env_key(&path, "GOOGLE_REFRESH_TOKEN").unwrap_err();
        assert!(absent.contains("GOOGLE_REFRESH_TOKEN"), "{absent}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn settings_refuse_to_run_without_a_home_timezone() {
        let config = Config {
            database_url: "host=127.0.0.1".into(),
            port: 8087,
            home_timezone: None,
            home_city: None,
            trips_base_url: String::new(),
            google: GoogleConfig {
                calendar_id: Some("primary".into()),
                ..Default::default()
            },
        };
        let error = Settings::resolve(&config).unwrap_err();
        assert!(error.contains("home_timezone is not configured"), "{error}");
        // Assert against the path this environment actually resolves, not a
        // literal. `config_path()` answers the overlay's `config/calendar.json`
        // on a configured machine and falls back to the repo's
        // `calendar.config.json` where there is no overlay — and
        // "calendar.config.json" does not contain "calendar.json", so a
        // hardcoded literal passes on any developer machine and fails in CI.
        // It did, on this test's first remote run. The contract worth pinning is
        // the one `config_path` documents: the error names the exact file the
        // operator has to create, whichever that is here.
        let expected = crate::config::config_path();
        assert!(
            error.contains(&format!("{expected:?}")),
            "error should name {expected:?}, got: {error}"
        );
    }

    #[test]
    fn settings_refuse_to_guess_which_calendar() {
        let config = Config {
            database_url: "host=127.0.0.1".into(),
            port: 8087,
            home_timezone: Some("Europe/Berlin".into()),
            home_city: None,
            trips_base_url: String::new(),
            google: GoogleConfig::default(),
        };
        let error = Settings::resolve(&config).unwrap_err();
        assert!(error.contains("calendar_id is not configured"), "{error}");
        assert!(error.contains("no default"), "{error}");
    }

    #[test]
    fn settings_resolve_when_both_are_present() {
        let config = Config {
            database_url: "host=127.0.0.1".into(),
            port: 8087,
            home_timezone: Some("Europe/Berlin".into()),
            home_city: None,
            trips_base_url: String::new(),
            google: GoogleConfig {
                calendar_id: Some("  primary  ".into()),
                ..Default::default()
            },
        };
        let settings = Settings::resolve(&config).unwrap();
        assert_eq!(settings.calendar_id, "primary");
        assert_eq!(settings.tz.name(), "Europe/Berlin");
    }

    #[test]
    fn the_import_window_is_utc_marked_and_never_empty() {
        let today = date::parse_date("2026-08-14").unwrap();
        let (from, to) = import_window(today, 7, 120);
        assert_eq!(from, "2026-08-07T00:00:00Z");
        assert_eq!(to, "2026-12-12T00:00:00Z");

        // A nonsensical config still produces a forward-looking window rather
        // than an inverted one Google would reject.
        let (from, to) = import_window(today, -5, 0);
        assert_eq!(from, "2026-08-14T00:00:00Z");
        assert_eq!(to, "2026-08-15T00:00:00Z");
    }

    #[test]
    fn a_secondary_calendar_id_is_url_encoded() {
        assert_eq!(encode_segment("primary"), "primary");
        assert_eq!(
            encode_segment("abc123@group.calendar.google.com"),
            "abc123%40group.calendar.google.com"
        );
        assert_eq!(encode_segment("a b/c"), "a%20b%2Fc");
    }

    #[test]
    fn http_failures_name_the_likely_cause() {
        assert!(fail_for(reqwest::StatusCode::FORBIDDEN, "events.insert").contains("scope"));
        assert!(fail_for(reqwest::StatusCode::NOT_FOUND, "events.list").contains("calendar_id"));
        assert!(fail_for(reqwest::StatusCode::UNAUTHORIZED, "events.list").contains("revoked"));
        let plain = fail_for(reqwest::StatusCode::BAD_GATEWAY, "events.list");
        assert!(plain.contains("502"), "{plain}");
    }

    #[test]
    fn exportable_filters_out_what_may_never_be_pushed() {
        let base = Entry {
            id: "cal:entry:1".into(),
            kind: "event".into(),
            commitment: crate::model::Commitment::Committed,
            title: "Vortrag".into(),
            starts_at: "2026-08-14T18:00:00".into(),
            ends_at: "2026-08-14T20:00:00".into(),
            all_day: false,
            location: None,
            notes: None,
            source: "manual".into(),
            external_id: None,
            rhythm_id: None,
            payload: Value::Null,
            created_at: "0".into(),
            updated_at: "0".into(),
        };
        let imported = Entry {
            id: "cal:entry:2".into(),
            source: "google".into(),
            kind: "draft".into(),
            ..base.clone()
        };
        let generated = Entry {
            id: "cal:entry:3".into(),
            rhythm_id: Some("cal:rhythm:1".into()),
            ..base.clone()
        };
        let entries = [base, imported, generated];
        let allowed = exportable(&entries);
        assert_eq!(allowed.len(), 1);
        assert_eq!(allowed[0].id, "cal:entry:1");
    }
}
