//! Producer provenance and precedence for the Feed processing pipeline.

pub const EXTRACTION_REVISION: &str = "comms-extraction-v1";
pub const NORMALIZATION_REVISION: &str = "comms-normalization-v1";
pub const HUMAN_VERDICT_REVISION: &str = "human-verdict-v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageProvenance {
    pub stage: String,
    pub tier: String,
    pub revision: String,
    pub completed_at: String,
}

impl StageProvenance {
    pub fn legacy(revision: impl Into<String>) -> Self {
        Self::pending("legacy", revision)
    }

    pub fn deterministic(revision: impl Into<String>) -> Self {
        Self::pending("deterministic", revision)
    }

    pub fn model(revision: impl Into<String>) -> Self {
        Self::pending("model", revision)
    }

    pub fn human(revision: impl Into<String>) -> Self {
        Self::pending("human", revision)
    }

    fn pending(tier: &str, revision: impl Into<String>) -> Self {
        Self {
            stage: String::new(),
            tier: tier.to_string(),
            revision: revision.into(),
            completed_at: String::new(),
        }
    }
}

pub fn tier_rank(tier: &str) -> Option<i16> {
    match tier {
        "legacy" => Some(0),
        "deterministic" => Some(10),
        "model" => Some(20),
        "human" => Some(30),
        _ => None,
    }
}

pub fn ranking_tier(mode: &str) -> &'static str {
    match mode {
        "reranked" | "semantic" => "model",
        _ => "deterministic",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiers_have_one_explicit_order() {
        assert!(tier_rank("legacy") < tier_rank("deterministic"));
        assert!(tier_rank("deterministic") < tier_rank("model"));
        assert!(tier_rank("model") < tier_rank("human"));
        assert_eq!(tier_rank("unknown"), None);
    }
}
