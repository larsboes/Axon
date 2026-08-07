# axon-config

Shared overlay/config resolution for Axon's Rust capabilities: tilde expansion, overlay
paths (`AXON_PERSONAL_ROOT`), the shared-Postgres connection string, the runner's port
contract (`AXON_PORT` first, capability escape hatch second, config file third, shipped
default last), and DSN redaction.

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
declare the workspace path dependency in Cargo and
`//libs/axon-config:axon-config` in Bazel. The library is compiled once per
build graph, so its API is an ordinary crate boundary rather than copied source.
