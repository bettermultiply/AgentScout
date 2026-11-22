use super::gateway::agent::protocol::{ActionKind, ActionStatus, BranchRole};
use super::speculative_tree::{AuthorityState, BranchStatus, ChanceStage, RuntimeExecutionState};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObservationEvent {
    pub sequence: u64,
    pub timestamp_ms: u64,
    pub task_id: String,
    #[serde(flatten)]
    pub content: ObservationContent,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum ObservationContent {
    UiPromptRequested {
        branch_id: String,
        prompt_id: String,
        #[serde(flatten)]
        prompt: super::gateway::agent::protocol::UiPrompt,
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
        chance_id: Option<String>,
        speculation_kind: Option<ActionKind>,
        speculation_started_at_ms: Option<u64>,
    },
    ChanceCreated {
        chance_id: String,
        parent_branch_id: String,
        parent_chance_id: Option<String>,
        operation_id: String,
        tool_name: String,
        stage: ChanceStage,
        authority_state: AuthorityState,
        /// Instrumentation 准入请求时刻，可早于 select 和候选启动。
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
        /// 无法配对时为 null，用量仍上报。
        action_id: Option<String>,
        model: String,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        cache_read_tokens: Option<u64>,
        cache_write_tokens: Option<u64>,
        total_tokens: Option<u64>,
        cost: Option<f64>,
        /// Instrumentation 测量的模型请求开始与结束时刻，数据不可得时为 null。
        started_at_ms: Option<u64>,
        finished_at_ms: Option<u64>,
        duration_ms: Option<u64>,
    },
    ActionStarted {
        branch_id: String,
        action_id: String,
        action_kind: ActionKind,
        name: String,
        timestamp_ms: u64,
    },
    ModelFinished {
        branch_id: String,
        action_id: String,
        status: ActionStatus,
        timestamp_ms: u64,
    },
    ToolFinished {
        branch_id: String,
        action_id: String,
        /// 来自同一 action 的 `ActionStarted.name`，缺失时为 null。
        tool_name: Option<String>,
        status: ActionStatus,
        /// 工具真正开始时刻来自 ActionStarted；只有准入请求时为 null。
        started_at_ms: Option<u64>,
        /// 工具结束时刻来自 ActionFinished；数据不可得时为 null。
        finished_at_ms: Option<u64>,
        duration_ms: Option<u64>,
    },
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        disposition: Option<String>,
        time_ms: ObservationAmount,
        money: ObservationAmount,
        currency: Option<String>,
    },
    RunStatus {
        ready: bool,
        running: bool,
        notice: String,
        connected: bool,
    },
}

/// 实测与估计分别报告，缺失时为 null。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObservationAmount {
    pub measured: Option<f64>,
    pub estimated: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
