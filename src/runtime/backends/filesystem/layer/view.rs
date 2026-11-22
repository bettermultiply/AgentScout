
use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Node {
    pub(crate) mode: u32,
    pub(crate) size: u64,
    pub(crate) mtime_sec: i64,
    pub(crate) mtime_nsec: i64,
    pub(crate) inode: u64,
    pub(crate) kind: NodeKind,
    pub(crate) source: Option<PathBuf>,
    pub(crate) opaque: bool,
    pub(crate) redirect: Option<String>,
    pub(crate) metacopy: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum NodeKind {
    Dir,
    File,
    Symlink(PathBuf),
    Whiteout,
    Other(u32),
}

impl Node {
    pub(crate) fn is_whiteout(&self) -> bool {
        matches!(self.kind, NodeKind::Whiteout)
    }
}

pub(crate) type View = BTreeMap<String, Node>;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct FilesystemChange {
    pub(crate) path: String,
    pub(crate) kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) old_path: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct DiffDocument {
    pub(crate) base: String,
    pub(crate) source: String,
    pub(crate) changes: Vec<FilesystemChange>,
}

pub(crate) fn equivalent(left: Option<&Node>, right: Option<&Node>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => {
            // Child changes already have their own paths. Ignore parent size and mtime.
            let content_equivalent = (left.kind == NodeKind::Dir && right.kind == NodeKind::Dir)
                || (left.size == right.size
                    && left.mtime_sec == right.mtime_sec
                    && left.mtime_nsec == right.mtime_nsec);
            left.mode == right.mode
                && content_equivalent
                && left.kind == right.kind
                && left.opaque == right.opaque
                && left.redirect == right.redirect
                && left.metacopy == right.metacopy
        }
        _ => false,
    }
}
