//! Runtime 后端接口和一次后端调用的内部结果。
//!
//! `Backend` 是 Runtime Adapter 使用的最小执行接口。它不假设后端是
//! NLE、虚拟机、容器还是本地进程；一个 Runtime 可以把多个实现按明确的
//! 顺序组合起来。旧的单 Backend 实现仍然可以只实现原来的方法，新的
//! Runtime 编排通过带上下文的方法调用它们。

use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Value;

use crate::protocol::contract::{
    BackendOutput, BackendState, CommandOutput, EnvironmentState, FilesystemResult, Operation,
    RuntimeCapabilities,
};
use crate::runtime::launch_plan::LaunchFragment;

/// 一次 Runtime 请求传给 Backend 的逻辑上下文。
///
/// Backend 只接收 Environment 和操作上下文，不接收或推导 control plane 的 Branch 身份。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BackendContext {
    /// Stable identity for an operation. Empty for legacy environment calls.
    pub operation_id: String,
    /// Original product tool call when one tool performs multiple operations.
    pub tool_call_id: Option<String>,
    /// Fences delayed filesystem results. Zero is the legacy value.
    pub generation: u64,
    pub task_id: Option<String>,
    pub environment_id: String,
    pub parent_environment_id: Option<String>,
    /// Filesystem-only optional access point to a target environment's merged
    /// mount view, for example `/proc/<pid>/root`.  Other Backends ignore it.
    pub target_mount_pid: Option<u32>,
    /// Mount namespace inode recorded by Processes for the target pid.
    /// FilesystemBackend validates it before replaying into `/proc/<pid>/root`.
    pub target_mount_namespace: Option<u64>,
}

impl BackendContext {
    pub fn new(environment_id: impl Into<String>) -> Self {
        Self {
            environment_id: environment_id.into(),
            operation_id: String::new(),
            generation: 0,
            ..Self::default()
        }
    }
}

type OutputObserver = dyn Fn(super::execution_wire::Message) -> std::io::Result<()> + Send + Sync;

/// Adapter 用它通知后端当前操作应当停止。
#[derive(Default)]
pub struct CancelHandle {
    cancelled: AtomicBool,
    output: std::sync::Mutex<Option<Box<OutputObserver>>>,
}

impl CancelHandle {
    pub(crate) fn observe_output(
        &self,
        observer: impl Fn(super::execution_wire::Message) -> std::io::Result<()> + Send + Sync + 'static,
    ) {
        *self.output.lock().unwrap() = Some(Box::new(observer));
    }
    pub fn output(&self, value: Value) {
        let _ = self.output_message(value.into());
    }
    pub(crate) fn output_message(
        &self,
        message: super::execution_wire::Message,
    ) -> Result<(), BackendError> {
        if let Some(observer) = self.output.lock().unwrap().as_ref() {
            observer(message).map_err(|error| BackendError::new(error.to_string()))?;
        }
        Ok(())
    }
    /// 每个操作开始前重置。
    pub fn reset(&self) {
        self.cancelled.store(false, Ordering::SeqCst);
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

#[derive(Debug, Clone)]
pub struct BackendError {
    /// Identity allocated by Runtime when creation failed after resources existed.
    pub created_environment_id: Option<String>,
    pub message: String,
    pub cancelled: bool,
    pub error_category: String,
    /// 已经完成的有序 Backend 标识。
    pub completed: Vec<String>,
    /// 因本次错误没有执行的 Backend 标识。
    pub remaining: Vec<String>,
    /// 回滚或清理时仍未完成的 Backend 标识。
    pub cleanup_failures: Vec<String>,
    /// 失败前已经产生、但不能丢弃的 Backend 输出。
    pub outputs: Vec<BackendOutput>,
}

impl BackendError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            created_environment_id: None,
            message: message.into(),
            cancelled: false,
            error_category: "backend_error".into(),
            completed: Vec::new(),
            remaining: Vec::new(),
            cleanup_failures: Vec::new(),
            outputs: Vec::new(),
        }
    }

    pub fn operation_error(message: impl Into<String>, category: impl Into<String>) -> Self {
        let category = category.into();
        Self {
            cancelled: category == "cancelled",
            error_category: category,
            ..Self::new(message)
        }
    }

    pub fn interrupted() -> Self {
        Self {
            message: "operation interrupted by suspend/terminate".to_string(),
            cancelled: true,
            ..Self::new("")
        }
    }

    pub fn with_completed(mut self, completed: Vec<String>) -> Self {
        self.completed = completed;
        self
    }

    pub fn with_remaining(mut self, remaining: Vec<String>) -> Self {
        self.remaining = remaining;
        self
    }

    pub fn with_outputs(mut self, outputs: Vec<BackendOutput>) -> Self {
        self.outputs = outputs;
        self
    }

    pub fn add_cleanup_failure(&mut self, backend_id: impl Into<String>, message: impl AsRef<str>) {
        self.cleanup_failures
            .push(format!("{}: {}", backend_id.into(), message.as_ref()));
    }

    pub fn is_partial(&self) -> bool {
        !self.completed.is_empty() || self.cancelled || !self.cleanup_failures.is_empty()
    }
}

impl std::fmt::Display for BackendError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)?;
        if !self.completed.is_empty() {
            write!(formatter, "; completed={:?}", self.completed)?;
        }
        if !self.remaining.is_empty() {
            write!(formatter, "; remaining={:?}", self.remaining)?;
        }
        if !self.cleanup_failures.is_empty() {
            write!(formatter, "; cleanup_failures={:?}", self.cleanup_failures)?;
        }
        Ok(())
    }
}

impl std::error::Error for BackendError {}

/// 一个 Backend 调用产生的所有可见输出。
#[derive(Debug, Clone, Default)]
pub struct BackendExecution {
    pub outputs: Vec<BackendOutput>,
    /// 由 Backend Policy 明确选择的旧单值输出来源。
    pub primary_backend: Option<String>,
}

impl BackendExecution {
    pub fn single(backend_id: impl Into<String>, output: CommandOutput) -> Self {
        Self {
            outputs: vec![BackendOutput {
                backend_id: backend_id.into(),
                output,
            }],
            primary_backend: None,
        }
    }

    /// 只有恰好一个 Backend 输出，或 Policy 明确选择了主 Backend 时，
    /// 旧的单 output 字段才有定义。
    pub fn single_output(&self) -> Option<CommandOutput> {
        if let Some(backend_id) = &self.primary_backend {
            return self
                .outputs
                .iter()
                .find(|value| &value.backend_id == backend_id)
                .map(|value| value.output.clone());
        }
        (self.outputs.len() == 1).then(|| self.outputs[0].output.clone())
    }
}

/// 机制侧的能力。Adapter 只通过这个接口触及环境。
pub trait Backend: Send + Sync {
    /// Stop residual tool writers, including descendants of completed calls.
    fn cancel_tool_writers(&self, _context: &BackendContext) -> Result<(), BackendError> {
        Ok(())
    }

    /// Runtime creates the identity before composing its internal Backend resources.
    /// The caller supplies operation metadata only; no Branch identity is accepted.
    fn create_environment(&self, context: &BackendContext) -> Result<String, BackendError> {
        let environment_id = super::identity::allocate(self);
        let context = BackendContext {
            environment_id: environment_id.clone(),
            ..context.clone()
        };
        if let Err(mut error) = self.establish_with_context(&context) {
            if self.state(&environment_id).is_some() {
                error.created_environment_id = Some(environment_id);
            }
            return Err(error);
        }
        Ok(environment_id)
    }

    fn create_child_environment(&self, context: &BackendContext) -> Result<String, BackendError> {
        let environment_id = super::identity::allocate(self);
        let parent = &context.environment_id;
        // Existing Backend branching methods receive the parent as their context;
        // their composition layer derives child-local contexts internally.
        if let Err(mut error) = self.branch_with_context(context, parent, &environment_id) {
            if self.state(&environment_id).is_some() {
                error.created_environment_id = Some(environment_id);
            }
            return Err(error);
        }
        Ok(environment_id)
    }
    /// Called only after task writers and candidate work have drained. Non-FS
    /// backends already own their authoritative state and need no host copy.
    fn materialize(&self, _context: &BackendContext) -> Result<FilesystemResult, BackendError> {
        Ok(FilesystemResult::default())
    }
    fn materialize_after(
        &self,
        context: &BackendContext,
        previous: &mut dyn FnMut() -> Result<FilesystemResult, BackendError>,
    ) -> Result<FilesystemResult, BackendError> {
        let mut result = previous()?;
        let publication = self.materialize(context)?;
        result.confirmation.published |= publication.confirmation.published;
        result.conflicts.extend(publication.conflicts);
        Ok(result)
    }
    /// 配置中用于 Policy 路由和结果关联的稳定标识。
    fn backend_id(&self) -> &str {
        "backend"
    }

    /// 这个 Runtime 实际由哪些 Backend 组成。单 Backend 返回它自己；
    /// 组合 Runtime 返回底层连接的有序标识。握手上报用它，control plane 据此
    /// 知道身份，但 Registry 与能力验证不在这里展开。
    fn backend_ids(&self) -> Vec<String> {
        vec![self.backend_id().to_string()]
    }

    fn capabilities(&self) -> RuntimeCapabilities {
        RuntimeCapabilities::default()
    }

    fn establish(&self, environment_id: &str) -> Result<(), BackendError>;

    fn establish_with_context(&self, context: &BackendContext) -> Result<(), BackendError> {
        self.establish(&context.environment_id)
    }

    fn operate(
        &self,
        environment_id: &str,
        command: &[String],
        cancel: &CancelHandle,
    ) -> Result<CommandOutput, BackendError>;

    fn operate_with_context(
        &self,
        context: &BackendContext,
        command: &[String],
        cancel: &CancelHandle,
    ) -> Result<BackendExecution, BackendError> {
        self.operate(&context.environment_id, command, cancel)
            .map(|output| BackendExecution::single(self.backend_id(), output))
    }

    /// Generic environment operations; product tool definitions stay in Integration.
    fn environment_operation(
        &self,
        _context: &BackendContext,
        _operation: &str,
        _arguments: &super::execution_wire::Message,
        _cancel: &CancelHandle,
    ) -> Result<super::execution_wire::Message, BackendError> {
        Err(BackendError::new("environment operations are unavailable"))
    }

    fn tool_context(
        &self,
        _environment_id: &str,
        tool_name: &str,
        _cancel: &CancelHandle,
    ) -> Result<CommandOutput, BackendError> {
        Err(BackendError::new(format!(
            "runtime tool context is unavailable for {tool_name}"
        )))
    }

    fn tool_context_with_context(
        &self,
        context: &BackendContext,
        tool_name: &str,
        cancel: &CancelHandle,
    ) -> Result<BackendExecution, BackendError> {
        self.tool_context(&context.environment_id, tool_name, cancel)
            .map(|output| BackendExecution::single(self.backend_id(), output))
    }

    fn invoke_tool(
        &self,
        _environment_id: &str,
        tool_name: &str,
        _arguments: &Value,
        _cancel: &CancelHandle,
    ) -> Result<CommandOutput, BackendError> {
        Err(BackendError::new(format!(
            "runtime tool is unavailable for {tool_name}"
        )))
    }

    fn invoke_tool_with_context(
        &self,
        context: &BackendContext,
        tool_name: &str,
        arguments: &Value,
        cancel: &CancelHandle,
    ) -> Result<BackendExecution, BackendError> {
        self.invoke_tool(&context.environment_id, tool_name, arguments, cancel)
            .map(|output| BackendExecution::single(self.backend_id(), output))
    }

    /// Filesystem version operations are deliberately a typed seam. The
    /// default is explicit rejection so an unconfigured Backend cannot claim
    /// that an Agent promotion also published files.
    fn filesystem_operation(
        &self,
        _context: &BackendContext,
        _operation: &Operation,
        _cancel: &CancelHandle,
    ) -> Result<FilesystemResult, BackendError> {
        Err(BackendError::new(
            "filesystem operation is unsupported by this Backend",
        ))
    }

    fn branch(&self, parent: &str, child: &str) -> Result<(), BackendError>;

    fn branch_with_context(
        &self,
        context: &BackendContext,
        parent: &str,
        child: &str,
    ) -> Result<(), BackendError> {
        let _ = context;
        self.branch(parent, child)
    }

    /// Shared state needs no copy. Backends must explicitly declare this.
    fn acceptance_state_is_shared(&self) -> bool {
        false
    }

    /// Confirm completion of writers that can outlive an execution request.
    fn drain_acceptance(&self, _source: &str) -> Result<(), BackendError> {
        Err(BackendError::new(
            "backend cannot confirm acceptance writer drain",
        ))
    }

    /// Transfer isolated state after the target is paused and source writers drain.
    fn accept_state(
        &self,
        _target: &BackendContext,
        _source: &str,
        _full: bool,
        _part: Option<&crate::protocol::contract::PartSelection>,
    ) -> Result<FilesystemResult, BackendError> {
        Err(BackendError::new("backend does not support state transfer"))
    }

    /// 把环境提升为权威环境。默认 no-op，供没有环境侧 promote 语义的 Runtime 继承。
    fn promote(&self, _environment_id: &str) -> Result<(), BackendError> {
        Ok(())
    }

    fn promote_with_context(&self, context: &BackendContext) -> Result<(), BackendError> {
        self.promote(&context.environment_id)
    }

    fn suspend(&self, environment_id: &str) -> Result<(), BackendError>;

    fn suspend_with_context(&self, context: &BackendContext) -> Result<(), BackendError> {
        self.suspend(&context.environment_id)
    }

    fn resume(&self, environment_id: &str) -> Result<(), BackendError>;

    fn resume_with_context(&self, context: &BackendContext) -> Result<(), BackendError> {
        self.resume(&context.environment_id)
    }

    fn terminate(&self, environment_id: &str) -> Result<(), BackendError>;

    fn terminate_with_context(&self, context: &BackendContext) -> Result<(), BackendError> {
        self.terminate(&context.environment_id)
    }

    fn cleanup(&self, environment_id: &str) -> Result<(), BackendError>;

    fn cleanup_with_context(&self, context: &BackendContext) -> Result<(), BackendError> {
        self.cleanup(&context.environment_id)
    }

    fn state(&self, environment_id: &str) -> Option<EnvironmentState>;

    fn backend_states(&self, environment_id: &str) -> Vec<BackendState> {
        vec![BackendState {
            backend_id: self.backend_id().to_string(),
            state: self.state(environment_id),
        }]
    }

    fn environments(&self) -> Vec<String>;

    /// Explicit opt-in: serializing an environment ID is not a saved execution.
    fn save_runtime(&self, _path: &std::path::Path) -> Result<(), BackendError> {
        Err(BackendError::new(
            "Runtime does not support restart recovery",
        ))
    }

    /// Save after earlier components. Backends may quiesce their execution
    /// across the preceding saves as well as their own capture.
    fn save_runtime_after(
        &self,
        path: &std::path::Path,
        previous: &mut dyn FnMut() -> Result<(), BackendError>,
    ) -> Result<(), BackendError> {
        previous()?;
        self.save_runtime(path)
    }

    fn load_runtime(&self, _path: &std::path::Path) -> Result<(), BackendError> {
        Err(BackendError::new(
            "Runtime does not support restart recovery",
        ))
    }

    /// Adapter 退出时回收所有仍然存在的环境。
    ///
    /// 默认实现逐个 terminate + cleanup，并且单个失败不会跳过其余环境；
    /// 失败汇总在返回错误中。具体 Backend 可以覆盖，但不应改变“尽力清理
    /// 所有环境”的语义。
    fn shutdown(&self) -> Result<(), BackendError> {
        let mut failures = Vec::new();
        for environment_id in self.environments() {
            if let Err(error) = self.terminate(&environment_id) {
                failures.push(format!("{environment_id}: terminate failed: {error}"));
            }
            if let Err(error) = self.cleanup(&environment_id) {
                failures.push(format!("{environment_id}: cleanup failed: {error}"));
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(BackendError::new(format!(
                "Runtime shutdown partially failed: {}",
                failures.join("; ")
            )))
        }
    }

    /// Collect environment variables for the run host. Mounts remain owned by
    /// the backend that starts environment execution.
    fn launch_fragment(
        &self,
        _context: &BackendContext,
        _launch: &crate::protocol::contract::AgentLaunch,
    ) -> Result<LaunchFragment, BackendError> {
        Ok(LaunchFragment::default())
    }

    /// Start execution for an existing Environment, independently of the run host.
    /// A new operation may retry after partial startup, so an already running
    /// resource for this Environment must be reused.
    fn start_environment_execution(
        &self,
        _context: &BackendContext,
        _launch: &crate::protocol::contract::AgentLaunch,
    ) -> Result<(), BackendError> {
        Ok(())
    }

    /// Release live execution resources while retaining saved environment state.
    fn stop_environment_execution(&self) -> Result<(), BackendError> {
        Ok(())
    }

    // A backend can prepare its execution before a preceding lifecycle step,
    // then publish its own transition once the preceding step succeeds.
    fn suspend_after(
        &self,
        context: &BackendContext,
        previous: &mut dyn FnMut() -> Result<(), BackendError>,
    ) -> Result<(), BackendError> {
        previous()?;
        self.suspend_with_context(context)
    }

    fn resume_after(
        &self,
        context: &BackendContext,
        previous: &mut dyn FnMut() -> Result<(), BackendError>,
    ) -> Result<(), BackendError> {
        previous()?;
        self.resume_with_context(context)
    }

    fn terminate_after(
        &self,
        context: &BackendContext,
        previous: &mut dyn FnMut() -> Result<(), BackendError>,
    ) -> Result<(), BackendError> {
        previous()?;
        self.terminate_with_context(context)
    }
}
