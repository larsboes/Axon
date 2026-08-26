# axon-server

The one way a capability server comes up: `resolve_port` (re-exported from
`axon-config`: `AXON_PORT` from the runner first, capability escape hatch second, config
third, shipped default last), a loopback-only bind, **the inbound authentication gate**,
uniform startup logging, and a named single-line exit on bind failure instead of a panic
backtrace.

## The inbound gate

| Configured token | `/health`, `/ready`, CORS preflight | Every other route | Reach beyond loopback |
|---|---|---|---|
| yes | served | `401` without a matching token | permitted |
| no | served | served (or `403`, see below) | **refused at bind** |

A token is presented as `Authorization: Bearer <token>` or `X-Axon-Token: <token>`, and
compared byte-by-byte in constant time. Two header forms because two kinds of client
call these ports: proxies and HTTP tooling that already speak `Authorization`, and the
browser extension and `curl` callers for which a dedicated header is one fewer thing to
get wrong.

`/health` and `/ready` answer before the gate. They are what the runner, the dashboard
proxy and axon-status poll to find out whether a process is alive; behind a token they
would report a healthy capability as down, and their answer carries nothing a caller
could not learn by observing that the port accepts a connection. `/routes` is **not**
exempt: a route manifest describes the surface, which is not liveness.

`InboundAuth::refuse_without_token()` closes the non-exempt routes with `403` instead of
serving them when no token is configured. comms is the reason it exists: `POST /ingest`
fetches an attacker-chosen URL, and a page open in the operator's own browser is already
inside the loopback boundary, so `127.0.0.1` was never what contained that route.

### Token sourcing: one token for the deployment

`<overlay>/config/deployment.env` declares `AXON_INBOUND_TOKEN_FILE=<path>` and the token
is that private file's contents (`schemas/deployment.env.example`). A reference, not a
value, following the pattern comms established for `api_secret_file` — a path is not a
secret, which is why it may live in a tracked-shape file.

Shared rather than per-capability because it gates one thing: whether an inbound request
reached this machine legitimately. Twelve tokens would be twelve secrets for one boundary
and twelve injections in every client that fans out across capabilities — the dashboard's
Vite proxy and axon-status' `/routes` aggregation both do exactly that.

A capability may still pass its own token to `InboundAuth::resolve`, and it wins. comms'
`api_secret_file` is the one caller that does, because the browser extension, `axon-clip`
and the dashboard proxy already hold that value. A deployment converges the two by
pointing both references at one file.

### Why the loopback rule is now a type

`bind_addr_for(Reach::AllInterfaces, port, auth)` returns `Err` when `auth` carries no
token. It is the only constructor of a non-loopback `SocketAddr` in this crate, so
"served beyond this machine without authentication" has no value a caller can obtain
and then use. That is the half this crate can enforce; the other half is below.

## Why this exists

Server binaries carried the same ~10 startup lines with three divergences none of which
was a decision: three bound `0.0.0.0` while the others argued `127.0.0.1` in a comment,
punctuality exited cleanly on a bind failure while the rest panicked, and comms had
stopped honouring the runner's port contract.

The last of those, `scout-server`, mattered more than the tidiness: it bound `0.0.0.0`
with permissive CORS in front of a mutating `POST /opportunities/:id/status`, so any
device on the LAN could write opportunity state without auth.

The gate arrived for the same reason one level up. Exactly one of twelve Rust
capabilities authenticated an inbound request — comms, on its mutating routes only. The
other eleven treated the loopback bind as the whole boundary, axon-status among them,
which serves `POST /api/axon-status/capabilities/:name/start|stop`: process control.
"Reachable from the phone" and "unauthenticated process control" cannot both be true, so
the check belongs in the crate all twelve already route their startup through rather
than in twelve copies that drift.

CORS is deliberately not in here. Whether a server carries `CorsLayer::permissive()` is
a per-capability security decision that stays visible in that capability's source —
axon-status, which can start and stop the machine's capabilities, correctly carries
none.

## What actually enforces this

`serve_local` alone enforces nothing: a server that ignores it and builds its own
listener compiles fine, and so does one that skips the gate by never calling this crate.
The check that makes both policies real is doctor's **Server bind policy** section, which
fails when any `capabilities/*/src/*.rs` that builds a `Router`
also constructs its own `axum::serve` or `TcpListener::bind`. It lives in doctor rather
than a repo gate because half the servers it has to cover are in the overlay, outside this
repo, and a gate that globs Axon alone would report a clean policy while an overlay server
binds the LAN (README.md#documentation-stays-owned-and-current, same reasoning as the decision path-rot sweep).

## Build boundary

This is a normal workspace crate. Consumers declare an `axon-server` path dependency,
and the one root `Cargo.lock` keeps the `axum::Router` type identical across the library
and every consumer. `cargo tree` is what exposes the architectural edge.

## Consumers

Every capability server: `scout-server`, `comms-server`, `transit-server`,
`trips-server`, `punctuality-server`, `calendar-server`, `axon-status`.
