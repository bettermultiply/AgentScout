
use std::fs;
use std::path::PathBuf;

use super::cgroup::{sanitize, wait_frozen};
use super::*;

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
}

impl Processes {
    pub fn freeze(&self, environment_id: &str) -> Result<(), BackendError> {
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

    #[cfg(test)]
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
            .map_err(|_| BackendError::new("Processes lock poisoned"))?
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
                    Err(failure) => {
                        failures.push(format!("failed to write {}: {failure}", kill.display()))
                    }
                }
            }
        }
        if !killed && unsafe { libc::kill(-process.process_group, libc::SIGKILL) } != 0 {
            let failure = std::io::Error::last_os_error();
            if failure.raw_os_error() != Some(libc::ESRCH) {
                failures.push(format!("failed to kill process group: {failure}"));
            }
        }
        // Sweep the mount namespace to include descendants outside the group and cgroup.
        // Keep the namespace fd open throughout so its inode cannot be reused.
        let swept = process
            .namespace_fd
            .as_ref()
            .map(|_| super::kill_namespace_processes(process.mount_namespace));
        if let Err(failure) = process.child.wait() {
            failures.push(format!("failed to reap child process: {failure}"));
        }
        if let Some(Err(failure)) = swept {
            failures.push(format!("failed to sweep branch mount namespace: {failure}"));
        }
        if failures.is_empty() {
            Ok(())
        } else {
            // Retain failed resources so cleanup can retry.
            self.processes
                .lock()
                .map_err(|_| BackendError::new("Processes lock poisoned"))?
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
        let processes = self
            .processes
            .lock()
            .map_err(|_| BackendError::new("Processes lock poisoned"))?;
        let process = processes
            .get(environment_id)
            .ok_or_else(|| BackendError::new("unknown process environment"))?;
        Ok(AgentProcessRef {
            process_group: process.process_group,
            cgroup_path: process.cgroup_path.clone(),
            #[cfg(test)]
            mount_namespace: process.mount_namespace,
            #[cfg(test)]
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
    #[cfg(test)]
    mount_namespace: u64,
    #[cfg(test)]
    namespace_fd: Option<OwnedFd>,
}
