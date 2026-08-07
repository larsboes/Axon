pub mod config;
pub mod extractor;
pub mod hafas;
pub mod punctuality;
pub mod store;
pub mod travel;

// When the schema migration runs, and the advisory lock around it. Seven
// capabilities own a Postgres schema and all seven used to migrate on every
// `Store::open`; libs/axon-store/README.md has the deadlock that caused.
#[path = "../../../libs/axon-store/src/lib.rs"]
#[allow(dead_code)]
pub(crate) mod axon_store;
