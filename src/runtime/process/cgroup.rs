//! cgroup v2 目录探测与冻结等待。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use super::*;

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
