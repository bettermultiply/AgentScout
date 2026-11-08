//! fork 数据面操作。

use super::*;

impl FilesystemBackend {
    pub fn fork(
        &self,
        parent: &str,
        child: &str,
        scope: &[PathScope],
    ) -> Result<FilesystemResult, BackendError> {
        let scopes = normalize_scopes(scope)?;
        validate_component(child)?;
        let (parent_environment, existing, existing_snapshot) = {
            let state = self.lock()?;
            let parent = Self::resolve_environment(&state, parent)?;
            let existing = state.environments.get(child).cloned();
            let snapshot = existing
                .as_ref()
                .and_then(|environment| environment.base_snapshot_id.as_ref())
                .and_then(|id| state.snapshots.get(id))
                .cloned();
            (parent, existing, snapshot)
        };
        if let Some(existing) = existing {
            let matches_parent =
                existing.parent_snapshot_id.as_deref() == Some(parent_environment.snapshot_id.as_str());
            let verified = matches_parent
                && existing_snapshot
                    .as_ref()
                    .is_some_and(|snapshot| path_exists(&snapshot.path))
                && path_exists(&self.environment_metadata_path(&existing.id))
                && existing
                    .base_snapshot_id
                    .as_ref()
                    .is_some_and(|id| path_exists(&self.snapshot_metadata_path(id)));
            if !verified {
                return Err(BackendError::new(format!(
                    "replayed fork for {child} could not verify the snapshot and durable metadata"
                )));
            }
            let mut result = Self::snapshot_result(existing.snapshot_id);
            result.confirmation.snapshot_created = true;
            result.confirmation.metadata_durable = true;
            return Ok(result);
        }
        if !matches!(
            parent_environment.state,
            EnvironmentState::Available | EnvironmentState::Suspended
        ) {
            return Err(BackendError::new(
                "filesystem parent must be available or suspended before fork",
            ));
        }

        let child_root = self.environments_root().join(child);
        let snapshot_id = Self::base_snapshot_id_for_child(child);
        let layer_path = self.layer_path_for(&snapshot_id);
        if path_exists(&child_root) || path_exists(&layer_path) {
            let _ = remove_path(
                &self.config.btrfs_path,
                &child_root,
                self.mechanism == FilesystemMechanism::Btrfs,
            );
            let _ = remove_path(
                &self.config.btrfs_path,
                &layer_path,
                self.mechanism == FilesystemMechanism::Btrfs,
            );
            if path_exists(&child_root) || path_exists(&layer_path) {
                return Err(BackendError::new(format!(
                    "filesystem data for {child} already exists without durable metadata"
                )));
            }
        }

        let selected = parent_environment.selected_mounts(&scopes)?;
        let selected_targets: BTreeSet<String> = selected
            .iter()
            .map(|mount| mount.target.display().to_string())
            .collect();
        let mut child_mounts = BTreeMap::new();
        for mount in parent_environment.mounts.values() {
            let key = mount.target.display().to_string();
            let mut unit = MountUnit {
                target: mount.target.clone(),
                upper: child_root.join(&mount.layer_key).join("upper"),
                work: child_root.join(&mount.layer_key).join("work"),
                lower: mount.lower.clone(),
                layer_key: mount.layer_key.clone(),
            };
            if selected_targets.contains(&key) {
                unit.lower.push(layer_path.clone());
            }
            child_mounts.insert(key, unit);
        }
        let child_environment = Environment {
            id: child.to_string(),
            root: child_root.clone(),
            state: EnvironmentState::Available,
            snapshot_id: Self::snapshot_id_for_child(child),
            parent_snapshot_id: Some(parent_environment.snapshot_id.clone()),
            base_snapshot_id: Some(snapshot_id.clone()),
            scopes: scopes.clone(),
            mounts: child_mounts.clone(),
            lower: BTreeMap::new(),
            marks: BTreeMap::new(),
            workspace_root: parent_environment.workspace_root.clone(),
        };
        let creation = (|| -> Result<(), BackendError> {
            match self.mechanism {
                FilesystemMechanism::Btrfs => {
                    // One read-only snapshot per selected mount upper, then a
                    // writable child upper per mount.  The parent is untouched.
                    fs::create_dir_all(&layer_path).map_err(error)?;
                    for mount in &selected {
                        let layer_upper = mount.layer_upper(&layer_path);
                        if let Some(parent) = layer_upper.parent() {
                            fs::create_dir_all(parent).map_err(error)?;
                        }
                        self.btrfs_snapshot(&mount.upper, &layer_upper, true)?;
                        let child_mount = child_environment
                            .mounts
                            .get(&mount.target.display().to_string())
                            .expect("child mount exists");
                        if let Some(parent) = child_mount.upper.parent() {
                            fs::create_dir_all(parent).map_err(error)?;
                        }
                        if self.mount_uses_flattening(mount) {
                            // Writable snapshot carries the parent state, so
                            // the layer is metadata/lower only and the mount
                            // is launched with the host mount as sole lower.
                            self.btrfs_snapshot(&layer_upper, &child_mount.upper, false)?;
                        } else {
                            self.btrfs_create(&child_mount.upper)?;
                        }
                    }
                    for mount in child_environment.mounts.values() {
                        if path_exists(&mount.upper) {
                            continue;
                        }
                        if let Some(parent) = mount.upper.parent() {
                            fs::create_dir_all(parent).map_err(error)?;
                        }
                        self.btrfs_create(&mount.upper)?;
                    }
                    let units: Vec<MountUnit> = child_environment.mounts.values().cloned().collect();
                    Self::prepare_mount_dirs(&units)?;
                }
                FilesystemMechanism::Overlay => {
                    fs::create_dir_all(&layer_path).map_err(error)?;
                    for mount in &selected {
                        clone_upper(
                            &mount.upper,
                            &mount.layer_upper(&layer_path),
                            self.fork_cost == ForkCost::Reflink,
                        )?;
                        if self.mount_uses_flattening(mount) {
                            // The state directory is below this mount, so the
                            // snapshot layer may not be used as an overlay
                            // lower (directories would overlap).  Flatten the
                            // parent upper into the child upper instead; the
                            // layer copy is metadata for diff/accept only.
                            let child_mount = child_environment
                                .mounts
                                .get(&mount.target.display().to_string())
                                .expect("child mount exists");
                            clone_upper(
                                &mount.upper,
                                &child_mount.upper,
                                self.fork_cost == ForkCost::Reflink,
                            )?;
                        }
                    }
                    fs::create_dir_all(&child_root).map_err(error)?;
                    let units: Vec<MountUnit> = child_environment.mounts.values().cloned().collect();
                    Self::prepare_mount_dirs(&units)?;
                }
            }
            Ok(())
        })();

        if let Err(error) = creation {
            self.rollback_fork(&child_root, &layer_path);
            return Err(error);
        }

        let snapshot_lower: BTreeMap<String, Vec<PathBuf>> = child_environment
            .mounts
            .iter()
            .map(|(key, mount)| (key.clone(), mount.lower.clone()))
            .collect();
        let snapshot = SnapshotRecord {
            snapshot_id: snapshot_id.clone(),
            path: layer_path.clone(),
            read_only: true,
            lower: snapshot_lower,
            refs: BTreeSet::new(),
        };
        let garbage = {
            let mut state = self.lock()?;
            if state.environments.contains_key(child) {
                drop(state);
                self.rollback_fork(&child_root, &layer_path);
                return Err(BackendError::new(format!(
                    "child filesystem environment {child} already exists"
                )));
            }
            state.environments.insert(child.to_string(), child_environment.clone());
            state.snapshots.insert(snapshot_id.clone(), snapshot.clone());
            self.refresh_references(&mut state)?
        };
        self.schedule_gc(garbage)?;

        let durable = (|| -> Result<(), BackendError> {
            self.persist_snapshot(&snapshot)?;
            self.persist_environment(&child_environment)?;
            Ok(())
        })();
        if let Err(error) = durable {
            {
                let mut state = self.lock()?;
                state.environments.remove(child);
                let _ = self.refresh_references(&mut state);
            }
            let _ = self.remove_environment_metadata(child);
            self.remove_snapshot_metadata(&snapshot_id);
            self.rollback_fork(&child_root, &layer_path);
            return Err(BackendError::new(format!(
                "filesystem fork metadata was not durable: {error}"
            )));
        }

        let mut result = Self::snapshot_result(child_environment.snapshot_id);
        result.confirmation.snapshot_created = true;
        result.confirmation.metadata_durable = true;
        result.confirmation.refs_updated = true;
        result.confirmation.writers_quiesced = false;
        Ok(result)
    }

    pub(super) fn rollback_fork(&self, child_root: &Path, layer_path: &Path) {
        let _ = remove_path(
            &self.config.btrfs_path,
            child_root,
            self.mechanism == FilesystemMechanism::Btrfs,
        );
        let _ = remove_path(
            &self.config.btrfs_path,
            layer_path,
            self.mechanism == FilesystemMechanism::Btrfs,
        );
    }

}
