
use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchRole {
    Authority,
    Candidate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockedReason {
    ExtentReached,
    Rebalanced,
    EffectGate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandOperation {
    PrepareAcceptance,
    CancelAcceptance,
    Abort,
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

impl AgentCommand {
    pub fn branch_id(&self) -> &str {
        match self {
            Self::UiPromptAnswer { branch_id, .. }
            | Self::GetCommands { branch_id, .. }
            | Self::Model { branch_id, .. }
            | Self::SuspendRecovery { branch_id }
            | Self::CancelSave { branch_id }
            | Self::Continue { branch_id }
            | Self::Save { branch_id, .. }
            | Self::Load { branch_id, .. }
            | Self::LocalPolicy { branch_id, .. }
            | Self::Prompt { branch_id, .. }
            | Self::Abort { branch_id, .. }
            | Self::Pause { branch_id }
            | Self::Resume { branch_id }
            | Self::StartCandidate { branch_id, .. }
            | Self::ResolvePending { branch_id, .. }
            | Self::StartBranch { branch_id, .. }
            | Self::PrepareAcceptance { branch_id, .. }
            | Self::CancelAcceptance { branch_id, .. }
            | Self::Promote { branch_id, .. }
            | Self::Discard { branch_id, .. }
            | Self::Cleanup { branch_id, .. } => branch_id,
        }
    }
}

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
    UiPromptAnswer {
        branch_id: String,
        prompt_id: String,
        answer: Value,
        cancelled: bool,
        reason: Option<String>,
    },
    GetCommands {
        branch_id: String,
        request_id: String,
    },
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
    Continue {
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
        #[serde(default)]
        execution_accesses: BTreeMap<String, crate::protocol::contract::ExecutionAccess>,
    },
    LocalPolicy {
        branch_id: String,
        policy_digest: String,
        has_predictor: bool,
        decisions: Value,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        pure: BTreeMap<String, Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        decision_inputs: Option<BTreeMap<String, Vec<String>>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        policy_model_grant: Option<crate::control_plane::policy::PolicyModelGrant>,
    },
    Prompt {
        branch_id: String,
        prompt: String,
    },
    Abort {
        branch_id: String,
        command_id: String,
        generation: u64,
    },
    Pause {
        branch_id: String,
    },
    Resume {
        branch_id: String,
    },
    StartCandidate {
        branch_id: String,
        environment_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        execution_access: Option<crate::protocol::contract::ExecutionAccess>,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        execution_access: Option<crate::protocol::contract::ExecutionAccess>,
        role: BranchRole,
        workspace: PathBuf,
        prompt: String,
    },
    PrepareAcceptance {
        branch_id: String,
        authority_branch_id: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        target_suspended: bool,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        part: bool,
        command_id: String,
        generation: u64,
    },
    CancelAcceptance {
        branch_id: String,
        command_id: String,
        generation: u64,
    },
    Promote {
        branch_id: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        reuse_parent_environment: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_branch_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        execution_access: Option<crate::protocol::contract::ExecutionAccess>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        part: bool,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiPrompt {
    pub kind: String,
    pub title: String,
    pub message_text: Option<String>,
    pub options: Vec<String>,
    pub placeholder: Option<String>,
    pub prefill: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "message", rename_all = "snake_case")]
pub enum AgentMessage {
    UiPromptRequested {
        branch_id: String,
        prompt_id: String,
        #[serde(flatten)]
        prompt: UiPrompt,
    },
    UiPromptCancelled {
        branch_id: String,
        prompt_id: String,
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
        /// Instrumentation 测量的事件时刻（Unix 毫秒），不是 control plane 收到消息的时刻。
        timestamp_ms: u64,
    },
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fast_start: Option<Value>,
        /// 投机开始的 Instrumentation 测量时刻（Unix 毫秒）。
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
        /// Instrumentation 观察到的结果到达时刻（Unix 毫秒）。
        timestamp_ms: u64,
    },
    Usage {
        branch_id: String,
        /// 无法配对时为 None，用量仍上报。
        #[serde(default)]
        action_id: Option<String>,
        model: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thinking_level: Option<String>,
        usage: Value,
        /// Instrumentation 测量的模型请求开始与结束时刻（Unix 毫秒）。
        started_at_ms: u64,
        finished_at_ms: u64,
    },
    BranchFailed {
        branch_id: String,
        reason: String,
    },
    AgentReady {
        agent_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session: Option<serde_json::Value>,
    },
    AgentDisconnected {
        agent_id: String,
    },
    AgentFailed {
        agent_id: String,
        reason: String,
    },
    SessionHistory {
        branch_id: String,
        entries: Vec<Value>,
    },
    AgentReleased {
        agent_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session: Option<serde_json::Value>,
    },
    BranchCreated {
        branch_id: String,
    },
    BranchReady {
        branch_id: String,
    },
    EnvironmentRequested {
        branch_id: String,
    },
    BranchSuspended {
        branch_id: String,
        reason: BlockedReason,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum AgentWireMessage {
    Command(AgentCommand),
    Delivery {
        id: u64,
        command: AgentCommand,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trace_context: Option<crate::telemetry::Carrier>,
    },
    Received { id: u64 },
    Message(AgentMessage),
}
