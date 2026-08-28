//! Finance: the decision layer above the ledger.
//!
//! A private plaintext journal is canonical and Postgres is its rebuildable index.
//! The replaceable [`AccountingEngine`](accounting::AccountingEngine) boundary keeps
//! double-entry semantics outside Axon while this capability owns reviewed import,
//! analytics, budgets, subscriptions and the local UI contract.
//!
//! Spec, phases and the decision record: the principal's `Projects/Ledger` note.

pub mod accounting;
pub mod allocation;
pub mod analytics;
pub mod balance;
pub mod config;
pub mod import;
pub mod investment;
pub mod journal;
pub mod obsidian;
pub mod planning;
pub mod store;
pub mod subscription;

pub use accounting::{AccountingEngine, JournalEngine};
pub use config::Config;
pub use obsidian::{scan, seed_from_note, ScanError, ScannedNote, WriteBack};
pub use store::FinanceStore;
pub use subscription::{
    burn_at, burn_by_currency, BillingCycle, Burn, BurnByCurrency, CurrencyBurn, PricePoint, State,
    StateChange, Subscription,
};
