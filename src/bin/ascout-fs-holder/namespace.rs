use ascout::runtime::launch_plan::{MountEntry, sandbox_root_needs_assembly};
use serde::Deserialize;
use serde_json::json;
use std::ffi::{CString, OsStr};
use std::fs::{self, File};
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::os::unix::{ffi::OsStrExt, fs::MetadataExt};
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
struct Plan {
    entries: Vec<MountEntry>,
    stage: PathBuf,
    socket: PathBuf,
    chdir: PathBuf,
    parent: u32,
    cgroup: Option<PathBuf>,
}

fn check(result: libc::c_int) -> io::Result<()> {
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn cstr(value: &OsStr) -> io::Result<CString> {
    CString::new(value.as_bytes()).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))
}

fn mount(
    source: Option<&Path>,
    target: &Path,
    kind: Option<&str>,
    flags: libc::c_ulong,
    data: Option<&str>,
) -> io::Result<()> {
    let source = source.map(|p| cstr(p.as_os_str())).transpose()?;
    let target_name = cstr(target.as_os_str())?;
    let kind = kind.map(CString::new).transpose()?;
    let data = data.map(CString::new).transpose()?;
    let ptr = |s: Option<&CString>| s.map_or(std::ptr::null(), |s| s.as_ptr());
    check(unsafe {
        libc::mount(
            ptr(source.as_ref()),
            target_name.as_ptr(),
            ptr(kind.as_ref()),
            flags,
            ptr(data.as_ref()).cast(),
        )
    })
    .map_err(|err| io::Error::new(err.kind(), format!("mount {target:?} {kind:?}: {err}")))
}

fn readonly(target: &Path) -> io::Result<()> {
    let name = cstr(target.as_os_str())?;
    let attributes = [1_u64, 0, 0, 0]; // MOUNT_ATTR_RDONLY, remaining fields zero
    let result = unsafe {
        libc::syscall(
            libc::SYS_mount_setattr,
            libc::AT_FDCWD,
            name.as_ptr(),
            libc::AT_RECURSIVE,
            attributes.as_ptr(),
            std::mem::size_of_val(&attributes),
        )
    };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn bind(source: &Path, target: &Path, ro: bool) -> io::Result<()> {
    let directory = source.metadata()?.is_dir();
    if !target.exists() {
        if directory {
            fs::create_dir_all(target)?;
        } else {
            fs::create_dir_all(target.parent().unwrap())?;
            File::create(target)?;
        }
    }
    mount(
        Some(source),
        target,
        None,
        libc::MS_BIND | if directory { libc::MS_REC } else { 0 },
        None,
    )?;
    if ro {
        readonly(target)?;
    }
    Ok(())
}

fn tmpfs(path: &Path, mode: &str) -> io::Result<()> {
    fs::create_dir_all(path)?;
    mount(
        None,
        path,
        Some("tmpfs"),
        libc::MS_NOSUID | libc::MS_NODEV,
        Some(mode),
    )
}

fn devices(target: &Path) -> io::Result<()> {
    tmpfs(target, "mode=0755")?;
    for name in ["null", "zero", "full", "random", "urandom", "tty"] {
        bind(&Path::new("/dev").join(name), &target.join(name), false)?;
    }
    for (name, source) in [
        ("fd", "/proc/self/fd"),
        ("stdin", "/proc/self/fd/0"),
        ("stdout", "/proc/self/fd/1"),
        ("stderr", "/proc/self/fd/2"),
        ("ptmx", "pts/ptmx"),
    ] {
        std::os::unix::fs::symlink(source, target.join(name))?;
    }
    tmpfs(&target.join("shm"), "mode=1777")?;
    fs::create_dir(target.join("pts"))?;
    mount(
        None,
        &target.join("pts"),
        Some("devpts"),
        libc::MS_NOSUID | libc::MS_NOEXEC,
        Some("newinstance,ptmxmode=0666,mode=0620"),
    )
}

fn setup(plan: &Plan) -> io::Result<()> {
    mount(
        None,
        Path::new("/"),
        None,
        libc::MS_REC | libc::MS_PRIVATE,
        None,
    )?;
    // `/proc` and `/dev` are replaced, so their bind targets may not exist on the host.
    let root_entries: Vec<_> = plan
        .entries
        .iter()
        .filter(|entry| {
            !entry
                .target()
                .is_some_and(|p| p.starts_with("/dev") || p.starts_with("/proc"))
        })
        .cloned()
        .collect();
    let assemble = sandbox_root_needs_assembly(&root_entries);
    if assemble {
        tmpfs(&plan.stage, "mode=0755")?;
        mount(None, &plan.stage, None, libc::MS_UNBINDABLE, None)?;
        for entry in fs::read_dir("/")? {
            let entry = entry?;
            let name = entry.file_name();
            if name == "proc" || name == "dev" {
                continue;
            }
            let path = entry.path();
            if root_entries
                .iter()
                .any(|e| matches!(e, MountEntry::Overlay { target, .. } if target == &path))
            {
                continue;
            }
            let destination = plan.stage.join(name);
            if entry.file_type()?.is_symlink() {
                std::os::unix::fs::symlink(fs::read_link(path)?, destination)?;
            } else {
                bind(&path, &destination, true)?;
            }
        }
    }
    for entry in &plan.entries {
        let path = |absolute: &Path| -> io::Result<PathBuf> {
            if !absolute.is_absolute()
                || absolute
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "invalid mount target",
                ));
            }
            Ok(plan.stage.join(absolute.strip_prefix("/").unwrap()))
        };
        match entry {
            MountEntry::RoBind { source, target } if !(assemble && target == Path::new("/")) => {
                bind(source, &path(target)?, true)?
            }
            MountEntry::RoBind { .. } => (),
            MountEntry::Bind { source, target } => bind(source, &path(target)?, false)?,
            MountEntry::Overlay {
                lowers,
                upper,
                work,
                target,
            } => {
                let target = path(target)?;
                fs::create_dir_all(&target)?;
                // Fds protect layer paths from delimiters and overlays on the original mount points.
                let files: Vec<File> = lowers
                    .iter()
                    .rev()
                    .chain([upper, work])
                    .map(File::open)
                    .collect::<io::Result<_>>()?;
                let paths: Vec<_> = files
                    .iter()
                    .map(|file| format!("/proc/self/fd/{}", file.as_raw_fd()))
                    .collect();
                let n = lowers.len();
                let options = format!(
                    "userxattr,lowerdir={},upperdir={},workdir={}",
                    paths[..n].join(":"),
                    paths[n],
                    paths[n + 1]
                );
                mount(
                    None,
                    &target,
                    Some("overlay"),
                    libc::MS_NOSUID | libc::MS_NODEV,
                    Some(&options),
                )?;
            }
            MountEntry::Proc { target } => {
                let target = path(target)?;
                fs::create_dir_all(&target)?;
                mount(
                    None,
                    &target,
                    Some("proc"),
                    libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC | libc::MS_RDONLY,
                    None,
                )?;
            }
            MountEntry::Dev { target } => devices(&path(target)?)?,
            MountEntry::Tmpfs { target } => tmpfs(&path(target)?, "mode=0755")?,
            MountEntry::RemountRo { target } => readonly(&path(target)?)?,
            MountEntry::Dir { target } => fs::create_dir_all(path(target)?)?,
        }
    }
    if assemble {
        // Mark only the root read-only. Child mounts may be writable.
        mount(
            None,
            &plan.stage,
            None,
            libc::MS_BIND | libc::MS_REMOUNT | libc::MS_RDONLY | libc::MS_NOSUID | libc::MS_NODEV,
            None,
        )?;
    }
    std::env::set_current_dir(&plan.stage)?;
    check(
        unsafe { libc::syscall(libc::SYS_pivot_root, c".".as_ptr(), c".".as_ptr()) } as libc::c_int,
    )?;
    check(unsafe { libc::umount2(c".".as_ptr(), libc::MNT_DETACH) })?;
    std::env::set_current_dir(&plan.chdir)?;
    // Even UID 0 in a user namespace must not regain mount capabilities on exec.
    check(unsafe { libc::prctl(libc::PR_SET_SECUREBITS, 3, 0, 0, 0) })?; // NOROOT | NOROOT_LOCKED
    check(unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) })?;
    Ok(())
}

pub fn run() -> io::Result<()> {
    let plan: Plan = serde_json::from_reader(io::stdin().lock())?;
    check(unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0) })?;
    if unsafe { libc::getppid() } as u32 != plan.parent {
        return Err(io::Error::other("adapter exited"));
    }
    let (uid, gid) = unsafe { (libc::geteuid(), libc::getegid()) };
    let namespaces = libc::CLONE_NEWUSER | libc::CLONE_NEWNS | libc::CLONE_NEWPID;
    let cgroup = plan.cgroup.as_ref().map(File::open).transpose()?;
    // Clone while single-threaded. The child starts as PID 1 in its own namespace.
    let mut args: libc::clone_args = unsafe { std::mem::zeroed() };
    args.flags = namespaces as u64;
    args.exit_signal = libc::SIGCHLD as u64;
    if let Some(file) = &cgroup {
        args.flags |= 1 << 33; // CLONE_INTO_CGROUP
        args.cgroup = file.as_raw_fd() as u64;
    }
    let mut child = unsafe { libc::syscall(libc::SYS_clone3, &args, std::mem::size_of_val(&args)) }
        as libc::pid_t;
    drop(cgroup);
    if child < 0 {
        let failure = io::Error::last_os_error();
        if !matches!(
            failure.raw_os_error(),
            Some(libc::ENOSYS | libc::EINVAL | libc::EOPNOTSUPP | libc::EPERM | libc::EACCES)
        ) {
            return Err(failure);
        }
        if let Some(path) = &plan.cgroup {
            fs::write(path.join("cgroup.procs"), "0")?;
        }
        check(unsafe { libc::unshare(namespaces) })?;
        child = unsafe { libc::fork() };
        check(child)?;
    }
    if child != 0 {
        unsafe {
            libc::close(libc::STDOUT_FILENO);
        }
        let mut status = 0;
        loop {
            let result = unsafe { libc::waitpid(child, &mut status, 0) };
            if result == child {
                break;
            }
            if io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
                check(result)?;
            }
        }
        std::process::exit(if libc::WIFEXITED(status) {
            libc::WEXITSTATUS(status)
        } else {
            1
        });
    }
    check(unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0) })?;
    fs::write("/proc/self/setgroups", "deny")?;
    fs::write("/proc/self/uid_map", format!("{uid} {uid} 1"))?;
    fs::write("/proc/self/gid_map", format!("{gid} {gid} 1"))?;
    // The inherited proc mount still reports the PID as seen by the adapter.
    let proc_pid = fs::read_link("/proc/self")?;
    let pid: u32 = proc_pid
        .to_string_lossy()
        .parse()
        .map_err(io::Error::other)?;
    let namespace = fs::metadata("/proc/self/ns/mnt")?.ino();
    setup(&plan)?;
    let listener = super::protocol::listen(&plan.socket)?;
    let mut ready = io::stdout().lock();
    writeln!(
        ready,
        "{}",
        json!({"child-pid": pid, "mnt-namespace": namespace})
    )?;
    ready.flush()?;
    drop(ready);
    unsafe {
        libc::close(libc::STDOUT_FILENO);
    }
    let null = File::open("/dev/null")?;
    if null.as_raw_fd() == libc::STDOUT_FILENO {
        std::mem::forget(null);
    }
    super::protocol::serve(listener)
}
