//! fork/mark/diff/accept/discard 数据面操作。

pub(super) mod accept;
pub(super) mod diff;
pub(super) mod discard;
pub(super) mod fork;
pub(super) mod mark;

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

}
