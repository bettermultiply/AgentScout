//! BaseRuntime：在另一个 Backend 上管理 Agent 进程与文件系统装配。

use std::path::PathBuf;
use std::sync::Arc;

use crate::runtime::backends::filesystem::FilesystemBackend;
use serde_json::Value;

use crate::protocol::contract::{
    AgentLaunch, BackendState, CommandOutput, EnvironmentState, FilesystemResult, Operation,
    RuntimeCapabilities,
};
use super::backend::{Backend, BackendContext, BackendError, BackendExecution, CancelHandle};
use super::process::{FreezeMechanism, LaunchFragment, ProcessBackend, ProcessHandle};
use super::selection::merge_launch_fragment;

pub struct BaseRuntime {
    id: String,
    inner: Arc<dyn Backend>,
    filesystem: Option<Arc<FilesystemBackend>>,
    pub(super) process: ProcessBackend,
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




