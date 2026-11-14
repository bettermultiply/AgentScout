//! 控制面协调：Tree 动作投影与 AgentController。

mod actions;
mod controller;
pub mod execution;
pub mod recovery;
pub mod task;

#[cfg(test)]
mod tests;

pub use controller::AgentController;

use std::io;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::control_plane::agent_control::{
    AgentCommand, AgentMessage, AuthorityState, BlockedReason, BranchRole, BranchStatus,
    ChanceStage, DiscardedReason, OperationKind, OperationStatus, ProposalRecord,
    RuntimeExecutionState, SpeculativeTree,
};
use crate::protocol::contract::{Operation, RuntimeCapabilities};
use crate::control_plane::gateway::runtime::Runtime;
use crate::control_plane::policy::{CompiledPolicy, Decision, PolicyEngine};

/// 一个 Chance 的等待点：权威 Branch 在哪个 operation 上等待哪个工具结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolPending {
    pub chance_id: String,
    pub branch_id: String,
    pub operation_id: String,
    pub tool_name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControlOperation {
    /// 对给定的 Pending Chance 求值 `select`。
    Select(Vec<String>),
    Predict(ToolPending),
    Launch(ToolPending),
    Invoke(ToolPending),
    Establish(AgentCommand),
    Validate(String),
    Promote(String, Value),
    Terminate(String),
    Cleanup(String),
    Rebalance,
}

impl ControlOperation {
    pub fn references(&self, branch: &str) -> bool {
        self.branch() == Some(branch)
            || matches!(self, Self::Establish(AgentCommand::StartCandidate {parent_branch_id, ..}) if parent_branch_id == branch)
    }

    pub fn branch(&self) -> Option<&str> {
        match self {
            Self::Select(_) | Self::Rebalance => None,
            Self::Predict(tool) | Self::Launch(tool) | Self::Invoke(tool) => {
                Some(&tool.branch_id)
            }
            Self::Establish(command) => {
                let AgentCommand::StartCandidate { branch_id, .. } = command else {
                    unreachable!()
                };
                Some(branch_id)
            }
            Self::Validate(id) | Self::Promote(id, _) | Self::Terminate(id) | Self::Cleanup(id) => {
                Some(id)
            }
        }
    }

    pub fn chance_id(&self) -> Option<&str> {
        match self {
            Self::Predict(tool) | Self::Launch(tool) | Self::Invoke(tool) => {
                Some(&tool.chance_id)
            }
            _ => None,
        }
    }
}

pub enum ControlResult {
    Selected(Vec<String>),
    Predictions(Vec<Value>),
    Launch(bool),
    Outcome(Value, Value),
    Disposition(bool, String, Option<String>, Option<u64>),
    /// `(branch_id, action)` pairs returned by `rebalance`.
    Rebalance(Vec<(String, String)>),
    Completed,
}

#[derive(Default)]
pub struct ControlActions {
    pub commands: Vec<AgentCommand>,
    pub operations: Vec<(u64, ControlOperation)>,
    pub records: Vec<Value>,
}
