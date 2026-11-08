//! refs 相关实现。

use super::*;

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
        for snapshot in state.snapshots.values() {
            self.persist_snapshot(snapshot)?;
        }
        Ok(garbage)
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
        if self.mechanism == FilesystemMechanism::Btrfs {
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
        self.delete_in_background(destination);
        Ok(())
    }

    pub(super) fn delete_in_background(&self, path: PathBuf) {
        let btrfs = self.mechanism == FilesystemMechanism::Btrfs;
        let fast_delete = self.fast_delete;
        let btrfs_path = self.config.btrfs_path.clone();
        thread::spawn(move || {
            if let Err(error) = remove_path_with_mode(&btrfs_path, &path, btrfs, fast_delete) {
                eprintln!(
                    "filesystem: background removal of {} failed: {error}",
                    path.display()
                );
            }
        });
    }

}
