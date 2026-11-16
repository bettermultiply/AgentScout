//! 从 delta / upper 副本生成 FilesystemChange。

use super::*;

pub(crate) fn changes_from_delta(
    delta: &View,
    redirects: &[(String, String)],
) -> Vec<FilesystemChange> {
    let mut changes = Vec::new();
    for (path, node) in delta {
        if node.is_whiteout() {
            changes.push(FilesystemChange {
                path: path.clone(),
                kind: "delete".to_string(),
                old_path: None,
            });
            continue;
        }
        // Phase 2 will add a structured changeset with create/modify
        // classification.  Phase 1 only records that this upper entry changed;
        // it deliberately does not touch base content to stay O(upper delta).
        changes.push(FilesystemChange {
            path: path.clone(),
            kind: "change".to_string(),
            old_path: None,
        });
    }
    for (old, new) in redirects {
        changes.push(FilesystemChange {
            path: new.clone(),
            kind: "rename".to_string(),
            old_path: Some(old.clone()),
        });
    }
    changes.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then_with(|| left.kind.cmp(&right.kind))
    });
    changes.dedup_by(|left, right| left.path == right.path && left.kind == right.kind);
    changes
}

pub(crate) fn changes_from_upper_copy(current: &View, mark: &View) -> Vec<FilesystemChange> {
    let mut changes = Vec::new();
    let mut redirects = Vec::new();
    for (path, node) in current {
        if node.is_whiteout() {
            changes.push(FilesystemChange {
                path: path.clone(),
                kind: "delete".to_string(),
                old_path: None,
            });
            continue;
        }
        if let Some(target) = &node.redirect {
            redirects.push((target.clone(), path.clone()));
        }
        if !equivalent(mark.get(path), Some(node)) {
            changes.push(FilesystemChange {
                path: path.clone(),
                kind: "change".to_string(),
                old_path: None,
            });
        }
    }
    for path in mark.keys() {
        if !current.contains_key(path) {
            changes.push(FilesystemChange {
                path: path.clone(),
                kind: "delete".to_string(),
                old_path: None,
            });
        }
    }
    for (old, new) in redirects {
        changes.push(FilesystemChange {
            path: new,
            kind: "rename".to_string(),
            old_path: Some(old),
        });
    }
    changes.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then_with(|| left.kind.cmp(&right.kind))
    });
    changes.dedup_by(|left, right| left.path == right.path && left.kind == right.kind);
    changes
}

pub(crate) fn node_summary(node: Option<&Node>) -> String {
    match node {
        None => "absent".to_string(),
        Some(node) => match &node.kind {
            NodeKind::Whiteout => "whiteout".to_string(),
            NodeKind::Dir => "dir".to_string(),
            NodeKind::File => format!(
                "file size={} mtime={}.{}",
                node.size, node.mtime_sec, node.mtime_nsec
            ),
            NodeKind::Symlink(target) => format!("symlink:{}", target.display()),
            NodeKind::Other(kind) => format!("special:0o{kind:o}"),
        },
    }
}
