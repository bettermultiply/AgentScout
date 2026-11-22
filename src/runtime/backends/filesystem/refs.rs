
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Condvar};
use std::time::Duration;

use super::*;

/// Concurrent btrfs reclamation contends with foreground forks.
static BACKGROUND_DELETE_LOCK: Mutex<()> = Mutex::new(());

const DELETE_RETRY_DELAY: Duration = Duration::from_millis(500);

/// The worker must allow backend Drop to run and leave unfinished trash for startup.
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
                // Staging may still be in use. Retry on the next pass or startup.
                remaining = true;
                continue;
            }
            let path = entry.path();
            if let Err(error) =
                remove_path_with_mode(&self.btrfs_path, &path, self.use_btrfs, self.fast_delete)
            {
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
                worker.wait_for_retry();
            }
        }
    });
}

impl FilesystemBackend {
    pub(super) fn retain_retired_root(
        &self,
        state: &mut BackendState,
        root: &Path,
    ) -> Result<(), BackendError> {
        let id = format!("owned-root-{}", short_hash(&root.display().to_string()));
        let record = SnapshotRecord {
            snapshot_id: id.clone(),
            path: root.into(),
            read_only: true,
            lower: BTreeMap::new(),
            refs: BTreeSet::new(),
        };
        self.persist_snapshot(&record)?;
        state.snapshots.insert(id, record);
        Ok(())
    }
    pub(super) fn root_referenced(state: &BackendState, root: &Path) -> bool {
        state.environments.values().any(|env| {
            env.root.starts_with(root)
                || env.mounts.values().any(|mount| {
                    mount.upper.starts_with(root)
                        || mount.work.starts_with(root)
                        || mount
                            .saved_base
                            .as_ref()
                            .is_some_and(|base| base.starts_with(root))
                        || mount.lower.iter().any(|layer| layer.starts_with(root))
                })
        }) || state.snapshots.values().any(|snapshot| {
            snapshot.path.starts_with(root)
                || snapshot
                    .lower
                    .values()
                    .flatten()
                    .any(|layer| layer.starts_with(root))
        })
    }
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
                for layer in mount.lower.iter().chain(mount.saved_base.iter()) {
                    for (path, id) in &path_to_id {
                        if !layer.starts_with(path) {
                            continue;
                        }
                        tokens.push((id.clone(), format!("env:{}:lower:{target}", environment.id)));
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
        // Count lower references only from reachable snapshots so dead chains can be reclaimed.
        let mut pending: Vec<String> = tokens.iter().map(|(id, _)| id.clone()).collect();
        let mut visited = BTreeSet::new();
        while let Some(id) = pending.pop() {
            if !visited.insert(id.clone()) {
                continue;
            }
            if let Some(snapshot) = state.snapshots.get(&id) {
                for layer in snapshot.lower.values().flatten() {
                    for (path, lower_id) in &path_to_id {
                        if !layer.starts_with(path) {
                            continue;
                        }
                        if lower_id != &id {
                            tokens.push((lower_id.clone(), format!("snapshot:{id}:lower")));
                            pending.push(lower_id.clone());
                        }
                    }
                }
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

    /// Call after `refresh_references` and after releasing the global state lock.
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
        self.delete_worker.signal();
        Ok(())
    }
}
