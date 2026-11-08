//! 控制面协调：Tree 动作投影与 AgentController。

mod actions;
mod controller;

#[cfg(test)]
mod tests;

pub use controller::AgentController;

use std::io;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::control_plane::agent_control::{
    AgentCommand, AgentMessage, BranchPhase, BranchStatus, OperationKind, OperationStatus,
    ProposalRecord, RuntimeExecutionState, SpeculativeTree,
};
use crate::protocol::contract::{Operation, RuntimeCapabilities};
use crate::control_plane::session::ControlSession;
use crate::control_plane::policy::{CompiledPolicy, Decision, PolicyEngine};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolPending {
    pub branch_id: String,
    pub operation_id: String,
    pub tool_name: String,
    pub arguments: Value,
    pub checkpoint: Option<Value>,
    pub launch: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControlOperation {
    Predict(ToolPending),
    Establish(AgentCommand),
    Invoke(ToolPending),
    Validate(String),
    Promote(String, Value),
    Terminate(String),
    Cleanup(String),
}

impl ControlOperation {
    pub fn references(&self, branch: &str) -> bool {
        self.branch() == branch
            || matches!(self, Self::Establish(AgentCommand::StartCandidate {parent_branch_id, ..}) if parent_branch_id == branch)
    }

    pub fn branch(&self) -> &str {
        match self {
            Self::Predict(tool) | Self::Invoke(tool) => &tool.branch_id,
            Self::Establish(command) => {
                let AgentCommand::StartCandidate { branch_id, .. } = command else {
                    unreachable!()
                };
                branch_id
            }
            Self::Validate(id) | Self::Promote(id, _) | Self::Terminate(id) | Self::Cleanup(id) => {
                id
            }
        }
    }
}

pub enum ControlResult {
    Predictions(Vec<Value>),
    PredictionsWithoutAuthority(Vec<Value>),
    Outcome(Value, Value),
    Disposition(bool, String),
    Completed,
}

#[derive(Default)]
pub struct ControlActions {
    pub commands: Vec<AgentCommand>,
    pub operations: Vec<(u64, ControlOperation)>,
    pub records: Vec<Value>,
}

