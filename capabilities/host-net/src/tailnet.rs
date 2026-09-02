//! The tailnet's posture, as three unprivileged reads: `tailscale status --json`,
//! `tailscale debug prefs` and `tailscale lock status`.
//!
//! # Nothing here prints a key
//!
//! `tailscale lock status` writes real key material to stdout — the node's own tailnet-lock
//! key, a KeyID and a WrappingPubkey, all in full. This verb takes one fact from that output,
//! the number of trusted signing keys, and drops the rest before it reaches any surface. The
//! table and `--json` both print the count; neither prints a key. An operator who wants the
//! keys runs the tool that owns them.
//!
//! Node names are treated the same way, one step softer: the table prints counts only, and
//! `--json` names a peer only when this verb has flagged that peer.

use serde::Serialize;
use serde_json::Value;

use crate::sys::capture;

#[derive(Debug, Clone, Default, Serialize)]
pub struct Prefs {
    /// Shields up blocks all incoming tailnet connections. It is NOT in `status --json` —
    /// measured — which is why prefs is a separate read rather than a field lookup.
    pub shields_up: Option<bool>,
    pub route_all: Option<bool>,
    pub run_ssh: Option<bool>,
    pub posture_checking: Option<bool>,
    pub want_running: Option<bool>,
    pub advertised_routes: usize,
    pub uses_exit_node: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Lock {
    pub enabled: bool,
    /// Count only. See this module's header.
    pub trusted_keys: usize,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct TailnetReport {
    pub backend_state: String,
    pub tun: Option<bool>,
    pub peers: usize,
    pub peers_online: usize,
    /// Peers whose node key does not expire, by short name. A key with no expiry is a
    /// credential that outlives every rotation policy the tailnet has.
    pub peers_never_expiring: Vec<String>,
    pub self_key_expires: bool,
    pub prefs: Prefs,
    pub lock: Lock,
}

fn as_bool(v: &Value, key: &str) -> Option<bool> {
    v.get(key).and_then(Value::as_bool)
}

/// The leading label of a MagicDNS name: `host.tail1234.ts.net` → `host`. The tailnet's own
/// suffix identifies the deployment and is dropped here rather than carried around.
fn short_name(peer: &Value) -> String {
    let dns = peer.get("DNSName").and_then(Value::as_str).unwrap_or("");
    let first = dns.split('.').next().unwrap_or("");
    if !first.is_empty() {
        return first.to_string();
    }
    peer.get("HostName")
        .and_then(Value::as_str)
        .unwrap_or("(unnamed)")
        .to_string()
}

/// Parse `tailscale status --json`.
///
/// `KeyExpiry` is ABSENT, not null, on a node whose key does not expire — tailscale omits the
/// field. Treating "absent" as "unknown, assume fine" is the failure this test pins: on the
/// build host exactly one of three peers omits it, and that peer is the one worth knowing about.
pub fn parse_status(text: &str) -> Result<TailnetReport, String> {
    let v: Value =
        serde_json::from_str(text).map_err(|e| format!("tailscale status --json: {e}"))?;
    let mut r = TailnetReport {
        backend_state: v
            .get("BackendState")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string(),
        tun: as_bool(&v, "TUN"),
        self_key_expires: v
            .get("Self")
            .and_then(|s| s.get("KeyExpiry"))
            .is_some_and(|k| !k.is_null()),
        ..Default::default()
    };
    if let Some(peers) = v.get("Peer").and_then(Value::as_object) {
        r.peers = peers.len();
        for peer in peers.values() {
            if peer.get("Online").and_then(Value::as_bool) == Some(true) {
                r.peers_online += 1;
            }
            if peer.get("KeyExpiry").is_none_or(Value::is_null) {
                r.peers_never_expiring.push(short_name(peer));
            }
        }
        r.peers_never_expiring.sort();
    }
    Ok(r)
}

/// Parse `tailscale debug prefs`.
pub fn parse_prefs(text: &str) -> Result<Prefs, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("tailscale debug prefs: {e}"))?;
    Ok(Prefs {
        shields_up: as_bool(&v, "ShieldsUp"),
        route_all: as_bool(&v, "RouteAll"),
        run_ssh: as_bool(&v, "RunSSH"),
        posture_checking: as_bool(&v, "PostureChecking"),
        want_running: as_bool(&v, "WantRunning"),
        advertised_routes: v
            .get("AdvertiseRoutes")
            .and_then(Value::as_array)
            .map_or(0, Vec::len),
        uses_exit_node: v
            .get("ExitNodeID")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty()),
    })
}

/// Parse `tailscale lock status` down to two facts and discard the rest.
///
/// "NOT enabled" contains the word "enabled", so the negative forms are tested first. Getting
/// that backwards reports a tailnet with no lock as locked, which is the one error in this
/// function that would be reassuring rather than noisy.
pub fn parse_lock(text: &str) -> Lock {
    let mut lock = Lock::default();
    let mut in_keys = false;
    for line in text.lines() {
        let l = line.trim();
        let upper = l.to_ascii_uppercase();
        if upper.starts_with("TAILNET LOCK IS") {
            lock.enabled = !upper.contains("NOT ENABLED")
                && !upper.contains("DISABLED")
                && upper.contains("ENABLED");
        }
        if upper.starts_with("TRUSTED SIGNING KEYS") {
            in_keys = true;
            continue;
        }
        if in_keys && l.starts_with("tlpub:") {
            lock.trusted_keys += 1;
        }
    }
    lock
}

/// Read the tailnet. Both `debug prefs` and `lock status` are optional: a node that is not
/// signed in still has a `status`, and a report that refuses to print anything because one of
/// three reads failed is less useful than one that says which part is missing.
pub fn collect() -> Result<TailnetReport, String> {
    let status = capture("tailscale", &["status", "--json"])?;
    let mut report = parse_status(&status)?;
    if let Ok(text) = capture("tailscale", &["debug", "prefs"]) {
        report.prefs = parse_prefs(&text).unwrap_or_default();
    }
    if let Ok(text) = capture("tailscale", &["lock", "status"]) {
        report.lock = parse_lock(&text);
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATUS: &str = include_str!("../fixtures/tailscale-status.json");
    const PREFS: &str = include_str!("../fixtures/tailscale-prefs.json");
    const LOCK: &str = include_str!("../fixtures/tailscale-lock.txt");

    #[test]
    fn a_peer_with_no_key_expiry_is_flagged() {
        let r = parse_status(STATUS).unwrap();
        assert_eq!(r.peers, 3);
        assert_eq!(r.peers_online, 2);
        assert_eq!(r.peers_never_expiring, vec!["example-node-b".to_string()]);
        assert!(r.self_key_expires);
    }

    /// ShieldsUp is in prefs and NOT in `status --json`. A test that read it from a status mock
    /// would pass here and report the wrong answer against the real tool.
    #[test]
    fn shields_up_comes_from_prefs_not_status() {
        let status: Value = serde_json::from_str(STATUS).unwrap();
        assert!(status.get("ShieldsUp").is_none());
        let p = parse_prefs(PREFS).unwrap();
        assert_eq!(p.shields_up, Some(false));
        assert_eq!(p.route_all, Some(true));
        assert_eq!(p.want_running, Some(false));
        assert_eq!(p.advertised_routes, 0);
        assert!(!p.uses_exit_node);
    }

    /// One trusted signing key is one lost laptop away from a tailnet nobody can re-sign into.
    #[test]
    fn lock_reports_a_count_and_no_key_material() {
        let l = parse_lock(LOCK);
        assert!(l.enabled);
        assert_eq!(l.trusted_keys, 1);
        let rendered = serde_json::to_string(&l).unwrap();
        assert!(
            !rendered.contains("tlpub:"),
            "no key material leaves this parser"
        );
    }

    #[test]
    fn a_tailnet_without_lock_is_not_read_as_locked() {
        assert!(!parse_lock("Tailnet Lock is NOT enabled.\n").enabled);
        assert!(!parse_lock("Tailnet lock is disabled.\n").enabled);
    }
}
