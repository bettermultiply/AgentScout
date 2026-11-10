//! Decision 枚举及其对外字段名。

use serde::{Deserialize, Serialize};

/// `admit` has two separately invoked entries, not a tenth Decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Decision {
    #[serde(rename = "admit.opportunity_pass")]
    OpportunityPass,
    #[serde(rename = "admit.feasibility_pass")]
    FeasibilityPass,
    #[serde(rename = "select")]
    Select,
    #[serde(rename = "rebalance")]
    Rebalance,
    #[serde(rename = "extent")]
    Extent,
    #[serde(rename = "predict")]
    Predict,
    #[serde(rename = "launch")]
    Launch,
    #[serde(rename = "effect")]
    Effect,
    #[serde(rename = "match")]
    Match,
    #[serde(rename = "dispose")]
    Dispose,
}

impl Decision {
    pub const ALL: [Self; 10] = [
        Self::OpportunityPass,
        Self::FeasibilityPass,
        Self::Select,
        Self::Rebalance,
        Self::Extent,
        Self::Predict,
        Self::Launch,
        Self::Effect,
        Self::Match,
        Self::Dispose,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpportunityPass => "admit.opportunity_pass",
            Self::FeasibilityPass => "admit.feasibility_pass",
            Self::Select => "select",
            Self::Rebalance => "rebalance",
            Self::Extent => "extent",
            Self::Predict => "predict",
            Self::Launch => "launch",
            Self::Effect => "effect",
            Self::Match => "match",
            Self::Dispose => "dispose",
        }
    }

    pub(super) fn fields(self) -> (&'static [&'static str], &'static [&'static str]) {
        match self {
            Self::OpportunityPass => (&["op"], &[]),
            Self::FeasibilityPass => (&["op", "state", "contract_context"], &[]),
            Self::Select => (&["frontier_view", "scheduling_context"], &[]),
            Self::Rebalance => (&["active_view", "paused_view", "scheduling_context"], &[]),
            Self::Extent => (&["process_state"], &[]),
            Self::Predict => (
                &[
                    "chance_context",
                    "process_context",
                    "assumption_set",
                    "capability_context",
                ],
                &["history"],
            ),
            Self::Launch => (
                &["chance_context", "authority_state", "candidate_view"],
                &["scheduling_context"],
            ),
            Self::Effect => (&["effect_descriptor", "context"], &[]),
            Self::Match => (&["proposal", "evidence", "match_context"], &["outcome"]),
            Self::Dispose => (&["branch", "resolution", "context"], &[]),
        }
    }
}

