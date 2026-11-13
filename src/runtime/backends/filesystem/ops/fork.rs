//! fork 数据面操作。

use std::time::Instant;

use super::*;

/// Maximum number of concurrent per-mount producers.  btrfs snapshot and
/// subvolume creation serialize inside the kernel/filesystem, so more workers
/// only add contention.
const MAX_FORK_WORKERS: usize = 4;

/// Per-mount copy producer used by fork/mark.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ForkProducer {
    /// `btrfs subvolume snapshot`; fixed cost independent of entry count.
    Snapshot,
    /// reflink/copy of the upper tree; cheaper for small uppers.
    Clone,
}

/// How a fork places the parent's current upper for one mount.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ForkLayout {
    /// Add the new read-only snapshot layer S to the child lower chain and
    /// leave the child upper empty.  Cost is O(changed entries) for later
    /// diff/accept, but each such fork deepens the chain.
    Chain,
    /// Copy the parent upper into the child upper and keep the parent lower
    /// chain.  The chain does not grow, but diff/accept compare the whole
    /// upper against the snapshot layer.
    Flatten,
}

/// One step of a per-mount production chain.  Steps of one mount stay
/// sequential (a writable child snapshot needs its layer snapshot first);
/// different mounts may run concurrently.
pub(super) enum ForkTask {
    SnapshotLayer { source: PathBuf, target: PathBuf },
    SnapshotChild { source: PathBuf, target: PathBuf },
    CreateVolume { target: PathBuf },
    CloneLayer { source: PathBuf, target: PathBuf, reflink: bool },
    CloneChild { source: PathBuf, target: PathBuf, reflink: bool },
    MakeDir { path: PathBuf },
}

impl ForkTask {
    pub(super) fn phase(&self) -> &'static str {
        match self {
            ForkTask::SnapshotLayer { .. } => "snapshot_layer",
            ForkTask::SnapshotChild { .. } => "snapshot_child",
            ForkTask::CreateVolume { .. } => "create_child",
            ForkTask::CloneLayer { .. } => "clone_layer",
            ForkTask::CloneChild { .. } => "clone_child",
            ForkTask::MakeDir { .. } => "prepare_dirs",
        }
    }
}

struct ForkPlan {
    selected: Vec<MountUnit>,
    produced: BTreeMap<String, ForkProducer>,
    layouts: BTreeMap<String, ForkLayout>,
    /// View chain recorded on the snapshot: parent lower + S for every
    /// produced mount, regardless of whether the child mount itself flattens.
    snapshot_lower: BTreeMap<String, Vec<PathBuf>>,
    child_environment: Environment,
}

impl FilesystemBackend {
    pub fn fork(
        &self,
        parent: &str,
        child: &str,
        scope: &[PathScope],
    ) -> Result<FilesystemResult, BackendError> {
        let scopes = normalize_scopes(scope)?;
        validate_component(child)?;
        let mut profile = Profile::new("fork");
        let (parent_environment, existing, existing_snapshot) =
            self.load_fork_parent(parent, child, &mut profile)?;
        if let Some(result) = self.replayed_fork(
            child,
            &parent_environment,
            existing,
            existing_snapshot,
        )? {
            return Ok(result);
        }
        self.check_fork_parent(&parent_environment)?;

        let child_root = self.environments_root().join(child);
        let snapshot_id = Self::base_snapshot_id_for_child(child);
        let layer_path = self.layer_path_for(&snapshot_id);
        self.clear_orphan_fork_data(child, &child_root, &layer_path)?;

        let plan = self.plan_fork(
            &parent_environment,
            &scopes,
            child,
            &child_root,
            &layer_path,
            &mut profile,
        )?;
        let creation = match self.mechanism {
            FilesystemMechanism::Btrfs => {
                self.create_btrfs_mounts(&plan, &layer_path, &mut profile)
            }
            FilesystemMechanism::Overlay => {
                self.create_overlay_mounts(&plan, &child_root, &layer_path, &mut profile)
            }
        };
        if let Err(failure) = creation {
            self.rollback_fork(&child_root, &layer_path);
            return Err(failure);
        }
        self.commit_fork(child, &child_root, &layer_path, &plan, &mut profile)?;

        profile.finish();
        let mut result = Self::snapshot_result(plan.child_environment.snapshot_id);
        result.confirmation.snapshot_created = true;
        result.confirmation.metadata_durable = true;
        result.confirmation.refs_updated = true;
        result.confirmation.writers_quiesced = false;
        Ok(result)
    }

    fn load_fork_parent(
        &self,
        parent: &str,
        child: &str,
        profile: &mut Profile,
    ) -> Result<(Environment, Option<Environment>, Option<SnapshotRecord>), BackendError> {
        profile.run("resolve", || {
            let state = self.lock()?;
            let parent = Self::resolve_environment(&state, parent)?;
            let existing = state.environments.get(child).cloned();
            let snapshot = existing
                .as_ref()
                .and_then(|environment| environment.base_snapshot_id.as_ref())
                .and_then(|id| state.snapshots.get(id))
                .cloned();
            Ok((parent, existing, snapshot))
        })
    }

    fn replayed_fork(
        &self,
        child: &str,
        parent_environment: &Environment,
        existing: Option<Environment>,
        existing_snapshot: Option<SnapshotRecord>,
    ) -> Result<Option<FilesystemResult>, BackendError> {
        let Some(existing) = existing else {
            return Ok(None);
        };
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
        Ok(Some(result))
    }

    fn check_fork_parent(&self, parent_environment: &Environment) -> Result<(), BackendError> {
        if matches!(
            parent_environment.state,
            EnvironmentState::Available | EnvironmentState::Suspended
        ) {
            Ok(())
        } else {
            Err(BackendError::new(
                "filesystem parent must be available or suspended before fork",
            ))
        }
    }

    fn clear_orphan_fork_data(
        &self,
        child: &str,
        child_root: &Path,
        layer_path: &Path,
    ) -> Result<(), BackendError> {
        if path_exists(child_root) || path_exists(layer_path) {
            let _ = remove_path(
                &self.config.btrfs_path,
                child_root,
                self.uses_btrfs_storage(),
            );
            let _ = remove_path(
                &self.config.btrfs_path,
                layer_path,
                self.uses_btrfs_storage(),
            );
            if path_exists(child_root) || path_exists(layer_path) {
                return Err(BackendError::new(format!(
                    "filesystem data for {child} already exists without durable metadata"
                )));
            }
        }
        Ok(())
    }

    fn plan_fork(
        &self,
        parent_environment: &Environment,
        scopes: &[PathScope],
        child: &str,
        child_root: &Path,
        layer_path: &Path,
        profile: &mut Profile,
    ) -> Result<ForkPlan, BackendError> {
        let selected =
            profile.run("scan", || parent_environment.selected_mounts(scopes))?;
        // Empty uppers are not copied at all: the child mount inherits the
        // parent lower chain and starts with an empty upper.  This is the
        // dominant case for speculative branches (only the workspace mount
        // is written).
        let produced = profile.run("plan", || self.fork_producers(&selected));
        let layouts = profile.run("plan", || self.fork_layouts(&selected, &produced))?;
        let child_mounts = profile.run("plan", || {
            let mut child_mounts = BTreeMap::new();
            for mount in parent_environment.mounts.values() {
                let key = mount.target.display().to_string();
                let produced_here = produced.contains_key(&key);
                let mut lower = mount.lower.clone();
                if produced_here && layouts.get(&key) == Some(&ForkLayout::Chain) {
                    lower.push(layer_path.to_path_buf());
                }
                let unit = MountUnit {
                    target: mount.target.clone(),
                    saved_base: mount.saved_base.clone(),
                    upper: child_root.join(&mount.layer_key).join("upper"),
                    work: child_root.join(&mount.layer_key).join("work"),
                    lower,
                    layer_key: mount.layer_key.clone(),
                    volume_is_subvolume: self.mechanism == FilesystemMechanism::Btrfs
                        || (produced_here
                            && produced.get(&key) == Some(&ForkProducer::Snapshot)
                            && self.snapshot_copies),
                };
                child_mounts.insert(key, unit);
            }
            child_mounts
        });
        // The merge base must see the parent state that was frozen into S even
        // when the child mount itself uses Flatten and does not list S as an
        // overlay lower.
        let snapshot_lower: BTreeMap<String, Vec<PathBuf>> = parent_environment
            .mounts
            .iter()
            .map(|(key, mount)| {
                let mut chain = mount.lower.clone();
                if produced.contains_key(key) {
                    chain.push(layer_path.to_path_buf());
                }
                (key.clone(), chain)
            })
            .collect();
        let child_environment = Environment {
            id: child.to_string(),
            root: child_root.to_path_buf(),
            state: EnvironmentState::Available,
            snapshot_id: Self::snapshot_id_for_child(child),
            parent_snapshot_id: Some(parent_environment.snapshot_id.clone()),
            base_snapshot_id: Some(Self::base_snapshot_id_for_child(child)),
            scopes: scopes.to_vec(),
            mounts: child_mounts,
            lower: BTreeMap::new(),
            marks: BTreeMap::new(),
            workspace_root: parent_environment.workspace_root.clone(),
        };
        Ok(ForkPlan {
            selected,
            produced,
            layouts,
            snapshot_lower,
            child_environment,
        })
    }

    /// Choose the per-fork layout for every mount whose upper is non-empty.
    /// The entry count stops at the configured threshold, so the decision is
    /// O(min(entries, threshold)).
    fn fork_layouts(
        &self,
        selected: &[MountUnit],
        produced: &BTreeMap<String, ForkProducer>,
    ) -> Result<BTreeMap<String, ForkLayout>, BackendError> {
        let mut layouts = BTreeMap::new();
        for mount in selected {
            let key = mount.target.display().to_string();
            if !produced.contains_key(&key) {
                continue;
            }
            let layout = if self.config.state_directory.starts_with(&mount.target) {
                // The state directory lives below this mount; a snapshot layer
                // there cannot be a lowerdir of the same mount.
                ForkLayout::Flatten
            } else if self.config.fork_flatten_entry_limit == 0 {
                ForkLayout::Chain
            } else {
                let limit = self.config.fork_flatten_entry_limit;
                let count = upper_entry_count(&mount.upper, limit);
                if count < limit {
                    ForkLayout::Flatten
                } else {
                    ForkLayout::Chain
                }
            };
            if layout == ForkLayout::Chain && mount.lower.len() >= MAX_LOWER_DEPTH {
                return Err(BackendError::new(format!(
                    "filesystem fork would deepen the lower chain of mount {} from {} to {} layers, exceeding MAX_LOWER_DEPTH={}; refusing instead of silently switching to Flatten. Fork fewer generations or move the state directory to an independent mount",
                    mount.target.display(),
                    mount.lower.len(),
                    mount.lower.len() + 1,
                    MAX_LOWER_DEPTH
                )));
            }
            layouts.insert(key, layout);
        }
        Ok(layouts)
    }

    fn create_btrfs_mounts(
        &self,
        plan: &ForkPlan,
        layer_path: &Path,
        profile: &mut Profile,
    ) -> Result<(), BackendError> {
        // Snapshot each volume containing upper/work. Copy-up temporaries
        // must stay in the same subvolume as upper, including empty children.
        fs::create_dir_all(layer_path).map_err(error)?;
        let mut chains = Vec::new();
        let mut chain_targets = BTreeSet::new();
        for mount in &plan.selected {
            let key = mount.target.display().to_string();
            if plan.produced.get(&key) != Some(&ForkProducer::Snapshot) {
                continue;
            }
            chain_targets.insert(key.clone());
            let layer_volume = layer_path.join(&mount.layer_key);
            let child_mount = plan
                .child_environment
                .mounts
                .get(&key)
                .expect("child mount exists");
            let mut chain = vec![ForkTask::SnapshotLayer {
                source: mount.volume()?.to_path_buf(),
                target: layer_volume.clone(),
            }];
            if plan.layouts.get(&key) == Some(&ForkLayout::Flatten) {
                // Writable snapshot carries the parent state, so the layer is
                // metadata/lower only and the mount is launched with the host
                // mount as sole lower.
                chain.push(ForkTask::SnapshotChild {
                    source: layer_volume,
                    target: child_mount.volume()?.to_path_buf(),
                });
            } else {
                chain.push(ForkTask::CreateVolume {
                    target: child_mount.volume()?.to_path_buf(),
                });
            }
            chains.push(chain);
        }
        for mount in plan.child_environment.mounts.values() {
            let key = mount.target.display().to_string();
            if path_exists(&mount.upper) || chain_targets.contains(&key) {
                continue;
            }
            chains.push(vec![ForkTask::CreateVolume {
                target: mount.volume()?.to_path_buf(),
            }]);
        }
        self.run_fork_chains(profile, chains)?;
        let units: Vec<MountUnit> = plan.child_environment.mounts.values().cloned().collect();
        profile.run("prepare_dirs", || Self::prepare_mount_dirs(&units))
    }

    fn create_overlay_mounts(
        &self,
        plan: &ForkPlan,
        child_root: &Path,
        layer_path: &Path,
        profile: &mut Profile,
    ) -> Result<(), BackendError> {
        let staging = staging_path(layer_path);
        let built = (|| -> Result<(), BackendError> {
            fs::create_dir_all(&staging).map_err(error)?;
            fs::create_dir_all(child_root).map_err(error)?;
            let mut chains = Vec::new();
            let mut chain_targets = BTreeSet::new();
            for mount in &plan.selected {
                let key = mount.target.display().to_string();
                let Some(producer) = plan.produced.get(&key) else {
                    continue;
                };
                let child_mount = plan
                    .child_environment
                    .mounts
                    .get(&key)
                    .expect("child mount exists");
                let chain = self.overlay_chain(
                    mount,
                    child_mount,
                    *producer,
                    plan.layouts.get(&key).copied().unwrap_or(ForkLayout::Flatten),
                    &staging,
                    &key,
                    &mut chain_targets,
                )?;
                chains.push(chain);
            }
            for mount in plan.child_environment.mounts.values() {
                let key = mount.target.display().to_string();
                if path_exists(&mount.upper) || chain_targets.contains(&key) {
                    continue;
                }
                chains.push(vec![ForkTask::MakeDir {
                    path: mount.upper.clone(),
                }]);
            }
            self.run_fork_chains(profile, chains)?;
            let units: Vec<MountUnit> =
                plan.child_environment.mounts.values().cloned().collect();
            profile.run("prepare_dirs", || Self::prepare_mount_dirs(&units))?;
            Ok(())
        })();
        if let Err(failure) = built {
            let _ = remove_directory_force(&staging);
            return Err(failure);
        }
        profile.run("finalize", || finalize_staging(&staging, layer_path))
    }

    fn overlay_chain(
        &self,
        mount: &MountUnit,
        child_mount: &MountUnit,
        producer: ForkProducer,
        layout: ForkLayout,
        staging: &Path,
        key: &str,
        chain_targets: &mut BTreeSet<String>,
    ) -> Result<Vec<ForkTask>, BackendError> {
        match producer {
            ForkProducer::Snapshot => self.overlay_snapshot_chain(
                mount,
                child_mount,
                layout,
                staging,
                key,
                chain_targets,
            ),
            ForkProducer::Clone => self.overlay_clone_chain(
                mount,
                child_mount,
                layout,
                staging,
                key,
                chain_targets,
            ),
        }
    }

    fn overlay_snapshot_chain(
        &self,
        mount: &MountUnit,
        child_mount: &MountUnit,
        layout: ForkLayout,
        staging: &Path,
        key: &str,
        chain_targets: &mut BTreeSet<String>,
    ) -> Result<Vec<ForkTask>, BackendError> {
        let parent_volume = mount.upper.parent().ok_or_else(|| {
            BackendError::new("snapshot-copy mount upper has no volume parent")
        })?;
        let layer_volume = staging.join(&mount.layer_key);
        let child_volume = child_mount.upper.parent().ok_or_else(|| {
            BackendError::new("snapshot-copy child upper has no volume parent")
        })?;
        let mut chain = vec![ForkTask::SnapshotLayer {
            source: parent_volume.to_path_buf(),
            target: layer_volume.clone(),
        }];
        chain_targets.insert(key.to_string());
        if layout == ForkLayout::Flatten {
            // Writable snapshot carries the parent state; the read-only layer
            // remains diff metadata.
            chain.push(ForkTask::SnapshotChild {
                source: layer_volume,
                target: child_volume.to_path_buf(),
            });
        } else {
            chain.push(ForkTask::CreateVolume {
                target: child_volume.to_path_buf(),
            });
        }
        Ok(chain)
    }

    fn overlay_clone_chain(
        &self,
        mount: &MountUnit,
        child_mount: &MountUnit,
        layout: ForkLayout,
        staging: &Path,
        key: &str,
        chain_targets: &mut BTreeSet<String>,
    ) -> Result<Vec<ForkTask>, BackendError> {
        let reflink = self.reflink_available;
        let mut chain = vec![ForkTask::CloneLayer {
            source: mount.upper.clone(),
            target: mount.layer_upper(staging),
            reflink,
        }];
        if layout == ForkLayout::Flatten {
            // Flatten the parent upper into the child upper.  The layer copy
            // is metadata for diff/accept only and never joins the child
            // lower chain.
            let _ = child_mount.upper.parent().ok_or_else(|| {
                BackendError::new("snapshot-copy child upper has no volume parent")
            })?;
            chain_targets.insert(key.to_string());
            chain.push(ForkTask::CloneChild {
                source: mount.upper.clone(),
                target: child_mount.upper.clone(),
                reflink,
            });
        }
        Ok(chain)
    }

    fn commit_fork(
        &self,
        child: &str,
        child_root: &Path,
        layer_path: &Path,
        plan: &ForkPlan,
        profile: &mut Profile,
    ) -> Result<(), BackendError> {
        let snapshot_lower: BTreeMap<String, Vec<PathBuf>> = plan.snapshot_lower.clone();
        let snapshot = SnapshotRecord {
            snapshot_id: Self::base_snapshot_id_for_child(child),
            path: layer_path.to_path_buf(),
            read_only: true,
            lower: snapshot_lower,
            refs: BTreeSet::new(),
        };
        let garbage = profile.run("state_refs", || {
            let mut state = self.lock()?;
            if state.environments.contains_key(child) {
                drop(state);
                self.rollback_fork(child_root, layer_path);
                return Err(BackendError::new(format!(
                    "child filesystem environment {child} already exists"
                )));
            }
            state
                .environments
                .insert(child.to_string(), plan.child_environment.clone());
            state
                .snapshots
                .insert(snapshot.snapshot_id.clone(), snapshot.clone());
            self.refresh_references(&mut state)
        })?;
        profile.run("gc", || self.schedule_gc(garbage))?;

        let durable = profile.run("persist", || {
            // Only the new snapshot changed in this operation; persisting the
            // whole snapshot set would make fork O(historical snapshots).
            self.persist_snapshot(&snapshot)?;
            self.persist_environment(&plan.child_environment)?;
            Ok::<_, BackendError>(())
        });
        if let Err(failure) = durable {
            {
                let mut state = self.lock()?;
                state.environments.remove(child);
                let _ = self.refresh_references(&mut state);
            }
            let _ = self.remove_environment_metadata(child);
            self.remove_snapshot_metadata(&snapshot.snapshot_id);
            self.rollback_fork(child_root, layer_path);
            return Err(BackendError::new(format!(
                "filesystem fork metadata was not durable: {failure}"
            )));
        }
        Ok(())
    }

    /// Decide per selected mount whether its upper is empty, and if not which
    /// producer to use.  Counting stops at the threshold so cost is
    /// O(min(entries, threshold)).
    pub(crate) fn fork_producers(
        &self,
        selected: &[MountUnit],
    ) -> BTreeMap<String, ForkProducer> {
        let mut produced = BTreeMap::new();
        for mount in selected {
            if upper_is_empty(&mount.upper) {
                continue;
            }
            let key = mount.target.display().to_string();
            let producer = if self.mechanism == FilesystemMechanism::Btrfs {
                ForkProducer::Snapshot
            } else if self.snapshot_copies {
                if !mount.volume_is_subvolume {
                    ForkProducer::Clone
                } else {
                    let count = upper_entry_count(&mount.upper, SNAPSHOT_CLONE_ENTRY_LIMIT);
                    if count < SNAPSHOT_CLONE_ENTRY_LIMIT {
                        ForkProducer::Clone
                    } else {
                        ForkProducer::Snapshot
                    }
                }
            } else {
                ForkProducer::Clone
            };
            produced.insert(key, producer);
        }
        produced
    }

    fn run_fork_task(&self, task: &ForkTask) -> Result<(), BackendError> {
        match task {
            ForkTask::SnapshotLayer { source, target } => {
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent).map_err(error)?;
                }
                self.btrfs_snapshot(source, target, true)
            }
            ForkTask::SnapshotChild { source, target } => {
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent).map_err(error)?;
                }
                self.btrfs_snapshot(source, target, false)
            }
            ForkTask::CreateVolume { target } => {
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent).map_err(error)?;
                }
                self.btrfs_create(target)
            }
            ForkTask::CloneLayer {
                source,
                target,
                reflink,
            } => clone_upper(source, target, *reflink),
            ForkTask::CloneChild {
                source,
                target,
                reflink,
            } => clone_upper(source, target, *reflink),
            ForkTask::MakeDir { path } => fs::create_dir_all(path).map_err(error),
        }
    }

    fn run_fork_chain(
        &self,
        chain: &[ForkTask],
    ) -> (Result<(), BackendError>, Vec<(&'static str, std::time::Duration)>) {
        let mut timings = Vec::new();
        for task in chain {
            let started = Instant::now();
            let result = self.run_fork_task(task);
            if profile::enabled() {
                timings.push((task.phase(), started.elapsed()));
            }
            if let Err(failure) = result {
                return (Err(failure), timings);
            }
        }
        (Ok(()), timings)
    }

    /// Run independent per-mount chains with a bounded worker count and
    /// deterministic error selection (lowest mount index wins, matching the
    /// previous serial loop).
    pub(super) fn run_fork_chains(
        &self,
        profile: &mut Profile,
        chains: Vec<Vec<ForkTask>>,
    ) -> Result<(), BackendError> {
        if chains.is_empty() {
            return Ok(());
        }
        let worker_count = if clone_failure_requires_serial_execution() {
            1
        } else {
            chains.len().min(MAX_FORK_WORKERS)
        };
        if worker_count <= 1 {
            for chain in &chains {
                let (result, timings) = self.run_fork_chain(chain);
                for (phase, elapsed) in timings {
                    profile.add(phase, elapsed);
                }
                result?;
            }
            return Ok(());
        }
        let mut outcomes: Vec<Option<Result<(), BackendError>>> =
            (0..chains.len()).map(|_| None).collect();
        std::thread::scope(|scope| {
            let mut handles = Vec::with_capacity(worker_count);
            for worker in 0..worker_count {
                let indices: Vec<usize> = (worker..chains.len()).step_by(worker_count).collect();
                let chains = &chains;
                handles.push(scope.spawn(move || {
                    let mut results = Vec::with_capacity(indices.len());
                    for index in indices {
                        let (result, timings) = self.run_fork_chain(&chains[index]);
                        results.push((index, result, timings));
                    }
                    results
                }));
            }
            for handle in handles {
                match handle.join() {
                    Ok(results) => {
                        for (index, result, timings) in results {
                            for (phase, elapsed) in timings {
                                profile.add(phase, elapsed);
                            }
                            outcomes[index] = Some(result);
                        }
                    }
                    Err(_) => {
                        // The outcome slots for this worker stay empty and
                        // are reported as an error below.
                    }
                }
            }
        });
        for outcome in outcomes {
            match outcome {
                Some(Err(failure)) => return Err(failure),
                Some(Ok(())) => {}
                None => {
                    return Err(BackendError::new("filesystem fork producer task panicked"));
                }
            }
        }
        Ok(())
    }

    pub(super) fn rollback_fork(&self, child_root: &Path, layer_path: &Path) {
        let _ = remove_path(
            &self.config.btrfs_path,
            child_root,
            self.uses_btrfs_storage(),
        );
        let _ = remove_path(
            &self.config.btrfs_path,
            layer_path,
            self.uses_btrfs_storage(),
        );
    }
}
