
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::protocol::contract::{EnvironmentState, FilesystemResult, PathScope};
use crate::runtime::backend::BackendError;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct MountUnit {
    pub(crate) target: PathBuf,
    /// Host lower and publication target. None uses the mount target itself.
    #[serde(default)]
    pub(crate) base: Option<PathBuf>,
    #[serde(default)]
    pub(crate) saved_base: Option<PathBuf>,
    pub(crate) upper: PathBuf,
    pub(crate) work: PathBuf,
    /// Immutable lowers in bottom-to-top order, each containing `layer_key/upper`.
    #[serde(default)]
    pub(crate) lower: Vec<PathBuf>,
    /// New snapshots use `mounts/<hash>/volume`; migrated keys may be `workspace` or `tmp`.
    #[serde(default = "default_layer_key")]
    pub(crate) layer_key: String,
    /// Empty child mounts may be plain directories and need cloning on the next fork.
    #[serde(default = "default_volume_is_subvolume")]
    pub(crate) volume_is_subvolume: bool,
}

fn default_volume_is_subvolume() -> bool {
    true
}

fn default_layer_key() -> String {
    "mounts/unknown".to_string()
}

impl MountUnit {
    pub(crate) fn volume(&self) -> Result<&Path, BackendError> {
        self.upper
            .parent()
            .ok_or_else(|| BackendError::new("mount upper has no volume parent"))
    }

    pub(crate) fn layer_upper(&self, layer: &Path) -> PathBuf {
        layer.join(&self.layer_key).join("upper")
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Environment {
    pub(crate) id: String,
    pub(crate) root: PathBuf,
    pub(crate) state: EnvironmentState,
    pub(crate) snapshot_id: String,
    pub(crate) parent_snapshot_id: Option<String>,
    pub(crate) base_snapshot_id: Option<String>,
    /// An empty filter or both legacy scopes select every mount.
    pub(crate) scopes: Vec<PathScope>,
    pub(crate) mounts: BTreeMap<String, MountUnit>,
    pub(crate) marks: BTreeMap<String, String>,
    pub(crate) workspace_root: Option<PathBuf>,
}

impl Environment {
    pub(crate) fn mount_for_path(&self, path: &Path) -> Option<&MountUnit> {
        self.mounts
            .values()
            .filter(|mount| path.starts_with(&mount.target))
            .max_by_key(|mount| mount.target.as_os_str().len())
    }

    pub(crate) fn mount_by_target(&self, target: &Path) -> Option<&MountUnit> {
        self.mounts.get(&target.display().to_string())
    }

    pub(crate) fn scope_target(&self, scope: PathScope) -> Option<PathBuf> {
        match scope {
            PathScope::Workspace => self
                .workspace_root
                .as_deref()
                .and_then(|path| self.mount_for_path(path))
                .map(|mount| mount.target.clone()),
            PathScope::Tmp => self
                .mount_for_path(Path::new("/tmp"))
                .or_else(|| self.mount_by_target(Path::new("/tmp")))
                .map(|mount| mount.target.clone()),
        }
    }

    pub(crate) fn selected_mounts(
        &self,
        scopes: &[PathScope],
    ) -> Result<Vec<MountUnit>, BackendError> {
        if self.mounts.is_empty() {
            return Err(BackendError::new(
                "filesystem environment has no recorded mount units; launch it once or recreate it",
            ));
        }
        let all = scopes.is_empty()
            || (scopes.contains(&PathScope::Workspace) && scopes.contains(&PathScope::Tmp));
        if all {
            let mut mounts: Vec<_> = self.mounts.values().cloned().collect();
            mounts.sort_by(|left, right| left.target.cmp(&right.target));
            return Ok(mounts);
        }
        let mut mounts = Vec::new();
        for scope in scopes {
            let target = self.scope_target(*scope);
            if let Some(target) = target
                && let Some(mount) = self.mount_by_target(&target)
                && !mounts
                    .iter()
                    .any(|unit: &MountUnit| unit.target == mount.target)
            {
                mounts.push(mount.clone());
            }
        }
        if mounts.is_empty() {
            // An unresolved filter selects all mounts to avoid losing requested writes.
            let mut mounts: Vec<_> = self.mounts.values().cloned().collect();
            mounts.sort_by(|left, right| left.target.cmp(&right.target));
            return Ok(mounts);
        }
        mounts.sort_by(|left, right| left.target.cmp(&right.target));
        Ok(mounts)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SnapshotRecord {
    pub(crate) snapshot_id: String,
    pub(crate) path: PathBuf,
    pub(crate) read_only: bool,
    pub(crate) lower: BTreeMap<String, Vec<PathBuf>>,
    pub(crate) refs: BTreeSet<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct EnvironmentMetadata {
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) root: Option<PathBuf>,
    pub(crate) snapshot_id: String,
    pub(crate) parent_snapshot_id: Option<String>,
    pub(crate) base_snapshot_id: Option<String>,
    pub(crate) state: EnvironmentState,
    #[serde(default)]
    pub(crate) scopes: Vec<PathScope>,
    #[serde(default)]
    pub(crate) mounts: BTreeMap<String, MountUnit>,
    #[serde(default)]
    pub(crate) lower: BTreeMap<String, Vec<PathBuf>>,
    #[serde(default)]
    pub(crate) marks: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) workspace_root: Option<PathBuf>,
}

impl EnvironmentMetadata {
    pub(crate) fn from_environment(environment: &Environment) -> Self {
        Self {
            id: environment.id.clone(),
            root: Some(environment.root.clone()),
            snapshot_id: environment.snapshot_id.clone(),
            parent_snapshot_id: environment.parent_snapshot_id.clone(),
            base_snapshot_id: environment.base_snapshot_id.clone(),
            state: environment.state,
            scopes: environment.scopes.clone(),
            mounts: environment.mounts.clone(),
            lower: BTreeMap::new(),
            marks: environment.marks.clone(),
            workspace_root: environment.workspace_root.clone(),
        }
    }

    pub(crate) fn into_environment(self, root: PathBuf) -> Result<Environment, BackendError> {
        let root = self.root.unwrap_or(root);
        let mut mounts = self.mounts;
        if mounts.is_empty() && !self.lower.is_empty() {
            // Legacy workspace layers need a recorded launch root for migration.
            for (key, layers) in &self.lower {
                let target = match key.as_str() {
                    "workspace" => self.workspace_root.clone().ok_or_else(|| {
                        BackendError::new(format!(
                            "legacy filesystem environment {} has workspace layers but no workspace_root;                              cannot migrate scope metadata to mount units",
                            self.id
                        ))
                    })?,
                    "tmp" => PathBuf::from("/tmp"),
                    absolute if absolute.starts_with('/') => PathBuf::from(absolute),
                    other => {
                        return Err(BackendError::new(format!(
                            "legacy filesystem environment {} has unknown scope key {other:?}",
                            self.id
                        )))
                    }
                };
                mounts.insert(
                    target.display().to_string(),
                    MountUnit {
                        upper: root.join(key).join("upper"),
                        work: root.join(key).join("work"),
                        target: target.clone(),
                        base: None,
                        saved_base: None,
                        lower: layers.clone(),
                        layer_key: key.clone(),
                        volume_is_subvolume: true,
                    },
                );
            }
        }
        Ok(Environment {
            id: self.id,
            root,
            state: self.state,
            snapshot_id: self.snapshot_id,
            parent_snapshot_id: self.parent_snapshot_id,
            base_snapshot_id: self.base_snapshot_id,
            scopes: self.scopes,
            mounts,
            marks: self.marks,
            workspace_root: self.workspace_root,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SnapshotMetadata {
    pub(crate) snapshot_id: String,
    pub(crate) path: PathBuf,
    pub(crate) read_only: bool,
    #[serde(default)]
    pub(crate) lower: BTreeMap<String, Vec<PathBuf>>,
    #[serde(default)]
    pub(crate) refs: BTreeSet<String>,
}

impl SnapshotMetadata {
    pub(crate) fn from_record(record: &SnapshotRecord) -> Self {
        Self {
            snapshot_id: record.snapshot_id.clone(),
            path: record.path.clone(),
            read_only: record.read_only,
            lower: record.lower.clone(),
            refs: record.refs.clone(),
        }
    }

    pub(crate) fn into_record(self) -> SnapshotRecord {
        SnapshotRecord {
            snapshot_id: self.snapshot_id,
            path: self.path,
            read_only: self.read_only,
            lower: self.lower,
            refs: self.refs,
        }
    }
}

/// Reuse the durable result for the same Adapter operation ID and generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct OperationRecord {
    pub(crate) operation: String,
    pub(crate) generation: u64,
    pub(crate) status: String,
    pub(crate) result: FilesystemResult,
    pub(crate) reason: Option<String>,
}

pub(crate) struct BackendState {
    pub(crate) environments: HashMap<String, Environment>,
    pub(crate) snapshots: HashMap<String, SnapshotRecord>,
    pub(crate) operations: BTreeMap<String, OperationRecord>,
}
