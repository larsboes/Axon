pub mod config;
pub mod obsidian;
pub mod store;

// Shared config helpers, compiled in by #[path] include rather than a cargo
// dependency: rules_rust's splicer flattens listed manifests and breaks
// ../../libs path deps, and folding a small shared shape in as a module is
// this repo's stated preference anyway (libs/axon-config/README.md).
#[path = "../../../libs/axon-config/src/lib.rs"]
#[allow(dead_code)]
pub(crate) mod axon_config;
