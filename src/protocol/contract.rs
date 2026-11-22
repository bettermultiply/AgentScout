
use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PROTOCOL_VERSION: u32 = 1;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentState {
    Available,
    Suspended,
    Terminated,
    Cleaned,
    /// 各 Backend 的状态不一致，不能继续普通操作。
    PartiallyAvailable,
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FollowUp {
    Suspend,
    Resume,
    Terminate,
    Cleanup,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceNote {
    pub content: Content,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentLaunch {
    pub command: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub log: PathBuf,
    /// Optional execution identity supplied by the caller. The host belongs to the run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_id: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeTool {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeCapabilities {
    #[serde(default)]
    pub backends: Vec<String>,
    #[serde(default)]
    pub tools: Vec<RuntimeTool>,
    /// 能从父环境派生出具有独立身份和生命周期的子环境；不承诺复制内部资源。
    #[serde(default)]
    pub branch_environment: bool,
    /// `Some(false)` 表示 discard 可用，后台删除为 O(文件数)。`None` 表示未声明。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filesystem_fast_delete: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filesystem_fork_cost: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filesystem_btrfs_incremental_diff: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freeze_mechanism: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filesystem_passthrough: Option<Vec<String>>,
    /// 包含状态目录的 overlay 挂载点在 fork 时使用 flatten（O(upper)）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_directory_overlaps_mount: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum Operation {
    MaterializeAuthority,
    SaveRuntime {
        path: std::path::PathBuf,
    },
    LoadRuntime {
        path: std::path::PathBuf,
    },
    EstablishEnvironment,
    OperateEnvironment {
        command: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent: Option<AgentLaunch>,
    },
    BranchEnvironment {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent: Option<AgentLaunch>,
    },
    PromoteEnvironment,
    AcceptEnvironment {
        source_environment_id: String,
        full: bool,
        /// None means Policy did not authorize part. Empty means results only.
        part: Option<PartSelection>,
        /// The caller owns a preceding SuspendEnvironment and the matching resume.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        target_suspended: bool,
    },
    SuspendEnvironment,
    ResumeEnvironment,
    TerminateEnvironment,
    CleanupEnvironment,
    ToolContext {
        tool_name: String,
    },
    FilesystemFork {
        parent_snapshot_id: String,
        child_snapshot_id: String,
        path_scope: Vec<PathScope>,
    },
    FilesystemMark {
        snapshot_id: String,
        mark_id: String,
        path_scope: Vec<PathScope>,
    },
    FilesystemDiff {
        base_snapshot_id: String,
        source_snapshot_id: String,
        path_scope: Vec<PathScope>,
    },
    FilesystemAccept {
        base_snapshot_id: String,
        source_snapshot_id: String,
        target_snapshot_id: String,
        path_scope: Vec<PathScope>,
    },
    FilesystemDiscard {
        snapshot_id: String,
        path_scope: Vec<PathScope>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathScope {
    Workspace,
    Tmp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PartSelection {
    Scopes(Vec<PathScope>),
    Results { results: Vec<PartResult> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartResult {
    pub id: String,
    pub files: Vec<String>,
}

impl PartSelection {
    pub fn results_only(&self) -> bool {
        match self {
            Self::Scopes(scopes) => scopes.is_empty(),
            Self::Results { results } => results.iter().all(|result| result.files.is_empty()),
        }
    }

    pub fn valid_results(results: &[PartResult]) -> bool {
        let mut ids = std::collections::BTreeSet::new();
        !results.is_empty() && results.iter().all(|result| {
            !result.id.trim().is_empty() && ids.insert(&result.id) && {
                let mut files = std::collections::BTreeSet::new();
                result.files.iter().all(|path| {
                    path.starts_with('/') && !path.contains('\0')
                        && path[1..].split('/').all(|part| !matches!(part, "" | "." | ".."))
                        && files.insert(path)
                })
            }
        })
    }

    pub fn results_only_receipt(&self) -> Acceptance {
        match self {
            Self::Scopes(_) => Acceptance::Part { scopes: vec![] },
            Self::Results { results } => Acceptance::PartResults {
                results: results.iter().filter(|result| result.files.is_empty())
                    .map(|result| result.id.clone()).collect(),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FilesystemConfirmation {
    #[serde(default)]
    pub target_paused: bool,
    #[serde(default)]
    pub context_ready: bool,
    #[serde(default)]
    pub snapshot_created: bool,
    #[serde(default)]
    pub metadata_durable: bool,
    #[serde(default)]
    pub writers_quiesced: bool,
    #[serde(default)]
    pub published: bool,
    #[serde(default)]
    pub refs_updated: bool,
    #[serde(default)]
    pub refs_released: bool,
    #[serde(default)]
    pub gc_eligible: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FilesystemConflict {
    pub path: String,
    pub kind: String,
    pub reason: String,
    /// 仅需冲突路径时，可省略三方版本的内容摘要。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ours: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theirs: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Acceptance {
    Full,
    Part { scopes: Vec<PathScope> },
    PartResults { results: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FilesystemResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acceptance: Option<Acceptance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_snapshot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff_ref: Option<String>,
    #[serde(default)]
    pub confirmation: FilesystemConfirmation,
    #[serde(default)]
    pub conflicts: Vec<FilesystemConflict>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_context: Option<crate::telemetry::Carrier>,
    pub request_id: String,
    /// Fences delayed filesystem results; legacy callers default to zero.
    #[serde(default)]
    pub generation: u64,
    /// Absent for establish and Runtime-wide save/load. Never a proposed identity.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub environment_id: String,
    pub operation: Operation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendOutput {
    pub backend_id: String,
    pub output: CommandOutput,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendState {
    pub backend_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<EnvironmentState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationResult {
    pub request_id: String,
    /// Created Environment on successful establish/branch, target otherwise.
    /// Establish failures without a created Environment omit this field.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub environment_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_access: Option<ExecutionAccess>,
    pub status: ResultStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_state: Option<EnvironmentState>,
    pub can_accept_operations: bool,
    pub follow_up: Vec<FollowUp>,
    pub resources: Vec<ResourceNote>,
    pub remaining: Vec<ResourceNote>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub backend_states: Vec<BackendState>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<BackendOutput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<CommandOutput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filesystem: Option<FilesystemResult>,
}

/// Instrumentation treats Runtime endpoint addresses as opaque.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionAccess {
    pub transport: String,
    pub address: String,
    pub environment_id: String,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "message", rename_all = "snake_case")]
pub enum AdapterMessage {
    Hello {
        protocol_version: u32,
        adapter_instance: String,
        #[serde(default)]
        capabilities: RuntimeCapabilities,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        backend_ids: Vec<String>,
    },
    Received {
        request_id: String,
    },
    Rejected {
        request_id: String,
        reason: String,
    },
    Result(OperationResult),
    Event(Event),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "message", rename_all = "snake_case")]
pub enum ControlMessage {
    Request(OperationRequest),
}
