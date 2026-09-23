//! The traveller profile: who is travelling, as data a solver reads.
//!
//! Read `README.md` here for what this owns and what it deliberately does not.
//! `model.rs` carries the shape and the rules; `store.rs` carries the table and
//! the conditional write; `server.rs` is the binary and its HTTP surface.

pub mod config;
pub mod derive;
pub mod model;
pub mod store;

pub use derive::{derive, DerivedTravel};
pub use model::{
    Anchor, HardConstraints, Pace, ProfileError, ProfileInput, Provenance, SoftWeights,
    TravelProfile, BASIS_KEYS, DEFAULT_PROFILE_ID,
};
pub use store::{PutOutcome, TravelerStore};
