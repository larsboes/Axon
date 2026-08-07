pub mod adapters;
pub mod calendar_promote;
pub mod config;
pub mod embed;
pub mod event_route;
pub mod http;
pub mod localtime;
pub mod merge;
pub mod normalize;
pub mod opportunity;
pub mod pipeline;
pub mod score;
pub mod source;
pub mod sources;
pub mod store;
pub mod vault_linker;

// Which model answers which job on this machine. This runtime-heavy source
// include is migrated separately in Axon#111.
#[path = "../../../libs/inference/src/lib.rs"]
#[allow(dead_code)]
pub(crate) mod inference;

// When the schema migration runs, and the advisory lock around it. Seven
// capabilities own a Postgres schema and all seven used to migrate on every
// `Store::open`; libs/axon-store/README.md has the deadlock that caused.
#[path = "../../../libs/axon-store/src/lib.rs"]
#[allow(dead_code)]
pub(crate) mod axon_store;
