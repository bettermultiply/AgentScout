//! Runtime 组合层与 BaseRuntime 生命周期测试。

use super::base::BaseRuntime;
use super::composite::CompositeRuntime;
use super::launch_plan::{LaunchFragment, MountEntry};
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
        let log_dir = tempfile::tempdir().unwrap();
        let log = log_dir.path().join("agent.log");
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

    fn launch_test_agent(
        runtime: &BaseRuntime,
        command: &str,
        args: Vec<String>,
    ) -> (BackendContext, tempfile::TempDir) {
        let context = BackendContext::new("env");
        let log_dir = tempfile::tempdir().unwrap();
        let log = log_dir.path().join("agent.log");
        let launch = AgentLaunch {
            command: command.to_string(),
            args,
            cwd: PathBuf::from("/tmp"),
            log,
            instance_id: Some("test-agent".into()),
            env: BTreeMap::new(),
        };
        runtime.launch_agent(&context, &launch).unwrap();
        (context, log_dir)
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
        let (context, _log_dir) = launch_test_agent(&runtime, "/bin/sleep", vec!["3".into()]);
        let result = runtime
            .filesystem_operation(&context, &env_fork_operation(), &CancelHandle::default())
            .unwrap();
        assert!(!result.confirmation.writers_quiesced);
        runtime.stop_agents().unwrap();
    }

    #[test]
    fn freeze_failure_fails_before_filesystem_operation() {
        let runtime = BaseRuntime::new("runtime", MockBackend::new("mock"));
        let (context, _log_dir) = launch_test_agent(&runtime, "/bin/sleep", vec!["3".into()]);
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
        let (context, _log_dir) = launch_test_agent(&runtime, "/bin/sleep", vec!["3".into()]);
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
        let (context, _log_dir) = launch_test_agent(&runtime, "/bin/true", Vec::new());
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
    fn base_runtime_accepts_any_backend_as_data_plane() {
        let runtime = BaseRuntime::new("runtime", MockBackend::new("inner"))
            .with_data_plane(MockBackend::new("plane"));
        let ids = runtime.backend_ids();
        assert!(ids.iter().any(|id| id == "plane"));
        assert!(!ids.iter().any(|id| id == "filesystem"));
        let context = BackendContext::new("env");
        let operation = env_fork_operation();
        let result = runtime
            .filesystem_operation(&context, &operation, &CancelHandle::default())
            .unwrap();
        assert!(result.confirmation.snapshot_created);
    }

    struct FailLifecycle {
        suspend: bool,
        resume: bool,
    }

    impl Backend for FailLifecycle {
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
        fn suspend(&self, _: &str) -> Result<(), BackendError> {
            if self.suspend {
                Err(BackendError::new("inner suspend failed"))
            } else {
                Ok(())
            }
        }
        fn resume(&self, _: &str) -> Result<(), BackendError> {
            if self.resume {
                Err(BackendError::new("inner resume failed"))
            } else {
                Ok(())
            }
        }
        fn terminate(&self, _: &str) -> Result<(), BackendError> { Ok(()) }
        fn cleanup(&self, _: &str) -> Result<(), BackendError> { Ok(()) }
        fn state(&self, _: &str) -> Option<EnvironmentState> {
            Some(EnvironmentState::Available)
        }
        fn environments(&self) -> Vec<String> { vec!["env".into()] }
    }

    #[test]
    fn suspend_rollback_thaw_failure_is_reported() {
        let runtime = BaseRuntime::new("runtime", Arc::new(FailLifecycle { suspend: true, resume: false }));
        if runtime.process.freeze_mechanism() == FreezeMechanism::None {
            eprintln!("SKIP suspend_rollback_thaw_failure_is_reported: no freeze mechanism");
            return;
        }
        let (context, _log_dir) = launch_test_agent(&runtime, "/bin/sleep", vec!["30".into()]);
        runtime.process.inject_thaw_failure(true);
        let error = runtime.suspend(context.environment_id.as_str()).unwrap_err();
        assert!(error.message.contains("inner suspend failed"), "{error}");
        assert!(
            error
                .cleanup_failures
                .iter()
                .any(|failure| failure.contains("thaw")),
            "{error:?}"
        );
        runtime.stop_agents().unwrap();
    }

    #[test]
    fn resume_rollback_freeze_failure_is_reported() {
        let runtime = BaseRuntime::new("runtime", Arc::new(FailLifecycle { suspend: false, resume: true }));
        if runtime.process.freeze_mechanism() == FreezeMechanism::None {
            eprintln!("SKIP resume_rollback_freeze_failure_is_reported: no freeze mechanism");
            return;
        }
        let (context, _log_dir) = launch_test_agent(&runtime, "/bin/sleep", vec!["30".into()]);
        runtime.process.inject_freeze_failure(true);
        let error = runtime.resume(context.environment_id.as_str()).unwrap_err();
        assert!(error.message.contains("inner resume failed"), "{error}");
        assert!(
            error
                .cleanup_failures
                .iter()
                .any(|failure| failure.contains("freeze")),
            "{error:?}"
        );
        runtime.stop_agents().unwrap();
    }

    #[test]
    fn launch_fragment_merge_rejects_duplicate_mount_targets() {
        let target_path = PathBuf::from("/workspace");
        let mut target = LaunchFragment {
            mounts: vec![MountEntry::Bind {
                source: PathBuf::from("/one"),
                target: target_path.clone(),
            }],
            ..LaunchFragment::default()
        };
        let source = LaunchFragment {
            mounts: vec![MountEntry::RoBind {
                source: PathBuf::from("/two"),
                target: target_path,
            }],
            ..LaunchFragment::default()
        };
        let error = super::selection::merge_launch_fragment(&mut target, source).unwrap_err();
        assert!(error.message.contains("multiple Backends"), "{error}");
        assert_eq!(target.mounts.len(), 1);
    }

    #[test]
    fn launch_fragment_merge_accepts_distinct_mount_targets() {
        let mut target = LaunchFragment {
            mounts: vec![MountEntry::Bind {
                source: PathBuf::from("/one"),
                target: PathBuf::from("/workspace"),
            }],
            ..LaunchFragment::default()
        };
        let source = LaunchFragment {
            mounts: vec![MountEntry::Tmpfs {
                target: PathBuf::from("/tmp"),
            }],
            ..LaunchFragment::default()
        };
        super::selection::merge_launch_fragment(&mut target, source).unwrap();
        assert_eq!(target.mounts.len(), 2);
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
