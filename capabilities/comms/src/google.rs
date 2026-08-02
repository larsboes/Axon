//! READ-ONLY Gmail access. The only Google endpoints this module touches:
//!   - OAuth token refresh: POST https://oauth2.googleapis.com/token
//!   - list threads:        GET  .../gmail/v1/users/me/threads?q=in:inbox
//!   - thread metadata:     GET  .../gmail/v1/users/me/threads/{id}?format=metadata
//!
//! There is intentionally NO modify/trash/delete/labels/send call here -- not
//! behind a flag. Credentials come from the overlay's `comms.env`
//! (GOOGLE_CLIENT_ID / GOOGLE_CLIENT_SECRET / GOOGLE_REFRESH_TOKEN); token
//! values are never logged.

use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;

use crate::{CommsError, Result};

const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const THREADS_URL: &str = "https://gmail.googleapis.com/gmail/v1/users/me/threads";
/// Courtesy pause between per-thread metadata fetches (see `thread_meta`).
const THREAD_FETCH_PAUSE: std::time::Duration = std::time::Duration::from_millis(60);

/// In-process token cache (never persisted). Refreshed when within 60s of expiry.
struct CachedToken {
    token: String,
    expires_at: u64,
}
static TOKEN_CACHE: OnceLock<Mutex<Option<CachedToken>>> = OnceLock::new();

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .user_agent("AxonComms/0.1")
        .gzip(true)
        .build()
        .map_err(CommsError::from)
}

/// Reads a single `KEY=value` from a plain env file. Values are never logged.
fn read_env_key(env_path: &Path, key: &str) -> Result<String> {
    let body = std::fs::read_to_string(env_path).map_err(|e| {
        CommsError::Config(format!("cannot read google env file {env_path:?}: {e}"))
    })?;
    body.lines()
        .find_map(|l| l.strip_prefix(&format!("{key}=")).map(|v| v.trim().to_string()))
        .filter(|v| !v.is_empty())
        .ok_or_else(|| CommsError::Config(format!("{key} missing or empty in {env_path:?}")))
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

/// Returns a valid access token, refreshing via the OAuth refresh grant when
/// the cache is empty or near expiry. The token string is cached in-process
/// only and never printed.
pub fn access_token(env_path: &Path) -> Result<String> {
    let cache = TOKEN_CACHE.get_or_init(|| Mutex::new(None));
    {
        let guard = cache.lock().unwrap();
        if let Some(c) = guard.as_ref() {
            if c.expires_at > now_secs() + 60 {
                return Ok(c.token.clone());
            }
        }
    }

    let client_id = read_env_key(env_path, "GOOGLE_CLIENT_ID")?;
    let client_secret = read_env_key(env_path, "GOOGLE_CLIENT_SECRET")?;
    let refresh_token = read_env_key(env_path, "GOOGLE_REFRESH_TOKEN")?;

    let resp = client()?
        .post(TOKEN_URL)
        .form(&[
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
            ("refresh_token", refresh_token.as_str()),
            ("grant_type", "refresh_token"),
        ])
        .send()?;

    if !resp.status().is_success() {
        // Deliberately does not echo the response body -- it can contain token
        // material on some error shapes.
        return Err(CommsError::Auth(format!(
            "token refresh failed with HTTP {}",
            resp.status()
        )));
    }

    let tok: TokenResponse = resp.json()?;
    let mut guard = cache.lock().unwrap();
    *guard = Some(CachedToken {
        token: tok.access_token.clone(),
        expires_at: now_secs() + tok.expires_in,
    });
    Ok(tok.access_token)
}

/// A thread id from the list endpoint.
#[derive(Debug, Clone)]
pub struct ThreadStub {
    pub id: String,
}

/// Metadata for one thread's latest message.
#[derive(Debug, Clone, Default)]
pub struct ThreadMeta {
    pub id: String,
    pub from_addr: Option<String>,
    pub subject: Option<String>,
    pub date: Option<String>,
    pub list_unsubscribe: Option<String>,
    pub snippet: Option<String>,
    pub label_ids: Vec<String>,
    /// internalDate of the latest message, epoch milliseconds.
    pub internal_date_ms: Option<i64>,
}

impl ThreadMeta {
    pub fn has_list_unsubscribe(&self) -> bool {
        self.list_unsubscribe.is_some()
    }
}

#[derive(Deserialize)]
struct ThreadListResponse {
    #[serde(default)]
    threads: Vec<ThreadListEntry>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}
#[derive(Deserialize)]
struct ThreadListEntry {
    id: String,
}

/// Lists inbox threads (read-only, `q=in:inbox`), paging until `limit` ids are
/// collected or the inbox is exhausted.
pub fn list_inbox_threads(token: &str, limit: usize) -> Result<Vec<ThreadStub>> {
    let http = client()?;
    let mut out: Vec<ThreadStub> = Vec::new();
    let mut page_token: Option<String> = None;

    while out.len() < limit {
        let remaining = limit - out.len();
        let page_size = remaining.min(100).to_string();
        let mut query: Vec<(&str, String)> = vec![
            ("q", "in:inbox".to_string()),
            ("maxResults", page_size),
        ];
        if let Some(pt) = &page_token {
            query.push(("pageToken", pt.clone()));
        }

        let resp = http.get(THREADS_URL).bearer_auth(token).query(&query).send()?;
        if !resp.status().is_success() {
            return Err(CommsError::Other(format!(
                "thread list failed with HTTP {}",
                resp.status()
            )));
        }
        let parsed: ThreadListResponse = resp.json()?;
        for t in parsed.threads {
            out.push(ThreadStub { id: t.id });
            if out.len() >= limit {
                break;
            }
        }
        match parsed.next_page_token {
            Some(pt) if out.len() < limit => page_token = Some(pt),
            _ => break,
        }
    }
    Ok(out)
}

#[derive(Deserialize)]
struct ThreadDetail {
    #[serde(default)]
    messages: Vec<MessageDetail>,
}
#[derive(Deserialize)]
struct MessageDetail {
    #[serde(default)]
    snippet: Option<String>,
    #[serde(rename = "labelIds", default)]
    label_ids: Vec<String>,
    #[serde(rename = "internalDate", default)]
    internal_date: Option<String>,
    #[serde(default)]
    payload: Option<MessagePayload>,
}
#[derive(Deserialize)]
struct MessagePayload {
    #[serde(default)]
    headers: Vec<Header>,
}
#[derive(Deserialize)]
struct Header {
    name: String,
    value: String,
}

fn find_header<'a>(headers: &'a [Header], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case(name))
        .map(|h| h.value.as_str())
}

/// Fetches metadata for one thread (read-only, `format=metadata`), reading the
/// latest message's headers/snippet/labels/internalDate. Sleeps briefly first
/// as a rate-limit courtesy between per-thread fetches.
pub fn thread_meta(token: &str, id: &str) -> Result<ThreadMeta> {
    std::thread::sleep(THREAD_FETCH_PAUSE);
    let http = client()?;
    let url = format!("{THREADS_URL}/{id}");
    let resp = http
        .get(&url)
        .bearer_auth(token)
        .query(&[
            ("format", "metadata"),
            ("metadataHeaders", "From"),
            ("metadataHeaders", "Subject"),
            ("metadataHeaders", "Date"),
            ("metadataHeaders", "List-Unsubscribe"),
        ])
        .send()?;
    if !resp.status().is_success() {
        return Err(CommsError::Other(format!(
            "thread metadata failed with HTTP {}",
            resp.status()
        )));
    }

    let detail: ThreadDetail = resp.json()?;
    let mut meta = ThreadMeta {
        id: id.to_string(),
        ..Default::default()
    };

    if let Some(msg) = detail.messages.last() {
        meta.snippet = msg.snippet.clone();
        meta.label_ids = msg.label_ids.clone();
        meta.internal_date_ms = msg.internal_date.as_ref().and_then(|s| s.parse::<i64>().ok());
        if let Some(payload) = &msg.payload {
            meta.from_addr = find_header(&payload.headers, "From").map(str::to_string);
            meta.subject = find_header(&payload.headers, "Subject").map(str::to_string);
            meta.date = find_header(&payload.headers, "Date").map(str::to_string);
            meta.list_unsubscribe =
                find_header(&payload.headers, "List-Unsubscribe").map(str::to_string);
        }
    }
    Ok(meta)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_env_key_errors_on_missing() {
        let dir = std::env::temp_dir().join(format!("comms-env-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("comms.env");
        std::fs::write(&f, "GOOGLE_CLIENT_ID=abc\nGOOGLE_CLIENT_SECRET=\n").unwrap();
        assert_eq!(read_env_key(&f, "GOOGLE_CLIENT_ID").unwrap(), "abc");
        assert!(read_env_key(&f, "GOOGLE_CLIENT_SECRET").is_err(), "empty value is an error");
        assert!(read_env_key(&f, "GOOGLE_REFRESH_TOKEN").is_err(), "missing key is an error");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn header_lookup_is_case_insensitive() {
        let headers = vec![
            Header { name: "From".into(), value: "a@b.com".into() },
            Header { name: "List-Unsubscribe".into(), value: "<mailto:x>".into() },
        ];
        assert_eq!(find_header(&headers, "from"), Some("a@b.com"));
        assert_eq!(find_header(&headers, "LIST-UNSUBSCRIBE"), Some("<mailto:x>"));
        assert_eq!(find_header(&headers, "Subject"), None);
    }
}
