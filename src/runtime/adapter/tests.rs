//! Adapter 结果确认与结果窗口测试。

#[cfg(test)]
mod shutdown_tests {
    use crate::protocol::contract::{
        CommandOutput, EnvironmentState, RuntimeCapabilities,
    };
    use crate::runtime::adapter::Adapter;
    use crate::runtime::backend::{Backend, BackendError, CancelHandle};
    use std::sync::{Arc, Mutex};

    struct ShutdownBackend {
        environments: Vec<String>,
        terminated: Mutex<Vec<String>>,
        cleaned: Mutex<Vec<String>>,
        fail_terminate: Option<String>,
    }

    impl Backend for ShutdownBackend {
        fn capabilities(&self) -> RuntimeCapabilities {
            RuntimeCapabilities {
                branch_environment: true,
                shared_environment: false,
                ..RuntimeCapabilities::default()
            }
        }
        fn establish(&self, _: &str) -> Result<(), BackendError> { Ok(()) }
        fn operate(
            &self,
            _: &str,
            _: &[String],
            _: &CancelHandle,
        ) -> Result<CommandOutput, BackendError> {
            Err(BackendError::new("unused"))
        }
        fn branch(&self, _: &str, _: &str) -> Result<(), BackendError> { Ok(()) }
        fn suspend(&self, _: &str) -> Result<(), BackendError> { Ok(()) }
        fn resume(&self, _: &str) -> Result<(), BackendError> { Ok(()) }
        fn terminate(&self, environment_id: &str) -> Result<(), BackendError> {
            self.terminated
                .lock()
                .unwrap()
                .push(environment_id.to_string());
            if self.fail_terminate.as_deref() == Some(environment_id) {
                return Err(BackendError::new("injected terminate failure"));
            }
            Ok(())
        }
        fn cleanup(&self, environment_id: &str) -> Result<(), BackendError> {
            self.cleaned
                .lock()
                .unwrap()
                .push(environment_id.to_string());
            Ok(())
        }
        fn state(&self, _: &str) -> Option<EnvironmentState> {
            Some(EnvironmentState::Available)
        }
        fn environments(&self) -> Vec<String> {
            self.environments.clone()
        }
    }

    #[test]
    fn adapter_disconnect_terminates_and_cleans_every_environment() {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("control.sock");
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let backend = Arc::new(ShutdownBackend {
            environments: vec!["one".into(), "two".into(), "three".into()],
            terminated: Mutex::new(Vec::new()),
            cleaned: Mutex::new(Vec::new()),
            fail_terminate: Some("two".into()),
        });
        let adapter_backend = backend.clone();
        let adapter = std::thread::spawn(move || {
            Adapter {
                adapter_instance: "test".into(),
                runtime: adapter_backend,
            }
            .run(&socket)
            .unwrap();
        });
        let (mut stream, _) = listener.accept().unwrap();
        // Read the hello so the Adapter is fully running before disconnect.
        let _: Option<crate::protocol::contract::AdapterMessage> =
            crate::protocol::transport::read_message(&mut stream).unwrap();
        drop(stream);
        adapter.join().unwrap();

        assert_eq!(
            *backend.terminated.lock().unwrap(),
            vec!["one".to_string(), "two".to_string(), "three".to_string()]
        );
        assert_eq!(
            *backend.cleaned.lock().unwrap(),
            vec!["one".to_string(), "two".to_string(), "three".to_string()]
        );
    }
}

#[cfg(test)]
mod queue_panic_tests {
    use crate::protocol::contract::{
        AdapterMessage, CommandOutput, EnvironmentState, Operation, OperationRequest,
        ResultStatus, RuntimeCapabilities,
    };
    use crate::runtime::backend::{Backend, BackendError, CancelHandle};
    use crate::runtime::adapter::queue::Queue;
    use crate::runtime::adapter::{ResultCache, Shared};
    use std::collections::{HashMap, HashSet};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Condvar, Mutex, mpsc};
    use std::time::Duration;

    struct PanicOnce {
        calls: AtomicUsize,
    }

    impl Backend for PanicOnce {
        fn capabilities(&self) -> RuntimeCapabilities {
            RuntimeCapabilities::default()
        }
        fn establish(&self, _: &str) -> Result<(), BackendError> { Ok(()) }
        fn operate(
            &self,
            _: &str,
            _: &[String],
            _: &CancelHandle,
        ) -> Result<CommandOutput, BackendError> {
            if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                panic!("injected operation panic");
            }
            Ok(CommandOutput {
                exit_code: 0,
                stdout: "ok".into(),
                stderr: String::new(),
            })
        }
        fn branch(&self, _: &str, _: &str) -> Result<(), BackendError> { Ok(()) }
        fn suspend(&self, _: &str) -> Result<(), BackendError> { Ok(()) }
        fn resume(&self, _: &str) -> Result<(), BackendError> { Ok(()) }
        fn terminate(&self, _: &str) -> Result<(), BackendError> { Ok(()) }
        fn cleanup(&self, _: &str) -> Result<(), BackendError> { Ok(()) }
        fn state(&self, _: &str) -> Option<EnvironmentState> {
            Some(EnvironmentState::Available)
        }
        fn environments(&self) -> Vec<String> { vec!["env".into()] }
    }

    fn request(id: &str) -> OperationRequest {
        OperationRequest {
            request_id: id.to_string(),
            generation: 0,
            environment_id: "env".into(),
            operation: Operation::OperateEnvironment {
                command: vec!["true".into()],
                agent: None,
            },
            task_id: None,
            branch_id: None,
        }
    }

    fn wait_result(receiver: &mpsc::Receiver<AdapterMessage>, id: &str) -> crate::protocol::contract::OperationResult {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            match receiver.recv_timeout(remaining).unwrap() {
                AdapterMessage::Result(result) if result.request_id == id => return result,
                _ => {}
            }
        }
    }

    #[test]
    fn panicking_operation_does_not_kill_the_environment_queue() {
        let (out, receiver) = mpsc::channel();
        let shared = Arc::new(Shared {
            runtime: Arc::new(PanicOnce {
                calls: AtomicUsize::new(0),
            }),
            out,
            queues: Mutex::new(HashMap::new()),
            seen: Mutex::new(HashSet::new()),
            results: Mutex::new(ResultCache::default()),
            result_signal: Condvar::new(),
            known: Mutex::new(HashMap::new()),
            agent: Mutex::new(None),
        });
        let queue = Queue::start(shared.clone(), "env".into());
        queue.enqueue(request("first"), false);
        let first = wait_result(&receiver, "first");
        assert_eq!(first.status, ResultStatus::Failed);
        assert!(first.reason.unwrap().contains("panicked"));

        queue.enqueue(request("second"), false);
        let second = wait_result(&receiver, "second");
        assert_eq!(second.status, ResultStatus::Completed, "{second:?}");
    }
}

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
