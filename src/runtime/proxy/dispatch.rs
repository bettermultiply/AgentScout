//! 请求分发、执行与结果构造。

use super::*;

const RESULT_WAIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
use super::result::{can_accept, filesystem_failure_reason, result as make_result};
use crate::protocol::contract::*;
use crate::runtime::backend::*;

fn operation_context(environment_id: &str, request: &OperationRequest) -> BackendContext {
    BackendContext {
        tool_call_id: None,
        operation_id: request.request_id.clone(),
        generation: request.generation,
        task_id: request.task_id.clone(),
        environment_id: environment_id.to_string(),
        parent_environment_id: None,
        target_mount_pid: None,
        target_mount_namespace: None,
    }
}

impl Shared {
    pub(super) fn dispatch(self: &Arc<Self>, request: OperationRequest) {
        use sha2::{Digest, Sha256};
        let fingerprint: [u8; 32] =
            Sha256::digest(serde_json::to_vec(&request).expect("operation requests serialize"))
                .into();
        {
            let mut seen = self.seen.lock().unwrap();
            if let Some((original, completed)) = seen.get(&request.request_id) {
                if original != &fingerprint {
                    let _ = self.out.send(AdapterMessage::Rejected {
                        request_id: request.request_id.clone(),
                        reason: "request id reused with different content".into(),
                    });
                    return;
                }
                let completed = *completed;
                let _ = self.out.send(AdapterMessage::Received {
                    request_id: request.request_id.clone(),
                });
                drop(seen);
                let results = self.results.lock().unwrap();
                if completed && results.get(&request.request_id).is_none() {
                    let mut result = make_result(&request, ResultStatus::Failed, None, false);
                    result.reason =
                        Some("original result expired; request will not execute again".into());
                    let _ = self.out.send(AdapterMessage::Result(result));
                    return;
                }
                let (_results, timeout) = self
                    .result_signal
                    .wait_timeout_while(results, RESULT_WAIT_TIMEOUT, |cache| {
                        cache.get(&request.request_id).is_none()
                    })
                    .unwrap();
                if timeout.timed_out() {
                    eprintln!(
                        "adapter: retry for {} timed out waiting for the original result",
                        request.request_id
                    );
                    return;
                }
                if let Some(result) = _results.get(&request.request_id) {
                    let _ = self.out.send(AdapterMessage::Result(result));
                }
                return;
            }
            if let Some(reason) = seen.admission_error(&request.request_id) {
                let _ = self.out.send(AdapterMessage::Rejected {
                    request_id: request.request_id.clone(),
                    reason: reason.into(),
                });
                return;
            }
            seen.insert(request.request_id.clone(), (fingerprint, false));
            let _ = self.out.send(AdapterMessage::Received {
                request_id: request.request_id.clone(),
            });
        }
        let queue = self.queue_for(&request.environment_id);
        let preempt = matches!(
            &request.operation,
            Operation::SuspendEnvironment | Operation::TerminateEnvironment
        );
        queue.enqueue(request, preempt);
    }

    pub(super) fn queue_for(self: &Arc<Self>, environment_id: &str) -> Arc<Queue> {
        let mut queues = self.queues.lock().unwrap();
        queues
            .entry(environment_id.to_string())
            .or_insert_with(|| Queue::start(self.clone(), environment_id.to_string()))
            .clone()
    }

    pub(super) fn set_known(&self, environment_id: &str, state: EnvironmentState) {
        self.known
            .lock()
            .unwrap()
            .insert(environment_id.to_string(), state);
    }

    pub(super) fn execute(
        &self,
        environment_id: &str,
        request: &OperationRequest,
        cancel: &CancelHandle,
    ) -> OperationResult {
        // Recovery owns its admission in BaseRuntime, including exclusive restore.
        let _operation = if matches!(
            request.operation,
            Operation::SaveRuntime { .. } | Operation::LoadRuntime { .. }
        ) {
            None
        } else {
            match self.runtime.enter_operation() {
                Ok(operation) => Some(operation),
                Err(reason) => return self.reasoned_failure(request, None, reason),
            }
        };
        let mut environments = vec![environment_id.to_owned()];
        match &request.operation {
            Operation::AcceptEnvironment {
                source_environment_id,
                ..
            } => environments.push(source_environment_id.clone()),
            Operation::FilesystemAccept {
                source_snapshot_id,
                target_snapshot_id,
                ..
            } => {
                environments.extend([source_snapshot_id.clone(), target_snapshot_id.clone()]);
            }
            Operation::FilesystemFork {
                parent_snapshot_id,
                child_snapshot_id,
                ..
            } => {
                environments.extend([parent_snapshot_id.clone(), child_snapshot_id.clone()]);
            }
            Operation::FilesystemMark { snapshot_id, .. }
            | Operation::FilesystemDiscard { snapshot_id, .. } => {
                environments.push(snapshot_id.clone())
            }
            _ => {}
        }
        let _admission = match self.runtime.admit_control(environments) {
            Ok(admission) => admission,
            Err(reason) => return self.reasoned_failure(request, None, reason),
        };
        let state = self.runtime.state(environment_id);
        let context = operation_context(environment_id, request);
        match &request.operation {
            Operation::MaterializeAuthority => match self.runtime.materialize(&context) {
                Ok(filesystem) => {
                    let mut result = self.success(request, state, can_accept(state));
                    result.filesystem = Some(filesystem);
                    result
                }
                Err(error) => self.backend_error(request, environment_id, error),
            },
            Operation::SaveRuntime { path } => {
                self.recovery_result(request, state, self.runtime.save_runtime(path))
            }
            Operation::LoadRuntime { path } => {
                self.recovery_result(request, state, self.runtime.load_runtime(path))
            }
            Operation::EstablishEnvironment => {
                self.execute_establish(environment_id, request, state, &context)
            }
            Operation::OperateEnvironment { command, agent } => self.execute_operate(
                environment_id,
                request,
                state,
                &context,
                command,
                agent.as_ref(),
                cancel,
            ),
            Operation::BranchEnvironment { agent } => {
                self.execute_branch(environment_id, request, state, &context, agent.as_ref())
            }
            Operation::PromoteEnvironment => {
                self.execute_promote(environment_id, request, state, &context)
            }
            Operation::AcceptEnvironment {
                source_environment_id,
                full,
                part,
                target_suspended,
            } => {
                let Some(commands) = &self.commands else {
                    return self.reasoned_failure(
                        request,
                        state,
                        "Runtime execution admission is unavailable",
                    );
                };
                let admission =
                    match commands.pause_admission(&[environment_id, source_environment_id]) {
                        Ok(admission) => admission,
                        Err(reason) => return self.reasoned_failure(request, state, reason),
                    };
                match self.runtime.accept_environment(
                    &context,
                    source_environment_id,
                    *full,
                    part.as_ref(),
                    *target_suspended,
                    |id| admission.wait(id),
                ) {
                    Ok(filesystem) => {
                        let actual = self.runtime.state(environment_id);
                        let mut result = self.success(request, actual, can_accept(actual));
                        result.filesystem = Some(filesystem);
                        result
                    }
                    Err(error) => self.backend_error(request, environment_id, error),
                }
            }
            Operation::SuspendEnvironment => {
                self.execute_suspend(environment_id, request, state, &context)
            }
            Operation::ResumeEnvironment => {
                self.execute_resume(environment_id, request, state, &context)
            }
            Operation::TerminateEnvironment => {
                self.execute_terminate(environment_id, request, state, &context)
            }
            Operation::CleanupEnvironment => {
                self.execute_cleanup(environment_id, request, state, &context)
            }
            Operation::ToolContext { tool_name } => self.execute_tool_context(
                environment_id,
                request,
                state,
                &context,
                tool_name,
                cancel,
            ),
            Operation::FilesystemFork { .. }
            | Operation::FilesystemMark { .. }
            | Operation::FilesystemDiff { .. }
            | Operation::FilesystemAccept { .. }
            | Operation::FilesystemDiscard { .. } => {
                self.execute_filesystem(environment_id, request, state, &context, cancel)
            }
        }
    }

    fn recovery_result(
        &self,
        request: &OperationRequest,
        state: Option<EnvironmentState>,
        result: Result<(), BackendError>,
    ) -> OperationResult {
        match result {
            Ok(()) => make_result(
                request,
                ResultStatus::Completed,
                self.runtime.state(&request.environment_id),
                false,
            ),
            Err(error) => self.reasoned_failure(request, state, &error.to_string()),
        }
    }

    fn execute_establish(
        &self,
        environment_id: &str,
        request: &OperationRequest,
        state: Option<EnvironmentState>,
        context: &BackendContext,
    ) -> OperationResult {
        if !request.environment_id.is_empty() {
            return self.reasoned_failure(
                request,
                state,
                "establish must not supply an Environment identity",
            );
        }
        match self.runtime.create_environment(context) {
            Ok(new_environment_id) => {
                let actual = self.runtime.state(&new_environment_id);
                let mut result = self.success_for_environment(
                    request,
                    &new_environment_id,
                    actual,
                    can_accept(actual),
                );
                result.resources = vec![
                    ResourceNote {
                        content: Content::Process,
                        note: "environment process started".to_string(),
                    },
                    ResourceNote {
                        content: Content::Workspace,
                        note: "environment workspace created".to_string(),
                    },
                ];
                result
            }
            Err(error) => self.backend_error(request, environment_id, error),
        }
    }

    fn execute_operate(
        &self,
        environment_id: &str,
        request: &OperationRequest,
        state: Option<EnvironmentState>,
        context: &BackendContext,
        command: &[String],
        agent: Option<&AgentLaunch>,
        cancel: &CancelHandle,
    ) -> OperationResult {
        match state {
            Some(EnvironmentState::Available) => {}
            Some(other) => {
                return self.not_applicable(
                    request,
                    Some(other),
                    "environment cannot accept operations",
                );
            }
            None => return self.unknown_environment(request),
        }
        if let Some(agent) = agent {
            let launch = self.runtime.launch_agent(context, agent);
            return match launch {
                Ok(()) => {
                    let actual = self.runtime.state(environment_id);
                    self.success(request, actual, can_accept(actual))
                }
                Err(mut error) => {
                    error.message = format!("agent launch failed: {}", error.message);
                    self.backend_error(request, environment_id, error)
                }
            };
        }
        match self.runtime.operate_with_context(context, command, cancel) {
            Ok(execution) => self.execution_result(request, environment_id, execution, false),
            Err(error) => self.backend_error(request, environment_id, error),
        }
    }

    fn execute_branch(
        &self,
        environment_id: &str,
        request: &OperationRequest,
        state: Option<EnvironmentState>,
        context: &BackendContext,
        agent: Option<&AgentLaunch>,
    ) -> OperationResult {
        match state {
            Some(EnvironmentState::Available | EnvironmentState::Suspended) => {}
            Some(other) => {
                return self.not_applicable(request, Some(other), "environment cannot be branched");
            }
            None => return self.unknown_environment(request),
        }
        match self.runtime.branch_environment(context, agent) {
            Ok(child_environment_id) => {
                let actual = self.runtime.state(&child_environment_id);
                let mut result = self.success_for_environment(
                    request,
                    &child_environment_id,
                    actual,
                    can_accept(actual),
                );
                result.resources = vec![ResourceNote {
                    content: Content::Process,
                    note: format!("environment {child_environment_id} created"),
                }];
                result
            }
            Err(error) => self.backend_error(request, environment_id, error),
        }
    }

    fn execute_promote(
        &self,
        environment_id: &str,
        request: &OperationRequest,
        state: Option<EnvironmentState>,
        context: &BackendContext,
    ) -> OperationResult {
        if state != Some(EnvironmentState::Available) {
            return self.not_applicable(request, state, "environment cannot be promoted");
        }
        match self.runtime.promote_with_context(context) {
            Ok(()) => {
                let actual = self.runtime.state(environment_id);
                self.success(request, actual, can_accept(actual))
            }
            Err(error) => self.backend_error(request, environment_id, error),
        }
    }

    fn execute_suspend(
        &self,
        environment_id: &str,
        request: &OperationRequest,
        state: Option<EnvironmentState>,
        context: &BackendContext,
    ) -> OperationResult {
        if state != Some(EnvironmentState::Available) {
            return self.not_applicable(request, state, "environment cannot be suspended");
        }
        // Gate execution requests while their foreground writers finish and
        // while the backend freezes managed processes, including background writers.
        let _admission = self.commands.as_ref().map(|commands| commands.pause_admission(&[environment_id])).transpose();
        let _admission = match _admission {
            Ok(admission) => admission,
            Err(reason) => return self.reasoned_failure(request, state, reason),
        };
        if let Some(admission) = &_admission {
            admission.wait(environment_id);
        }
        match self.runtime.suspend_with_context(context) {
            Ok(()) => {
                let mut result = self.success(request, self.runtime.state(environment_id), true);
                result.follow_up = vec![FollowUp::Resume];
                result
            }
            Err(error) => self.backend_error(request, environment_id, error),
        }
    }

    fn execute_resume(
        &self,
        environment_id: &str,
        request: &OperationRequest,
        state: Option<EnvironmentState>,
        context: &BackendContext,
    ) -> OperationResult {
        if state != Some(EnvironmentState::Suspended) {
            return self.not_applicable(request, state, "environment cannot be resumed");
        }
        match self.runtime.resume_with_context(context) {
            Ok(()) => self.success(request, self.runtime.state(environment_id), true),
            Err(error) => self.backend_error(request, environment_id, error),
        }
    }

    fn execute_terminate(
        &self,
        environment_id: &str,
        request: &OperationRequest,
        state: Option<EnvironmentState>,
        context: &BackendContext,
    ) -> OperationResult {
        if matches!(
            state,
            Some(EnvironmentState::Terminated | EnvironmentState::Cleaned)
        ) {
            let mut result = self.success(request, state, false);
            result.follow_up = vec![FollowUp::Cleanup];
            return result;
        }
        if state.is_none() {
            return self.unknown_environment(request);
        }
        if !matches!(
            state,
            Some(
                EnvironmentState::Available
                    | EnvironmentState::Suspended
                    | EnvironmentState::PartiallyAvailable
            )
        ) {
            return self.not_applicable(request, state, "environment cannot be terminated");
        }
        match self.runtime.terminate_with_context(context) {
            Ok(()) => {
                let mut result = self.success(request, self.runtime.state(environment_id), false);
                result.follow_up = vec![FollowUp::Cleanup];
                result
            }
            Err(error) => self.backend_error(request, environment_id, error),
        }
    }

    fn execute_cleanup(
        &self,
        environment_id: &str,
        request: &OperationRequest,
        state: Option<EnvironmentState>,
        context: &BackendContext,
    ) -> OperationResult {
        if state.is_none() {
            return self.not_applicable(request, None, "unknown environment");
        }
        if state == Some(EnvironmentState::Cleaned) {
            return self.success(request, state, false);
        }
        if state != Some(EnvironmentState::Terminated) {
            return self.not_applicable(
                request,
                state,
                "cleanup requires a terminated environment",
            );
        }
        match self.runtime.cleanup_with_context(context) {
            // Successful cleanup is the backend's resource-release receipt.
            // Backends may remove the record, so a subsequent state lookup
            // cannot be used to reconstruct this completed transition.
            Ok(()) => self.success(request, Some(EnvironmentState::Cleaned), false),
            Err(error) => self.backend_error(request, environment_id, error),
        }
    }

    fn execute_tool_context(
        &self,
        environment_id: &str,
        request: &OperationRequest,
        state: Option<EnvironmentState>,
        context: &BackendContext,
        tool_name: &str,
        cancel: &CancelHandle,
    ) -> OperationResult {
        if !self.tool_declared(tool_name) {
            return self.unsupported(
                request,
                state,
                format!("runtime does not declare tool {tool_name}"),
            );
        }
        match self
            .runtime
            .tool_context_with_context(context, tool_name, cancel)
        {
            Ok(execution) => self.execution_result(request, environment_id, execution, true),
            Err(error) => self.backend_error(request, environment_id, error),
        }
    }

    fn execute_filesystem(
        &self,
        environment_id: &str,
        request: &OperationRequest,
        state: Option<EnvironmentState>,
        context: &BackendContext,
        cancel: &CancelHandle,
    ) -> OperationResult {
        match self
            .runtime
            .filesystem_operation(context, &request.operation, cancel)
        {
            Ok(filesystem) => {
                let Some(reason) = filesystem_failure_reason(&request.operation, &filesystem)
                else {
                    let mut result = self.success(request, state, can_accept(state));
                    result.filesystem = Some(filesystem);
                    return result;
                };
                let mut result = self.reasoned_failure(request, state, reason);
                result.status = ResultStatus::PartiallyCompleted;
                result.filesystem = Some(filesystem);
                result
            }
            Err(error) => self.backend_error(request, environment_id, error),
        }
    }

    pub(super) fn tool_declared(&self, tool_name: &str) -> bool {
        self.runtime
            .capabilities()
            .tools
            .iter()
            .any(|tool| tool.name == tool_name)
    }

    pub(super) fn success(
        &self,
        request: &OperationRequest,
        state: Option<EnvironmentState>,
        can_accept_operations: bool,
    ) -> OperationResult {
        // 环境实际停留在 PartiallyAvailable 时不能报告 Completed：部分
        // Backend 尚未就绪，control plane 据此得到 PartiallyCompleted 与原因。
        let status = if state == Some(EnvironmentState::PartiallyAvailable) {
            ResultStatus::PartiallyCompleted
        } else {
            ResultStatus::Completed
        };
        // 收口：只要环境停留在 PartiallyAvailable，就不能接受后续普通
        // 操作，无论调用方传入 true 还是 false。
        let can_accept_operations =
            can_accept_operations && state != Some(EnvironmentState::PartiallyAvailable);
        let mut result = make_result(request, status, state, can_accept_operations);
        if status == ResultStatus::PartiallyCompleted {
            result.reason = Some(
                "environment is partially available; not all Backends are consistently available"
                    .to_string(),
            );
        }
        result.backend_states = self.runtime.backend_states(&request.environment_id);
        result.execution_access = self
            .commands
            .as_ref()
            .map(|commands| commands.access(&request.environment_id));
        result
    }

    pub(super) fn success_for_environment(
        &self,
        request: &OperationRequest,
        environment_id: &str,
        state: Option<EnvironmentState>,
        can_accept_operations: bool,
    ) -> OperationResult {
        let mut result = self.success(request, state, can_accept_operations);
        result.environment_id = environment_id.to_string();
        result.backend_states = self.runtime.backend_states(environment_id);
        result.execution_access = self
            .commands
            .as_ref()
            .map(|commands| commands.access(environment_id));
        if state.is_none() {
            result.status = ResultStatus::Failed;
            result.can_accept_operations = false;
            result.reason = Some(
                "Runtime creation returned an identity without a registered Environment".into(),
            );
        }
        result
    }

    pub(super) fn execution_result(
        &self,
        request: &OperationRequest,
        environment_id: &str,
        execution: BackendExecution,
        require_single_output: bool,
    ) -> OperationResult {
        let state = self.runtime.state(environment_id);
        let mut result = self.success(request, state, can_accept(state));
        result.output = execution.single_output();
        result.outputs = execution.outputs;
        if require_single_output && result.output.is_none() {
            result.status = ResultStatus::PartiallyCompleted;
            result.reason = Some(
                "multiple or missing Backend outputs; configure an explicit output_backend"
                    .to_string(),
            );
        }
        result
    }

    pub(super) fn backend_error(
        &self,
        request: &OperationRequest,
        environment_id: &str,
        error: BackendError,
    ) -> OperationResult {
        let environment_id = error
            .created_environment_id
            .as_deref()
            .unwrap_or(environment_id);
        let state = self.runtime.state(environment_id);
        let status = if error.is_partial() {
            ResultStatus::PartiallyCompleted
        } else {
            ResultStatus::Failed
        };
        let mut result = make_result(request, status, state, can_accept(state));
        result.environment_id = environment_id.to_string();
        result.reason = Some(error.to_string());
        result.backend_states = self.runtime.backend_states(environment_id);
        result.outputs = error.outputs;
        result
            .resources
            .extend(error.completed.into_iter().map(|backend_id| ResourceNote {
                content: Content::Resource,
                note: format!("Backend {backend_id} completed"),
            }));
        result
            .remaining
            .extend(error.remaining.into_iter().map(|backend_id| ResourceNote {
                content: Content::Resource,
                note: format!("Backend {backend_id} was not completed"),
            }));
        result.remaining.extend(
            error
                .cleanup_failures
                .into_iter()
                .map(|failure| ResourceNote {
                    content: Content::Resource,
                    note: format!("cleanup incomplete: {failure}"),
                }),
        );
        result
    }

    pub(super) fn reasoned_failure(
        &self,
        request: &OperationRequest,
        state: Option<EnvironmentState>,
        reason: impl Into<String>,
    ) -> OperationResult {
        let mut result = make_result(request, ResultStatus::Failed, state, can_accept(state));
        result.reason = Some(reason.into());
        result.backend_states = self.runtime.backend_states(&request.environment_id);
        result
    }

    pub(super) fn not_applicable(
        &self,
        request: &OperationRequest,
        state: Option<EnvironmentState>,
        reason: &str,
    ) -> OperationResult {
        let mut result = make_result(request, ResultStatus::NotApplicable, state, false);
        result.reason = Some(reason.to_string());
        result.backend_states = self.runtime.backend_states(&request.environment_id);
        result
    }

    pub(super) fn unsupported(
        &self,
        request: &OperationRequest,
        state: Option<EnvironmentState>,
        reason: String,
    ) -> OperationResult {
        let mut result = make_result(request, ResultStatus::Unsupported, state, false);
        result.reason = Some(reason);
        result.backend_states = self.runtime.backend_states(&request.environment_id);
        result
    }

    pub(super) fn unknown_environment(&self, request: &OperationRequest) -> OperationResult {
        let mut result = make_result(request, ResultStatus::Failed, None, false);
        result.reason = Some("unknown environment".to_string());
        result.backend_states = self.runtime.backend_states(&request.environment_id);
        result
    }
}
