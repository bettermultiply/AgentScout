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
        /// 被准入投机的起点（准入请求的接入侧时刻）；非投机候选为 null。
        speculation_started_at_ms: Option<u64>,
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
        status: ActionStatus,
        /// 工具真正开始时刻来自 ActionStarted；只有准入请求时为 null。
        started_at_ms: Option<u64>,
        /// 工具结束时刻来自 ActionFinished；数据不可得时为 null。
        finished_at_ms: Option<u64>,
        duration_ms: Option<u64>,
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

#[derive(Serialize)]
pub struct StateEventSet {
    pub sequence: u64,
    pub events: Vec<ObservationEvent>,
}
