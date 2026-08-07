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

//! This is a normal workspace crate. Cargo consumers declare a path dependency,
//! and Bazel consumers depend on `//libs/axon-server:axon-server`; both build
//! graphs therefore enforce the same boundary and resolve the same `axum` API.

use std::net::SocketAddr;

// Re-exported so a server binary that depends only on axon-server still gets the
// port contract.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bind_addr_is_loopback_only() {
        let addr = bind_addr(8084);
        assert!(addr.ip().is_loopback());
        assert_eq!(addr.port(), 8084);
    }
}
