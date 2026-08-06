//! Empirical rail punctuality.
//!
//! What this capability owns: how reliable a train actually is, measured rather than
//! predicted. It folds Deutsche Bahn's own published stop history (`upstreams.toml`
//! [deutsche-bahn-data]) into per-station statistics and serves them to whoever asks —
//! today `punctuality stats`, next `capabilities/transit`, which carries a
//! `delay_risk_score` field that has been `None` since the port.
//!
//! It is deliberately the bottom rung of the intelligence ladder (README.md#implementation-languages-and-intelligence). A
//! lookup table over two years of observations is not a placeholder for a model; it is
//! the number a model has to beat, and without it "our prediction is good" is not a
//! claim anyone can check.

pub mod config;
pub mod dataset;
pub mod ingest;
pub mod stats;
pub mod store;

// Shared config helpers, compiled in by #[path] include rather than a cargo
// dependency: rules_rust's splicer flattens listed manifests and breaks
// ../../libs path deps, and folding a small shared shape in as a module is
// this repo's stated preference anyway (libs/axon-config/README.md).
#[path = "../../../libs/axon-config/src/lib.rs"]
#[allow(dead_code)]
pub(crate) mod axon_config;

// When the schema migration runs, and the advisory lock around it. Seven
// capabilities own a Postgres schema and all seven used to migrate on every
// `Store::open`; libs/axon-store/README.md has the deadlock that caused.
#[path = "../../../libs/axon-store/src/lib.rs"]
#[allow(dead_code)]
pub(crate) mod axon_store;
