//! Backend 选择 Policy、路由与连接注册。

use std::collections::BTreeSet;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::backend::{Backend, BackendContext, BackendError};
use super::launch_plan::LaunchFragment;
use crate::protocol::contract::Operation;

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
    SaveRuntime,
    LoadRuntime,
    Establish,
    Operate,
    FilesystemFork,
    FilesystemMark,
    FilesystemDiff,
    FilesystemAccept,
    FilesystemDiscard,
    Branch,
    Promote,
    Materialize,
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
            Operation::MaterializeAuthority => Self::Materialize,
            Operation::SaveRuntime { .. } => Self::SaveRuntime,
            Operation::LoadRuntime { .. } => Self::LoadRuntime,
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

fn mount_target(entry: &super::launch_plan::MountEntry) -> &std::path::Path {
    use super::launch_plan::MountEntry;
    match entry {
        MountEntry::RoBind { target, .. }
        | MountEntry::Bind { target, .. }
        | MountEntry::Overlay { target, .. }
        | MountEntry::Proc { target }
        | MountEntry::Dev { target }
        | MountEntry::Tmpfs { target }
        | MountEntry::RemountRo { target }
        | MountEntry::Dir { target } => target,
    }
}

/// 合并多个 Backend 的启动片段。同一个挂载 target 只能由一个 Backend
/// 提供，否则启动参数会出现两个互相覆盖的挂载，且控制面无法知道谁赢。
pub(super) fn merge_launch_fragment(
    target: &mut LaunchFragment,
    source: LaunchFragment,
) -> Result<(), BackendError> {
    let previous_targets: Vec<_>=target.mounts.iter().map(|entry|mount_target(entry).to_path_buf()).collect();
    for entry in source.mounts {
        let source_target = mount_target(&entry).to_path_buf();
        if previous_targets.iter().any(|existing| existing == &source_target)
        {
            return Err(BackendError::new(format!(
                "multiple Backends provide a launch mount for target {}",
                source_target.display()
            )));
        }
        target.mounts.push(entry);
    }
    if target.chdir.is_none() {
        target.chdir = source.chdir;
    }
    target.env.extend(source.env);
    Ok(())
}
