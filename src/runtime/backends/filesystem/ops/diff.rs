//! diff 数据面操作。

use super::*;

impl FilesystemBackend {
    /// Compare the branch's own delta with the speculation snapshot it is
    /// based on.  Only delta layers after `base` plus the branch upper are
    /// scanned; no file content is read.
    pub fn diff(
        &self,
        base: &str,
        source: &str,
        target: Option<&str>,
        scope: &[PathScope],
    ) -> Result<FilesystemResult, BackendError> {
        let scopes = normalize_scopes(scope)?;
        if target.is_some() {
            return Err(BackendError::new(
                "filesystem diff target filtering is not implemented (Phase 2)",
            ));
        }
        let (base_record, source_environment) = {
            let state = self.lock()?;
            let base = Self::resolve_snapshot(&state, base)?;
            let source = Self::resolve_environment(&state, source)?;
            (base, source)
        };
        let mounts = source_environment.selected_mounts(&scopes)?;
        let mut changes = Vec::new();
        for mount in &mounts {
            let in_lower = !self.mount_uses_flattening(mount)
                && mount.lower.iter().any(|layer| layer == &base_record.path);
            let mount_changes = if in_lower {
                let (delta, redirects) =
                    self.delta_since(&source_environment, mount, &base_record.path)?;
                changes_from_delta(&delta, &redirects)
            } else if self.mechanism == FilesystemMechanism::Overlay {
                // A mark is an independent copy of the upper, not a layer in
                // the live lower chain.  Compare the branch's own upper with
                // that copy; cost is O(upper entries), never the lineage.
                let current = read_layer(&mount.upper)?;
                let mark = read_layer(&mount.layer_upper(&base_record.path))?;
                changes_from_upper_copy(&current, &mark)
            } else {
                let (_delta, _redirects, raw) =
                    self.btrfs_delta(&source_environment, mount, &base_record.path)?;
                raw
            };
            changes.extend(
                mount_changes
                    .into_iter()
                    .map(|change| prefix_change(&mount.target, change)),
            );
        }
        changes.sort_by(|left, right| left.path.cmp(&right.path));
        changes.dedup_by(|left, right| left.path == right.path && left.kind == right.kind);
        let document = DiffDocument {
            base: base.to_string(),
            source: source.to_string(),
            changes,
        };
        let path = self.diffs_root().join(format!(
            "diff-{}-{}-{}.json",
            std::process::id(),
            self.token(),
            now_nanos()
        ));
        self.write_json(&path, &document)?;
        let mut result = FilesystemResult::default();
        result.snapshot_id = Some(source_environment.snapshot_id);
        result.diff_ref = Some(path.display().to_string());
        result.confirmation.metadata_durable = true;
        Ok(result)
    }

}
