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
    /// The time to plan around: real-time when HAFAS has one, scheduled
    /// otherwise. Kept as the primary field because every existing consumer
    /// reads it and "when does this actually leave" is the usual question.
    pub departure_time: String, // ISO 8601 datetime
    pub arrival_time: String, // ISO 8601 datetime
    /// Scheduled and real-time, kept apart.
    ///
    /// HAFAS serves `sollzeit` (scheduled) and `istzeit` (real-time) separately
    /// and this collapsed them with an `or_else`, so a train running twenty
    /// minutes late was indistinguishable from one running on time: the delay
    /// existed in the response and was discarded on the way in. `None` means
    /// HAFAS gave no real-time value, which is different from "no delay".
    #[serde(default)]
    pub scheduled_departure: Option<String>,
    #[serde(default)]
    pub realtime_departure: Option<String>,
    #[serde(default)]
    pub scheduled_arrival: Option<String>,
    #[serde(default)]
    pub realtime_arrival: Option<String>,
    /// `departure_time`/`arrival_time` as unambiguous UTC instants ("...Z").
    ///
    /// The naive fields above are each stop's OWN local wall-clock (verified
    /// live 2026-08-12: a London arrival comes back in BST), so subtracting
    /// them across a zone boundary is wrong by the zone delta. Arithmetic uses
    /// these; the naive fields stay untouched because the dashboard renders
    /// them station-local as-is. `None` when the station's UIC prefix is not
    /// in station-time's table -- absent, never guessed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub departure_utc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arrival_utc: Option<String>,
    /// Whether HAFAS marked this leg cancelled. A cancelled train was previously
    /// invisible: it came back as an ordinary leg with times on it.
    #[serde(default)]
    pub cancelled: bool,
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
    /// `arrival_punctuality.share_late_6`, flattened, and nothing more.
    ///
    /// It began as an ML prediction slot that was never ported, then
    /// `punctuality::enrich` started filling it with a *measured* historical
    /// share, which is a different quantity under the old name. Kept because
    /// consumers read it; read `arrival_punctuality` instead, which says how
    /// many observations the number rests on.
    pub delay_risk_score: Option<f64>,
    /// The delay history behind `delay_risk_score`, unflattened.
    ///
    /// Absent when punctuality has no cell for this train type at this station
    /// in this hour, when the cell is thinner than punctuality's own sample
    /// floor, or when punctuality is not running. Those are three different
    /// states and only the last is a fault, but none of them is a low risk --
    /// a consumer that renders absence as "punctual" is inventing a
    /// measurement. Carrying `n` is what lets one be told from another.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arrival_punctuality: Option<ArrivalPunctuality>,
}

/// One punctuality cell: the arriving train's type, at the journey's destination,
/// in the arrival hour, split weekday/weekend.
///
/// Emphatically not a forecast for this journey and not transfer risk -- the
/// module doc on `punctuality::enrich` says why this data cannot produce either.
/// It is what happened to comparable trains at that stop, and `n` says how many.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArrivalPunctuality {
    pub station_name: Option<String>,
    pub train_type: String,
    pub hour: i16,
    pub weekend: bool,
    /// Observations behind every other field here.
    ///
    /// The difference between a statistic and a coincidence, and the only reason
    /// a consumer can show its own confidence rather than a bare float. Never
    /// below punctuality's sample floor, because thinner cells come back as no
    /// cell at all.
    pub n: i64,
    pub mean_delay: f32,
    pub p50: i16,
    pub p90: i16,
    pub share_late_6: f32,
    pub cancel_rate: f32,
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

/// One ticket boundary inside a split chain: the station where one
/// Beförderungsvertrag ends and the next begins.
///
/// Separate tickets mean separate contracts (BB Nr. 1.3.4): a delayed first
/// leg does not release the next ticket's Zugbindung, so every boundary where
/// the traveller changes trains carries missed-connection risk the through
/// ticket would not have. This struct carries the FACTS of that risk and
/// deliberately no probability: punctuality's arrival-delay share is not a
/// transfer-risk model (see punctuality.rs), and pretending otherwise would
/// dress a guess as a measurement. Calibration is L2, gated on real outcome
/// records.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContractBoundary {
    pub station: Station,
    /// True when both tickets ride the same train across the boundary -- a
    /// mid-run ticket split, where no connection can be missed.
    pub same_train: bool,
    /// Scheduled transfer buffer, from the UTC instants when both sides carry
    /// them. `None` when either side's zone is unknown.
    pub transfer_minutes: Option<i64>,
    /// Share of the incoming train type's stops at this station in this hour
    /// that ran >= 6 minutes late, from delay history. Context for the buffer,
    /// not a verdict. `None` when punctuality has no cell.
    #[serde(default)]
    pub incoming_share_late_6: Option<f64>,
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
    /// The ticket boundaries between consecutive segments, in chain order.
    /// Empty on results stored before this field existed.
    #[serde(default)]
    pub contract_boundaries: Vec<ContractBoundary>,
    pub confidence: SplitConfidence,
    /// Stop pairs the solver asked HAFAS to price and got nothing back for. The
    /// chosen chain is fully priced by construction, so this does not invalidate
    /// it; it says the search ran against an incomplete table and a cheaper split
    /// may exist that was never visible.
    pub unpriced_pairs: usize,
    pub queried_pairs: usize,
}
