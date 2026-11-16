//! accept 数据面操作。

use super::*;

impl FilesystemBackend {
    /// Three-way merge `source` into `target` using `base` as the common
    /// ancestor.  Data is staged outside the live upper and swapped only after
    /// the merge is fully materialized.
    pub fn accept(
        &self,
        base: &str,
        source: &str,
        target: &str,
        scope: &[PathScope],
        target_mount_pid: Option<u32>,
        target_mount_namespace: Option<u64>,
    ) -> Result<FilesystemResult, BackendError> {
        let scopes = normalize_scopes(scope)?;
        let target_environment = {
            let state = self.lock()?;
            Self::resolve_environment(&state, target)?
        };
        if target_environment.state != EnvironmentState::Suspended {
            return Err(BackendError::new(
                "target filesystem environment must be suspended before accept",
            ));
        }
        let safe_root = match target_mount_pid {
            Some(pid) => {
                let expected = target_mount_namespace.ok_or_else(|| {
                    BackendError::new(
                        "target environment has no recorded sandbox mount namespace; \
                         ProcessBackend must supply it from process_handle()",
                    )
                })?;
                let actual = fs::metadata(format!("/proc/{pid}/ns/mnt"))
                    .map_err(error)?
                    .ino();
                let host = fs::metadata("/proc/self/ns/mnt").map_err(error)?.ino();
                if actual == host {
                    return Err(BackendError::new(
                        "target_mount_pid is in the host mount namespace (likely the outer bwrap pid); \
                         pass the sandbox child pid from bwrap --json-status-fd",
                    ));
                }
                if actual != expected {
                    return Err(BackendError::new(
                        "target_mount_pid mount namespace does not match the recorded target environment",
                    ));
                }
                Some(SafeRoot::open(pid)?)
            }
            None => None,
        };
        // A None target_mount_pid is the caller's assertion that no process
        // currently has this environment mounted.  The filesystem backend does
        // not own process or mount-namespace liveness.

        let (base_record, source_environment) = {
            let state = self.lock()?;
            let base = Self::resolve_snapshot(&state, base)?;
            let source = Self::resolve_environment(&state, source)?;
            (base, source)
        };
        let mounts = source_environment.selected_mounts(&scopes)?;
        let mut conflicts = Vec::new();
        let mut prepared = Vec::new();
        for mount in &mounts {
            let key = mount.target.display().to_string();
            let target_mount = target_environment
                .mount_by_target(&mount.target)
                .ok_or_else(|| {
                    BackendError::new(format!(
                        "target environment does not manage mount {}",
                        mount.target.display()
                    ))
                })?;
            let base_layer = mount.layer_upper(&base_record.path);
            // Chain layout lists the base snapshot layer in the root lower
            // chain and its upper holds only this branch's changes, so the
            // cheap delta scan is both correct and sufficient.  Flatten layout
            // copies the base into the upper and must compare against the
            // whole base layer instead.
            let in_lower = mount.lower.iter().any(|layer| layer == &base_record.path);
            let (delta, redirects) = if in_lower {
                self.delta_since(&source_environment, mount, &base_record.path)?
            } else if self.mechanism == FilesystemMechanism::Btrfs && path_exists(&base_layer) {
                let (delta, redirects, _) =
                    self.btrfs_delta(&source_environment, mount, &base_record.path)?;
                (delta, redirects)
            } else {
                // Marks and empty fork bases are not in the lower chain; a
                // missing layer is an empty base view.
                self.upper_delta_from_base(mount, &base_record.path)?
            };
            let base_layers = Self::snapshot_mount_layers(&base_record, &source_environment, mount);
            conflicts.extend(
                merge_delta_layers(
                    &delta,
                    &redirects,
                    &base_layers,
                    target_mount,
                    &target_mount.upper,
                )?
                .into_iter()
                .map(|conflict| prefix_conflict(&mount.target, conflict)),
            );
            // Only rename/metacopy handling needs a source-wide view.  The
            // common modify/delete/create path replays nodes directly from the
            // delta and never scans the whole source or target upper.
            let needs_source_maps =
                !redirects.is_empty() || delta.values().any(|node| node.metacopy);
            let source_maps = if needs_source_maps {
                if mount.lower.is_empty() {
                    vec![read_layer(&mount.upper)?]
                } else {
                    let mut maps = self.read_chain(&mount.lower, mount)?;
                    maps.push(read_layer(&mount.upper)?);
                    maps
                }
            } else {
                Vec::new()
            };
            let _ = key;
            prepared.push((
                mount.clone(),
                target_mount.clone(),
                delta,
                redirects,
                source_maps,
            ));
        }
        if !conflicts.is_empty() {
            let mut result = FilesystemResult::default();
            result.conflicts = conflicts;
            return Ok(result);
        }

        let journal_root = self
            .config
            .state_directory
            .join("replay-journal")
            .join(self.token());
        #[cfg(test)]
        let fail_rollback = *self
            .replay_fail_rollback
            .lock()
            .map_err(|_| BackendError::new("rollback failure lock poisoned"))?;
        #[cfg(not(test))]
        let fail_rollback = false;
        let mut journal = ReplayJournal::new_with_rollback_failure(journal_root, fail_rollback)?;
        #[cfg(test)]
        let fail_after = *self
            .replay_fail_after
            .lock()
            .map_err(|_| BackendError::new("replay failure lock poisoned"))?;
        #[cfg(not(test))]
        let fail_after = None;
        for (mount, target_mount, delta, redirects, source_maps) in prepared {
            let replay_root = match &safe_root {
                Some(root) => {
                    let prefix = mount
                        .target
                        .strip_prefix("/")
                        .unwrap_or(&mount.target)
                        .to_string_lossy()
                        .into_owned();
                    ReplayRoot::Mount { root, prefix }
                }
                None => ReplayRoot::Upper(&target_mount.upper),
            };
            // Replay is rollback-capable but not atomic: every overwrite or
            // delete records the previous target-upper state first.  On a
            // failure all recorded operations are reversed in reverse order;
            // concurrent readers may still observe intermediate states.
            if let Err(mut error) = replay_delta(
                &replay_root,
                &delta,
                &redirects,
                &source_maps,
                &mut journal,
                &target_mount.upper,
                fail_after,
            ) {
                let (failures, warnings) = journal.rollback();
                if failures.is_empty() {
                    error
                        .message
                        .push_str("; rollback restored all recorded paths");
                } else {
                    error.message.push_str(&format!(
                        "; rollback failed for {} path(s): {}",
                        failures.len(),
                        failures.join(", ")
                    ));
                    error.add_cleanup_failure("filesystem", "accept replay rollback");
                }
                if warnings > 0 {
                    error.message.push_str(&format!(
                        "; {warnings} rollback backup(s) required copying files over 64MiB without reflink"
                    ));
                }
                return Err(error);
            }
        }
        journal.sync()?;
        journal.cleanup();

        let mut result = FilesystemResult::default();
        result.confirmation.published = true;
        result.confirmation.target_paused = false;
        result.confirmation.writers_quiesced = false;
        result.confirmation.metadata_durable = false;
        result.confirmation.refs_updated = false;
        Ok(result)
    }
}
