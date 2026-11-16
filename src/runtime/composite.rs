//! CompositeRuntime：按 Policy 顺序组合多个 Backend。

use std::collections::{BTreeSet, HashMap};
use std::sync::Mutex;

use serde_json::Value;

use super::backend::{Backend, BackendContext, BackendError, BackendExecution, CancelHandle};
use super::launch_plan::LaunchFragment;
use super::selection::{BackendConnection, BackendOperation, BackendPolicy, merge_launch_fragment};
use crate::protocol::contract::{
    AgentLaunch, BackendState, CommandOutput, EnvironmentState, FilesystemResult, Operation,
    RuntimeCapabilities,
};

pub struct CompositeRuntime {
    id: String,
    connections: Vec<BackendConnection>,
    policy: BackendPolicy,
    /// 每个环境参与过 establish/branch 的 Backend id 集合。成功时记录全部
    /// 参与者；失败时记录仍持有环境状态的参与者，供后续清理。
    /// cleanup/terminate 后仍保留，用于解释最终状态。
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

    fn selected_existing(
        &self,
        context: &BackendContext,
        operation: BackendOperation,
    ) -> Result<(Vec<&BackendConnection>, Option<String>), BackendError> {
        let (selected, output_backend) = self.selected(context, operation)?;
        let participants = self
            .participants_for(&context.environment_id)
            .ok_or_else(|| {
                BackendError::new(format!(
                    "environment {} has no recorded Backend participants",
                    context.environment_id
                ))
            })?;
        let selected_ids: BTreeSet<_> = selected
            .iter()
            .map(|connection| connection.id.as_str())
            .collect();
        let participant_ids: BTreeSet<_> = participants.iter().map(String::as_str).collect();
        if !selected_ids.is_subset(&participant_ids) {
            return Err(BackendError::new(format!(
                "Runtime Policy selection does not match participants for environment {}",
                context.environment_id
            )));
        }
        Ok((selected, output_backend))
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
        let (connections, output_backend) = self.selected_existing(context, operation)?;
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

    fn execute_lifecycle_existing<F>(
        &self,
        context: &BackendContext,
        operation: BackendOperation,
        mut call: F,
    ) -> Result<(), BackendError>
    where
        F: FnMut(&BackendConnection, &BackendContext) -> Result<(), BackendError>,
    {
        let (connections, _) = self.selected_existing(context, operation)?;
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
                    let retained: Vec<_> = connections[..=index]
                        .iter()
                        .filter(|connection| {
                            connection.backend.state(&context.environment_id).is_some()
                        })
                        .map(|connection| connection.id.clone())
                        .collect();
                    if !retained.is_empty() {
                        self.record_participants(&context.environment_id, &retained)?;
                        error.created_environment_id = Some(context.environment_id.clone());
                    }
                    return Err(error);
                }
            }
        }
        // 成功记录全部参与者；失败路径仅记录仍持有环境状态的参与者供清理。
        self.record_participants(&context.environment_id, &ids)?;
        Ok(())
    }

    fn cleanup_all(&self, context: &BackendContext) -> Result<(), BackendError> {
        let participants = self
            .participants_for(&context.environment_id)
            .ok_or_else(|| {
                BackendError::new(format!(
                    "cannot clean environment {} without recorded Backend participants",
                    context.environment_id
                ))
            })?;
        let connections: Vec<_> = self
            .connections
            .iter()
            .filter(|connection| participants.contains(&connection.id))
            .collect();
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
                created_environment_id: None,
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
        // 未知环境不观察或操作未绑定的 Backend。
        None
    }

    fn merged_capabilities(&self) -> RuntimeCapabilities {
        super::capabilities::merged_capabilities(&self.connections)
    }
}

fn normalize_outputs(
    backend_id: &str,
    outputs: Vec<crate::protocol::contract::BackendOutput>,
) -> Vec<crate::protocol::contract::BackendOutput> {
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
    fn materialize(&self, context: &BackendContext) -> Result<FilesystemResult, BackendError> {
        let mut result = FilesystemResult::default();
        let (connections, _) = self.selected_existing(context, BackendOperation::Materialize)?;
        for connection in connections {
            let publication = connection.backend.materialize(context)?;
            result.confirmation.published |= publication.confirmation.published;
            result.conflicts.extend(publication.conflicts);
        }
        Ok(result)
    }
    fn save_runtime(&self, path: &std::path::Path) -> Result<(), BackendError> {
        let backends: Vec<_> = self
            .connections
            .iter()
            .map(|c| (c.id.as_str(), c.backend.as_ref()))
            .collect();
        super::recovery::save(
            path,
            &backends,
            serde_json::json!({"participants":*self.participants.lock().map_err(|_|BackendError::new("Runtime participants poisoned"))?}),
        )
    }

    fn load_runtime(&self, path: &std::path::Path) -> Result<(), BackendError> {
        let backends: Vec<_> = self
            .connections
            .iter()
            .map(|c| (c.id.as_str(), c.backend.as_ref()))
            .collect();
        let record: Value = serde_json::from_slice(
            &std::fs::read(path).map_err(|e| BackendError::new(e.to_string()))?,
        )
        .map_err(|e| BackendError::new(e.to_string()))?;
        let metadata = &record["metadata"];
        let participants: HashMap<String, BTreeSet<String>> =
            serde_json::from_value(metadata["participants"].clone())
                .map_err(|e| BackendError::new(e.to_string()))?;
        for ids in participants.values() {
            if ids
                .iter()
                .any(|id| !self.connections.iter().any(|c| &c.id == id))
            {
                return Err(BackendError::new("Unknown saved runtime participant"));
            }
        }
        super::recovery::load(path, &backends)?;
        let validation = (|| {
            for (environment, ids) in &participants {
                if ids.is_empty() {
                    return Err(BackendError::new("Empty saved runtime participant set"));
                }
                for connection in &self.connections {
                    let present = connection
                        .backend
                        .environments()
                        .iter()
                        .any(|id| id == environment);
                    if present != ids.contains(&connection.id) {
                        return Err(BackendError::new(format!(
                            "Runtime participant environment mismatch: {environment}/{}",
                            connection.id
                        )));
                    }
                }
            }
            for connection in &self.connections {
                for environment in connection.backend.environments() {
                    if !participants
                        .get(&environment)
                        .is_some_and(|ids| ids.contains(&connection.id))
                    {
                        return Err(BackendError::new(format!(
                            "Untracked runtime participant: {environment}/{}",
                            connection.id
                        )));
                    }
                }
            }
            Ok(())
        })();
        if let Err(error) = validation {
            for connection in &self.connections {
                let _ = connection.backend.shutdown();
            }
            return Err(error);
        }
        *self
            .participants
            .lock()
            .map_err(|_| BackendError::new("Runtime participants poisoned"))? = participants;
        Ok(())
    }
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
        let (connections, _) = self.selected_existing(context, BackendOperation::Operate)?;
        let mut fragment = LaunchFragment::default();
        for connection in connections {
            merge_launch_fragment(
                &mut fragment,
                connection.backend.launch_fragment(context, launch)?,
            )?;
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
        let (connections, _) = self.selected_existing(context, backend_operation)?;
        if connections.len() != 1 {
            return Err(BackendError::new(
                "filesystem operation requires exactly one selected Backend",
            ));
        }
        connections[0]
            .backend
            .filesystem_operation(context, operation, cancel)
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
                    let retained: Vec<_> = connections[..=index]
                        .iter()
                        .filter(|connection| connection.backend.state(child).is_some())
                        .map(|connection| connection.id.clone())
                        .collect();
                    if !retained.is_empty() {
                        self.record_participants(child, &retained)?;
                        error.created_environment_id = Some(child.to_string());
                    }
                    return Err(error);
                }
            }
        }
        // 成功记录全部参与者；失败路径保留实际参与者以继续清理。
        self.record_participants(child, &ids)?;
        Ok(())
    }

    fn promote(&self, environment_id: &str) -> Result<(), BackendError> {
        self.promote_with_context(&BackendContext::new(environment_id))
    }

    fn promote_with_context(&self, context: &BackendContext) -> Result<(), BackendError> {
        self.execute_lifecycle_existing(
            context,
            BackendOperation::Promote,
            |connection, context| connection.backend.promote_with_context(context),
        )
    }

    fn suspend(&self, environment_id: &str) -> Result<(), BackendError> {
        self.suspend_with_context(&BackendContext::new(environment_id))
    }

    fn suspend_with_context(&self, context: &BackendContext) -> Result<(), BackendError> {
        self.execute_lifecycle_existing(
            context,
            BackendOperation::Suspend,
            |connection, context| connection.backend.suspend_with_context(context),
        )
    }

    fn resume(&self, environment_id: &str) -> Result<(), BackendError> {
        self.resume_with_context(&BackendContext::new(environment_id))
    }

    fn resume_with_context(&self, context: &BackendContext) -> Result<(), BackendError> {
        self.execute_lifecycle_existing(context, BackendOperation::Resume, |connection, context| {
            connection.backend.resume_with_context(context)
        })
    }

    fn terminate(&self, environment_id: &str) -> Result<(), BackendError> {
        self.terminate_with_context(&BackendContext::new(environment_id))
    }

    fn terminate_with_context(&self, context: &BackendContext) -> Result<(), BackendError> {
        self.execute_lifecycle_existing(
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
        Vec::new()
    }

    fn environments(&self) -> Vec<String> {
        let mut environments = BTreeSet::new();
        for connection in &self.connections {
            environments.extend(connection.backend.environments());
        }
        environments.into_iter().collect()
    }
}
