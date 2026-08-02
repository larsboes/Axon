//! Shared travel types (Station/Leg/Journey/SplitResult). Ported from
//! LifeOS-mono's `schemas/travel` crate and folded in as a module here --
//! same call as `scouting`'s `opportunity.rs`: it's ~40 lines used by one
//! consumer, not worth a second crate (Axon README.md#documentation-stays-owned-and-current).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Station {
    pub id: String,   // EVA code, e.g. "8000105"
    pub name: String, // e.g. "Frankfurt(Main)Hbf"
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Leg {
    pub origin: Station,
    pub destination: Station,
    pub departure_time: String, // ISO 8601 datetime
    pub arrival_time: String,   // ISO 8601 datetime
    pub train_name: String,     // e.g. "ICE 690"
    pub train_number: String,   // e.g. "690"
    pub train_category: String, // e.g. "ICE", "RE", "RB", "S"
    pub platform: Option<String>,
    pub is_regional: bool, // true if covered by Deutschland-Ticket (HAFAS attribute "9G")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Journey {
    pub id: String,
    pub start_station: Station,
    pub end_station: Station,
    pub legs: Vec<Leg>,
    pub total_duration_minutes: u32,
    pub total_price: Option<f64>,
    /// ML delay-risk prediction slot (0.0-1.0). Always `None` in this port --
    /// see README's "Known gaps" section: the ONNX model + tract-onnx
    /// machinery that used to populate this was deliberately NOT ported
    /// (no model artifact exists in Axon yet, would be dead-weight code that
    /// can never predict anything real). The field stays so wiring it back in
    /// later, once a real model exists, is additive, not a schema change.
    pub delay_risk_score: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SplitResult {
    pub original_price: Option<f64>,
    pub split_price: f64,
    pub savings: f64,
    pub segments: Vec<Journey>,
}
