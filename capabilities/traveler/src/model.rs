//! The traveller profile: who is travelling, as data a solver reads.
//!
//! Axon's travel stack could search and could rank, and the ranking weights were
//! the same four constants for every person and every trip. The interests a plan
//! carried were echoed back to the page and read by nothing. This module is the
//! missing half: a stated profile with hard limits a search must obey, weights
//! that are the traveller's rather than the repo's, and a `basis` map that says
//! for every single field whether the operator typed it, something derived it, or
//! it is still the built-in default.
//!
//! The `basis` map is not decoration. `soft.budget_fit` set to `0.30` because a
//! person decided that, and `0.30` because nobody has looked yet, produce
//! identical ranking behaviour and completely different confidence. A profile
//! that cannot tell those apart is a profile that will be trusted further than
//! it deserves, which is the failure `capabilities/punctuality` refuses by
//! returning `None` rather than a zero.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The profile every consumer reads unless it names another.
pub const DEFAULT_PROFILE_ID: &str = "default";

/// How far the five weights may miss 1.0 before a write is refused.
///
/// A tolerance rather than an equality because `0.30 + 0.25 + 0.15 + 0.20 + 0.10`
/// is not 1.0 in binary floating point, and a gate that rejected the arithmetic
/// a person would write by hand would be rejecting the wrong thing.
pub const WEIGHT_SUM_TOLERANCE: f64 = 1e-6;

/// Every field `basis` may name, in the order the profile declares them.
///
/// One list, so a test can assert the map covers it exactly. A `basis` that
/// silently omits a field is how "I set this" and "nobody has looked" start
/// reading the same.
pub const BASIS_KEYS: &[&str] = &[
    "hard.earliest_departure",
    "hard.latest_arrival",
    "hard.max_changes",
    "hard.min_transfer_buffer_min",
    "hard.modes",
    "hard.avoid_overnight_travel",
    "hard.home_station",
    "hard.home_airport",
    "hard.cards",
    "soft.budget_fit",
    "soft.feasibility",
    "soft.season",
    "soft.events",
    "soft.retrospective",
    "interests",
    "pace",
    "anchors",
];

/// Where one profile value came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    /// The built-in value. Nobody has established this yet, and it is the only
    /// provenance a reader should treat as provisional.
    Default,
    /// The operator typed it. Nothing derived may overwrite it.
    Stated,
    /// Computed from stored rows — plans, offered options, retrospectives.
    Derived,
    /// Proposed from the vault's TELOS notes and not yet confirmed.
    Vault,
}

/// The limits a search must obey, not trade off.
///
/// Every field is optional and every absent field means "no limit", so a profile
/// with nothing stated constrains nothing and the search behaves exactly as it
/// did before this capability existed. That is the same rule
/// `capabilities/punctuality` states for an unscored leg: absence degrades, it
/// never fails.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct HardConstraints {
    /// `HH:MM`, local to the boarding station. No departure before this.
    pub earliest_departure: Option<String>,
    /// `HH:MM`, local to the arriving station. No arrival after this.
    pub latest_arrival: Option<String>,
    /// Changes beyond this are refused outright.
    pub max_changes: Option<u32>,
    /// A transfer below this many minutes is refused. Minutes, not a risk
    /// figure: the risk belongs to `punctuality` and is read per leg, while
    /// this is the floor a person wants regardless of what the history says.
    pub min_transfer_buffer_min: Option<u32>,
    /// Allowed modes. Empty means no restriction.
    pub modes: Vec<String>,
    /// Overnight travel is refused rather than merely penalised.
    pub avoid_overnight_travel: bool,
    /// The station a search defaults its origin to.
    pub home_station: Option<String>,
    /// The airport a flight search defaults its origin to.
    pub home_airport: Option<String>,
    /// Discount cards the fare lookups must price against, e.g.
    /// `bahncard_25`, `deutschlandticket`.
    pub cards: Vec<String>,
}

impl Default for HardConstraints {
    fn default() -> Self {
        Self {
            earliest_departure: None,
            latest_arrival: None,
            max_changes: None,
            min_transfer_buffer_min: None,
            // Rail and flight, because those are the two the stack can actually
            // search. An empty list would mean "no restriction", which is not
            // what an unconfigured profile should claim.
            modes: vec!["rail".into(), "flight".into()],
            avoid_overnight_travel: false,
            home_station: None,
            home_airport: None,
            cards: Vec::new(),
        }
    }
}

/// The weights a ranking reads, keyed exactly as `plan_search`'s factors are.
///
/// The keys are not decoration either: `capabilities/trips/src/plan_search.rs`
/// builds `ScoreFactor`s with `key: "budget_fit"` and friends, drops any factor
/// it could not compute, and re-normalises the rest. Naming the same keys here
/// is what lets that file read a profile weight by factor key without a
/// translation table — and a translation table is where the two would drift.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SoftWeights {
    pub budget_fit: f64,
    pub feasibility: f64,
    pub season: f64,
    pub events: f64,
    /// The slot `plan_search.rs` declares and does not yet compute
    /// (`FACTOR_RETROSPECTIVE`). Carried here now so the profile shape does not
    /// change on the day the factor lands.
    pub retrospective: f64,
}

impl Default for SoftWeights {
    /// The defaults are the shape a traveller who cares about being somewhere
    /// specific would choose: what is on and what it costs outweigh the season.
    /// Deliberately NOT `plan-search-v1`'s constants — those were the repo's
    /// guess for everyone, and copying them here would make the first
    /// personalised profile indistinguishable from the unpersonalised one.
    fn default() -> Self {
        Self {
            budget_fit: 0.30,
            feasibility: 0.25,
            season: 0.15,
            events: 0.20,
            retrospective: 0.10,
        }
    }
}

impl SoftWeights {
    pub fn sum(&self) -> f64 {
        self.budget_fit + self.feasibility + self.season + self.events + self.retrospective
    }

    /// Whether the five weights sum to 1.0 within [`WEIGHT_SUM_TOLERANCE`].
    pub fn is_normalised(&self) -> bool {
        (self.sum() - 1.0).abs() <= WEIGHT_SUM_TOLERANCE
    }
}

/// How full a day is meant to be.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Pace {
    Slow,
    #[default]
    Balanced,
    Packed,
}

/// What a trip is anchored on. The order is the preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    /// A dated event — a conference, a hackathon, a show.
    Event,
    /// Visiting someone.
    Social,
    /// Doing something: a hike, a climb, water.
    Activity,
    /// Being somewhere to work.
    Work,
    /// Being somewhere to do nothing.
    Rest,
}

/// The profile as stored: the input plus the facts the server owns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TravelProfile {
    pub id: String,
    pub hard: HardConstraints,
    pub soft: SoftWeights,
    pub interests: Vec<String>,
    pub pace: Pace,
    pub anchors: Vec<Anchor>,
    /// One entry per field in [`BASIS_KEYS`]. A missing key is a bug, not a
    /// default — `ProfileInput::validate` refuses a map that does not cover the
    /// set exactly.
    pub basis: BTreeMap<String, Provenance>,
    /// 0 means nothing has ever been stored. Bumped on every write, and the
    /// value a conditional write compares against.
    pub revision: u32,
    pub updated_at: String,
}

impl TravelProfile {
    /// What a consumer gets before anything has been stored.
    ///
    /// Returned rather than a 404, because every consumer has to work with no
    /// profile at all and a `404` would make each of them invent the same
    /// fallback. `revision: 0` is how a caller tells this apart from a stored
    /// profile that happens to hold the defaults.
    pub fn unstated() -> Self {
        let mut basis = BTreeMap::new();
        for key in BASIS_KEYS {
            basis.insert((*key).to_string(), Provenance::Default);
        }
        Self {
            id: DEFAULT_PROFILE_ID.to_string(),
            hard: HardConstraints::default(),
            soft: SoftWeights::default(),
            interests: Vec::new(),
            pace: Pace::default(),
            anchors: Vec::new(),
            basis,
            revision: 0,
            updated_at: String::new(),
        }
    }

    /// The weights `plan_search` should use, or `None` when nothing has been
    /// stated and the caller should keep its own defaults.
    ///
    /// This is the seam that makes the upgrade safe: an unstated profile returns
    /// `None`, so a search against a fresh install ranks exactly as
    /// `plan-search-v1` did.
    pub fn stated_weights(&self) -> Option<SoftWeights> {
        self.basis
            .get("soft.budget_fit")
            .filter(|provenance| **provenance != Provenance::Default)
            .map(|_| self.soft)
    }
}

/// The write body. The server owns `id`, `revision` and `updated_at`, so a
/// caller cannot set them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProfileInput {
    pub hard: HardConstraints,
    pub soft: SoftWeights,
    pub interests: Vec<String>,
    pub pace: Pace,
    pub anchors: Vec<Anchor>,
    #[serde(default)]
    pub basis: BTreeMap<String, Provenance>,
}

impl ProfileInput {
    /// What a caller with nothing to say sends. Every field at its default and
    /// every basis entry `default`.
    pub fn unstated() -> Self {
        let profile = TravelProfile::unstated();
        Self {
            hard: profile.hard,
            soft: profile.soft,
            interests: profile.interests,
            pace: profile.pace,
            anchors: profile.anchors,
            basis: profile.basis,
        }
    }

    pub fn validate(&self) -> Result<(), ProfileError> {
        if !self.soft.is_normalised() {
            return Err(ProfileError::WeightSum(self.soft.sum()));
        }
        for (field, value) in [
            ("earliest_departure", &self.hard.earliest_departure),
            ("latest_arrival", &self.hard.latest_arrival),
        ] {
            if let Some(value) = value {
                if !is_clock(value) {
                    return Err(ProfileError::Clock {
                        field,
                        value: value.clone(),
                    });
                }
            }
        }
        for key in self.basis.keys() {
            if !BASIS_KEYS.contains(&key.as_str()) {
                return Err(ProfileError::UnknownBasisKey(key.clone()));
            }
        }
        for key in BASIS_KEYS {
            if !self.basis.contains_key(*key) {
                return Err(ProfileError::MissingBasisKey((*key).to_string()));
            }
        }
        Ok(())
    }
}

/// `HH:MM`, 24-hour, zero-padded. Minutes past the hour must be under 60 and
/// hours under 24; `7:5` and `25:00` are both refused rather than repaired.
pub fn is_clock(value: &str) -> bool {
    let Some((hours, minutes)) = value.split_once(':') else {
        return false;
    };
    if hours.len() != 2 || minutes.len() != 2 {
        return false;
    }
    if !hours.bytes().all(|b| b.is_ascii_digit()) || !minutes.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let (Ok(hours), Ok(minutes)) = (hours.parse::<u32>(), minutes.parse::<u32>()) else {
        return false;
    };
    hours < 24 && minutes < 60
}

/// Why a profile write was refused. Each variant carries the value that was
/// wrong, because "invalid profile" tells a caller nothing it can act on.
#[derive(Debug, Clone, PartialEq)]
pub enum ProfileError {
    WeightSum(f64),
    Clock { field: &'static str, value: String },
    UnknownBasisKey(String),
    MissingBasisKey(String),
}

impl std::fmt::Display for ProfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProfileError::WeightSum(sum) => write!(
                f,
                "soft weights must sum to 1.0, got {sum:.6} (budget_fit, feasibility, \
                 season, events, retrospective)"
            ),
            ProfileError::Clock { field, value } => write!(
                f,
                "hard.{field} must be HH:MM in 24-hour form, got {value:?}"
            ),
            ProfileError::UnknownBasisKey(key) => {
                write!(f, "basis names a field the profile does not have: {key:?}")
            }
            ProfileError::MissingBasisKey(key) => write!(
                f,
                "basis is missing {key:?}; every field must say where its value came from"
            ),
        }
    }
}

impl std::error::Error for ProfileError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unstated_profile_constrains_nothing_and_claims_no_weights() {
        let profile = TravelProfile::unstated();
        assert_eq!(profile.revision, 0);
        assert!(profile.hard.earliest_departure.is_none());
        assert!(profile.hard.latest_arrival.is_none());
        assert!(profile.hard.max_changes.is_none());
        // The seam the upgrade rests on: nothing stated, so a search keeps its
        // own v1 defaults rather than reading these.
        assert!(profile.stated_weights().is_none());
    }

    #[test]
    fn a_stated_weight_makes_the_profile_the_source() {
        let mut profile = TravelProfile::unstated();
        profile
            .basis
            .insert("soft.budget_fit".into(), Provenance::Stated);
        assert!(profile.stated_weights().is_some());
    }

    #[test]
    fn every_declared_field_has_a_basis_entry_and_none_are_invented() {
        let input = ProfileInput::unstated();
        input.validate().unwrap();
        assert_eq!(input.basis.len(), BASIS_KEYS.len());
        for key in BASIS_KEYS {
            assert!(
                input.basis.contains_key(*key),
                "{key} has no provenance, so a reader cannot tell a decision from a default"
            );
        }
    }

    #[test]
    fn a_basis_that_omits_a_field_is_refused_rather_than_defaulted() {
        let mut input = ProfileInput::unstated();
        input.basis.remove("hard.max_changes");
        assert_eq!(
            input.validate(),
            Err(ProfileError::MissingBasisKey("hard.max_changes".into()))
        );
    }

    #[test]
    fn a_basis_naming_a_field_that_does_not_exist_is_refused() {
        let mut input = ProfileInput::unstated();
        input.basis.insert("soft.vibes".into(), Provenance::Stated);
        assert_eq!(
            input.validate(),
            Err(ProfileError::UnknownBasisKey("soft.vibes".into()))
        );
    }

    #[test]
    fn weights_that_do_not_sum_to_one_are_refused_and_the_sum_is_named() {
        let mut input = ProfileInput::unstated();
        input.soft.events = 0.90;
        match input.validate() {
            Err(ProfileError::WeightSum(sum)) => assert!((sum - 1.70).abs() < 1e-9),
            other => panic!("expected a weight-sum refusal, got {other:?}"),
        }
    }

    #[test]
    fn the_default_weights_are_normalised() {
        assert!(SoftWeights::default().is_normalised());
    }

    #[test]
    fn a_clock_is_hh_mm_and_everything_else_is_refused() {
        for good in ["00:00", "07:00", "23:59", "09:05"] {
            assert!(is_clock(good), "{good} should be a clock");
        }
        for bad in [
            "7:00", "07:0", "24:00", "23:60", "0700", "", "07:00:00", "aa:bb",
        ] {
            assert!(!is_clock(bad), "{bad} should not be a clock");
        }
    }

    #[test]
    fn a_bad_clock_is_refused_and_the_field_is_named() {
        let mut input = ProfileInput::unstated();
        input.hard.latest_arrival = Some("24:30".into());
        match input.validate() {
            Err(ProfileError::Clock { field, value }) => {
                assert_eq!(field, "latest_arrival");
                assert_eq!(value, "24:30");
            }
            other => panic!("expected a clock refusal, got {other:?}"),
        }
    }
}
