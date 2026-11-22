
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::os::fd::OwnedFd;
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
use crate::runtime::backend::{BackendContext, BackendError};
use crate::runtime::launch_plan::LaunchFragment;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProcessRole {
    Agent,
    NamespaceHolder,
}

struct AgentProcess {
    instance_id: String,
    role: ProcessRole,
    child: Child,
    process_group: libc::pid_t,
    inner_pid: u32,
    mount_namespace: u64,
    /// Pins the inode until the last namespace sweep so it cannot match a reused namespace.
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

pub struct Processes {
    bubblewrap_path: PathBuf,
    freeze: FreezeMechanism,
    cgroup_root: Option<PathBuf>,
    processes: Mutex<HashMap<String, AgentProcess>>,
    pending_status: Mutex<HashMap<String, PendingStatus>>,
    #[cfg(test)]
    fail_mount_wait: AtomicBool,
}

impl Processes {
    pub(crate) fn with_frozen<T>(
        &self,
        should_freeze: impl Fn(&str) -> bool,
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
            if !should_freeze(&id) {
                continue;
            }
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
            pending_status: Mutex::new(HashMap::new()),
            #[cfg(test)]
            fail_mount_wait: AtomicBool::new(false),
        }
    }

    #[cfg(test)]
    pub fn inject_mount_wait_failure(&self, value: bool) {
        self.fail_mount_wait.store(value, Ordering::SeqCst);
    }

    pub fn is_running(&self, environment_id: &str) -> Result<bool, BackendError> {
        let mut processes = self
            .processes
            .lock()
            .map_err(|_| BackendError::new("Processes lock poisoned"))?;
        let Some(process) = processes.get_mut(environment_id) else {
            return Ok(false);
        };
        match process.child.try_wait().map_err(error)? {
            Some(_) => Ok(false),
            None => Ok(true),
        }
    }

    pub fn launch_holder(
        &self,
        context: &BackendContext,
        fragment: &LaunchFragment,
    ) -> Result<(), BackendError> {
        let launch = AgentLaunch {
            command: String::new(),
            args: Vec::new(),
            cwd: PathBuf::from("/"),
            log: PathBuf::from("/dev/null"),
            instance_id: Some(format!("namespace-holder:{}", context.environment_id)),
            env: std::collections::BTreeMap::new(),
        };
        self.launch_with_role(context, &launch, fragment, ProcessRole::NamespaceHolder)
    }

    /// A namespace holder alone is not a writer and must not block materialization.
    pub fn writer_active(&self, environment_id: &str) -> Result<bool, BackendError> {
        let mut processes = self
            .processes
            .lock()
            .map_err(|_| BackendError::new("Processes lock poisoned"))?;
        let Some(process) = processes.get_mut(environment_id) else {
            return Ok(false);
        };
        if process.role != ProcessRole::Agent {
            return Ok(false);
        }
        Ok(process.child.try_wait().map_err(error)?.is_none())
    }

    #[cfg(test)]
    pub fn new_process_group(_id: impl Into<String>, bubblewrap_path: impl Into<PathBuf>) -> Self {
        Self::empty(bubblewrap_path.into(), FreezeMechanism::ProcessGroup, None)
    }

    pub fn freeze_mechanism(&self) -> FreezeMechanism {
        self.freeze
    }

    #[cfg(test)]
    pub fn try_wait(&self, environment_id: &str) -> Result<Option<std::process::ExitStatus>, BackendError> {
        let mut processes = self.processes.lock().map_err(error)?;
        let process = processes.get_mut(environment_id)
            .ok_or_else(|| BackendError::new("unknown process environment"))?;
        process.child.try_wait().map_err(error)
    }

    #[cfg(test)]
    pub fn wait(&self, environment_id: &str) -> Result<std::process::ExitStatus, BackendError> {
        let mut processes = self
            .processes
            .lock()
            .map_err(|_| BackendError::new("Processes lock poisoned"))?;
        let process = processes
            .get_mut(environment_id)
            .ok_or_else(|| BackendError::new("unknown process environment"))?;
        process.child.wait().map_err(error)
    }

    pub fn process_handle(&self, environment_id: &str) -> Option<ProcessHandle> {
        let processes = self.processes.lock().ok()?;
        let process = processes.get(environment_id)?;
        Some(ProcessHandle {
            inner_pid: process.inner_pid,
            mount_namespace: process.mount_namespace,
        })
    }
}

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

impl Drop for Processes {
    fn drop(&mut self) {
        let ids: Vec<_> = self.processes.lock().map(|table| table.keys().cloned().collect()).unwrap_or_default();
        for id in ids {
            if let Err(error) = self.cleanup(&id) {
                eprintln!("process: failed to clean up {id}: {error}");
            }
        }
        self.stop_all();
        if let Some(root) = &self.cgroup_root {
            let _ = fs::remove_dir(root);
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
        backend: &Processes,
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

    /// `scripts/priv-check.sh` reads the mechanism actually selected here.
    fn report_freeze(backend: &Processes, test: &str) {
        eprintln!(
            "ASCOUT_FREEZE_MECHANISM={} test={test}",
            backend.freeze_mechanism().as_str()
        );
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

        let backend = Processes::new("process", &script);
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
        let backend = Processes::new("process", "bwrap");
        report_freeze(&backend, "stop_reports_cgroup_kill_write_failure");
        let context = launch(&backend, "/bin/sleep", vec!["30".into()], root.path());
        let fake_cgroup = tempfile::tempdir().unwrap();
        // Make the kill write fail reliably without a real cgroup.
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

}
