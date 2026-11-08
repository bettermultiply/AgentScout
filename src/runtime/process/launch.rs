//! bubblewrap LaunchFragment 翻译、spawn 与内层状态读取。

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use super::*;

#[derive(Debug, Clone, Default)]
pub struct LaunchFragment {
    pub mounts: Vec<MountEntry>,
    pub chdir: Option<PathBuf>,
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub enum MountEntry {
    RoBind { source: PathBuf, target: PathBuf },
    Bind { source: PathBuf, target: PathBuf },
    Overlay {
        /// Bottom to top.
        lowers: Vec<PathBuf>,
        upper: PathBuf,
        work: PathBuf,
        target: PathBuf,
    },
    Proc { target: PathBuf },
    Dev { target: PathBuf },
    Tmpfs { target: PathBuf },
    RemountRo { target: PathBuf },
    Dir { target: PathBuf },
}

#[derive(Debug, Clone, Default)]
pub struct MountSpec {
    pub entries: Vec<MountEntry>,
    pub chdir: Option<PathBuf>,
    pub json_status_fd: Option<libc::c_int>,
}

impl MountSpec {
    pub fn apply(&self, command: &mut Command) {
        command.arg("--die-with-parent").arg("--unshare-user");
        for entry in &self.entries {
            match entry {
                MountEntry::RoBind { source, target } => {
                    command.arg("--ro-bind").arg(source).arg(target);
                }
                MountEntry::Bind { source, target } => {
                    command.arg("--bind").arg(source).arg(target);
                }
                MountEntry::Overlay {
                    lowers,
                    upper,
                    work,
                    target,
                } => {
                    for lower in lowers {
                        command.arg("--overlay-src").arg(lower);
                    }
                    command.arg("--overlay").arg(upper).arg(work).arg(target);
                }
                MountEntry::Proc { target } => {
                    command.arg("--proc").arg(target);
                }
                MountEntry::Dev { target } => {
                    command.arg("--dev").arg(target);
                }
                MountEntry::Tmpfs { target } => {
                    command.arg("--tmpfs").arg(target);
                }
                MountEntry::RemountRo { target } => {
                    command.arg("--remount-ro").arg(target);
                }
                MountEntry::Dir { target } => {
                    command.arg("--dir").arg(target);
                }
            }
        }
        if let Some(chdir) = &self.chdir {
            command.arg("--chdir").arg(chdir);
        }
        if let Some(fd) = self.json_status_fd {
            command.arg("--json-status-fd").arg(fd.to_string());
        }
    }
}


impl ProcessBackend {
    pub fn launch(
        &self,
        context: &BackendContext,
        launch: &AgentLaunch,
        fragment: &LaunchFragment,
    ) -> Result<(), BackendError> {
        let instance_id = launch
            .instance_id
            .clone()
            .or_else(|| context.branch_id.clone())
            .unwrap_or_else(|| context.environment_id.clone());
        {
            let processes = self
                .processes
                .lock()
                .map_err(|_| BackendError::new("ProcessBackend lock poisoned"))?;
            if let Some(process) = processes.get(&context.environment_id) {
                if process.instance_id == instance_id {
                    return Ok(());
                }
                return Err(BackendError::new(format!(
                    "ProcessBackend already owns Agent instance {} for environment {}",
                    process.instance_id, context.environment_id
                )));
            }
        }
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&launch.log)
            .map_err(error)?;

        let cgroup_path = match &self.cgroup_root {
            Some(root) => {
                let path = root.join(sanitize(&context.environment_id));
                fs::create_dir_all(&path).map_err(error)?;
                Some(path)
            }
            None => None,
        };

        let use_bwrap = !fragment.mounts.is_empty();
        let mut command = if use_bwrap {
            let (read, write) = status_pipe()?;
            let status_fd = write.as_raw_fd();
            {
                let mut pending = self
                    .pending_status
                    .lock()
                    .map_err(|_| BackendError::new("ProcessBackend status lock poisoned"))?;
                pending.insert(
                    context.environment_id.clone(),
                    PendingStatus { read, write },
                );
            }
            let spec = MountSpec {
                entries: fragment.mounts.clone(),
                chdir: fragment.chdir.clone().or_else(|| Some(launch.cwd.clone())),
                json_status_fd: Some(status_fd),
            };
            let mut command = Command::new(&self.bubblewrap_path);
            spec.apply(&mut command);
            command.arg("--").arg(&launch.command).args(&launch.args);
            command
        } else {
            let mut command = Command::new(&launch.command);
            command.args(&launch.args).current_dir(&launch.cwd);
            command
        };

        let cgroup_procs_fd = cgroup_path
            .as_ref()
            .map(|path| {
                OpenOptions::new()
                    .write(true)
                    .open(path.join("cgroup.procs"))
                    .map(|file| file.into_raw_fd())
            })
            .transpose()
            .map_err(error)?;
        unsafe {
            command.pre_exec(move || {
                if libc::setpgid(0, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if let Some(fd) = cgroup_procs_fd {
                    let pid = libc::getpid().to_string();
                    let bytes = pid.as_bytes();
                    let written = libc::write(fd, bytes.as_ptr().cast(), bytes.len());
                    if written < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    libc::close(fd);
                }
                Ok(())
            });
        }
        let command = command
            .stdin(Stdio::null())
            .stdout(Stdio::from(log.try_clone().map_err(error)?))
            .stderr(Stdio::from(log))
            .envs(&launch.env)
            .envs(&fragment.env);
        let child = command.spawn().map_err(error)?;
        let process_group = child.id() as libc::pid_t;
        unsafe {
            if libc::setpgid(process_group, process_group) != 0
                && std::io::Error::last_os_error().raw_os_error() != Some(libc::EACCES)
            {
                let value = std::io::Error::last_os_error();
                let mut child = child;
                let _ = child.kill();
                let _ = child.wait();
                return Err(error(value));
            }
        }

        let (inner_pid, mount_namespace) = if use_bwrap {
            let (child_pid, namespace) = self.read_status(&context.environment_id)?;
            let overlay_targets: Vec<_> = fragment
                .mounts
                .iter()
                .filter_map(|entry| match entry {
                    MountEntry::Overlay { target, .. } => Some(target.clone()),
                    _ => None,
                })
                .collect();
            if !overlay_targets.is_empty() {
                wait_for_mounts(child_pid, &overlay_targets)?;
            }
            (child_pid, namespace)
        } else {
            let namespace = fs::metadata("/proc/self/ns/mnt").map_err(error)?.ino();
            (child.id(), namespace)
        };
        let process = AgentProcess {
            instance_id,
            child,
            process_group,
            inner_pid,
            mount_namespace,
            cgroup_path,
        };
        self.processes
            .lock()
            .map_err(|_| BackendError::new("ProcessBackend lock poisoned"))?
            .insert(context.environment_id.clone(), process);
        Ok(())
    }

    fn read_status(&self, environment_id: &str) -> Result<(u32, u64), BackendError> {
        let pending = {
            let mut pending = self
                .pending_status
                .lock()
                .map_err(|_| BackendError::new("ProcessBackend status lock poisoned"))?;
            pending.remove(environment_id)
        };
        let Some(PendingStatus { read, write }) = pending else {
            return Err(BackendError::new("missing bwrap status pipe"));
        };
        drop(write);
        let line = read_status_line(&read)?;
        let value: serde_json::Value = serde_json::from_str(line.trim())
            .map_err(|error| BackendError::new(format!("invalid bwrap status: {error}")))?;
        let child_pid = value
            .get("child-pid")
            .and_then(|value| value.as_u64())
            .ok_or_else(|| BackendError::new("bwrap status did not report child-pid"))?
            as u32;
        let namespace = value
            .get("mnt-namespace")
            .and_then(|value| value.as_u64())
            .ok_or_else(|| BackendError::new("bwrap status did not report mnt-namespace"))?;
        let host_namespace = fs::metadata("/proc/self/ns/mnt").map_err(error)?.ino();
        if namespace == host_namespace {
            return Err(BackendError::new(
                "bwrap child is still in the host mount namespace",
            ));
        }
        // Keep draining so bwrap does not SIGPIPE on later status updates.
        let reader = read
            .try_clone()
            .map_err(error)?;
        drop(read);
        std::thread::spawn(move || {
            let mut buffer = [0u8; 4096];
            loop {
                let read = unsafe {
                    libc::read(reader.as_raw_fd(), buffer.as_mut_ptr().cast(), buffer.len())
                };
                if read <= 0 {
                    break;
                }
            }
        });
        Ok((child_pid, namespace))
    }

}

fn status_pipe() -> Result<(OwnedFd, OwnedFd), BackendError> {
    let mut fds = [0 as libc::c_int; 2];
    if unsafe { libc::pipe2(fds.as_mut_ptr(), 0) } != 0 {
        return Err(error(std::io::Error::last_os_error()));
    }
    unsafe {
        libc::fcntl(fds[0], libc::F_SETFD, libc::FD_CLOEXEC);
    }
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

fn read_status_line(fd: &OwnedFd) -> Result<String, BackendError> {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    while line.len() < 8192 {
        let read = unsafe { libc::read(fd.as_raw_fd(), byte.as_mut_ptr().cast(), 1) };
        if read < 0 {
            let value = std::io::Error::last_os_error();
            if value.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error(value));
        }
        if read == 0 {
            break;
        }
        line.push(byte[0]);
        if byte[0] == b'\n' {
            break;
        }
    }
    Ok(String::from_utf8_lossy(&line).into_owned())
}

fn wait_for_mounts(pid: u32, targets: &[PathBuf]) -> Result<(), BackendError> {
    let path = format!("/proc/{pid}/mountinfo");
    for _ in 0..400 {
        match fs::read_to_string(&path) {
            Ok(text) => {
                let ready = text.lines().any(|line| {
                    let Some((left, right)) = line.split_once(" - ") else {
                        return false;
                    };
                    if !right.starts_with("overlay ") {
                        return false;
                    }
                    let fields: Vec<&str> = left.split_whitespace().collect();
                    fields
                        .get(4)
                        .map(|target| {
                            targets.iter().any(|expected| expected == &PathBuf::from(*target))
                        })
                        .unwrap_or(false)
                });
                if ready {
                    return Ok(());
                }
            }
            Err(_) => return Ok(()),
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    Err(BackendError::new(
        "sandbox did not finish mounting overlays before the readiness timeout",
    ))
}
