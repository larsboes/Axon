pub mod config;
pub mod correlate;
pub mod date;
pub mod google;
pub mod google_sync;
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
