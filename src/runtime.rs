//! 通用 Runtime 组合层。
//!
//! Runtime Adapter 只运行一个进程，但这个进程可以持有多个 Backend
//! 连接。`CompositeRuntime` 负责根据显式 Policy 选择 Backend，并按固定
//! 顺序执行生命周期操作；`BaseRuntime` 是一个特殊的 Backend，它在另
//! 一个 Backend 之上管理一个 Agent 实际执行实例的进程句柄。
//!
//! 这里不认识 NLE、Firecracker 或任何具体 workload。具体 Backend 由
//! `ascout-runtime-adapter` binary 装配进来。Runtime 指一个 Adapter 实例
//! 及其组合的 Backend 的整体，不是一个独立的类型或标记 trait；`CompositeRuntime`
//! 与 `BaseRuntime` 都实现 `Backend`，Adapter 一视同仁地通过 `Arc<dyn Backend>`
//! 调用它们。

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::backend::{Backend, BackendContext, BackendError, BackendExecution, CancelHandle};
use crate::contract::{
    AgentLaunch, BackendState, CommandOutput, EnvironmentState, Operation, RuntimeCapabilities,
    RuntimeTool, FilesystemResult,
};
use crate::filesystem::FilesystemBackend;
use crate::process::{FreezeMechanism, LaunchFragment, ProcessBackend, ProcessHandle};

/// Runtime 的一个有序 Backend 选择。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendSelection {
    pub backend_ids: Vec<String>,
    /// 多个 Backend 都返回输出时，Policy 可以明确指定哪个输出进入旧
    /// 的单值 `OperationResult.output` 字段。所有输出仍保留在 `outputs`。
    pub output_backend: Option<String>,
}

/// Policy 可选择的 Runtime 操作。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendOperation {
    Establish,
    Operate,
    FilesystemFork,
    FilesystemMark,
    FilesystemDiff,
    FilesystemAccept,
    FilesystemDiscard,
    Branch,
    Promote,
    Suspend,
    Resume,
    Terminate,
    Cleanup,
    ToolContext,
    InvokeTool,
}

impl BackendOperation {
    pub fn for_operation(operation: &Operation) -> Self {
        match operation {
            Operation::EstablishEnvironment => Self::Establish,
            Operation::OperateEnvironment { .. } => Self::Operate,
            Operation::BranchEnvironment { .. } => Self::Branch,
            Operation::PromoteEnvironment => Self::Promote,
            Operation::SuspendEnvironment => Self::Suspend,
            Operation::ResumeEnvironment => Self::Resume,
            Operation::TerminateEnvironment => Self::Terminate,
            Operation::CleanupEnvironment => Self::Cleanup,
            Operation::ToolContext { .. } => Self::ToolContext,
            Operation::InvokeTool { .. } => Self::InvokeTool,
            Operation::FilesystemFork { .. } => Self::FilesystemFork,
            Operation::FilesystemMark { .. } => Self::FilesystemMark,
            Operation::FilesystemDiff { .. } => Self::FilesystemDiff,
            Operation::FilesystemAccept { .. } => Self::FilesystemAccept,
            Operation::FilesystemDiscard { .. } => Self::FilesystemDiscard,
        }
    }
}

/// 一个有序 Backend 路由规则。
///
/// 未填写的匹配字段表示“任意”。多个规则同时匹配时，约束字段最多的
/// 规则胜出；同等具体度的规则不能靠排列顺序静默覆盖，因此会返回明确
/// 的歧义错误。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackendRoute {
    #[serde(default)]
    pub operation: Option<BackendOperation>,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub environment_id: Option<String>,
    #[serde(default)]
    pub branch_id: Option<String>,
    pub backends: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_backend: Option<String>,
}

impl BackendRoute {
    fn matches(&self, context: &BackendContext, operation: BackendOperation) -> bool {
        self.operation.is_none_or(|value| value == operation)
            && self
                .task_id
                .as_deref()
                .is_none_or(|value| context.task_id.as_deref() == Some(value))
            && self
                .environment_id
                .as_deref()
                .is_none_or(|value| value == context.environment_id)
            && self
                .branch_id
                .as_deref()
                .is_none_or(|value| context.branch_id.as_deref() == Some(value))
    }

    fn specificity(&self) -> usize {
        usize::from(self.operation.is_some())
            + usize::from(self.task_id.is_some())
            + usize::from(self.environment_id.is_some())
            + usize::from(self.branch_id.is_some())
    }
}

/// Backend 选择 Policy。
///
/// `default` 是没有规则匹配时使用的有序 Backend 列表。多 Backend 配置
/// 必须显式给出这个列表或至少一条匹配规则；Runtime 不会因为“恰好有
/// 多个连接”就把请求广播到所有连接。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackendPolicy {
    #[serde(default)]
    pub default: Vec<String>,
    #[serde(default)]
    pub routes: Vec<BackendRoute>,
}

impl BackendPolicy {
    pub fn single(backend_id: impl Into<String>) -> Self {
        Self {
            default: vec![backend_id.into()],
            routes: Vec::new(),
        }
    }

    pub fn select(
        &self,
        context: &BackendContext,
        operation: BackendOperation,
    ) -> Result<BackendSelection, BackendError> {
        let mut best: Option<&BackendRoute> = None;
        let mut best_specificity = 0;
        for route in &self.routes {
            if !route.matches(context, operation) {
                continue;
            }
            let specificity = route.specificity();
            if best.is_some() && specificity == best_specificity {
                return Err(BackendError::new(format!(
                    "ambiguous Runtime Backend Policy for {:?} and environment {}",
                    operation, context.environment_id
                )));
            }
            if best.is_none() || specificity > best_specificity {
                best = Some(route);
                best_specificity = specificity;
            }
        }
        let (backend_ids, output_backend) = best
            .map(|route| (route.backends.clone(), route.output_backend.clone()))
            .unwrap_or_else(|| (self.default.clone(), None));
        if backend_ids.is_empty() {
            return Err(BackendError::new(format!(
                "Runtime Backend Policy selected no Backend for {:?} and environment {}",
                operation, context.environment_id
            )));
        }
        if let Some(output_backend) = &output_backend
            && !backend_ids.iter().any(|id| id == output_backend)
        {
            return Err(BackendError::new(format!(
                "output Backend {output_backend} is not selected for {:?}",
                operation
            )));
        }
        Ok(BackendSelection {
            backend_ids,
            output_backend,
        })
    }

    pub fn validate(&self, backend_ids: &BTreeSet<String>) -> Result<(), BackendError> {
        validate_backend_list(&self.default, backend_ids, "default")?;
        for (index, route) in self.routes.iter().enumerate() {
            validate_backend_list(
                &route.backends,
                backend_ids,
                &format!("routes[{index}].backends"),
            )?;
            if let Some(output_backend) = &route.output_backend
                && !route.backends.iter().any(|id| id == output_backend)
            {
                return Err(BackendError::new(format!(
                    "routes[{index}].output_backend {output_backend} is not selected"
                )));
            }
        }
        Ok(())
    }
}

fn validate_backend_list(
    selected: &[String],
    available: &BTreeSet<String>,
    location: &str,
) -> Result<(), BackendError> {
    if selected.is_empty() {
        return Ok(());
    }
    let mut seen = BTreeSet::new();
    for id in selected {
        if !available.contains(id) {
            return Err(BackendError::new(format!(
                "{location} selects unknown Backend {id}"
            )));
        }
        if !seen.insert(id) {
            return Err(BackendError::new(format!(
                "{location} selects Backend {id} more than once"
            )));
        }
    }
    Ok(())
}

/// 由 Adapter 统一管理的 Backend 连接。
pub struct BackendConnection {
    pub id: String,
    pub backend: Arc<dyn Backend>,
}

impl BackendConnection {
    pub fn new(id: impl Into<String>, backend: Arc<dyn Backend>) -> Result<Self, BackendError> {
        let id = id.into();
        if id.trim().is_empty() {
            return Err(BackendError::new("Backend id must not be empty"));
        }
        Ok(Self { id, backend })
    }
}

/// 一个 Adapter 进程内的多 Backend Runtime。
pub struct CompositeRuntime {
    id: String,
    connections: Vec<BackendConnection>,
    policy: BackendPolicy,
    /// 每个环境参与过 establish/branch 的 Backend id 集合。只有成功完成
    /// 的 establish/branch 才登记；cleanup/terminate 后仍保留，用于解释
    /// 最终状态。空的或缺席的记录走旧路径兼容行为。
    participants: Mutex<HashMap<String, BTreeSet<String>>>,
}

impl CompositeRuntime {
    pub fn new(
        id: impl Into<String>,
        connections: Vec<BackendConnection>,
        policy: BackendPolicy,
    ) -> Result<Self, BackendError> {
        if connections.is_empty() {
            return Err(BackendError::new("Runtime requires at least one Backend"));
        }
        let mut ids = BTreeSet::new();
        for connection in &connections {
            if !ids.insert(connection.id.clone()) {
                return Err(BackendError::new(format!(
                    "duplicate Runtime Backend id {}",
                    connection.id
                )));
            }
        }
        policy.validate(&ids)?;
        if connections.len() > 1 && policy.default.is_empty() && policy.routes.is_empty() {
            return Err(BackendError::new(
                "multiple Runtime Backends require an explicit Backend Policy",
            ));
        }
        Ok(Self {
            id: id.into(),
            connections,
            policy,
            participants: Mutex::new(HashMap::new()),
        })
    }

    pub fn policy(&self) -> &BackendPolicy {
        &self.policy
    }

    fn selected(
        &self,
        context: &BackendContext,
        operation: BackendOperation,
    ) -> Result<(Vec<&BackendConnection>, Option<String>), BackendError> {
        let selection = self.policy.select(context, operation)?;
        let mut result = Vec::with_capacity(selection.backend_ids.len());
        for id in &selection.backend_ids {
            let Some(connection) = self.connections.iter().find(|value| value.id == *id) else {
                return Err(BackendError::new(format!(
                    "Runtime Policy selected unknown Backend {id}"
                )));
            };
            result.push(connection);
        }
        Ok((result, selection.output_backend))
    }

    fn execute_outputs<F>(
        &self,
        context: &BackendContext,
        operation: BackendOperation,
        cancel: &CancelHandle,
        mut call: F,
    ) -> Result<BackendExecution, BackendError>
    where
        F: FnMut(
            &BackendConnection,
            &BackendContext,
            &CancelHandle,
        ) -> Result<BackendExecution, BackendError>,
    {
        let (connections, output_backend) = self.selected(context, operation)?;
        let ids: Vec<_> = connections
            .iter()
            .map(|connection| connection.id.clone())
            .collect();
        let mut outputs = Vec::new();
        let mut completed = Vec::new();
        for (index, connection) in connections.iter().enumerate() {
            if cancel.is_cancelled() {
                return Err(BackendError::interrupted()
                    .with_completed(completed)
                    .with_remaining(ids[index..].to_vec())
                    .with_outputs(outputs));
            }
            match call(connection, context, &cancel) {
                Ok(execution) => {
                    outputs.extend(normalize_outputs(&connection.id, execution.outputs));
                    completed.push(connection.id.clone());
                }
                Err(mut error) => {
                    outputs.extend(normalize_outputs(
                        &connection.id,
                        error.outputs.drain(..).collect(),
                    ));
                    error.message = format!(
                        "Backend {} failed during {:?}: {}",
                        connection.id, operation, error.message
                    );
                    error.completed = completed;
                    error.remaining = ids[index..].to_vec();
                    error.outputs = outputs;
                    return Err(error);
                }
            }
        }
        Ok(BackendExecution {
            outputs,
            primary_backend: output_backend,
        })
    }

    fn execute_lifecycle<F>(
        &self,
        context: &BackendContext,
        operation: BackendOperation,
        mut call: F,
    ) -> Result<(), BackendError>
    where
        F: FnMut(&BackendConnection, &BackendContext) -> Result<(), BackendError>,
    {
        let (connections, _) = self.selected(context, operation)?;
        let ids: Vec<_> = connections
            .iter()
            .map(|connection| connection.id.clone())
            .collect();
        let mut completed = Vec::new();
        for (index, connection) in connections.iter().enumerate() {
            match call(connection, context) {
                Ok(()) => completed.push(connection.id.clone()),
                Err(mut error) => {
                    error.message = format!(
                        "Backend {} failed during {:?}: {}",
                        connection.id, operation, error.message
                    );
                    error.completed = completed;
                    error.remaining = ids[index..].to_vec();
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    fn establish_all(&self, context: &BackendContext) -> Result<(), BackendError> {
        let (connections, _) = self.selected(context, BackendOperation::Establish)?;
        let ids: Vec<_> = connections
            .iter()
            .map(|connection| connection.id.clone())
            .collect();
        let mut completed: Vec<&BackendConnection> = Vec::new();
        for (index, connection) in connections.iter().enumerate() {
            match connection.backend.establish_with_context(context) {
                Ok(()) => completed.push(connection),
                Err(mut error) => {
                    let mut cleanup_failures = Vec::new();
                    for previous in completed.iter().rev() {
                        if let Err(cleanup) = previous.backend.cleanup_with_context(context) {
                            cleanup_failures.push(format!("{}: {}", previous.id, cleanup.message));
                        }
                    }
                    error.message = format!(
                        "Backend {} failed during establish: {}",
                        connection.id, error.message
                    );
                    error.completed = completed.iter().map(|value| value.id.clone()).collect();
                    error.remaining = ids[index..].to_vec();
                    error.cleanup_failures.extend(cleanup_failures);
                    return Err(error);
                }
            }
        }
        // 只有全部 Backend 建立成功才登记参与者；中途失败已经回滚并返回错误，
        // 不会把失败环境登记成完整参与者。
        self.record_participants(&context.environment_id, &ids)?;
        Ok(())
    }

    fn cleanup_all(&self, context: &BackendContext) -> Result<(), BackendError> {
        let (connections, _) = self.selected(context, BackendOperation::Cleanup)?;
        let mut completed = Vec::new();
        let mut failed = Vec::new();
        let mut messages = Vec::new();
        // 清理必须反向执行，以先释放依赖于其他 Backend 的外层资源。
        for connection in connections.iter().rev() {
            match connection.backend.cleanup_with_context(context) {
                Ok(()) => completed.push(connection.id.clone()),
                Err(error) => {
                    failed.push(connection.id.clone());
                    messages.push(format!("{}: {}", connection.id, error.message));
                }
            }
        }
        if failed.is_empty() {
            Ok(())
        } else {
            // 清理会尽力覆盖所有选中的 Backend，不因为单个失败就跳过后续；
            // 因此 `remaining` 保持为空，失败以 `cleanup_failures` 如实上报。
            Err(BackendError {
                message: format!("Runtime cleanup partially failed: {}", messages.join("; ")),
                cancelled: false,
                completed,
                remaining: Vec::new(),
                cleanup_failures: messages,
                outputs: Vec::new(),
            })
        }
    }

    fn record_participants(
        &self,
        environment_id: &str,
        backend_ids: &[String],
    ) -> Result<(), BackendError> {
        let mut participants = self
            .participants
            .lock()
            .map_err(|_| BackendError::new("Runtime participant lock poisoned"))?;
        participants
            .entry(environment_id.to_string())
            .or_default()
            .extend(backend_ids.iter().cloned());
        Ok(())
    }

    fn participants_for(&self, environment_id: &str) -> Option<BTreeSet<String>> {
        self.participants
            .lock()
            .ok()?
            .get(environment_id)
            .filter(|participants| !participants.is_empty())
            .cloned()
    }

    fn aggregate_state(&self, environment_id: &str) -> Option<EnvironmentState> {
        // 有参与者记录时以参与者为准：任一参与者缺状态或状态不一致都判为
        // PartiallyAvailable，避免某个参与建立但未报告状态的 Backend 被静默丢弃。
        if let Some(participants) = self.participants_for(environment_id) {
            let mut states = Vec::with_capacity(participants.len());
            for id in &participants {
                let Some(state) = self
                    .connections
                    .iter()
                    .find(|connection| &connection.id == id)
                    .and_then(|connection| connection.backend.state(environment_id))
                else {
                    return Some(EnvironmentState::PartiallyAvailable);
                };
                states.push(state);
            }
            let first = states[0];
            return states
                .iter()
                .all(|state| *state == first)
                .then_some(first)
                .or(Some(EnvironmentState::PartiallyAvailable));
        }
        // 没有参与者记录（未知环境或旧路径）：保持观察所有报告状态的 Backend 的兼容行为。
        let states: Vec<_> = self
            .connections
            .iter()
            .filter_map(|connection| connection.backend.state(environment_id))
            .collect();
        if states.is_empty() {
            return None;
        }
        let first = states[0];
        states
            .iter()
            .all(|state| *state == first)
            .then_some(first)
            .or(Some(EnvironmentState::PartiallyAvailable))
    }

    fn merged_capabilities(&self) -> RuntimeCapabilities {
        let mut tools = BTreeMap::<String, RuntimeTool>::new();
        let mut conflicting_tools = BTreeSet::new();
        let mut shared_environment = false;
        let mut branch_environment = true;
        let mut filesystem_fast_delete: Option<bool> = None;
        let mut fast_delete_conflict = false;
        let mut filesystem_fork_cost: Option<String> = None;
        let mut fork_cost_conflict = false;
        let mut filesystem_btrfs_incremental_diff: Option<bool> = None;
        let mut incremental_conflict = false;
        let mut freeze_mechanism: Option<String> = None;
        let mut freeze_conflict = false;
        let mut filesystem_passthrough: Option<Vec<String>> = None;
        let mut passthrough_conflict = false;
        for connection in &self.connections {
            let capabilities = connection.backend.capabilities();
            shared_environment |= capabilities.shared_environment;
            branch_environment &= capabilities.branch_environment;
            if let Some(value) = capabilities.filesystem_fast_delete {
                match filesystem_fast_delete {
                    None => filesystem_fast_delete = Some(value),
                    Some(previous) if previous != value => fast_delete_conflict = true,
                    _ => {}
                }
            }
            if let Some(value) = capabilities.filesystem_fork_cost {
                match &filesystem_fork_cost {
                    None => filesystem_fork_cost = Some(value),
                    Some(previous) if previous != &value => fork_cost_conflict = true,
                    _ => {}
                }
            }
            if let Some(value) = capabilities.filesystem_btrfs_incremental_diff {
                match filesystem_btrfs_incremental_diff {
                    None => filesystem_btrfs_incremental_diff = Some(value),
                    Some(previous) if previous != value => incremental_conflict = true,
                    _ => {}
                }
            }
            if let Some(value) = capabilities.freeze_mechanism {
                match &freeze_mechanism {
                    None => freeze_mechanism = Some(value),
                    Some(previous) if previous != &value => freeze_conflict = true,
                    _ => {}
                }
            }
            if let Some(value) = capabilities.filesystem_passthrough {
                match &filesystem_passthrough {
                    None => filesystem_passthrough = Some(value),
                    Some(previous) if previous != &value => passthrough_conflict = true,
                    _ => {}
                }
            }
            for tool in capabilities.tools {
                if let Some(previous) = tools.get(&tool.name)
                    && previous != &tool
                {
                    conflicting_tools.insert(tool.name.clone());
                } else {
                    tools.insert(tool.name.clone(), tool);
                }
            }
        }
        for name in conflicting_tools {
            tools.remove(&name);
        }
        RuntimeCapabilities {
            tools: tools.into_values().collect(),
            shared_environment,
            branch_environment,
            filesystem_fast_delete: if fast_delete_conflict {
                None
            } else {
                filesystem_fast_delete
            },
            filesystem_fork_cost: if fork_cost_conflict {
                None
            } else {
                filesystem_fork_cost
            },
            filesystem_btrfs_incremental_diff: if incremental_conflict {
                None
            } else {
                filesystem_btrfs_incremental_diff
            },
            freeze_mechanism: if freeze_conflict {
                None
            } else {
                freeze_mechanism
            },
            filesystem_passthrough: if passthrough_conflict {
                None
            } else {
                filesystem_passthrough
            },
        }
    }
}

fn normalize_outputs(
    backend_id: &str,
    outputs: Vec<crate::contract::BackendOutput>,
) -> Vec<crate::contract::BackendOutput> {
    outputs
        .into_iter()
        .map(|mut output| {
            if output.backend_id == "backend" {
                output.backend_id = backend_id.to_string();
            } else if output.backend_id != backend_id
                && !output.backend_id.starts_with(&format!("{backend_id}/"))
            {
                output.backend_id = format!("{backend_id}/{}", output.backend_id);
            }
            output
        })
        .collect()
}

impl Backend for CompositeRuntime {
    fn backend_id(&self) -> &str {
        &self.id
    }

    fn backend_ids(&self) -> Vec<String> {
        self.connections
            .iter()
            .map(|connection| connection.id.clone())
            .collect()
    }

    fn capabilities(&self) -> RuntimeCapabilities {
        self.merged_capabilities()
    }

    fn launch_fragment(
        &self,
        context: &BackendContext,
        launch: &AgentLaunch,
    ) -> Result<LaunchFragment, BackendError> {
        let (connections, _) = self.selected(context, BackendOperation::Operate)?;
        let mut fragment = LaunchFragment::default();
        for connection in connections {
            merge_launch_fragment(
                &mut fragment,
                connection.backend.launch_fragment(context, launch)?,
            );
        }
        Ok(fragment)
    }

    fn establish(&self, environment_id: &str) -> Result<(), BackendError> {
        self.establish_with_context(&BackendContext::new(environment_id))
    }

    fn establish_with_context(&self, context: &BackendContext) -> Result<(), BackendError> {
        self.establish_all(context)
    }

    fn operate(
        &self,
        environment_id: &str,
        command: &[String],
        cancel: &CancelHandle,
    ) -> Result<CommandOutput, BackendError> {
        let execution =
            self.operate_with_context(&BackendContext::new(environment_id), command, cancel)?;
        execution.single_output().ok_or_else(|| {
            BackendError::new(
                "multiple Backend outputs require an explicit output_backend selection",
            )
            .with_outputs(execution.outputs)
        })
    }

    fn operate_with_context(
        &self,
        context: &BackendContext,
        command: &[String],
        cancel: &CancelHandle,
    ) -> Result<BackendExecution, BackendError> {
        self.execute_outputs(
            context,
            BackendOperation::Operate,
            cancel,
            |connection, context, cancel| {
                connection
                    .backend
                    .operate_with_context(context, command, cancel)
            },
        )
    }

    fn tool_context(
        &self,
        environment_id: &str,
        tool_name: &str,
        cancel: &CancelHandle,
    ) -> Result<CommandOutput, BackendError> {
        let execution = self.tool_context_with_context(
            &BackendContext::new(environment_id),
            tool_name,
            cancel,
        )?;
        execution.single_output().ok_or_else(|| {
            BackendError::new(
                "multiple Backend outputs require an explicit output_backend selection",
            )
            .with_outputs(execution.outputs)
        })
    }

    fn tool_context_with_context(
        &self,
        context: &BackendContext,
        tool_name: &str,
        cancel: &CancelHandle,
    ) -> Result<BackendExecution, BackendError> {
        self.execute_outputs(
            context,
            BackendOperation::ToolContext,
            cancel,
            |connection, context, cancel| {
                connection
                    .backend
                    .tool_context_with_context(context, tool_name, cancel)
            },
        )
    }

    fn invoke_tool(
        &self,
        environment_id: &str,
        tool_name: &str,
        arguments: &Value,
        cancel: &CancelHandle,
    ) -> Result<CommandOutput, BackendError> {
        let execution = self.invoke_tool_with_context(
            &BackendContext::new(environment_id),
            tool_name,
            arguments,
            cancel,
        )?;
        execution.single_output().ok_or_else(|| {
            BackendError::new(
                "multiple Backend outputs require an explicit output_backend selection",
            )
            .with_outputs(execution.outputs)
        })
    }

    fn invoke_tool_with_context(
        &self,
        context: &BackendContext,
        tool_name: &str,
        arguments: &Value,
        cancel: &CancelHandle,
    ) -> Result<BackendExecution, BackendError> {
        self.execute_outputs(
            context,
            BackendOperation::InvokeTool,
            cancel,
            |connection, context, cancel| {
                connection
                    .backend
                    .invoke_tool_with_context(context, tool_name, arguments, cancel)
            },
        )
    }

    fn filesystem_operation(
        &self,
        context: &BackendContext,
        operation: &Operation,
        cancel: &CancelHandle,
    ) -> Result<FilesystemResult, BackendError> {
        let backend_operation = BackendOperation::for_operation(operation);
        if !matches!(
            backend_operation,
            BackendOperation::FilesystemFork
                | BackendOperation::FilesystemMark
                | BackendOperation::FilesystemDiff
                | BackendOperation::FilesystemAccept
                | BackendOperation::FilesystemDiscard
        ) {
            return Err(BackendError::new(
                "filesystem dispatch requires a filesystem operation",
            ));
        }
        if cancel.is_cancelled() {
            return Err(BackendError::interrupted());
        }
        let (connections, _) = self.selected(context, backend_operation)?;
        if connections.len() != 1 {
            return Err(BackendError::new("filesystem operation requires exactly one selected Backend"));
        }
        connections[0].backend.filesystem_operation(context, operation, cancel)
    }

    fn branch(&self, parent: &str, child: &str) -> Result<(), BackendError> {
        let context = BackendContext {
            environment_id: child.to_string(),
            parent_environment_id: Some(parent.to_string()),
            ..BackendContext::default()
        };
        self.branch_with_context(&context, parent, child)
    }

    fn branch_with_context(
        &self,
        context: &BackendContext,
        parent: &str,
        child: &str,
    ) -> Result<(), BackendError> {
        let (connections, _) = self.selected(context, BackendOperation::Branch)?;
        let ids: Vec<_> = connections
            .iter()
            .map(|connection| connection.id.clone())
            .collect();
        let child_context = BackendContext {
            operation_id: context.operation_id.clone(),
            generation: context.generation,
            environment_id: child.to_string(),
            parent_environment_id: Some(parent.to_string()),
            task_id: context.task_id.clone(),
            branch_id: context.branch_id.clone(),
            target_mount_pid: context.target_mount_pid,
            target_mount_namespace: context.target_mount_namespace,
        };
        let mut completed: Vec<&BackendConnection> = Vec::new();
        for (index, connection) in connections.iter().enumerate() {
            match connection
                .backend
                .branch_with_context(&child_context, parent, child)
            {
                Ok(()) => completed.push(connection),
                Err(mut error) => {
                    let mut cleanup_failures = Vec::new();
                    for previous in completed.iter().rev() {
                        if let Err(cleanup) = previous.backend.cleanup_with_context(&child_context)
                        {
                            cleanup_failures.push(format!("{}: {}", previous.id, cleanup.message));
                        }
                    }
                    error.message = format!(
                        "Backend {} failed during branch: {}",
                        connection.id, error.message
                    );
                    error.completed = completed.iter().map(|value| value.id.clone()).collect();
                    error.remaining = ids[index..].to_vec();
                    error.cleanup_failures.extend(cleanup_failures);
                    return Err(error);
                }
            }
        }
        // 只有全部 Backend 派生成功才登记子环境参与者；失败已回滚并返回错误。
        self.record_participants(child, &ids)?;
        Ok(())
    }

    fn promote(&self, environment_id: &str) -> Result<(), BackendError> {
        self.promote_with_context(&BackendContext::new(environment_id))
    }

    fn promote_with_context(&self, context: &BackendContext) -> Result<(), BackendError> {
        self.execute_lifecycle(context, BackendOperation::Promote, |connection, context| {
            connection.backend.promote_with_context(context)
        })
    }

    fn suspend(&self, environment_id: &str) -> Result<(), BackendError> {
        self.suspend_with_context(&BackendContext::new(environment_id))
    }

    fn suspend_with_context(&self, context: &BackendContext) -> Result<(), BackendError> {
        self.execute_lifecycle(context, BackendOperation::Suspend, |connection, context| {
            connection.backend.suspend_with_context(context)
        })
    }

    fn resume(&self, environment_id: &str) -> Result<(), BackendError> {
        self.resume_with_context(&BackendContext::new(environment_id))
    }

    fn resume_with_context(&self, context: &BackendContext) -> Result<(), BackendError> {
        self.execute_lifecycle(context, BackendOperation::Resume, |connection, context| {
            connection.backend.resume_with_context(context)
        })
    }

    fn terminate(&self, environment_id: &str) -> Result<(), BackendError> {
        self.terminate_with_context(&BackendContext::new(environment_id))
    }

    fn terminate_with_context(&self, context: &BackendContext) -> Result<(), BackendError> {
        self.execute_lifecycle(
            context,
            BackendOperation::Terminate,
            |connection, context| connection.backend.terminate_with_context(context),
        )
    }

    fn cleanup(&self, environment_id: &str) -> Result<(), BackendError> {
        self.cleanup_with_context(&BackendContext::new(environment_id))
    }

    fn cleanup_with_context(&self, context: &BackendContext) -> Result<(), BackendError> {
        self.cleanup_all(context)
    }

    fn state(&self, environment_id: &str) -> Option<EnvironmentState> {
        self.aggregate_state(environment_id)
    }

    fn backend_states(&self, environment_id: &str) -> Vec<BackendState> {
        // 有参与者记录时只返回参与者快照，缺失状态用 None，便于控制面诊断；
        // 否则保持返回所有 Backend 的旧行为。
        if let Some(participants) = self.participants_for(environment_id) {
            return self
                .connections
                .iter()
                .filter(|connection| participants.contains(&connection.id))
                .map(|connection| BackendState {
                    backend_id: connection.id.clone(),
                    state: connection.backend.state(environment_id),
                })
                .collect();
        }
        self.connections
            .iter()
            .map(|connection| BackendState {
                backend_id: connection.id.clone(),
                state: connection.backend.state(environment_id),
            })
            .collect()
    }

    fn environments(&self) -> Vec<String> {
        let mut environments = BTreeSet::new();
        for connection in &self.connections {
            environments.extend(connection.backend.environments());
        }
        environments.into_iter().collect()
    }
}


/// Base Runtime：Runtime 的特殊实现，负责一个 Agent 实际执行实例的
/// 进程生命周期。Agent Session、会话恢复和工具协议仍由接入侧负责。
pub struct BaseRuntime {
    id: String,
    inner: Arc<dyn Backend>,
    filesystem: Option<Arc<FilesystemBackend>>,
    process: ProcessBackend,
}

impl BaseRuntime {
    pub fn new(id: impl Into<String>, inner: Arc<dyn Backend>) -> Self {
        Self {
            id: id.into(),
            inner,
            filesystem: None,
            process: ProcessBackend::new("process", "bwrap"),
        }
    }

    /// Wrap Agent startup with a filesystem backend. The backend remains
    /// filesystem-only; this Runtime wrapper only delegates process lifetime.
    pub fn with_filesystem(mut self, filesystem: Arc<FilesystemBackend>) -> Self {
        self.filesystem = Some(filesystem);
        self
    }

    pub fn with_bubblewrap(mut self, path: impl Into<PathBuf>) -> Self {
        self.process = ProcessBackend::new("process", path);
        self
    }

    fn launch_agent_process(
        &self,
        context: &BackendContext,
        launch: &AgentLaunch,
    ) -> Result<(), BackendError> {
        let mut fragment = self.inner.launch_fragment(context, launch)?;
        if let Some(filesystem) = &self.filesystem {
            merge_launch_fragment(&mut fragment, filesystem.launch_fragment(context, launch)?);
        }
        self.process.launch(context, launch, &fragment)
    }

    fn stop_agent_process(&self) -> Result<(), BackendError> {
        self.process.stop_all();
        Ok(())
    }

    pub fn process_handle(&self, environment_id: &str) -> Option<ProcessHandle> {
        self.process.process_handle(environment_id)
    }

    fn ensure_filesystem_available(&self, context: &BackendContext) -> Result<(), BackendError> {
        if let Some(filesystem) = &self.filesystem {
            if filesystem.state(&context.environment_id) != Some(EnvironmentState::Available) {
                return Err(BackendError::new(
                    "filesystem environment is not available for operation",
                ));
            }
        }
        Ok(())
    }

    fn ensure_filesystem_state(
        &self,
        context: &BackendContext,
        expected: EnvironmentState,
    ) -> Result<(), BackendError> {
        if let Some(filesystem) = &self.filesystem {
            if filesystem.state(&context.environment_id) != Some(expected) {
                return Err(BackendError::new(format!(
                    "filesystem environment is not in required state: expected {expected:?}"
                )));
            }
        }
        Ok(())
    }
}

impl Drop for BaseRuntime {
    fn drop(&mut self) {
        let _ = self.stop_agent_process();
    }
}

impl Backend for BaseRuntime {
    fn backend_id(&self) -> &str {
        &self.id
    }

    fn backend_ids(&self) -> Vec<String> {
        let mut ids = self.inner.backend_ids();
        if self.filesystem.is_some() {
            ids.push("filesystem".to_string());
        }
        ids.push("process".to_string());
        ids
    }

    fn capabilities(&self) -> RuntimeCapabilities {
        let mut capabilities = self.inner.capabilities();
        if let Some(filesystem) = &self.filesystem {
            let filesystem_capabilities = filesystem.capabilities();
            capabilities.branch_environment &= filesystem_capabilities.branch_environment;
            capabilities.shared_environment |= filesystem_capabilities.shared_environment;
            capabilities.filesystem_fast_delete = filesystem_capabilities.filesystem_fast_delete;
            capabilities.filesystem_fork_cost = filesystem_capabilities.filesystem_fork_cost;
            capabilities.filesystem_btrfs_incremental_diff =
                filesystem_capabilities.filesystem_btrfs_incremental_diff;
            capabilities.filesystem_passthrough = filesystem_capabilities.filesystem_passthrough;
        }
        capabilities.freeze_mechanism =
            Some(self.process.freeze_mechanism().as_str().to_string());
        capabilities
    }

    fn establish(&self, environment_id: &str) -> Result<(), BackendError> {
        self.establish_with_context(&BackendContext::new(environment_id))
    }

    fn establish_with_context(&self, context: &BackendContext) -> Result<(), BackendError> {
        if let Some(filesystem) = &self.filesystem {
            filesystem.establish_with_context(context)?;
        }
        if let Err(error) = self.inner.establish_with_context(context) {
            if let Some(filesystem) = &self.filesystem {
                let mut error = error;
                if let Err(terminate) = filesystem.terminate_with_context(context) {
                    error.add_cleanup_failure("filesystem", format!("terminate: {terminate}"));
                }
                if let Err(cleanup) = filesystem.cleanup_with_context(context) {
                    error.add_cleanup_failure("filesystem", format!("cleanup: {cleanup}"));
                }
                return Err(error);
            }
            return Err(error);
        }
        Ok(())
    }

    fn operate(
        &self,
        environment_id: &str,
        command: &[String],
        cancel: &CancelHandle,
    ) -> Result<CommandOutput, BackendError> {
        let execution =
            self.operate_with_context(&BackendContext::new(environment_id), command, cancel)?;
        execution.single_output().ok_or_else(|| {
            BackendError::new(
                "multiple Backend outputs require an explicit output_backend selection",
            )
            .with_outputs(execution.outputs)
        })
    }

    fn operate_with_context(
        &self,
        context: &BackendContext,
        command: &[String],
        cancel: &CancelHandle,
    ) -> Result<BackendExecution, BackendError> {
        // Filesystem is not a second business command executor. Its state
        // check prevents direct Backend callers from bypassing suspend, while
        // the command itself remains owned by the inner Runtime.
        self.ensure_filesystem_available(context)?;
        self.inner.operate_with_context(context, command, cancel)
    }

    fn tool_context(
        &self,
        environment_id: &str,
        tool_name: &str,
        cancel: &CancelHandle,
    ) -> Result<CommandOutput, BackendError> {
        let execution = self.tool_context_with_context(
            &BackendContext::new(environment_id),
            tool_name,
            cancel,
        )?;
        execution.single_output().ok_or_else(|| {
            BackendError::new(
                "multiple Backend outputs require an explicit output_backend selection",
            )
            .with_outputs(execution.outputs)
        })
    }

    fn tool_context_with_context(
        &self,
        context: &BackendContext,
        tool_name: &str,
        cancel: &CancelHandle,
    ) -> Result<BackendExecution, BackendError> {
        self.ensure_filesystem_available(context)?;
        self.inner
            .tool_context_with_context(context, tool_name, cancel)
    }

    fn invoke_tool(
        &self,
        environment_id: &str,
        tool_name: &str,
        arguments: &Value,
        cancel: &CancelHandle,
    ) -> Result<CommandOutput, BackendError> {
        let execution = self.invoke_tool_with_context(
            &BackendContext::new(environment_id),
            tool_name,
            arguments,
            cancel,
        )?;
        execution.single_output().ok_or_else(|| {
            BackendError::new(
                "multiple Backend outputs require an explicit output_backend selection",
            )
            .with_outputs(execution.outputs)
        })
    }

    fn invoke_tool_with_context(
        &self,
        context: &BackendContext,
        tool_name: &str,
        arguments: &Value,
        cancel: &CancelHandle,
    ) -> Result<BackendExecution, BackendError> {
        self.ensure_filesystem_available(context)?;
        self.inner
            .invoke_tool_with_context(context, tool_name, arguments, cancel)
    }

    fn branch(&self, parent: &str, child: &str) -> Result<(), BackendError> {
        self.branch_with_context(
            &BackendContext {
                environment_id: child.to_string(),
                parent_environment_id: Some(parent.to_string()),
                ..BackendContext::default()
            },
            parent,
            child,
        )
    }

    fn branch_with_context(
        &self,
        context: &BackendContext,
        parent: &str,
        child: &str,
    ) -> Result<(), BackendError> {
        if let Some(filesystem) = &self.filesystem {
            filesystem.branch_with_context(context, parent, child)?;
        }
        if let Err(error) = self.inner.branch_with_context(context, parent, child) {
            if let Some(filesystem) = &self.filesystem {
                let child_context = BackendContext {
                    environment_id: child.to_string(),
                    ..context.clone()
                };
                let mut error = error;
                if let Err(terminate) = filesystem.terminate_with_context(&child_context) {
                    error.add_cleanup_failure("filesystem", format!("terminate: {terminate}"));
                }
                if let Err(cleanup) = filesystem.cleanup_with_context(&child_context) {
                    error.add_cleanup_failure("filesystem", format!("cleanup: {cleanup}"));
                }
                return Err(error);
            }
            return Err(error);
        }
        Ok(())
    }

    fn promote(&self, environment_id: &str) -> Result<(), BackendError> {
        self.promote_with_context(&BackendContext::new(environment_id))
    }

    fn promote_with_context(&self, context: &BackendContext) -> Result<(), BackendError> {
        self.ensure_filesystem_state(context, EnvironmentState::Available)?;
        self.inner.promote_with_context(context)?;
        if let Some(filesystem) = &self.filesystem {
            filesystem.promote_with_context(context)?;
        }
        Ok(())
    }

    fn suspend(&self, environment_id: &str) -> Result<(), BackendError> {
        self.suspend_with_context(&BackendContext::new(environment_id))
    }

    fn suspend_with_context(&self, context: &BackendContext) -> Result<(), BackendError> {
        self.ensure_filesystem_state(context, EnvironmentState::Available)?;
        let running = self
            .process
            .process_handle(&context.environment_id)
            .is_some_and(|_| {
                self.process
                    .is_running(&context.environment_id)
                    .unwrap_or(false)
            });
        if running && self.process.freeze_mechanism() == FreezeMechanism::None {
            return Err(BackendError::new(
                "cannot suspend: no freeze mechanism is available",
            ));
        }
        if running {
            self.process.freeze(&context.environment_id)?;
        }
        let inner = self.inner.suspend_with_context(context);
        if let Err(error) = inner {
            if running {
                let _ = self.process.thaw(&context.environment_id);
            }
            return Err(error);
        }
        if let Some(filesystem) = &self.filesystem
            && let Err(error) = filesystem.suspend_with_context(context)
        {
            if running {
                let _ = self.process.thaw(&context.environment_id);
            }
            return Err(error);
        }
        Ok(())
    }

    fn resume(&self, environment_id: &str) -> Result<(), BackendError> {
        self.resume_with_context(&BackendContext::new(environment_id))
    }

    fn resume_with_context(&self, context: &BackendContext) -> Result<(), BackendError> {
        self.ensure_filesystem_state(context, EnvironmentState::Suspended)?;
        let thaw_needed = self
            .process
            .process_handle(&context.environment_id)
            .is_some()
            && self.process.freeze_mechanism() != FreezeMechanism::None
            && self
                .process
                .is_running(&context.environment_id)
                .unwrap_or(false);
        if thaw_needed {
            self.process.thaw(&context.environment_id)?;
        }
        let inner = self.inner.resume_with_context(context);
        if let Err(error) = inner {
            if thaw_needed {
                let _ = self.process.freeze(&context.environment_id);
            }
            return Err(error);
        }
        if let Some(filesystem) = &self.filesystem
            && let Err(error) = filesystem.resume_with_context(context)
        {
            if thaw_needed {
                let _ = self.process.freeze(&context.environment_id);
            }
            return Err(error);
        }
        Ok(())
    }

    fn terminate(&self, environment_id: &str) -> Result<(), BackendError> {
        self.terminate_with_context(&BackendContext::new(environment_id))
    }

    fn terminate_with_context(&self, context: &BackendContext) -> Result<(), BackendError> {
        let stopped = self
            .process
            .terminate(&context.environment_id)
            .and_then(|_| self.process.stop(&context.environment_id));
        let inner = self.inner.terminate_with_context(context);
        let filesystem = self
            .filesystem
            .as_ref()
            .map(|value| value.terminate_with_context(context));
        let mut failures = Vec::new();
        if let Err(error) = stopped {
            failures.push(format!("process terminate failed: {error}"));
        }
        if let Err(error) = inner {
            failures.push(format!("inner terminate failed: {error}"));
        }
        if let Some(Err(error)) = filesystem {
            failures.push(format!("filesystem terminate failed: {error}"));
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(BackendError::new(failures.join("; ")))
        }
    }

    fn cleanup(&self, environment_id: &str) -> Result<(), BackendError> {
        self.cleanup_with_context(&BackendContext::new(environment_id))
    }

    fn cleanup_with_context(&self, context: &BackendContext) -> Result<(), BackendError> {
        let process = self.process.cleanup(&context.environment_id);
        let inner = self.inner.cleanup_with_context(context);
        let filesystem = self
            .filesystem
            .as_ref()
            .map(|value| value.cleanup_with_context(context));
        let mut failures = Vec::new();
        if let Err(error) = process {
            failures.push(format!("process cleanup failed: {error}"));
        }
        if let Err(error) = inner {
            failures.push(format!("inner cleanup failed: {error}"));
        }
        if let Some(Err(error)) = filesystem {
            failures.push(format!("filesystem cleanup failed: {error}"));
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(BackendError::new(failures.join("; ")))
        }
    }

    fn state(&self, environment_id: &str) -> Option<EnvironmentState> {
        let inner = self.inner.state(environment_id);
        let filesystem = self
            .filesystem
            .as_ref()
            .and_then(|value| value.state(environment_id));
        match (inner, filesystem) {
            (Some(left), Some(right)) if left == right => Some(left),
            (Some(_), Some(_)) => Some(EnvironmentState::PartiallyAvailable),
            (Some(_), None) | (None, Some(_)) => Some(EnvironmentState::PartiallyAvailable),
            (None, None) => None,
        }
    }

    fn backend_states(&self, environment_id: &str) -> Vec<BackendState> {
        let mut states = self.inner.backend_states(environment_id);
        if let Some(filesystem) = &self.filesystem {
            states.push(BackendState {
                backend_id: filesystem.backend_id().to_string(),
                state: filesystem.state(environment_id),
            });
        }
        states
    }

    fn filesystem_operation(
        &self,
        context: &BackendContext,
        operation: &Operation,
        cancel: &CancelHandle,
    ) -> Result<FilesystemResult, BackendError> {
        let mut context = context.clone();
        let freeze_environment = match operation {
            Operation::FilesystemFork {
                parent_snapshot_id, ..
            } => Some(parent_snapshot_id.clone()),
            Operation::FilesystemMark { snapshot_id, .. } => Some(snapshot_id.clone()),
            Operation::FilesystemAccept {
                target_snapshot_id, ..
            } => Some(target_snapshot_id.clone()),
            _ => None,
        };
        let target = match operation {
            Operation::FilesystemAccept {
                target_snapshot_id, ..
            } => Some(target_snapshot_id.as_str()),
            _ => None,
        };
        let mut frozen = false;
        if let Some(environment) = freeze_environment.as_deref()
            && let Some(handle) = self.process.process_handle(environment)
        {
            // A process that already exited has no writers left to quiesce.
            // Treat it as "not frozen" and let the filesystem operation run
            // instead of waiting forever on an empty cgroup.
            let running = self.process.is_running(environment).unwrap_or(false);
            if running && self.process.freeze_mechanism() != FreezeMechanism::None {
                self.process.freeze(environment)?;
                frozen = true;
                if target.is_some() {
                    context.target_mount_pid = Some(handle.inner_pid);
                    context.target_mount_namespace = Some(handle.mount_namespace);
                }
            }
        }
        let result = if let Some(filesystem) = &self.filesystem {
            filesystem.filesystem_operation(&context, operation, cancel)
        } else {
            self.inner.filesystem_operation(&context, operation, cancel)
        };
        let thaw = if frozen {
            self.process
                .thaw(freeze_environment.as_deref().unwrap())
                .err()
        } else {
            None
        };
        match result {
            Ok(mut result) => {
                if frozen && thaw.is_none() {
                    result.confirmation.writers_quiesced = true;
                    if target.is_some() {
                        result.confirmation.target_paused = true;
                    }
                }
                if let Some(error) = thaw {
                    let mut error = BackendError::new(format!(
                        "filesystem operation succeeded but process thaw failed: {error}"
                    ));
                    error.add_cleanup_failure("process", "thaw after filesystem operation");
                    return Err(error);
                }
                Ok(result)
            }
            Err(mut error) => {
                if let Some(thaw) = thaw {
                    error.add_cleanup_failure("process", format!("thaw: {thaw}"));
                }
                Err(error)
            }
        }
    }

    fn launch_fragment(
        &self,
        context: &BackendContext,
        launch: &AgentLaunch,
    ) -> Result<LaunchFragment, BackendError> {
        let mut fragment = self.inner.launch_fragment(context, launch)?;
        if let Some(filesystem) = &self.filesystem {
            merge_launch_fragment(&mut fragment, filesystem.launch_fragment(context, launch)?);
        }
        Ok(fragment)
    }

    fn environments(&self) -> Vec<String> {
        let mut environments = self.inner.environments();
        if let Some(filesystem) = &self.filesystem {
            environments.extend(filesystem.environments());
        }
        environments.sort();
        environments.dedup();
        environments
    }

    fn manages_agent_process(&self) -> bool {
        true
    }

    fn launch_agent(
        &self,
        context: &BackendContext,
        launch: &AgentLaunch,
    ) -> Result<(), BackendError> {
        self.launch_agent_process(context, launch)
    }

    fn stop_agents(&self) -> Result<(), BackendError> {
        self.stop_agent_process()
    }
}

fn merge_launch_fragment(target: &mut LaunchFragment, source: LaunchFragment) {
    target.mounts.extend(source.mounts);
    if target.chdir.is_none() {
        target.chdir = source.chdir;
    }
    target.env.extend(source.env);
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{FilesystemConfirmation, FilesystemResult};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct MockBackend {
        id: String,
        calls: AtomicUsize,
        seen: Mutex<Vec<(BackendContext, Operation)>>,
    }

    impl MockBackend {
        fn new(id: &str) -> Arc<Self> {
            Arc::new(Self {
                id: id.to_string(),
                calls: AtomicUsize::new(0),
                seen: Mutex::new(Vec::new()),
            })
        }
    }

    impl Backend for MockBackend {
        fn backend_id(&self) -> &str { &self.id }
        fn establish(&self, _: &str) -> Result<(), BackendError> { Ok(()) }
        fn operate(&self, _: &str, _: &[String], _: &CancelHandle) -> Result<CommandOutput, BackendError> {
            Ok(CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() })
        }
        fn branch(&self, _: &str, _: &str) -> Result<(), BackendError> { Ok(()) }
        fn suspend(&self, _: &str) -> Result<(), BackendError> { Ok(()) }
        fn resume(&self, _: &str) -> Result<(), BackendError> { Ok(()) }
        fn terminate(&self, _: &str) -> Result<(), BackendError> { Ok(()) }
        fn cleanup(&self, _: &str) -> Result<(), BackendError> { Ok(()) }
        fn state(&self, _: &str) -> Option<EnvironmentState> { Some(EnvironmentState::Available) }
        fn environments(&self) -> Vec<String> { vec!["env".to_string()] }

        fn filesystem_operation(
            &self,
            context: &BackendContext,
            operation: &Operation,
            _: &CancelHandle,
        ) -> Result<FilesystemResult, BackendError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.seen.lock().unwrap().push((context.clone(), operation.clone()));
            Ok(FilesystemResult {
                snapshot_id: Some(self.id.clone()),
                confirmation: FilesystemConfirmation {
                    snapshot_created: true,
                    metadata_durable: true,
                    published: true,
                    ..Default::default()
                },
                ..Default::default()
            })
        }
    }

    fn fork_operation() -> Operation {
        Operation::FilesystemFork {
            parent_snapshot_id: "parent".into(),
            child_snapshot_id: "child".into(),
            path_scope: vec![],
        }
    }

    #[test]
    fn filesystem_policy_selects_backend_and_forwards_context_and_operation() {
        let left = MockBackend::new("left");
        let right = MockBackend::new("right");
        let runtime = CompositeRuntime::new(
            "runtime",
            vec![
                BackendConnection::new("left", left.clone()).unwrap(),
                BackendConnection::new("right", right.clone()).unwrap(),
            ],
            BackendPolicy {
                default: vec!["right".into()],
                routes: vec![BackendRoute {
                    operation: Some(BackendOperation::FilesystemFork),
                    task_id: Some("task".into()),
                    backends: vec!["left".into()],
                    ..Default::default()
                }],
            },
        ).unwrap();
        let context = BackendContext {
            task_id: Some("task".into()),
            environment_id: "env".into(),
            ..Default::default()
        };
        let cancel = CancelHandle::default();
        runtime.filesystem_operation(&context, &fork_operation(), &cancel).unwrap();
        assert_eq!(left.calls.load(Ordering::SeqCst), 1);
        assert_eq!(right.calls.load(Ordering::SeqCst), 0);
        let seen = left.seen.lock().unwrap();
        assert_eq!(seen[0].0, context);
        assert_eq!(seen[0].1, fork_operation());
    }

    #[test]
    fn filesystem_multiple_selection_and_cancellation_make_no_calls() {
        let left = MockBackend::new("left");
        let right = MockBackend::new("right");
        let runtime = CompositeRuntime::new(
            "runtime",
            vec![
                BackendConnection::new("left", left.clone()).unwrap(),
                BackendConnection::new("right", right.clone()).unwrap(),
            ],
            BackendPolicy { default: vec!["left".into(), "right".into()], routes: vec![] },
        ).unwrap();
        let context = BackendContext::new("env");
        let cancel = CancelHandle::default();
        assert!(runtime.filesystem_operation(&context, &fork_operation(), &cancel).is_err());
        cancel.cancel();
        assert!(runtime.filesystem_operation(&context, &fork_operation(), &cancel).is_err());
        assert_eq!(left.calls.load(Ordering::SeqCst), 0);
        assert_eq!(right.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn base_runtime_freezes_and_marks_writers_quiesced() {
        let backend = MockBackend::new("mock");
        let runtime = BaseRuntime::new("runtime", backend);
        let context = BackendContext::new("env");
        let log = PathBuf::from(format!(
            "/tmp/opencode/ascout-freeze-test-{}.log",
            std::process::id()
        ));
        let launch = AgentLaunch {
            command: "/bin/sleep".into(),
            args: vec!["5".into()],
            cwd: PathBuf::from("/tmp"),
            log: log.clone(),
            instance_id: Some("test-agent".into()),
            env: BTreeMap::new(),
        };
        runtime.launch_agent(&context, &launch).unwrap();
        let operation = Operation::FilesystemFork {
            parent_snapshot_id: "env".into(),
            child_snapshot_id: "child".into(),
            path_scope: vec![],
        };
        let result = runtime
            .filesystem_operation(&context, &operation, &CancelHandle::default())
            .unwrap();
        assert!(
            result.confirmation.writers_quiesced,
            "freeze must be reflected in writers_quiesced"
        );
        runtime.stop_agents().unwrap();
        let _ = std::fs::remove_file(log);

        let result = runtime
            .filesystem_operation(
                &BackendContext::new("missing"),
                &Operation::FilesystemFork {
                    parent_snapshot_id: "missing".into(),
                    child_snapshot_id: "child".into(),
                    path_scope: vec![],
                },
                &CancelHandle::default(),
            )
            .unwrap();
        assert!(!result.confirmation.writers_quiesced);
    }

    fn launch_test_agent(runtime: &BaseRuntime, command: &str, args: Vec<String>) -> BackendContext {
        let context = BackendContext::new("env");
        let log = PathBuf::from(format!(
            "/tmp/opencode/ascout-freeze-test-{}-{}.log",
            std::process::id(),
            command.replace('/', "_")
        ));
        let launch = AgentLaunch {
            command: command.to_string(),
            args,
            cwd: PathBuf::from("/tmp"),
            log: log.clone(),
            instance_id: Some("test-agent".into()),
            env: BTreeMap::new(),
        };
        runtime.launch_agent(&context, &launch).unwrap();
        context
    }

    fn env_fork_operation() -> Operation {
        Operation::FilesystemFork {
            parent_snapshot_id: "env".into(),
            child_snapshot_id: "child".into(),
            path_scope: vec![],
        }
    }

    #[test]
    fn base_runtime_without_freeze_mechanism_still_runs_operations() {
        let mut runtime = BaseRuntime::new("runtime", MockBackend::new("mock"));
        runtime.process = ProcessBackend::with_mechanism(
            "process",
            "bwrap",
            FreezeMechanism::None,
        );
        let context = launch_test_agent(&runtime, "/bin/sleep", vec!["3".into()]);
        let result = runtime
            .filesystem_operation(&context, &env_fork_operation(), &CancelHandle::default())
            .unwrap();
        assert!(!result.confirmation.writers_quiesced);
        runtime.stop_agents().unwrap();
    }

    #[test]
    fn freeze_failure_fails_before_filesystem_operation() {
        let runtime = BaseRuntime::new("runtime", MockBackend::new("mock"));
        let context = launch_test_agent(&runtime, "/bin/sleep", vec!["3".into()]);
        runtime.process.inject_freeze_failure(true);
        let error = runtime
            .filesystem_operation(&context, &env_fork_operation(), &CancelHandle::default())
            .unwrap_err();
        assert!(
            error.message.contains("injected freeze failure"),
            "{}",
            error.message
        );
        runtime.process.inject_freeze_failure(false);
        runtime.stop_agents().unwrap();
    }

    #[test]
    fn thaw_failure_is_reported_as_cleanup_failure() {
        let runtime = BaseRuntime::new("runtime", MockBackend::new("mock"));
        let context = launch_test_agent(&runtime, "/bin/sleep", vec!["3".into()]);
        runtime.process.inject_thaw_failure(true);
        let error = runtime
            .filesystem_operation(&context, &env_fork_operation(), &CancelHandle::default())
            .unwrap_err();
        assert!(error.message.contains("thaw"), "{}", error.message);
        assert!(
            error
                .cleanup_failures
                .iter()
                .any(|value| value.contains("thaw")),
            "missing thaw cleanup failure: {:?}",
            error.cleanup_failures
        );
        runtime.process.inject_thaw_failure(false);
        runtime.stop_agents().unwrap();
    }

    #[test]
    fn exited_process_does_not_block_filesystem_operation() {
        let runtime = BaseRuntime::new("runtime", MockBackend::new("mock"));
        let context = launch_test_agent(&runtime, "/bin/true", Vec::new());
        for _ in 0..200 {
            if !runtime.process.is_running("env").unwrap() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(!runtime.process.is_running("env").unwrap());
        assert!(runtime.process.process_handle("env").is_some());
        let result = runtime
            .filesystem_operation(&context, &env_fork_operation(), &CancelHandle::default())
            .unwrap();
        assert!(!result.confirmation.writers_quiesced);
        runtime.stop_agents().unwrap();
    }

    #[test]
    fn backend_default_filesystem_operation_is_unsupported() {
        struct DefaultBackend;
        impl Backend for DefaultBackend {
            fn establish(&self, _: &str) -> Result<(), BackendError> { Ok(()) }
            fn operate(&self, _: &str, _: &[String], _: &CancelHandle) -> Result<CommandOutput, BackendError> {
                Err(BackendError::new("unused"))
            }
            fn branch(&self, _: &str, _: &str) -> Result<(), BackendError> { Ok(()) }
            fn suspend(&self, _: &str) -> Result<(), BackendError> { Ok(()) }
            fn resume(&self, _: &str) -> Result<(), BackendError> { Ok(()) }
            fn terminate(&self, _: &str) -> Result<(), BackendError> { Ok(()) }
            fn cleanup(&self, _: &str) -> Result<(), BackendError> { Ok(()) }
            fn state(&self, _: &str) -> Option<EnvironmentState> { None }
            fn environments(&self) -> Vec<String> { Vec::new() }
        }
        let backend = DefaultBackend;
        let cancel = CancelHandle::default();
        let result = Backend::filesystem_operation(
            &backend,
            &BackendContext::new("env"),
            &Operation::OperateEnvironment {
                command: vec!["true".into()],
                agent: None,
            },
            &cancel,
        );
        assert_eq!(result.unwrap_err().message, "filesystem operation is unsupported by this Backend");
    }
}
