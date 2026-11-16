//! Process-group Runtime: the only component that spawns, freezes and kills
//! Agent processes.  It consumes `LaunchFragment`s produced by other Backends
//! and translates the combined `MountSpec` to bubblewrap arguments.

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::os::fd::{IntoRawFd, OwnedFd};
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::process::Child;
mod cgroup;
mod freeze;
mod launch;

pub use freeze::{FreezeMechanism, ProcessHandle};

use std::sync::Mutex;
#[cfg(test)]
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
#[cfg(test)]
use std::sync::atomic::Ordering;

use crate::protocol::contract::AgentLaunch;
#[cfg(test)]
use crate::protocol::contract::RuntimeCapabilities;
use crate::runtime::backend::{BackendContext, BackendError};
use crate::runtime::launch_plan::LaunchFragment;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProcessRole {
    /// A real Agent process; it owns the execution path and its writes.
    Agent,
    /// A branch namespace holder; it owns no Agent state and never writes the
    /// task workspace by itself.
    NamespaceHolder,
}

struct AgentProcess {
    instance_id: String,
    role: ProcessRole,
    child: Child,
    process_group: libc::pid_t,
    inner_pid: u32,
    mount_namespace: u64,
    /// Open while this process is owned. It pins the branch namespace inode so
    /// a later last-resort sweep cannot match an unrelated newer namespace that
    /// reused a stale inode number.
    namespace_fd: Option<OwnedFd>,
    cgroup_path: Option<PathBuf>,
}

static NEXT_PROCESS_ID: AtomicU64 = AtomicU64::new(1);

struct PendingStatus {
    read: OwnedFd,
    write: OwnedFd,
}

#[cfg(test)]
use crate::runtime::launch_plan::MountEntry;

pub struct ProcessBackend {
    bubblewrap_path: PathBuf,
    freeze: FreezeMechanism,
    cgroup_root: Option<PathBuf>,
    processes: Mutex<HashMap<String, AgentProcess>>,
    /// The Pi host process is owned by the run, not by a branch namespace.
    host_agent: Mutex<Option<Child>>,
    pending_status: Mutex<HashMap<String, PendingStatus>>,
    #[cfg(test)]
    fail_thaw: AtomicBool,
    #[cfg(test)]
    fail_freeze: AtomicBool,
    #[cfg(test)]
    fail_mount_wait: AtomicBool,
}

impl ProcessBackend {
    /// Temporarily stop all owned execution while filesystem state is copied.
    /// Thaw even when the copy fails so the user can continue or retry saving.
    pub(crate) fn with_frozen<T>(
        &self,
        operation: impl FnOnce() -> Result<T, BackendError>,
    ) -> Result<T, BackendError> {
        let ids: Vec<_> = self
            .processes
            .lock()
            .map_err(|_| BackendError::new("Process table poisoned"))?
            .keys()
            .cloned()
            .collect();
        let mut frozen: Vec<String> = Vec::new();
        for id in ids {
            let running = self.is_running(&id);
            if let Err(error) = running {
                for previous in &frozen {
                    let _ = self.thaw(previous);
                }
                return Err(error);
            }
            if !running.unwrap() {
                continue;
            }
            if let Err(error) = self.freeze(&id) {
                for previous in &frozen {
                    let _ = self.thaw(previous);
                }
                return Err(error);
            }
            frozen.push(id);
        }
        let result = operation();
        let mut thaw_error = None;
        for id in frozen {
            if let Err(error) = self.thaw(&id) {
                thaw_error = Some(error);
            }
        }
        if let Some(error) = thaw_error {
            return Err(BackendError::new(format!(
                "Recovery process thaw failed: {error}"
            )));
        }
        result
    }

    pub fn new(_id: impl Into<String>, bubblewrap_path: impl Into<PathBuf>) -> Self {
        let bubblewrap_path = bubblewrap_path.into();
        let cgroup_root = cgroup::probe_cgroup_root();
        let freeze = if cgroup_root.is_some() {
            FreezeMechanism::Cgroup
        } else {
            FreezeMechanism::ProcessGroup
        };
        eprintln!(
            "process: freeze_mechanism={} cgroup_root={:?}",
            freeze.as_str(),
            cgroup_root
        );
        Self::empty(bubblewrap_path, freeze, cgroup_root)
    }

    fn empty(
        bubblewrap_path: PathBuf,
        freeze: FreezeMechanism,
        cgroup_root: Option<PathBuf>,
    ) -> Self {
        Self {
            bubblewrap_path,
            freeze,
            cgroup_root,
            processes: Mutex::new(HashMap::new()),
            host_agent: Mutex::new(None),
            pending_status: Mutex::new(HashMap::new()),
            #[cfg(test)]
            fail_thaw: AtomicBool::new(false),
            #[cfg(test)]
            fail_freeze: AtomicBool::new(false),
            #[cfg(test)]
            fail_mount_wait: AtomicBool::new(false),
        }
    }

    #[cfg(test)]
    pub fn with_mechanism(
        _id: impl Into<String>,
        bubblewrap_path: impl Into<PathBuf>,
        freeze: FreezeMechanism,
    ) -> Self {
        Self::empty(
            bubblewrap_path.into(),
            freeze,
            if freeze == FreezeMechanism::Cgroup {
                cgroup::probe_cgroup_root()
            } else {
                None
            },
        )
    }

    #[cfg(test)]
    pub fn inject_thaw_failure(&self, value: bool) {
        self.fail_thaw.store(value, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub fn inject_freeze_failure(&self, value: bool) {
        self.fail_freeze.store(value, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub fn inject_mount_wait_failure(&self, value: bool) {
        self.fail_mount_wait.store(value, Ordering::SeqCst);
    }

    pub fn is_running(&self, environment_id: &str) -> Result<bool, BackendError> {
        let mut processes = self
            .processes
            .lock()
            .map_err(|_| BackendError::new("ProcessBackend lock poisoned"))?;
        let Some(process) = processes.get_mut(environment_id) else {
            return Ok(false);
        };
        match process.child.try_wait().map_err(error)? {
            Some(_) => Ok(false),
            None => Ok(true),
        }
    }

    /// Start the single Pi host process.  It intentionally receives no launch
    /// fragment: Pi owns sessions for all branches and must remain in the host
    /// namespace.
    pub fn launch_host(
        &self,
        launch: &AgentLaunch,
        env: &std::collections::BTreeMap<String, String>,
    ) -> Result<(), BackendError> {
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&launch.log)
            .map_err(error)?;
        // Pi runs on the host, so its own temporary files must stay on the
        // host. Tool commands receive their own namespace through the holder
        // and are not given this Pi-owned temp directory.
        let temporary = launch
            .log
            .parent()
            .map(|parent| parent.join("agent-tmp"))
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        fs::create_dir_all(&temporary).map_err(error)?;
        let mut environment = launch.env.clone();
        environment.extend(env.clone());
        environment
            .entry("TMPDIR".into())
            .or_insert_with(|| temporary.display().to_string());
        let mut host = self
            .host_agent
            .lock()
            .map_err(|_| BackendError::new("ProcessBackend lock poisoned"))?;
        if let Some(existing) = host.as_mut() {
            match existing.try_wait().map_err(error)? {
                None => return Ok(()),
                Some(status) => {
                    return Err(BackendError::new(format!(
                        "Pi host process exited before the run completed: {status}"
                    )));
                }
            }
        }
        let child = std::process::Command::new(&launch.command)
            .args(&launch.args)
            .current_dir(&launch.cwd)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::from(log.try_clone().map_err(error)?))
            .stderr(std::process::Stdio::from(log))
            .envs(&environment)
            .spawn()
            .map_err(error)?;
        *host = Some(child);
        Ok(())
    }

    /// Keep one inexpensive process alive in the branch mount namespace.  The
    /// process is deliberately independent from the Pi host process; its only
    /// ownership is the namespace and the branch environment lifetime.
    pub fn launch_holder(
        &self,
        context: &BackendContext,
        fragment: &LaunchFragment,
    ) -> Result<(), BackendError> {
        let launch = AgentLaunch {
            command: "python3".into(),
            args: vec![
                "-c".into(),
                include_str!("holder.py").into(),
                "/dev/ascout-holder/cmd.sock".into(),
            ],
            cwd: PathBuf::from("/"),
            log: PathBuf::from("/dev/null"),
            instance_id: Some(format!("namespace-holder:{}", context.environment_id)),
            env: std::collections::BTreeMap::new(),
        };
        self.launch_with_role(context, &launch, fragment, ProcessRole::NamespaceHolder)?;
        if let Some((directory, _)) = &fragment.holder_bind {
            // sockaddr_un limits the whole pathname, not the Environment id.
            // Resolve through an open directory without truncating identity.
            use std::os::fd::AsRawFd;
            let directory_fd = std::fs::File::open(directory).map_err(error)?;
            let socket = PathBuf::from(format!(
                "/proc/self/fd/{}/cmd.sock",
                directory_fd.as_raw_fd()
            ));
            for _ in 0..200 {
                if socket.exists() && std::os::unix::net::UnixStream::connect(&socket).is_ok() {
                    return Ok(());
                }
                if !self.is_running(&context.environment_id)? {
                    return Err(BackendError::new(
                        "namespace holder exited before accepting commands",
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            return Err(BackendError::new(format!(
                "namespace holder did not open command socket {}",
                socket.display()
            )));
        }
        Ok(())
    }

    /// Whether this environment is owned by a namespace holder instead of a
    /// real Agent process.
    pub fn is_holder(&self, environment_id: &str) -> Result<bool, BackendError> {
        let processes = self
            .processes
            .lock()
            .map_err(|_| BackendError::new("ProcessBackend lock poisoned"))?;
        Ok(processes
            .get(environment_id)
            .is_some_and(|process| process.role == ProcessRole::NamespaceHolder))
    }

    /// Whether a real Agent process is still running in this environment.  A
    /// holder keeps the namespace but is not a writer, so it must not block
    /// filesystem materialization.
    pub fn writer_active(&self, environment_id: &str) -> Result<bool, BackendError> {
        let mut processes = self
            .processes
            .lock()
            .map_err(|_| BackendError::new("ProcessBackend lock poisoned"))?;
        let Some(process) = processes.get_mut(environment_id) else {
            return Ok(false);
        };
        if process.role != ProcessRole::Agent {
            return Ok(false);
        }
        Ok(process.child.try_wait().map_err(error)?.is_none())
    }

    pub fn stop_host(&self) -> Result<(), BackendError> {
        let mut host = self
            .host_agent
            .lock()
            .map_err(|_| BackendError::new("ProcessBackend lock poisoned"))?;
        let Some(mut child) = host.take() else {
            return Ok(());
        };
        let _ = child.kill();
        child.wait().map(|_| ()).map_err(error)
    }

    #[cfg(test)]
    pub fn new_process_group(_id: impl Into<String>, bubblewrap_path: impl Into<PathBuf>) -> Self {
        Self::empty(bubblewrap_path.into(), FreezeMechanism::ProcessGroup, None)
    }

    pub fn freeze_mechanism(&self) -> FreezeMechanism {
        self.freeze
    }

    pub fn try_wait(
        &self,
        environment_id: &str,
    ) -> Result<Option<std::process::ExitStatus>, BackendError> {
        let mut processes = self
            .processes
            .lock()
            .map_err(|_| BackendError::new("ProcessBackend lock poisoned"))?;
        let process = processes
            .get_mut(environment_id)
            .ok_or_else(|| BackendError::new("unknown process environment"))?;
        process.child.try_wait().map_err(error)
    }

    pub fn wait(&self, environment_id: &str) -> Result<std::process::ExitStatus, BackendError> {
        let mut processes = self
            .processes
            .lock()
            .map_err(|_| BackendError::new("ProcessBackend lock poisoned"))?;
        let process = processes
            .get_mut(environment_id)
            .ok_or_else(|| BackendError::new("unknown process environment"))?;
        process.child.wait().map_err(error)
    }

    #[cfg(test)]
    pub fn capabilities(&self) -> RuntimeCapabilities {
        RuntimeCapabilities {
            freeze_mechanism: Some(self.freeze.as_str().to_string()),
            ..RuntimeCapabilities::default()
        }
    }

    pub fn process_handle(&self, environment_id: &str) -> Option<ProcessHandle> {
        let processes = self.processes.lock().ok()?;
        let process = processes.get(environment_id)?;
        Some(ProcessHandle {
            inner_pid: process.inner_pid,
            mount_namespace: process.mount_namespace,
            cgroup_path: process.cgroup_path.clone(),
            process_group: process.process_group,
        })
    }
}

/// Kill every process that still references the branch mount namespace.  The
/// holder and its descendants are normally reached by cgroup/process-group
/// termination; this is the last-resort sweep for a descendant that detached
/// from the group.  A process outside a private mount namespace is never
/// swept because that namespace belongs to the host.
fn kill_namespace_processes(namespace: u64) -> Result<(), BackendError> {
    let host = fs::metadata("/proc/self/ns/mnt").map_err(error)?.ino();
    if namespace == host {
        return Ok(());
    }
    let directory = match fs::read_dir("/proc") {
        Ok(directory) => directory,
        Err(failure) if failure.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(failure) => return Err(error(failure)),
    };
    let own = std::process::id();
    for entry in directory.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        if pid == own {
            continue;
        }
        let path = entry.path().join("ns/mnt");
        let Ok(metadata) = fs::metadata(&path) else {
            continue;
        };
        if metadata.ino() == namespace {
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGKILL);
            }
        }
    }
    Ok(())
}

impl Drop for ProcessBackend {
    fn drop(&mut self) {
        self.stop_all();
        if let Ok(mut host) = self.host_agent.lock() {
            if let Some(mut child) = host.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        if let Some(root) = &self.cgroup_root {
            let _ = fs::remove_dir_all(root);
        }
    }
}

fn error(value: impl std::fmt::Display) -> BackendError {
    BackendError::new(value.to_string())
}

fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|value| {
            if value.is_ascii_alphanumeric() || value == '-' || value == '_' {
                value
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::contract::AgentLaunch;
    use std::collections::BTreeMap;
    use std::path::Path;

    fn launch(
        backend: &ProcessBackend,
        command: &str,
        args: Vec<String>,
        cwd: &Path,
    ) -> BackendContext {
        let context = BackendContext::new("env");
        let launch = AgentLaunch {
            command: command.to_string(),
            args,
            cwd: cwd.to_path_buf(),
            log: cwd.join("process-test.log"),
            instance_id: Some("test-instance".into()),
            env: BTreeMap::new(),
        };
        backend
            .launch(&context, &launch, &LaunchFragment::default())
            .unwrap();
        context
    }

    /// Fixed marker consumed by scripts/priv-check.sh.  It reports the
    /// mechanism this ProcessBackend actually selected, not a guess.
    fn report_freeze(backend: &ProcessBackend, test: &str) {
        eprintln!(
            "ASCOUT_FREEZE_MECHANISM={} test={test}",
            backend.freeze_mechanism().as_str()
        );
    }

    #[test]
    fn terminate_uses_cgroup_or_group_kill_and_cleanup_removes_cgroup() {
        let root = tempfile::tempdir().unwrap();
        let backend = ProcessBackend::new("process", "bwrap");
        report_freeze(
            &backend,
            "terminate_uses_cgroup_or_group_kill_and_cleanup_removes_cgroup",
        );
        let context = launch(&backend, "/bin/sleep", vec!["30".into()], root.path());
        let handle = backend.process_handle(&context.environment_id).unwrap();
        assert!(backend.is_running(&context.environment_id).unwrap());
        backend.terminate(&context.environment_id).unwrap();
        backend.stop(&context.environment_id).unwrap();
        let cgroup = handle.cgroup_path.clone();
        backend.cleanup(&context.environment_id).unwrap();
        assert!(
            !Path::new(&format!("/proc/{}", handle.inner_pid)).exists(),
            "terminated process still exists"
        );
        if let Some(path) = cgroup {
            assert!(!path.exists(), "cleanup left cgroup {}", path.display());
        }
    }

    #[test]
    fn failed_launch_cleans_process_pending_status_and_cgroup() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let script = root.path().join("fake-bwrap");
        let pid_file = root.path().join("child.pid");
        fs::write(
            &script,
            format!(
                "#!/bin/bash\necho $$ > '{}'\nargs=(\"$@\")\nfd=\"\"\nfor ((i=0;i<${{#args[@]}};i++)); do\n  if [ \"${{args[i]}}\" = \"--json-status-fd\" ]; then fd=\"${{args[i+1]}}\"; fi\ndone\necho \"{{\\\"child-pid\\\":$$,\\\"mnt-namespace\\\":1}}\" >&\"$fd\"\nsleep 30\n",
                pid_file.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();

        let backend = ProcessBackend::new("process", &script);
        report_freeze(
            &backend,
            "failed_launch_cleans_process_pending_status_and_cgroup",
        );
        let cgroup = backend
            .cgroup_root
            .as_ref()
            .map(|root| root.join(sanitize("env")));
        backend.inject_mount_wait_failure(true);
        let context = BackendContext::new("env");
        let launch = AgentLaunch {
            command: "/bin/sleep".into(),
            args: vec!["30".into()],
            cwd: root.path().to_path_buf(),
            log: root.path().join("process-test.log"),
            instance_id: Some("test-instance".into()),
            env: BTreeMap::new(),
        };
        let fragment = LaunchFragment {
            mounts: vec![MountEntry::Overlay {
                lowers: Vec::new(),
                upper: root.path().join("upper"),
                work: root.path().join("work"),
                target: PathBuf::from("/workspace"),
            }],
            ..LaunchFragment::default()
        };
        let error = backend.launch(&context, &launch, &fragment).unwrap_err();
        assert!(error.message.contains("injected overlay"), "{error}");
        assert!(backend.processes.lock().unwrap().is_empty());
        assert!(backend.pending_status.lock().unwrap().is_empty());
        if let Some(path) = &cgroup {
            assert!(!path.exists(), "cgroup {} was not removed", path.display());
        }
        let pid: u32 = fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        for _ in 0..3000 {
            if !Path::new(&format!("/proc/{pid}")).exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            !Path::new(&format!("/proc/{pid}")).exists(),
            "launched child {pid} was not killed"
        );
    }

    #[test]
    fn stop_reports_cgroup_kill_write_failure() {
        let root = tempfile::tempdir().unwrap();
        let backend = ProcessBackend::new("process", "bwrap");
        report_freeze(&backend, "stop_reports_cgroup_kill_write_failure");
        let context = launch(&backend, "/bin/sleep", vec!["30".into()], root.path());
        let fake_cgroup = tempfile::tempdir().unwrap();
        // A directory named cgroup.kill makes fs::write fail instead of silently
        // succeeding, which reproduces the old "always Ok" behavior.
        fs::create_dir(fake_cgroup.path().join("cgroup.kill")).unwrap();
        {
            let mut processes = backend.processes.lock().unwrap();
            processes
                .get_mut(&context.environment_id)
                .unwrap()
                .cgroup_path = Some(fake_cgroup.path().to_path_buf());
        }
        let error = backend.stop(&context.environment_id).unwrap_err();
        assert!(error.message.contains("cgroup.kill"), "{error}");
        assert!(
            backend.process_handle(&context.environment_id).is_some(),
            "failed stop must retain the process handle for cleanup retry"
        );
        fs::remove_dir(fake_cgroup.path().join("cgroup.kill")).unwrap();
        backend.cleanup(&context.environment_id).unwrap();
        assert!(backend.process_handle(&context.environment_id).is_none());
    }

    #[test]
    fn stop_unknown_or_exited_process_is_not_an_error() {
        let root = tempfile::tempdir().unwrap();
        let backend = ProcessBackend::new("process", "bwrap");
        report_freeze(&backend, "stop_unknown_or_exited_process_is_not_an_error");
        backend.stop("missing").unwrap();

        let context = launch(&backend, "/bin/true", Vec::new(), root.path());
        for _ in 0..3000 {
            if !backend.is_running(&context.environment_id).unwrap() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        backend.stop(&context.environment_id).unwrap();
    }

    #[test]
    fn exited_process_is_reported_as_not_running() {
        let root = tempfile::tempdir().unwrap();
        let backend = ProcessBackend::new("process", "bwrap");
        report_freeze(&backend, "exited_process_is_reported_as_not_running");
        let context = launch(&backend, "/bin/true", Vec::new(), root.path());
        for _ in 0..3000 {
            if !backend.is_running(&context.environment_id).unwrap() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            backend.process_handle(&context.environment_id).is_some(),
            "the handle should remain observable after exit"
        );
        assert!(!backend.is_running(&context.environment_id).unwrap());
        backend.cleanup(&context.environment_id).unwrap();
    }

    #[test]
    fn launch_passes_agent_environment_to_child() {
        let root = tempfile::tempdir().unwrap();
        let backend = ProcessBackend::new("process", "bwrap");
        let context = BackendContext::new("env");
        let output = root.path().join("agent-env.txt");
        let launch = AgentLaunch {
            command: "/bin/sh".into(),
            args: vec![
                "-c".into(),
                "printf '%s' \"$PI_CODING_AGENT_DIR\" > \"$ASCOUT_ENV_OUTPUT\"".into(),
            ],
            cwd: root.path().to_path_buf(),
            log: root.path().join("process-test.log"),
            instance_id: Some("test-instance".into()),
            env: BTreeMap::from([(
                String::from("PI_CODING_AGENT_DIR"),
                String::from("/parent/pi"),
            )]),
        };
        let fragment = LaunchFragment {
            env: BTreeMap::from([(
                String::from("ASCOUT_ENV_OUTPUT"),
                output.display().to_string(),
            )]),
            ..LaunchFragment::default()
        };
        backend.launch(&context, &launch, &fragment).unwrap();
        for _ in 0..3000 {
            if output.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(fs::read_to_string(&output).unwrap(), "/parent/pi");
        backend.cleanup(&context.environment_id).unwrap();
    }
}
