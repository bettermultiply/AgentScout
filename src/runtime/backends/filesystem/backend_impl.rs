//! `impl Backend for FilesystemBackend`。

use super::*;

impl FilesystemBackend {
    fn transition_state(
        &self,
        id: &str,
        from: &[EnvironmentState],
        to: EnvironmentState,
    ) -> Result<(), BackendError> {
        let (candidate, previous) = {
            let state = self.environment()?;
            let environment = state
                .environments
                .get(id)
                .ok_or_else(|| BackendError::new("unknown filesystem environment"))?;
            if !from.contains(&environment.state) {
                return Err(BackendError::new(format!(
                    "filesystem environment cannot transition from {:?} to {:?}",
                    environment.state, to
                )));
            }
            let previous = environment.state;
            let mut candidate = environment.clone();
            candidate.state = to;
            (candidate, previous)
        };
        // Persist before publishing the in-memory transition.  The Adapter
        // serializes operations per environment; if a direct backend caller
        // races, restore the old metadata and fail instead of lying.
        self.persist_environment(&candidate)?;
        let mut state = self.environment()?;
        let current = state
            .environments
            .get_mut(id)
            .ok_or_else(|| BackendError::new("unknown filesystem environment"))?;
        if current.state != previous {
            let rollback = current.clone();
            drop(state);
            let _ = self.persist_environment(&rollback);
            return Err(BackendError::new(
                "filesystem environment state changed during transition",
            ));
        }
        current.state = to;
        Ok(())
    }

}

impl Backend for FilesystemBackend {
    fn launch_fragment(&self, context:&BackendContext, launch:&crate::protocol::contract::AgentLaunch)->Result<LaunchFragment,BackendError> {
        FilesystemBackend::launch_fragment(self,context,launch)
    }
    fn save_runtime(&self, path: &Path) -> Result<(), BackendError> { self.save_recovery(path) }

    fn load_runtime(&self, path: &Path) -> Result<(), BackendError> { self.load_recovery(path) }

    fn backend_id(&self) -> &str {
        "filesystem"
    }

    fn capabilities(&self) -> RuntimeCapabilities {
        RuntimeCapabilities {
            shared_environment: false,
            branch_environment: self.usable,
            tools: Vec::new(),
            filesystem_fast_delete: Some(self.fast_delete),
            filesystem_fork_cost: Some(self.fork_cost.as_str().to_string()),
            filesystem_btrfs_incremental_diff: Some(self.btrfs_incremental_diff),
            filesystem_passthrough: Some(
                self.config
                    .passthrough_rw
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect(),
            ),
            freeze_mechanism: None,
            state_directory_overlaps_mount: Some(
                self.state_overlap_mounts()
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect(),
            ),
        }
    }

    fn establish(&self, id: &str) -> Result<(), BackendError> {
        validate_component(id)?;
        if !self.usable {
            return Err(BackendError::new(
                "no usable filesystem data plane (bubblewrap overlay probe failed)",
            ));
        }
        let root = self.paths(id);
        let scopes = vec![PathScope::Workspace, PathScope::Tmp];
        {
            let state = self.environment()?;
            if state.environments.contains_key(id) {
                return Err(BackendError::new("filesystem environment already exists"));
            }
        }
        self.ensure_state_directory_outside_isolated_mounts(self.overlayable_mounts())?;
        fs::create_dir_all(&root).map_err(error)?;
        let mut mounts = BTreeMap::new();
        for target in self.overlayable_mounts() {
            if !target.is_dir() {
                continue;
            }
            let key = target.display().to_string();
            let layer_key = if self.uses_btrfs_storage() {
                format!("mounts/{}/volume", short_hash(&key))
            } else {
                format!("mounts/{}", short_hash(&key))
            };
            mounts.insert(
                key,
                MountUnit {
                    upper: root.join(&layer_key).join("upper"),
                    work: root.join(&layer_key).join("work"),
                    target,
                    saved_base: None,
                    lower: Vec::new(),
                    layer_key,
                    volume_is_subvolume: self.uses_btrfs_storage(),
                },
            );
        }
        let units: Vec<MountUnit> = mounts.values().cloned().collect();
        if self.uses_btrfs_storage() {
            for mount in &units {
                let volume = mount.volume()?;
                if let Some(parent) = volume.parent() {
                    fs::create_dir_all(parent).map_err(error)?;
                }
                if let Err(error) = self.btrfs_create(volume) {
                    let _ = remove_path(&self.config.btrfs_path, &root, true);
                    return Err(error);
                }
            }
        }
        if let Err(error) = Self::prepare_mount_dirs(&units) {
            let _ = remove_path(
                &self.config.btrfs_path,
                &root,
                self.uses_btrfs_storage(),
            );
            return Err(error);
        }
        let environment = Environment {
            id: id.to_string(),
            root,
            state: EnvironmentState::Available,
            snapshot_id: Self::snapshot_id_for_child(id),
            parent_snapshot_id: None,
            base_snapshot_id: None,
            scopes,
            mounts,
            lower: BTreeMap::new(),
            marks: BTreeMap::new(),
            workspace_root: None,
        };
        {
            let mut state = self.environment()?;
            state
                .environments
                .insert(id.to_string(), environment.clone());
        }
        if let Err(error) = self.persist_environment(&environment) {
            let mut state = self.environment()?;
            state.environments.remove(id);
            drop(state);
            let _ = remove_path(
                &self.config.btrfs_path,
                &environment.root,
                self.uses_btrfs_storage(),
            );
            return Err(error);
        }
        Ok(())
    }

    fn branch(&self, parent: &str, child: &str) -> Result<(), BackendError> {
        let scopes = {
            let state = self.environment()?;
            Self::resolve_environment(&state, parent)?.scopes
        };
        self.fork(parent, child, &scopes).map(|_| ())
    }

    fn operate(
        &self,
        id: &str,
        command: &[String],
        cancel: &CancelHandle,
    ) -> Result<crate::protocol::contract::CommandOutput, BackendError> {
        if cancel.is_cancelled() {
            return Err(BackendError::interrupted());
        }
        if !self.usable {
            return Err(BackendError::new(
                "no usable filesystem data plane (bubblewrap overlay probe failed)",
            ));
        }
        let environment = {
            let state = self.environment()?;
            state
                .environments
                .get(id)
                .cloned()
                .ok_or_else(|| BackendError::new("unknown filesystem environment"))?
        };
        if environment.state != EnvironmentState::Available {
            return Err(BackendError::new(format!(
                "filesystem environment is not available: {:?}",
                environment.state
            )));
        }
        let mut spec = MountSpec {
            chdir: Some(PathBuf::from("/")),
            json_status_fd: None,
            ..MountSpec::default()
        };
        spec.entries.push(MountEntry::RoBind {
            source: PathBuf::from("/"),
            target: PathBuf::from("/"),
        });
        spec.entries.push(MountEntry::Proc {
            target: PathBuf::from("/proc"),
        });
        spec.entries.push(MountEntry::Dev {
            target: PathBuf::from("/dev"),
        });
        let mut mounts: Vec<MountUnit> = environment.mounts.values().cloned().collect();
        mounts.sort_by_key(|mount| mount.target.components().count());
        for mount in &mounts {
            if !mount.target.is_dir() {
                continue;
            }
            ensure_overlay_disjoint(&mount.target, &mount.upper)?;
            fs::create_dir_all(&mount.upper).map_err(error)?;
            reset_dir(&mount.work)?;
            ensure_overlay_work(&mount.upper, &mount.work)?;
            let lowers = self.launch_lowers(mount);
            spec.entries.push(MountEntry::Overlay {
                lowers,
                upper: mount.upper.clone(),
                work: mount.work.clone(),
                target: mount.target.clone(),
            });
        }
        spec.entries.push(MountEntry::Tmpfs {
            target: self.config.state_directory.clone(),
        });
        spec.entries.push(MountEntry::RemountRo {
            target: self.config.state_directory.clone(),
        });
        let mut wrapped = Command::new(&self.config.bubblewrap_path);
        spec.apply(&mut wrapped);
        wrapped.arg("--").args(command);
        let output = wrapped.output().map_err(error)?;
        Ok(crate::protocol::contract::CommandOutput {
            exit_code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    fn promote(&self, id: &str) -> Result<(), BackendError> {
        let state = self.environment()?;
        let environment = state
            .environments
            .get(id)
            .ok_or_else(|| BackendError::new("unknown filesystem environment"))?;
        if environment.state != EnvironmentState::Available {
            return Err(BackendError::new(format!(
                "filesystem environment cannot be promoted from {:?}",
                environment.state
            )));
        }
        Ok(())
    }

    fn suspend(&self, id: &str) -> Result<(), BackendError> {
        self.transition_state(
            id,
            &[EnvironmentState::Available],
            EnvironmentState::Suspended,
        )
    }

    fn resume(&self, id: &str) -> Result<(), BackendError> {
        self.transition_state(
            id,
            &[EnvironmentState::Suspended],
            EnvironmentState::Available,
        )
    }

    fn terminate(&self, id: &str) -> Result<(), BackendError> {
        self.transition_state(
            id,
            &[EnvironmentState::Available, EnvironmentState::Suspended],
            EnvironmentState::Terminated,
        )
    }

    fn cleanup(&self, id: &str) -> Result<(), BackendError> {
        let (environment, garbage) = {
            let mut state = self.environment()?;
            let environment = state
                .environments
                .get(id)
                .cloned()
                .ok_or_else(|| BackendError::new("unknown filesystem environment"))?;
            if environment.state != EnvironmentState::Terminated {
                return Err(BackendError::new(
                    "filesystem cleanup requires a terminated environment",
                ));
            }
            remove_path(
                &self.config.btrfs_path,
                &environment.root,
                self.uses_btrfs_storage(),
            )?;
            state.environments.remove(id);
            let garbage = self.refresh_references(&mut state)?;
            (environment, garbage)
        };
        self.persist_all_snapshots()?;
        self.remove_environment_metadata(&environment.id)?;
        self.schedule_gc(garbage)?;
        Ok(())
    }

    fn state(&self, id: &str) -> Option<EnvironmentState> {
        self.state.lock().ok()?.environments.get(id).map(|value| value.state)
    }

    fn environments(&self) -> Vec<String> {
        self.state
            .lock()
            .map(|state| state.environments.keys().cloned().collect())
            .unwrap_or_default()
    }

    fn filesystem_operation(
        &self,
        context: &BackendContext,
        operation: &Operation,
        cancel: &CancelHandle,
    ) -> Result<FilesystemResult, BackendError> {
        if cancel.is_cancelled() {
            return Err(BackendError::interrupted());
        }
        let name = Self::operation_name(operation);
        if let Some(record) = self.replay(&context.operation_id, name, context.generation)? {
            if record.status == "failed" {
                return Err(BackendError::new(
                    record
                        .reason
                        .unwrap_or_else(|| "filesystem operation failed".to_string()),
                ));
            }
            return Ok(record.result);
        }
        let result = match operation {
            Operation::FilesystemFork {
                parent_snapshot_id,
                child_snapshot_id,
                path_scope,
            } => self.fork(parent_snapshot_id, child_snapshot_id, path_scope),
            Operation::FilesystemMark {
                snapshot_id,
                mark_id,
                path_scope,
            } => self.mark(snapshot_id, mark_id, path_scope),
            Operation::FilesystemDiff {
                base_snapshot_id,
                source_snapshot_id,
                path_scope,
            } => self.diff(base_snapshot_id, source_snapshot_id, path_scope),
            Operation::FilesystemAccept {
                base_snapshot_id,
                source_snapshot_id,
                target_snapshot_id,
                path_scope,
            } => self.accept(
                base_snapshot_id,
                source_snapshot_id,
                target_snapshot_id,
                path_scope,
                context.target_mount_pid,
                context.target_mount_namespace,
            ),
            Operation::FilesystemDiscard {
                snapshot_id,
                path_scope,
            } => self.discard(snapshot_id, path_scope),
            _ => {
                return Err(BackendError::new(
                    "filesystem dispatch requires a filesystem operation",
                ))
            }
        };
        match result {
            Ok(result) => {
                if let Err(error) = self.remember(
                    &context.operation_id,
                    name,
                    context.generation,
                    &result,
                    None,
                ) {
                    let mut error = BackendError::new(format!(
                        "filesystem operation {name} was applied but its operation record was not persisted: {error}"
                    ));
                    error.add_cleanup_failure("filesystem", "operation record persistence");
                    return Err(error);
                }
                Ok(result)
            }
            Err(mut error) => {
                if let Err(remember) = self.remember(
                    &context.operation_id,
                    name,
                    context.generation,
                    &FilesystemResult::default(),
                    Some(error.to_string()),
                ) {
                    error.add_cleanup_failure(
                        "filesystem",
                        format!("failed operation record persistence: {remember}"),
                    );
                }
                Err(error)
            }
        }
    }
}
