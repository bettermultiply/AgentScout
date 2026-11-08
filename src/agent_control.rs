//! AScout 控制面管理一个 Agent 的最小状态与 Agent 接入协议。
//!
//! 这里的状态属于控制面。Pi 接入侧只能报告事实并执行返回的命令，不能以
//! 自己的对象保存或裁决 Speculative Tree。

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 一个 Agent 内 Branch 的角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchRole {
    Authority,
    Candidate,
}

/// 控制面已知的 Branch 生命周期。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchStatus {
    Created,
    Running,
    AwaitingValidation,
    Promoting,
    Promoted,
    Discarding,
    Discarded,
    Cleaning,
    Cleaned,
}

/// The latest decision phase actually reached by this Branch.  This is an
/// observation of progress, not a permission, quota, or required ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BranchPhase {
    #[default]
    Idle,
    Select,
    Rebalance,
    Extent,
    Predict,
    Launch,
    Effect,
    Match,
    Dispose,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandOperation {
    Promote,
    ResolvePending,
    Discard,
    Cleanup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandStatus {
    Accepted,
    Started,
    Applied,
    Rejected,
    Failed,
    Timeout,
    Cancelled,
}

/// Agent action 类型不包含 Authority/Candidate；角色始终由 Tree 决定。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    ModelInference,
    ToolExecution,
    StartSubagent,
    SendMessage,
    Wait,
    Finish,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionStatus {
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionRecord {
    pub action_kind: ActionKind,
    pub name: String,
    pub parent_action_id: Option<String>,
}

/// 控制面操作的种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    Tool,
    StartAgent,
    StartCandidate,
    ResolvePending,
    Promote,
    Discard,
    Cleanup,
}

/// 控制面操作的落实状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationStatus {
    Planned,
    Requested,
    Succeeded,
    Failed,
}

/// Agent 接入侧报告的实际状态。
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

/// Runtime 报告的实际状态。
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

/// 控制面保存的一条 Branch 记录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchRecord {
    pub branch_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_branch_id: Option<String>,
    pub role: BranchRole,
    pub status: BranchStatus,
    #[serde(default)]
    pub generation: u64,
    #[serde(default)]
    pub execution_position: ExecutionPosition,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal_id: Option<String>,
    /// Unresolved Proposal IDs carried by this Branch. The prototype currently
    /// creates one direct Proposal per candidate, but preserves this boundary.
    #[serde(default)]
    pub assumption_set: Vec<String>,
    #[serde(default)]
    pub pending_operations: Vec<PendingOperation>,
    #[serde(default)]
    pub execution_state: BranchExecutionState,
}

/// Branch 在完整投机执行历史中的位置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ExecutionPosition {
    pub sequence: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    #[serde(default)]
    pub transcript_index: usize,
}

/// Candidate 携带的预测。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposalRecord {
    pub proposal_id: String,
    pub operation_id: String,
    pub payload: Value,
}

/// Capabilities declared by an Agent Adapter during the wire handshake.
/// These are descriptive facts; the control plane does not currently use them
/// to evaluate local Decisions or admit ordinary actions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterCapabilities {
    #[serde(rename = "preExecutionInterception")]
    pub pre_execution_interception: bool,
    #[serde(rename = "localDecisionEvaluation")]
    pub local_decision_evaluation: bool,
    #[serde(rename = "modelInferencePreExecutionInterception")]
    pub model_inference_pre_execution_interception: bool,
    #[serde(rename = "subagentPreExecutionInterception")]
    pub subagent_pre_execution_interception: bool,
    #[serde(rename = "checkpointContinuation")]
    pub checkpoint_continuation: bool,
    #[serde(rename = "independentCandidateSession")]
    pub independent_candidate_session: bool,
    #[serde(rename = "cancelCompletion")]
    pub cancel_completion: bool,
    #[serde(rename = "toolResultInjection")]
    pub tool_result_injection: bool,
}

/// 控制面希望发生的操作，不代表外部操作已经完成。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingOperation {
    pub operation_id: String,
    pub branch_id: String,
    pub kind: OperationKind,
    pub status: OperationStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
}

/// Branch 对应的 Agent 与 Runtime 实际事实。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BranchExecutionState {
    pub agent: AgentExecutionState,
    pub runtime: RuntimeExecutionState,
    #[serde(default)]
    pub turn_active: bool,
    #[serde(default)]
    pub phase: BranchPhase,
    /// Validation facts belong to the Branch execution record, not to the
    /// command used to start a candidate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_outcome: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_evidence: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
}

/// 一个 Agent 的完整投机树视图。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpeculativeTree {
    pub agent_id: String,
    #[serde(default)]
    pub execution_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_root_branch_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authority_branch_id: Option<String>,
    pub branches: BTreeMap<String, BranchRecord>,
    /// Immutable Proposal records, indexed independently from Branch state.
    #[serde(default)]
    pub proposals: BTreeMap<String, ProposalRecord>,
    /// Only live actions are retained; completed facts belong to the Ledger.
    #[serde(default)]
    pub active_actions: BTreeMap<String, BTreeMap<String, ActionRecord>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_binding: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_binding: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_view: Option<Value>,
    /// Digest of the local Policy artifact delivered to the Agent Adapter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_policy_digest: Option<String>,
    #[serde(default)]
    pending_commands: BTreeMap<String, PendingCommand>,
    #[serde(default)]
    queued_prompts: VecDeque<String>,
    #[serde(default)]
    pub workspace_root: PathBuf,
    #[serde(default)]
    pub(crate) pending_work: BTreeMap<u64, crate::coordination::ControlOperation>,
    #[serde(default)]
    pub(crate) tool_invocations: Vec<crate::coordination::ToolPending>,
    #[serde(default)]
    pub(crate) next_work: u64,
    #[serde(default)]
    pub(crate) stopping: bool,
    #[serde(default)]
    shutdown_deadline: Option<u64>,
    #[serde(default = "first_id")]
    next_generation: u64,
    #[serde(default = "first_id")]
    next_command_id: u64,
}

impl SpeculativeTree {
    pub fn new(agent_id: String) -> Self {
        Self {
            agent_id,
            execution_id: String::new(),
            history_root_branch_id: None,
            authority_branch_id: None,
            branches: BTreeMap::new(),
            proposals: BTreeMap::new(),
            active_actions: BTreeMap::new(),
            policy_binding: None,
            runtime_binding: None,
            capability_view: None,
            local_policy_digest: None,
            pending_commands: BTreeMap::new(),
            queued_prompts: VecDeque::new(),
            workspace_root: PathBuf::new(),
            pending_work: BTreeMap::new(),
            tool_invocations: Vec::new(),
            next_work: 1,
            stopping: false,
            shutdown_deadline: None,
            next_generation: 1,
            next_command_id: 1,
        }
    }

    /// 绑定这次投机执行使用的 Policy、Runtime Adapter 和能力视图。
    /// 这些值属于 Tree，不属于某个 Branch。
    pub fn bind_execution(&mut self, policy: Value, runtime: Value, capabilities: Value) {
        self.policy_binding = Some(policy);
        self.runtime_binding = Some(runtime);
        self.capability_view = Some(capabilities);
    }

    pub fn bind_local_policy(&mut self, digest: impl Into<String>) {
        self.local_policy_digest = Some(digest.into());
    }

    /// 根据完整历史中的父子关系计算当前执行树。
    /// 当前执行树不是第二套事实，只是从活动根派生的 Branch 集合。
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

    /// 返回当前执行树中的 Branch，而不是完整历史中的全部 Branch。
    pub fn active_branches(&self) -> Vec<&BranchRecord> {
        let ids = self.active_branch_ids();
        self.branches
            .values()
            .filter(|branch| ids.contains(&branch.branch_id))
            .collect()
    }
}

/// 控制面向 Pi 接入侧发送的命令。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum AgentCommand {
    /// Delivers the compiled local Decision artifact. The adapter may reject
    /// it when it has no real Hook/evaluation seam.
    LocalPolicy {
        branch_id: String,
        policy_digest: String,
        decisions: Value,
    },
    Prompt {
        branch_id: String,
        prompt: String,
    },
    Abort {
        branch_id: String,
    },
    StartCandidate {
        branch_id: String,
        parent_branch_id: String,
        operation_id: String,
        tool_name: String,
        workspace: PathBuf,
        prediction: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        checkpoint: Option<Value>,
        #[serde(default)]
        command_id: String,
        #[serde(default)]
        generation: u64,
    },
    ResolvePending {
        branch_id: String,
        operation_id: String,
        accepted: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        outcome: Option<Value>,
        #[serde(default)]
        command_id: String,
        #[serde(default)]
        generation: u64,
    },
    StartBranch {
        branch_id: String,
        role: BranchRole,
        workspace: PathBuf,
        prompt: String,
    },
    Promote {
        branch_id: String,
        #[serde(default)]
        command_id: String,
        #[serde(default)]
        generation: u64,
    },
    Discard {
        branch_id: String,
        #[serde(default)]
        command_id: String,
        #[serde(default)]
        generation: u64,
    },
    Cleanup {
        branch_id: String,
        workspace: PathBuf,
        #[serde(default)]
        command_id: String,
        #[serde(default)]
        generation: u64,
    },
}

/// Agent Adapter 向控制面报告的事实；不包含 Runtime 或 Tree 完成事实。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "message", rename_all = "snake_case")]
pub enum AgentMessage {
    Output {
        branch_id: String,
        #[serde(default)]
        action_id: Option<String>,
        text: String,
    },
    TurnCompleted {
        branch_id: String,
    },
    ActionStarted {
        task_id: String,
        branch_id: String,
        action_id: String,
        action_kind: ActionKind,
        #[serde(default)]
        parent_action_id: Option<String>,
        /// true 表示接入侧停在执行前边界，尚未执行真实 action。
        result_pending: bool,
        name: String,
        arguments: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        checkpoint: Option<Value>,
    },
    /// Explicit request from the adapter's local decision path. An ordinary
    /// ActionStarted is only an execution fact and never admits speculation.
    SpeculativeAdmissionRequest {
        task_id: String,
        policy_digest: String,
        branch_id: String,
        action_id: String,
        action_kind: ActionKind,
        #[serde(default)]
        parent_action_id: Option<String>,
        name: String,
        arguments: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        checkpoint: Option<Value>,
    },
    Transcript {
        branch_id: String,
        entry: Value,
    },
    BranchReleased {
        branch_id: String,
    },
    AgentSessionReleased {
        branch_id: String,
    },
    AgentConnected {
        agent_id: String,
        capabilities: AdapterCapabilities,
    },
    AdapterCapabilityGap {
        branch_id: String,
        capability: String,
        status: String,
        reason: String,
    },
    BranchStarted {
        branch_id: String,
    },
    ActionFinished {
        task_id: String,
        branch_id: String,
        action_id: String,
        action_kind: ActionKind,
        status: ActionStatus,
        result: Value,
        #[serde(default)]
        evidence: Option<Value>,
    },
    Usage {
        branch_id: String,
        #[serde(default)]
        action_id: Option<String>,
        model: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thinking_level: Option<String>,
        usage: Value,
    },
    BranchFailed {
        branch_id: String,
        reason: String,
    },
    AgentReady {
        agent_id: String,
    },
    AgentDisconnected {
        agent_id: String,
    },
    AgentFailed {
        agent_id: String,
        reason: String,
    },
    AgentReleased {
        agent_id: String,
    },
    BranchCreated {
        branch_id: String,
    },
    BranchReady {
        branch_id: String,
    },
    BranchSuspended {
        branch_id: String,
    },
    BranchResumed {
        branch_id: String,
    },
    CommandResult {
        command_id: String,
        branch_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        operation_id: Option<String>,
        operation: CommandOperation,
        status: CommandStatus,
        generation: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
}

/// Agent socket 上的双向消息。长度前缀由传输层负责，消息本身只描述方向。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum AgentWireMessage {
    Command(AgentCommand),
    Message(AgentMessage),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PendingCommand {
    branch_id: String,
    operation: CommandOperation,
    operation_id: Option<String>,
    generation: u64,
}

impl SpeculativeTree {
    pub fn with_workspace(
        agent_id: impl Into<String>,
        root: impl Into<PathBuf>,
    ) -> io::Result<Self> {
        let mut tree = Self::new(agent_id.into());
        tree.workspace_root = root.into();
        fs::create_dir_all(tree.workspace())?;
        Ok(tree)
    }

    pub fn workspace(&self) -> PathBuf {
        self.workspace_root.join("workspace")
    }

    /// Runtime tools may only execute for the live authority recorded by the Tree.
    /// Candidate and late messages must never be routed into another Branch's world.
    pub fn can_invoke_tool(&self, branch_id: &str) -> bool {
        self.authority_branch_id.as_deref() == Some(branch_id)
            && self.branches.get(branch_id).is_some_and(|branch| {
                matches!(
                    branch.status,
                    BranchStatus::Running | BranchStatus::Promoted
                ) && branch.execution_state.agent == AgentExecutionState::Running
            })
    }

    pub fn authority_ready(&self) -> bool {
        self.authority_branch_id
            .as_ref()
            .and_then(|id| self.branches.get(id))
            .is_some_and(|branch| {
                branch.execution_state.agent == AgentExecutionState::Running
                    && !matches!(
                        branch.status,
                        BranchStatus::Cleaning | BranchStatus::Cleaned
                    )
            })
    }

    pub fn authority_running(&self) -> bool {
        self.authority_branch_id
            .as_ref()
            .and_then(|id| self.branches.get(id))
            .is_some_and(|branch| branch.execution_state.turn_active)
    }

    pub fn queue_prompt(&mut self, prompt: String) {
        if !prompt.trim().is_empty() {
            self.queued_prompts.push_back(prompt);
        }
    }

    pub fn next_prompt(&mut self) -> io::Result<Option<AgentCommand>> {
        let Some(prompt) = self.queued_prompts.front().cloned() else {
            return Ok(None);
        };
        let command = self.continue_authority(prompt)?;
        if command.is_some() {
            self.queued_prompts.pop_front();
        }
        Ok(command)
    }

    /// A prompt starts a turn on the confirmed authority, never on an intended winner.
    pub fn continue_authority(&mut self, prompt: String) -> io::Result<Option<AgentCommand>> {
        if !self.authority_ready()
            || self.authority_running()
            || self
                .branches
                .values()
                .any(|branch| branch.status == BranchStatus::Promoting)
        {
            return Ok(None);
        }
        let branch_id = self.authority_branch_id.clone().unwrap();
        self.branch_mut(&branch_id)?.execution_state.turn_active = true;
        Ok(Some(AgentCommand::Prompt { branch_id, prompt }))
    }

    /// Create the candidate, proposal, position and start intent as one Tree operation.
    pub fn start_candidate(
        &mut self,
        parent: &str,
        operation_id: &str,
        tool_name: &str,
        prediction: Value,
        checkpoint: Option<Value>,
    ) -> io::Result<AgentCommand> {
        if self.authority_branch_id.as_deref() != Some(parent) {
            return Err(invalid_input(
                "candidate parent is not the current authority",
            ));
        }
        let sequence = self.next_generation;
        let branch_id = format!("candidate-{sequence}");
        let transcript_index = self.branch(parent)?.execution_position.transcript_index;
        if !matches!(
            self.branch(parent)?.status,
            BranchStatus::Created | BranchStatus::Running | BranchStatus::Promoted
        ) {
            return Err(invalid_input("parent branch cannot create a candidate"));
        }
        self.insert_branch(
            &branch_id,
            Some(parent.to_string()),
            BranchRole::Candidate,
            BranchStatus::Created,
        )?;
        self.record_candidate_context(
            &branch_id,
            operation_id,
            prediction.clone(),
            sequence,
            transcript_index,
        )?;
        self.mark_operation_requested(&branch_id, operation_id, OperationKind::StartCandidate)?;
        let branch = self.branch_mut(&branch_id)?;
        branch.pending_operations.last_mut().unwrap().tool_name = Some(tool_name.to_string());
        branch.execution_state.runtime = RuntimeExecutionState::EnvironmentRequested;
        branch.execution_state.turn_active = true;
        Ok(AgentCommand::StartCandidate {
            branch_id: branch_id.clone(),
            parent_branch_id: parent.to_string(),
            operation_id: operation_id.to_string(),
            tool_name: tool_name.to_string(),
            workspace: self.workspace(),
            prediction,
            checkpoint,
            command_id: format!("start-{branch_id}"),
            generation: self.branch(&branch_id)?.generation,
        })
    }

    pub fn pending_candidates(&self) -> Vec<String> {
        self.branches
            .values()
            .filter(|branch| {
                branch.role == BranchRole::Candidate
                    && matches!(
                        branch.status,
                        BranchStatus::Created
                            | BranchStatus::Running
                            | BranchStatus::AwaitingValidation
                    )
                    && branch.proposal_id.is_some()
            })
            .map(|branch| branch.branch_id.clone())
            .collect()
    }

    pub fn candidate_outcome(&self, candidate: &str) -> Option<&Value> {
        let branch = self.branches.get(candidate)?;
        branch.execution_state.validation_outcome.as_ref()
    }

    /// Runtime evidence gathered for the candidate's real action. This is the
    /// verifier input (`before`/`after` observations), not the model-facing
    /// tool result.
    pub fn candidate_evidence(&self, candidate: &str) -> Option<&Value> {
        let branch = self.branches.get(candidate)?;
        branch.execution_state.validation_evidence.as_ref()
    }

    pub fn candidate_due(&self, candidate: &str) -> bool {
        let Some(branch) = self.branches.get(candidate) else {
            return false;
        };
        if !self.pending_candidates().iter().any(|id| id == candidate)
            || self
                .branches
                .values()
                .any(|branch| branch.status == BranchStatus::Promoting)
        {
            return false;
        }
        (self.candidate_outcome(candidate).is_some()
            && matches!(
                branch.status,
                BranchStatus::Running | BranchStatus::AwaitingValidation
            ))
            || branch.pending_operations.iter().any(|operation| {
                operation.kind == OperationKind::StartCandidate
                    && operation
                        .deadline_at
                        .is_some_and(|deadline| deadline <= now_ms())
            })
    }

    /// A candidate may only win while the authority is still at the waiting
    /// point the candidate forked from. Once the authority has started another
    /// tool operation, promoting the candidate would roll the authority back
    /// past work that has already been executed.
    fn authority_advanced_past(&self, candidate: &BranchRecord) -> bool {
        let Some(proposal_id) = candidate.proposal_id.as_ref() else {
            return true;
        };
        let Some(proposal) = self.proposals.get(proposal_id) else {
            return true;
        };
        let Some(authority) = self
            .authority_branch_id
            .as_deref()
            .and_then(|id| self.branches.get(id))
        else {
            return true;
        };
        authority.pending_operations.iter().any(|operation| {
            operation.kind == OperationKind::Tool
                && operation.status == OperationStatus::Requested
                && operation.operation_id != proposal.operation_id
        })
    }

    pub fn can_promote(&self, candidate: &str, compatible: bool) -> bool {
        compatible
            && self.branches.get(candidate).is_some_and(|branch| {
                branch.role == BranchRole::Candidate
                    && matches!(
                        branch.status,
                        BranchStatus::Running | BranchStatus::AwaitingValidation
                    )
                    && branch.parent_branch_id == self.authority_branch_id
                    && !self.authority_advanced_past(branch)
            })
            && !self
                .branches
                .values()
                .any(|branch| branch.status == BranchStatus::Promoting)
    }

    pub fn is_stopping(&self) -> bool {
        self.stopping
    }

    pub fn check_command_deadlines(&mut self) -> io::Result<()> {
        if self
            .shutdown_deadline
            .is_some_and(|deadline| deadline <= now_ms())
        {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Agent shutdown incomplete: cleanup or connection drain timed out",
            ));
        }
        if let Some(operation) = self
            .branches
            .values()
            .flat_map(|branch| &branch.pending_operations)
            .find(|operation| {
                operation.kind == OperationKind::Tool
                    && operation.status == OperationStatus::Requested
                    && operation
                        .deadline_at
                        .is_some_and(|deadline| deadline <= now_ms())
            })
        {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "Tool {} timed out for {}",
                    operation.operation_id, operation.branch_id
                ),
            ));
        }
        if let Some(branch) = self.branches.values().find(|branch| {
            branch.status == BranchStatus::Created
                && branch.pending_operations.iter().any(|operation| {
                    operation.kind == OperationKind::StartAgent
                        && operation
                            .deadline_at
                            .is_some_and(|deadline| deadline <= now_ms())
                })
        }) {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("Agent startup timed out for {}", branch.branch_id),
            ));
        }
        let expired = self
            .pending_commands
            .iter()
            .find(|(id, command)| {
                self.branches[&command.branch_id]
                    .pending_operations
                    .iter()
                    .any(|operation| {
                        operation.operation_id == **id
                            && operation
                                .deadline_at
                                .is_some_and(|deadline| deadline <= now_ms())
                    })
            })
            .map(|(id, command)| (id.clone(), command.clone()));
        if let Some((command_id, pending)) = expired {
            self.complete_operation(
                &pending.branch_id,
                Some(&command_id),
                OperationStatus::Failed,
            )?;
            // A timed-out authority switch is ambiguous. Never resume the old
            // authority as if the Agent had confirmed rejection.
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "{} confirmation timed out for {}",
                    operation_name(pending.operation),
                    pending.branch_id
                ),
            ));
        }
        Ok(())
    }

    /// Cancellation affects all pending candidates; their subtrees are discarded by resolve_candidate.
    pub fn interrupt(&mut self) -> io::Result<Vec<AgentCommand>> {
        self.queued_prompts.clear();
        self.tool_invocations.clear();
        for operation in self
            .branches
            .values_mut()
            .flat_map(|branch| &mut branch.pending_operations)
        {
            if operation.kind == OperationKind::Tool
                && operation.status == OperationStatus::Requested
            {
                operation.status = OperationStatus::Failed;
            }
        }
        let mut commands = Vec::new();
        for candidate in self.pending_candidates() {
            if self.pending_candidates().contains(&candidate) {
                commands.extend(self.resolve_candidate(&candidate, false)?);
            }
        }
        // A switch already in flight cannot be assumed rejected. Abort the
        // intended winner too, while retaining Promote's receipt until known.
        for branch in self
            .branches
            .values()
            .filter(|branch| branch.status == BranchStatus::Promoting)
        {
            commands.push(AgentCommand::Abort {
                branch_id: branch.branch_id.clone(),
            });
        }
        if let Some(branch_id) = self.authority_branch_id.clone() {
            commands.push(AgentCommand::Abort { branch_id });
        }
        Ok(commands)
    }

    pub fn begin_shutdown(&mut self) -> io::Result<Vec<AgentCommand>> {
        if self.stopping {
            return Ok(Vec::new());
        }
        self.stopping = true;
        self.shutdown_deadline = Some(now_ms() + 10_000);
        self.tool_invocations.clear();
        self.queued_prompts.clear();
        let ids: Vec<_> = self
            .branches
            .values()
            .filter(|branch| branch.status != BranchStatus::Cleaned)
            .filter(|branch| {
                !self.pending_commands.values().any(|command| {
                    command.branch_id == branch.branch_id
                        && command.operation == CommandOperation::Cleanup
                })
            })
            .map(|branch| branch.branch_id.clone())
            .collect();
        ids.into_iter().map(|id| self.begin_cleanup(&id)).collect()
    }

    pub fn cleanup_pending(&self) -> io::Result<bool> {
        if self.branches.values().any(|branch| {
            branch.pending_operations.iter().any(|operation| {
                operation.kind == OperationKind::Cleanup
                    && operation.status == OperationStatus::Failed
            })
        }) {
            return Err(io::Error::other("Agent cleanup failed"));
        }
        Ok(self
            .pending_commands
            .values()
            .any(|command| command.operation == CommandOperation::Cleanup))
    }

    pub fn resolve_pending(
        &mut self,
        branch_id: &str,
        operation_id: &str,
        outcome: Value,
    ) -> io::Result<AgentCommand> {
        let generation = self.branch(branch_id)?.generation;
        let command_id = self.register(
            branch_id,
            CommandOperation::ResolvePending,
            Some(operation_id.to_string()),
            generation,
        )?;
        Ok(AgentCommand::ResolvePending {
            branch_id: branch_id.to_string(),
            operation_id: operation_id.to_string(),
            accepted: false,
            outcome: Some(outcome),
            command_id,
            generation,
        })
    }

    /// 把 Candidate 的预测和执行位置写入 Tree；候选运行数据随后只从
    /// BranchRecord 读取，不再需要控制循环维护第二份 Proposal 状态。
    pub fn record_candidate_context(
        &mut self,
        branch_id: &str,
        operation_id: impl Into<String>,
        prediction: Value,
        sequence: u64,
        transcript_index: usize,
    ) -> io::Result<()> {
        let operation_id = operation_id.into();
        {
            let branch = self.branch_mut(branch_id)?;
            if branch.role != BranchRole::Candidate {
                return Err(invalid_input("only a candidate can have a proposal"));
            }
            branch.execution_position = ExecutionPosition {
                sequence,
                operation_id: Some(operation_id.clone()),
                transcript_index,
            };
            branch.execution_state.phase = BranchPhase::Predict;
        }
        let proposal_id = format!("proposal-{branch_id}");
        if self.proposals.contains_key(&proposal_id) {
            return Err(invalid_input("proposal id already exists"));
        }
        self.proposals.insert(
            proposal_id.clone(),
            ProposalRecord {
                proposal_id: proposal_id.clone(),
                operation_id: operation_id.clone(),
                payload: prediction,
            },
        );
        let branch = self.branch_mut(branch_id)?;
        branch.proposal_id = Some(proposal_id.clone());
        branch.assumption_set = vec![proposal_id];
        branch.pending_operations.push(PendingOperation {
            operation_id,
            branch_id: branch_id.to_string(),
            kind: OperationKind::StartCandidate,
            status: OperationStatus::Planned,
            outcome: None,
            evidence: None,
            requested_at: None,
            deadline_at: None,
            tool_name: None,
        });
        branch.execution_state.agent = AgentExecutionState::StartRequested;
        Ok(())
    }

    pub fn mark_operation_requested(
        &mut self,
        branch_id: &str,
        operation_id: &str,
        kind: OperationKind,
    ) -> io::Result<()> {
        let branch = self.branch_mut(branch_id)?;
        if let Some(operation) = branch
            .pending_operations
            .iter_mut()
            .find(|operation| operation.operation_id == operation_id)
        {
            operation.status = OperationStatus::Requested;
            operation.requested_at = Some(now_ms());
            operation.deadline_at = Some(now_ms() + 120_000);
        } else {
            branch.pending_operations.push(PendingOperation {
                operation_id: operation_id.to_string(),
                branch_id: branch_id.to_string(),
                kind,
                status: OperationStatus::Requested,
                outcome: None,
                evidence: None,
                requested_at: Some(now_ms()),
                deadline_at: Some(now_ms() + 120_000),
                tool_name: None,
            });
        }
        Ok(())
    }

    /// Runtime facts are independent of Agent session cleanup. In particular,
    /// releasing a session does not clean a shared execution environment.
    pub fn record_runtime_state(
        &mut self,
        branch_id: &str,
        state: RuntimeExecutionState,
    ) -> io::Result<()> {
        self.branch_mut(branch_id)?.execution_state.runtime = state;
        Ok(())
    }

    /// Orderly process exit also releases the current Authority's session.
    /// Runtime environment cleanup remains a separate confirmed operation.
    pub fn begin_cleanup(&mut self, branch_id: &str) -> io::Result<AgentCommand> {
        self.branch(branch_id)?;
        let command = self.make_cleanup_command(branch_id)?;
        self.branch_mut(branch_id)?.status = BranchStatus::Cleaning;
        Ok(command)
    }

    pub fn complete_operation(
        &mut self,
        branch_id: &str,
        operation_id: Option<&str>,
        status: OperationStatus,
    ) -> io::Result<()> {
        let branch = self.branch_mut(branch_id)?;
        if let Some(operation) = branch
            .pending_operations
            .iter_mut()
            .find(|operation| operation_id.is_none_or(|id| operation.operation_id == id))
        {
            operation.status = status;
        }
        Ok(())
    }

    /// 将外部结果写回拥有该 Proposal 的 Candidate。
    /// 控制循环只提供事件，不在本地保存结果副本。
    pub fn record_candidate_outcome(
        &mut self,
        parent_branch_id: &str,
        operation_id: &str,
        outcome: Value,
        evidence: Option<Value>,
    ) -> io::Result<()> {
        let candidate_ids: Vec<String> = self
            .branches
            .values()
            .filter(|branch| {
                branch.parent_branch_id.as_deref() == Some(parent_branch_id)
                    && branch
                        .proposal_id
                        .as_ref()
                        .and_then(|id| self.proposals.get(id))
                        .is_some_and(|proposal| proposal.operation_id == operation_id)
            })
            .map(|branch| branch.branch_id.clone())
            .collect();
        for candidate_id in candidate_ids {
            let branch = self.branch_mut(&candidate_id)?;
            branch.execution_state.validation_outcome = Some(outcome.clone());
            branch.execution_state.validation_evidence = evidence.clone();
        }
        Ok(())
    }

    /// Return terminal Branches that may be handed to the Ledger.
    pub fn evictable_branches(&self) -> Vec<String> {
        self.branches
            .values()
            .filter(|branch| {
                !self
                    .pending_work
                    .values()
                    .any(|work| work.references(&branch.branch_id))
            })
            .filter(|branch| {
                branch.branch_id != self.authority_branch_id.as_deref().unwrap_or_default()
                    && matches!(branch.status, BranchStatus::Cleaned)
                    && !self
                        .pending_commands
                        .values()
                        .any(|command| command.branch_id == branch.branch_id)
                    && !self.branches.values().any(|child| {
                        child.parent_branch_id.as_deref() == Some(branch.branch_id.as_str())
                    })
            })
            .map(|branch| branch.branch_id.clone())
            .collect()
    }

    /// 从活动 Tree 中取出终止 Branch，并把它的所有权交给调用方的 Ledger。
    ///
    /// 从这一步开始，核心数据结构已经不再拥有该 Branch；Ledger 决定
    /// 记录对象何时释放。调用方应立即把返回值传给 Ledger。
    pub fn take_archivable_branch(&mut self, branch_id: &str) -> io::Result<BranchRecord> {
        if !self.evictable_branches().iter().any(|id| id == branch_id) {
            return Err(invalid_input("branch is not safe to archive"));
        }
        self.active_actions.remove(branch_id);
        self.branches
            .remove(branch_id)
            .ok_or_else(|| invalid_input("branch does not exist"))
    }

    /// Set only when the authority Branch itself can no longer continue.
    pub fn authority_failure(&self) -> Option<&str> {
        self.authority_branch_id
            .as_ref()
            .and_then(|id| self.branches.get(id))
            .and_then(|branch| branch.execution_state.failure.as_deref())
    }

    /// 由控制面登记初始权威 Branch，并要求 Pi 启动它。
    pub fn create_authority(
        &mut self,
        branch_id: impl Into<String>,
        prompt: impl Into<String>,
    ) -> io::Result<AgentCommand> {
        let branch_id = branch_id.into();
        if self.authority_branch_id.is_some() {
            return Err(invalid_input("tree already has an authority"));
        }
        self.insert_branch(
            &branch_id,
            None,
            BranchRole::Authority,
            BranchStatus::Created,
        )?;
        self.authority_branch_id = Some(branch_id.clone());
        self.history_root_branch_id = Some(branch_id.clone());
        self.branch_mut(&branch_id)?.execution_state.agent = AgentExecutionState::StartRequested;
        let prompt = prompt.into();
        self.branch_mut(&branch_id)?.execution_state.turn_active = !prompt.is_empty();
        self.mark_operation_requested(&branch_id, "start-agent", OperationKind::StartAgent)?;
        Ok(AgentCommand::StartBranch {
            role: BranchRole::Authority,
            workspace: self.workspace(),
            branch_id,
            prompt,
        })
    }

    /// 接收 Pi 报告的启动事实。
    pub fn handle_message(&mut self, message: AgentMessage) -> io::Result<Vec<AgentCommand>> {
        match message {
            AgentMessage::AgentConnected { agent_id, .. } if agent_id == self.agent_id => {
                Ok(vec![])
            }
            AgentMessage::AgentConnected { .. } => {
                Err(invalid_input("agent id does not match controller"))
            }
            AgentMessage::AgentReady { agent_id } | AgentMessage::AgentReleased { agent_id } => {
                if agent_id != self.agent_id {
                    return Err(invalid_input("agent id does not match controller"));
                }
                Ok(vec![])
            }
            AgentMessage::AgentDisconnected { agent_id } => {
                if agent_id != self.agent_id {
                    return Err(invalid_input("agent id does not match controller"));
                }
                Err(io::Error::other(
                    "Agent disconnected; session release is not confirmed",
                ))
            }
            AgentMessage::AgentFailed { agent_id, reason } => {
                if agent_id != self.agent_id {
                    return Err(invalid_input("agent id does not match controller"));
                }
                Err(io::Error::other(reason))
            }
            AgentMessage::BranchCreated { branch_id } | AgentMessage::BranchReady { branch_id } => {
                self.branch(&branch_id)?;
                Ok(vec![])
            }
            AgentMessage::BranchSuspended { branch_id } => {
                let branch = self.branch_mut(&branch_id)?;
                if matches!(
                    branch.status,
                    BranchStatus::Running
                        | BranchStatus::Promoted
                        | BranchStatus::AwaitingValidation
                ) {
                    branch.execution_state.agent = AgentExecutionState::Stopped;
                }
                Ok(vec![])
            }
            AgentMessage::BranchResumed { branch_id } => {
                let branch = self.branch_mut(&branch_id)?;
                if matches!(
                    branch.status,
                    BranchStatus::Running
                        | BranchStatus::Promoted
                        | BranchStatus::AwaitingValidation
                ) {
                    branch.execution_state.agent = AgentExecutionState::Running;
                }
                Ok(vec![])
            }
            AgentMessage::BranchStarted { branch_id } => {
                let branch = self.branch_mut(&branch_id)?;
                // A start fact already in transit may arrive after cancellation
                // or after a duplicate start report. It must not resurrect a
                // discarded Branch or abort Parent fallback.
                if branch.status == BranchStatus::Created {
                    branch.status = BranchStatus::Running;
                    branch.execution_state.agent = AgentExecutionState::Running;
                    for operation in &mut branch.pending_operations {
                        if matches!(
                            operation.kind,
                            OperationKind::StartAgent | OperationKind::StartCandidate
                        ) {
                            operation.status = OperationStatus::Succeeded;
                        }
                    }
                }
                Ok(vec![])
            }
            AgentMessage::ActionFinished {
                task_id,
                branch_id,
                action_id: operation_id,
                result: outcome,
                action_kind,
                status,
                evidence,
            } => {
                if task_id != self.execution_id {
                    return Err(invalid_input("action task does not match tree"));
                }
                self.branch(&branch_id)?;
                let Some(action) = self
                    .active_actions
                    .get(&branch_id)
                    .and_then(|actions| actions.get(&operation_id))
                else {
                    return Ok(vec![]); // Duplicate/late completion cannot advance the Tree.
                };
                if action.action_kind != action_kind {
                    return Err(invalid_input("action kind does not match start"));
                }
                self.active_actions
                    .get_mut(&branch_id)
                    .unwrap()
                    .remove(&operation_id);
                if action_kind != ActionKind::ToolExecution {
                    let branch = self.branch_mut(&branch_id)?;
                    if action_kind == ActionKind::ModelInference
                        && status == ActionStatus::Succeeded
                        && branch.role == BranchRole::Candidate
                        && branch.status == BranchStatus::Running
                    {
                        branch.status = BranchStatus::AwaitingValidation;
                    }
                    return Ok(vec![]);
                }
                self.complete_operation(
                    &branch_id,
                    Some(&operation_id),
                    if status == ActionStatus::Succeeded {
                        OperationStatus::Succeeded
                    } else {
                        OperationStatus::Failed
                    },
                )?;
                for branch in self
                    .branches
                    .values_mut()
                    .filter(|branch| branch.parent_branch_id.as_deref() == Some(&branch_id))
                {
                    for operation in &mut branch.pending_operations {
                        // Runtime evidence is authoritative; an Agent's textual
                        // rendering of that result must not overwrite it.
                        if operation.kind == OperationKind::StartCandidate
                            && operation.operation_id == operation_id
                            && operation.evidence.is_none()
                        {
                            operation.outcome = Some(outcome.clone());
                            operation.evidence = evidence.clone();
                        }
                    }
                }
                Ok(vec![])
            }
            AgentMessage::TurnCompleted { branch_id } => {
                self.branch_mut(&branch_id)?.execution_state.turn_active = false;
                Ok(vec![])
            }
            AgentMessage::Transcript { branch_id, .. } => {
                self.branch_mut(&branch_id)?
                    .execution_position
                    .transcript_index += 1;
                Ok(vec![])
            }
            AgentMessage::Output { branch_id, .. }
            | AgentMessage::Usage { branch_id, .. }
            | AgentMessage::AdapterCapabilityGap { branch_id, .. } => {
                self.branch(&branch_id)?;
                Ok(vec![])
            }
            AgentMessage::ActionStarted {
                task_id,
                branch_id,
                action_id,
                action_kind,
                name,
                parent_action_id,
                ..
            } => {
                if task_id != self.execution_id {
                    return Err(invalid_input("action task does not match tree"));
                }
                let branch = self.branch(&branch_id)?;
                if matches!(
                    branch.status,
                    BranchStatus::Discarding
                        | BranchStatus::Discarded
                        | BranchStatus::Cleaning
                        | BranchStatus::Cleaned
                ) {
                    return Ok(vec![]);
                }
                let actions = self.active_actions.entry(branch_id).or_default();
                if actions.contains_key(&action_id) {
                    return Ok(vec![]);
                }
                actions.insert(
                    action_id,
                    ActionRecord {
                        action_kind,
                        name,
                        parent_action_id,
                    },
                );
                Ok(vec![])
            }
            AgentMessage::SpeculativeAdmissionRequest { .. } => Err(invalid_input(
                "speculative admission must be handled by the coordinator",
            )),
            AgentMessage::BranchReleased { branch_id } => {
                self.active_actions.remove(&branch_id);
                let branch = self.branch_mut(&branch_id)?;
                if branch.execution_state.agent != AgentExecutionState::Released {
                    branch.execution_state.agent = AgentExecutionState::Stopped;
                }
                self.branch_mut(&branch_id)?.execution_state.turn_active = false;
                Ok(vec![])
            }
            AgentMessage::AgentSessionReleased { branch_id } => {
                self.active_actions.remove(&branch_id);
                self.branch_mut(&branch_id)?.execution_state.agent = AgentExecutionState::Released;
                self.branch_mut(&branch_id)?.execution_state.turn_active = false;
                Ok(vec![])
            }
            AgentMessage::BranchFailed { branch_id, reason } => {
                let branch = self.branch(&branch_id)?.clone();
                if matches!(
                    branch.status,
                    BranchStatus::Discarding
                        | BranchStatus::Discarded
                        | BranchStatus::Cleaning
                        | BranchStatus::Cleaned
                ) {
                    return Ok(vec![]);
                }
                self.branch_mut(&branch_id)?.execution_state.agent = AgentExecutionState::Failed;
                self.branch_mut(&branch_id)?.execution_state.turn_active = false;
                self.branch_mut(&branch_id)?.execution_state.failure = Some(reason.clone());
                // A failure of the current authority is not a candidate
                // discard: there is no older authority to fall back to.
                if self.authority_branch_id.as_deref() == Some(branch_id.as_str()) {
                    return Ok(vec![]);
                }
                // Promote may already be in flight. Do not discard until the
                // Agent has answered whether the authority switch happened.
                if branch.status == BranchStatus::Promoting {
                    return Ok(vec![]);
                }
                self.resolve_candidate(&branch_id, false)
            }
            AgentMessage::CommandResult {
                command_id,
                branch_id,
                operation_id,
                operation,
                status,
                generation,
                ..
            } => {
                let Some(pending) = self.pending_commands.get(&command_id).cloned() else {
                    return Ok(vec![]);
                };
                if pending.branch_id != branch_id
                    || pending.operation != operation
                    || pending.operation_id != operation_id
                    || pending.generation != generation
                {
                    return Ok(vec![]);
                }
                if matches!(status, CommandStatus::Accepted | CommandStatus::Started) {
                    return Ok(vec![]);
                }
                self.pending_commands.remove(&command_id);
                let operation_status = if status == CommandStatus::Applied {
                    OperationStatus::Succeeded
                } else {
                    OperationStatus::Failed
                };
                self.complete_operation(&branch_id, Some(&command_id), operation_status)?;
                let branch = self.branch(&branch_id)?.clone();
                match (pending.operation, status) {
                    (CommandOperation::Promote, CommandStatus::Applied)
                        if branch.status == BranchStatus::Promoting =>
                    {
                        let old_authority = self
                            .authority_branch_id
                            .clone()
                            .ok_or_else(|| invalid_input("tree has no authority branch"))?;
                        if old_authority == branch_id {
                            return Ok(vec![]);
                        }
                        self.branch_mut(&branch_id)?.role = BranchRole::Authority;
                        self.branch_mut(&branch_id)?.status = BranchStatus::Promoted;
                        self.authority_branch_id = Some(branch_id.clone());
                        // A model failure that arrived while Promote was in
                        // flight is now a failure of the new authority, not
                        // a reason to discard the candidate.
                        let old = self.branch_mut(&old_authority)?;
                        old.role = BranchRole::Candidate;
                        old.status = BranchStatus::Discarding;
                        let mut commands = vec![self.make_discard_command(&old_authority)?];
                        // Only the confirmed switch invalidates the old authority's other continuations.
                        let obsolete: Vec<_> = self
                            .pending_candidates()
                            .into_iter()
                            .filter(|id| {
                                self.branches[id].parent_branch_id.as_deref()
                                    == Some(&old_authority)
                            })
                            .collect();
                        for id in obsolete {
                            if self.pending_candidates().contains(&id) {
                                commands.extend(self.resolve_candidate(&id, false)?);
                            }
                        }
                        Ok(commands)
                    }
                    (CommandOperation::Promote, _) if branch.status == BranchStatus::Promoting => {
                        self.branch_mut(&branch_id)?.status = BranchStatus::Discarding;
                        Ok(vec![self.make_discard_command(&branch_id)?])
                    }
                    (CommandOperation::Discard, CommandStatus::Applied)
                        if branch.status == BranchStatus::Discarding =>
                    {
                        self.branch_mut(&branch_id)?.execution_state.agent =
                            AgentExecutionState::Stopped;
                        self.branch_mut(&branch_id)?.execution_state.turn_active = false;
                        self.branch_mut(&branch_id)?.status = BranchStatus::Cleaning;
                        Ok(vec![self.make_cleanup_command(&branch_id)?])
                    }
                    (CommandOperation::Cleanup, CommandStatus::Applied)
                        if branch.status == BranchStatus::Cleaning =>
                    {
                        self.branch_mut(&branch_id)?.execution_state.agent =
                            AgentExecutionState::Released;
                        self.branch_mut(&branch_id)?.execution_state.turn_active = false;
                        self.branch_mut(&branch_id)?.status = BranchStatus::Cleaned;
                        // Cleanup is the terminal receipt for this session; no
                        // earlier in-flight command may keep it live or revive it.
                        self.pending_commands
                            .retain(|_, command| command.branch_id != branch_id);
                        for operation in &mut self.branch_mut(&branch_id)?.pending_operations {
                            if matches!(
                                operation.status,
                                OperationStatus::Planned | OperationStatus::Requested
                            ) {
                                operation.status = OperationStatus::Failed;
                            }
                        }
                        Ok(vec![])
                    }
                    _ => Ok(vec![]),
                }
            }
        }
    }

    /// 只有控制面可以接受 Candidate；接受后旧权威失去权威身份。
    pub(crate) fn finish_runtime_promotion(
        &mut self,
        candidate: &str,
        success: bool,
    ) -> io::Result<Vec<AgentCommand>> {
        if success {
            Ok(vec![self.make_promote_command(candidate)?])
        } else {
            self.branch_mut(candidate)?.status = BranchStatus::Discarding;
            Ok(vec![self.make_discard_command(candidate)?])
        }
    }

    pub fn next_deadline(&self) -> Option<u64> {
        if self.stopping {
            return self.shutdown_deadline;
        }
        self.branches
            .values()
            .flat_map(|branch| &branch.pending_operations)
            .filter(|op| {
                if op.kind == OperationKind::StartCandidate {
                    self.pending_candidates().contains(&op.branch_id)
                        && !self
                            .pending_work
                            .values()
                            .any(|work| work.branch() == op.branch_id)
                } else {
                    op.status == OperationStatus::Requested
                }
            })
            .filter_map(|op| op.deadline_at)
            .min()
    }

    pub fn resolve_candidate(
        &mut self,
        branch_id: &str,
        compatible: bool,
    ) -> io::Result<Vec<AgentCommand>> {
        let candidate = self
            .branches
            .get(branch_id)
            .ok_or_else(|| invalid_input("candidate branch does not exist"))?
            .clone();
        if candidate.role != BranchRole::Candidate
            || !(candidate.status == BranchStatus::Running
                || candidate.status == BranchStatus::AwaitingValidation
                || (!compatible && candidate.status == BranchStatus::Created))
        {
            return Err(invalid_input("branch is not a running candidate"));
        }

        if !compatible {
            let mut ids = BTreeSet::from([branch_id.to_string()]);
            loop {
                let before = ids.len();
                for branch in self.branches.values() {
                    if branch
                        .parent_branch_id
                        .as_ref()
                        .is_some_and(|parent| ids.contains(parent))
                    {
                        ids.insert(branch.branch_id.clone());
                    }
                }
                if before == ids.len() {
                    break;
                }
            }
            let mut commands = Vec::new();
            for id in ids {
                if matches!(
                    self.branch(&id)?.status,
                    BranchStatus::Created
                        | BranchStatus::Running
                        | BranchStatus::AwaitingValidation
                ) {
                    self.branch_mut(&id)?.status = BranchStatus::Discarding;
                    commands.push(self.make_discard_command(&id)?);
                }
            }
            return Ok(commands);
        }

        let old_authority = self
            .authority_branch_id
            .clone()
            .ok_or_else(|| invalid_input("tree has no authority branch"))?;
        if candidate.parent_branch_id.as_deref() != Some(&old_authority) {
            return Err(invalid_input(
                "candidate is not a child of current authority",
            ));
        }
        if self
            .branches
            .values()
            .any(|branch| branch.status == BranchStatus::Promoting)
        {
            return Err(invalid_input("a promotion is already in progress"));
        }
        if !matches!(
            self.branch(&old_authority)?.status,
            BranchStatus::Running | BranchStatus::Promoted
        ) {
            return Err(invalid_input("current authority is not live"));
        }
        let promoted = self.branch_mut(branch_id)?;
        promoted.status = BranchStatus::Promoting;
        Ok(vec![self.make_promote_command(branch_id)?])
    }

    fn insert_branch(
        &mut self,
        branch_id: &str,
        parent_branch_id: Option<String>,
        role: BranchRole,
        status: BranchStatus,
    ) -> io::Result<()> {
        if self.branches.contains_key(branch_id) {
            return Err(invalid_input("branch id already exists"));
        }
        let generation = self.next_generation;
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .ok_or_else(|| invalid_input("generation counter exhausted"))?;
        self.branches.insert(
            branch_id.to_string(),
            BranchRecord {
                branch_id: branch_id.to_string(),
                parent_branch_id,
                role,
                status,
                generation,
                execution_position: Default::default(),
                proposal_id: None,
                assumption_set: Vec::new(),
                pending_operations: Vec::new(),
                execution_state: Default::default(),
            },
        );
        Ok(())
    }

    fn branch(&self, branch_id: &str) -> io::Result<&BranchRecord> {
        self.branches
            .get(branch_id)
            .ok_or_else(|| invalid_input("branch does not exist"))
    }

    fn branch_mut(&mut self, branch_id: &str) -> io::Result<&mut BranchRecord> {
        self.branches
            .get_mut(branch_id)
            .ok_or_else(|| invalid_input("branch does not exist"))
    }

    fn command_id(&mut self, operation: CommandOperation) -> io::Result<String> {
        let id = format!(
            "command-{}-{}",
            self.next_command_id,
            operation_name(operation)
        );
        self.next_command_id = self
            .next_command_id
            .checked_add(1)
            .ok_or_else(|| invalid_input("command counter exhausted"))?;
        Ok(id)
    }

    fn register(
        &mut self,
        branch_id: &str,
        operation: CommandOperation,
        operation_id: Option<String>,
        generation: u64,
    ) -> io::Result<String> {
        self.branch(branch_id)?;
        let id = self.command_id(operation)?;
        self.pending_commands.insert(
            id.clone(),
            PendingCommand {
                branch_id: branch_id.to_string(),
                operation,
                operation_id,
                generation,
            },
        );
        let kind = match operation {
            CommandOperation::Promote => OperationKind::Promote,
            CommandOperation::Discard => OperationKind::Discard,
            CommandOperation::Cleanup => OperationKind::Cleanup,
            CommandOperation::ResolvePending => OperationKind::ResolvePending,
        };
        self.mark_operation_requested(branch_id, &id, kind)?;
        Ok(id)
    }

    fn make_promote_command(&mut self, branch_id: &str) -> io::Result<AgentCommand> {
        let generation = self.branch(branch_id)?.generation;
        let command_id = self.register(branch_id, CommandOperation::Promote, None, generation)?;
        Ok(AgentCommand::Promote {
            branch_id: branch_id.to_string(),
            command_id,
            generation,
        })
    }

    fn make_discard_command(&mut self, branch_id: &str) -> io::Result<AgentCommand> {
        let generation = self.branch(branch_id)?.generation;
        let command_id = self.register(branch_id, CommandOperation::Discard, None, generation)?;
        Ok(AgentCommand::Discard {
            branch_id: branch_id.to_string(),
            command_id,
            generation,
        })
    }

    fn make_cleanup_command(&mut self, branch_id: &str) -> io::Result<AgentCommand> {
        let generation = self.branch(branch_id)?.generation;
        let command_id = self.register(branch_id, CommandOperation::Cleanup, None, generation)?;
        Ok(AgentCommand::Cleanup {
            branch_id: branch_id.to_string(),
            workspace: self.workspace(),
            command_id,
            generation,
        })
    }
}

fn operation_name(operation: CommandOperation) -> &'static str {
    match operation {
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

pub use crate::coordination::AgentController;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn authority_continuation_waits_for_promotion_and_prunes_only_after_confirmation() {
        let mut tree = tree();
        tree.workspace_root =
            std::env::temp_dir().join(format!("ascout-continuation-{}", std::process::id()));
        tree.create_authority("authority", "").unwrap();
        tree.handle_message(AgentMessage::BranchStarted {
            branch_id: "authority".into(),
        })
        .unwrap();
        let first = tree
            .start_candidate("authority", "op", "tool", json!("first"), None)
            .unwrap();
        let AgentCommand::StartCandidate {
            branch_id: winner, ..
        } = first
        else {
            unreachable!()
        };
        let second = tree
            .start_candidate("authority", "op", "tool", json!("second"), None)
            .unwrap();
        let AgentCommand::StartCandidate {
            branch_id: sibling, ..
        } = second
        else {
            unreachable!()
        };
        for id in [&winner, &sibling] {
            tree.handle_message(AgentMessage::BranchStarted {
                branch_id: id.clone(),
            })
            .unwrap();
        }
        tree.handle_message(AgentMessage::TurnCompleted {
            branch_id: winner.clone(),
        })
        .unwrap();
        let commands = tree.resolve_candidate(&winner, true).unwrap();
        assert_eq!(tree.authority_branch_id.as_deref(), Some("authority"));
        assert_eq!(tree.branches[&sibling].status, BranchStatus::Running);
        tree.queue_prompt("next".into());
        assert!(tree.next_prompt().unwrap().is_none());
        let AgentCommand::Promote {
            command_id,
            generation,
            ..
        } = &commands[0]
        else {
            unreachable!()
        };
        let commands = tree
            .handle_message(AgentMessage::CommandResult {
                command_id: command_id.clone(),
                branch_id: winner.clone(),
                operation_id: None,
                operation: CommandOperation::Promote,
                status: CommandStatus::Applied,
                generation: *generation,
                reason: None,
            })
            .unwrap();
        assert!(commands.iter().any(|command| matches!(command, AgentCommand::Discard { branch_id, .. } if branch_id == &sibling)));
        assert!(
            matches!(tree.next_prompt().unwrap(), Some(AgentCommand::Prompt { branch_id, .. }) if branch_id == winner)
        );
        assert!(tree.authority_running());
        assert!(
            tree.branches[&winner]
                .pending_operations
                .iter()
                .all(
                    |operation| operation.requested_at.is_some() && operation.deadline_at.is_some()
                )
        );
    }

    fn tree() -> SpeculativeTree {
        let mut tree = SpeculativeTree::new("agent-1".into());
        tree.workspace_root = PathBuf::from("/memory");
        tree
    }

    #[test]
    fn candidate_shares_run_workspace_and_promotes() {
        let mut tree = tree();
        tree.branches.insert(
            "parent".into(),
            BranchRecord {
                branch_id: "parent".into(),
                parent_branch_id: None,
                role: BranchRole::Authority,
                status: BranchStatus::Running,
                generation: 1,
                execution_position: Default::default(),
                proposal_id: None,
                assumption_set: Vec::new(),
                pending_operations: Vec::new(),
                execution_state: Default::default(),
            },
        );
        tree.branches.insert(
            "candidate".into(),
            BranchRecord {
                branch_id: "candidate".into(),
                parent_branch_id: Some("parent".into()),
                role: BranchRole::Candidate,
                status: BranchStatus::Running,
                generation: 2,

                execution_position: Default::default(),
                proposal_id: None,
                assumption_set: Vec::new(),
                pending_operations: Vec::new(),
                execution_state: Default::default(),
            },
        );
        tree.authority_branch_id = Some("parent".into());
        let commands = tree.resolve_candidate("candidate", true).unwrap();
        assert!(
            commands
                .iter()
                .any(|command| matches!(command, AgentCommand::Promote { .. }))
        );
        assert_eq!(tree.authority_branch_id.as_deref(), Some("parent"));
        assert_eq!(tree.branches["parent"].status, BranchStatus::Running);
        let promote = commands
            .into_iter()
            .find_map(|command| match command {
                AgentCommand::Promote {
                    command_id,
                    generation,
                    ..
                } => Some((command_id, generation)),
                _ => None,
            })
            .unwrap();
        let commands = tree
            .handle_message(AgentMessage::CommandResult {
                command_id: promote.0,
                branch_id: "candidate".into(),
                operation_id: None,
                operation: CommandOperation::Promote,
                status: CommandStatus::Applied,
                generation: promote.1,
                reason: None,
            })
            .unwrap();
        assert!(
            commands
                .iter()
                .any(|command| matches!(command, AgentCommand::Discard { .. }))
        );
        assert_eq!(tree.authority_branch_id.as_deref(), Some("candidate"));
    }

    #[test]
    fn mismatch_discards_and_cleans_candidate() {
        let mut tree = tree();
        tree.branches.insert(
            "parent".into(),
            BranchRecord {
                branch_id: "parent".into(),
                parent_branch_id: None,
                role: BranchRole::Authority,
                status: BranchStatus::Running,
                generation: 1,
                execution_position: Default::default(),
                proposal_id: None,
                assumption_set: Vec::new(),
                pending_operations: Vec::new(),
                execution_state: Default::default(),
            },
        );
        tree.branches.insert(
            "candidate".into(),
            BranchRecord {
                branch_id: "candidate".into(),
                parent_branch_id: Some("parent".into()),
                role: BranchRole::Candidate,
                status: BranchStatus::Running,
                generation: 2,

                execution_position: Default::default(),
                proposal_id: None,
                assumption_set: Vec::new(),
                pending_operations: Vec::new(),
                execution_state: Default::default(),
            },
        );
        tree.authority_branch_id = Some("parent".into());
        let commands = tree.resolve_candidate("candidate", false).unwrap();
        assert!(
            commands
                .iter()
                .any(|command| matches!(command, AgentCommand::Discard { .. }))
        );
        let discard = commands
            .into_iter()
            .find_map(|command| match command {
                AgentCommand::Discard {
                    command_id,
                    generation,
                    ..
                } => Some((command_id, generation)),
                _ => None,
            })
            .unwrap();
        let commands = tree
            .handle_message(AgentMessage::CommandResult {
                command_id: discard.0,
                branch_id: "candidate".into(),
                operation_id: None,
                operation: CommandOperation::Discard,
                status: CommandStatus::Applied,
                generation: discard.1,
                reason: None,
            })
            .unwrap();
        let cleanup = commands
            .into_iter()
            .find_map(|command| match command {
                AgentCommand::Cleanup {
                    command_id,
                    generation,
                    ..
                } => Some((command_id, generation)),
                _ => None,
            })
            .unwrap();
        tree.handle_message(AgentMessage::CommandResult {
            command_id: cleanup.0,
            branch_id: "candidate".into(),
            operation_id: None,
            operation: CommandOperation::Cleanup,
            status: CommandStatus::Applied,
            generation: cleanup.1,
            reason: None,
        })
        .unwrap();
        assert_eq!(tree.branches["candidate"].status, BranchStatus::Cleaned);
    }

    #[test]
    fn command_results_require_exact_pending_identity_and_are_consumed() {
        let mut tree = tree();
        tree.branches.insert(
            "parent".into(),
            BranchRecord {
                branch_id: "parent".into(),
                parent_branch_id: None,
                role: BranchRole::Authority,
                status: BranchStatus::Running,
                generation: 1,
                execution_position: Default::default(),
                proposal_id: None,
                assumption_set: Vec::new(),
                pending_operations: Vec::new(),
                execution_state: Default::default(),
            },
        );
        tree.branches.insert(
            "candidate".into(),
            BranchRecord {
                branch_id: "candidate".into(),
                parent_branch_id: Some("parent".into()),
                role: BranchRole::Candidate,
                status: BranchStatus::Running,
                generation: 2,

                execution_position: Default::default(),
                proposal_id: None,
                assumption_set: Vec::new(),
                pending_operations: Vec::new(),
                execution_state: Default::default(),
            },
        );
        tree.authority_branch_id = Some("parent".into());
        let command = tree.resolve_candidate("candidate", true).unwrap().remove(0);
        let (command_id, generation) = match command {
            AgentCommand::Promote {
                command_id,
                generation,
                ..
            } => (command_id, generation),
            _ => unreachable!(),
        };
        let stale = tree
            .handle_message(AgentMessage::CommandResult {
                command_id: command_id.clone(),
                branch_id: "candidate".into(),
                operation_id: None,
                operation: CommandOperation::Promote,
                status: CommandStatus::Applied,
                generation: generation + 1,
                reason: None,
            })
            .unwrap();
        assert!(stale.is_empty());
        assert_eq!(tree.authority_branch_id.as_deref(), Some("parent"));
        tree.handle_message(AgentMessage::CommandResult {
            command_id: command_id.clone(),
            branch_id: "candidate".into(),
            operation_id: None,
            operation: CommandOperation::Promote,
            status: CommandStatus::Applied,
            generation,
            reason: None,
        })
        .unwrap();
        let duplicate = tree
            .handle_message(AgentMessage::CommandResult {
                command_id,
                branch_id: "candidate".into(),
                operation_id: None,
                operation: CommandOperation::Promote,
                status: CommandStatus::Applied,
                generation,
                reason: None,
            })
            .unwrap();
        assert!(duplicate.is_empty());
        assert_eq!(tree.authority_branch_id.as_deref(), Some("candidate"));
    }

    #[test]
    fn failed_promotion_discards_without_switching_authority() {
        let mut tree = tree();
        tree.branches.insert(
            "parent".into(),
            BranchRecord {
                branch_id: "parent".into(),
                parent_branch_id: None,
                role: BranchRole::Authority,
                status: BranchStatus::Running,
                generation: 1,
                execution_position: Default::default(),
                proposal_id: None,
                assumption_set: Vec::new(),
                pending_operations: Vec::new(),
                execution_state: Default::default(),
            },
        );
        tree.branches.insert(
            "candidate".into(),
            BranchRecord {
                branch_id: "candidate".into(),
                parent_branch_id: Some("parent".into()),
                role: BranchRole::Candidate,
                status: BranchStatus::Running,
                generation: 2,

                execution_position: Default::default(),
                proposal_id: None,
                assumption_set: Vec::new(),
                pending_operations: Vec::new(),
                execution_state: Default::default(),
            },
        );
        tree.authority_branch_id = Some("parent".into());
        let command = tree.resolve_candidate("candidate", true).unwrap().remove(0);
        let (command_id, generation) = match command {
            AgentCommand::Promote {
                command_id,
                generation,
                ..
            } => (command_id, generation),
            _ => unreachable!(),
        };
        let commands = tree
            .handle_message(AgentMessage::CommandResult {
                command_id,
                branch_id: "candidate".into(),
                operation_id: None,
                operation: CommandOperation::Promote,
                status: CommandStatus::Failed,
                generation,
                reason: Some("failed".into()),
            })
            .unwrap();
        assert!(
            commands
                .iter()
                .any(|command| matches!(command, AgentCommand::Discard { .. }))
        );
        assert_eq!(tree.authority_branch_id.as_deref(), Some("parent"));
        assert_eq!(tree.branches["candidate"].status, BranchStatus::Discarding);
    }

    #[test]
    fn branch_failure_during_promotion_waits_for_promote_result() {
        let mut tree = tree();
        tree.branches.insert(
            "parent".into(),
            BranchRecord {
                branch_id: "parent".into(),
                parent_branch_id: None,
                role: BranchRole::Authority,
                status: BranchStatus::Running,
                generation: 1,
                execution_position: Default::default(),
                proposal_id: None,
                assumption_set: Vec::new(),
                pending_operations: Vec::new(),
                execution_state: Default::default(),
            },
        );
        tree.branches.insert(
            "candidate".into(),
            BranchRecord {
                branch_id: "candidate".into(),
                parent_branch_id: Some("parent".into()),
                role: BranchRole::Candidate,
                status: BranchStatus::Running,
                generation: 2,

                execution_position: Default::default(),
                proposal_id: None,
                assumption_set: Vec::new(),
                pending_operations: Vec::new(),
                execution_state: Default::default(),
            },
        );
        tree.authority_branch_id = Some("parent".into());
        let command = tree.resolve_candidate("candidate", true).unwrap().remove(0);
        let (command_id, generation) = match command {
            AgentCommand::Promote {
                command_id,
                generation,
                ..
            } => (command_id, generation),
            _ => unreachable!(),
        };

        let commands = tree
            .handle_message(AgentMessage::BranchFailed {
                branch_id: "candidate".into(),
                reason: "429".into(),
            })
            .unwrap();

        assert!(commands.is_empty());
        assert_eq!(tree.branches["candidate"].status, BranchStatus::Promoting);
        assert_eq!(tree.authority_branch_id.as_deref(), Some("parent"));
        assert!(tree.authority_failure().is_none());

        let commands = tree
            .handle_message(AgentMessage::CommandResult {
                command_id,
                branch_id: "candidate".into(),
                operation_id: None,
                operation: CommandOperation::Promote,
                status: CommandStatus::Applied,
                generation,
                reason: None,
            })
            .unwrap();

        assert_eq!(tree.authority_branch_id.as_deref(), Some("candidate"));
        assert_eq!(tree.branches["candidate"].status, BranchStatus::Promoted);
        assert!(tree.authority_failure().is_some());
        assert!(commands.iter().any(|command| matches!(
            command,
            AgentCommand::Discard { branch_id, .. } if branch_id == "parent"
        )));
    }

    #[test]
    fn late_promote_result_after_failed_promotion_is_ignored() {
        let mut tree = tree();
        tree.branches.insert(
            "parent".into(),
            BranchRecord {
                branch_id: "parent".into(),
                parent_branch_id: None,
                role: BranchRole::Authority,
                status: BranchStatus::Running,
                generation: 1,
                execution_position: Default::default(),
                proposal_id: None,
                assumption_set: Vec::new(),
                pending_operations: Vec::new(),
                execution_state: Default::default(),
            },
        );
        tree.branches.insert(
            "candidate".into(),
            BranchRecord {
                branch_id: "candidate".into(),
                parent_branch_id: Some("parent".into()),
                role: BranchRole::Candidate,
                status: BranchStatus::Running,
                generation: 2,

                execution_position: Default::default(),
                proposal_id: None,
                assumption_set: Vec::new(),
                pending_operations: Vec::new(),
                execution_state: Default::default(),
            },
        );
        tree.authority_branch_id = Some("parent".into());
        let command = tree.resolve_candidate("candidate", true).unwrap().remove(0);
        let (command_id, generation) = match command {
            AgentCommand::Promote {
                command_id,
                generation,
                ..
            } => (command_id, generation),
            _ => unreachable!(),
        };
        tree.handle_message(AgentMessage::BranchFailed {
            branch_id: "candidate".into(),
            reason: "429".into(),
        })
        .unwrap();
        tree.handle_message(AgentMessage::CommandResult {
            command_id: command_id.clone(),
            branch_id: "candidate".into(),
            operation_id: None,
            operation: CommandOperation::Promote,
            status: CommandStatus::Failed,
            generation,
            reason: Some("promote rejected".into()),
        })
        .unwrap();
        assert_eq!(tree.branches["candidate"].status, BranchStatus::Discarding);
        assert_eq!(tree.authority_branch_id.as_deref(), Some("parent"));

        let commands = tree
            .handle_message(AgentMessage::CommandResult {
                command_id,
                branch_id: "candidate".into(),
                operation_id: None,
                operation: CommandOperation::Promote,
                status: CommandStatus::Applied,
                generation,
                reason: None,
            })
            .unwrap();

        assert!(commands.is_empty());
        assert_eq!(tree.branches["candidate"].status, BranchStatus::Discarding);
        assert_eq!(tree.authority_branch_id.as_deref(), Some("parent"));
        assert!(tree.authority_failure().is_none());
    }

    #[test]
    fn authority_branch_failure_fails_the_run_without_discard() {
        let mut tree = tree();
        tree.branches.insert(
            "authority".into(),
            BranchRecord {
                branch_id: "authority".into(),
                parent_branch_id: None,
                role: BranchRole::Authority,
                status: BranchStatus::Running,
                generation: 1,
                execution_position: Default::default(),
                proposal_id: None,
                assumption_set: Vec::new(),
                pending_operations: Vec::new(),
                execution_state: Default::default(),
            },
        );
        tree.authority_branch_id = Some("authority".into());

        let commands = tree
            .handle_message(AgentMessage::BranchFailed {
                branch_id: "authority".into(),
                reason: "429".into(),
            })
            .unwrap();

        assert!(commands.is_empty());
        assert_eq!(tree.branches["authority"].status, BranchStatus::Running);
        assert!(tree.authority_failure().is_some());
    }

    #[test]
    fn late_or_duplicate_branch_started_is_ignored() {
        let mut tree = tree();
        for status in [
            BranchStatus::Running,
            BranchStatus::Discarding,
            BranchStatus::Discarded,
            BranchStatus::Cleaning,
            BranchStatus::Cleaned,
        ] {
            tree.branches.insert(
                "candidate".into(),
                BranchRecord {
                    branch_id: "candidate".into(),
                    parent_branch_id: None,
                    role: BranchRole::Candidate,
                    status,
                    generation: 1,

                    execution_position: Default::default(),
                    proposal_id: None,
                    assumption_set: Vec::new(),
                    pending_operations: Vec::new(),
                    execution_state: Default::default(),
                },
            );
            let commands = tree
                .handle_message(AgentMessage::BranchStarted {
                    branch_id: "candidate".into(),
                })
                .unwrap();
            assert!(commands.is_empty());
            assert_eq!(tree.branches["candidate"].status, status);
        }
    }

    #[test]
    fn candidate_completion_only_marks_readiness_and_cleanup_failure_is_not_cleaned() {
        let mut tree = tree();
        tree.branches.insert(
            "candidate".into(),
            BranchRecord {
                branch_id: "candidate".into(),
                parent_branch_id: None,
                role: BranchRole::Candidate,
                status: BranchStatus::Running,
                generation: 1,
                execution_position: Default::default(),
                proposal_id: None,
                assumption_set: Vec::new(),
                pending_operations: Vec::new(),
                execution_state: Default::default(),
            },
        );
        tree.handle_message(AgentMessage::ActionStarted {
            task_id: tree.execution_id.clone(),
            branch_id: "candidate".into(),
            action_id: "inference".into(),
            action_kind: ActionKind::ModelInference,
            parent_action_id: None,
            result_pending: false,
            name: "model".into(),
            arguments: Value::Null,
            checkpoint: None,
        })
        .unwrap();
        tree.handle_message(AgentMessage::ActionFinished {
            task_id: tree.execution_id.clone(),
            branch_id: "candidate".into(),
            action_id: "inference".into(),
            action_kind: ActionKind::ModelInference,
            status: ActionStatus::Succeeded,
            result: Value::Null,
            evidence: None,
        })
        .unwrap();
        assert_eq!(
            tree.branches["candidate"].status,
            BranchStatus::AwaitingValidation
        );
        tree.branches.get_mut("candidate").unwrap().status = BranchStatus::Cleaning;
        let command = tree.make_cleanup_command("candidate").unwrap();
        let (command_id, generation) = match command {
            AgentCommand::Cleanup {
                command_id,
                generation,
                ..
            } => (command_id, generation),
            _ => unreachable!(),
        };
        tree.handle_message(AgentMessage::CommandResult {
            command_id,
            branch_id: "candidate".into(),
            operation_id: None,
            operation: CommandOperation::Cleanup,
            status: CommandStatus::Failed,
            generation,
            reason: Some("busy".into()),
        })
        .unwrap();
        assert_eq!(tree.branches["candidate"].status, BranchStatus::Cleaning);
        let command = tree.begin_cleanup("candidate").unwrap();
        let AgentCommand::Cleanup {
            command_id,
            generation,
            ..
        } = command
        else {
            unreachable!()
        };
        tree.handle_message(AgentMessage::CommandResult {
            command_id,
            branch_id: "candidate".into(),
            operation_id: None,
            operation: CommandOperation::Cleanup,
            status: CommandStatus::Applied,
            generation,
            reason: None,
        })
        .unwrap();
        assert_eq!(
            tree.branches["candidate"].execution_state.agent,
            AgentExecutionState::Released
        );
        // A late start fact must not resurrect a released Agent session.
        tree.handle_message(AgentMessage::BranchStarted {
            branch_id: "candidate".into(),
        })
        .unwrap();
        assert_eq!(
            tree.branches["candidate"].execution_state.agent,
            AgentExecutionState::Released
        );
        assert_eq!(
            tree.branches["candidate"].execution_state.runtime,
            RuntimeExecutionState::Unknown
        );
    }
}
