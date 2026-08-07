# axon-server

The one way a capability server comes up: `resolve_port` (re-exported from
`axon-config`: `AXON_PORT` from the runner first, capability escape hatch second, config
third, shipped default last), a loopback-only bind, uniform startup logging, and a named
single-line exit on bind failure instead of a panic backtrace.

## Why this exists

Server binaries carried the same ~10 startup lines with three divergences none of which
was a decision: three bound `0.0.0.0` while the others argued `127.0.0.1` in a comment,
punctuality exited cleanly on a bind failure while the rest panicked, and comms had
stopped honouring the runner's port contract.

The last of those, `scout-server`, mattered more than the tidiness: it bound `0.0.0.0`
with permissive CORS in front of a mutating `POST /opportunities/:id/status`, so any
device on the LAN could write opportunity state without auth.

CORS is deliberately not in here. Whether a server carries `CorsLayer::permissive()` is
a per-capability security decision that stays visible in that capability's source —
axon-status, which can start and stop the machine's capabilities, correctly carries
none.

## What actually enforces this

`serve_local` alone enforces nothing: a server that ignores it and builds its own
listener compiles fine. The check that makes the policy real is doctor's **Server bind
policy** section, which fails when any `capabilities/*/src/*.rs` that builds a `Router`
also constructs its own `axum::serve` or `TcpListener::bind`. It lives in doctor rather
than Bazel because every Rust capability is its own Bazel package, so a root-level glob
cannot reach these sources and a hand-maintained label list is the thing that rots
(README.md#documentation-stays-owned-and-current, same reasoning as the decision path-rot sweep).

## Build boundary

This is a normal workspace crate in both build graphs. Cargo consumers declare an
`axon-server` path dependency and Bazel consumers depend on
`//libs/axon-server:axon-server`. The shared crate universe keeps the `axum::Router`
type identical across the library and its consumers, while dependency queries expose
the actual architectural edge.

## Consumers

Every capability server: `scout-server`, `comms-server`, `transit-server`,
`trips-server`, `punctuality-server`, `calendar-server`, `axon-status`.
