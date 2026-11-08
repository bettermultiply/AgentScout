//! Process-group Runtime: the only component that spawns, freezes and kills
//! Agent processes.  It consumes `LaunchFragment`s produced by other Backends
//! and translates the combined `MountSpec` to bubblewrap arguments.

use std::collections::{BTreeMap, HashMap};
use std::fs::{self, OpenOptions};
use std::os::fd::{IntoRawFd, OwnedFd};
use std::path::PathBuf;
use std::process::Child;
mod cgroup;
mod freeze;
mod launch;

pub use freeze::{FreezeMechanism, ProcessHandle};
pub use launch::{LaunchFragment, MountEntry, MountSpec};

#[cfg(test)]
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
#[cfg(test)]
use std::sync::atomic::Ordering;
use std::sync::Mutex;

use crate::runtime::backend::{BackendError, BackendContext};
use crate::protocol::contract::{AgentLaunch, RuntimeCapabilities};

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
    id: String,
    bubblewrap_path: PathBuf,
    freeze: FreezeMechanism,
    cgroup_root: Option<PathBuf>,
    processes: Mutex<HashMap<String, AgentProcess>>,
    pending_status: Mutex<HashMap<String, PendingStatus>>,
    #[cfg(test)]
    fail_thaw: AtomicBool,
    #[cfg(test)]
    fail_freeze: AtomicBool,
}



impl ProcessBackend {
    pub fn new(id: impl Into<String>, bubblewrap_path: impl Into<PathBuf>) -> Self {
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
        Self {
            id: id.into(),
            bubblewrap_path,
            freeze,
            cgroup_root,
            processes: Mutex::new(HashMap::new()),
            pending_status: Mutex::new(HashMap::new()),
            #[cfg(test)]
            fail_thaw: AtomicBool::new(false),
            #[cfg(test)]
            fail_freeze: AtomicBool::new(false),
        }
    }

    pub fn with_mechanism(
        id: impl Into<String>,
        bubblewrap_path: impl Into<PathBuf>,
        freeze: FreezeMechanism,
    ) -> Self {
        Self {
            id: id.into(),
            bubblewrap_path: bubblewrap_path.into(),
            freeze,
            cgroup_root: if freeze == FreezeMechanism::Cgroup {
                cgroup::probe_cgroup_root()
            } else {
                None
            },
            processes: Mutex::new(HashMap::new()),
            pending_status: Mutex::new(HashMap::new()),
            #[cfg(test)]
            fail_thaw: AtomicBool::new(false),
            #[cfg(test)]
            fail_freeze: AtomicBool::new(false),
        }
    }

    #[cfg(test)]
    pub fn inject_thaw_failure(&self, value: bool) {
        self.fail_thaw.store(value, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub fn inject_freeze_failure(&self, value: bool) {
        self.fail_freeze.store(value, Ordering::SeqCst);
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

    pub fn new_process_group(id: impl Into<String>, bubblewrap_path: impl Into<PathBuf>) -> Self {
        Self {
            id: id.into(),
            bubblewrap_path: bubblewrap_path.into(),
            freeze: FreezeMechanism::ProcessGroup,
            cgroup_root: None,
            processes: Mutex::new(HashMap::new()),
            pending_status: Mutex::new(HashMap::new()),
            #[cfg(test)]
            fail_thaw: AtomicBool::new(false),
            #[cfg(test)]
            fail_freeze: AtomicBool::new(false),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
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
