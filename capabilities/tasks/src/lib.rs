//! Axon doctrine: this crate is public. No task text, sender, or personal
//! value lives here — those are rows in the operator's own Postgres, written
//! through the API at runtime.
//!
//! Tasks owns one thing: the record that says something needs doing, and where
//! it came from. It observes nothing itself. Comms promotes a mail into a task,
//! Calendar keeps the dated commitments, and neither writes the other's store.

pub mod config;
pub mod store;

pub use content_item;

// When the schema migration runs, and the advisory lock around it. Seven
// capabilities own a Postgres schema and all seven used to migrate on every
// `Store::open`; libs/axon-store/README.md has the deadlock that caused.
#[path = "../../../libs/axon-store/src/lib.rs"]
#[allow(dead_code)]
pub(crate) mod axon_store;
