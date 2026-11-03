//! 控制面与 Runtime Adapter 之间交换的契约数据。
//!
//! 语义以 `docs/design/runtime-adapter-contract.md` 为准；这里只定义线上的类型、
//! 字段和枚举，不包含行为。

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 线协议版本，会话开始时由 Adapter 上报。
pub const PROTOCOL_VERSION: u32 = 1;

/// 一次请求的处置结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResultStatus {
    Accepted,
    InProgress,
    Completed,
    Failed,
    PartiallyCompleted,
    NotApplicable,
    Unsupported,
}

/// 环境实际停留的状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentState {
    Available,
    Suspended,
    Terminated,
    Cleaned,
}

/// 环境内容，用于归类资源处理和隔离声明。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Content {
    Process,
    Workspace,
    Memory,
    Io,
    Network,
    Resource,
    ControlSurface,
}

/// 仍需的后续操作。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FollowUp {
    Suspend,
    Resume,
    Terminate,
    Cleanup,
}

/// 按环境内容归类的资源说明。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceNote {
    pub content: Content,
    pub note: String,
}

/// 环境内操作的输出。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Agent 接入侧的启动参数。
///
/// 用哪个 Agent 产品、哪个版本由控制面（用户）决定；真正启动它属于 Runtime。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentLaunch {
    pub command: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub log: PathBuf,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
}

/// 固定环境操作。
/// Runtime 可执行的环境工具。控制面只按声明调用，不解释具体 workload。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeTool {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

/// Runtime 在握手中声明的能力。工具调用和共享环境由 Runtime 自己承担。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeCapabilities {
    #[serde(default)]
    pub tools: Vec<RuntimeTool>,
    /// 所有 Branch 共用同一个底层环境。
    #[serde(default)]
    pub shared_environment: bool,
    /// 能从父环境派生出独立子环境。
    #[serde(default)]
    pub branch_environment: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum Operation {
    EstablishEnvironment,
    /// 环境内操作。`agent` 在场时表示在环境内建立 Agent 接入侧。
    OperateEnvironment {
        command: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent: Option<AgentLaunch>,
    },
    BranchEnvironment {
        child_environment_id: String,
    },
    /// 把 Candidate 的环境提升为权威环境。共享环境 Runtime 可以是 no-op。
    PromoteEnvironment,
    SuspendEnvironment,
    ResumeEnvironment,
    TerminateEnvironment,
    CleanupEnvironment,
    /// 读取环境工具的公开上下文，供 Policy 预测使用。
    ToolContext {
        tool_name: String,
    },
    /// 在权威环境中执行一次工具调用，返回结果与可供匹配的证据。
    InvokeTool {
        tool_name: String,
        arguments: Value,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationRequest {
    pub request_id: String,
    pub environment_id: String,
    pub operation: Operation,
}

/// 统一的操作结果信封。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationResult {
    pub request_id: String,
    pub environment_id: String,
    pub status: ResultStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_state: Option<EnvironmentState>,
    pub can_accept_operations: bool,
    pub follow_up: Vec<FollowUp>,
    pub resources: Vec<ResourceNote>,
    pub remaining: Vec<ResourceNote>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<CommandOutput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub environment_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    pub payload: EventPayload,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum EventPayload {
    EnvironmentStateChanged {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        previous: Option<EnvironmentState>,
        current: EnvironmentState,
        reason: String,
    },
}

/// Adapter 发给控制面的消息。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "message", rename_all = "snake_case")]
pub enum AdapterMessage {
    Hello {
        protocol_version: u32,
        adapter_instance: String,
        #[serde(default)]
        capabilities: RuntimeCapabilities,
    },
    Result(OperationResult),
    Event(Event),
}

/// 控制面发给 Adapter 的消息。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "message", rename_all = "snake_case")]
pub enum ControlMessage {
    Request(OperationRequest),
}
