//! Finance: the decision layer above the ledger.
//!
//! hledger owns the ledger and Postgres owns the index over it. This capability
//! owns everything above both: what a subscription costs over its life, what a trip
//! actually cost, whether a position crossed its exit rule, and the analytics that
//! need all three. The split is not a compromise. A plaintext journal under git is
//! the vault boundary's V1 and an index rebuilt from it is V2, so choosing the
//! storage format did the boundary work rather than a rule someone has to remember.
//!
//! Phase 2, which is what exists today, is subscriptions and needs no ledger at all
//! — the vault notes already carry the frontmatter. That is deliberate: it proves
//! the whole loop, from vault read through Postgres and REST to writeback, before
//! the first bank export is parsed.
//!
//! Spec, phases and the decision record: the principal's `Projects/Ledger` note.

pub mod config;
pub mod obsidian;
pub mod store;
pub mod subscription;

pub use config::Config;
pub use obsidian::{scan, seed_from_note, ScannedNote, WriteBack};
pub use store::FinanceStore;
pub use subscription::{burn_at, BillingCycle, Burn, PricePoint, State, StateChange, Subscription};
