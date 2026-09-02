//! `check`: which wildcard listeners this host was not told to expect.
//!
//! The policy is the whole machine-specific half of this capability, and it lives in the
//! overlay at `<overlay>/config/host-net-policy.toml`. Nothing in this crate carries a process
//! name (README.md#generic-in-axon-specific-in-the-overlay).

use serde::{Deserialize, Serialize};

use crate::listen::{Listener, Scope};

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Policy {
    #[serde(default)]
    pub expect_wildcard: Vec<ExpectWildcard>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ExpectWildcard {
    /// The executable's BASENAME, as `ps` reports it.
    ///
    /// Not netstat's field, which is truncated to sixteen characters, and not a port. Tailscale
    /// assigns its extension's wildcard ports per start — two different numbers on two runs of
    /// the build host — so a policy keyed on a port would need editing after every restart, and
    /// a check nobody can keep current is a check that gets deleted.
    pub process: String,
    /// Why this listener is allowed to be reachable from everywhere. Required, because the
    /// reason is exactly what a scanner cannot infer — the same contract
    /// `schemas/host-watch-policy.toml.example` states for its allowlist.
    pub reason: String,
}

/// One wildcard listener the policy does not account for.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Exposure {
    pub process: String,
    pub port: String,
    /// Every protocol this process/port pair is bound on, joined: `tcp4+tcp6`.
    pub protos: String,
    pub pid: u32,
    pub path: Option<String>,
}

impl Exposure {
    pub fn describe(&self) -> String {
        format!("{} on *:{} ({})", self.process, self.port, self.protos)
    }
}

/// Wildcard listeners minus the expected ones, one row per process and port.
///
/// Collapsed on `(process, port)` because a service that binds both `tcp4` and `tcp6` is one
/// decision, not two, and printing it twice is how a short report becomes one nobody reads.
pub fn unexpected(listeners: &[Listener], policy: &Policy) -> Vec<Exposure> {
    let mut out: Vec<Exposure> = Vec::new();
    for l in listeners
        .iter()
        .filter(|l| l.scope == Scope::Wildcard)
        .filter(|l| {
            !policy
                .expect_wildcard
                .iter()
                .any(|e| e.process == l.process)
        })
    {
        match out
            .iter_mut()
            .find(|e| e.process == l.process && e.port == l.port)
        {
            Some(e) => {
                if !e.protos.split('+').any(|p| p == l.proto) {
                    e.protos.push('+');
                    e.protos.push_str(&l.proto);
                }
            }
            None => out.push(Exposure {
                process: l.process.clone(),
                port: l.port.clone(),
                protos: l.proto.clone(),
                pid: l.pid,
                path: l.path.clone(),
            }),
        }
    }
    out.sort_by(|a, b| (&a.process, &a.port).cmp(&(&b.process, &b.port)));
    out
}

/// Read the overlay policy. `Ok(None)` means there is no overlay to read one from, which the
/// caller reports as a setup problem rather than as a clean host.
pub fn load() -> Result<Option<(std::path::PathBuf, Policy)>, String> {
    let Some(path) = axon_config::overlay_config("host-net-policy.toml") else {
        return Ok(None);
    };
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let policy: Policy = toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(Some((path, policy)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::listen::{assemble, parse_ifconfig, parse_launchctl, parse_netstat, parse_ps};

    fn listeners() -> Vec<Listener> {
        let sockets = parse_netstat(include_str!("../fixtures/netstat-tcp.txt"));
        assemble(
            &sockets,
            &parse_ps(include_str!("../fixtures/ps.txt")),
            &parse_launchctl(include_str!("../fixtures/launchctl.txt")),
            &parse_ifconfig(include_str!("../fixtures/ifconfig.txt")),
        )
    }

    fn policy(toml_text: &str) -> Policy {
        toml::from_str(toml_text).unwrap()
    }

    /// The match is on the basename `ps` reports, never on netstat's truncated field. The
    /// fixture's extension is `example-vpn-netw` to netstat and
    /// `example-vpn-network-extension` to ps; a policy naming the truncated form must NOT
    /// silence it, or the policy file becomes a list of strings that happen to be sixteen
    /// characters long.
    #[test]
    fn an_expected_entry_matches_the_full_basename() {
        let rows = listeners();
        let truncated = policy(
            "[[expect_wildcard]]\nprocess = \"example-vpn-netw\"\nreason = \"truncated form\"\n",
        );
        assert!(unexpected(&rows, &truncated)
            .iter()
            .any(|e| e.process == "example-vpn-network-extension"));

        let full = policy(
            "[[expect_wildcard]]\nprocess = \"example-vpn-network-extension\"\nreason = \"the real name\"\n",
        );
        assert!(!unexpected(&rows, &full)
            .iter()
            .any(|e| e.process == "example-vpn-network-extension"));
    }

    #[test]
    fn only_wildcard_binds_can_ever_be_unexpected() {
        let rows = listeners();
        let found = unexpected(&rows, &Policy::default());
        assert!(!found.is_empty());
        // Loopback listeners are in the fixture and must never appear here.
        assert!(rows.iter().any(|l| l.scope == Scope::Loopback));
        assert!(!found.iter().any(|e| e.process == "launchd"));
    }

    /// tcp4 and tcp6 on the same port are one decision.
    #[test]
    fn one_row_per_process_and_port() {
        let rows = listeners();
        let found = unexpected(&rows, &Policy::default());
        let rapportd: Vec<&Exposure> = found.iter().filter(|e| e.process == "rapportd").collect();
        assert_eq!(rapportd.len(), 1);
        assert_eq!(rapportd[0].describe(), "rapportd on *:59039 (tcp4+tcp6)");
    }

    #[test]
    fn a_fully_declared_host_has_nothing_unexpected() {
        let rows = listeners();
        let all = policy(
            r#"
[[expect_wildcard]]
process = "example-vpn-network-extension"
reason = "the VPN extension's relay port"
[[expect_wildcard]]
process = "rapportd"
reason = "stock macOS continuity"
[[expect_wildcard]]
process = "interceptor-daemon"
reason = "an operator-accepted daemon"
"#,
        );
        assert_eq!(unexpected(&rows, &all), vec![]);
    }
}
