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

/// Whether a priced segment is priced for the train the traveller will be on.
///
/// The solver prices each stop pair with a *fresh* connection search and takes
/// the first journey that comes back. Nothing forced that journey to be the one
/// the traveller sits on, so a four-ticket chain could contain a fare for a train
/// leaving two hours later. Buying three of four tickets in a chain like that
/// leaves you with a broken itinerary and no refund, which is why this is
/// reported per segment rather than averaged into one number.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TrainMatch {
    /// The priced journey rides exactly the trains the direct journey rides
    /// over this stop pair, in the same order.
    Exact,
    /// It shares at least one train with the direct journey but not all of them.
    Partial,
    /// It shares no train with the direct journey: this fare is for a different
    /// service, and buying it does not buy a seat on the planned trip.
    Different,
    /// Not determinable, because one side carried no train number at all.
    Unknown,
}

/// One purchased leg of a split-ticket chain.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SplitSegment {
    pub journey: Journey,
    pub train_match: TrainMatch,
    /// The train numbers the direct journey uses over this stop pair. Empty when
    /// the direct journey carried none, which is also what makes `train_match`
    /// `Unknown` rather than a verdict nobody can check.
    pub expected_trains: Vec<String>,
}

/// How much of the chain is trustworthy, as one value a caller can gate on.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SplitConfidence {
    /// Every segment priced for exactly the planned trains, and every stop pair
    /// the solver wanted a fare for returned one.
    Exact,
    /// Some segment is `Partial` or `Unknown`, or some pairwise query failed so
    /// the search ran against an incomplete price table.
    Partial,
    /// At least one segment is priced for a train the traveller will not be on.
    Low,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SplitResult {
    pub original_price: Option<f64>,
    pub split_price: f64,
    /// `None` when the direct fare is unknown. It used to be `0.0` in that case,
    /// which a reader cannot tell apart from "the split saves nothing" -- and the
    /// dashboard rendered exactly that as "Direct is cheapest".
    pub savings: Option<f64>,
    pub segments: Vec<SplitSegment>,
    pub confidence: SplitConfidence,
    /// Stop pairs the solver asked HAFAS to price and got nothing back for. The
    /// chosen chain is fully priced by construction, so this does not invalidate
    /// it; it says the search ran against an incomplete table and a cheaper split
    /// may exist that was never visible.
    pub unpriced_pairs: usize,
    pub queried_pairs: usize,
}
