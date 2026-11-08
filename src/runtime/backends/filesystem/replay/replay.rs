//! 将 delta 重放到目标 upper 或 live mount，并回滚已创建项。

use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::Path;

use super::super::layer::*;
use super::super::util::*;
use super::super::merge::source_content_source;
use super::super::copy::copy_file;
use super::safe_root::SafeRoot;
use crate::runtime::backend::BackendError;

pub(crate) fn remove_visible(path: &Path) -> Result<(), BackendError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => remove_directory_force(path),
        Ok(_) => {
            fs::remove_file(path).map_err(error)?;
            Ok(())
        }
        Err(value) if value.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(value) => Err(error(value)),
    }
}

pub(crate) enum ReplayRoot<'a> {
    Mount { root: &'a SafeRoot, prefix: String },
    Upper(&'a Path),
}


pub(crate) enum AppliedOp {
    CreatedFile(String),
    CreatedDir(String),
    CreatedSymlink(String),
    Other,
}

pub(crate) fn replay_delta(
    root: &ReplayRoot<'_>,
    delta: &View,
    redirects: &[(String, String)],
    source_maps: &[View],
) -> Result<(), BackendError> {
    let mut log: Vec<AppliedOp> = Vec::new();
    let mut non_reversible = 0usize;
    let mut apply = |path: &str, node: &Node| -> Result<(), BackendError> {
        match replay_node(root, path, node, source_maps) {
            Ok(AppliedOp::Other) => {
                non_reversible += 1;
                Ok(())
            }
            Ok(op) => {
                log.push(op);
                Ok(())
            }
            Err(value) => {
                let mut rollback_failures = 0;
                for op in log.iter().rev() {
                    let path = match op {
                        AppliedOp::CreatedFile(path)
                        | AppliedOp::CreatedDir(path)
                        | AppliedOp::CreatedSymlink(path) => path,
                        AppliedOp::Other => continue,
                    };
                    let result = match root {
                        ReplayRoot::Mount { root, prefix } => {
                            root.remove(&format!("{prefix}/{path}"))
                        }
                        ReplayRoot::Upper(upper) => {
                            let target = upper.join(path);
                            remove_directory_force(&target)
                        }
                    };
                    if result.is_err() {
                        rollback_failures += 1;
                    }
                }
                return Err(BackendError::new(format!(
                    "replay failed at {path}: {value}; rolled back creations with {rollback_failures} failure(s); \
                     {non_reversible} applied operation(s) are not reversible"
                )));
            }
        }
    };
    let mut paths: Vec<(&String, &Node)> = delta.iter().collect();
    paths.sort_by_key(|(path, _)| path.len());
    for (path, node) in paths {
        apply(path, node)?;
    }
    for (old, new) in redirects {
        apply(
            old,
            &Node {
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
            },
        )?;
        let mut renamed: BTreeSet<String> = BTreeSet::new();
        let old_prefix = format!("{old}/");
        let new_prefix = format!("{new}/");
        for map in source_maps {
            for key in map.keys() {
                if key == old {
                    renamed.insert(new.clone());
                } else if key.starts_with(&old_prefix) {
                    renamed.insert(format!("{new_prefix}{}", &key[old_prefix.len()..]));
                } else if key == new || key.starts_with(&new_prefix) {
                    renamed.insert(key.clone());
                }
            }
        }
        let mut renamed: Vec<String> = renamed.into_iter().collect();
        renamed.sort_by_key(|path| path.len());
        for path in renamed {
            if let Some(node) = resolve_node(source_maps, &path) {
                apply(&path, &node)?;
            }
        }
    }
    Ok(())
}

pub(crate) fn replay_node(
    root: &ReplayRoot<'_>,
    path: &str,
    node: &Node,
    source_maps: &[View],
) -> Result<AppliedOp, BackendError> {
    match root {
        ReplayRoot::Upper(upper) => {
            let target = safe_join(upper, path)?;
            let existed = path_exists(&target);
            if node.opaque {
                let _ = remove_directory_force(&target);
                return Ok(AppliedOp::Other);
            }
            stage_node(upper, path, node, source_maps)?;
            match &node.kind {
                NodeKind::Whiteout | NodeKind::Other(_) => Ok(AppliedOp::Other),
                NodeKind::Dir if !existed => Ok(AppliedOp::CreatedDir(path.to_string())),
                NodeKind::File if !existed => Ok(AppliedOp::CreatedFile(path.to_string())),
                NodeKind::Symlink(_) if !existed => {
                    Ok(AppliedOp::CreatedSymlink(path.to_string()))
                }
                _ => Ok(AppliedOp::Other),
            }
        }
        ReplayRoot::Mount { root, prefix } => {
            let target = format!("{prefix}/{path}");
            let existed = root.exists(&target)?;
            match &node.kind {
                NodeKind::Whiteout => {
                    root.remove(&target)?;
                    Ok(AppliedOp::Other)
                }
                NodeKind::Dir => {
                    if node.opaque {
                        root.remove(&target)?;
                        root.mkdir(&target, node.mode)?;
                        Ok(AppliedOp::Other)
                    } else {
                        root.mkdir(&target, node.mode)?;
                        if existed {
                            Ok(AppliedOp::Other)
                        } else {
                            Ok(AppliedOp::CreatedDir(path.to_string()))
                        }
                    }
                }
                NodeKind::File => {
                    let source = if node.metacopy {
                        source_content_source(source_maps, path)
                    } else {
                        node.source.clone()
                    }
                    .ok_or_else(|| BackendError::new(format!("no content source for {path}")))?;
                    root.write_file(&target, &source, node.mode)?;
                    if existed {
                        Ok(AppliedOp::Other)
                    } else {
                        Ok(AppliedOp::CreatedFile(path.to_string()))
                    }
                }
                NodeKind::Symlink(link) => {
                    if existed {
                        root.remove(&target)?;
                    }
                    root.symlink(&target, link)?;
                    if existed {
                        Ok(AppliedOp::Other)
                    } else {
                        Ok(AppliedOp::CreatedSymlink(path.to_string()))
                    }
                }
                NodeKind::Other(kind) => Err(BackendError::new(format!(
                    "cannot replay special file {path} (mode 0o{kind:o})"
                ))),
            }
        }
    }
}

pub(crate) fn stage_node(
    root: &Path,
    path: &str,
    node: &Node,
    source_maps: &[View],
) -> Result<(), BackendError> {
    let target = safe_join(root, path)?;
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(error)?;
    }
    match &node.kind {
        NodeKind::Whiteout => create_whiteout(root, path),
        NodeKind::Dir => {
            if let Ok(metadata) = fs::symlink_metadata(&target)
                && !metadata.is_dir()
            {
                remove_visible(&target)?;
            }
            fs::create_dir_all(&target).map_err(error)?;
            if node.opaque {
                set_xattr(&target, "user.overlay.opaque", b"y")?;
            }
            Ok(())
        }
        NodeKind::File => {
            let source = if node.metacopy {
                source_content_source(source_maps, path)
            } else {
                node.source.clone()
            }
            .ok_or_else(|| BackendError::new(format!("no content source for {path}")))?;
            if let Ok(metadata) = fs::symlink_metadata(&target)
                && !metadata.file_type().is_file()
            {
                remove_visible(&target)?;
            }
            copy_file(&source, &target)?;
            fs::set_permissions(&target, fs::Permissions::from_mode(node.mode & 0o7777))
                .map_err(error)?;
            Ok(())
        }
        NodeKind::Symlink(link) => {
            let _ = fs::remove_file(&target);
            symlink(link, &target).map_err(error)?;
            Ok(())
        }
        NodeKind::Other(kind) => Err(BackendError::new(format!(
            "cannot stage special file {path} (mode 0o{kind:o})"
        ))),
    }
}

