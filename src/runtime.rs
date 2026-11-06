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
use std::fs::OpenOptions;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::backend::{Backend, BackendContext, BackendError, BackendExecution, CancelHandle};
use crate::contract::{
    AgentLaunch, BackendState, CommandOutput, EnvironmentState, Operation, RuntimeCapabilities,
    RuntimeTool,
};

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
        for connection in &self.connections {
            let capabilities = connection.backend.capabilities();
            shared_environment |= capabilities.shared_environment;
            branch_environment &= capabilities.branch_environment;
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
            environment_id: child.to_string(),
            parent_environment_id: Some(parent.to_string()),
            task_id: context.task_id.clone(),
            branch_id: context.branch_id.clone(),
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

struct AgentProcess {
    instance_id: String,
    child: Child,
}

/// Base Runtime：Runtime 的特殊实现，负责一个 Agent 实际执行实例的
/// 进程生命周期。Agent Session、会话恢复和工具协议仍由接入侧负责。
pub struct BaseRuntime {
    id: String,
    inner: Arc<dyn Backend>,
    agent: Mutex<Option<AgentProcess>>,
}

impl BaseRuntime {
    pub fn new(id: impl Into<String>, inner: Arc<dyn Backend>) -> Self {
        Self {
            id: id.into(),
            inner,
            agent: Mutex::new(None),
        }
    }

    fn launch_agent_process(
        &self,
        context: &BackendContext,
        launch: &AgentLaunch,
    ) -> Result<(), BackendError> {
        let instance_id = launch
            .instance_id
            .clone()
            .or_else(|| context.branch_id.clone())
            .unwrap_or_else(|| context.environment_id.clone());
        let mut slot = self
            .agent
            .lock()
            .map_err(|_| BackendError::new("Base Runtime Agent lock poisoned"))?;
        if let Some(process) = slot.as_ref() {
            if process.instance_id == instance_id {
                return Ok(());
            }
            return Err(BackendError::new(format!(
                "Base Runtime already owns Agent instance {}",
                process.instance_id
            )));
        }
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&launch.log)
            .map_err(error)?;
        let mut command = Command::new(&launch.command);
        command
            .args(&launch.args)
            .current_dir(&launch.cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::from(log.try_clone().map_err(error)?))
            .stderr(Stdio::from(log))
            .envs(&launch.env);
        let child = command.spawn().map_err(error)?;
        *slot = Some(AgentProcess { instance_id, child });
        Ok(())
    }

    fn stop_agent_process(&self) -> Result<(), BackendError> {
        let process = self
            .agent
            .lock()
            .map_err(|_| BackendError::new("Base Runtime Agent lock poisoned"))?
            .take();
        if let Some(mut process) = process {
            process.child.kill().map_err(error)?;
            process.child.wait().map_err(error)?;
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
        self.inner.backend_ids()
    }

    fn capabilities(&self) -> RuntimeCapabilities {
        self.inner.capabilities()
    }

    fn establish(&self, environment_id: &str) -> Result<(), BackendError> {
        self.inner.establish(environment_id)
    }

    fn establish_with_context(&self, context: &BackendContext) -> Result<(), BackendError> {
        self.inner.establish_with_context(context)
    }

    fn operate(
        &self,
        environment_id: &str,
        command: &[String],
        cancel: &CancelHandle,
    ) -> Result<CommandOutput, BackendError> {
        self.inner.operate(environment_id, command, cancel)
    }

    fn operate_with_context(
        &self,
        context: &BackendContext,
        command: &[String],
        cancel: &CancelHandle,
    ) -> Result<BackendExecution, BackendError> {
        self.inner.operate_with_context(context, command, cancel)
    }

    fn tool_context(
        &self,
        environment_id: &str,
        tool_name: &str,
        cancel: &CancelHandle,
    ) -> Result<CommandOutput, BackendError> {
        self.inner.tool_context(environment_id, tool_name, cancel)
    }

    fn tool_context_with_context(
        &self,
        context: &BackendContext,
        tool_name: &str,
        cancel: &CancelHandle,
    ) -> Result<BackendExecution, BackendError> {
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
        self.inner
            .invoke_tool(environment_id, tool_name, arguments, cancel)
    }

    fn invoke_tool_with_context(
        &self,
        context: &BackendContext,
        tool_name: &str,
        arguments: &Value,
        cancel: &CancelHandle,
    ) -> Result<BackendExecution, BackendError> {
        self.inner
            .invoke_tool_with_context(context, tool_name, arguments, cancel)
    }

    fn branch(&self, parent: &str, child: &str) -> Result<(), BackendError> {
        self.inner.branch(parent, child)
    }

    fn branch_with_context(
        &self,
        context: &BackendContext,
        parent: &str,
        child: &str,
    ) -> Result<(), BackendError> {
        self.inner.branch_with_context(context, parent, child)
    }

    fn promote(&self, environment_id: &str) -> Result<(), BackendError> {
        self.inner.promote(environment_id)
    }

    fn promote_with_context(&self, context: &BackendContext) -> Result<(), BackendError> {
        self.inner.promote_with_context(context)
    }

    fn suspend(&self, environment_id: &str) -> Result<(), BackendError> {
        self.inner.suspend(environment_id)
    }

    fn suspend_with_context(&self, context: &BackendContext) -> Result<(), BackendError> {
        self.inner.suspend_with_context(context)
    }

    fn resume(&self, environment_id: &str) -> Result<(), BackendError> {
        self.inner.resume(environment_id)
    }

    fn resume_with_context(&self, context: &BackendContext) -> Result<(), BackendError> {
        self.inner.resume_with_context(context)
    }

    fn terminate(&self, environment_id: &str) -> Result<(), BackendError> {
        self.inner.terminate(environment_id)
    }

    fn terminate_with_context(&self, context: &BackendContext) -> Result<(), BackendError> {
        self.inner.terminate_with_context(context)
    }

    fn cleanup(&self, environment_id: &str) -> Result<(), BackendError> {
        self.inner.cleanup(environment_id)
    }

    fn cleanup_with_context(&self, context: &BackendContext) -> Result<(), BackendError> {
        self.inner.cleanup_with_context(context)
    }

    fn state(&self, environment_id: &str) -> Option<EnvironmentState> {
        self.inner.state(environment_id)
    }

    fn backend_states(&self, environment_id: &str) -> Vec<BackendState> {
        self.inner.backend_states(environment_id)
    }

    fn environments(&self) -> Vec<String> {
        self.inner.environments()
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

fn error(error: impl std::fmt::Display) -> BackendError {
    BackendError::new(error.to_string())
}
