//! mark 数据面操作。

use super::*;

impl FilesystemBackend {
    /// Take a read-only snapshot of the branch's current state.
    pub fn mark(
        &self,
        snapshot: &str,
        mark_id: &str,
        scope: &[PathScope],
    ) -> Result<FilesystemResult, BackendError> {
        let scopes = normalize_scopes(scope)?;
        let mut profile = Profile::new("mark");
        let (environment, existing, existing_snapshot) = profile.run("resolve", || {
            let state = self.lock()?;
            let environment = Self::resolve_environment(&state, snapshot)?;
            let existing = environment.marks.get(mark_id).cloned();
            let record = existing
                .as_ref()
                .and_then(|id| state.snapshots.get(id))
                .cloned();
            Ok::<_, BackendError>((environment, existing, record))
        })?;
        if let Some(snapshot_id) = existing {
            let verified = existing_snapshot
                .as_ref()
                .is_some_and(|record| path_exists(&record.path))
                && path_exists(&self.snapshot_metadata_path(&snapshot_id));
            if !verified {
                return Err(BackendError::new(format!(
                    "replayed mark {mark_id} could not verify the snapshot and durable metadata"
                )));
            }
            let mut result = Self::snapshot_result(snapshot_id);
            result.confirmation.snapshot_created = true;
            result.confirmation.metadata_durable = true;
            return Ok(result);
        }
        if environment.state == EnvironmentState::Terminated {
            return Err(BackendError::new(
                "terminated filesystem environment cannot be marked",
            ));
        }
        let snapshot_id = Self::mark_snapshot_id(&environment, mark_id);
        let layer_path = self.layer_path_for(&snapshot_id);
        if path_exists(&layer_path) {
            return Err(BackendError::new(format!(
                "filesystem mark snapshot {snapshot_id} already exists"
            )));
        }

        let selected = profile.run("scan", || environment.selected_mounts(&scopes))?;
        let produced = profile.run("plan", || self.fork_producers(&selected));
        let mut updated = environment.clone();
        let mut record_lower: BTreeMap<String, Vec<PathBuf>> = environment
            .mounts
            .iter()
            .map(|(key, mount)| {
                let mut chain = mount.lower.clone();
                if produced.contains_key(key) {
                    chain.push(layer_path.clone());
                }
                (key.clone(), chain)
            })
            .collect();
        for mount in &selected {
            record_lower
                .entry(mount.target.display().to_string())
                .or_default();
        }
        let created = (|| -> Result<(), BackendError> {
            match self.mechanism {
                FilesystemMechanism::Btrfs => {
                    // One read-only snapshot per non-empty selected mount
                    // upper.  The live mount and uppers are not changed.
                    fs::create_dir_all(&layer_path).map_err(error)?;
                    let mut chains = Vec::new();
                    for mount in &selected {
                        let key = mount.target.display().to_string();
                        if produced.get(&key) != Some(&ForkProducer::Snapshot) {
                            continue;
                        }
                        chains.push(vec![ForkTask::SnapshotLayer {
                            source: mount.upper.clone(),
                            target: mount.layer_upper(&layer_path),
                        }]);
                    }
                    self.run_fork_chains(&mut profile, chains)?;
                }
                FilesystemMechanism::Overlay => {
                    // Independent clone of each non-empty selected mount upper
                    // only.  Empty uppers are represented by an empty view and
                    // do not get a layer at all.
                    let staging = staging_path(&layer_path);
                    let built = (|| -> Result<(), BackendError> {
                        fs::create_dir_all(&staging).map_err(error)?;
                        let mut chains = Vec::new();
                        for mount in &selected {
                            let key = mount.target.display().to_string();
                            let Some(producer) = produced.get(&key) else {
                                continue;
                            };
                            let chain = match producer {
                                ForkProducer::Snapshot => {
                                    let parent_volume = mount.upper.parent().ok_or_else(|| {
                                        BackendError::new(
                                            "snapshot-copy mount upper has no volume parent",
                                        )
                                    })?;
                                    vec![ForkTask::SnapshotLayer {
                                        source: parent_volume.to_path_buf(),
                                        target: staging.join(&mount.layer_key),
                                    }]
                                }
                                ForkProducer::Clone => vec![ForkTask::CloneLayer {
                                    source: mount.upper.clone(),
                                    target: mount.layer_upper(&staging),
                                    reflink: self.reflink_available,
                                }],
                            };
                            chains.push(chain);
                        }
                        self.run_fork_chains(&mut profile, chains)?;
                        Ok(())
                    })();
                    if let Err(error) = built {
                        let _ = remove_directory_force(&staging);
                        return Err(error);
                    }
                    profile.run("finalize", || finalize_staging(&staging, &layer_path))?;
                }
            }
            Ok(())
        })();
        if let Err(error) = created {
            let _ = remove_path(
                &self.config.btrfs_path,
                &layer_path,
                self.uses_btrfs_storage(),
            );
            return Err(error);
        }

        let record = SnapshotRecord {
            snapshot_id: snapshot_id.clone(),
            path: layer_path,
            read_only: true,
            lower: record_lower,
            refs: BTreeSet::new(),
        };
        let garbage = profile.run("state_refs", || {
            let mut state = self.lock()?;
            state.snapshots.insert(snapshot_id.clone(), record.clone());
            updated
                .marks
                .insert(mark_id.to_string(), snapshot_id.clone());
            state.environments.insert(updated.id.clone(), updated.clone());
            self.refresh_references(&mut state)
        })?;
        let durable = profile.run("persist", || {
            // Only the new snapshot changed in this operation; persisting the
            // whole snapshot set would make mark O(historical snapshots).
            self.persist_snapshot(&record)?;
            self.persist_environment(&updated)?;
            Ok::<_, BackendError>(())
        });
        if let Err(error) = durable {
            {
                let mut state = self.lock()?;
                state.snapshots.remove(&snapshot_id);
                state
                    .environments
                    .insert(environment.id.clone(), environment.clone());
                let garbage = self.refresh_references(&mut state)?;
                let _ = self.schedule_gc(garbage);
            }
            self.remove_snapshot_metadata(&snapshot_id);
            let _ = remove_path(
                &self.config.btrfs_path,
                &record.path,
                self.uses_btrfs_storage(),
            );
            let _ = self.persist_environment(&environment);
            return Err(BackendError::new(format!(
                "filesystem mark metadata was not durable: {error}"
            )));
        }
        profile.run("gc", || self.schedule_gc(garbage))?;
        profile.finish();
        let mut result = Self::snapshot_result(snapshot_id);
        result.confirmation.snapshot_created = true;
        result.confirmation.metadata_durable = true;
        result.confirmation.refs_updated = true;
        result.confirmation.writers_quiesced = false;
        Ok(result)
    }
}
