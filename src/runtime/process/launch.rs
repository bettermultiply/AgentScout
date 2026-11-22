//! bubblewrap LaunchFragment 翻译、spawn 与内层状态读取。

use std::io::{BufRead, BufReader, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use super::*;
use crate::runtime::launch_plan::*;

const MOUNT_INFO_ATTEMPTS: usize = 400;
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(5);

impl Processes {
    #[cfg(test)]
    pub fn launch(
        &self,
        context: &BackendContext,
        launch: &AgentLaunch,
        fragment: &LaunchFragment,
    ) -> Result<(), BackendError> {
        self.launch_with_role(context, launch, fragment, ProcessRole::Agent)
    }

    pub(super) fn launch_with_role(
        &self,
        context: &BackendContext,
        launch: &AgentLaunch,
        fragment: &LaunchFragment,
        role: ProcessRole,
    ) -> Result<(), BackendError> {
        let instance_id = launch
            .instance_id
            .clone()
            .unwrap_or_else(|| context.environment_id.clone());
        {
            let processes = self
                .processes
                .lock()
                .map_err(|_| BackendError::new("Processes lock poisoned"))?;
            if let Some(process) = processes.get(&context.environment_id) {
                if process.instance_id == instance_id {
                    return Ok(());
                }
                return Err(BackendError::new(format!(
                    "Processes already owns execution instance {} for environment {}",
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

        let native = role == ProcessRole::NamespaceHolder;
        let use_bwrap = !native && !fragment.mounts.is_empty();
        let mut native_plan = None;
        let mut command = if native {
            let prepared = (|| {
                let (source, target) = fragment
                    .holder_bind
                    .as_ref()
                    .ok_or_else(|| BackendError::new("namespace holder requires a socket bind"))?;
                let stage = tempfile::Builder::new()
                    .prefix(".mount-root-")
                    .tempdir_in(
                        source
                            .parent()
                            .ok_or_else(|| BackendError::new("missing holder directory parent"))?,
                    )
                    .map_err(error)?;
                let mut entries = fragment.mounts.clone();
                // Native root setup recursively protects inherited mounts.
                entries.retain(|entry| !matches!(entry, MountEntry::RoBind { source, target } if source == target && target != std::path::Path::new("/")));
                entries.push(MountEntry::Bind {
                    source: source.clone(),
                    target: target.clone(),
                });
                let plan = serde_json::to_vec(&serde_json::json!({
                    "entries": entries,
                    "stage": stage.path(),
                    "socket": target.join("cmd.sock"),
                    "chdir": fragment.chdir.as_ref().unwrap_or(&launch.cwd),
                    "parent": std::process::id(),
                    "cgroup": cgroup_path,
                }))
                .map_err(error)?;
                Ok((holder_executable()?, plan, stage))
            })();
            match prepared {
                Ok((executable, plan, stage)) => {
                    native_plan = Some((plan, stage));
                    Command::new(executable)
                }
                Err(failure) => {
                    remove_cgroup(&cgroup_path);
                    return Err(failure);
                }
            }
        } else if use_bwrap {
            let (read, write) = match status_pipe() {
                Ok(pipe) => pipe,
                Err(failure) => {
                    self.discard_pending_status(&context.environment_id);
                    remove_cgroup(&cgroup_path);
                    return Err(failure);
                }
            };
            let status_fd = write.as_raw_fd();
            {
                let mut pending = match self.pending_status.lock() {
                    Ok(pending) => pending,
                    Err(_) => {
                        remove_cgroup(&cgroup_path);
                        return Err(BackendError::new("Processes status lock poisoned"));
                    }
                };
                pending.insert(
                    context.environment_id.clone(),
                    PendingStatus { read, write },
                );
            }
            let spec = MountSpec {
                entries: {
                    let mut entries = fragment.mounts.clone();
                    if let Some((source, target)) = &fragment.holder_bind {
                        entries.push(MountEntry::Bind {
                            source: source.clone(),
                            target: target.clone(),
                        });
                    }
                    entries
                },
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

        let cgroup_procs_fd = match cgroup_path
            .as_ref()
            .filter(|_| !native)
            .map(|path| {
                OpenOptions::new()
                    .write(true)
                    .open(path.join("cgroup.procs"))
            })
            .transpose()
        {
            Ok(fd) => fd,
            Err(failure) => {
                self.discard_pending_status(&context.environment_id);
                remove_cgroup(&cgroup_path);
                return Err(error(failure));
            }
        };
        unsafe {
            command.pre_exec(move || {
                if libc::setpgid(0, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if let Some(file) = &cgroup_procs_fd {
                    // Writing zero moves the calling process. No allocation
                    // is safe in this post-fork, pre-exec callback.
                    let written = libc::write(file.as_raw_fd(), c"0".as_ptr().cast(), 1);
                    if written < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                }
                Ok(())
            });
        }
        let log_clone = match log.try_clone() {
            Ok(log) => log,
            Err(failure) => {
                self.discard_pending_status(&context.environment_id);
                remove_cgroup(&cgroup_path);
                return Err(error(failure));
            }
        };
        let command = command
            .stdin(Stdio::null())
            .stdout(Stdio::from(log_clone))
            .stderr(Stdio::from(log))
            .envs(&launch.env)
            .envs(&fragment.env);
        if native {
            command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit());
        }
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(failure) => {
                self.discard_pending_status(&context.environment_id);
                remove_cgroup(&cgroup_path);
                return Err(error(failure));
            }
        };
        let process_group = child.id() as libc::pid_t;
        unsafe {
            if libc::setpgid(process_group, process_group) != 0
                && std::io::Error::last_os_error().raw_os_error() != Some(libc::EACCES)
            {
                let value = std::io::Error::last_os_error();
                abort_launch(
                    &context.environment_id,
                    &mut child,
                    process_group,
                    &cgroup_path,
                    self,
                );
                return Err(error(value));
            }
        }

        let prepared = (|| -> Result<(u32, u64), BackendError> {
            if let Some((plan, _stage)) = native_plan {
                child
                    .stdin
                    .take()
                    .unwrap()
                    .write_all(&plan)
                    .map_err(error)?;
                let stdout = child.stdout.take().unwrap();
                let mut poll = libc::pollfd {
                    fd: stdout.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                };
                loop {
                    match unsafe { libc::poll(&mut poll, 1, 2_000) } {
                        0 => return Err(BackendError::new("namespace holder readiness timed out")),
                        n if n > 0 => break,
                        _ => {
                            let failure = std::io::Error::last_os_error();
                            if failure.kind() != std::io::ErrorKind::Interrupted {
                                return Err(error(failure));
                            }
                        }
                    }
                }
                let mut line = String::new();
                BufReader::new(stdout).read_line(&mut line).map_err(error)?;
                let (pid, namespace) = parse_status(&line)?;
                let observed = fs::metadata(format!("/proc/{pid}/ns/mnt"))
                    .map_err(error)?
                    .ino();
                if observed != namespace {
                    return Err(BackendError::new(
                        "namespace holder reported a mismatched mount namespace",
                    ));
                }
                #[cfg(test)]
                if self
                    .fail_mount_wait
                    .load(std::sync::atomic::Ordering::SeqCst)
                {
                    return Err(BackendError::new(
                        "injected overlay mount readiness failure",
                    ));
                }
                Ok((pid, namespace))
            } else if use_bwrap {
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
                    #[cfg(test)]
                    if self
                        .fail_mount_wait
                        .load(std::sync::atomic::Ordering::SeqCst)
                    {
                        return Err(BackendError::new(
                            "injected overlay mount readiness failure",
                        ));
                    }
                    wait_for_mounts(child_pid, &overlay_targets, &mut child)?;
                }
                Ok((child_pid, namespace))
            } else {
                let namespace = fs::metadata("/proc/self/ns/mnt").map_err(error)?.ino();
                Ok((child.id(), namespace))
            }
        })();
        let (inner_pid, mount_namespace) = match prepared {
            Ok(prepared) => prepared,
            Err(failure) => {
                abort_launch(
                    &context.environment_id,
                    &mut child,
                    process_group,
                    &cgroup_path,
                    self,
                );
                return Err(failure);
            }
        };
        // Pin the branch namespace while it is owned. Without the open fd the
        // inode can be freed and reused by another Branch, which would make a
        // later namespace sweep kill the wrong processes. A short command can
        // already be reaped (and even reuse its pid), so only keep the fd when
        // it still denotes the namespace reported by bwrap.
        let namespace_fd = if native || use_bwrap {
            fs::File::open(format!("/proc/{inner_pid}/ns/mnt"))
                .ok()
                .and_then(|file| {
                    let matches = file
                        .metadata()
                        .map(|metadata| metadata.ino() == mount_namespace)
                        .unwrap_or(false);
                    matches.then(|| OwnedFd::from(file))
                })
        } else {
            None
        };
        let mut process = AgentProcess {
            instance_id,
            role,
            child,
            process_group,
            inner_pid,
            mount_namespace,
            namespace_fd,
            cgroup_path,
        };
        let mut processes = match self.processes.lock() {
            Ok(processes) => processes,
            Err(_) => {
                abort_launch(
                    &context.environment_id,
                    &mut process.child,
                    process.process_group,
                    &process.cgroup_path,
                    self,
                );
                return Err(BackendError::new("Processes lock poisoned"));
            }
        };
        processes.insert(context.environment_id.clone(), process);
        Ok(())
    }

    fn discard_pending_status(&self, environment_id: &str) {
        if let Ok(mut pending) = self.pending_status.lock() {
            pending.remove(environment_id);
        }
    }

    fn read_status(&self, environment_id: &str) -> Result<(u32, u64), BackendError> {
        let pending = {
            let mut pending = self
                .pending_status
                .lock()
                .map_err(|_| BackendError::new("Processes status lock poisoned"))?;
            pending.remove(environment_id)
        };
        let Some(PendingStatus { read, write }) = pending else {
            return Err(BackendError::new("missing bwrap status pipe"));
        };
        drop(write);
        let line = read_status_line(&read)?;
        let (child_pid, namespace) = parse_status(&line)?;
        // Keep draining so bwrap does not SIGPIPE on later status updates.
        let reader = read.try_clone().map_err(error)?;
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

fn holder_executable() -> Result<PathBuf, BackendError> {
    let executable = std::env::current_exe().map_err(error)?;
    // Cargo may put integration-test executables under build/<package>/out.
    let cargo_build_output = executable.parent().is_some_and(|parent| parent.ends_with("out")
        && parent.ancestors().nth(3).is_some_and(|path| path.ends_with("build")));
    let directory = executable
        .ancestors()
        .skip(1)
        .take(if cfg!(test) || cargo_build_output { 5 } else { 2 })
        .map(|path| path.join("ascout-fs-holder"))
        .find(|path| path.is_file())
        .ok_or_else(|| {
            BackendError::new(format!(
                "ascout-fs-holder is missing alongside {} (build with cargo build --bins)",
                executable.display()
            ))
        })?;
    Ok(directory)
}

fn parse_status(line: &str) -> Result<(u32, u64), BackendError> {
    let value: serde_json::Value = serde_json::from_str(line.trim()).map_err(|failure| {
        BackendError::new(format!("invalid sandbox readiness status: {failure}"))
    })?;
    let child_pid = value["child-pid"]
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .filter(|&pid| pid != 0)
        .ok_or_else(|| BackendError::new("sandbox status did not report child-pid"))?;
    let namespace = value["mnt-namespace"]
        .as_u64()
        .ok_or_else(|| BackendError::new("sandbox status did not report mnt-namespace"))?;
    let host_namespace = fs::metadata("/proc/self/ns/mnt").map_err(error)?.ino();
    if namespace == host_namespace {
        return Err(BackendError::new(
            "sandbox child is still in the host mount namespace",
        ));
    }
    Ok((child_pid, namespace))
}

/// spawn 之后任一准备步骤失败时的统一回收：杀掉整个进程组、等待回收、
/// 删除 cgroup 目录并移除可能残留的 status pipe。
fn abort_launch(
    environment_id: &str,
    child: &mut std::process::Child,
    process_group: libc::pid_t,
    cgroup_path: &Option<PathBuf>,
    backend: &Processes,
) {
    if let Some(path) = cgroup_path {
        let kill = path.join("cgroup.kill");
        if kill.exists() {
            let _ = fs::write(&kill, "1");
        }
    }
    unsafe {
        libc::kill(-process_group, libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
    backend.discard_pending_status(environment_id);
    remove_cgroup(cgroup_path);
}

fn remove_cgroup(path: &Option<PathBuf>) {
    let Some(path) = path else {
        return;
    };
    for _ in 0..200 {
        match fs::remove_dir(path) {
            Ok(()) => return,
            Err(failure) if failure.kind() == std::io::ErrorKind::NotFound => return,
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(5)),
        }
    }
    let _ = fs::remove_dir_all(path);
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

fn wait_for_mounts(
    pid: u32,
    targets: &[PathBuf],
    child: &mut std::process::Child,
) -> Result<(), BackendError> {
    let path = format!("/proc/{pid}/mountinfo");
    for _ in 0..MOUNT_INFO_ATTEMPTS {
        match fs::read_to_string(&path) {
            Ok(text) => {
                if overlay_targets_ready(&text, targets) {
                    return Ok(());
                }
            }
            Err(failure)
                if failure.kind() == std::io::ErrorKind::NotFound
                    || failure.raw_os_error() == Some(libc::EINVAL) =>
            {
                // Exiting processes have no mount namespace: mountinfo may
                // return EINVAL before reap or ENOENT after reap. The exit check
                // below also covers readable but incomplete mountinfo.
            }
            Err(failure) => {
                return Err(BackendError::new(format!(
                    "cannot inspect sandbox overlay readiness at {path}: {failure}"
                )));
            }
        }
        // A short command can finish between the readiness checks. Only the
        // owned outer child's exit confirms completion; an incomplete or
        // disappearing mountinfo alone never proves readiness.
        if child.try_wait().map_err(error)?.is_some() {
            return Ok(());
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    Err(BackendError::new(format!(
        "sandbox {pid} did not finish mounting all overlays before the readiness timeout: {targets:?}"
    )))
}

fn overlay_targets_ready(text: &str, targets: &[PathBuf]) -> bool {
    use std::os::unix::ffi::OsStringExt;

    let mounted: Vec<PathBuf> = text
        .lines()
        .filter_map(|line| {
            let Some((left, right)) = line.split_once(" - ") else {
                return None;
            };
            if !right.starts_with("overlay ") {
                return None;
            }
            let encoded = left.split_whitespace().nth(4)?.as_bytes();
            let mut decoded = Vec::new();
            let mut index = 0;
            while index < encoded.len() {
                if encoded[index] == b'\\'
                    && index + 3 < encoded.len()
                    && encoded[index + 1..index + 4]
                        .iter()
                        .all(|byte| (b'0'..=b'7').contains(byte))
                {
                    let octal = std::str::from_utf8(&encoded[index + 1..index + 4]).ok()?;
                    decoded.push(u8::from_str_radix(octal, 8).ok()?);
                    index += 4;
                } else {
                    decoded.push(encoded[index]);
                    index += 1;
                }
            }
            Some(PathBuf::from(std::ffi::OsString::from_vec(decoded)))
        })
        .collect();
    targets.iter().all(|target| mounted.contains(target))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_accepts_mountinfo_of_an_exited_unreaped_child() {
        for code in [0, 7] {
            let mut child = Command::new("/bin/sh")
                .args(["-c", &format!("exit {code}")])
                .spawn()
                .unwrap();
            let pid = child.id();
            let mut status: libc::siginfo_t = unsafe { std::mem::zeroed() };
            // WNOWAIT leaves the exited process in /proc. Linux returns EINVAL
            // for its mountinfo before the parent reaps it, rather than ENOENT.
            let waited = unsafe {
                libc::waitid(libc::P_PID, pid, &mut status, libc::WEXITED | libc::WNOWAIT)
            };
            let result = wait_for_mounts(pid, &[PathBuf::from("/workspace")], &mut child);
            let exit = child.wait().unwrap();
            assert_eq!(waited, 0);
            assert!(result.is_ok(), "exited child mountinfo: {result:?}");
            assert_eq!(
                exit.code(),
                Some(code),
                "readiness must retain the exit status"
            );
        }
    }

    #[test]
    fn readiness_checks_child_exit_even_when_mountinfo_is_readable() {
        let mut namespace_holder = Command::new("/bin/cat")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let mut child = Command::new("/bin/true").spawn().unwrap();
        let mut status: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let waited = unsafe {
            libc::waitid(
                libc::P_PID,
                child.id(),
                &mut status,
                libc::WEXITED | libc::WNOWAIT,
            )
        };
        // Hold a real readable /proc entry independently of the managed child.
        // This exercises successful reads with incomplete mounts deterministically,
        // without relying on a race between reading mountinfo and process exit.
        let pid = namespace_holder.id();
        let targets = [PathBuf::from("/not-a-sandbox-overlay")];
        let mountinfo = fs::read_to_string(format!("/proc/{pid}/mountinfo"));
        let result = wait_for_mounts(pid, &targets, &mut child);
        let exit = child.wait().unwrap();
        drop(namespace_holder.stdin.take());
        let holder_exit = namespace_holder.wait().unwrap();
        assert_eq!(waited, 0);
        assert!(!overlay_targets_ready(&mountinfo.unwrap(), &targets));
        assert!(result.is_ok(), "completed child readiness: {result:?}");
        assert!(exit.success());
        assert!(holder_exit.success());
    }

    #[test]
    fn readiness_requires_every_overlay_and_decodes_mount_paths() {
        let targets = vec![PathBuf::from("/workspace"), PathBuf::from("/extra mount")];
        let first = "21 1 0:1 / /workspace rw - overlay overlay rw\n";
        assert!(
            !overlay_targets_ready(first, &targets),
            "one overlay is not all overlays"
        );
        let both = format!("{first}22 1 0:2 / /extra\\040mount rw - overlay overlay rw\n");
        assert!(overlay_targets_ready(&both, &targets));
    }
}
