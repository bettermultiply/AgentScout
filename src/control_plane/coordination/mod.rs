
mod controller;
pub mod execution;
pub mod recovery;
pub mod session;
pub mod task;

#[cfg(test)]
mod tests;

pub use crate::control_plane::speculative_tree::{
    ControlActions, ControlOperation, ControlResult, ToolPending,
};
pub use controller::SpeculationCoordinator;

use std::io;

use serde_json::{Value, json};

use crate::control_plane::gateway::agent::protocol::{
    AgentCommand, AgentMessage, BlockedReason, BranchRole,
};
use crate::control_plane::gateway::runtime::Runtime;
use crate::control_plane::policy::{CompiledPolicy, Decision, PolicyEngine};
use crate::control_plane::speculative_tree::{
    BranchStatus, ProposalRecord, RuntimeExecutionState, SpeculativeTree,
};
use crate::protocol::contract::{Operation, RuntimeCapabilities};
