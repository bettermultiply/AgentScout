
use std::fs;
use std::path::{Path, PathBuf};

use super::layer::*;
use super::state::MountUnit;
use crate::protocol::contract::FilesystemConflict;
use crate::runtime::backend::BackendError;

pub(crate) fn conflict(
    path: &str,
    kind: &str,
    base: Option<&Node>,
    ours: Option<&Node>,
    theirs: Option<&Node>,
) -> FilesystemConflict {
    FilesystemConflict {
        path: path.to_string(),
        kind: kind.to_string(),
        reason: "target and source changed the same path differently".to_string(),
        base: Some(node_summary(base)),
        ours: Some(node_summary(ours)),
        theirs: Some(node_summary(theirs)),
    }
}

pub(crate) fn target_upper_path_has_changes(upper: &Path, path: &str) -> bool {
    let target = upper.join(path);
    match fs::read_dir(&target) {
        Ok(mut entries) => entries.next().is_some(),
        Err(_) => false,
    }
}

pub(crate) fn merge_delta_layers(
    delta: &View,
    redirects: &[(String, String)],
    base_layers: &[PathBuf],
    target_mount: &MountUnit,
    target_upper: &Path,
) -> Result<Vec<FilesystemConflict>, BackendError> {
    let mut conflicts = Vec::new();
    for (path, node) in delta {
        let base = resolve_path_in_layers(base_layers, target_mount, path)?;
        let ours = resolve_path_in_environment(target_mount, path)?;
        let target_changed = !equivalent(ours.as_ref(), base.as_ref());
        let replaces_directory = ours.as_ref().is_some_and(|ours| ours.kind == NodeKind::Dir)
            && (node.kind != NodeKind::Dir || node.opaque);
        if replaces_directory && (target_upper_path_has_changes(target_upper, path)
            || target_mount.lower.iter().map(|layer| target_mount.layer_upper(layer))
                .filter(|upper| !base_layers.contains(upper))
                .any(|upper| target_upper_path_has_changes(&upper, path)))
        {
            let kind = if node.opaque { "opaque_replace" } else if node.is_whiteout() { "delete_modify" } else { "content" };
            conflicts.push(conflict(path, kind, base.as_ref(), ours.as_ref(), (!node.is_whiteout()).then_some(node)));
            continue;
        }
        if node.is_whiteout() {
            if target_changed && ours.is_some() {
                conflicts.push(conflict(
                    path,
                    "delete_modify",
                    base.as_ref(),
                    ours.as_ref(),
                    None,
                ));
            }
            continue;
        }
        if target_changed && !equivalent(ours.as_ref(), Some(node)) {
            let kind = if ours.is_none() {
                "modify_delete"
            } else if base.is_none() {
                "add_add"
            } else {
                "content"
            };
            conflicts.push(conflict(
                path,
                kind,
                base.as_ref(),
                ours.as_ref(),
                Some(node),
            ));
        }
    }
    for (old, new) in redirects {
        let base_old = resolve_path_in_layers(base_layers, target_mount, old)?;
        let ours_old = resolve_path_in_environment(target_mount, old)?;
        let base_new = resolve_path_in_layers(base_layers, target_mount, new)?;
        let ours_new = resolve_path_in_environment(target_mount, new)?;
        if !equivalent(ours_old.as_ref(), base_old.as_ref())
            || !equivalent(ours_new.as_ref(), base_new.as_ref())
        {
            conflicts.push(conflict(
                new,
                "rename",
                base_new.as_ref(),
                ours_new.as_ref(),
                delta.get(old),
            ));
        }
    }
    conflicts.sort_by(|left, right| left.path.cmp(&right.path));
    conflicts.dedup_by(|left, right| left.path == right.path && left.kind == right.kind);
    Ok(conflicts)
}

pub(crate) fn source_content_source(maps: &[View], path: &str) -> Option<PathBuf> {
    for map in maps.iter().rev() {
        if let Some(node) = map.get(path) {
            if node.is_whiteout() {
                return None;
            }
            if node.metacopy {
                continue;
            }
            return node.source.clone();
        }
    }
    None
}
