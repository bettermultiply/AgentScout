//! Process-group Runtime: the only component that spawns, freezes and kills
//! Agent processes.  It consumes `LaunchFragment`s produced by other Backends
//! and translates the combined `MountSpec` to bubblewrap arguments.

use std::collections::{BTreeMap, HashMap};
use std::fs::{self, OpenOptions};
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd};
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
#[cfg(test)]
use std::sync::atomic::AtomicBool;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use crate::backend::{BackendError, BackendContext};
use crate::contract::{AgentLaunch, RuntimeCapabilities};

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
        let cgroup_root = probe_cgroup_root();
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
                probe_cgroup_root()
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
        if let Some(path) = &process.cgroup_path {
            let kill = path.join("cgroup.kill");
            if kill.exists() {
                let _ = fs::write(&kill, "1");
            } else {
                let _ = unsafe { libc::kill(-process.process_group, libc::SIGKILL) };
            }
        } else {
            let _ = unsafe { libc::kill(-process.process_group, libc::SIGKILL) };
        }
        let _ = process.child.wait();
        Ok(())
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

fn probe_cgroup_root() -> Option<PathBuf> {
    let text = fs::read_to_string("/proc/self/cgroup").ok()?;
    let path = text
        .lines()
        .find_map(|line| line.strip_prefix("0::"))?
        .trim();
    let root = PathBuf::from("/sys/fs/cgroup").join(path.trim_start_matches('/'));
    let candidate = root.join(format!(
        "ascout-process-{}-{}",
        std::process::id(),
        NEXT_PROCESS_ID.fetch_add(1, Ordering::SeqCst)
    ));
    fs::create_dir_all(&candidate).ok()?;
    if !candidate.join("cgroup.freeze").exists() {
        let _ = fs::remove_dir(&candidate);
        return None;
    }
    Some(candidate)
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
                    .map(|target| targets.iter().any(|expected| expected == &PathBuf::from(*target)))
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

fn wait_frozen(path: &Path) -> Result<(), BackendError> {
    for _ in 0..200 {
        if let Ok(events) = fs::read_to_string(path.join("cgroup.events"))
            && events.lines().any(|line| line.trim() == "frozen 1")
        {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    Err(BackendError::new("cgroup did not report frozen=1"))
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::AgentLaunch;

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
        let root = tempfile::tempdir_in("/tmp/opencode").unwrap();
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
        let _ = std::fs::remove_file(root.path().join("process-test.log"));
    }

    #[test]
    fn exited_process_is_reported_as_not_running() {
        let root = tempfile::tempdir_in("/tmp/opencode").unwrap();
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
        let _ = std::fs::remove_file(root.path().join("process-test.log"));
    }
}
