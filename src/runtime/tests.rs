//! Runtime 组合层与 BaseRuntime 生命周期测试。

use super::base::BaseRuntime;
use super::launch_plan::{LaunchFragment, MountEntry};
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

#[test]
fn cleanup_accepts_a_removed_participant_and_a_retained_cleaned_participant() {
    struct Lifecycle(Mutex<Option<EnvironmentState>>, bool);
    impl Backend for Lifecycle {
        fn establish(&self, _: &str) -> Result<(), BackendError> {
            Ok(())
        }
        fn operate(
            &self,
            _: &str,
            _: &[String],
            _: &CancelHandle,
        ) -> Result<CommandOutput, BackendError> {
            Err(BackendError::new("unused"))
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
            *self.0.lock().unwrap() = Some(EnvironmentState::Terminated);
            Ok(())
        }
        fn cleanup(&self, _: &str) -> Result<(), BackendError> {
            *self.0.lock().unwrap() = if self.1 {
                None
            } else {
                Some(EnvironmentState::Cleaned)
            };
            Ok(())
        }
        fn state(&self, _: &str) -> Option<EnvironmentState> {
            *self.0.lock().unwrap()
        }
        fn environments(&self) -> Vec<String> {
            vec!["env".into()]
        }
    }
    let inner = Arc::new(Lifecycle(
        Mutex::new(Some(EnvironmentState::Available)),
        false,
    ));
    let plane = Arc::new(Lifecycle(
        Mutex::new(Some(EnvironmentState::Available)),
        true,
    ));
    let runtime = BaseRuntime::with_policy(
        "runtime",
        vec![
            BackendConnection::new("inner", inner).unwrap(),
            BackendConnection::new("plane", plane.clone()).unwrap(),
        ],
        BackendPolicy {
            default: vec!["inner".into(), "plane".into()],
            ..Default::default()
        },
    )
    .unwrap();
    runtime.establish("env").unwrap();
    assert_eq!(runtime.state("env"), Some(EnvironmentState::Available));
    runtime.terminate("env").unwrap();
    assert_eq!(runtime.state("env"), Some(EnvironmentState::Terminated));
    runtime.cleanup("env").unwrap();
    assert_eq!(plane.state("env"), None);
    assert_eq!(runtime.state("env"), Some(EnvironmentState::Cleaned));
}

struct MockBackend {
    id: String,
    calls: AtomicUsize,
    cleanup_calls: AtomicUsize,
    seen: Mutex<Vec<(BackendContext, Operation)>>,
}

impl MockBackend {
    fn new(id: &str) -> Arc<Self> {
        Arc::new(Self {
            id: id.to_string(),
            calls: AtomicUsize::new(0),
            cleanup_calls: AtomicUsize::new(0),
            seen: Mutex::new(Vec::new()),
        })
    }
}

impl Backend for MockBackend {
    fn backend_id(&self) -> &str {
        &self.id
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
        Ok(CommandOutput {
            exit_code: 0,
            stdout: String::new(),
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
        self.cleanup_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn state(&self, _: &str) -> Option<EnvironmentState> {
        Some(EnvironmentState::Available)
    }
    fn environments(&self) -> Vec<String> {
        vec!["env".to_string()]
    }

    fn filesystem_operation(
        &self,
        context: &BackendContext,
        operation: &Operation,
        _: &CancelHandle,
    ) -> Result<FilesystemResult, BackendError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.seen
            .lock()
            .unwrap()
            .push((context.clone(), operation.clone()));
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
fn environment_participants_are_scoped_and_cleanup_does_not_touch_other_backend() {
    let fs1 = MockBackend::new("fs1");
    let process1 = MockBackend::new("process1");
    let runtime = BaseRuntime::with_policy(
        "runtime",
        vec![
            BackendConnection::new("fs1", fs1.clone()).unwrap(),
            BackendConnection::new("process1", process1.clone()).unwrap(),
        ],
        BackendPolicy {
            default: vec!["fs1".into(), "process1".into()],
            routes: vec![
                BackendRoute {
                    environment_id: Some("env2".into()),
                    operation: Some(BackendOperation::Establish),
                    backends: vec!["fs1".into()],
                    ..Default::default()
                },
                BackendRoute {
                    environment_id: Some("env2".into()),
                    operation: Some(BackendOperation::Cleanup),
                    backends: vec!["fs1".into()],
                    ..Default::default()
                },
            ],
        },
    )
    .unwrap();

    runtime.establish("env1").unwrap();
    runtime.establish("env2").unwrap();
    assert!(
        runtime
            .operate("env2", &[], &CancelHandle::default())
            .is_err()
    );
    assert_eq!(process1.calls.load(Ordering::SeqCst), 0);
    runtime.cleanup("env2").unwrap();

    assert_eq!(fs1.cleanup_calls.load(Ordering::SeqCst), 1);
    assert_eq!(process1.cleanup_calls.load(Ordering::SeqCst), 0);
    assert_eq!(runtime.backend_states("env2").len(), 1);
    assert_eq!(runtime.backend_states("env2")[0].backend_id, "fs1");
}

#[test]
fn filesystem_policy_selects_backend_and_forwards_context_and_operation() {
    let left = MockBackend::new("left");
    let right = MockBackend::new("right");
    let runtime = BaseRuntime::with_policy(
        "runtime",
        vec![
            BackendConnection::new("left", left.clone()).unwrap(),
            BackendConnection::new("right", right.clone()).unwrap(),
        ],
        BackendPolicy {
            default: vec!["right".into()],
            routes: vec![
                BackendRoute {
                    operation: Some(BackendOperation::Establish),
                    task_id: Some("task".into()),
                    backends: vec!["left".into()],
                    ..Default::default()
                },
                BackendRoute {
                    operation: Some(BackendOperation::FilesystemFork),
                    task_id: Some("task".into()),
                    backends: vec!["left".into()],
                    ..Default::default()
                },
            ],
        },
    )
    .unwrap();
    let context = BackendContext {
        task_id: Some("task".into()),
        environment_id: "env".into(),
        ..Default::default()
    };
    let cancel = CancelHandle::default();
    runtime.establish_with_context(&context).unwrap();
    runtime
        .filesystem_operation(&context, &fork_operation(), &cancel)
        .unwrap();
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
    let runtime = BaseRuntime::with_policy(
        "runtime",
        vec![
            BackendConnection::new("left", left.clone()).unwrap(),
            BackendConnection::new("right", right.clone()).unwrap(),
        ],
        BackendPolicy {
            default: vec!["left".into(), "right".into()],
            routes: vec![],
        },
    )
    .unwrap();
    let context = BackendContext::new("env");
    let cancel = CancelHandle::default();
    assert!(
        runtime
            .filesystem_operation(&context, &fork_operation(), &cancel)
            .is_err()
    );
    cancel.cancel();
    assert!(
        runtime
            .filesystem_operation(&context, &fork_operation(), &cancel)
            .is_err()
    );
    assert_eq!(left.calls.load(Ordering::SeqCst), 0);
    assert_eq!(right.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn trusted_host_does_not_become_an_environment_writer() {
    let root = tempfile::tempdir().unwrap();
    let runtime = BaseRuntime::new("runtime", MockBackend::new("mock"));
    let backend = super::agent_supervisor::AgentSupervisor::default();
    let context = BackendContext::new("trusted-host");
    runtime.establish_with_context(&context).unwrap();
    let launch = AgentLaunch {
        command: "/bin/sleep".into(),
        args: vec!["30".into()],
        cwd: root.path().into(),
        log: root.path().join("host.log"),
        instance_id: None,
        env: BTreeMap::new(),
    };
    backend.launch(&launch, &BTreeMap::new()).unwrap();
    runtime.materialize(&context).unwrap();
    // Reusing the run host after publication must succeed without restart.
    backend.launch(&launch, &BTreeMap::new()).unwrap();
    backend.stop().unwrap();
}

fn env_fork_operation() -> Operation {
    Operation::FilesystemFork {
        parent_snapshot_id: "env".into(),
        child_snapshot_id: "child".into(),
        path_scope: vec![],
    }
}

#[test]
fn filesystem_interface_routes_to_an_ordinary_registered_backend() {
    let runtime = BaseRuntime::with_policy(
        "runtime",
        vec![
            BackendConnection::new("inner", MockBackend::new("inner")).unwrap(),
            BackendConnection::new("plane", MockBackend::new("plane")).unwrap(),
        ],
        BackendPolicy {
            default: vec!["inner".into(), "plane".into()],
            routes: vec![BackendRoute {
                operation: Some(BackendOperation::FilesystemFork),
                backends: vec!["plane".into()],
                ..Default::default()
            }],
        },
    )
    .unwrap();
    runtime.establish("env").unwrap();
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
        fn establish(&self, _: &str) -> Result<(), BackendError> {
            Ok(())
        }
        fn operate(
            &self,
            _: &str,
            _: &[String],
            _: &CancelHandle,
        ) -> Result<CommandOutput, BackendError> {
            Err(BackendError::new("unused"))
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
            None
        }
        fn environments(&self) -> Vec<String> {
            Vec::new()
        }
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
    assert_eq!(
        result.unwrap_err().message,
        "filesystem operation is unsupported by this Backend"
    );
}

fn holder_request(socket: &std::path::Path, command: &str) -> serde_json::Value {
    use std::io::{BufRead, BufReader, Write};
    let mut stream = std::os::unix::net::UnixStream::connect(socket).unwrap();
    stream
        .write_all(
            (serde_json::json!({"op":"exec","command":command,"cwd":"/app"}).to_string() + "\n")
                .as_bytes(),
        )
        .unwrap();
    let mut reader = BufReader::new(stream);
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap() == 0 {
            panic!("holder closed without a result");
        }
        let event: serde_json::Value = serde_json::from_str(&line).unwrap();
        if event.get("exit_code").is_some() || event.get("error").is_some() {
            return event;
        }
    }
}

#[test]
fn materialization_restarts_the_namespace_holder_on_the_new_generation() {
    use crate::runtime::backends::filesystem::{FilesystemBackend, FilesystemConfig};
    let root = tempfile::Builder::new()
        .prefix("materialize-holder-")
        .tempdir_in("/tmp/opencode")
        .unwrap();
    let workspace = root.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let mut config = FilesystemConfig::new(root.path().join("state"));
    config.force_overlay = true;
    config.allow_overlapping_state_directory = true;
    config.isolated_paths = Some(vec![workspace.clone()]);
    config.isolated_targets = vec![(workspace.clone(), PathBuf::from("/app"))];
    let plane = Arc::new(FilesystemBackend::new(config).unwrap());
    let runtime = BaseRuntime::with_policy(
        "runtime",
        vec![
            BackendConnection::new("inner", MockBackend::new("inner")).unwrap(),
            BackendConnection::new("filesystem", plane.clone()).unwrap(),
        ],
        BackendPolicy {
            default: vec!["inner".into(), "filesystem".into()],
            ..Default::default()
        },
    )
    .unwrap();
    let context = BackendContext::new("authority");
    runtime.establish_with_context(&context).unwrap();
    let launch = AgentLaunch {
        command: "/bin/sleep".into(),
        args: vec!["30".into()],
        cwd: workspace.clone(),
        log: workspace.join("agent.log"),
        instance_id: Some("host".into()),
        env: BTreeMap::new(),
    };
    runtime
        .start_environment_execution(&context, &launch)
        .unwrap();
    let socket = root.path().join("state/holder-sockets/authority/cmd.sock");
    assert_eq!(
        holder_request(&socket, "printf before > /app/before")["exit_code"],
        0
    );
    assert!(!workspace.join("before").exists());

    let materialized = runtime.materialize(&context).unwrap();
    assert!(materialized.confirmation.published);
    assert_eq!(
        std::fs::read_to_string(workspace.join("before")).unwrap(),
        "before"
    );
    assert_eq!(
        holder_request(&socket, "test \"$(cat /app/before)\" = before")["exit_code"],
        0,
        "the restarted holder must mount the new execution generation"
    );
    runtime.terminate("authority").unwrap();
    runtime.cleanup("authority").unwrap();
}

#[cfg(test)]
mod shutdown_tests {
    use crate::protocol::contract::AgentLaunch;
    use crate::protocol::contract::{CommandOutput, EnvironmentState, RuntimeCapabilities};
    use crate::runtime::BaseRuntime;
    use crate::runtime::backend::BackendContext;
    use crate::runtime::backend::{Backend, BackendError, CancelHandle};
    use crate::runtime::proxy::RuntimeProxy;
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::{Duration, Instant};

    struct ShutdownBackend {
        environments: Mutex<Vec<String>>,
        start: Option<StartExecution>,
        terminated: Mutex<Vec<String>>,
        cleaned: Mutex<Vec<String>>,
        fail_terminate: Option<String>,
    }

    struct StartExecution {
        blocked: mpsc::Sender<()>,
        release: Mutex<mpsc::Receiver<()>>,
        retry_calls: std::sync::atomic::AtomicUsize,
    }

    impl Backend for ShutdownBackend {
        fn start_environment_execution(
            &self,
            context: &BackendContext,
            _: &AgentLaunch,
        ) -> Result<(), BackendError> {
            use std::sync::atomic::Ordering;
            let start = self.start.as_ref().unwrap();
            match context.environment_id.as_str() {
                "blocked" => {
                    start.blocked.send(()).unwrap();
                    start
                        .release
                        .lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(5))
                        .unwrap();
                }
                "retry" if start.retry_calls.fetch_add(1, Ordering::SeqCst) == 0 => {
                    return Err(BackendError::new("injected retryable startup failure"));
                }
                "retry" => {}
                id => panic!("unexpected environment start: {id}"),
            }
            self.environments
                .lock()
                .unwrap()
                .push(context.environment_id.clone());
            Ok(())
        }
        fn capabilities(&self) -> RuntimeCapabilities {
            RuntimeCapabilities {
                branch_environment: true,
                ..RuntimeCapabilities::default()
            }
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
            Err(BackendError::new("unused"))
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
            self.environments.lock().unwrap().clone()
        }
    }

    #[test]
    fn concurrent_launch_and_retry_finish_before_shutdown_reclaims_late_resources() {
        let (entered, blocked) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        let backend = Arc::new(ShutdownBackend {
            environments: Mutex::new(Vec::new()),
            start: Some(StartExecution {
                blocked: entered,
                release: Mutex::new(resume),
                retry_calls: Default::default(),
            }),
            terminated: Default::default(),
            cleaned: Default::default(),
            fail_terminate: None,
        });
        let runtime = BaseRuntime::new("test", backend.clone());
        runtime.establish("blocked").unwrap();
        runtime.establish("retry").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let launch = AgentLaunch {
            command: "/bin/sleep".into(),
            args: vec!["30".into()],
            cwd: dir.path().into(),
            log: dir.path().join("host.log"),
            instance_id: None,
            env: Default::default(),
        };
        let blocked_context = BackendContext::new("blocked");
        let retry_context = BackendContext::new("retry");
        std::thread::scope(|scope| {
            let launch_thread = scope.spawn(|| runtime.launch_agent(&blocked_context, &launch));
            blocked.recv_timeout(Duration::from_secs(5)).unwrap();

            // Reusing the host must not try to spawn this nonexistent executable.
            let retry_launch = AgentLaunch {
                command: dir.path().join("nonexistent").display().to_string(),
                ..launch.clone()
            };
            let failure = runtime
                .launch_agent(&retry_context, &retry_launch)
                .unwrap_err();
            assert!(
                failure
                    .message
                    .contains("injected retryable startup failure")
            );
            runtime.launch_agent(&retry_context, &retry_launch).unwrap();

            let shutdown = scope.spawn(|| runtime.shutdown());
            let deadline = Instant::now() + Duration::from_secs(5);
            while !runtime.is_closing() {
                assert!(Instant::now() < deadline);
                std::thread::yield_now();
            }
            let failure = runtime.launch_agent(&retry_context, &launch).unwrap_err();
            assert!(failure.message.contains("shutting down"));
            assert!(backend.cleaned.lock().unwrap().is_empty());
            release.send(()).unwrap();
            launch_thread.join().unwrap().unwrap();
            shutdown.join().unwrap().unwrap();
        });
        assert_eq!(*backend.cleaned.lock().unwrap(), ["retry", "blocked"]);
    }

    #[test]
    fn proxy_disconnect_terminates_and_cleans_every_environment() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let backend = Arc::new(ShutdownBackend {
            environments: Mutex::new(vec!["one".into(), "two".into(), "three".into()]),
            start: None,
            terminated: Mutex::new(Vec::new()),
            cleaned: Mutex::new(Vec::new()),
            fail_terminate: Some("two".into()),
        });
        let adapter_backend = backend.clone();
        let adapter = std::thread::spawn(move || {
            RuntimeProxy {
                adapter_instance: "test".into(),
                runtime: Arc::new(BaseRuntime::new("test", adapter_backend)),
            }
            .run(&address.to_string())
            .unwrap();
        });
        let (mut stream, _) = listener.accept().unwrap();
        // Read the hello so the runtime proxy is fully running before disconnect.
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
