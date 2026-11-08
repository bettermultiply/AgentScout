//! 请求结果构造与文件系统确认校验。

use crate::protocol::contract::{
    EnvironmentState, FilesystemResult, Operation, OperationRequest, OperationResult, ResultStatus,
};

pub(super) fn filesystem_failure_reason(operation: &Operation, result: &FilesystemResult) -> Option<String> {
    if !result.conflicts.is_empty() {
        return Some(format!(
            "filesystem operation has {} conflict(s); publication was not accepted",
            result.conflicts.len()
        ));
    }
    let missing = match operation {
        Operation::FilesystemFork { .. } => (!result.confirmation.snapshot_created)
            .then_some("snapshot_created")
            .or_else(|| (!result.confirmation.metadata_durable).then_some("metadata_durable")),
        Operation::FilesystemMark { .. } => (!result.confirmation.metadata_durable)
            .then_some("metadata_durable"),
        Operation::FilesystemAccept { .. } => (!result.confirmation.published)
            .then_some("published"),
        _ => None,
    };
    missing.map(|field| format!("filesystem operation is not confirmed: missing {field}"))
}

pub(super) fn result(
    request: &OperationRequest,
    status: ResultStatus,
    environment_state: Option<EnvironmentState>,
    can_accept_operations: bool,
) -> OperationResult {
    OperationResult {
        request_id: request.request_id.clone(),
        environment_id: request.environment_id.clone(),
        status,
        environment_state,
        can_accept_operations,
        follow_up: Vec::new(),
        resources: Vec::new(),
        remaining: Vec::new(),
        backend_states: Vec::new(),
        outputs: Vec::new(),
        reason: None,
        output: None,
        filesystem: None,
    }
}

pub(super) fn can_accept(state: Option<EnvironmentState>) -> bool {
    matches!(
        state,
        Some(EnvironmentState::Available | EnvironmentState::Suspended)
    )
}

