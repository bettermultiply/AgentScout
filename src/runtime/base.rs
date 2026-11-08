//! BaseRuntime：在另一个 Backend 上管理 Agent 进程与数据面装配。

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;

use crate::protocol::contract::{
    AgentLaunch, BackendState, CommandOutput, EnvironmentState, FilesystemResult, Operation,
    RuntimeCapabilities,
};
use super::backend::{Backend, BackendContext, BackendError, BackendExecution, CancelHandle};
use super::launch_plan::LaunchFragment;
use super::process::{FreezeMechanism, ProcessBackend, ProcessHandle};
use super::selection::merge_launch_fragment;

pub struct BaseRuntime {
    id: String,
    inner: Arc<dyn Backend>,
    /// Backend that contributes launch mounts and owns FilesystemOperation.
    /// It is deliberately only a trait object so any composable Backend can
    /// occupy this slot; BaseRuntime does not know filesystem types.
    data_plane: Option<Arc<dyn Backend>>,
    pub(super) process: ProcessBackend,
}

impl BaseRuntime {
    pub fn new(id: impl Into<String>, inner: Arc<dyn Backend>) -> Self {
        Self {
            id: id.into(),
            inner,
            data_plane: None,
            process: ProcessBackend::new("process", "bwrap"),
        }
    }

    /// Wrap Agent startup with a data-plane Backend. The Backend remains
    /// responsible only for its own data plane; this Runtime wrapper delegates
    /// process lifetime, freeze/thaw ordering and launch-fragment merging.
    pub fn with_data_plane(mut self, data_plane: Arc<dyn Backend>) -> Self {
        self.data_plane = Some(data_plane);
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
        if let Some(data_plane) = &self.data_plane {
            merge_launch_fragment(&mut fragment, data_plane.launch_fragment(context, launch)?)?;
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

    fn ensure_data_plane_available(&self, context: &BackendContext) -> Result<(), BackendError> {
        if let Some(data_plane) = &self.data_plane {
            if data_plane.state(&context.environment_id) != Some(EnvironmentState::Available) {
                return Err(BackendError::new(
                    "data plane environment is not available for operation",
                ));
            }
        }
        Ok(())
    }

    fn ensure_data_plane_state(
        &self,
        context: &BackendContext,
        expected: EnvironmentState,
    ) -> Result<(), BackendError> {
        if let Some(data_plane) = &self.data_plane {
            if data_plane.state(&context.environment_id) != Some(expected) {
                return Err(BackendError::new(format!(
                    "data plane environment is not in required state: expected {expected:?}"
                )));
            }
        }
        Ok(())
    }
}

fn single_output(execution: BackendExecution) -> Result<CommandOutput, BackendError> {
    execution.single_output().ok_or_else(|| {
        BackendError::new("multiple Backend outputs require an explicit output_backend selection")
            .with_outputs(execution.outputs)
    })
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
        if let Some(data_plane) = &self.data_plane {
            ids.push(data_plane.backend_id().to_string());
        }
        ids.push("process".to_string());
        ids
    }

    fn capabilities(&self) -> RuntimeCapabilities {
        let mut capabilities = self.inner.capabilities();
        if let Some(data_plane) = &self.data_plane {
            let data_plane_capabilities = data_plane.capabilities();
            capabilities.branch_environment &= data_plane_capabilities.branch_environment;
            capabilities.shared_environment |= data_plane_capabilities.shared_environment;
            capabilities.filesystem_fast_delete = data_plane_capabilities.filesystem_fast_delete;
            capabilities.filesystem_fork_cost = data_plane_capabilities.filesystem_fork_cost;
            capabilities.filesystem_btrfs_incremental_diff =
                data_plane_capabilities.filesystem_btrfs_incremental_diff;
            capabilities.filesystem_passthrough = data_plane_capabilities.filesystem_passthrough;
            capabilities.state_directory_overlaps_mount =
                data_plane_capabilities.state_directory_overlaps_mount;
        }
        capabilities.freeze_mechanism =
            Some(self.process.freeze_mechanism().as_str().to_string());
        capabilities
    }

    fn establish(&self, environment_id: &str) -> Result<(), BackendError> {
        self.establish_with_context(&BackendContext::new(environment_id))
    }

    fn establish_with_context(&self, context: &BackendContext) -> Result<(), BackendError> {
        if let Some(data_plane) = &self.data_plane {
            data_plane.establish_with_context(context)?;
        }
        if let Err(error) = self.inner.establish_with_context(context) {
            if let Some(data_plane) = &self.data_plane {
                let mut error = error;
                if let Err(terminate) = data_plane.terminate_with_context(context) {
                    error.add_cleanup_failure("data_plane", format!("terminate: {terminate}"));
                }
                if let Err(cleanup) = data_plane.cleanup_with_context(context) {
                    error.add_cleanup_failure("data_plane", format!("cleanup: {cleanup}"));
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
        single_output(execution)
    }

    fn operate_with_context(
        &self,
        context: &BackendContext,
        command: &[String],
        cancel: &CancelHandle,
    ) -> Result<BackendExecution, BackendError> {
        // The data plane is not a second business command executor. Its state
        // check prevents direct Backend callers from bypassing suspend, while
        // the command itself remains owned by the inner Runtime.
        self.ensure_data_plane_available(context)?;
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
        single_output(execution)
    }

    fn tool_context_with_context(
        &self,
        context: &BackendContext,
        tool_name: &str,
        cancel: &CancelHandle,
    ) -> Result<BackendExecution, BackendError> {
        self.ensure_data_plane_available(context)?;
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
        single_output(execution)
    }

    fn invoke_tool_with_context(
        &self,
        context: &BackendContext,
        tool_name: &str,
        arguments: &Value,
        cancel: &CancelHandle,
    ) -> Result<BackendExecution, BackendError> {
        self.ensure_data_plane_available(context)?;
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
        if let Some(data_plane) = &self.data_plane {
            data_plane.branch_with_context(context, parent, child)?;
        }
        if let Err(error) = self.inner.branch_with_context(context, parent, child) {
            if let Some(data_plane) = &self.data_plane {
                let child_context = BackendContext {
                    environment_id: child.to_string(),
                    ..context.clone()
                };
                let mut error = error;
                if let Err(terminate) = data_plane.terminate_with_context(&child_context) {
                    error.add_cleanup_failure("data_plane", format!("terminate: {terminate}"));
                }
                if let Err(cleanup) = data_plane.cleanup_with_context(&child_context) {
                    error.add_cleanup_failure("data_plane", format!("cleanup: {cleanup}"));
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
        self.ensure_data_plane_state(context, EnvironmentState::Available)?;
        self.inner.promote_with_context(context)?;
        if let Some(data_plane) = &self.data_plane {
            data_plane.promote_with_context(context)?;
        }
        Ok(())
    }

    fn suspend(&self, environment_id: &str) -> Result<(), BackendError> {
        self.suspend_with_context(&BackendContext::new(environment_id))
    }

    fn suspend_with_context(&self, context: &BackendContext) -> Result<(), BackendError> {
        self.ensure_data_plane_state(context, EnvironmentState::Available)?;
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
        if let Err(mut error) = inner {
            if running
                && let Err(thaw) = self.process.thaw(&context.environment_id)
            {
                error.add_cleanup_failure("process", format!("thaw after suspend failure: {thaw}"));
            }
            return Err(error);
        }
        if let Some(data_plane) = &self.data_plane
            && let Err(mut error) = data_plane.suspend_with_context(context)
        {
            if running
                && let Err(thaw) = self.process.thaw(&context.environment_id)
            {
                error.add_cleanup_failure("process", format!("thaw after suspend failure: {thaw}"));
            }
            return Err(error);
        }
        Ok(())
    }

    fn resume(&self, environment_id: &str) -> Result<(), BackendError> {
        self.resume_with_context(&BackendContext::new(environment_id))
    }

    fn resume_with_context(&self, context: &BackendContext) -> Result<(), BackendError> {
        self.ensure_data_plane_state(context, EnvironmentState::Suspended)?;
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
        if let Err(mut error) = inner {
            if thaw_needed
                && let Err(freeze) = self.process.freeze(&context.environment_id)
            {
                error.add_cleanup_failure("process", format!("freeze after resume failure: {freeze}"));
            }
            return Err(error);
        }
        if let Some(data_plane) = &self.data_plane
            && let Err(mut error) = data_plane.resume_with_context(context)
        {
            if thaw_needed
                && let Err(freeze) = self.process.freeze(&context.environment_id)
            {
                error.add_cleanup_failure("process", format!("freeze after resume failure: {freeze}"));
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
            .data_plane
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
            failures.push(format!("data_plane terminate failed: {error}"));
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
            .data_plane
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
            failures.push(format!("data_plane cleanup failed: {error}"));
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
            .data_plane
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
        if let Some(data_plane) = &self.data_plane {
            states.push(BackendState {
                backend_id: data_plane.backend_id().to_string(),
                state: data_plane.state(environment_id),
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
        let result = if let Some(data_plane) = &self.data_plane {
            data_plane.filesystem_operation(&context, operation, cancel)
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
        if let Some(data_plane) = &self.data_plane {
            merge_launch_fragment(&mut fragment, data_plane.launch_fragment(context, launch)?)?;
        }
        Ok(fragment)
    }

    fn environments(&self) -> Vec<String> {
        let mut environments = self.inner.environments();
        if let Some(data_plane) = &self.data_plane {
            environments.extend(data_plane.environments());
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




