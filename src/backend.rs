//! Adapter 内部的后端接口。
//!
//! 后端是机制的封装：Adapter 负责会话、顺序和结果语义，后端负责真正建立和
//! 维持环境。接口只属于 AScout 自己的 Adapter，不是对外契约的一部分。

use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Value;

use crate::contract::{CommandOutput, EnvironmentState, RuntimeCapabilities};

/// Adapter 用它通知后端当前操作应当停止。
#[derive(Default)]
pub struct CancelHandle {
    cancelled: AtomicBool,
}

impl CancelHandle {
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
    pub message: String,
    pub cancelled: bool,
}

impl BackendError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            cancelled: false,
        }
    }

    pub fn interrupted() -> Self {
        Self {
            message: "operation interrupted by suspend/terminate".to_string(),
            cancelled: true,
        }
    }
}

impl std::fmt::Display for BackendError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for BackendError {}

/// 机制侧的能力。Adapter 只通过这个接口触及环境。
pub trait Backend: Send + Sync {
    fn capabilities(&self) -> RuntimeCapabilities {
        RuntimeCapabilities::default()
    }
    fn establish(&self, environment_id: &str) -> Result<(), BackendError>;
    fn operate(
        &self,
        environment_id: &str,
        command: &[String],
        cancel: &CancelHandle,
    ) -> Result<CommandOutput, BackendError>;
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
    fn branch(&self, parent: &str, child: &str) -> Result<(), BackendError>;
    /// 把环境提升为权威环境。默认 no-op，供没有环境侧 promote 语义的 Runtime 继承。
    fn promote(&self, _environment_id: &str) -> Result<(), BackendError> {
        Ok(())
    }
    fn suspend(&self, environment_id: &str) -> Result<(), BackendError>;
    fn resume(&self, environment_id: &str) -> Result<(), BackendError>;
    fn terminate(&self, environment_id: &str) -> Result<(), BackendError>;
    fn cleanup(&self, environment_id: &str) -> Result<(), BackendError>;
    fn state(&self, environment_id: &str) -> Option<EnvironmentState>;
    fn environments(&self) -> Vec<String>;
}
