//! Public observation facts, independent of transport and file storage.
use super::agent_control::{
    ActionKind, ActionStatus, AuthorityState, BranchRole, BranchStatus, ChanceStage,
    RuntimeExecutionState,
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
    UiPromptRequested {
        branch_id: String,
        prompt_id: String,
        #[serde(flatten)]
        prompt: super::agent_control::UiPrompt,
    },
    UiPromptResolved {
        branch_id: String,
        prompt_id: String,
        cancelled: bool,
        reason: Option<String>,
    },
    UiNotification {
        branch_id: String,
        text: String,
        level: String,
    },
    CommandsListed {
        branch_id: String,
        request_id: String,
        commands: Value,
    },
    BranchCreated {
        branch_id: String,
        parent_branch_id: Option<String>,
        role: BranchRole,
        /// 这个 Branch 所属的 Chance；初始权威为 null。
        chance_id: Option<String>,
        speculation_kind: Option<ActionKind>,
        /// 被准入投机的起点（准入请求的接入侧时刻）；非投机候选为 null。
        speculation_started_at_ms: Option<u64>,
    },
    /// `admit` 通过：Chance 获得 id，处于 `pending`。
    ChanceCreated {
        chance_id: String,
        parent_branch_id: String,
        parent_chance_id: Option<String>,
        operation_id: String,
        tool_name: String,
        stage: ChanceStage,
        authority_state: AuthorityState,
        /// 接入侧准入请求时刻，可早于 select 和候选启动。
        speculation_started_at_ms: Option<u64>,
    },
    ChanceStageChanged {
        chance_id: String,
        stage: ChanceStage,
        reason: Option<String>,
        /// Accepted 所依据的验证工作；其他状态或未知时为 null。
        validation_id: Option<u64>,
    },
    /// 在 Policy Match 实际完成时测量，而不是事件追加或 Promote 时刻。
    ProposalValidated {
        chance_id: String,
        proposal_id: String,
        validation_id: u64,
        validated_at_ms: Option<u64>,
    },
    AuthorityStateChanged {
        chance_id: String,
        state: AuthorityState,
    },
    /// Chance 上新增一个不可变 Proposal。
    ProposalAdded {
        chance_id: String,
        proposal_id: String,
        payload: Value,
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
    },
    ModelUsage {
        branch_id: String,
        /// 这次调用对应的模型 action；接入侧无法配对时为 null，不伪造身份，
        /// 也不会因此丢弃这次用量。
        action_id: Option<String>,
        model: String,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        cache_read_tokens: Option<u64>,
        cache_write_tokens: Option<u64>,
        total_tokens: Option<u64>,
        cost: Option<f64>,
        /// 接入侧测量的模型请求开始与结束时刻，数据不可得时为 null。
        started_at_ms: Option<u64>,
        finished_at_ms: Option<u64>,
        duration_ms: Option<u64>,
    },
    ToolFinished {
        branch_id: String,
        action_id: String,
        /// 工具名来自同一 action 的 `ActionStarted.name`；没有 ActionStarted
        /// （例如只有准入请求）时为 null，不推测。
        tool_name: Option<String>,
        status: ActionStatus,
        /// 工具真正开始时刻来自 ActionStarted；只有准入请求时为 null。
        started_at_ms: Option<u64>,
        /// 工具结束时刻来自 ActionFinished；数据不可得时为 null。
        finished_at_ms: Option<u64>,
        duration_ms: Option<u64>,
    },
    /// NLE workload's public observation before and after one real action.
    /// The Runtime bridge is the authority for these values; the control plane
    /// only forwards the already-public chars/blstats payload.
    NleMapAction {
        branch_id: String,
        action_id: String,
        action: u64,
        before: Value,
        after: Value,
    },
    Settlement {
        branch_id: String,
        hit: bool,
        time_ms: ObservationAmount,
        money: ObservationAmount,
        /// 金钱维度的币种；该维度没有数据时为 null，不猜测。
        currency: Option<String>,
    },
    RunStatus {
        ready: bool,
        running: bool,
        notice: String,
        connected: bool,
    },
}

/// 结算的一个维度。实测与估计是两个独立部分：没有可靠测量时 `measured`
/// 为 null，没有估计依据时 `estimated` 为 null；两者都不与对方相加或互相填充。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ObservationAmount {
    /// 实测部分，带符号；没有可靠测量时为 null，不补零。
    pub measured: Option<f64>,
    /// 估计部分，带符号；没有估计依据时为 null，不补零。
    pub estimated: Option<f64>,
}

/// 状态事件集中一个 Chance 的当前视图，由 Chance 与 Branch 事件折叠得到。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ObservedChance {
    pub chance_id: String,
    pub parent_branch_id: String,
    pub parent_chance_id: Option<String>,
    pub operation_id: String,
    pub tool_name: String,
    pub stage: ChanceStage,
    pub authority_state: AuthorityState,
    pub speculation_started_at_ms: Option<u64>,
    pub validation_id: Option<u64>,
    pub proposal_ids: Vec<String>,
    pub branch_ids: Vec<String>,
}

#[derive(Serialize)]
pub struct StateEventSet {
    pub sequence: u64,
    pub chances: Vec<ObservedChance>,
    pub events: Vec<ObservationEvent>,
}

impl StateEventSet {
    pub fn new(sequence: u64, events: Vec<ObservationEvent>) -> Self {
        let mut chances: Vec<ObservedChance> = Vec::new();
        for event in &events {
            match &event.content {
                ObservationContent::ChanceCreated {
                    chance_id,
                    parent_branch_id,
                    parent_chance_id,
                    operation_id,
                    tool_name,
                    stage,
                    authority_state,
                    speculation_started_at_ms,
                } => chances.push(ObservedChance {
                    chance_id: chance_id.clone(),
                    parent_branch_id: parent_branch_id.clone(),
                    parent_chance_id: parent_chance_id.clone(),
                    operation_id: operation_id.clone(),
                    tool_name: tool_name.clone(),
                    stage: stage.clone(),
                    authority_state: *authority_state,
                    speculation_started_at_ms: *speculation_started_at_ms,
                    validation_id: None,
                    proposal_ids: Vec::new(),
                    branch_ids: Vec::new(),
                }),
                ObservationContent::ChanceStageChanged {
                    chance_id,
                    stage,
                    validation_id,
                    ..
                } => {
                    if let Some(chance) = chances.iter_mut().find(|c| &c.chance_id == chance_id) {
                        chance.stage = stage.clone();
                        chance.validation_id = *validation_id;
                    }
                }
                ObservationContent::AuthorityStateChanged { chance_id, state } => {
                    if let Some(chance) = chances.iter_mut().find(|c| &c.chance_id == chance_id) {
                        chance.authority_state = *state;
                    }
                }
                ObservationContent::ProposalAdded {
                    chance_id,
                    proposal_id,
                    ..
                } => {
                    if let Some(chance) = chances.iter_mut().find(|c| &c.chance_id == chance_id) {
                        chance.proposal_ids.push(proposal_id.clone());
                    }
                }
                ObservationContent::BranchCreated {
                    branch_id,
                    chance_id: Some(chance_id),
                    ..
                } => {
                    if let Some(chance) = chances.iter_mut().find(|c| &c.chance_id == chance_id) {
                        chance.branch_ids.push(branch_id.clone());
                    }
                }
                _ => {}
            }
        }
        Self {
            sequence,
            chances,
            events,
        }
    }
}
