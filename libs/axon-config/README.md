# axon-config

Shared overlay/config resolution for Axon's Rust capabilities: tilde expansion, overlay
paths (`AXON_PERSONAL_ROOT`), the store's location, the runner's port contract (`AXON_PORT`
first, capability escape hatch second, config file third, shipped default last), and DSN
redaction.

## Where the store lives

`database_path()` — `AXON_DB_PATH`, else `<overlay>/data/axon/axon.db`, else a scratch file
under the temp directory. One file for every capability after PRD Q45 (2026-08-27), so it
takes no capability argument: cross-capability joins are why the shared instance existed,
and a file per capability would have dropped them. The last resort is deliberately obvious
scratch, because the Postgres fallback it replaces named the real database and the demo
overlay resolved straight to it.

`postgres_conn_from_shared_env()` and `redact_dsn()` stay while comms, finance, scouting,
places, punctuality and calendar are still on Postgres. They go with the last consumer.

## Why this exists

transit/config.rs once argued the duplication was cheaper than a shared crate, and at two
copies it was. By five capabilities the repo held six copies of `expand_tilde` and five
shared-Postgres DSN builders in two diverging forms; three crates still built the
`postgresql://user:password@…` URL form that comms had already documented as an auth trap
(the instance's real password is base64 and can contain `/`, `+`, `=`, which URL userinfo
silently mangles). One implementation ends both the drift and the divergence: the
keyword/value form everywhere, and the one redaction that survives an `@` inside a
password (`rfind`, ported from punctuality's copy, the best of the five).

Zero external dependencies on purpose: std-only, so consuming it never changes a
capability's dependency resolution, and it needs no crate universe of its own.

## Consumers

Every Rust capability uses this directly or through the server helper. Consumers
declare the workspace path dependency in Cargo. The library is compiled once per
build, so its API is an ordinary crate boundary rather than copied source.
