//! Authority publication reads only paths selected by its overlay delta.
//! A persistent package freezes the affected final view before any host write;
//! failed publication retains the package and its per-path receipts for retry.
use super::*;

impl FilesystemBackend {
    pub(super) fn materialize_authority(&self, id: &str) -> Result<FilesystemResult, BackendError> {
        let environment = Self::resolve_environment(&*self.lock()?, id)?;
        for mount in environment.mounts.values() {
            let package = environment.root.join(format!("publication-{}", short_hash(&mount.target.display().to_string())));
            if !package.join("ready").exists() {
                if package.exists() { remove_directory_force(&package)?; }
                fs::create_dir(&package).map_err(error)?;
                let mut selected = BTreeMap::<String, bool>::new();
                for layer in mount.lower.iter().map(|layer| mount.layer_upper(layer)).chain(std::iter::once(mount.upper.clone())) {
                    for (path, node) in read_layer_if_exists(&layer)? {
                        // Selection is a union, not a semantic merge: a later
                        // copy-up must not hide an earlier opaque/redirect's
                        // complete affected subtree. The kernel resolves values.
                        *selected.entry(path).or_default() |= node.opaque || node.redirect.is_some();
                    }
                }
                let paths: Vec<_> = selected.iter().map(|(path, subtree)| {
                    serde_json::json!({"path":path,"subtree":subtree})
                }).collect();
                fs::write(package.join("selection.json"), serde_json::to_vec(&paths).map_err(error)?).map_err(error)?;
                let mut command = Command::new(&self.config.bubblewrap_path);
                command.args(["--die-with-parent", "--unshare-user", "--ro-bind", "/", "/"]);
                for lower in self.launch_lowers(mount) { command.arg("--overlay-src").arg(lower); }
                command.arg("--overlay-src").arg(&mount.upper).arg("--ro-overlay").arg(&mount.target);
                command.arg("--bind").arg(&package).arg(&package);
                command.args(["--", "python3", "-c", include_str!("materialize.py"), "export"])
                    .arg(&mount.target).arg(&package);
                checked(command, "read Authority delta final view")?;
            }
        }
        // Freeze every affected mount before touching host lowers. Keep all
        // packages until the complete publication succeeds, including receipts
        // for mounts already completed before a later mount failed.
        for mount in environment.mounts.values() {
            let package = environment.root.join(format!("publication-{}", short_hash(&mount.target.display().to_string())));
            let mut command = Command::new("python3");
            command.args(["-c", include_str!("materialize.py"), "publish"]).arg(&mount.target).arg(&package);
            checked(command, "publish Authority delta")?;
            // Execution reset requires coordinated live-mount replacement;
            // never approximate it by copying the host mount.
        }
        // No tool mount is live at this boundary. Replace the execution plan
        // with fresh uppers, never clear or mutate an existing mounted upper.
        // Keep the old root/package until metadata and the new plan are durable.
        let mut next=environment.clone();
        next.root=self.environments_root().join(format!("{}-task-{}",environment.id,now_nanos()));
        next.base_snapshot_id=None;
        next.parent_snapshot_id=None;
        next.marks.clear();
        for mount in next.mounts.values_mut() {
            mount.saved_base=None;
            mount.lower.clear();
            let volume=next.root.join(&mount.layer_key);
            if self.uses_btrfs_storage() {
                fs::create_dir_all(volume.parent().unwrap()).map_err(error)?;
                self.btrfs_create(&volume)?;
            }
            mount.upper=volume.join("upper");mount.work=volume.join("work");
        }
        Self::prepare_mount_dirs(&next.mounts.values().cloned().collect::<Vec<_>>())?;
        // Until both files are installed keep memory on the old generation and
        // retain its publication receipts. Retry must not export the host again.
        // Retaining the old generation can fail (for example metadata storage
        // is unwritable). Do it before publishing a new execution plan.
        let mut state = self.lock()?;
        self.retain_retired_root(&mut state, &environment.root)?;
        if let Err(failure) = self.persist_environment(&next) {
            // persist_environment may have installed metadata before its plan
            // write failed. Restore the previous durable description as well.
            self.write_json(&self.environment_metadata_path(id), &EnvironmentMetadata::from_environment(&environment))?;
            return Err(failure);
        }
        state.environments.insert(next.id.clone(), next);
        // Publication and execution-plan replacement are now committed. GC is
        // maintenance: failure must not turn this into a retry with a new delta.
        // Retain storage rather than delete anything if metadata persistence fails.
        let garbage = self.refresh_references(&mut state)?;
        let referenced = Self::root_referenced(&state, &environment.root);
        drop(state);
        if let Err(failure) = self.persist_all_snapshots().and_then(|_| self.schedule_gc(garbage)) {
            eprintln!("filesystem: publication succeeded; deferred snapshot cleanup: {failure}");
        } else if !referenced {
            if let Err(failure) = self.retire_path(&environment.root) {
                eprintln!("filesystem: publication succeeded; retained old root: {failure}");
            }
        }
        let mut result = FilesystemResult::default();
        result.confirmation.published = true;
        Ok(result)
    }
}

fn checked(mut command: Command, operation: &str) -> Result<(), BackendError> {
    let output = command.output().map_err(error)?;
    if !output.status.success() {
        return Err(BackendError::new(format!("{operation}: {}", String::from_utf8_lossy(&output.stderr))));
    }
    Ok(())
}
