pub mod config;
pub mod content;
pub mod correlate;
pub mod date;
pub mod google;
pub mod google_sync;
pub mod markdown_import;
pub mod model;
pub mod rhythm;
pub mod store;
pub mod zone;

pub use content_item;

// When the schema migration runs, and the advisory lock around it. Seven
// capabilities own a Postgres schema and all seven used to migrate on every
// `Store::open`; libs/axon-store/README.md has the deadlock that caused.
#[path = "../../../libs/axon-store/src/lib.rs"]
#[allow(dead_code)]
pub(crate) mod axon_store;
