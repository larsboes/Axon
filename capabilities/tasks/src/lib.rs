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
