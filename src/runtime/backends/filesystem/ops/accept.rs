//! accept 数据面操作。

use super::*;

fn regular_file(node: &Node) -> Result<(), BackendError> {
    if node.kind == NodeKind::File && !node.metacopy && !node.opaque
        && node.redirect.is_none() && node.mode & 0o7000 == 0
        && let Some(path) = &node.source
        && fs::symlink_metadata(path).map_err(error)?.nlink() == 1
    {
        Ok(())
    } else {
        Err(BackendError::new("selective acceptance requires a plain regular file"))
    }
}

fn same_file(left: Option<&Node>, right: &Node) -> Result<bool, BackendError> {
    use std::io::{BufRead, BufReader};
    let Some(left) = left else { return Ok(false) };
    if left.kind != right.kind || left.mode != right.mode || left.size != right.size {
        return Ok(false);
    }
    let mut left = BufReader::new(fs::File::open(left.source.as_ref().unwrap()).map_err(error)?);
    let mut right = BufReader::new(fs::File::open(right.source.as_ref().unwrap()).map_err(error)?);
    loop {
        let a = left.fill_buf().map_err(error)?;
        let b = right.fill_buf().map_err(error)?;
        let count = a.len().min(b.len());
        if count == 0 {
            return Ok(a.is_empty() && b.is_empty());
        }
        if a[..count] != b[..count] {
            return Ok(false);
        }
        left.consume(count);
        right.consume(count);
    }
}

fn selected_file_delta(
    paths: &[String],
    base_layers: &[PathBuf],
    source: &MountUnit,
    target: &MountUnit,
) -> Result<(View, Vec<crate::protocol::contract::FilesystemConflict>), BackendError> {
    let mut delta = View::new();
    let mut conflicts = Vec::new();
    for path in paths {
        for parent in ancestors(path) {
            for entry in [resolve_path_in_layers(base_layers, source, &parent)?,
                resolve_path_in_environment(source, &parent)?,
                resolve_path_in_environment(target, &parent)?] {
                if !entry.is_some_and(|node| node.kind == NodeKind::Dir && !node.opaque && node.redirect.is_none()) {
                    return Err(BackendError::new(format!("unsupported parent directory for {path}: {parent}")));
                }
            }
        }
        let base = resolve_path_in_layers(base_layers, source, path)?;
        let ours = resolve_path_in_environment(target, path)?;
        let theirs = resolve_path_in_environment(source, path)?.ok_or_else(|| {
            BackendError::new(format!("selected source file missing: {path}"))
        })?;
        for node in base.iter().chain(ours.iter()).chain(std::iter::once(&theirs)) {
            regular_file(node).map_err(|error| BackendError::new(format!("{path}: {error}")))?;
        }
        if same_file(ours.as_ref(), &theirs)? {
            continue;
        }
        let target_matches_base = match &base {
            Some(base) => same_file(ours.as_ref(), base)?,
            None => ours.is_none(),
        };
        if !target_matches_base {
            conflicts.push(crate::protocol::contract::FilesystemConflict {
                path: path.clone(),
                kind: "content".into(),
                reason: "selected file differs from both fork base and candidate".into(),
                ..Default::default()
            });
        } else {
            delta.insert(path.clone(), theirs);
        }
    }
    Ok((delta, conflicts))
}

impl FilesystemBackend {
    pub(in crate::runtime::backends::filesystem) fn accept_environment_state(
        &self,
        target: &BackendContext,
        source: &str,
        full: bool,
        part: Option<&crate::protocol::contract::PartSelection>,
    ) -> Result<FilesystemResult, BackendError> {
        use crate::protocol::contract::{Acceptance, PartSelection};
        let source_environment = Self::resolve_environment(&*self.lock()?, source)?;
        let base = source_environment
            .base_snapshot_id
            .as_deref()
            .ok_or_else(|| BackendError::new("source has no fork base"))?;
        self.wait_for_holder(&target.environment_id)?;
        self.wait_for_holder(source)?;
        let handle = self.process.process_handle(&target.environment_id);
        if self.process.is_running(&target.environment_id)? && handle.is_none() {
            return Err(BackendError::new("target mount namespace is unavailable"));
        }
        let merge = |scopes: &[PathScope]| {
            self.accept(
                base,
                source,
                &target.environment_id,
                scopes,
                handle.as_ref().map(|h| h.inner_pid),
                handle.as_ref().map(|h| h.mount_namespace),
            )
        };
        let selective = |files: &[String], preflight| {
            self.accept_selection(
                base,
                source,
                &target.environment_id,
                &[],
                handle.as_ref().map(|h| (h.inner_pid, h.mount_namespace)),
                Some(files),
                preflight,
            )
        };
        if let Some(PartSelection::Results { results }) = part {
            if !PartSelection::valid_results(results) {
                return Err(BackendError::new("invalid part result dependencies"));
            }
            // Check every selection before replay, including a fallback from full.
            for result in results.iter().filter(|result| !result.files.is_empty()) {
                selective(&result.files, true)?;
            }
        }
        let mut report = FilesystemResult::default();
        if full {
            match merge(&[]) {
                Ok(mut result) if result.confirmation.published => {
                    result.acceptance = Some(Acceptance::Full);
                    return Ok(result);
                }
                Ok(result) => report = result,
                Err(error) if part.is_some() && !error.is_partial() => {
                    report
                        .conflicts
                        .push(crate::protocol::contract::FilesystemConflict {
                            kind: "accept".into(),
                            reason: error.to_string(),
                            ..Default::default()
                        });
                }
                Err(error) => return Err(error),
            }
        }
        let scopes = match part {
            None => return Ok(report),
            Some(PartSelection::Scopes(scopes)) => scopes,
            Some(PartSelection::Results { results }) => {
                let mut committed = Vec::new();
                for result in results {
                    if result.files.is_empty() {
                        committed.push(result.id.clone());
                        continue;
                    }
                    match selective(&result.files, false) {
                        Ok(receipt) if receipt.confirmation.published => {
                            report.confirmation.published = true;
                            committed.push(result.id.clone());
                        }
                        Ok(receipt) => report.conflicts.extend(receipt.conflicts),
                        Err(error) if !error.is_partial() => report.conflicts.push(
                            crate::protocol::contract::FilesystemConflict {
                                kind: "accept".into(),
                                reason: format!("result {}: {error}", result.id),
                                ..Default::default()
                            },
                        ),
                        Err(error) => return Err(error),
                    }
                }
                if !committed.is_empty() {
                    report.acceptance = Some(Acceptance::PartResults { results: committed });
                }
                return Ok(report);
            }
        };
        let mut committed = Vec::new();
        for scope in scopes.iter().copied() {
            if committed.contains(&scope) {
                continue;
            }
            if source_environment
                .scope_target(scope)
                .and_then(|path| source_environment.mount_by_target(&path))
                .is_none()
            {
                continue;
            }
            match merge(&[scope]) {
                Ok(result) if result.confirmation.published => committed.push(scope),
                Ok(result) => report.conflicts.extend(result.conflicts),
                Err(error) if error.cleanup_failures.is_empty() => {
                    report
                        .conflicts
                        .push(crate::protocol::contract::FilesystemConflict {
                            kind: "accept".into(),
                            reason: error.to_string(),
                            ..Default::default()
                        });
                }
                Err(error) => return Err(error),
            }
        }
        if scopes.is_empty() || !committed.is_empty() {
            report.confirmation.published = !committed.is_empty();
            report.acceptance = Some(Acceptance::Part { scopes: committed });
        }
        Ok(report)
    }

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
        let mount = match (target_mount_pid, target_mount_namespace) {
            (Some(pid), Some(namespace)) => Some((pid, namespace)),
            (None, _) => None,
            _ => return Err(BackendError::new("target environment has no recorded sandbox mount namespace")),
        };
        self.accept_selection(base, source, target, scope, mount, None, false)
    }

    fn accept_selection(
        &self,
        base: &str,
        source: &str,
        target: &str,
        scope: &[PathScope],
        target_mount: Option<(u32, u64)>,
        files: Option<&[String]>,
        preflight: bool,
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
        let safe_root = match target_mount {
            Some((pid, expected)) => {
                let actual = fs::metadata(format!("/proc/{pid}/ns/mnt"))
                    .map_err(error)?
                    .ino();
                let host = fs::metadata("/proc/self/ns/mnt").map_err(error)?.ino();
                if actual == host {
                    return Err(BackendError::new(
                        "target_mount_pid is in the host mount namespace. \
                         Pass the recorded namespace holder PID",
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
        let mut selected_paths = BTreeMap::<PathBuf, Vec<String>>::new();
        let mounts = if let Some(files) = files {
            for file in files {
                let path = Path::new(file);
                let source_mount = source_environment.mount_for_path(path);
                let target_mount = target_environment.mount_for_path(path);
                let mount = source_mount.filter(|source| {
                    target_mount.is_some_and(|target| target.target == source.target)
                }).ok_or_else(|| BackendError::new(format!("unmatched managed mount for {file}")))?;
                let relative = path.strip_prefix(&mount.target).unwrap().to_str().unwrap();
                if relative.is_empty() {
                    return Err(BackendError::new(format!("cannot accept a mount point as a file: {file}")));
                }
                selected_paths.entry(mount.target.clone()).or_default().push(relative.into());
            }
            selected_paths.keys().map(|path| source_environment.mount_by_target(path).unwrap().clone()).collect()
        } else {
            source_environment.selected_mounts(&scopes)?
        };
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
            let base_layers = Self::snapshot_mount_layers(&base_record, &source_environment, mount);
            if let Some(paths) = selected_paths.get(&mount.target) {
                let (delta, file_conflicts) = selected_file_delta(paths, &base_layers, mount, target_mount)?;
                conflicts.extend(file_conflicts.into_iter().map(|conflict| prefix_conflict(&mount.target, conflict)));
                prepared.push((mount.clone(), target_mount.clone(), delta, Vec::new(), Vec::new()));
                continue;
            }
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
        if preflight || !conflicts.is_empty() {
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
