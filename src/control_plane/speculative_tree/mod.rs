//! AScout control plane 管理一个 Agent 的投机状态及其合法转换。
//!
//! 这里的状态属于 control plane。Pi integration 只能报告事实并执行返回的命令，不能以
//! 自己的对象保存或裁决 Speculative Tree。

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

/// control plane 已知的 Branch 生命周期与运行状态。`Finished` 表示已经跑完、等待裁决。
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

/// control plane 操作的种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    Abort,
    Tool,
    StartAgent,
    StartCandidate,
    ResolvePending,
    Promote,
    Discard,
    Cleanup,
}

/// control plane 操作的落实状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationStatus {
    Planned,
    Requested,
    Succeeded,
    Failed,
}

/// Instrumentation 报告的实际状态。
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

/// control plane 保存的一条 Branch 记录。
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
    /// 这个 Branch 所属的 Chance；Proposal 内容和验证依据都在 Chance 上。
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

/// 被 `admit` 接纳的投机机会，从创建到收敛一直由 control plane 持有。
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

/// control plane 希望发生的操作，不代表外部操作已经完成。
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

/// 一次模型调用在 control plane 侧保留的用量事实。用于结算比较；公共展示仍由
/// `model_usage` 观察事件表达。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ModelUsageFact {
    /// 这次调用对应的模型 action；Instrumentation 无法配对时为 None，不伪造身份。
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
    /// 尚未收敛的 Chance；Proposal 和验证依据都挂在这里。
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
    /// Digest of the local Policy artifact delivered to the Agent Adapter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    local_policy_digest: Option<String>,
    #[serde(default)]
    pending_commands: BTreeMap<String, PendingCommand>,
    #[serde(default)]
    queued_prompts: VecDeque<String>,
    #[serde(default)]
    pub(crate) workspace_root: PathBuf,
    #[serde(default)]
    completed_work: BTreeMap<u64, [u8; 32]>,
    pending_work: BTreeMap<u64, ControlOperation>,
    #[serde(skip)]
    policy_dependencies: BTreeMap<u64, Value>,
    capability_revision: u64,
    policy_chance_revisions: BTreeMap<String, (u64, u64)>,
    policy_record_revisions: BTreeMap<String, u64>,
    policy_revisions: BTreeMap<String, u64>,
    policy_work_context: BTreeMap<u64, BTreeMap<String, Value>>,
    /// 已准入投机的内部事实，按 Branch 保存；不进入 Branch 归档。
    #[serde(default)]
    pub(crate) speculations: BTreeMap<String, SpeculationRecord>,
    /// 模型用量事实，按 Branch 保存每次调用，供结算比较。无法配对 action
    /// 的调用也保留，不因为缺少 action_id 而丢弃。
    #[serde(default)]
    pub(crate) model_usage: BTreeMap<String, Vec<ModelUsageFact>>,
    /// 工具 `ActionFinished` 的 Instrumentation 时刻，按 Branch 和 action 保存。
    #[serde(default)]
    pub(crate) tool_finished_at_ms: BTreeMap<String, BTreeMap<String, u64>>,
    #[serde(default)]
    next_work: u64,
    #[serde(default = "first_id")]
    next_chance: u64,
    /// 每次 Branch 或 Chance 状态变化加一；`launch`、`rebalance` 只在版本
    /// 变化后重新求值，避免空转。
    #[serde(default)]
    epoch: u64,
    #[serde(default)]
    rebalance_epoch: Option<u64>,
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
    /// The control plane's logical transcript, retained independently of
    /// native Agent session files.
    #[serde(default)]
    pub(crate) transcripts: BTreeMap<String, Vec<Value>>,
    /// Runtime launch template for the run host and branch environment execution.
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
            completed_work: BTreeMap::new(),
            pending_work: BTreeMap::new(),
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

    /// 绑定这次投机执行使用的 Policy、Runtime Adapter 和能力视图。
    /// 这些值属于 Tree，不属于某个 Branch。
    pub fn bind_execution(&mut self, policy: Value, runtime: Value, capabilities: Value) {
        self.policy_binding = Some(policy);
        self.runtime_binding = Some(runtime);
        self.capability_revision = self.capability_revision.checked_add(1).expect("Capability revision exhausted");
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PendingCommand {
    branch_id: String,
    operation: CommandOperation,
    operation_id: Option<String>,
    generation: u64,
}

fn operation_name(operation: CommandOperation) -> &'static str {
    match operation {
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
