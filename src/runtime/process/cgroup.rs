//! cgroup v2 目录探测与冻结等待。

use std::fs;
use std::io::{Read, Seek};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use super::*;

const FREEZE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);

pub(super) fn probe_cgroup_root() -> Option<PathBuf> {
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

pub(super) fn wait_frozen(path: &Path) -> Result<(), BackendError> {
    let mut file = fs::File::open(path.join("cgroup.events")).map_err(error)?;
    let deadline = std::time::Instant::now() + FREEZE_TIMEOUT;
    let mut events = String::new();
    loop {
        events.clear();
        file.rewind().map_err(error)?;
        file.read_to_string(&mut events).map_err(error)?;
        if events.lines().any(|line| line.trim() == "frozen 1") {
            return Ok(());
        }
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        let mut fd = libc::pollfd {
            fd: file.as_raw_fd(),
            events: libc::POLLPRI,
            revents: 0,
        };
        let result = unsafe { libc::poll(&mut fd, 1, remaining.as_millis().max(1) as i32) };
        if result < 0 {
            let failure = std::io::Error::last_os_error();
            if failure.kind() != std::io::ErrorKind::Interrupted {
                return Err(error(failure));
            }
        }
    }
    Err(BackendError::new("cgroup did not report frozen=1"))
}

pub(super) fn sanitize(value: &str) -> String {
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
