//! fork/mark/diff/accept/discard 数据面操作。

pub(super) mod accept;
pub(super) mod diff;
pub(super) mod discard;
pub(super) mod fork;
pub(super) mod mark;

use fork::{ForkProducer, ForkTask};

use super::*;

impl FilesystemBackend {
    /// Scan only the delta layers after `base` plus the branch's own upper
    /// for one mount unit.  No file content is read and the cost is
    /// O(changed entries), not the size of the whole lineage.
    pub(super) fn delta_since(
        &self,
        environment: &Environment,
        mount: &MountUnit,
        base: &Path,
    ) -> Result<(View, Vec<(String, String)>), BackendError> {
        let layers = &mount.lower;
        let Some(index) = layers.iter().position(|layer| layer == base) else {
            return Err(BackendError::new(format!(
                "diff base {} is not an ancestor layer of environment {} mount {}",
                base.display(),
                environment.id,
                mount.target.display()
            )));
        };
        let mut view = View::new();
        let mut redirects = Vec::new();
        for layer in &layers[index + 1..] {
            redirects.extend(apply_layer(&mut view, &mount.layer_upper(layer))?);
        }
        redirects.extend(apply_layer(&mut view, &mount.upper)?);
        Ok((view, redirects))
    }

    /// Materialize the metadata maps for one mount's layer chain, bottom to
    /// top.  This is O(entries in the chain), never O(file contents).
    pub(super) fn read_chain(
        &self,
        layers: &[PathBuf],
        mount: &MountUnit,
    ) -> Result<Vec<BTreeMap<String, Node>>, BackendError> {
        let mut maps = Vec::with_capacity(layers.len());
        for layer in layers {
            maps.push(read_layer(&mount.layer_upper(layer))?);
        }
        Ok(maps)
    }

    /// Compare the live upper with one base layer copy.  Used for marks and
    /// for fork bases whose upper was empty (the layer does not exist).
    pub(super) fn upper_delta_from_base(
        &self,
        mount: &MountUnit,
        base: &Path,
    ) -> Result<(View, Vec<(String, String)>), BackendError> {
        let current = read_layer(&mount.upper)?;
        let base_view = read_layer_if_exists(&mount.layer_upper(base))?;
        let changes = changes_from_upper_copy(&current, &base_view);
        let mut delta = View::new();
        let mut redirects = Vec::new();
        for change in changes {
            match change.kind.as_str() {
                "delete" => {
                    delta.insert(change.path.clone(), whiteout_node());
                }
                "rename" => {
                    if let Some(old) = change.old_path.clone() {
                        delta.insert(old, whiteout_node());
                    }
                    if let Some(node) = layer_node_at(&mount.upper, &change.path)? {
                        if let Some(redirect) = node.redirect.clone() {
                            redirects.push((redirect, change.path.clone()));
                        }
                        delta.insert(change.path.clone(), node);
                    }
                }
                _ => {
                    if let Some(node) = layer_node_at(&mount.upper, &change.path)? {
                        if node.is_whiteout() {
                            delta.insert(change.path.clone(), whiteout_node());
                        } else {
                            if let Some(redirect) = node.redirect.clone() {
                                redirects.push((redirect, change.path.clone()));
                            }
                            delta.insert(change.path.clone(), node);
                        }
                    } else {
                        delta.insert(change.path.clone(), whiteout_node());
                    }
                }
            }
        }
        Ok((delta, redirects))
    }

}
