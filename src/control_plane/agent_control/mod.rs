//! AScout 控制面管理一个 Agent 的最小状态与 Agent 接入协议。
//!
//! 这里的状态属于控制面。Pi 接入侧只能报告事实并执行返回的命令，不能以
//! 自己的对象保存或裁决 Speculative Tree。

mod candidates;
mod chances;
mod commands;
mod protocol;
mod settlement;
mod tree;

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

/// Branch 暂时不能前进的原因，同一时刻只记最近挡住它的 Decision。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockedReason {
    ExtentReached,
    Rebalanced,
    EffectGate,
}

/// 控制面已知的 Branch 生命周期与运行状态。`Finished` 表示已经跑完、等待裁决。
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
    /// Candidate 仍在参与投机：尚未进入晋级、丢弃或清理。
    pub fn is_live_candidate(self) -> bool {
        matches!(
            self,
            Self::Created | Self::Running | Self::Blocked(_) | Self::Finished
        )
    }

    /// Candidate 可以被裁决接受：没有被 extent 或 rebalance 挡住。
    pub fn is_decidable(self) -> bool {
        matches!(
            self,
            Self::Running | Self::Finished | Self::Blocked(BlockedReason::EffectGate)
        )
    }

    /// 权威 Branch 仍持有权威路径；`Blocked` 只是暂时停住它的下一步，
    /// 不是它失效。
    pub fn is_live_authority(self) -> bool {
        matches!(self, Self::Running | Self::Promoted | Self::Blocked(_))
    }

    /// 已经进入丢弃或之后的阶段。
    pub fn is_discarding_or_later(self) -> bool {
        matches!(
            self,
            Self::Discarding | Self::Discarded | Self::Cleaning | Self::Cleaned
        )
    }
}

/// Chance 的阶段与结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChanceStage {
    /// 已 `admit`，等待 `select`。
    Pending,
    /// 已被选中：预测、执行或取权威值中。
    Running,
    /// 权威值已到，`match` 与处置中。
    Resolving,
    /// 某个 Proposal 被接受。
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

/// 权威值的取得状态，属于 Chance，与 `Running` 并行，由 `launch` 改变。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AuthorityState {
    #[default]
    NotRequested,
    Fetching,
    Arrived,
    Unavailable,
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
    #[serde(default)]
    pub environment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_branch_id: Option<String>,
    pub role: BranchRole,
    pub status: BranchStatus,
    #[serde(default)]
    pub generation: u64,
    #[serde(default)]
    pub execution_position: ExecutionPosition,
    /// 这个 Branch 所属的 Chance；Proposal 内容和验证依据都在 Chance 上。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chance_id: Option<String>,
    /// 这个 Branch 携带的未确认前提，含祖先路径上的前提；自身的前提是
    /// `chance_id` 对应的那一项。
    #[serde(default)]
    pub assumption_set: Vec<Assumption>,
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

/// AssumptionSet 的一项：引用某个 Chance 上的某个 Proposal。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Assumption {
    pub chance_id: String,
    pub proposal_id: String,
}

/// Chance 上不可变的候选输入。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposalRecord {
    pub proposal_id: String,
    pub payload: Value,
}

/// 被 `admit` 接纳的投机机会，从创建到收敛一直由控制面持有。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChanceRecord {
    pub chance_id: String,
    /// 等待结果的 Branch 与它的 operation。
    pub parent_branch_id: String,
    /// 父 Branch 自身所属的 Chance（嵌套关系）；父是初始权威时为空。
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
    /// 验证依据：权威侧真实结果和 Runtime 证据，属于 Chance，不属于任何 Branch。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authority_outcome: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authority_evidence: Option<Value>,
    /// 产生这次投机的准入请求时刻。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speculation_started_at_ms: Option<u64>,
    /// 验证通过并用于当前 Promote 尝试的工作 id。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) validation_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speculation_kind: Option<ActionKind>,
    /// 最近一次 `launch` 求值时的 Tree 版本；版本不变就不重复求值。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_epoch: Option<u64>,
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
    pub requested_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline_at: Option<u64>,
}

/// 一次已准入投机的内部记录。结算只需要投机起点、类型和是否已经结算；
/// 它不进入公共事件，BranchRemoved 时随之释放。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SpeculationRecord {
    pub started_at_ms: u64,
    pub kind: Option<ActionKind>,
    pub settled: bool,
}

/// 一次模型调用在控制面侧保留的用量事实。用于结算比较；公共展示仍由
/// `model_usage` 观察事件表达。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ModelUsageFact {
    /// 这次调用对应的模型 action；接入侧无法配对时为 None，不伪造身份。
    #[serde(default)]
    pub action_id: Option<String>,
    pub model: String,
    pub started_at_ms: u64,
    pub finished_at_ms: u64,
    pub duration_ms: Option<u64>,
    pub cost: Option<f64>,
}

/// Branch 对应的 Agent 与 Runtime 实际事实。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BranchExecutionState {
    pub agent: AgentExecutionState,
    pub runtime: RuntimeExecutionState,
    #[serde(default)]
    pub turn_active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
}

/// 一个 Agent 的完整投机树视图。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpeculativeTree {
    pub agent_id: String,
    #[serde(default)]
    pub execution_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_root_branch_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authority_branch_id: Option<String>,
    pub branches: BTreeMap<String, BranchRecord>,
    /// 尚未收敛的 Chance；Proposal 和验证依据都挂在这里。
    #[serde(default)]
    pub chances: BTreeMap<String, ChanceRecord>,
    /// Only live actions are retained; completed facts belong to the Ledger.
    #[serde(default)]
    pub active_actions: BTreeMap<String, BTreeMap<String, ActionRecord>>,
    #[serde(default)]
    pub(crate) action_facts: BTreeMap<String, BTreeMap<String, ActionFinishedFact>>,
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
    pub(crate) pending_work: BTreeMap<u64, crate::control_plane::coordination::ControlOperation>,
    /// 已准入投机的内部事实，按 Branch 保存；不进入 Branch 归档。
    #[serde(default)]
    pub(crate) speculations: BTreeMap<String, SpeculationRecord>,
    /// 模型用量事实，按 Branch 保存每次调用，供结算比较。无法配对 action
    /// 的调用也保留，不因为缺少 action_id 而丢弃。
    #[serde(default)]
    pub(crate) model_usage: BTreeMap<String, Vec<ModelUsageFact>>,
    /// 工具 `ActionFinished` 的接入侧时刻，按 Branch 和 action 保存。
    #[serde(default)]
    pub(crate) tool_finished_at_ms: BTreeMap<String, BTreeMap<String, u64>>,
    #[serde(default)]
    pub(crate) next_work: u64,
    #[serde(default = "first_id")]
    next_chance: u64,
    /// 每次 Branch 或 Chance 状态变化加一；`launch`、`rebalance` 只在版本
    /// 变化后重新求值，避免空转。
    #[serde(default)]
    pub(crate) epoch: u64,
    #[serde(default)]
    pub(crate) rebalance_epoch: Option<u64>,
    #[serde(default)]
    pub(crate) stopping: bool,
    #[serde(default)]
    pub(crate) recovering: bool,
    #[serde(default)]
    pub(crate) task_finishing: bool,
    #[serde(default)]
    pub(crate) recovery_paused_at: Option<u64>,
    #[serde(default)]
    shutdown_deadline: Option<u64>,
    #[serde(default = "first_id")]
    next_generation: u64,
    #[serde(default = "first_id")]
    next_command_id: u64,
    #[serde(skip)]
    pending_observations: Vec<crate::control_plane::observation::ObservationContent>,
    #[serde(skip)]
    pending_records: Vec<Value>,
    /// The control plane's logical transcript, retained independently of
    /// native Agent session files.
    #[serde(default)]
    pub(crate) transcripts: BTreeMap<String, Vec<Value>>,
    /// Runtime launch template used to derive one Agent process per branch.
    #[serde(default)]
    pub(crate) agent_launch: Option<crate::protocol::contract::AgentLaunch>,
}

impl SpeculativeTree {
    pub fn new(agent_id: String) -> Self {
        Self {
            agent_id,
            execution_id: String::new(),
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
            pending_work: BTreeMap::new(),
            speculations: BTreeMap::new(),
            model_usage: BTreeMap::new(),
            tool_finished_at_ms: BTreeMap::new(),
            next_work: 1,
            next_chance: 1,
            epoch: 0,
            rebalance_epoch: None,
            stopping: false,
            recovering: false,
            task_finishing: false,
            recovery_paused_at: None,
            shutdown_deadline: None,
            next_generation: 1,
            next_command_id: 1,
            pending_observations: Vec::new(),
            pending_records: Vec::new(),
            transcripts: BTreeMap::new(),
            agent_launch: None,
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
fn deserialize_notification<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    let text = String::deserialize(deserializer)?;
    if text.trim().is_empty() {
        return Err(serde::de::Error::custom("notification must be nonblank"));
    }
    Ok(Some(text))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum AgentCommand {
    Model {
        branch_id: String,
        model: String,
    },
    SuspendRecovery {
        branch_id: String,
    },
    CancelSave {
        branch_id: String,
    },
    Save {
        branch_id: String,
        path: PathBuf,
    },
    Load {
        branch_id: String,
        path: PathBuf,
        environments: BTreeMap<String, String>,
    },
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
    /// `rebalance` 的 `pause`：Agent Adapter 在下一个边界停住该 Branch，
    /// 并以 `branch_suspended` 报告。
    Pause {
        branch_id: String,
    },
    /// `rebalance` 的 `resume`：放行被停住的 Branch，并以 `branch_resumed` 报告。
    Resume {
        branch_id: String,
    },
    StartCandidate {
        branch_id: String,
        environment_id: Option<String>,
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
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        history: Vec<Value>,
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
        environment_id: Option<String>,
        role: BranchRole,
        workspace: PathBuf,
        prompt: String,
    },
    Promote {
        branch_id: String,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "deserialize_notification"
        )]
        notification: Option<String>,
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
    TurnFailed {
        branch_id: String,
        reason: String,
    },
    ModelChanged {
        branch_id: String,
        model: String,
        error: Option<String>,
    },
    RecoverySuspended,
    RecoverySaved {
        path: PathBuf,
        error: Option<String>,
    },
    RecoveryLoaded {
        error: Option<String>,
    },
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
        name: String,
        arguments: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        checkpoint: Option<Value>,
        /// 接入侧测量的事件时刻（Unix 毫秒），不是控制面收到消息的时刻。
        timestamp_ms: u64,
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
        /// 投机开始的接入侧测量时刻（Unix 毫秒）。
        timestamp_ms: u64,
    },
    Transcript {
        branch_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_entry_id: Option<String>,
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
        /// 接入侧观察到的结果到达时刻（Unix 毫秒）。
        timestamp_ms: u64,
    },
    Usage {
        branch_id: String,
        /// 这次调用对应的模型 action。接入侧确实无法配对时为 None：仍然上报
        /// 用量，但不伪造 action 身份。
        #[serde(default)]
        action_id: Option<String>,
        model: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thinking_level: Option<String>,
        usage: Value,
        /// 接入侧测量的模型请求开始与结束时刻（Unix 毫秒）。
        started_at_ms: u64,
        finished_at_ms: u64,
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
        reason: BlockedReason,
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

pub use crate::control_plane::coordination::AgentController;

#[cfg(test)]
mod tests;
