//! Linux file-system-only isolation for Runtime Agent processes.
//!
//! This backend deliberately does not own network, memory, resource, or Agent
//! Session state.  It owns only the branch file view and starts the Agent entry
//! point in a bubblewrap mount namespace.  The namespace is inherited by every
//! descendant process.
//!
//! Two data planes are available.  The btrfs plane snapshots the branch
//! subvolume and materializes a writable child snapshot; the overlay plane
//! seals the parent upper into an immutable layer.  Both read and write
//! OverlayFS whiteouts, opaque directories, redirects and metacopy markers
//! through the same semantic view.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, File};
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::thread;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::runtime::backend::{Backend, BackendContext, BackendError, CancelHandle};
use crate::runtime::process::{LaunchFragment, MountEntry, MountSpec};
use crate::protocol::contract::{
    EnvironmentState, FilesystemResult, Operation, PathScope,
    RuntimeCapabilities,
};


mod backend_impl;
mod btrfs;
mod config;
mod copy;
mod mounts;
mod ops;
mod probe;
mod refs;
mod state;
mod layer;
mod merge;
mod replay;
mod util;

pub use config::{FilesystemConfig, FilesystemMechanism, ForkCost};
use copy::*;
use layer::*;
use merge::*;
use replay::*;
use util::*;
use state::{
    BackendState, Environment, EnvironmentMetadata, MountUnit, OperationRecord, SnapshotMetadata,
    SnapshotRecord,
};

/// A Backend whose sole responsibility is the Agent-visible filesystem.
pub struct FilesystemBackend {
    config: FilesystemConfig,
    mechanism: FilesystemMechanism,
    /// Strict `btrfs subvolume delete` ioctl succeeded during the probe.
    /// False means discard falls back to an O(file count) background removal.
    fast_delete: bool,
    btrfs_incremental_diff: bool,
    fork_cost: ForkCost,
    usable: bool,
    state: Mutex<BackendState>,
    overlay_targets: Mutex<Option<Vec<PathBuf>>>,
    next_token: AtomicU64,
}

impl FilesystemBackend {
    pub fn new(config: FilesystemConfig) -> Result<Self, BackendError> {
        fs::create_dir_all(&config.state_directory).map_err(error)?;
        let state_directory = fs::canonicalize(&config.state_directory).map_err(error)?;
        let config = FilesystemConfig {
            state_directory,
            ..config
        };
        let probe = Self {
            config: config.clone(),
            mechanism: FilesystemMechanism::Overlay,
            fast_delete: false,
            btrfs_incremental_diff: false,
            fork_cost: ForkCost::Copy,
            usable: false,
            state: Mutex::new(BackendState {
                environments: HashMap::new(),
                snapshots: HashMap::new(),
                operations: BTreeMap::new(),
            }),
            overlay_targets: Mutex::new(None),
            next_token: AtomicU64::new(1),
        };
        let overlay_ok = probe.probe_overlay();
        let (btrfs_ok, fast_delete) = if overlay_ok && !config.force_overlay {
            probe.probe_btrfs()
        } else {
            (false, false)
        };
        // The kernel may expose find-new under some privileges, but this
        // round has not implemented a btrfs incremental diff/accept path.
        // Selecting btrfs while diff/accept are unavailable would make the
        // default mechanism unusable, so the gate stays closed until that
        // implementation exists.
        const BTRFS_INCREMENTAL_DIFF_IMPLEMENTED: bool = true;
        let kernel_incremental = btrfs_ok && probe.probe_btrfs_incremental();
        let btrfs_incremental_diff =
            BTRFS_INCREMENTAL_DIFF_IMPLEMENTED && kernel_incremental;
        if kernel_incremental && !btrfs_incremental_diff {
            eprintln!(
                "filesystem: btrfs find-new is available, but btrfs incremental diff/accept is not implemented; \
                 staying on overlay"
            );
        }
        let mechanism = if btrfs_ok && btrfs_incremental_diff {
            FilesystemMechanism::Btrfs
        } else {
            FilesystemMechanism::Overlay
        };
        if btrfs_ok && !btrfs_incremental_diff {
            eprintln!(
                "filesystem: btrfs create/snapshot works but incremental diff is unavailable; falling back to overlay"
            );
        }
        let fork_cost = if mechanism == FilesystemMechanism::Btrfs {
            ForkCost::BtrfsSnapshot
        } else if probe.probe_reflink() {
            ForkCost::Reflink
        } else {
            ForkCost::Copy
        };
        eprintln!(
            "filesystem: data plane={} fast_delete={} incremental_diff={} fork_cost={}",
            mechanism.as_str(),
            mechanism == FilesystemMechanism::Btrfs && fast_delete,
            btrfs_incremental_diff,
            fork_cost.as_str()
        );
        let backend = Self {
            config,
            mechanism,
            fast_delete,
            btrfs_incremental_diff,
            fork_cost,
            usable: overlay_ok,
            state: Mutex::new(BackendState {
                environments: HashMap::new(),
                snapshots: HashMap::new(),
                operations: BTreeMap::new(),
            }),
            overlay_targets: Mutex::new(None),
            next_token: AtomicU64::new(1),
        };
        fs::create_dir_all(backend.environments_root()).map_err(error)?;
        fs::create_dir_all(backend.snapshots_root()).map_err(error)?;
        fs::create_dir_all(backend.layers_root()).map_err(error)?;
        fs::create_dir_all(backend.trash_root()).map_err(error)?;
        fs::create_dir_all(backend.metadata_root()).map_err(error)?;
        fs::create_dir_all(backend.diffs_root()).map_err(error)?;
        backend.load_metadata()?;
        Ok(backend)
    }

    pub fn mechanism(&self) -> FilesystemMechanism {
        self.mechanism
    }

    pub fn fast_delete(&self) -> bool {
        self.fast_delete
    }

    pub fn fork_cost(&self) -> ForkCost {
        self.fork_cost
    }

    pub fn bubblewrap_path(&self) -> &Path {
        &self.config.bubblewrap_path
    }

    pub(super) fn environments_root(&self) -> PathBuf {
        self.config.state_directory.join("environments")
    }

    pub(super) fn snapshots_root(&self) -> PathBuf {
        self.config.state_directory.join("snapshots")
    }

    pub(super) fn layers_root(&self) -> PathBuf {
        self.config.state_directory.join("layers")
    }

    pub(super) fn trash_root(&self) -> PathBuf {
        self.config.state_directory.join("trash")
    }

    pub(super) fn metadata_root(&self) -> PathBuf {
        self.config.state_directory.join("metadata")
    }

    pub(super) fn environment_metadata_path(&self, id: &str) -> PathBuf {
        self.metadata_root().join("environments").join(format!("{id}.json"))
    }

    pub(super) fn snapshot_metadata_path(&self, id: &str) -> PathBuf {
        self.metadata_root().join("snapshots").join(format!("{id}.json"))
    }

    pub(super) fn operations_path(&self) -> PathBuf {
        self.metadata_root().join("operations.json")
    }

    pub(super) fn diffs_root(&self) -> PathBuf {
        self.config.state_directory.join("diffs")
    }

    pub(super) fn lock(&self) -> Result<std::sync::MutexGuard<'_, BackendState>, BackendError> {
        self.state
            .lock()
            .map_err(|_| BackendError::new("filesystem state lock failed"))
    }

    pub(super) fn token(&self) -> String {
        let value = self.next_token.fetch_add(1, Ordering::SeqCst);
        format!("{}-{value}", std::process::id())
    }

    pub(super) fn load_metadata(&self) -> Result<(), BackendError> {
        let mut state = self.lock()?;
        let environment_dir = self.metadata_root().join("environments");
        if let Ok(entries) = fs::read_dir(&environment_dir) {
            for entry in entries.flatten() {
                if entry.path().extension().and_then(|value| value.to_str()) != Some("json") {
                    continue;
                }
                let Ok(bytes) = fs::read(entry.path()) else {
                    continue;
                };
                match serde_json::from_slice::<EnvironmentMetadata>(&bytes) {
                    Ok(metadata) => {
                        let root = self.environments_root().join(&metadata.id);
                        let environment = metadata.into_environment(root).map_err(|error| {
                            BackendError::new(format!(
                                "cannot load environment metadata {}: {error}",
                                entry.path().display()
                            ))
                        })?;
                        state
                            .environments
                            .insert(environment.id.clone(), environment);
                    }
                    Err(error) => eprintln!(
                        "filesystem: ignoring unreadable environment metadata {}: {error}",
                        entry.path().display()
                    ),
                }
            }
        }
        let snapshot_dir = self.metadata_root().join("snapshots");
        if let Ok(entries) = fs::read_dir(&snapshot_dir) {
            for entry in entries.flatten() {
                if entry.path().extension().and_then(|value| value.to_str()) != Some("json") {
                    continue;
                }
                let Ok(bytes) = fs::read(entry.path()) else {
                    continue;
                };
                match serde_json::from_slice::<SnapshotMetadata>(&bytes) {
                    Ok(metadata) => {
                        let record = metadata.into_record();
                        state.snapshots.insert(record.snapshot_id.clone(), record);
                    }
                    Err(error) => eprintln!(
                        "filesystem: ignoring unreadable snapshot metadata {}: {error}",
                        entry.path().display()
                    ),
                }
            }
        }
        if let Ok(bytes) = fs::read(self.operations_path())
            && let Ok(operations) = serde_json::from_slice::<BTreeMap<String, OperationRecord>>(&bytes)
        {
            state.operations = operations;
        }
        drop(state);
        Ok(())
    }

    pub(super) fn write_json<T: Serialize>(&self, path: &Path, value: &T) -> Result<(), BackendError> {
        let parent = path
            .parent()
            .ok_or_else(|| BackendError::new("metadata path has no parent"))?;
        fs::create_dir_all(parent).map_err(error)?;
        let file_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("metadata");
        let temporary = parent.join(format!(".{file_name}.tmp"));
        {
            let bytes = serde_json::to_vec_pretty(value).map_err(error)?;
            let mut file = File::create(&temporary).map_err(error)?;
            file.write_all(&bytes).map_err(error)?;
            file.sync_all().map_err(error)?;
        }
        fs::rename(&temporary, path).map_err(error)?;
        if let Ok(directory) = File::open(parent) {
            let _ = directory.sync_all();
        }
        Ok(())
    }

    pub(super) fn persist_environment(&self, environment: &Environment) -> Result<(), BackendError> {
        self.write_json(
            &self.environment_metadata_path(&environment.id),
            &EnvironmentMetadata::from_environment(environment),
        )
    }

    pub(super) fn persist_snapshot(&self, snapshot: &SnapshotRecord) -> Result<(), BackendError> {
        self.write_json(
            &self.snapshot_metadata_path(&snapshot.snapshot_id),
            &SnapshotMetadata::from_record(snapshot),
        )
    }

    pub(super) fn persist_operations(&self, operations: &BTreeMap<String, OperationRecord>) -> Result<(), BackendError> {
        self.write_json(&self.operations_path(), operations)
    }

    pub(super) fn remove_environment_metadata(&self, id: &str) -> Result<(), BackendError> {
        let path = self.environment_metadata_path(id);
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(value) if value.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(value) => Err(error(value)),
        }
    }

    pub(super) fn remove_snapshot_metadata(&self, id: &str) {
        let _ = fs::remove_file(self.snapshot_metadata_path(id));
    }

    pub(super) fn resolve_environment(state: &BackendState, id: &str) -> Result<Environment, BackendError> {
        if let Some(environment) = state.environments.get(id) {
            return Ok(environment.clone());
        }
        state
            .environments
            .values()
            .find(|environment| environment.snapshot_id == id)
            .cloned()
            .ok_or_else(|| BackendError::new(format!("unknown filesystem environment {id}")))
    }

    pub(super) fn resolve_snapshot(
        state: &BackendState,
        id: &str,
    ) -> Result<SnapshotRecord, BackendError> {
        state
            .snapshots
            .get(id)
            .cloned()
            .ok_or_else(|| BackendError::new(format!("unknown filesystem snapshot {id}")))
    }

    pub(super) fn prepare_mount_dirs(mounts: &[MountUnit]) -> Result<(), BackendError> {
        for mount in mounts {
            fs::create_dir_all(&mount.upper).map_err(error)?;
            reset_dir(&mount.work)?;
        }
        Ok(())
    }

    /// Immutable lower chain recorded for a snapshot.  New metadata keys by
    /// absolute mount point; old metadata used `workspace`/`tmp`.
    pub(super) fn snapshot_mount_layers(
        snapshot: &SnapshotRecord,
        environment: &Environment,
        mount: &MountUnit,
    ) -> Vec<PathBuf> {
        if let Some(layers) = snapshot.lower.get(&mount.target.display().to_string()) {
            return layers.clone();
        }
        let legacy = if mount.target == Path::new("/tmp") {
            Some("tmp")
        } else if environment.workspace_root.as_deref() == Some(mount.target.as_path()) {
            Some("workspace")
        } else {
            None
        };
        legacy
            .and_then(|key| snapshot.lower.get(key))
            .cloned()
            .unwrap_or_default()
    }

    pub(super) fn snapshot_id_for_child(child: &str) -> String {
        format!("snapshot-{child}")
    }

    pub(super) fn base_snapshot_id_for_child(child: &str) -> String {
        format!("snapshot-{child}-base")
    }

    pub(super) fn mark_snapshot_id(environment: &Environment, mark_id: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(environment.snapshot_id.as_bytes());
        hasher.update(b"\0");
        hasher.update(mark_id.as_bytes());
        format!("snapshot-{}-mark-{:x}", environment.id, hasher.finalize())
    }

    pub(super) fn snapshot_result(snapshot_id: String) -> FilesystemResult {
        let mut result = FilesystemResult::default();
        result.snapshot_id = Some(snapshot_id);
        result
    }

    pub(super) fn operation_name(operation: &Operation) -> &'static str {
        match operation {
            Operation::FilesystemFork { .. } => "filesystem_fork",
            Operation::FilesystemMark { .. } => "filesystem_mark",
            Operation::FilesystemDiff { .. } => "filesystem_diff",
            Operation::FilesystemAccept { .. } => "filesystem_accept",
            Operation::FilesystemDiscard { .. } => "filesystem_discard",
            _ => "filesystem_operation",
        }
    }

    pub(super) fn replay(
        &self,
        operation_id: &str,
        operation: &str,
        generation: u64,
    ) -> Result<Option<OperationRecord>, BackendError> {
        if operation_id.is_empty() {
            return Ok(None);
        }
        let state = self.lock()?;
        let Some(record) = state.operations.get(operation_id) else {
            return Ok(None);
        };
        if record.operation != operation {
            return Err(BackendError::new(format!(
                "operation id {operation_id} was already used for {}",
                record.operation
            )));
        }
        if generation < record.generation {
            return Err(BackendError::new(format!(
                "stale generation {generation} for operation {operation_id}; latest is {}",
                record.generation
            )));
        }
        if generation == record.generation {
            return Ok(Some(record.clone()));
        }
        Ok(None)
    }

    pub(super) fn remember(
        &self,
        operation_id: &str,
        operation: &str,
        generation: u64,
        result: &FilesystemResult,
        reason: Option<String>,
    ) -> Result<(), BackendError> {
        if operation_id.is_empty() {
            return Ok(());
        }
        let status = if reason.is_some() {
            "failed"
        } else if result.conflicts.is_empty() {
            "completed"
        } else {
            "conflict"
        };
        let mut state = self.lock()?;
        state.operations.insert(
            operation_id.to_string(),
            OperationRecord {
                operation: operation.to_string(),
                generation,
                status: status.to_string(),
                result: result.clone(),
                reason: reason.clone(),
            },
        );
        self.persist_operations(&state.operations)
    }

    pub(super) fn layer_path_for(&self, snapshot_id: &str) -> PathBuf {
        match self.mechanism {
            FilesystemMechanism::Btrfs => self.snapshots_root().join(snapshot_id),
            FilesystemMechanism::Overlay => self.layers_root().join(snapshot_id),
        }
    }

    pub(super) fn environment(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, BackendState>, BackendError> {
        self.lock()
    }

    pub(super) fn paths(&self, id: &str) -> PathBuf {
        self.environments_root().join(id)
    }

}



#[cfg(test)]
mod tests;
