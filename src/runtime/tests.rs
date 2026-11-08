//! Runtime 组合层与 BaseRuntime 生命周期测试。

use super::base::BaseRuntime;
use super::composite::CompositeRuntime;
use super::process::{FreezeMechanism, ProcessBackend};
use super::selection::{BackendConnection, BackendOperation, BackendPolicy, BackendRoute};
use crate::protocol::contract::{
    AgentLaunch, CommandOutput, EnvironmentState, FilesystemConfirmation, FilesystemResult,
    Operation,
};
use crate::runtime::backend::{Backend, BackendContext, BackendError, CancelHandle};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

    struct MockBackend {
        id: String,
        calls: AtomicUsize,
        seen: Mutex<Vec<(BackendContext, Operation)>>,
    }

    impl MockBackend {
        fn new(id: &str) -> Arc<Self> {
            Arc::new(Self {
                id: id.to_string(),
                calls: AtomicUsize::new(0),
                seen: Mutex::new(Vec::new()),
            })
        }
    }

    impl Backend for MockBackend {
        fn backend_id(&self) -> &str { &self.id }
        fn establish(&self, _: &str) -> Result<(), BackendError> { Ok(()) }
        fn operate(&self, _: &str, _: &[String], _: &CancelHandle) -> Result<CommandOutput, BackendError> {
            Ok(CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() })
        }
        fn branch(&self, _: &str, _: &str) -> Result<(), BackendError> { Ok(()) }
        fn suspend(&self, _: &str) -> Result<(), BackendError> { Ok(()) }
        fn resume(&self, _: &str) -> Result<(), BackendError> { Ok(()) }
        fn terminate(&self, _: &str) -> Result<(), BackendError> { Ok(()) }
        fn cleanup(&self, _: &str) -> Result<(), BackendError> { Ok(()) }
        fn state(&self, _: &str) -> Option<EnvironmentState> { Some(EnvironmentState::Available) }
        fn environments(&self) -> Vec<String> { vec!["env".to_string()] }

        fn filesystem_operation(
            &self,
            context: &BackendContext,
            operation: &Operation,
            _: &CancelHandle,
        ) -> Result<FilesystemResult, BackendError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.seen.lock().unwrap().push((context.clone(), operation.clone()));
            Ok(FilesystemResult {
                snapshot_id: Some(self.id.clone()),
                confirmation: FilesystemConfirmation {
                    snapshot_created: true,
                    metadata_durable: true,
                    published: true,
                    ..Default::default()
                },
                ..Default::default()
            })
        }
    }

    fn fork_operation() -> Operation {
        Operation::FilesystemFork {
            parent_snapshot_id: "parent".into(),
            child_snapshot_id: "child".into(),
            path_scope: vec![],
        }
    }

    #[test]
    fn filesystem_policy_selects_backend_and_forwards_context_and_operation() {
        let left = MockBackend::new("left");
        let right = MockBackend::new("right");
        let runtime = CompositeRuntime::new(
            "runtime",
            vec![
                BackendConnection::new("left", left.clone()).unwrap(),
                BackendConnection::new("right", right.clone()).unwrap(),
            ],
            BackendPolicy {
                default: vec!["right".into()],
                routes: vec![BackendRoute {
                    operation: Some(BackendOperation::FilesystemFork),
                    task_id: Some("task".into()),
                    backends: vec!["left".into()],
                    ..Default::default()
                }],
            },
        ).unwrap();
        let context = BackendContext {
            task_id: Some("task".into()),
            environment_id: "env".into(),
            ..Default::default()
        };
        let cancel = CancelHandle::default();
        runtime.filesystem_operation(&context, &fork_operation(), &cancel).unwrap();
        assert_eq!(left.calls.load(Ordering::SeqCst), 1);
        assert_eq!(right.calls.load(Ordering::SeqCst), 0);
        let seen = left.seen.lock().unwrap();
        assert_eq!(seen[0].0, context);
        assert_eq!(seen[0].1, fork_operation());
    }

    #[test]
    fn filesystem_multiple_selection_and_cancellation_make_no_calls() {
        let left = MockBackend::new("left");
        let right = MockBackend::new("right");
        let runtime = CompositeRuntime::new(
            "runtime",
            vec![
                BackendConnection::new("left", left.clone()).unwrap(),
                BackendConnection::new("right", right.clone()).unwrap(),
            ],
            BackendPolicy { default: vec!["left".into(), "right".into()], routes: vec![] },
        ).unwrap();
        let context = BackendContext::new("env");
        let cancel = CancelHandle::default();
        assert!(runtime.filesystem_operation(&context, &fork_operation(), &cancel).is_err());
        cancel.cancel();
        assert!(runtime.filesystem_operation(&context, &fork_operation(), &cancel).is_err());
        assert_eq!(left.calls.load(Ordering::SeqCst), 0);
        assert_eq!(right.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn base_runtime_freezes_and_marks_writers_quiesced() {
        let backend = MockBackend::new("mock");
        let runtime = BaseRuntime::new("runtime", backend);
        let context = BackendContext::new("env");
        let log = PathBuf::from(format!(
            "/tmp/opencode/ascout-freeze-test-{}.log",
            std::process::id()
        ));
        let launch = AgentLaunch {
            command: "/bin/sleep".into(),
            args: vec!["5".into()],
            cwd: PathBuf::from("/tmp"),
            log: log.clone(),
            instance_id: Some("test-agent".into()),
            env: BTreeMap::new(),
        };
        runtime.launch_agent(&context, &launch).unwrap();
        let operation = Operation::FilesystemFork {
            parent_snapshot_id: "env".into(),
            child_snapshot_id: "child".into(),
            path_scope: vec![],
        };
        let result = runtime
            .filesystem_operation(&context, &operation, &CancelHandle::default())
            .unwrap();
        assert!(
            result.confirmation.writers_quiesced,
            "freeze must be reflected in writers_quiesced"
        );
        runtime.stop_agents().unwrap();
        let _ = std::fs::remove_file(log);

        let result = runtime
            .filesystem_operation(
                &BackendContext::new("missing"),
                &Operation::FilesystemFork {
                    parent_snapshot_id: "missing".into(),
                    child_snapshot_id: "child".into(),
                    path_scope: vec![],
                },
                &CancelHandle::default(),
            )
            .unwrap();
        assert!(!result.confirmation.writers_quiesced);
    }

    fn launch_test_agent(runtime: &BaseRuntime, command: &str, args: Vec<String>) -> BackendContext {
        let context = BackendContext::new("env");
        let log = PathBuf::from(format!(
            "/tmp/opencode/ascout-freeze-test-{}-{}.log",
            std::process::id(),
            command.replace('/', "_")
        ));
        let launch = AgentLaunch {
            command: command.to_string(),
            args,
            cwd: PathBuf::from("/tmp"),
            log: log.clone(),
            instance_id: Some("test-agent".into()),
            env: BTreeMap::new(),
        };
        runtime.launch_agent(&context, &launch).unwrap();
        context
    }

    fn env_fork_operation() -> Operation {
        Operation::FilesystemFork {
            parent_snapshot_id: "env".into(),
            child_snapshot_id: "child".into(),
            path_scope: vec![],
        }
    }

    #[test]
    fn base_runtime_without_freeze_mechanism_still_runs_operations() {
        let mut runtime = BaseRuntime::new("runtime", MockBackend::new("mock"));
        runtime.process = ProcessBackend::with_mechanism(
            "process",
            "bwrap",
            FreezeMechanism::None,
        );
        let context = launch_test_agent(&runtime, "/bin/sleep", vec!["3".into()]);
        let result = runtime
            .filesystem_operation(&context, &env_fork_operation(), &CancelHandle::default())
            .unwrap();
        assert!(!result.confirmation.writers_quiesced);
        runtime.stop_agents().unwrap();
    }

    #[test]
    fn freeze_failure_fails_before_filesystem_operation() {
        let runtime = BaseRuntime::new("runtime", MockBackend::new("mock"));
        let context = launch_test_agent(&runtime, "/bin/sleep", vec!["3".into()]);
        runtime.process.inject_freeze_failure(true);
        let error = runtime
            .filesystem_operation(&context, &env_fork_operation(), &CancelHandle::default())
            .unwrap_err();
        assert!(
            error.message.contains("injected freeze failure"),
            "{}",
            error.message
        );
        runtime.process.inject_freeze_failure(false);
        runtime.stop_agents().unwrap();
    }

    #[test]
    fn thaw_failure_is_reported_as_cleanup_failure() {
        let runtime = BaseRuntime::new("runtime", MockBackend::new("mock"));
        let context = launch_test_agent(&runtime, "/bin/sleep", vec!["3".into()]);
        runtime.process.inject_thaw_failure(true);
        let error = runtime
            .filesystem_operation(&context, &env_fork_operation(), &CancelHandle::default())
            .unwrap_err();
        assert!(error.message.contains("thaw"), "{}", error.message);
        assert!(
            error
                .cleanup_failures
                .iter()
                .any(|value| value.contains("thaw")),
            "missing thaw cleanup failure: {:?}",
            error.cleanup_failures
        );
        runtime.process.inject_thaw_failure(false);
        runtime.stop_agents().unwrap();
    }

    #[test]
    fn exited_process_does_not_block_filesystem_operation() {
        let runtime = BaseRuntime::new("runtime", MockBackend::new("mock"));
        let context = launch_test_agent(&runtime, "/bin/true", Vec::new());
        for _ in 0..200 {
            if !runtime.process.is_running("env").unwrap() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(!runtime.process.is_running("env").unwrap());
        assert!(runtime.process.process_handle("env").is_some());
        let result = runtime
            .filesystem_operation(&context, &env_fork_operation(), &CancelHandle::default())
            .unwrap();
        assert!(!result.confirmation.writers_quiesced);
        runtime.stop_agents().unwrap();
    }

    #[test]
    fn backend_default_filesystem_operation_is_unsupported() {
        struct DefaultBackend;
        impl Backend for DefaultBackend {
            fn establish(&self, _: &str) -> Result<(), BackendError> { Ok(()) }
            fn operate(&self, _: &str, _: &[String], _: &CancelHandle) -> Result<CommandOutput, BackendError> {
                Err(BackendError::new("unused"))
            }
            fn branch(&self, _: &str, _: &str) -> Result<(), BackendError> { Ok(()) }
            fn suspend(&self, _: &str) -> Result<(), BackendError> { Ok(()) }
            fn resume(&self, _: &str) -> Result<(), BackendError> { Ok(()) }
            fn terminate(&self, _: &str) -> Result<(), BackendError> { Ok(()) }
            fn cleanup(&self, _: &str) -> Result<(), BackendError> { Ok(()) }
            fn state(&self, _: &str) -> Option<EnvironmentState> { None }
            fn environments(&self) -> Vec<String> { Vec::new() }
        }
        let backend = DefaultBackend;
        let cancel = CancelHandle::default();
        let result = Backend::filesystem_operation(
            &backend,
            &BackendContext::new("env"),
            &Operation::OperateEnvironment {
                command: vec!["true".into()],
                agent: None,
            },
            &cancel,
        );
        assert_eq!(result.unwrap_err().message, "filesystem operation is unsupported by this Backend");
    }
