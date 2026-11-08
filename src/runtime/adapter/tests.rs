//! Adapter 结果确认与结果窗口测试。

#[cfg(test)]
mod filesystem_confirmation_tests {
    
use crate::runtime::adapter::result::{filesystem_failure_reason, result};
use crate::runtime::adapter::result_cache::{ResultCache, RESULT_CACHE_LIMIT};
use crate::protocol::contract::*;

    use crate::protocol::contract::{FilesystemConflict, PathScope};

    #[test]
    fn accept_requires_publication_and_rejects_conflicts() {
        let operation = Operation::FilesystemAccept {
            base_snapshot_id: "base".into(), source_snapshot_id: "source".into(),
            target_snapshot_id: "target".into(), path_scope: vec![PathScope::Workspace, PathScope::Tmp],
        };
        let mut result = FilesystemResult::default();
        assert!(filesystem_failure_reason(&operation, &result).is_some());
        result.confirmation.published = true;
        assert!(filesystem_failure_reason(&operation, &result).is_none());
        result.conflicts.push(FilesystemConflict { path: "workspace/a".into(), kind: "modify_delete".into(), reason: "conflict".into(), ..Default::default() });
        assert!(filesystem_failure_reason(&operation, &result).is_some());
    }

    #[test]
    fn fork_requires_creation_and_durable_metadata() {
        let operation = Operation::FilesystemFork { parent_snapshot_id: "base".into(), child_snapshot_id: "child".into(), path_scope: vec![PathScope::Workspace, PathScope::Tmp] };
        let mut result = FilesystemResult::default();
        assert!(filesystem_failure_reason(&operation, &result).is_some());
        result.confirmation.snapshot_created = true;
        assert!(filesystem_failure_reason(&operation, &result).is_some());
        result.confirmation.metadata_durable = true;
        assert!(filesystem_failure_reason(&operation, &result).is_none());
    }

    #[test]
    fn completed_result_cache_is_bounded() {
        let mut cache = ResultCache::default();
        let mut last_evicted = Vec::new();
        for index in 0..(RESULT_CACHE_LIMIT + 8) {
            let request = OperationRequest {
                request_id: format!("request-{index}"),
                generation: 0,
                environment_id: "env".into(),
                operation: Operation::PromoteEnvironment,
                task_id: None,
                branch_id: None,
            };
            last_evicted = cache.insert(
                request.request_id.clone(),
                result(&request, ResultStatus::Completed, None, false),
            );
        }
        assert_eq!(cache.entries.len(), RESULT_CACHE_LIMIT);
        assert_eq!(last_evicted.len(), 1);
        assert!(!cache.entries.contains_key("request-0"));
        assert!(cache.entries.contains_key(&format!(
            "request-{}",
            RESULT_CACHE_LIMIT + 7
        )));
    }
}
