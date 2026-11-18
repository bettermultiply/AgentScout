//! Runtime proxy 结果确认与结果窗口测试。

#[cfg(test)]
mod identity_tests {
    use super::super::*;
    use crate::control_plane::gateway::runtime::{ControlServer, Runtime};
    use crate::protocol::contract::*;
    use crate::runtime::backend::*;
    use std::time::Duration;

    #[derive(Default)]
    struct FakeBackend(
        Mutex<HashMap<String, EnvironmentState>>,
        std::sync::atomic::AtomicBool,
    );
    impl Backend for FakeBackend {
        fn establish(&self, id: &str) -> Result<(), BackendError> {
            assert!(!id.is_empty());
            assert!(
                self.0
                    .lock()
                    .unwrap()
                    .insert(id.into(), EnvironmentState::Available)
                    .is_none()
            );
            Ok(())
        }
        fn branch(&self, parent: &str, child: &str) -> Result<(), BackendError> {
            assert_ne!(parent, child);
            assert_eq!(self.state(parent), Some(EnvironmentState::Available));
            self.establish(child)?;
            if self.1.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(BackendError::new("injected failure after creation"));
            }
            Ok(())
        }
        fn operate(
            &self,
            id: &str,
            _: &[String],
            _: &CancelHandle,
        ) -> Result<CommandOutput, BackendError> {
            assert_eq!(self.state(id), Some(EnvironmentState::Available));
            Ok(CommandOutput {
                exit_code: 0,
                stdout: id.into(),
                stderr: String::new(),
            })
        }
        fn suspend(&self, id: &str) -> Result<(), BackendError> {
            self.0
                .lock()
                .unwrap()
                .insert(id.into(), EnvironmentState::Suspended);
            Ok(())
        }
        fn resume(&self, id: &str) -> Result<(), BackendError> {
            self.establish(id)
        }
        fn terminate(&self, id: &str) -> Result<(), BackendError> {
            self.0
                .lock()
                .unwrap()
                .insert(id.into(), EnvironmentState::Terminated);
            Ok(())
        }
        fn cleanup(&self, id: &str) -> Result<(), BackendError> {
            self.0
                .lock()
                .unwrap()
                .insert(id.into(), EnvironmentState::Cleaned);
            Ok(())
        }
        fn state(&self, id: &str) -> Option<EnvironmentState> {
            self.0.lock().unwrap().get(id).copied()
        }
        fn environments(&self) -> Vec<String> {
            self.0.lock().unwrap().keys().cloned().collect()
        }
    }

    #[test]
    fn runtime_returns_independent_identities_and_retry_returns_the_original_creation() {
        let server = ControlServer::bind("127.0.0.1:0").unwrap();
        let address = server.local_addr().unwrap().to_string();
        let backend = Arc::new(FakeBackend::default());
        let adapter_backend = Arc::new(
            crate::runtime::BaseRuntime::with_policy(
                "fake-runtime",
                vec![
                    crate::runtime::selection::BackendConnection::new(
                        "fake-backend",
                        backend.clone(),
                    )
                    .unwrap(),
                ],
                crate::runtime::selection::BackendPolicy::single("fake-backend"),
            )
            .unwrap(),
        );
        let adapter = std::thread::spawn(move || {
            RuntimeProxy {
                adapter_instance: "fake".into(),
                runtime: adapter_backend,
            }
            .run(&address)
            .unwrap()
        });
        let session = server.accept().unwrap();
        session.wait_for_hello(Duration::from_secs(5)).unwrap();
        let parent = session
            .call_with_id("create", "", Operation::EstablishEnvironment)
            .unwrap();
        assert_eq!(parent.status, ResultStatus::Completed);
        assert!(!parent.environment_id.is_empty());
        let retry = session
            .call_with_id("create", "", Operation::EstablishEnvironment)
            .unwrap();
        assert_eq!(parent, retry);
        let child = session.branch(&parent.environment_id, None).unwrap();
        assert_eq!(child.status, ResultStatus::Completed);
        assert_ne!(child.environment_id, parent.environment_id);
        let sibling = session.branch(&parent.environment_id, None).unwrap();
        assert_ne!(sibling.environment_id, child.environment_id);
        assert_eq!(backend.environments().len(), 3);
        session
            .call(&child.environment_id, Operation::SuspendEnvironment)
            .unwrap();
        assert_eq!(
            backend.state(&parent.environment_id),
            Some(EnvironmentState::Available)
        );
        assert_eq!(
            backend.state(&sibling.environment_id),
            Some(EnvironmentState::Available)
        );
        let output = session
            .call(
                &sibling.environment_id,
                Operation::OperateEnvironment {
                    command: vec!["observe".into()],
                    agent: None,
                },
            )
            .unwrap();
        assert_eq!(output.output.unwrap().stdout, sibling.environment_id);
        let rejected = session
            .call("control-plane-invented", Operation::EstablishEnvironment)
            .unwrap();
        assert_eq!(rejected.status, ResultStatus::Failed);
        assert_eq!(backend.environments().len(), 3);
        backend.1.store(true, std::sync::atomic::Ordering::Relaxed);
        let failed = session.branch(&parent.environment_id, None).unwrap();
        assert_eq!(failed.status, ResultStatus::Failed);
        assert!(!failed.environment_id.is_empty());
        assert_ne!(failed.environment_id, parent.environment_id);
        assert_eq!(
            backend.state(&failed.environment_id),
            Some(EnvironmentState::Cleaned),
            "creation rollback must stop and clean the failing participant before replying"
        );
        let terminated = session
            .call(&failed.environment_id, Operation::TerminateEnvironment)
            .unwrap();
        assert_eq!(terminated.status, ResultStatus::Completed);
        let cleaned = session
            .call(&failed.environment_id, Operation::CleanupEnvironment)
            .unwrap();
        assert_eq!(cleaned.status, ResultStatus::Completed);
        assert_eq!(cleaned.environment_state, Some(EnvironmentState::Cleaned));
        assert_eq!(
            backend.state(&parent.environment_id),
            Some(EnvironmentState::Available)
        );
        session.shutdown().unwrap();
        adapter.join().unwrap();
    }
}

#[cfg(test)]
mod queue_panic_tests {
    use crate::protocol::contract::{
        AdapterMessage, CommandOutput, EnvironmentState, Operation, OperationRequest, ResultStatus,
        RuntimeCapabilities,
    };
    use crate::runtime::BaseRuntime;
    use crate::runtime::backend::{Backend, BackendError, CancelHandle};
    use crate::runtime::proxy::queue::Queue;
    use crate::runtime::proxy::{ResultCache, Shared};
    use std::collections::HashMap;
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
        fn establish(&self, _: &str) -> Result<(), BackendError> {
            Ok(())
        }
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
        fn branch(&self, _: &str, _: &str) -> Result<(), BackendError> {
            Ok(())
        }
        fn suspend(&self, _: &str) -> Result<(), BackendError> {
            Ok(())
        }
        fn resume(&self, _: &str) -> Result<(), BackendError> {
            Ok(())
        }
        fn terminate(&self, _: &str) -> Result<(), BackendError> {
            Ok(())
        }
        fn cleanup(&self, _: &str) -> Result<(), BackendError> {
            Ok(())
        }
        fn state(&self, _: &str) -> Option<EnvironmentState> {
            Some(EnvironmentState::Available)
        }
        fn environments(&self) -> Vec<String> {
            vec!["env".into()]
        }
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
        }
    }

    fn wait_result(
        receiver: &mpsc::Receiver<AdapterMessage>,
        id: &str,
    ) -> crate::protocol::contract::OperationResult {
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
    fn reused_request_content_is_rejected_before_receipt_and_never_executed() {
        let (out, receiver) = mpsc::channel();
        let backend = Arc::new(PanicOnce {
            calls: AtomicUsize::new(1),
        });
        let runtime = Arc::new(BaseRuntime::new("test", backend.clone()));
        runtime.establish("env").unwrap();
        let shared = Arc::new(Shared {
            commands: None,
            runtime,
            out,
            queues: Mutex::new(HashMap::new()),
            seen: Mutex::new(Default::default()),
            results: Mutex::new(ResultCache::default()),
            result_signal: Condvar::new(),
            known: Mutex::new(HashMap::new()),
        });
        shared.dispatch(request("once"));
        assert_eq!(
            wait_result(&receiver, "once").status,
            ResultStatus::Completed
        );
        while receiver.try_recv().is_ok() {}
        let mut conflict = request("once");
        conflict.generation += 1;
        shared.dispatch(conflict);
        let AdapterMessage::Rejected { reason, .. } = receiver.recv().unwrap() else {
            panic!("conflicting request must not receive a reception receipt")
        };
        assert!(reason.contains("different content"));
        shared.dispatch(request("once"));
        assert_eq!(
            wait_result(&receiver, "once").status,
            ResultStatus::Completed
        );
        assert_eq!(backend.calls.load(Ordering::SeqCst), 2);
        // Simulate eviction of the result payload without dropping its identity.
        *shared.results.lock().unwrap() = ResultCache::default();
        shared.dispatch(request("once"));
        let expired = wait_result(&receiver, "once");
        assert_eq!(expired.status, ResultStatus::Failed);
        assert!(expired.reason.unwrap().contains("will not execute again"));
        assert_eq!(backend.calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn expired_numeric_request_never_reaches_backend() {
        let (out, receiver) = mpsc::channel();
        let backend = Arc::new(PanicOnce {
            calls: AtomicUsize::new(1),
        });
        let runtime = Arc::new(BaseRuntime::new("test", backend.clone()));
        runtime.establish("env").unwrap();
        let shared = Arc::new(Shared {
            commands: None,
            runtime,
            out,
            queues: Mutex::new(HashMap::new()),
            seen: Mutex::new(Default::default()),
            results: Mutex::new(ResultCache::default()),
            result_signal: Condvar::new(),
            known: Mutex::new(HashMap::new()),
        });
        for n in 1..600 {
            let id = format!("req-{n}");
            shared.dispatch(request(&id));
            assert_eq!(wait_result(&receiver, &id).status, ResultStatus::Completed);
        }
        while receiver.try_recv().is_ok() {}
        let before = backend.calls.load(Ordering::SeqCst);
        shared.dispatch(request("req-1"));
        assert!(
            matches!(receiver.recv().unwrap(), AdapterMessage::Rejected { reason, .. } if reason.contains("expired"))
        );
        assert_eq!(backend.calls.load(Ordering::SeqCst), before);
    }

    #[test]
    fn panicking_operation_does_not_kill_the_environment_queue() {
        let (out, receiver) = mpsc::channel();
        let backend = Arc::new(PanicOnce {
            calls: AtomicUsize::new(0),
        });
        let runtime = Arc::new(BaseRuntime::new("test", backend.clone()));
        runtime.establish("env").unwrap();
        let shared = Arc::new(Shared {
            commands: None,
            runtime,
            out,
            queues: Mutex::new(HashMap::new()),
            seen: Mutex::new(Default::default()),
            results: Mutex::new(ResultCache::default()),
            result_signal: Condvar::new(),
            known: Mutex::new(HashMap::new()),
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

    use crate::protocol::contract::*;
    use crate::runtime::proxy::result::{filesystem_failure_reason, result};
    use crate::runtime::proxy::result_cache::{RESULT_CACHE_LIMIT, ResultCache};

    use crate::protocol::contract::{FilesystemConflict, PathScope};

    #[test]
    fn accept_requires_publication_and_rejects_conflicts() {
        let operation = Operation::FilesystemAccept {
            base_snapshot_id: "base".into(),
            source_snapshot_id: "source".into(),
            target_snapshot_id: "target".into(),
            path_scope: vec![PathScope::Workspace, PathScope::Tmp],
        };
        let mut result = FilesystemResult::default();
        assert!(filesystem_failure_reason(&operation, &result).is_some());
        result.confirmation.published = true;
        assert!(filesystem_failure_reason(&operation, &result).is_none());
        result.conflicts.push(FilesystemConflict {
            path: "workspace/a".into(),
            kind: "modify_delete".into(),
            reason: "conflict".into(),
            ..Default::default()
        });
        assert!(filesystem_failure_reason(&operation, &result).is_some());
    }

    #[test]
    fn fork_requires_creation_and_durable_metadata() {
        let operation = Operation::FilesystemFork {
            parent_snapshot_id: "base".into(),
            child_snapshot_id: "child".into(),
            path_scope: vec![PathScope::Workspace, PathScope::Tmp],
        };
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
            };
            last_evicted = cache.insert(
                request.request_id.clone(),
                result(&request, ResultStatus::Completed, None, false),
            );
        }
        assert_eq!(cache.entries.len(), RESULT_CACHE_LIMIT);
        assert_eq!(last_evicted.len(), 1);
        assert!(!cache.entries.contains_key("request-0"));
        assert!(
            cache
                .entries
                .contains_key(&format!("request-{}", RESULT_CACHE_LIMIT + 7))
        );
    }
}
