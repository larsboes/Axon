//! What an outbound URL has to prove before a request is made.
//!
//! Promoted from `capabilities/comms/src/media.rs`, which was the only place in the
//! workspace that validated an outbound URL's destination — `rg` found no equivalent
//! anywhere else, while `capabilities/scouting` fetches URLs that come from adapter
//! config and remote feeds and `capabilities/places` fetches from a configured
//! geocoder base. The guard belongs on the way in to `client.get(url)`, which is
//! this crate, so one lib owns both halves of the same door.
//!
//! Two rules, and they close two halves of one hole:
//!
//! - [`check_scheme`] refuses anything that is not plain http(s). `file://` would
//!   make an extractor read the local disk.
//! - [`check_destination`] refuses a URL that resolves inside this machine or this
//!   network. `http://127.0.0.1:8086/api/plans` is still http, and every Axon service
//!   binds loopback (`libs/axon-server`), so without it an ingested link drives an
//!   internal API from the outside. CodeQL `rust/request-forgery` reported exactly
//!   that against comms' `extract_article`.
//!
//! Nothing here reads configuration. The allowlist [`check_destination`] consults is
//! the caller's, passed as a closure so a capability that has none pays nothing and
//! a capability that has one does not read its config file on the ordinary path.

use std::fmt;
use std::net::{IpAddr, ToSocketAddrs};

/// A URL this guard will not fetch, and why.
///
/// One opaque string rather than an enum of reasons: every caller reports it and
/// none of them branches on it, and the messages are asserted verbatim by comms'
/// `tests/ingest_allowlist.rs`. Wrap it in the capability's own error type with
/// `to_string()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused(String);

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Refused {}

/// Reject anything that is not plain http(s) before a URL reaches a fetcher.
///
/// `file://` would make a subprocess extractor read the local disk, and this runs
/// behind an HTTP endpoint — the check belongs at the one door every caller goes
/// through, not at each call site.
pub fn check_scheme(url: &str) -> Result<(), Refused> {
    let low = url.trim().to_lowercase();
    if low.starts_with("http://") || low.starts_with("https://") {
        Ok(())
    } else {
        Err(Refused("only http(s) URLs can be fetched".into()))
    }
}

/// Refuse a URL that resolves to an address inside this machine or this network.
///
/// [`check_scheme`] closes `file://`; this closes the other half of the same hole.
///
/// One escape, and it is written down rather than inferred: an origin the caller
/// lists passes even when it resolves inside this machine. It exists for
/// `tools/demo-up`, which stands a synthetic origin up on loopback and seeds Comms
/// by asking it to fetch from there — the one caller that legitimately points ingest
/// at this machine. `allowed_origins` is a closure, not a slice, because comms reads
/// its from a config file and that file must not be read on the ordinary path: the
/// closure runs only after the address check has already failed. A caller with no
/// allowlist passes `Vec::new`.
///
/// Two residuals, both deliberate and neither closed here:
///
/// 1. **DNS rebinding.** The name is resolved once for this check and again by the
///    connector, so a record with a one-second TTL can answer public here and private
///    there. Closing it needs the checked address to be the address the socket gets —
///    a pinned `ClientBuilder::resolve` or a custom connector — which is a larger
///    change than this guard, and one no unit test in this file could observe.
/// 2. **A blocking resolve on a redirect hop.** `to_socket_addrs` is synchronous, and
///    comms' redirect policy calls this from inside reqwest's own runtime thread, so a
///    slow resolver on a hop can push a request past the timeout set beside that
///    policy. One client per fetch bounds the damage to that fetch.
pub fn check_destination(
    url: &str,
    allowed_origins: impl FnOnce() -> Vec<String>,
) -> Result<(), Refused> {
    let parsed = reqwest::Url::parse(url.trim())
        .map_err(|e| Refused(format!("refused: unparsable URL ({e})")))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| Refused("refused: URL names no host".into()))?;
    // `host_str` keeps the brackets on an IPv6 literal; `to_socket_addrs` parses an
    // address literal before it resolves, and it cannot parse the brackets.
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let port = parsed.port_or_known_default().unwrap_or(80);
    let addrs: Vec<_> = (host, port)
        .to_socket_addrs()
        .map_err(|e| Refused(format!("refused: cannot resolve {host} ({e})")))?
        .collect();
    if addrs.is_empty() {
        return Err(Refused(format!("refused: {host} resolves to no address")));
    }
    // ALL, not ANY. A name that answers with one public address and one private
    // address is the ordinary way this check is bypassed.
    if addrs.iter().all(|a| is_public(a.ip())) {
        return Ok(());
    }
    if origin_is_allowed(&parsed, &allowed_origins()) {
        return Ok(());
    }
    Err(Refused(format!(
        "refused: {host} resolves to a non-public address"
    )))
}

/// `http://127.0.0.1:8099/articles/x` -> `http://127.0.0.1:8099`. `None` for anything
/// that is not an absolute http(s) URL naming a host.
///
/// The port is always written out, so `http://example.com` and
/// `http://example.com:80` normalise to one string and cannot be configured apart.
/// One normaliser, because the configured entries and the URL being checked have to
/// be compared as the same shape or the comparison is a coin toss:
/// [`origin_is_allowed`] calls this on the URL, and the caller assembling its
/// allowlist calls it on every entry.
pub fn normalize_origin(raw: &str) -> Option<String> {
    let url = reqwest::Url::parse(raw.trim()).ok()?;
    let scheme = url.scheme();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    // `host_str` keeps the brackets on an IPv6 literal. They stay: both sides of the
    // comparison come through here, and a bracketed host is what the operator writes
    // in the config file too.
    let host = url.host_str()?.to_ascii_lowercase();
    let port = url.port_or_known_default()?;
    Some(format!("{scheme}://{host}:{port}"))
}

/// Whether a URL's own origin — the scheme, host and port as written, never the
/// address it resolved to — is one the operator listed.
///
/// Matching the written host is the point. An attacker who publishes a name that
/// resolves to 127.0.0.1 still does not match `http://127.0.0.1:8099`, so the entry
/// clears exactly the origin it names and nothing that merely lands in the same place.
pub fn origin_is_allowed(url: &reqwest::Url, allowed: &[String]) -> bool {
    match normalize_origin(url.as_str()) {
        Some(origin) => allowed.contains(&origin),
        None => false,
    }
}

/// Whether an address is routable on the public internet. Stricter than "not
/// loopback" on purpose: link-local carries the cloud metadata service at
/// 169.254.169.254, and the CGNAT range carries this machine's VPN peers.
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            // Four ranges written out by hand. Three of them have a std predicate
            // that is still behind feature `ip` on stable (rust-toolchain.toml) --
            // `is_shared`, `is_reserved`, `is_benchmarking` -- and 0.0.0.0/8 has none
            // at all. Every other test below is stable std, so do not hand-roll those.
            let o = v4.octets();
            // RFC 6598 carrier-grade NAT, 100.64.0.0/10: this machine's VPN peers.
            let cgnat = o[0] == 100 && (64..128).contains(&o[1]);
            // RFC 1122 "this network", 0.0.0.0/8. Some stacks route 0.x.y.z to
            // localhost, and no destination on it is legitimate.
            let this_network = o[0] == 0;
            // RFC 1112 reserved, 240.0.0.0/4, and RFC 2544 benchmarking,
            // 198.18.0.0/15. Neither is routable, so neither is a public host.
            let reserved = o[0] >= 240;
            let benchmarking = o[0] == 198 && (18..20).contains(&o[1]);
            !(v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_multicast()
                || v4.is_unspecified()
                || cgnat
                || this_network
                || reserved
                || benchmarking)
        }
        IpAddr::V6(v6) => {
            // The v6 tests run FIRST. `::1` is an IPv4-compatible address as well as
            // the loopback one, and unwrapping it before testing it yields 0.0.0.1 --
            // which the v4 arm would have to special-case to avoid calling loopback
            // public.
            if v6.is_loopback()
                || v6.is_multicast()
                || v6.is_unspecified()
                || v6.is_unique_local()
                || v6.is_unicast_link_local()
            {
                return false;
            }
            // `::ffff:a.b.c.d` (mapped) and the deprecated `::a.b.c.d` (compatible)
            // are both an IPv4 destination wearing a v6 name; `::ffff:127.0.0.1` has
            // to fail for the reason 127.0.0.1 fails, and so does `::7f00:1`.
            match v6.to_ipv4() {
                Some(v4) => is_public(IpAddr::V4(v4)),
                None => true,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_allowlist() -> Vec<String> {
        Vec::new()
    }

    #[test]
    fn check_scheme_rejects_non_http() {
        assert!(check_scheme("https://example.com").is_ok());
        assert!(check_scheme("http://example.com").is_ok());
        assert!(check_scheme("file:///etc/passwd").is_err());
        assert!(check_scheme("ftp://example.com/x").is_err());
        assert!(check_scheme("example.com").is_err());
    }

    /// The scheme test is case- and whitespace-insensitive, because a caller passes
    /// the string a remote feed handed it.
    #[test]
    fn check_scheme_reads_the_url_as_written_not_as_typed() {
        assert!(check_scheme("  HTTPS://example.com  ").is_ok());
        assert!(check_scheme("HtTp://example.com").is_ok());
        assert!(check_scheme("FILE:///etc/passwd").is_err());
    }

    /// Every case is an IP literal, so the test asserts the address policy and never
    /// touches a resolver. comms' redirect policy calls this same function on each
    /// hop, but `reqwest::redirect::Attempt` cannot be constructed outside reqwest,
    /// so that wiring is only exercised by an end-to-end ingest of a public URL that
    /// redirects to a private one.
    #[test]
    fn check_destination_refuses_non_public_addresses() {
        for url in [
            "http://127.0.0.1:8086/api/plans",
            "http://[::1]/x",
            "http://[::ffff:127.0.0.1]/x",
            "http://10.0.0.1/",
            "http://192.168.1.1/",
            "http://172.16.0.1/",
            "http://169.254.169.254/latest/meta-data/",
            "http://[fe80::1]/",
            "http://[fd00::1]/",
            "http://100.64.0.1/",
            "http://0.0.0.0/",
            "http://255.255.255.255/",
            "http://[ff02::1]/",
            // The deprecated IPv4-compatible spelling of 127.0.0.1. Modern stacks do
            // not route it, but the guard must not be the thing that depends on that.
            "http://[::7f00:1]/",
            "http://240.0.0.1/",
            "http://198.18.0.1/",
            "http://0.1.2.3/",
        ] {
            let err = check_destination(url, no_allowlist).expect_err(url);
            assert!(err.to_string().starts_with("refused: "), "{url}: {err}");
        }
    }

    #[test]
    fn check_destination_allows_a_public_address() {
        assert!(check_destination("http://93.184.216.34/index.html", no_allowlist).is_ok());
        assert!(check_destination("https://[2606:2800:220:1::1]/", no_allowlist).is_ok());
    }

    #[test]
    fn check_destination_refuses_a_url_without_a_host() {
        assert!(check_destination("http:///nowhere", no_allowlist).is_err());
    }

    #[test]
    fn check_destination_refuses_what_is_not_a_url_at_all() {
        let err = check_destination("not a url", no_allowlist).expect_err("unparsable");
        assert!(
            err.to_string().starts_with("refused: unparsable URL"),
            "{err}"
        );
    }

    /// A listed origin clears the address check, and it is the only thing that does.
    #[test]
    fn check_destination_lets_a_listed_loopback_origin_through() {
        let listed = || vec!["http://127.0.0.1:8099".to_string()];
        assert!(check_destination("http://127.0.0.1:8099/articles/x", listed).is_ok());
        // The neighbouring port on the same host is a different Axon service.
        assert!(check_destination("http://127.0.0.1:8086/api/plans", listed).is_err());
    }

    /// The allowlist closure is the reason comms does not read its config file on
    /// every fetch. A destination that passes the address check must not run it.
    #[test]
    fn the_allowlist_is_only_consulted_after_the_address_check_fails() {
        let mut consulted = false;
        let watcher = || {
            consulted = true;
            Vec::new()
        };
        assert!(check_destination("http://93.184.216.34/", watcher).is_ok());
        assert!(!consulted, "a public address must not read the allowlist");

        let mut consulted = false;
        let watcher = || {
            consulted = true;
            Vec::new()
        };
        assert!(check_destination("http://127.0.0.1:8086/", watcher).is_err());
        assert!(consulted, "a private address must read the allowlist");
    }

    #[test]
    fn is_public_maps_the_boundaries_of_the_hand_written_ranges() {
        // Every range below is written out because its std predicate is behind
        // feature `ip`. The neighbours on each side prove the mask, not just the
        // middle.
        // 100.64.0.0/10, carrier-grade NAT.
        assert!(!is_public("100.64.0.0".parse().unwrap()));
        assert!(!is_public("100.127.255.255".parse().unwrap()));
        assert!(is_public("100.63.255.255".parse().unwrap()));
        assert!(is_public("100.128.0.0".parse().unwrap()));
        // 0.0.0.0/8, "this network".
        assert!(!is_public("0.255.255.255".parse().unwrap()));
        assert!(is_public("1.0.0.0".parse().unwrap()));
        // 198.18.0.0/15, benchmarking. 198.20.0.0 is an ordinary public host.
        assert!(!is_public("198.18.0.0".parse().unwrap()));
        assert!(!is_public("198.19.255.255".parse().unwrap()));
        assert!(is_public("198.17.255.255".parse().unwrap()));
        assert!(is_public("198.20.0.0".parse().unwrap()));
        // 240.0.0.0/4, reserved. Its lower neighbour is inside multicast
        // (224.0.0.0/4), so the last public IPv4 address is 223.255.255.255.
        assert!(!is_public("240.0.0.0".parse().unwrap()));
        assert!(!is_public("239.255.255.254".parse().unwrap()));
        assert!(is_public("223.255.255.255".parse().unwrap()));
    }

    /// `::1` is IPv4-compatible as well as loopback, so it is the case that decides
    /// the order of the two tests in the v6 arm: unwrapped first it becomes 0.0.0.1,
    /// which no v4 predicate calls loopback.
    #[test]
    fn is_public_unwraps_both_v4_in_v6_forms_without_laundering_v6_loopback() {
        assert!(!is_public("::1".parse().unwrap()));
        assert!(!is_public("::ffff:127.0.0.1".parse().unwrap()));
        assert!(!is_public("::7f00:1".parse().unwrap()));
        assert!(!is_public("::ffff:192.168.1.1".parse().unwrap()));
        assert!(is_public("::ffff:93.184.216.34".parse().unwrap()));
        assert!(is_public("2606:2800:220:1::1".parse().unwrap()));
    }

    /// The allowlist is matched against the URL as written, so it clears the origin
    /// the operator named and nothing else that happens to resolve to the same
    /// machine. `tools/demo-up` is the one caller that sets it.
    #[test]
    fn origin_is_allowed_matches_the_written_origin_only() {
        let allowed = vec!["http://127.0.0.1:8099".to_string()];
        let url = |u: &str| reqwest::Url::parse(u).unwrap();

        assert!(origin_is_allowed(
            &url("http://127.0.0.1:8099/articles/a-slug"),
            &allowed
        ));
        // A different port on the same host is a different Axon service. This is the
        // whole reason the entry is an origin and not a host.
        assert!(!origin_is_allowed(
            &url("http://127.0.0.1:8086/api/plans"),
            &allowed
        ));
        // A name that resolves to loopback is still not the listed origin.
        assert!(!origin_is_allowed(
            &url("http://localhost:8099/articles/a-slug"),
            &allowed
        ));
        assert!(!origin_is_allowed(
            &url("https://127.0.0.1:8099/x"),
            &allowed
        ));
        // Nothing is allowed by default, which is what every non-demo machine runs
        // with.
        assert!(!origin_is_allowed(&url("http://127.0.0.1:8099/x"), &[]));
    }

    #[test]
    fn normalize_origin_writes_the_port_out_and_drops_the_path() {
        assert_eq!(
            normalize_origin("http://127.0.0.1:8099/articles/x").as_deref(),
            Some("http://127.0.0.1:8099")
        );
        // The default port is written out, so the two spellings of one origin are one
        // string on both sides of the comparison.
        assert_eq!(
            normalize_origin("http://example.com").as_deref(),
            Some("http://example.com:80")
        );
        assert_eq!(
            normalize_origin("https://Example.COM/").as_deref(),
            Some("https://example.com:443")
        );
        assert_eq!(
            normalize_origin("http://[::1]:9000/").as_deref(),
            Some("http://[::1]:9000")
        );
    }

    #[test]
    fn normalize_origin_refuses_what_is_not_an_http_origin() {
        // A scheme that is not http(s) must not be configurable as an escape from a
        // guard whose other half exists to refuse `file://`.
        assert_eq!(normalize_origin("file:///etc/passwd"), None);
        assert_eq!(normalize_origin("ftp://example.com/"), None);
        // Relative, and host-less: neither names an origin.
        assert_eq!(normalize_origin("127.0.0.1:8099"), None);
        assert_eq!(normalize_origin(""), None);
    }
}
