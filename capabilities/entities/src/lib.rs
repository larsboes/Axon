//! The entity core: who and what Axon knows about, as typed data (PRD Q117).
//!
//! `model.rs` carries the kinds, the field registry and the rules. `store.rs` carries the
//! tables. `places.rs` asks `capabilities/places` for a coordinate. `server.rs` is the
//! binary and its HTTP surface.

pub mod config;
pub mod model;
pub mod places;
pub mod store;
