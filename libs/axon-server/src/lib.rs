//! The one way a capability server comes up. Extracted after the same ~10
//! lines (resolve port, build a SocketAddr, log, bind, serve) existed in five
//! server binaries with three divergences none of which was a decision:
//! two servers bound 0.0.0.0 while three argued 127.0.0.1 in a comment, one
//! exited cleanly on a bind failure while four panicked, and one had just
//! stopped honouring the runner's port contract.
//!
//! What is deliberately NOT here: CORS. Whether a server carries
//! `CorsLayer::permissive()` is a per-capability security decision that must
//! stay visible in that capability's own source — axon-status, which can
//! start and stop the machine's capabilities, correctly carries none, and a
//! helper that silently added it would have widened that surface.

//! Consumed by `#[path]` include, not as a cargo dependency: rules_rust's
//! splicer flattens listed manifests into sibling dirs, which breaks any
//! `../../libs/...` path dependency — and the repo's own doctrine (transit's
//! old config comment) prefers folding small shared shapes in as a module
//! anyway. Each consumer adds
//! `#[path = "../../../libs/axon-server/src/lib.rs"] mod axon_server;`
//! to its binary root and gets this compiled against its own crate universe's
//! axum — which is also what makes the Router type compatible per consumer.

use std::net::SocketAddr;

// Re-exported so a server binary that includes only axon_server still gets the
// port contract; allow(unused_imports) because not every consumer calls it.
#[allow(unused_imports)]
pub use axon_config::resolve_port;

/// Loopback-only address for a capability server. 127.0.0.1 is the policy,
/// not a default: these are local services with no auth, reached through the
/// dashboard's proxy on the same machine. A capability that genuinely needs
/// LAN exposure builds its own listener next to a comment saying why —
/// none does today.
pub fn bind_addr(port: u16) -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], port))
}

/// Binds, logs, serves, never returns on success. On failure it exits with a
/// named, single-line error instead of a panic backtrace — the runner captures
/// stderr, and "cannot bind" with the address is the whole diagnosis.
pub async fn serve_local(name: &str, port: u16, router: axum::Router) {
    let addr = bind_addr(port);
    println!("{name} starting on {addr}");
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("{name}: cannot bind {addr}: {e}");
            std::process::exit(1);
        }
    };
    if let Err(e) = axum::serve(listener, router).await {
        eprintln!("{name}: {e}");
        std::process::exit(1);
    }
}

// Feature-gated like axon-config's tests: standalone only, never compiled into
// a consumer's test binary via the #[path] include.
#[cfg(all(test, feature = "standalone-tests"))]
mod tests {
    use super::*;

    #[test]
    fn bind_addr_is_loopback_only() {
        let addr = bind_addr(8084);
        assert!(addr.ip().is_loopback());
        assert_eq!(addr.port(), 8084);
    }
}
