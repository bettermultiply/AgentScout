//! 单层与多层视图的路径解析。

use std::fs;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::{Path, PathBuf};

use super::*;
use super::super::util::*;

use super::super::state::MountUnit;

#[cfg(test)]
thread_local! {
    /// Per-thread entry counter.  `read_layer`/`walk_layer` are called from
    /// the calling thread on the accept path (the btrfs subprocess stderr
    /// drain thread does not walk layers), so a process-global counter would
    /// let unrelated tests running in parallel pollute this measurement.
    static READ_LAYER_ENTRIES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn reset_read_layer_entries() {
    READ_LAYER_ENTRIES.with(|entries| entries.set(0));
}

#[cfg(test)]
pub(crate) fn read_layer_entries() -> usize {
    READ_LAYER_ENTRIES.with(std::cell::Cell::get)
}
use crate::runtime::backend::BackendError;

pub(crate) fn node_from_metadata(path: &Path, metadata: &fs::Metadata) -> Result<Node, BackendError> {
    let file_type = metadata.file_type();
    let mode = metadata.mode();
    let mut opaque = false;
    let mut redirect = None;
    let mut metacopy = false;
    let kind = if file_type.is_char_device() && metadata.rdev() == 0 {
        NodeKind::Whiteout
    } else if has_overlay_xattr(path, "whiteout") {
        NodeKind::Whiteout
    } else if file_type.is_symlink() {
        NodeKind::Symlink(fs::read_link(path).map_err(error)?)
    } else if file_type.is_dir() {
        opaque = overlay_xattr(path, "opaque")
            .map(|value| value == b"y" || value == b"Y")
            .unwrap_or(false);
        redirect = overlay_xattr(path, "redirect")
            .and_then(|value| String::from_utf8(value).ok());
        NodeKind::Dir
    } else if file_type.is_file() {
        metacopy = has_overlay_xattr(path, "metacopy");
        NodeKind::File
    } else {
        NodeKind::Other(mode & libc::S_IFMT)
    };
    let source = matches!(kind, NodeKind::File | NodeKind::Dir | NodeKind::Symlink(_))
        .then(|| path.to_path_buf());
    Ok(Node {
        mode,
        size: metadata.len(),
        mtime_sec: metadata.mtime(),
        mtime_nsec: metadata.mtime_nsec(),
        inode: metadata.ino(),
        kind,
        source,
        opaque,
        redirect,
        metacopy,
    })
}

pub(crate) fn walk_layer(
    root: &Path,
    current: &Path,
    nodes: &mut View,
) -> Result<(), BackendError> {
    let mut entries: Vec<_> = fs::read_dir(current)
        .map_err(error)?
        .filter_map(Result::ok)
        .collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        #[cfg(test)]
        READ_LAYER_ENTRIES.with(|entries| entries.set(entries.get() + 1));
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(error)?;
        let relative = path
            .strip_prefix(root)
            .map_err(|_| BackendError::new("filesystem layer path escaped its root"))?
            .to_string_lossy()
            .into_owned();
        let file_type = metadata.file_type();
        let node = node_from_metadata(&path, &metadata)?;
        nodes.insert(relative, node);
        if file_type.is_dir() {
            walk_layer(root, &path, nodes)?;
        }
    }
    Ok(())
}

pub(crate) fn resolve_node(maps: &[View], path: &str) -> Option<Node> {
    let mut lookup = path.to_string();
    for map in maps.iter().rev() {
        if let Some(node) = map.get(&lookup) {
            return if node.is_whiteout() {
                None
            } else {
                Some(node.clone())
            };
        }
        let mut redirected = false;
        for ancestor in ancestors(&lookup) {
            if let Some(node) = map.get(&ancestor) {
                if matches!(node.kind, NodeKind::Dir) && node.opaque {
                    return None;
                }
                if let Some(target) = &node.redirect {
                    let suffix = lookup
                        .strip_prefix(&format!("{ancestor}/"))
                        .unwrap_or("");
                    lookup = if suffix.is_empty() {
                        target.clone()
                    } else {
                        format!("{target}/{suffix}")
                    };
                    redirected = true;
                    break;
                }
            }
        }
        if !redirected {
            continue;
        }
    }
    None
}

pub(crate) fn layer_node_at(root: &Path, path: &str) -> Result<Option<Node>, BackendError> {
    let target = safe_join(root, path)?;
    match fs::symlink_metadata(&target) {
        Ok(metadata) => Ok(Some(node_from_metadata(&target, &metadata)?)),
        Err(value) if value.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(value) => Err(error(value)),
    }
}

#[derive(Debug)]
pub(crate) enum PathResolution {
    Node(Node),
    Hidden,
    NotPresent,
}

pub(crate) fn resolve_path_in_upper(
    upper: &Path,
    path: &str,
) -> Result<PathResolution, BackendError> {
    let mut lookup = path.to_string();
    loop {
        if let Some(node) = layer_node_at(upper, &lookup)? {
            return if node.is_whiteout() {
                Ok(PathResolution::Hidden)
            } else {
                Ok(PathResolution::Node(node))
            };
        }
        let mut redirected = None;
        for ancestor in ancestors(&lookup) {
            if let Some(node) = layer_node_at(upper, &ancestor)? {
                if matches!(node.kind, NodeKind::Dir) && node.opaque {
                    return Ok(PathResolution::Hidden);
                }
                if let Some(target) = node.redirect {
                    let suffix = lookup
                        .strip_prefix(&format!("{ancestor}/"))
                        .unwrap_or("");
                    redirected = Some(if suffix.is_empty() {
                        target
                    } else {
                        format!("{target}/{suffix}")
                    });
                    break;
                }
            }
        }
        match redirected {
            Some(value) => lookup = value,
            None => return Ok(PathResolution::NotPresent),
        }
    }
}

pub(crate) fn resolve_path_in_layers(
    layers: &[PathBuf],
    mount: &MountUnit,
    path: &str,
) -> Result<Option<Node>, BackendError> {
    let mut lookup = path.to_string();
    for layer in layers.iter().rev() {
        let upper = mount.layer_upper(layer);
        match resolve_path_in_upper(&upper, &lookup)? {
            PathResolution::Node(node) => return Ok(Some(node)),
            PathResolution::Hidden => return Ok(None),
            PathResolution::NotPresent => {
                // A redirect may have changed the lookup path.  Recompute it
                // by asking the same probe; no additional state is needed.
                if let Some(redirected) = redirected_lookup(&upper, &lookup)? {
                    lookup = redirected;
                }
            }
        }
    }
    // The bottom of every mount-backed chain is the host mount point itself.
    match resolve_path_in_upper(mount.saved_base.as_deref().unwrap_or(&mount.target), &lookup)? {
        PathResolution::Node(node) => Ok(Some(node)),
        PathResolution::Hidden | PathResolution::NotPresent => Ok(None),
    }
}

pub(crate) fn redirected_lookup(upper: &Path, lookup: &str) -> Result<Option<String>, BackendError> {
    for ancestor in ancestors(lookup) {
        if let Some(node) = layer_node_at(upper, &ancestor)?
            && let Some(target) = node.redirect
        {
            let suffix = lookup
                .strip_prefix(&format!("{ancestor}/"))
                .unwrap_or("");
            return Ok(Some(if suffix.is_empty() {
                target
            } else {
                format!("{target}/{suffix}")
            }));
        }
    }
    Ok(None)
}

pub(crate) fn resolve_path_in_environment(
    mount: &MountUnit,
    path: &str,
) -> Result<Option<Node>, BackendError> {
    match resolve_path_in_upper(&mount.upper, path)? {
        PathResolution::Node(node) => return Ok(Some(node)),
        PathResolution::Hidden => return Ok(None),
        PathResolution::NotPresent => {}
    }
    let lookup = redirected_lookup(&mount.upper, path)?.unwrap_or_else(|| path.to_string());
    resolve_path_in_layers(&mount.lower, mount, &lookup)
}

pub(crate) fn ancestors(path: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = path;
    while let Some(index) = current.rfind('/') {
        current = &current[..index];
        if !current.is_empty() {
            result.push(current.to_string());
        }
    }
    result
}
