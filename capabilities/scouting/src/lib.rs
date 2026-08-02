pub mod adapters;
pub mod calendar_promote;
pub mod config;
pub mod embed;
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

// Shared config helpers, compiled in by #[path] include rather than a cargo
// dependency: rules_rust's splicer flattens listed manifests and breaks
// ../../libs path deps, and folding a small shared shape in as a module is
// this repo's stated preference anyway (libs/axon-config/README.md).
#[path = "../../../libs/axon-config/src/lib.rs"]
#[allow(dead_code)]
pub(crate) mod axon_config;

// Which model answers which job on this machine. Same #[path] reasoning as
// axon_config above; libs/inference/README.md has the shape.
#[path = "../../../libs/inference/src/lib.rs"]
#[allow(dead_code)]
pub(crate) mod inference;
