
use std::path::Path;

use super::super::util::*;
use super::*;

use crate::runtime::backend::BackendError;

pub(crate) fn apply_layer(
    view: &mut View,
    layer: &Path,
) -> Result<Vec<(String, String)>, BackendError> {
    if !path_exists(layer) {
        return Ok(Vec::new());
    }
    let nodes = read_layer(layer)?;
    let mut renames = Vec::new();
    for (path, node) in nodes {
        match node.kind {
            NodeKind::Whiteout => {
                remove_subtree(view, &path);
                view.insert(path, node);
            }
            NodeKind::Dir => {
                if node.opaque {
                    remove_children(view, &path);
                }
                if let Some(target) = node.redirect.clone() {
                    let target = normalize_relative_path(&target)?;
                    if target != path {
                        renames.push((target, path.clone()));
                    }
                }
                view.insert(path, node);
            }
            _ => {
                view.insert(path, node);
            }
        }
    }
    Ok(renames)
}

pub(crate) fn read_layer(root: &Path) -> Result<View, BackendError> {
    let mut nodes = View::new();
    walk_layer(root, root, &mut nodes)
        .map_err(|value| BackendError::new(format!("read layer {}: {value}", root.display())))?;
    Ok(nodes)
}

/// A missing layer represents an upper that was empty at snapshot time.
pub(crate) fn read_layer_if_exists(root: &Path) -> Result<View, BackendError> {
    if !path_exists(root) {
        return Ok(View::new());
    }
    read_layer(root)
}
