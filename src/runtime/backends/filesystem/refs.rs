//! refs 相关实现。

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Condvar};
use std::time::Duration;

use super::*;

/// Only one background reclamation runs at a time.  Concurrent recursive
/// deletes of many btrfs snapshots contend for filesystem transactions and
/// slow the next fork; serializing them keeps the foreground latency bounded.
static BACKGROUND_DELETE_LOCK: Mutex<()> = Mutex::new(());

/// How long the worker waits before retrying entries that are still present.
/// A new signal wakes it earlier; shutdown wakes it immediately.
const DELETE_RETRY_DELAY: Duration = Duration::from_millis(500);

/// Shared state for the background deletion worker.  The worker owns no
/// strong reference to `FilesystemBackend`; it only holds the configuration
/// values it needs.  `FilesystemBackend::drop` sets `shutdown` so the worker
/// leaves unfinished entries in trash for the next startup to reclaim.
pub(super) struct DeleteWorker {
    signaled: Mutex<bool>,
    condvar: Condvar,
    shutdown: AtomicBool,
    trash_root: PathBuf,
    btrfs_path: PathBuf,
    use_btrfs: bool,
    fast_delete: bool,
}

impl DeleteWorker {
    pub(super) fn new(config: &FilesystemConfig, use_btrfs: bool, fast_delete: bool) -> Self {
        Self {
            signaled: Mutex::new(false),
            condvar: Condvar::new(),
            shutdown: AtomicBool::new(false),
            trash_root: config.state_directory.join("trash"),
            btrfs_path: config.btrfs_path.clone(),
            use_btrfs,
            fast_delete,
        }
    }

    pub(super) fn signal(&self) {
        let mut signaled = self
            .signaled
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // Repeated signals collapse into the one pending sweep.
        *signaled = true;
        self.condvar.notify_one();
    }

    pub(super) fn shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
        self.condvar.notify_all();
    }

    fn is_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::SeqCst)
    }

    /// Block until a sweep is requested or the backend shuts down.  Returns
    /// false when the worker must exit.
    fn wait_for_signal(&self) -> bool {
        let mut signaled = self
            .signaled
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while !*signaled && !self.is_shutdown() {
            signaled = self
                .condvar
                .wait(signaled)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
        let run = !self.is_shutdown();
        *signaled = false;
        run
    }

    /// Wait for a new signal or the bounded retry deadline.  A signal wakes
    /// the worker immediately; shutdown also ends the wait.
    fn wait_for_retry(&self) {
        let signaled = self
            .signaled
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _ = self
            .condvar
            .wait_timeout(signaled, DELETE_RETRY_DELAY)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.clear_signal();
    }

    fn clear_signal(&self) {
        let mut signaled = self
            .signaled
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *signaled = false;
    }

    /// Delete every settled entry in trash.  Returns true when trash is empty.
    /// Transient entries that may still be renamed into place are left alone;
    /// a failed entry is logged and retried later instead of being dropped.
    fn sweep(&self) -> bool {
        let entries = match fs::read_dir(&self.trash_root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return true,
            Err(error) => {
                eprintln!(
                    "filesystem: cannot scan trash {}: {error}",
                    self.trash_root.display()
                );
                return false;
            }
        };
        let _serial = BACKGROUND_DELETE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut remaining = false;
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    eprintln!(
                        "filesystem: cannot read entry in trash {}: {error}",
                        self.trash_root.display()
                    );
                    remaining = true;
                    continue;
                }
            };
            let name = entry.file_name();
            if is_transient_name(&name.to_string_lossy()) {
                // A staging item may not be fully renamed into place yet.
                // Do not delete it; retry on the next pass or startup.
                remaining = true;
                continue;
            }
            let path = entry.path();
            if let Err(error) = remove_path_with_mode(
                &self.btrfs_path,
                &path,
                self.use_btrfs,
                self.fast_delete,
            ) {
                eprintln!(
                    "filesystem: background removal of {} failed: {error}",
                    path.display()
                );
                remaining = true;
            }
        }
        !remaining && trash_is_empty(&self.trash_root)
    }
}

fn is_transient_name(name: &str) -> bool {
    name.starts_with('.')
}

fn trash_is_empty(path: &Path) -> bool {
    match fs::read_dir(path) {
        Ok(mut entries) => entries.next().is_none(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Err(_) => false,
    }
}

pub(super) fn spawn_delete_worker(worker: Arc<DeleteWorker>) {
    thread::spawn(move || {
        while worker.wait_for_signal() {
            loop {
                if worker.is_shutdown() {
                    return;
                }
                if worker.sweep() {
                    break;
                }
                // Failed or transient entries remain.  Wait for the next
                // signal or a bounded retry deadline; never busy-spin.
                worker.wait_for_retry();
            }
        }
    });
}

impl FilesystemBackend {
    /// Rebuild snapshot reference tokens from the current environment graph.
    /// Unreferenced snapshots are removed from memory and scheduled for GC.
    pub(super) fn refresh_references(
        &self,
        state: &mut BackendState,
    ) -> Result<Vec<(String, PathBuf)>, BackendError> {
        let path_to_id: HashMap<PathBuf, String> = state
            .snapshots
            .values()
            .map(|snapshot| (snapshot.path.clone(), snapshot.snapshot_id.clone()))
            .collect();
        let mut tokens = Vec::new();
        for environment in state.environments.values() {
            if let Some(base) = &environment.base_snapshot_id {
                tokens.push((base.clone(), format!("env:{}:base", environment.id)));
            }
            for (target, mount) in &environment.mounts {
                for layer in &mount.lower {
                    if let Some(id) = path_to_id.get(layer) {
                        tokens.push((
                            id.clone(),
                            format!("env:{}:lower:{target}", environment.id),
                        ));
                    }
                }
            }
            for (mark, snapshot) in &environment.marks {
                tokens.push((
                    snapshot.clone(),
                    format!("env:{}:mark:{mark}", environment.id),
                ));
            }
        }
        for snapshot in state.snapshots.values_mut() {
            snapshot.refs.clear();
        }
        for (id, token) in tokens {
            if let Some(snapshot) = state.snapshots.get_mut(&id) {
                snapshot.refs.insert(token);
            }
        }
        let garbage: Vec<(String, PathBuf)> = state
            .snapshots
            .values()
            .filter(|snapshot| snapshot.refs.is_empty())
            .map(|snapshot| (snapshot.snapshot_id.clone(), snapshot.path.clone()))
            .collect();
        for (id, _) in &garbage {
            state.snapshots.remove(id);
            self.remove_snapshot_metadata(id);
        }
        Ok(garbage)
    }

    /// Persist the current snapshot set after the global state lock has been
    /// released.  Callers must invoke this after `refresh_references`.
    pub(super) fn persist_all_snapshots(&self) -> Result<(), BackendError> {
        let snapshots: Vec<SnapshotRecord> = {
            let state = self.lock()?;
            state.snapshots.values().cloned().collect()
        };
        for snapshot in snapshots {
            self.persist_snapshot(&snapshot)?;
        }
        Ok(())
    }

    pub(super) fn schedule_gc(&self, paths: Vec<(String, PathBuf)>) -> Result<(), BackendError> {
        for (_, path) in paths {
            self.retire_path(&path)?;
        }
        Ok(())
    }

    /// Move a path out of the reachable namespace in one rename.  Actual
    /// reclamation happens in a background thread.  This is the hot path of
    /// discard; without the btrfs delete ioctl the background pass is
    /// O(file count), but the caller does not wait for it.
    pub(super) fn retire_path(&self, path: &Path) -> Result<(), BackendError> {
        self.retire_path_in(path, &self.trash_root())
    }

    pub(super) fn retire_path_in(&self, path: &Path, trash: &Path) -> Result<(), BackendError> {
        if !path_exists(path) {
            return Ok(());
        }
        if self.uses_btrfs_storage() {
            let _ = Command::new(&self.config.btrfs_path)
                .args([
                    std::ffi::OsStr::new("property"),
                    std::ffi::OsStr::new("set"),
                    path.as_os_str(),
                    std::ffi::OsStr::new("ro"),
                    std::ffi::OsStr::new("false"),
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        let token = self.token();
        fs::create_dir_all(trash).map_err(error)?;
        let name = path
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_else(|| "retired".to_string());
        let destination = trash.join(format!("{token}-{name}"));
        fs::rename(path, &destination).map_err(error)?;
        // Signal the worker; concurrent discards coalesce into one sweep.
        self.delete_worker.signal();
        Ok(())
    }

}
