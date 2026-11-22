use super::*;
use crate::protocol::contract::AgentLaunch;
use crate::runtime::process::FreezeMechanism;
use serde_json::Value;
use std::io::{BufRead, Write};
use std::os::fd::AsRawFd;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;

type HolderResult = Result<(), BackendError>;

pub(super) struct HolderStart {
    ready: Mutex<Receiver<HolderResult>>,
    result: std::sync::OnceLock<HolderResult>,
    release: Option<Sender<()>>,
    worker: Option<JoinHandle<()>>,
}

impl HolderStart {
    pub(super) fn wait(&self) -> HolderResult {
        self.result.get_or_init(|| {
            self.ready.lock().map_err(error)?.recv()
                .map_err(|_| BackendError::new("holder startup worker exited without reporting readiness"))?
        }).clone()
    }
}

impl Drop for HolderStart {
    fn drop(&mut self) {
        self.release.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl FilesystemBackend {
    pub(super) fn wait_for_holder(&self, id: &str) -> HolderResult {
        let start = self.holder_starts.lock().map_err(error)?.get(id).cloned();
        start.map_or(Ok(()), |start| start.wait())
    }

    pub(super) fn start_child_holder(
        &self,
        context: &BackendContext,
        launch: &AgentLaunch,
    ) -> HolderResult {
        let id = &context.environment_id;
        let mut launches = self.launches.lock().map_err(error)?;
        let mut starts = self.holder_starts.lock().map_err(error)?;
        if starts.contains_key(id) || self.process.is_running(id)? {
            return Ok(());
        }
        let fragment = self.holder_fragment(context, launch)?;
        let process = Arc::clone(&self.process);
        let context = context.clone();
        let (sender, ready) = mpsc::channel();
        let (release, finished) = mpsc::channel();
        let worker = std::thread::Builder::new().name("ascout-holder".into()).spawn(move || {
            let result = Self::launch_holder(&process, &context, &fragment);
            let running = result.is_ok();
            let _ = sender.send(result);
            // The spawning thread's exit triggers the holder's parent-death signal.
            if running {
                let _ = finished.recv();
            }
        }).map_err(error)?;
        starts.insert(id.clone(), Arc::new(HolderStart {
            ready: Mutex::new(ready),
            result: Default::default(),
            release: Some(release),
            worker: Some(worker),
        }));
        launches.insert(id.clone(), launch.clone());
        Ok(())
    }

    fn launch_holder(
        process: &crate::runtime::process::Processes,
        context: &BackendContext,
        fragment: &LaunchFragment,
    ) -> HolderResult {
        if let Err(mut failure) = process.launch_holder(context, fragment) {
            if let Err(cleanup) = process.cleanup(&context.environment_id) {
                failure.add_cleanup_failure("filesystem", format!("stop failed holder: {cleanup}"));
            }
            return Err(failure);
        }
        Ok(())
    }
    pub(super) fn start_holder(
        &self,
        context: &BackendContext,
        launch: &AgentLaunch,
    ) -> Result<(), BackendError> {
        let id = &context.environment_id;
        let mut launches = self.launches.lock().map_err(error)?;
        let _ = self.wait_for_holder(id);
        if !self.process.is_running(id)? {
            self.process.stop(id)?;
            self.holder_starts.lock().map_err(error)?.remove(id);
            let fragment = self.holder_fragment(context, launch)?;
            Self::launch_holder(&self.process, context, &fragment)?;
        }
        launches.insert(id.clone(), launch.clone());
        Ok(())
    }

    pub(super) fn drain_holder(&self, id: &str, cancel: bool) -> Result<(), BackendError> {
        self.wait_for_holder(id)?;
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
        self.wait_for_holder(id)?;
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
        self.wait_for_holder(id)?;
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
        // Regenerating the plan would touch workdirs still mounted by the holder.
        let plan: Value = serde_json::from_slice(
            &fs::read(
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
        self.wait_for_holder(id)?;
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
