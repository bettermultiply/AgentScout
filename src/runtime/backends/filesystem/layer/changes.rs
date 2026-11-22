
use std::path::Path;

use super::*;
use crate::protocol::contract::FilesystemConflict;

pub(crate) fn mount_relative_path(target: &Path, relative: &str) -> String {
    let relative = relative.trim_start_matches('/');
    let target = target.to_string_lossy();
    if target == "/" {
        format!("/{relative}")
    } else if relative.is_empty() {
        target.into_owned()
    } else {
        format!("{}/{relative}", target.trim_end_matches('/'))
    }
}

pub(crate) fn prefix_change(target: &Path, mut change: FilesystemChange) -> FilesystemChange {
    change.path = mount_relative_path(target, &change.path);
    if let Some(old) = change.old_path.take() {
        change.old_path = Some(mount_relative_path(target, &old));
    }
    change
}

pub(crate) fn prefix_conflict(
    target: &Path,
    mut conflict: FilesystemConflict,
) -> FilesystemConflict {
    conflict.path = mount_relative_path(target, &conflict.path);
    conflict
}

pub(crate) fn whiteout_node() -> Node {
    Node {
        mode: libc::S_IFCHR | 0o600,
        size: 0,
        mtime_sec: 0,
        mtime_nsec: 0,
        inode: 0,
        kind: NodeKind::Whiteout,
        source: None,
        opaque: false,
        redirect: None,
        metacopy: false,
    }
}
