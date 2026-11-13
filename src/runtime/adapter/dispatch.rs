//! 请求分发、执行与结果构造。

use super::*;

const RESULT_WAIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
use super::result::{can_accept, filesystem_failure_reason, result as make_result};
use crate::protocol::contract::*;
use crate::runtime::backend::*;
use serde_json::Value;


fn operation_context(environment_id: &str, request: &OperationRequest) -> BackendContext {
    BackendContext {
        operation_id: request.request_id.clone(),
        generation: request.generation,
        task_id: request.task_id.clone(),
        branch_id: request
            .branch_id
            .clone()
            .or_else(|| Some(request.environment_id.clone())),
        environment_id: environment_id.to_string(),
        parent_environment_id: None,
        target_mount_pid: None,
        target_mount_namespace: None,
    }
}

impl Shared {
    pub(super) fn dispatch(self: &Arc<Self>, request: OperationRequest) {
        {
            let mut seen = self.seen.lock().unwrap();
            if !seen.insert(request.request_id.clone()) {
                drop(seen);
                let results = self.results.lock().unwrap();
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
        let state = self.runtime.state(environment_id);
        let context = operation_context(environment_id, request);
        match &request.operation {
            Operation::SaveRuntime { path } | Operation::LoadRuntime { path } => {
                let result = if matches!(request.operation, Operation::SaveRuntime { .. }) {
                    // Even a failed save must not fall through to destructive EOF cleanup.
                    self.preserving.store(true, std::sync::atomic::Ordering::SeqCst);
                    self.runtime.save_runtime(path)
                } else {
                    self.preserving.store(true, std::sync::atomic::Ordering::SeqCst);
                    let result=self.runtime.load_runtime(path);
                    if result.is_err() {let _=self.runtime.shutdown();}
                    result
                };
                match result {
                    Ok(()) => make_result(request, ResultStatus::Completed, self.runtime.state(environment_id), false),
                    Err(error) => self.reasoned_failure(request, state, &error.to_string()),
                }
            }
            Operation::EstablishEnvironment => self.execute_establish(environment_id, request, state, &context),
            Operation::OperateEnvironment { command, agent } => self.execute_operate(environment_id, request, state, &context, command, agent.as_ref(), cancel),
            Operation::BranchEnvironment { child_environment_id } => self.execute_branch(environment_id, request, state, &context, child_environment_id),
            Operation::PromoteEnvironment => self.execute_promote(environment_id, request, state, &context),
            Operation::SuspendEnvironment => self.execute_suspend(environment_id, request, state, &context),
            Operation::ResumeEnvironment => self.execute_resume(environment_id, request, state, &context),
            Operation::TerminateEnvironment => self.execute_terminate(environment_id, request, state, &context),
            Operation::CleanupEnvironment => self.execute_cleanup(environment_id, request, state, &context),
            Operation::ToolContext { tool_name } => self.execute_tool_context(environment_id, request, state, &context, tool_name, cancel),
            Operation::InvokeTool { tool_name, arguments } => self.execute_invoke_tool(environment_id, request, state, &context, tool_name, arguments, cancel),
            Operation::FilesystemFork { .. }
            | Operation::FilesystemMark { .. }
            | Operation::FilesystemDiff { .. }
            | Operation::FilesystemAccept { .. }
            | Operation::FilesystemDiscard { .. } => self.execute_filesystem(environment_id, request, state, &context, cancel),
        }
    }

    fn execute_establish(
        &self,
        environment_id: &str,
        request: &OperationRequest,
        state: Option<EnvironmentState>,
        context: &BackendContext,
    ) -> OperationResult {
        if state.is_some() {
            return self.reasoned_failure(request, state, "environment already exists");
        }
        match self.runtime.establish_with_context(context) {
            Ok(()) => {
                let actual = self.runtime.state(environment_id);
                let mut result = self.success(request, actual, can_accept(actual));
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
            let launch = if self.runtime.manages_agent_process() {
                self.runtime
                    .launch_agent(context, agent)
                    .map_err(|error| error.to_string())
            } else {
                self.launch_agent(agent).map_err(|error| error.to_string())
            };
            return match launch {
                Ok(()) => {
                    let actual = self.runtime.state(environment_id);
                    self.success(request, actual, can_accept(actual))
                }
                Err(error) => self.reasoned_failure(
                    request,
                    state,
                    format!("agent launch failed: {error}"),
                ),
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
        child_environment_id: &str,
    ) -> OperationResult {
        match state {
            Some(EnvironmentState::Available | EnvironmentState::Suspended) => {}
            Some(other) => {
                return self.not_applicable(
                    request,
                    Some(other),
                    "environment cannot be branched",
                );
            }
            None => return self.unknown_environment(request),
        }
        if self.runtime.state(child_environment_id).is_some() {
            return self.reasoned_failure(
                request,
                state,
                "child environment already exists".to_string(),
            );
        }
        match self.runtime.branch_with_context(
            &BackendContext {
                operation_id: request.request_id.clone(),
                generation: 0,
                environment_id: environment_id.to_string(),
                parent_environment_id: Some(environment_id.to_string()),
                ..context.clone()
            },
            environment_id,
            child_environment_id,
        ) {
            Ok(()) => {
                let actual = self.runtime.state(environment_id);
                let mut result = self.success(request, actual, can_accept(actual));
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
        if state == Some(EnvironmentState::Terminated) {
            let mut result = self.success(request, state, false);
            result.follow_up = vec![FollowUp::Cleanup];
            return result;
        }
        if state.is_none() {
            return self.unknown_environment(request);
        }
        if !matches!(
            state,
            Some(EnvironmentState::Available | EnvironmentState::Suspended)
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
        if state != Some(EnvironmentState::Terminated) {
            return self.not_applicable(
                request,
                state,
                "cleanup requires a terminated environment",
            );
        }
        match self.runtime.cleanup_with_context(context) {
            Ok(()) => self.success(request, self.runtime.state(environment_id), false),
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

    fn execute_invoke_tool(
        &self,
        environment_id: &str,
        request: &OperationRequest,
        state: Option<EnvironmentState>,
        context: &BackendContext,
        tool_name: &str,
        arguments: &Value,
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
            .invoke_tool_with_context(context, tool_name, arguments, cancel)
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
                let Some(reason) = filesystem_failure_reason(&request.operation, &filesystem) else {
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
        // Backend 尚未就绪，控制面据此得到 PartiallyCompleted 与原因。
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
        let state = self.runtime.state(environment_id);
        let status = if error.is_partial() {
            ResultStatus::PartiallyCompleted
        } else {
            ResultStatus::Failed
        };
        let mut result = make_result(request, status, state, can_accept(state));
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
