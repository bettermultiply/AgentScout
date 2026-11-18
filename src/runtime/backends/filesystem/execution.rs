use super::*;
use crate::protocol::contract::AgentLaunch;
use crate::runtime::process::FreezeMechanism;
use serde_json::Value;
use std::io::{BufRead, Write};
use std::os::fd::AsRawFd;

impl FilesystemBackend {
    pub(super) fn start_holder(
        &self,
        context: &BackendContext,
        launch: &AgentLaunch,
    ) -> Result<(), BackendError> {
        let id = &context.environment_id;
        let mut launches = self.launches.lock().map_err(error)?;
        if !self.process.is_running(id)? {
            self.process.stop(id)?;
            let fragment = self.launch_fragment(context, launch)?;
            if let Err(mut failure) = self.process.launch_holder(context, &fragment) {
                if let Err(cleanup) = self.process.cleanup(id) {
                    failure.add_cleanup_failure(
                        "filesystem",
                        format!("stop failed holder: {cleanup}"),
                    );
                }
                return Err(failure);
            }
        }
        launches.insert(id.clone(), launch.clone());
        Ok(())
    }

    pub(super) fn drain_holder(&self, id: &str, cancel: bool) -> Result<(), BackendError> {
        if !self.process.is_running(id)? {
            return Ok(());
        }
        let directory = fs::File::open(self.config.state_directory.join("holder-sockets").join(id))
            .map_err(error)?;
        let mut socket = std::os::unix::net::UnixStream::connect(format!(
            "/proc/self/fd/{}/cmd.sock",
            directory.as_raw_fd()
        ))
        .map_err(error)?;
        let request = if cancel {
            "{\"op\":\"drain\",\"cancel\":true}\n"
        } else {
            "{\"op\":\"drain\"}\n"
        };
        socket.write_all(request.as_bytes()).map_err(error)?;
        let mut reply = String::new();
        std::io::BufReader::new(socket)
            .read_line(&mut reply)
            .map_err(error)?;
        let reply: Value = serde_json::from_str(&reply).map_err(error)?;
        if reply["exit_code"] != 0 {
            return Err(BackendError::new(format!(
                "holder writers did not drain: {reply}"
            )));
        }
        Ok(())
    }

    pub(super) fn materialize_with_execution(
        &self,
        context: &BackendContext,
        previous: &mut dyn FnMut() -> Result<FilesystemResult, BackendError>,
    ) -> Result<FilesystemResult, BackendError> {
        let id = &context.environment_id;
        if self.process.writer_active(id)? {
            return Err(BackendError::new(
                "cannot materialize while Runtime-managed execution is active; task writers have not been confirmed drained",
            ));
        }
        let launch = self.launches.lock().map_err(error)?.get(id).cloned();
        if launch.is_some() {
            if self.state(id) != Some(EnvironmentState::Available) {
                return Err(BackendError::new(
                    "filesystem environment is not available for materialization",
                ));
            }
            self.drain_holder(id, false)?;
            self.process.stop(id)?;
        }
        let result = (|| {
            let mut result = previous()?;
            let publication = self.materialize_authority(id)?;
            result.confirmation.published |= publication.confirmation.published;
            result.conflicts.extend(publication.conflicts);
            Ok(result)
        })();
        match (
            result,
            launch.map_or(Ok(()), |launch| self.start_holder(context, &launch)),
        ) {
            (Err(mut original), Err(restart)) => {
                original.add_cleanup_failure(
                    "filesystem",
                    format!("restart namespace holder: {restart}"),
                );
                Err(original)
            }
            (Err(error), _) | (Ok(_), Err(error)) => Err(error),
            (Ok(result), Ok(())) => Ok(result),
        }
    }

    pub(super) fn transition_execution(
        &self,
        id: &str,
        paused: bool,
        transition: impl FnOnce() -> Result<(), BackendError>,
    ) -> Result<(), BackendError> {
        if !self.process.is_running(id)? {
            return transition();
        }
        if paused {
            self.process.freeze(id)?;
        } else {
            self.process.thaw(id)?;
        }
        if let Err(mut error) = transition() {
            let rollback = if paused {
                self.process.thaw(id)
            } else {
                self.process.freeze(id)
            };
            if let Err(failure) = rollback {
                error.add_cleanup_failure(
                    "filesystem",
                    format!("restore execution state: {failure}"),
                );
            }
            return Err(error);
        }
        Ok(())
    }

    pub(super) fn execute_environment_operation(
        &self,
        context: &BackendContext,
        operation: &str,
        arguments: &crate::runtime::execution_wire::Message,
        cancel: &CancelHandle,
    ) -> Result<crate::runtime::execution_wire::Message, BackendError> {
        use std::os::fd::AsRawFd;
        // launch_fragment prepares mount work directories and must never be
        // regenerated while the holder has those overlays mounted.
        let plan: Value = serde_json::from_reader(
            fs::File::open(
                self.config
                    .state_directory
                    .join("tool-plans")
                    .join(format!("{}.json", context.environment_id)),
            )
            .map_err(error)?,
        )
        .map_err(error)?;
        let holder = std::path::Path::new(
            plan["holder_socket"]
                .as_str()
                .ok_or_else(|| BackendError::new("namespace holder missing"))?,
        );
        let directory = holder
            .parent()
            .ok_or_else(|| BackendError::new("holder directory missing"))?;
        let directory =
            std::fs::File::open(directory).map_err(|e| BackendError::new(e.to_string()))?;
        let mut socket = std::os::unix::net::UnixStream::connect(format!(
            "/proc/self/fd/{}/cmd.sock",
            directory.as_raw_fd()
        ))
        .map_err(|e| BackendError::new(e.to_string()))?;
        let mut request = arguments.meta.clone();
        // Runtime alone reads its backend's private mount mapping. Integration
        // sends ordinary absolute paths, never backend-internal plan paths.
        {
            if let (Some(host), Some(target)) = (
                plan["workspace"]["host"].as_str(),
                plan["workspace"]["target"].as_str(),
            ) {
                for field in ["path", "cwd"] {
                    if let Some(path) = request[field].as_str() {
                        if let Ok(relative) = std::path::Path::new(path).strip_prefix(host) {
                            request[field] = Value::String(
                                std::path::Path::new(target)
                                    .join(relative)
                                    .display()
                                    .to_string(),
                            );
                        }
                    }
                }
            }
        }
        request["op"] = Value::String(operation.into());
        request["bound_cwd"] = plan["workspace"]["target"].clone();
        if operation == "exec" && request["cwd"].is_null() {
            request["cwd"] = request["bound_cwd"].clone();
        }
        let io_error = |error: std::io::Error| BackendError::new(error.to_string());
        socket
            .set_write_timeout(Some(std::time::Duration::from_secs(30)))
            .map_err(io_error)?;
        crate::runtime::execution_wire::Message {
            meta: request,
            body: arguments.body.clone(),
        }
        .write(&mut socket)
        .map_err(io_error)?;
        socket
            .set_read_timeout(Some(std::time::Duration::from_millis(50)))
            .map_err(io_error)?;
        crate::runtime::execution_wire::operation_result(&socket, &socket, cancel)
    }
    pub(super) fn with_filesystem_execution(
        &self,
        context: &BackendContext,
        operation: &Operation,
        apply: impl FnOnce(&BackendContext) -> Result<FilesystemResult, BackendError>,
    ) -> Result<FilesystemResult, BackendError> {
        let mut context = context.clone();
        let (target, accepting) = match operation {
            Operation::FilesystemFork {
                parent_snapshot_id, ..
            } => (parent_snapshot_id, false),
            Operation::FilesystemMark { snapshot_id, .. } => (snapshot_id, false),
            Operation::FilesystemAccept {
                target_snapshot_id, ..
            } => (target_snapshot_id, true),
            _ => return apply(&context),
        };
        let Some(environment) = ({
            let state = self.environment()?;
            Self::resolve_environment(&state, target).ok()
        }) else {
            return apply(&context);
        };
        let id = &environment.id;
        let running = self.process.is_running(id)?;
        let paused = running && environment.state == EnvironmentState::Suspended;
        let frozen = running && !paused && self.process.freeze_mechanism() != FreezeMechanism::None;
        if frozen {
            self.process.freeze(id)?;
        }
        if accepting
            && (frozen || paused)
            && let Some(handle) = self.process.process_handle(id)
        {
            context.target_mount_pid = Some(handle.inner_pid);
            context.target_mount_namespace = Some(handle.mount_namespace);
        }
        let result = apply(&context);
        let thaw = if frozen {
            self.process.thaw(id).err()
        } else {
            None
        };
        match result {
            Ok(mut result) => {
                if (frozen || paused) && thaw.is_none() {
                    result.confirmation.writers_quiesced = true;
                    if accepting {
                        result.confirmation.target_paused = true;
                    }
                }
                if let Some(error) = thaw {
                    let mut error = BackendError::new(format!(
                        "filesystem operation succeeded but process thaw failed: {error}"
                    ));
                    error.add_cleanup_failure("filesystem", "thaw after filesystem operation");
                    return Err(error);
                }
                Ok(result)
            }
            Err(mut error) => {
                if let Some(thaw) = thaw {
                    error.add_cleanup_failure("filesystem", format!("thaw: {thaw}"));
                }
                Err(error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn backend() -> (FilesystemBackend, tempfile::TempDir) {
        let root = tempfile::tempdir().unwrap();
        let mut backend = FilesystemBackend::new(FilesystemConfig::new(root.path())).unwrap();
        // These fixtures exercise real process groups and metadata, without mounting an overlay.
        backend.usable = true;
        *backend.overlay_targets.lock().unwrap() = Some(Vec::new());
        backend.establish("env").unwrap();
        (backend, root)
    }

    #[test]
    fn filesystem_freezes_and_marks_writers_quiesced() {
        let (runtime, _root) = backend();
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
        runtime
            .process
            .launch(&context, &launch, &Default::default())
            .unwrap();
        let operation = Operation::FilesystemFork {
            parent_snapshot_id: "env".into(),
            child_snapshot_id: "child".into(),
            path_scope: vec![],
        };
        let result = runtime
            .with_filesystem_execution(&context, &operation, |_| Ok(FilesystemResult::default()))
            .unwrap();
        assert!(
            result.confirmation.writers_quiesced,
            "freeze must be reflected in writers_quiesced"
        );
        runtime.process.stop_all();

        let result = runtime
            .with_filesystem_execution(
                &BackendContext::new("missing"),
                &Operation::FilesystemFork {
                    parent_snapshot_id: "missing".into(),
                    child_snapshot_id: "child".into(),
                    path_scope: vec![],
                },
                |_| Ok(FilesystemResult::default()),
            )
            .unwrap();
        assert!(!result.confirmation.writers_quiesced);
    }

    #[test]
    fn materialization_refuses_active_managed_writer_without_stopping_it() {
        let root = tempfile::Builder::new()
            .prefix("materialize-writer-")
            .tempdir()
            .unwrap();
        let (runtime, _root) = backend();
        let context = BackendContext::new("materialize-active");
        let launch = AgentLaunch {
            command: "/bin/sleep".into(),
            args: vec!["30".into()],
            cwd: root.path().into(),
            log: root.path().join("agent.log"),
            instance_id: None,
            env: BTreeMap::new(),
        };
        runtime
            .process
            .launch(&context, &launch, &Default::default())
            .unwrap();
        let failure = runtime.materialize(&context).unwrap_err();
        assert!(
            failure
                .to_string()
                .contains("writers have not been confirmed drained")
        );
        assert!(runtime.process.is_running(&context.environment_id).unwrap());
        runtime.process.cleanup(&context.environment_id).unwrap();
    }

    fn launch_test_agent(
        runtime: &FilesystemBackend,
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
        // This fixture represents an environment writer, not the trusted Agent host.
        runtime
            .process
            .launch(&context, &launch, &Default::default())
            .unwrap();
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
    fn filesystem_without_freeze_mechanism_still_runs_operations() {
        let (mut runtime, _root) = backend();
        runtime.process = crate::runtime::process::Processes::with_mechanism(
            "process",
            "bwrap",
            FreezeMechanism::None,
        );
        let (context, _log_dir) = launch_test_agent(&runtime, "/bin/sleep", vec!["3".into()]);
        let result = runtime
            .with_filesystem_execution(&context, &env_fork_operation(), |_| {
                Ok(FilesystemResult::default())
            })
            .unwrap();
        assert!(!result.confirmation.writers_quiesced);
        runtime.process.stop_all();
    }

    #[test]
    fn freeze_failure_fails_before_filesystem_operation() {
        let (runtime, _root) = backend();
        let (context, _log_dir) = launch_test_agent(&runtime, "/bin/sleep", vec!["3".into()]);
        runtime.process.inject_freeze_failure(true);
        let error = runtime
            .with_filesystem_execution(&context, &env_fork_operation(), |_| {
                Ok(FilesystemResult::default())
            })
            .unwrap_err();
        assert!(
            error.message.contains("injected freeze failure"),
            "{}",
            error.message
        );
        runtime.process.inject_freeze_failure(false);
        runtime.process.stop_all();
    }

    #[test]
    fn thaw_failure_is_reported_as_cleanup_failure() {
        let (runtime, _root) = backend();
        let (context, _log_dir) = launch_test_agent(&runtime, "/bin/sleep", vec!["3".into()]);
        runtime.process.inject_thaw_failure(true);
        let error = runtime
            .with_filesystem_execution(&context, &env_fork_operation(), |_| {
                Ok(FilesystemResult::default())
            })
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
        runtime.process.stop_all();
    }

    #[test]
    fn exited_process_does_not_block_filesystem_operation() {
        let (runtime, _root) = backend();
        let (context, _log_dir) = launch_test_agent(&runtime, "/bin/true", Vec::new());
        for _ in 0..3000 {
            if !runtime.process.is_running("env").unwrap() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!runtime.process.is_running("env").unwrap());
        assert!(runtime.process.process_handle("env").is_some());
        let result = runtime
            .with_filesystem_execution(&context, &env_fork_operation(), |_| {
                Ok(FilesystemResult::default())
            })
            .unwrap();
        assert!(!result.confirmation.writers_quiesced);
        runtime.process.stop_all();
    }

    #[test]
    fn suspend_rollback_thaw_failure_is_reported() {
        let (runtime, _root) = backend();
        let (context, _log_dir) = launch_test_agent(&runtime, "/bin/sleep", vec!["3".into()]);
        runtime.process.inject_thaw_failure(true);
        let error = runtime
            .suspend_after(&context, &mut || {
                Err(BackendError::new("inner suspend failed"))
            })
            .unwrap_err();
        assert_eq!(error.message, "inner suspend failed");
        assert!(!error.cleanup_failures.is_empty());
    }

    #[test]
    fn resume_rollback_freeze_failure_is_reported() {
        let (runtime, _root) = backend();
        let (context, _log_dir) = launch_test_agent(&runtime, "/bin/sleep", vec!["3".into()]);
        runtime.suspend(&context.environment_id).unwrap();
        runtime.process.inject_freeze_failure(true);
        let error = runtime
            .resume_after(&context, &mut || {
                Err(BackendError::new("inner resume failed"))
            })
            .unwrap_err();
        assert_eq!(error.message, "inner resume failed");
        assert!(!error.cleanup_failures.is_empty());
    }
}
