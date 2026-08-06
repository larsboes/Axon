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

// Shared config helpers, compiled in by #[path] include rather than a cargo
// dependency: rules_rust's splicer flattens listed manifests and breaks
// ../../libs path deps, and folding a small shared shape in as a module is
// this repo's stated preference anyway (libs/axon-config/README.md).
#[path = "../../../libs/axon-config/src/lib.rs"]
#[allow(dead_code)]
pub(crate) mod axon_config;

// The `content-item-v1` reader contract, on the same include terms. Compiled
// separately into each consumer, so this `ContentItem` is a different Rust type
// from comms' — the boundary between them is the serialized JSON, never a call.
#[path = "../../../libs/content-item/src/lib.rs"]
#[allow(dead_code)]
pub mod content_item;

// Bounded reads out of a declared markdown root, on the same include terms.
// The markdown importer's whole containment story lives there rather than
// here; scouting folds in the same file. libs/markdown-root/README.md.
#[path = "../../../libs/markdown-root/src/lib.rs"]
#[allow(dead_code)]
pub(crate) mod markdown_root;

// When the schema migration runs, and the advisory lock around it. Seven
// capabilities own a Postgres schema and all seven used to migrate on every
// `Store::open`; libs/axon-store/README.md has the deadlock that caused.
#[path = "../../../libs/axon-store/src/lib.rs"]
#[allow(dead_code)]
pub(crate) mod axon_store;
