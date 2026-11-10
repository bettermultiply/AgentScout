//! Public observation facts, independent of transport and file storage.
use super::agent_control::{
    ActionKind, ActionStatus, BranchRole, BranchStatus, RuntimeExecutionState,
};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ObservationEvent {
    pub sequence: u64,
    pub timestamp_ms: u64,
    pub task_id: String,
    #[serde(flatten)]
    pub content: ObservationContent,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum ObservationContent {
    BranchCreated {
        branch_id: String,
        parent_branch_id: Option<String>,
        role: BranchRole,
        edge_id: Option<String>,
        speculation_kind: Option<ActionKind>,
    },
    AuthorityChanged {
        branch_id: String,
        previous_authority: Option<String>,
    },
    StatusChanged {
        branch_id: String,
        status: BranchStatus,
        reason: Option<String>,
    },
    EnvironmentChanged {
        branch_id: String,
        state: RuntimeExecutionState,
        independent_instance: Option<bool>,
        parent_environment: Option<String>,
        configuration: BTreeMap<String, Value>,
    },
    EnvironmentCleaned {
        branch_id: String,
        succeeded: bool,
        reason: Option<String>,
    },
    ConversationUpdated {
        branch_id: String,
        unverified: bool,
    },
    ModelUsage {
        branch_id: String,
        action_id: Option<String>,
        model: String,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        cache_read_tokens: Option<u64>,
        cache_write_tokens: Option<u64>,
        total_tokens: Option<u64>,
        cost: Option<f64>,
        duration_ms: Option<u64>,
    },
    ToolFinished {
        branch_id: String,
        action_id: String,
        status: ActionStatus,
    },
    Settlement {
        branch_id: String,
        hit: bool,
        time_ms: ObservationAmount,
        money: ObservationAmount,
        currency: String,
    },
    RunStatus {
        ready: bool,
        running: bool,
        notice: String,
        connected: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ObservationAmount {
    pub value: f64,
    pub measured: bool,
}

#[derive(Serialize)]
pub struct StateEventSet {
    pub sequence: u64,
    pub events: Vec<ObservationEvent>,
}
