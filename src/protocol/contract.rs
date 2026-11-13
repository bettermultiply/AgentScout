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
    /// Backend 共同提供的环境中，Backend 的实际状态不一致。
    ///
    /// 这个状态只表示聚合层无法把环境归入一个单一生命周期状态；它
    /// 不是一个可以继续执行普通操作的状态。
    PartiallyAvailable,
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
    /// Agent 实际执行实例的稳定标识。旧配置省略时由 Adapter 使用环境
    /// 标识作为兼容性的默认值。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_id: Option<String>,
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
    /// 文件系统 Backend 是否能用严格子卷删除 ioctl 快速回收。`Some(false)`
    /// 表示 discard 仍可用，但后台物理删除是 O(文件数)；`None` 表示该
    /// Runtime 不声明文件系统回收方式。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filesystem_fast_delete: Option<bool>,
    /// 文件系统 fork 的成本类别：`reflink`、`copy` 或 `btrfs_snapshot`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filesystem_fork_cost: Option<String>,
    /// btrfs 增量 diff 是否可在当前权限下使用。`false` 表示已降级到 overlay。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filesystem_btrfs_incremental_diff: Option<bool>,
    /// 实际可用的写者冻结机制：`cgroup`、`process_group` 或 `none`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freeze_mechanism: Option<String>,
    /// 明确被允许直通宿主的路径列表。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filesystem_passthrough: Option<Vec<String>>,
    /// 状态目录位于其内部的 overlay 挂载点。这些挂载在 fork 时使用
    /// flatten（O(upper)）而不是 lower 链增量；空列表表示没有重叠。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_directory_overlaps_mount: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum Operation {
    SaveRuntime { path: std::path::PathBuf },
    LoadRuntime { path: std::path::PathBuf },
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
    FilesystemFork { parent_snapshot_id: String, child_snapshot_id: String, path_scope: Vec<PathScope> },
    FilesystemMark { snapshot_id: String, mark_id: String, path_scope: Vec<PathScope> },
    FilesystemDiff { base_snapshot_id: String, source_snapshot_id: String, path_scope: Vec<PathScope> },
    FilesystemAccept { base_snapshot_id: String, source_snapshot_id: String, target_snapshot_id: String, path_scope: Vec<PathScope> },
    FilesystemDiscard { snapshot_id: String, path_scope: Vec<PathScope> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathScope { Workspace, Tmp }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FilesystemConfirmation {
    #[serde(default)] pub target_paused: bool,
    #[serde(default)] pub context_ready: bool,
    #[serde(default)] pub snapshot_created: bool,
    #[serde(default)] pub metadata_durable: bool,
    #[serde(default)] pub writers_quiesced: bool,
    #[serde(default)] pub published: bool,
    #[serde(default)] pub refs_updated: bool,
    #[serde(default)] pub refs_released: bool,
    #[serde(default)] pub gc_eligible: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FilesystemConflict {
    pub path: String,
    pub kind: String,
    pub reason: String,
    /// 三方合并时的基准、目标分支和来源分支内容摘要。留空表示调用方
    /// 只关心冲突路径，不要求具体的版本描述。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ours: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theirs: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FilesystemResult {
    #[serde(default, skip_serializing_if = "Option::is_none")] pub snapshot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub published_snapshot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub diff_ref: Option<String>,
    #[serde(default)] pub confirmation: FilesystemConfirmation,
    #[serde(default)] pub conflicts: Vec<FilesystemConflict>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationRequest {
    pub request_id: String,
    /// Fences delayed filesystem results; legacy callers default to zero.
    #[serde(default)]
    pub generation: u64,
    pub environment_id: String,
    pub operation: Operation,
    /// Policy 用于选择 Runtime 的任务上下文。旧控制面不发送时保持为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// 逻辑 Branch 标识。它和 environment_id 分开，允许一个逻辑 Branch
    /// 在不同 Runtime 环境中继续执行。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_id: Option<String>,
}

/// 一个 Backend 对一次操作产生的输出。
///
/// `OperationResult.output` 只保留明确选中的单一主输出；所有 Backend
/// 输出都通过这个列表可见，避免组合多个 Backend 时后一个输出静默覆盖
/// 前一个输出。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendOutput {
    pub backend_id: String,
    pub output: CommandOutput,
}

/// 一个环境由多个 Backend 共同提供时，各 Backend 的实际状态快照。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendState {
    pub backend_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<EnvironmentState>,
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
    /// 每个参与 Backend 的状态，供控制面处理部分失败和状态不一致。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub backend_states: Vec<BackendState>,
    /// 组合 Runtime 的全部可见输出。不会因为 `output` 只有一个字段而
    /// 丢弃其他 Backend 的输出。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<BackendOutput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<CommandOutput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filesystem: Option<FilesystemResult>,
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
    OperationStarted {
        operation: Operation,
    },
    OperationFinished {
        result: OperationResult,
    },
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
        /// 本次 Runtime 实例由哪些 Backend 组成的有序标识。
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        backend_ids: Vec<String>,
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

#[cfg(test)]
mod filesystem_protocol_tests {
    use super::*;

    #[test]
    fn filesystem_operations_round_trip_with_both_managed_scopes() {
        let scopes = vec![PathScope::Workspace, PathScope::Tmp];
        let operations = vec![
            Operation::FilesystemFork { parent_snapshot_id: "base".into(), child_snapshot_id: "child".into(), path_scope: scopes.clone() },
            Operation::FilesystemMark { snapshot_id: "child".into(), mark_id: "mark".into(), path_scope: scopes.clone() },
            Operation::FilesystemDiff { base_snapshot_id: "base".into(), source_snapshot_id: "child".into(), path_scope: scopes.clone() },
            Operation::FilesystemAccept { base_snapshot_id: "base".into(), source_snapshot_id: "child".into(), target_snapshot_id: "target".into(), path_scope: scopes.clone() },
            Operation::FilesystemDiscard { snapshot_id: "child".into(), path_scope: scopes },
        ];
        for operation in operations {
            let json = serde_json::to_value(&operation).unwrap();
            assert_eq!(json["path_scope"], serde_json::json!(["workspace", "tmp"]));
            assert_eq!(serde_json::from_value::<Operation>(json).unwrap(), operation);
        }
    }

    #[test]
    fn conflicts_and_confirmation_survive_wire_round_trip() {
        let result = FilesystemResult {
            conflicts: vec![FilesystemConflict { path: "workspace/a".into(), kind: "modify_delete".into(), reason: "both sides changed".into(), ..Default::default() }],
            ..Default::default()
        };
        let decoded: FilesystemResult = serde_json::from_slice(&serde_json::to_vec(&result).unwrap()).unwrap();
        assert_eq!(decoded, result);
        assert!(!decoded.confirmation.published);
        assert!(!decoded.confirmation.gc_eligible);
    }
}
