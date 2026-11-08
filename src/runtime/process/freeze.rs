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
        if let Err(failure) = process.child.wait() {
            failures.push(format!("failed to reap child process: {failure}"));
        }
        if failures.is_empty() {
            Ok(())
        } else {
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
        if let Ok(mut processes) = self.processes.lock() {
            for (_, mut process) in processes.drain() {
                let _ = unsafe { libc::kill(-process.process_group, libc::SIGKILL) };
                let _ = process.child.wait();
            }
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
        })
    }
}

struct AgentProcessRef {
    process_group: libc::pid_t,
    cgroup_path: Option<PathBuf>,
}
