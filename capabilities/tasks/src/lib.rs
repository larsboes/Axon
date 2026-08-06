//! Axon doctrine: this crate is public. No task text, sender, or personal
//! value lives here — those are rows in the operator's own Postgres, written
//! through the API at runtime.
//!
//! Tasks owns one thing: the record that says something needs doing, and where
//! it came from. It observes nothing itself. Comms promotes a mail into a task,
//! Calendar keeps the dated commitments, and neither writes the other's store.

pub mod config;
pub mod store;

// Shared config helpers, compiled in by #[path] include rather than a cargo
// dependency: rules_rust's splicer flattens listed manifests and breaks
// ../../libs path deps, and folding a small shared shape in as a module is
// this repo's stated preference anyway (libs/axon-config/README.md).
#[path = "../../../libs/axon-config/src/lib.rs"]
#[allow(dead_code)]
pub(crate) mod axon_config;

// The `content-item-v1` reader contract. Tasks uses only its data-class
// vocabulary — a task inherits the class of whatever it was promoted from, and
// that vocabulary has exactly one definition in this repo.
#[path = "../../../libs/content-item/src/lib.rs"]
#[allow(dead_code)]
pub mod content_item;

// When the schema migration runs, and the advisory lock around it. Seven
// capabilities own a Postgres schema and all seven used to migrate on every
// `Store::open`; libs/axon-store/README.md has the deadlock that caused.
#[path = "../../../libs/axon-store/src/lib.rs"]
#[allow(dead_code)]
pub(crate) mod axon_store;
