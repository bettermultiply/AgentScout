//! cgroup / 进程组冻结、终止与清理。

use std::fs;
use std::path::PathBuf;

use super::*;
use super::cgroup::{sanitize, wait_frozen};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FreezeMechanism {
    Cgroup,
    ProcessGroup,
    None,
}

impl FreezeMechanism {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cgroup => "cgroup",
            Self::ProcessGroup => "process_group",
            Self::None => "none",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProcessHandle {
    pub inner_pid: u32,
    pub mount_namespace: u64,
    pub cgroup_path: Option<PathBuf>,
    pub process_group: i32,
}


impl ProcessBackend {
    pub fn freeze(&self, environment_id: &str) -> Result<(), BackendError> {
        #[cfg(test)]
        if self.fail_freeze.load(Ordering::SeqCst) {
            return Err(BackendError::new("injected freeze failure"));
        }
        let process = self.process(environment_id)?;
        match self.freeze {
            FreezeMechanism::Cgroup => {
                let path = process
                    .cgroup_path
                    .ok_or_else(|| BackendError::new("process has no cgroup path"))?;
                fs::write(path.join("cgroup.freeze"), "1").map_err(error)?;
                if let Err(value) = wait_frozen(&path) {
                    let _ = fs::write(path.join("cgroup.freeze"), "0");
                    return Err(value);
                }
                Ok(())
            }
            FreezeMechanism::ProcessGroup => {
                if unsafe { libc::kill(-process.process_group, libc::SIGSTOP) } != 0 {
                    return Err(error(std::io::Error::last_os_error()));
                }
                Ok(())
            }
            FreezeMechanism::None => Err(BackendError::new("no freeze mechanism is available")),
        }
    }

    pub fn thaw(&self, environment_id: &str) -> Result<(), BackendError> {
        #[cfg(test)]
        if self.fail_thaw.load(Ordering::SeqCst) {
            return Err(BackendError::new("injected thaw failure"));
        }
        let process = self.process(environment_id)?;
        match self.freeze {
            FreezeMechanism::Cgroup => {
                let path = process
                    .cgroup_path
                    .ok_or_else(|| BackendError::new("process has no cgroup path"))?;
                fs::write(path.join("cgroup.freeze"), "0").map_err(error)?;
                Ok(())
            }
            FreezeMechanism::ProcessGroup => {
                if unsafe { libc::kill(-process.process_group, libc::SIGCONT) } != 0 {
                    return Err(error(std::io::Error::last_os_error()));
                }
                Ok(())
            }
            FreezeMechanism::None => Err(BackendError::new("no freeze mechanism is available")),
        }
    }

    pub fn terminate(&self, environment_id: &str) -> Result<(), BackendError> {
        let process = match self.process(environment_id) {
            Ok(process) => process,
            Err(_) => return Ok(()),
        };
        if let Some(path) = &process.cgroup_path {
            let kill = path.join("cgroup.kill");
            if kill.exists() {
                fs::write(&kill, "1").map_err(error)?;
            } else if unsafe { libc::kill(-process.process_group, libc::SIGKILL) } != 0 {
                let value = std::io::Error::last_os_error();
                if value.raw_os_error() != Some(libc::ESRCH) {
                    return Err(error(value));
                }
            }
        } else if unsafe { libc::kill(-process.process_group, libc::SIGKILL) } != 0 {
            let value = std::io::Error::last_os_error();
            if value.raw_os_error() != Some(libc::ESRCH) {
                return Err(error(value));
            }
        }
        // A descendant that created its own session can escape the group; its
        // mount namespace is still the branch one and must not outlive the
        // environment. The duplicated namespace fd keeps this inode from being
        // reused while the sweep scans /proc.
        if process.namespace_fd.is_some() {
            super::kill_namespace_processes(process.mount_namespace)?;
        }
        Ok(())
    }

    pub fn stop(&self, environment_id: &str) -> Result<(), BackendError> {
        let process = self
            .processes
            .lock()
            .map_err(|_| BackendError::new("ProcessBackend lock poisoned"))?
            .remove(environment_id);
        let Some(mut process) = process else {
            return Ok(());
        };
        let mut failures = Vec::new();
        let mut killed = false;
        if let Some(path) = &process.cgroup_path {
            let kill = path.join("cgroup.kill");
            if kill.exists() {
                match fs::write(&kill, "1") {
                    Ok(()) => killed = true,
                    Err(failure) if failure.kind() == std::io::ErrorKind::NotFound => {}
                    Err(failure) => failures.push(format!(
                        "failed to write {}: {failure}",
                        kill.display()
                    )),
                }
            }
        }
        if !killed && unsafe { libc::kill(-process.process_group, libc::SIGKILL) } != 0 {
            let failure = std::io::Error::last_os_error();
            // 进程组已经不存在不是失败。
            if failure.raw_os_error() != Some(libc::ESRCH) {
                failures.push(format!("failed to kill process group: {failure}"));
            }
        }
        // The environment owns every process in this mount namespace, not only
        // the processes that stayed in the holder's group or cgroup. A
        // descendant that changed session must end with the Branch too. The
        // open namespace fd pins the inode for the whole sweep; without it a
        // freed inode could already belong to a new Branch.
        let swept = process.namespace_fd.as_ref().map(|_| {
            super::kill_namespace_processes(process.mount_namespace)
        });
        if let Err(failure) = process.child.wait() {
            failures.push(format!("failed to reap child process: {failure}"));
        }
        if let Some(Err(failure)) = swept {
            failures.push(format!("failed to sweep branch mount namespace: {failure}"));
        }
        if failures.is_empty() {
            Ok(())
        } else {
            // Keep ownership after failure so cleanup can retry the same
            // process/cgroup instead of losing a still-live resource.
            self.processes
                .lock()
                .map_err(|_| BackendError::new("ProcessBackend lock poisoned"))?
                .insert(environment_id.to_string(), process);
            Err(BackendError::new(failures.join("; ")))
        }
    }

    pub fn cleanup(&self, environment_id: &str) -> Result<(), BackendError> {
        self.stop(environment_id)?;
        if let Some(root) = &self.cgroup_root {
            let path = root.join(sanitize(environment_id));
            let mut last = None;
            for _ in 0..200 {
                match fs::remove_dir(&path) {
                    Ok(()) => return Ok(()),
                    Err(value) if value.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                    Err(value) => {
                        last = Some(value);
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                }
            }
            if let Some(value) = last {
                return Err(BackendError::new(format!(
                    "failed to remove cgroup {}: {value}",
                    path.display()
                )));
            }
        }
        Ok(())
    }

    pub fn stop_all(&self) {
        let mut owned = Vec::new();
        if let Ok(mut processes) = self.processes.lock() {
            for (_, process) in processes.drain() {
                let _ = unsafe { libc::kill(-process.process_group, libc::SIGKILL) };
                owned.push(process);
            }
        }
        // Sweep while every namespace fd is still open, so an inode freed by
        // one Branch cannot have been reused by another before the scan.
        let mut swept = std::collections::BTreeSet::new();
        for process in &owned {
            if process.namespace_fd.is_some() && swept.insert(process.mount_namespace) {
                let _ = super::kill_namespace_processes(process.mount_namespace);
            }
        }
        for mut process in owned {
            let _ = process.child.wait();
        }
    }

    fn process(&self, environment_id: &str) -> Result<AgentProcessRef, BackendError> {
        // The caller only needs pid/pgid/cgroup; clone them out so the map lock
        // is not held while filesystem operations run.
        let processes = self
            .processes
            .lock()
            .map_err(|_| BackendError::new("ProcessBackend lock poisoned"))?;
        let process = processes
            .get(environment_id)
            .ok_or_else(|| BackendError::new("unknown process environment"))?;
        Ok(AgentProcessRef {
            process_group: process.process_group,
            cgroup_path: process.cgroup_path.clone(),
            mount_namespace: process.mount_namespace,
            namespace_fd: process
                .namespace_fd
                .as_ref()
                .map(|fd| fd.try_clone().map_err(error))
                .transpose()?,
        })
    }
}

struct AgentProcessRef {
    process_group: libc::pid_t,
    cgroup_path: Option<PathBuf>,
    mount_namespace: u64,
    /// Keeps the branch namespace inode alive while terminate sweeps it.
    namespace_fd: Option<OwnedFd>,
}
