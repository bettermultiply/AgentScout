//! 三方合并与冲突检测。

use std::path::PathBuf;

use super::layer::*;
use crate::protocol::contract::FilesystemConflict;
use crate::runtime::backend::BackendError;
use super::state::MountUnit;

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

pub(crate) fn target_upper_has_changes_under(upper: &View, path: &str) -> bool {
    let prefix = format!("{path}/");
    upper.keys().any(|key| key.starts_with(&prefix))
}

#[cfg(test)]
pub(crate) fn merge_delta(
    delta: &View,
    redirects: &[(String, String)],
    base_maps: &[View],
    target_maps: &[View],
    target_upper: &View,
) -> Vec<FilesystemConflict> {
    let mut conflicts = Vec::new();
    for (path, node) in delta {
        let base = resolve_node(base_maps, path);
        let target = resolve_node(target_maps, path);
        let target_changed = !equivalent(target.as_ref(), base.as_ref());
        if node.is_whiteout() {
            if target_changed && target.is_some() {
                conflicts.push(conflict(
                    path,
                    "delete_modify",
                    base.as_ref(),
                    target.as_ref(),
                    None,
                ));
            }
            continue;
        }
        if target_changed && !equivalent(target.as_ref(), Some(node)) {
            let kind = if target.is_none() {
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
                target.as_ref(),
                Some(node),
            ));
        }
        if matches!(node.kind, NodeKind::Dir) && node.opaque
            && target_upper_has_changes_under(target_upper, path)
            && !conflicts.iter().any(|value| value.path == *path)
        {
            conflicts.push(conflict(
                path,
                "opaque_replace",
                base.as_ref(),
                target.as_ref(),
                Some(node),
            ));
        }
    }
    for (old, new) in redirects {
        let base_old = resolve_node(base_maps, old);
        let target_old = resolve_node(target_maps, old);
        let base_new = resolve_node(base_maps, new);
        let target_new = resolve_node(target_maps, new);
        if !equivalent(target_old.as_ref(), base_old.as_ref())
            || !equivalent(target_new.as_ref(), base_new.as_ref())
        {
            conflicts.push(conflict(
                new,
                "rename",
                base_new.as_ref(),
                target_new.as_ref(),
                delta.get(old),
            ));
        }
    }
    conflicts.sort_by(|left, right| left.path.cmp(&right.path));
    conflicts.dedup_by(|left, right| left.path == right.path && left.kind == right.kind);
    conflicts
}

pub(crate) fn merge_delta_layers(
    delta: &View,
    redirects: &[(String, String)],
    base_layers: &[PathBuf],
    target_mount: &MountUnit,
    target_upper: &View,
) -> Result<Vec<FilesystemConflict>, BackendError> {
    let mut conflicts = Vec::new();
    for (path, node) in delta {
        let base = resolve_path_in_layers(base_layers, target_mount, path)?;
        let ours = resolve_path_in_environment(target_mount, path)?;
        let target_changed = !equivalent(ours.as_ref(), base.as_ref());
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
        if matches!(node.kind, NodeKind::Dir) && node.opaque
            && target_upper_has_changes_under(target_upper, path)
            && !conflicts.iter().any(|value| value.path == *path)
        {
            conflicts.push(conflict(
                path,
                "opaque_replace",
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

