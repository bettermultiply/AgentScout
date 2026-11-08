//! Process-group Runtime: the only component that spawns, freezes and kills
//! Agent processes.  It consumes `LaunchFragment`s produced by other Backends
//! and translates the combined `MountSpec` to bubblewrap arguments.

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::os::fd::{IntoRawFd, OwnedFd};
use std::path::PathBuf;
use std::process::Child;
mod cgroup;
mod freeze;
mod launch;

pub use freeze::{FreezeMechanism, ProcessHandle};
pub use crate::runtime::launch_plan::{LaunchFragment, MountEntry, MountSpec};

#[cfg(test)]
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
#[cfg(test)]
use std::sync::atomic::Ordering;
use std::sync::Mutex;

use crate::runtime::backend::{BackendError, BackendContext};
use crate::protocol::contract::AgentLaunch;
#[cfg(test)]
use crate::protocol::contract::RuntimeCapabilities;

struct AgentProcess {
    instance_id: String,
    child: Child,
    process_group: libc::pid_t,
    inner_pid: u32,
    mount_namespace: u64,
    cgroup_path: Option<PathBuf>,
}

static NEXT_PROCESS_ID: AtomicU64 = AtomicU64::new(1);

struct PendingStatus {
    read: OwnedFd,
    write: OwnedFd,
}

pub struct ProcessBackend {
    bubblewrap_path: PathBuf,
    freeze: FreezeMechanism,
    cgroup_root: Option<PathBuf>,
    processes: Mutex<HashMap<String, AgentProcess>>,
    pending_status: Mutex<HashMap<String, PendingStatus>>,
    #[cfg(test)]
    fail_thaw: AtomicBool,
    #[cfg(test)]
    fail_freeze: AtomicBool,
    #[cfg(test)]
    fail_mount_wait: AtomicBool,
}



impl ProcessBackend {
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

    #[cfg(test)]
    pub fn new_process_group(_id: impl Into<String>, bubblewrap_path: impl Into<PathBuf>) -> Self {
        Self::empty(
            bubblewrap_path.into(),
            FreezeMechanism::ProcessGroup,
            None,
        )
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

    pub fn wait(
        &self,
        environment_id: &str,
    ) -> Result<std::process::ExitStatus, BackendError> {
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

impl Drop for ProcessBackend {
    fn drop(&mut self) {
        self.stop_all();
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
    use std::collections::BTreeMap;
    use std::path::Path;
    use crate::protocol::contract::AgentLaunch;

    fn launch(backend: &ProcessBackend, command: &str, args: Vec<String>, cwd: &Path) -> BackendContext {
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

    #[test]
    fn terminate_uses_cgroup_or_group_kill_and_cleanup_removes_cgroup() {
        let root = tempfile::tempdir().unwrap();
        let backend = ProcessBackend::new("process", "bwrap");
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
        for _ in 0..200 {
            if !Path::new(&format!("/proc/{pid}")).exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
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
    }

    #[test]
    fn stop_unknown_or_exited_process_is_not_an_error() {
        let root = tempfile::tempdir().unwrap();
        let backend = ProcessBackend::new("process", "bwrap");
        backend.stop("missing").unwrap();

        let context = launch(&backend, "/bin/true", Vec::new(), root.path());
        for _ in 0..200 {
            if !backend.is_running(&context.environment_id).unwrap() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        backend.stop(&context.environment_id).unwrap();
    }

    #[test]
    fn exited_process_is_reported_as_not_running() {
        let root = tempfile::tempdir().unwrap();
        let backend = ProcessBackend::new("process", "bwrap");
        let context = launch(&backend, "/bin/true", Vec::new(), root.path());
        for _ in 0..200 {
            if !backend.is_running(&context.environment_id).unwrap() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(
            backend.process_handle(&context.environment_id).is_some(),
            "the handle should remain observable after exit"
        );
        assert!(!backend.is_running(&context.environment_id).unwrap());
        backend.cleanup(&context.environment_id).unwrap();
    }
}
