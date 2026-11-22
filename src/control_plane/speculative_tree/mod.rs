
mod actions;
mod candidates;
mod chances;
mod commands;
mod messages;
mod settlement;
mod tree;

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::control_plane::gateway::agent::protocol::{
    ActionKind, ActionStatus, AgentCommand, AgentMessage, BlockedReason, BranchRole,
    CommandOperation, CommandStatus, UiPrompt,
};

pub use actions::{ControlActions, ControlOperation, ControlResult, ToolPending};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchStatus {
    Created,
    Running,
    Blocked(BlockedReason),
    Finished,
    Promoting,
    Promoted,
    Discarding,
    Discarded,
    Cleaning,
    Cleaned,
}

impl BranchStatus {
    pub fn is_live_candidate(self) -> bool {
        matches!(
            self,
            Self::Created | Self::Running | Self::Blocked(_) | Self::Finished
        )
    }

    pub fn is_decidable(self) -> bool {
        matches!(
            self,
            Self::Running | Self::Finished | Self::Blocked(BlockedReason::EffectGate | BlockedReason::ExtentReached)
        )
    }

    pub fn is_live_authority(self) -> bool {
        matches!(self, Self::Running | Self::Promoted | Self::Blocked(_))
    }

    pub fn is_discarding_or_later(self) -> bool {
        matches!(
            self,
            Self::Discarding | Self::Discarded | Self::Cleaning | Self::Cleaned
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChanceStage {
    Pending,
    Running,
    Resolving,
    Accepted(String),
    Discarded(DiscardedReason),
}

impl ChanceStage {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Accepted(_) | Self::Discarded(_))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscardedReason {
    MatchRejected,
    NotSelected,
    AncestorInvalidated,
    BudgetExhausted,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AuthorityState {
    #[default]
    NotRequested,
    Fetching,
    Arrived,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionRecord {
    pub action_kind: ActionKind,
    pub name: String,
    pub parent_action_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ActionFinishedFact {
    pub action_id: String,
    pub action_kind: ActionKind,
    pub status: ActionStatus,
    pub timestamp_ms: u64,
    pub result: Option<Value>,
    pub evidence: Option<Value>,
    pub tool_calls: Option<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    Abort,
    Tool,
    StartAgent,
    StartCandidate,
    ResolvePending,
    PrepareAcceptance,
    CancelAcceptance,
    Promote,
    Discard,
    Cleanup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationStatus {
    Planned,
    Requested,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentExecutionState {
    #[default]
    Unknown,
    StartRequested,
    Running,
    Stopped,
    Released,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeExecutionState {
    #[default]
    Unknown,
    EnvironmentRequested,
    Ready,
    PromotionRequested,
    Promoted,
    TerminationRequested,
    Terminated,
    Cleaned,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchRecord {
    pub(crate) branch_id: String,
    #[serde(default)]
    pub(crate) environment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) parent_branch_id: Option<String>,
    pub(crate) role: BranchRole,
    pub(crate) status: BranchStatus,
    #[serde(default)]
    pub(crate) generation: u64,
    #[serde(default)]
    pub(crate) execution_position: ExecutionPosition,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) chance_id: Option<String>,
    /// 这个 Branch 携带的未确认前提，含祖先路径上的前提；自身的前提是
    /// `chance_id` 对应的那一项。
    #[serde(default)]
    pub(crate) assumption_set: Vec<Assumption>,
    #[serde(default)]
    pub(crate) pending_operations: Vec<PendingOperation>,
    #[serde(default)]
    pub(crate) execution_state: BranchExecutionState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ExecutionPosition {
    pub sequence: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    #[serde(default)]
    pub transcript_index: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Assumption {
    pub chance_id: String,
    pub proposal_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposalRecord {
    pub proposal_id: String,
    pub payload: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChanceRecord {
    pub chance_id: String,
    pub parent_branch_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_chance_id: Option<String>,
    pub operation_id: String,
    pub tool_name: String,
    pub arguments: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<Value>,
    pub stage: ChanceStage,
    pub authority: AuthorityState,
    pub proposals: Vec<ProposalRecord>,
    pub branch_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authority_outcome: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authority_evidence: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speculation_started_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) validation_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speculation_kind: Option<ActionKind>,
    #[serde(default)]
    pub(crate) tool_continuation_pending: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_epoch: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingOperation {
    pub operation_id: String,
    pub branch_id: String,
    pub kind: OperationKind,
    pub status: OperationStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline_at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SpeculationRecord {
    pub started_at_ms: u64,
    pub kind: Option<ActionKind>,
    pub settled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ModelUsageFact {
    #[serde(default)]
    pub action_id: Option<String>,
    pub model: String,
    pub started_at_ms: u64,
    pub finished_at_ms: u64,
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub tokens_used: u64,
    pub cost: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BranchExecutionState {
    pub agent: AgentExecutionState,
    pub runtime: RuntimeExecutionState,
    #[serde(default)]
    pub turn_active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpeculativeTree {
    agent_id: String,
    #[serde(default)]
    execution_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) history_root_branch_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) authority_branch_id: Option<String>,
    pub(crate) branches: BTreeMap<String, BranchRecord>,
    #[serde(default)]
    pub(crate) chances: BTreeMap<String, ChanceRecord>,
    /// Only live actions are retained; completed facts belong to the Ledger.
    #[serde(default)]
    pub(crate) active_actions: BTreeMap<String, BTreeMap<String, ActionRecord>>,
    #[serde(default)]
    pub(crate) action_facts: BTreeMap<String, BTreeMap<String, ActionFinishedFact>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    policy_binding: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    runtime_binding: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    capability_view: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    local_policy_digest: Option<String>,
    #[serde(default)]
    pending_commands: BTreeMap<String, PendingCommand>,
    #[serde(default)]
    queued_prompts: VecDeque<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) workspace_path: Option<PathBuf>,
    #[serde(default)]
    pub(crate) workspace_root: PathBuf,
    #[serde(default)]
    completed_work: BTreeMap<u64, [u8; 32]>,
    pending_work: BTreeMap<u64, ControlOperation>,
    #[serde(default)]
    pending_acceptance: Option<(String, Value)>,
    #[serde(default)]
    retained_source_environments: BTreeSet<String>,
    #[serde(skip)]
    policy_dependencies: BTreeMap<u64, Value>,
    capability_revision: u64,
    policy_chance_revisions: BTreeMap<String, (u64, u64)>,
    policy_record_revisions: BTreeMap<String, u64>,
    policy_revisions: BTreeMap<String, u64>,
    policy_work_context: BTreeMap<u64, BTreeMap<String, Value>>,
    #[serde(default)]
    pub(crate) speculations: BTreeMap<String, SpeculationRecord>,
    #[serde(default)]
    pub(crate) model_usage: BTreeMap<String, Vec<ModelUsageFact>>,
    #[serde(default)]
    pub(crate) tool_finished_at_ms: BTreeMap<String, BTreeMap<String, u64>>,
    #[serde(default)]
    next_work: u64,
    #[serde(default = "first_id")]
    next_chance: u64,
    #[serde(default)]
    epoch: u64,
    #[serde(default)]
    rebalance_epoch: Option<u64>,
    extent_disposals: BTreeSet<String>,
    #[serde(default)]
    stopping: bool,
    #[serde(default)]
    recovering: bool,
    #[serde(default)]
    task_finishing: bool,
    #[serde(default)]
    recovery_paused_at: Option<u64>,
    #[serde(default)]
    shutdown_deadline: Option<u64>,
    #[serde(default = "first_id")]
    next_generation: u64,
    #[serde(default = "first_id")]
    next_command_id: u64,
    #[serde(skip)]
    pending_observations: Vec<crate::control_plane::observation::ObservationContent>,
    #[serde(skip)]
    ui_prompts: BTreeMap<String, (String, UiPrompt, bool)>,
    #[serde(default)]
    command_catalog: Option<Value>,
    #[serde(skip)]
    ui_commands: Vec<AgentCommand>,
    #[serde(skip)]
    command_queries: BTreeMap<String, String>,
    #[serde(skip)]
    pending_records: Vec<Value>,
    #[serde(default)]
    pub(crate) transcripts: BTreeMap<String, Vec<Value>>,
    #[serde(default)]
    pub(crate) agent_launch: Option<crate::protocol::contract::AgentLaunch>,
}

impl SpeculativeTree {
    pub fn new(agent_id: String) -> Self {
        Self {
            agent_id,
            execution_id: String::new(),
            run_id: None,
            history_root_branch_id: None,
            authority_branch_id: None,
            branches: BTreeMap::new(),
            chances: BTreeMap::new(),
            active_actions: BTreeMap::new(),
            action_facts: BTreeMap::new(),
            policy_binding: None,
            runtime_binding: None,
            capability_view: None,
            local_policy_digest: None,
            pending_commands: BTreeMap::new(),
            queued_prompts: VecDeque::new(),
            workspace_root: PathBuf::new(),
            workspace_path: None,
            completed_work: BTreeMap::new(),
            pending_work: BTreeMap::new(),
            pending_acceptance: None,
            retained_source_environments: BTreeSet::new(),
            policy_dependencies: BTreeMap::new(),
            capability_revision: 0,
            policy_chance_revisions: BTreeMap::new(),
            policy_record_revisions: BTreeMap::new(),
            policy_revisions: BTreeMap::new(),
            policy_work_context: BTreeMap::new(),
            speculations: BTreeMap::new(),
            model_usage: BTreeMap::new(),
            tool_finished_at_ms: BTreeMap::new(),
            next_work: 1,
            next_chance: 1,
            epoch: 0,
            rebalance_epoch: None,
            extent_disposals: BTreeSet::new(),
            stopping: false,
            recovering: false,
            task_finishing: false,
            recovery_paused_at: None,
            shutdown_deadline: None,
            next_generation: 1,
            next_command_id: 1,
            pending_observations: Vec::new(),
            ui_prompts: BTreeMap::new(),
            command_catalog: None,
            ui_commands: Vec::new(),
            command_queries: BTreeMap::new(),
            pending_records: Vec::new(),
            transcripts: BTreeMap::new(),
            agent_launch: None,
        }
    }

    pub fn bind_execution(&mut self, policy: Value, runtime: Value, capabilities: Value) {
        self.policy_binding = Some(policy);
        self.runtime_binding = Some(runtime);
        self.capability_revision = self
            .capability_revision
            .checked_add(1)
            .expect("Capability revision exhausted");
        self.capability_view = Some(capabilities);
    }

    pub fn bind_local_policy(&mut self, digest: impl Into<String>) {
        self.local_policy_digest = Some(digest.into());
    }

    pub fn active_branch_ids(&self) -> BTreeSet<String> {
        let Some(root) = self.authority_branch_id.as_deref() else {
            return BTreeSet::new();
        };
        let mut ids = BTreeSet::from([root.to_string()]);
        let mut changed = true;
        while changed {
            changed = false;
            for branch in self.branches.values() {
                if branch
                    .parent_branch_id
                    .as_deref()
                    .is_some_and(|parent| ids.contains(parent))
                    && ids.insert(branch.branch_id.clone())
                {
                    changed = true;
                }
            }
        }
        ids
    }

    pub fn active_branches(&self) -> Vec<&BranchRecord> {
        let ids = self.active_branch_ids();
        self.branches
            .values()
            .filter(|branch| ids.contains(&branch.branch_id))
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PendingCommand {
    branch_id: String,
    operation: CommandOperation,
    operation_id: Option<String>,
    generation: u64,
}

fn operation_name(operation: CommandOperation) -> &'static str {
    match operation {
        CommandOperation::PrepareAcceptance => "prepare_acceptance",
        CommandOperation::CancelAcceptance => "cancel_acceptance",
        CommandOperation::Abort => "abort",
        CommandOperation::Promote => "promote",
        CommandOperation::ResolvePending => "resolve",
        CommandOperation::Discard => "discard",
        CommandOperation::Cleanup => "cleanup",
    }
}

fn invalid_input(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn first_id() -> u64 {
    1
}

#[cfg(test)]
mod tests;
