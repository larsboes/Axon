//! The one call trips makes to interior: `POST /api/items`, once per gear proposal.
//!
//! Written the way `finance_client` is written, and for the same reason — the failure has a
//! type carrying a reason, and nothing here can turn a refused write into a silent success.
//!
//! **Why HTTP and not SQL.** Both capabilities resolve to the same database file, so a direct
//! `INSERT` would work and would be wrong: Q73's write-path rule says a capability's rows are
//! written by its own handler. Interior's handler is also what makes this idempotent for free
//! — it answers 409 for an id that already exists, so a second `--apply` over the same notes
//! changes nothing and says so per row.

use std::time::Duration;

use serde::Serialize;

/// Where interior listens. `capabilities/interior/service.toml` declares 8092. The fourth
/// capability to hardcode a sibling's port, which is the same argument `finance_client`
/// already records for the spine mechanism that would end it.
pub fn interior_base_url() -> String {
    std::env::var("AXON_INTERIOR_URL").unwrap_or_else(|_| "http://127.0.0.1:8092".to_string())
}

/// Long enough for a loopback write, short enough that a stopped interior fails the import
/// rather than hanging a terminal.
const TIMEOUT: Duration = Duration::from_secs(5);

/// What one proposal became.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Written {
    /// Interior created the row.
    Created,
    /// The id was already there. Not an error: the import is meant to be re-runnable, and
    /// this is the answer that makes it so.
    AlreadyThere,
    /// Interior answered, and refused. The message is its own.
    Refused(String),
    /// Interior did not answer at all.
    Unreachable(String),
}

/// `POST /api/items` for one gear proposal.
///
/// `state` is `wanted` for gear the operator does not own yet and `owned` otherwise; the
/// caller decides, because the note knows and this file does not. Interior requires the
/// field — a row with no state joins to nothing and appears in no list.
pub fn create_item<T: Serialize>(item: &T, state: &str, note: &str) -> Written {
    let body = match serde_json::to_value(item) {
        Ok(serde_json::Value::Object(mut map)) => {
            map.insert("state".into(), serde_json::Value::String(state.to_string()));
            map.insert("note".into(), serde_json::Value::String(note.to_string()));
            serde_json::Value::Object(map)
        }
        // A proposal that does not serialise to an object is a programming error here, not a
        // condition of the world, so it is reported and never guessed past.
        Ok(other) => return Written::Refused(format!("proposal is not an object: {other}")),
        Err(error) => return Written::Refused(error.to_string()),
    };

    let client = match axon_http::client(axon_http::Purpose::new("trips-interior"), TIMEOUT) {
        Ok(client) => client,
        Err(error) => return Written::Unreachable(format!("interior client: {error}")),
    };
    let mut request = client
        .post(format!("{}/api/items", interior_base_url()))
        .json(&body);
    // The inbound gate covers every route but /health and /ready. Without the token a gated
    // interior reads as "not running", which would turn a refused write into a false
    // unreachable — the same rule `finance_client` states, resolved per request so a rotated
    // token needs no restart.
    if let Some(bearer) = axon_server::InboundAuth::from_deployment().bearer_header() {
        request = request.header(reqwest::header::AUTHORIZATION, bearer);
    }
    let response = match request.send() {
        Ok(response) => response,
        // `without_url` for the same reason finance_client gives: the URL is already known
        // to the reader and reqwest prints it whole inside the message.
        Err(error) => return Written::Unreachable(error.without_url().to_string()),
    };

    match response.status().as_u16() {
        201 => Written::Created,
        409 => Written::AlreadyThere,
        status => Written::Refused(format!(
            "{status}: {}",
            response
                .text()
                .unwrap_or_else(|error| format!("(body unreadable: {error})"))
                .trim()
        )),
    }
}
