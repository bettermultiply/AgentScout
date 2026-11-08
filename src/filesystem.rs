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
use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt, symlink};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::thread;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::backend::{Backend, BackendContext, BackendError, CancelHandle};
use crate::process::{LaunchFragment, MountEntry, MountSpec};
use crate::contract::{
    EnvironmentState, FilesystemConflict, FilesystemResult, Operation, PathScope,
    RuntimeCapabilities,
};

#[derive(Debug, Clone)]
pub struct FilesystemConfig {
    pub state_directory: PathBuf,
    pub bubblewrap_path: PathBuf,
    pub btrfs_path: PathBuf,
    /// Force the portable overlay data plane even when btrfs is available.
    /// Used by tests that exercise overlay semantics on a btrfs host.
    pub force_overlay: bool,
    /// Paths explicitly allowed to pass through read-write to the host.
    pub passthrough_rw: Vec<PathBuf>,
    /// Make unmanaged writable mounts read-only inside the sandbox.
    pub isolate_unmanaged_mounts: bool,
}

impl FilesystemConfig {
    pub fn new(state_directory: impl Into<PathBuf>) -> Self {
        Self {
            state_directory: state_directory.into(),
            bubblewrap_path: PathBuf::from("bwrap"),
            btrfs_path: PathBuf::from("btrfs"),
            force_overlay: false,
            passthrough_rw: Vec::new(),
            isolate_unmanaged_mounts: true,
        }
    }
}

/// The data plane selected for this backend instance.  The btrfs plane is used
/// only when its snapshot/delete probe succeeds; otherwise the portable
/// overlay plane is used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilesystemMechanism {
    Btrfs,
    Overlay,
}

impl FilesystemMechanism {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Btrfs => "btrfs",
            Self::Overlay => "overlay",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForkCost {
    Reflink,
    Copy,
    BtrfsSnapshot,
}

impl ForkCost {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Reflink => "reflink",
            Self::Copy => "copy",
            Self::BtrfsSnapshot => "btrfs_snapshot",
        }
    }
}

/// One overlay mount unit.  The isolation unit is the filesystem mount point,
/// not a fixed `Workspace`/`Tmp` scope: every mounted filesystem that can be
/// overlaid gets its own upper, work and immutable lower chain.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct MountUnit {
    /// Canonical absolute mount point, for example `/home` or `/var/tmp`.
    target: PathBuf,
    /// Writable upper tree for this environment and mount.
    upper: PathBuf,
    /// Overlay work tree for this environment and mount.
    work: PathBuf,
    /// Immutable lower layers, bottom to top.  Each layer is a snapshot layer
    /// root; the mount's subtree inside it is `layer_key/upper`.
    #[serde(default)]
    lower: Vec<PathBuf>,
    /// Relative directory inside a snapshot layer root that holds this
    /// mount's upper tree.  New layers use `mounts/<hash>`; migrated metadata
    /// may still use `workspace` or `tmp`.
    #[serde(default = "default_layer_key")]
    layer_key: String,
}

fn default_layer_key() -> String {
    "mounts/unknown".to_string()
}

impl MountUnit {
    fn layer_upper(&self, layer: &Path) -> PathBuf {
        layer.join(&self.layer_key).join("upper")
    }
}

#[derive(Clone, Debug)]
struct Environment {
    id: String,
    root: PathBuf,
    state: EnvironmentState,
    snapshot_id: String,
    parent_snapshot_id: Option<String>,
    /// Read-only speculation point that is the three-way merge base.
    base_snapshot_id: Option<String>,
    /// Optional request filter retained for contract compatibility.  Data
    /// plane operations default to every mount when the filter is empty or
    /// contains both legacy scopes.
    scopes: Vec<PathScope>,
    /// Authoritative per-mount overlay units, keyed by normalized absolute
    /// mount point.
    mounts: BTreeMap<String, MountUnit>,
    /// Legacy metadata from the scope-based layout.  Migrated into `mounts`
    /// while loading; never used by new data plane operations.
    #[allow(dead_code)]
    lower: BTreeMap<String, Vec<PathBuf>>,
    /// Mark identifier to immutable snapshot identifier.
    marks: BTreeMap<String, String>,
    /// Absolute host workspace path observed at Agent launch.
    workspace_root: Option<PathBuf>,
}

impl Environment {
    fn mount_for_path(&self, path: &Path) -> Option<&MountUnit> {
        // Prefer the longest matching mount point so nested mounts win over
        // their parent.
        self.mounts
            .values()
            .filter(|mount| path.starts_with(&mount.target))
            .max_by_key(|mount| mount.target.as_os_str().len())
    }

    fn mount_by_target(&self, target: &Path) -> Option<&MountUnit> {
        self.mounts.get(&target.display().to_string())
    }

    fn scope_target(&self, scope: PathScope) -> Option<PathBuf> {
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

    fn selected_mounts(&self, scopes: &[PathScope]) -> Result<Vec<MountUnit>, BackendError> {
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
                && !mounts.iter().any(|unit: &MountUnit| unit.target == mount.target)
            {
                mounts.push(mount.clone());
            }
        }
        if mounts.is_empty() {
            // A request filter that cannot be resolved before launch is a
            // superset operation rather than an error: selecting all mounts
            // never loses a requested write.
            let mut mounts: Vec<_> = self.mounts.values().cloned().collect();
            mounts.sort_by(|left, right| left.target.cmp(&right.target));
            return Ok(mounts);
        }
        mounts.sort_by(|left, right| left.target.cmp(&right.target));
        Ok(mounts)
    }
}

#[derive(Clone, Debug)]
struct SnapshotRecord {
    snapshot_id: String,
    path: PathBuf,
    read_only: bool,
    /// Layer chain whose view this snapshot represents.  The snapshot path is
    /// the top delta layer; the chain below it supplies inherited state.
    lower: BTreeMap<String, Vec<PathBuf>>,
    /// Logical owners: environment base/lower references and mark identifiers.
    refs: BTreeSet<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EnvironmentMetadata {
    id: String,
    snapshot_id: String,
    parent_snapshot_id: Option<String>,
    base_snapshot_id: Option<String>,
    state: EnvironmentState,
    #[serde(default)]
    scopes: Vec<PathScope>,
    #[serde(default)]
    mounts: BTreeMap<String, MountUnit>,
    #[serde(default)]
    lower: BTreeMap<String, Vec<PathBuf>>,
    #[serde(default)]
    marks: BTreeMap<String, String>,
    #[serde(default)]
    workspace_root: Option<PathBuf>,
}

impl EnvironmentMetadata {
    fn from_environment(environment: &Environment) -> Self {
        Self {
            id: environment.id.clone(),
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

    fn into_environment(self, root: PathBuf) -> Result<Environment, BackendError> {
        let mut mounts = self.mounts;
        if mounts.is_empty() && !self.lower.is_empty() {
            // Explicit one-way migration from the scope layout.  A workspace
            // layer without a recorded launch root cannot be mapped to an
            // absolute mount point; refuse instead of silently dropping data.
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
                        lower: layers.clone(),
                        layer_key: key.clone(),
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
            lower: self.lower,
            marks: self.marks,
            workspace_root: self.workspace_root,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SnapshotMetadata {
    snapshot_id: String,
    path: PathBuf,
    read_only: bool,
    #[serde(default)]
    lower: BTreeMap<String, Vec<PathBuf>>,
    #[serde(default)]
    refs: BTreeSet<String>,
}

impl SnapshotMetadata {
    fn from_record(record: &SnapshotRecord) -> Self {
        Self {
            snapshot_id: record.snapshot_id.clone(),
            path: record.path.clone(),
            read_only: record.read_only,
            lower: record.lower.clone(),
            refs: record.refs.clone(),
        }
    }

    fn into_record(self) -> SnapshotRecord {
        SnapshotRecord {
            snapshot_id: self.snapshot_id,
            path: self.path,
            read_only: self.read_only,
            lower: self.lower,
            refs: self.refs,
        }
    }
}

/// Durable result of one filesystem operation.  The identifier is the
/// Adapter-supplied operation id; a replay with the same generation returns
/// this record instead of executing the operation again.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct OperationRecord {
    operation: String,
    generation: u64,
    status: String,
    result: FilesystemResult,
    reason: Option<String>,
}

struct BackendState {
    environments: HashMap<String, Environment>,
    snapshots: HashMap<String, SnapshotRecord>,
    operations: BTreeMap<String, OperationRecord>,
}


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

    fn environments_root(&self) -> PathBuf {
        self.config.state_directory.join("environments")
    }

    fn snapshots_root(&self) -> PathBuf {
        self.config.state_directory.join("snapshots")
    }

    fn layers_root(&self) -> PathBuf {
        self.config.state_directory.join("layers")
    }

    fn trash_root(&self) -> PathBuf {
        self.config.state_directory.join("trash")
    }

    fn metadata_root(&self) -> PathBuf {
        self.config.state_directory.join("metadata")
    }

    fn environment_metadata_path(&self, id: &str) -> PathBuf {
        self.metadata_root().join("environments").join(format!("{id}.json"))
    }

    fn snapshot_metadata_path(&self, id: &str) -> PathBuf {
        self.metadata_root().join("snapshots").join(format!("{id}.json"))
    }

    fn operations_path(&self) -> PathBuf {
        self.metadata_root().join("operations.json")
    }

    fn diffs_root(&self) -> PathBuf {
        self.config.state_directory.join("diffs")
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, BackendState>, BackendError> {
        self.state
            .lock()
            .map_err(|_| BackendError::new("filesystem state lock failed"))
    }

    fn token(&self) -> String {
        let value = self.next_token.fetch_add(1, Ordering::SeqCst);
        format!("{}-{value}", std::process::id())
    }

    fn load_metadata(&self) -> Result<(), BackendError> {
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

    fn write_json<T: Serialize>(&self, path: &Path, value: &T) -> Result<(), BackendError> {
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

    fn persist_environment(&self, environment: &Environment) -> Result<(), BackendError> {
        self.write_json(
            &self.environment_metadata_path(&environment.id),
            &EnvironmentMetadata::from_environment(environment),
        )
    }

    fn persist_snapshot(&self, snapshot: &SnapshotRecord) -> Result<(), BackendError> {
        self.write_json(
            &self.snapshot_metadata_path(&snapshot.snapshot_id),
            &SnapshotMetadata::from_record(snapshot),
        )
    }

    fn persist_operations(&self, operations: &BTreeMap<String, OperationRecord>) -> Result<(), BackendError> {
        self.write_json(&self.operations_path(), operations)
    }

    fn remove_environment_metadata(&self, id: &str) -> Result<(), BackendError> {
        let path = self.environment_metadata_path(id);
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(value) if value.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(value) => Err(error(value)),
        }
    }

    fn remove_snapshot_metadata(&self, id: &str) {
        let _ = fs::remove_file(self.snapshot_metadata_path(id));
    }

    /// Rebuild snapshot reference tokens from the current environment graph.
    /// Unreferenced snapshots are removed from memory and scheduled for GC.
    fn refresh_references(
        &self,
        state: &mut BackendState,
    ) -> Result<Vec<(String, PathBuf)>, BackendError> {
        let path_to_id: HashMap<PathBuf, String> = state
            .snapshots
            .values()
            .map(|snapshot| (snapshot.path.clone(), snapshot.snapshot_id.clone()))
            .collect();
        let mut tokens = Vec::new();
        for environment in state.environments.values() {
            if let Some(base) = &environment.base_snapshot_id {
                tokens.push((base.clone(), format!("env:{}:base", environment.id)));
            }
            for (target, mount) in &environment.mounts {
                for layer in &mount.lower {
                    if let Some(id) = path_to_id.get(layer) {
                        tokens.push((
                            id.clone(),
                            format!("env:{}:lower:{target}", environment.id),
                        ));
                    }
                }
            }
            for (mark, snapshot) in &environment.marks {
                tokens.push((
                    snapshot.clone(),
                    format!("env:{}:mark:{mark}", environment.id),
                ));
            }
        }
        for snapshot in state.snapshots.values_mut() {
            snapshot.refs.clear();
        }
        for (id, token) in tokens {
            if let Some(snapshot) = state.snapshots.get_mut(&id) {
                snapshot.refs.insert(token);
            }
        }
        let garbage: Vec<(String, PathBuf)> = state
            .snapshots
            .values()
            .filter(|snapshot| snapshot.refs.is_empty())
            .map(|snapshot| (snapshot.snapshot_id.clone(), snapshot.path.clone()))
            .collect();
        for (id, _) in &garbage {
            state.snapshots.remove(id);
            self.remove_snapshot_metadata(id);
        }
        for snapshot in state.snapshots.values() {
            self.persist_snapshot(snapshot)?;
        }
        Ok(garbage)
    }

    fn schedule_gc(&self, paths: Vec<(String, PathBuf)>) -> Result<(), BackendError> {
        for (_, path) in paths {
            self.retire_path(&path)?;
        }
        Ok(())
    }

    /// Move a path out of the reachable namespace in one rename.  Actual
    /// reclamation happens in a background thread.  This is the hot path of
    /// discard; without the btrfs delete ioctl the background pass is
    /// O(file count), but the caller does not wait for it.
    fn retire_path(&self, path: &Path) -> Result<(), BackendError> {
        self.retire_path_in(path, &self.trash_root())
    }

    fn retire_path_in(&self, path: &Path, trash: &Path) -> Result<(), BackendError> {
        if !path_exists(path) {
            return Ok(());
        }
        if self.mechanism == FilesystemMechanism::Btrfs {
            let _ = Command::new(&self.config.btrfs_path)
                .args([
                    std::ffi::OsStr::new("property"),
                    std::ffi::OsStr::new("set"),
                    path.as_os_str(),
                    std::ffi::OsStr::new("ro"),
                    std::ffi::OsStr::new("false"),
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        let token = self.token();
        fs::create_dir_all(trash).map_err(error)?;
        let name = path
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_else(|| "retired".to_string());
        let destination = trash.join(format!("{token}-{name}"));
        fs::rename(path, &destination).map_err(error)?;
        self.delete_in_background(destination);
        Ok(())
    }

    fn delete_in_background(&self, path: PathBuf) {
        let btrfs = self.mechanism == FilesystemMechanism::Btrfs;
        let fast_delete = self.fast_delete;
        let btrfs_path = self.config.btrfs_path.clone();
        thread::spawn(move || {
            if let Err(error) = remove_path_with_mode(&btrfs_path, &path, btrfs, fast_delete) {
                eprintln!(
                    "filesystem: background removal of {} failed: {error}",
                    path.display()
                );
            }
        });
    }

    fn probe_overlay(&self) -> bool {
        let root = self.config.state_directory.join(".probe-overlay");
        let _ = remove_directory_force(&root);
        let base = root.join("base");
        let upper = root.join("upper");
        let work = root.join("work");
        if fs::create_dir_all(&base)
            .and_then(|_| fs::create_dir_all(&upper))
            .and_then(|_| fs::create_dir_all(&work))
            .is_err()
        {
            let _ = remove_directory_force(&root);
            return false;
        }
        let command = if Path::new("/usr/bin/true").exists() {
            "/usr/bin/true"
        } else {
            "/bin/true"
        };
        let spec = MountSpec {
            entries: vec![
                MountEntry::RoBind {
                    source: PathBuf::from("/"),
                    target: PathBuf::from("/"),
                },
                MountEntry::Proc {
                    target: PathBuf::from("/proc"),
                },
                MountEntry::Dev {
                    target: PathBuf::from("/dev"),
                },
                MountEntry::Overlay {
                    lowers: vec![base.clone()],
                    upper: upper.clone(),
                    work: work.clone(),
                    target: base.clone(),
                },
            ],
            chdir: Some(PathBuf::from("/")),
            json_status_fd: None,
        };
        let mut probe = Command::new(&self.config.bubblewrap_path);
        spec.apply(&mut probe);
        let status = probe
            .arg("--")
            .arg(command)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = remove_directory_force(&root);
        matches!(status, Ok(status) if status.success())
    }

    fn probe_reflink(&self) -> bool {
        let root = self.config.state_directory.join(".probe-reflink");
        let _ = fs::remove_dir_all(&root);
        if fs::create_dir_all(&root).is_err() {
            return false;
        }
        let source = root.join("source");
        let target = root.join("target");
        let result = fs::write(&source, b"ascout reflink probe")
            .and_then(|_| reflink_file(&source, &target))
            .is_ok();
        let _ = fs::remove_dir_all(&root);
        result
    }

    /// Probe btrfs snapshot creation.  `fast_delete` reports whether the
    /// strict subvolume-delete ioctl works; without it discard still retires
    /// data by rename and removes it in the background with a fallback.
    fn probe_btrfs(&self) -> (bool, bool) {
        let root = self.config.state_directory.join(".probe-btrfs");
        if path_exists(&root) {
            self.cleanup_btrfs_probe(&root);
            if path_exists(&root) {
                return (false, false);
            }
        }
        if fs::create_dir_all(&root).is_err() {
            return (false, false);
        }
        let subvolume = root.join("sub");
        let snapshot = root.join("snap");
        let created = self.btrfs_create(&subvolume).is_ok();
        let snapshotted = created
            && self
                .btrfs_snapshot(&subvolume, &snapshot, true)
                .is_ok();
        let fast_delete = snapshotted && self.btrfs_delete_strict(&snapshot).is_ok();
        self.cleanup_btrfs_probe(&root);
        (created && snapshotted, fast_delete)
    }

    fn probe_btrfs_incremental(&self) -> bool {
        let root = self.config.state_directory.join(".probe-btrfs-diff");
        let _ = remove_path(&self.config.btrfs_path, &root, true);
        if fs::create_dir_all(&root).is_err() {
            return false;
        }
        let subvolume = root.join("sub");
        let first = root.join("first");
        let second = root.join("second");
        let ok = (|| -> Result<bool, BackendError> {
            self.btrfs_create(&subvolume)?;
            fs::write(subvolume.join("base"), b"base").map_err(error)?;
            self.btrfs_snapshot(&subvolume, &first, true)?;
            fs::write(subvolume.join("changed"), b"changed").map_err(error)?;
            self.btrfs_snapshot(&subvolume, &second, true)?;
            let dump = self.btrfs_send_dump(&first, &second)?;
            Ok(dump.contains("changed"))
        })()
        .unwrap_or(false);
        let _ = remove_path(&self.config.btrfs_path, &second, true);
        let _ = remove_path(&self.config.btrfs_path, &first, true);
        let _ = remove_path(&self.config.btrfs_path, &subvolume, true);
        let _ = remove_directory_force(&root);
        ok
    }

    fn btrfs_send_dump(&self, base: &Path, snapshot: &Path) -> Result<String, BackendError> {
        let mut send = Command::new(&self.config.btrfs_path)
            .args([
                std::ffi::OsStr::new("send"),
                std::ffi::OsStr::new("-p"),
                base.as_os_str(),
                snapshot.as_os_str(),
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(error)?;
        let stdout = send
            .stdout
            .take()
            .ok_or_else(|| BackendError::new("btrfs send has no stdout"))?;
        let dump = Command::new(&self.config.btrfs_path)
            .args([
                std::ffi::OsStr::new("receive"),
                std::ffi::OsStr::new("--dump"),
            ])
            .stdin(Stdio::from(stdout))
            .output()
            .map_err(error)?;
        let status = send.wait().map_err(error)?;
        if !status.success() || !dump.status.success() {
            let mut send_error = String::new();
            if let Some(mut stderr) = send.stderr.take() {
                use std::io::Read;
                let _ = stderr.read_to_string(&mut send_error);
            }
            return Err(BackendError::new(format!(
                "btrfs send/receive --dump failed: send={status}; send_err={}; receive_err={}",
                send_error.trim(),
                String::from_utf8_lossy(&dump.stderr).trim()
            )));
        }
        Ok(String::from_utf8_lossy(&dump.stdout).into_owned())
    }

    fn btrfs_delete_strict(&self, path: &Path) -> Result<(), BackendError> {
        self.run_btrfs(&[
            std::ffi::OsStr::new("subvolume"),
            std::ffi::OsStr::new("delete"),
            path.as_os_str(),
        ])
    }

    fn cleanup_btrfs_probe(&self, root: &Path) {
        let _ = remove_path(&self.config.btrfs_path, &root.join("snap"), true);
        let _ = remove_path(&self.config.btrfs_path, &root.join("sub"), true);
        let _ = remove_directory_force(root);
    }

    fn run_btrfs(&self, arguments: &[&std::ffi::OsStr]) -> Result<(), BackendError> {
        let output = Command::new(&self.config.btrfs_path)
            .args(arguments)
            .output()
            .map_err(error)?;
        if output.status.success() {
            Ok(())
        } else {
            Err(BackendError::new(format!(
                "btrfs {} failed: {}",
                arguments
                    .iter()
                    .map(|value| value.to_string_lossy())
                    .collect::<Vec<_>>()
                    .join(" "),
                String::from_utf8_lossy(&output.stderr).trim()
            )))
        }
    }

    fn btrfs_create(&self, path: &Path) -> Result<(), BackendError> {
        self.run_btrfs(&[
            std::ffi::OsStr::new("subvolume"),
            std::ffi::OsStr::new("create"),
            path.as_os_str(),
        ])
    }

    fn btrfs_snapshot(&self, source: &Path, target: &Path, read_only: bool) -> Result<(), BackendError> {
        let mut arguments = vec![
            std::ffi::OsStr::new("subvolume"),
            std::ffi::OsStr::new("snapshot"),
        ];
        if read_only {
            arguments.push(std::ffi::OsStr::new("-r"));
        }
        arguments.push(source.as_os_str());
        arguments.push(target.as_os_str());
        self.run_btrfs(&arguments)
    }

    fn resolve_environment(state: &BackendState, id: &str) -> Result<Environment, BackendError> {
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

    /// Scan only the delta layers after `base` plus the branch's own upper
    /// for one mount unit.  No file content is read and the cost is
    /// O(changed entries), not the size of the whole lineage.
    fn delta_since(
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
    fn read_chain(
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

    fn resolve_snapshot(
        state: &BackendState,
        id: &str,
    ) -> Result<SnapshotRecord, BackendError> {
        state
            .snapshots
            .get(id)
            .cloned()
            .ok_or_else(|| BackendError::new(format!("unknown filesystem snapshot {id}")))
    }

    fn prepare_mount_dirs(mounts: &[MountUnit]) -> Result<(), BackendError> {
        for mount in mounts {
            fs::create_dir_all(&mount.upper).map_err(error)?;
            reset_dir(&mount.work)?;
        }
        Ok(())
    }

    /// Immutable lower chain recorded for a snapshot.  New metadata keys by
    /// absolute mount point; old metadata used `workspace`/`tmp`.
    fn snapshot_mount_layers(
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

    fn snapshot_id_for_child(child: &str) -> String {
        format!("snapshot-{child}")
    }

    fn base_snapshot_id_for_child(child: &str) -> String {
        format!("snapshot-{child}-base")
    }

    fn mark_snapshot_id(environment: &Environment, mark_id: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(environment.snapshot_id.as_bytes());
        hasher.update(b"\0");
        hasher.update(mark_id.as_bytes());
        format!("snapshot-{}-mark-{:x}", environment.id, hasher.finalize())
    }

    fn snapshot_result(snapshot_id: String) -> FilesystemResult {
        let mut result = FilesystemResult::default();
        result.snapshot_id = Some(snapshot_id);
        result
    }

    fn operation_name(operation: &Operation) -> &'static str {
        match operation {
            Operation::FilesystemFork { .. } => "filesystem_fork",
            Operation::FilesystemMark { .. } => "filesystem_mark",
            Operation::FilesystemDiff { .. } => "filesystem_diff",
            Operation::FilesystemAccept { .. } => "filesystem_accept",
            Operation::FilesystemDiscard { .. } => "filesystem_discard",
            _ => "filesystem_operation",
        }
    }

    fn replay(
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

    fn remember(
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

    fn layer_path_for(&self, snapshot_id: &str) -> PathBuf {
        match self.mechanism {
            FilesystemMechanism::Btrfs => self.snapshots_root().join(snapshot_id),
            FilesystemMechanism::Overlay => self.layers_root().join(snapshot_id),
        }
    }

    pub fn fork(
        &self,
        parent: &str,
        child: &str,
        scope: &[PathScope],
    ) -> Result<FilesystemResult, BackendError> {
        let scopes = normalize_scopes(scope)?;
        validate_component(child)?;
        let (parent_environment, existing, existing_snapshot) = {
            let state = self.lock()?;
            let parent = Self::resolve_environment(&state, parent)?;
            let existing = state.environments.get(child).cloned();
            let snapshot = existing
                .as_ref()
                .and_then(|environment| environment.base_snapshot_id.as_ref())
                .and_then(|id| state.snapshots.get(id))
                .cloned();
            (parent, existing, snapshot)
        };
        if let Some(existing) = existing {
            let matches_parent =
                existing.parent_snapshot_id.as_deref() == Some(parent_environment.snapshot_id.as_str());
            let verified = matches_parent
                && existing_snapshot
                    .as_ref()
                    .is_some_and(|snapshot| path_exists(&snapshot.path))
                && path_exists(&self.environment_metadata_path(&existing.id))
                && existing
                    .base_snapshot_id
                    .as_ref()
                    .is_some_and(|id| path_exists(&self.snapshot_metadata_path(id)));
            if !verified {
                return Err(BackendError::new(format!(
                    "replayed fork for {child} could not verify the snapshot and durable metadata"
                )));
            }
            let mut result = Self::snapshot_result(existing.snapshot_id);
            result.confirmation.snapshot_created = true;
            result.confirmation.metadata_durable = true;
            return Ok(result);
        }
        if !matches!(
            parent_environment.state,
            EnvironmentState::Available | EnvironmentState::Suspended
        ) {
            return Err(BackendError::new(
                "filesystem parent must be available or suspended before fork",
            ));
        }

        let child_root = self.environments_root().join(child);
        let snapshot_id = Self::base_snapshot_id_for_child(child);
        let layer_path = self.layer_path_for(&snapshot_id);
        if path_exists(&child_root) || path_exists(&layer_path) {
            let _ = remove_path(
                &self.config.btrfs_path,
                &child_root,
                self.mechanism == FilesystemMechanism::Btrfs,
            );
            let _ = remove_path(
                &self.config.btrfs_path,
                &layer_path,
                self.mechanism == FilesystemMechanism::Btrfs,
            );
            if path_exists(&child_root) || path_exists(&layer_path) {
                return Err(BackendError::new(format!(
                    "filesystem data for {child} already exists without durable metadata"
                )));
            }
        }

        let selected = parent_environment.selected_mounts(&scopes)?;
        let selected_targets: BTreeSet<String> = selected
            .iter()
            .map(|mount| mount.target.display().to_string())
            .collect();
        let mut child_mounts = BTreeMap::new();
        for mount in parent_environment.mounts.values() {
            let key = mount.target.display().to_string();
            let mut unit = MountUnit {
                target: mount.target.clone(),
                upper: child_root.join(&mount.layer_key).join("upper"),
                work: child_root.join(&mount.layer_key).join("work"),
                lower: mount.lower.clone(),
                layer_key: mount.layer_key.clone(),
            };
            if selected_targets.contains(&key) {
                unit.lower.push(layer_path.clone());
            }
            child_mounts.insert(key, unit);
        }
        let child_environment = Environment {
            id: child.to_string(),
            root: child_root.clone(),
            state: EnvironmentState::Available,
            snapshot_id: Self::snapshot_id_for_child(child),
            parent_snapshot_id: Some(parent_environment.snapshot_id.clone()),
            base_snapshot_id: Some(snapshot_id.clone()),
            scopes: scopes.clone(),
            mounts: child_mounts.clone(),
            lower: BTreeMap::new(),
            marks: BTreeMap::new(),
            workspace_root: parent_environment.workspace_root.clone(),
        };
        let creation = (|| -> Result<(), BackendError> {
            match self.mechanism {
                FilesystemMechanism::Btrfs => {
                    // One read-only snapshot per selected mount upper, then a
                    // writable child upper per mount.  The parent is untouched.
                    fs::create_dir_all(&layer_path).map_err(error)?;
                    for mount in &selected {
                        let layer_upper = mount.layer_upper(&layer_path);
                        if let Some(parent) = layer_upper.parent() {
                            fs::create_dir_all(parent).map_err(error)?;
                        }
                        self.btrfs_snapshot(&mount.upper, &layer_upper, true)?;
                        let child_mount = child_environment
                            .mounts
                            .get(&mount.target.display().to_string())
                            .expect("child mount exists");
                        if let Some(parent) = child_mount.upper.parent() {
                            fs::create_dir_all(parent).map_err(error)?;
                        }
                        if self.mount_uses_flattening(mount) {
                            // Writable snapshot carries the parent state, so
                            // the layer is metadata/lower only and the mount
                            // is launched with the host mount as sole lower.
                            self.btrfs_snapshot(&layer_upper, &child_mount.upper, false)?;
                        } else {
                            self.btrfs_create(&child_mount.upper)?;
                        }
                    }
                    for mount in child_environment.mounts.values() {
                        if path_exists(&mount.upper) {
                            continue;
                        }
                        if let Some(parent) = mount.upper.parent() {
                            fs::create_dir_all(parent).map_err(error)?;
                        }
                        self.btrfs_create(&mount.upper)?;
                    }
                    let units: Vec<MountUnit> = child_environment.mounts.values().cloned().collect();
                    Self::prepare_mount_dirs(&units)?;
                }
                FilesystemMechanism::Overlay => {
                    fs::create_dir_all(&layer_path).map_err(error)?;
                    for mount in &selected {
                        clone_upper(
                            &mount.upper,
                            &mount.layer_upper(&layer_path),
                            self.fork_cost == ForkCost::Reflink,
                        )?;
                        if self.mount_uses_flattening(mount) {
                            // The state directory is below this mount, so the
                            // snapshot layer may not be used as an overlay
                            // lower (directories would overlap).  Flatten the
                            // parent upper into the child upper instead; the
                            // layer copy is metadata for diff/accept only.
                            let child_mount = child_environment
                                .mounts
                                .get(&mount.target.display().to_string())
                                .expect("child mount exists");
                            clone_upper(
                                &mount.upper,
                                &child_mount.upper,
                                self.fork_cost == ForkCost::Reflink,
                            )?;
                        }
                    }
                    fs::create_dir_all(&child_root).map_err(error)?;
                    let units: Vec<MountUnit> = child_environment.mounts.values().cloned().collect();
                    Self::prepare_mount_dirs(&units)?;
                }
            }
            Ok(())
        })();

        if let Err(error) = creation {
            self.rollback_fork(&child_root, &layer_path);
            return Err(error);
        }

        let snapshot_lower: BTreeMap<String, Vec<PathBuf>> = child_environment
            .mounts
            .iter()
            .map(|(key, mount)| (key.clone(), mount.lower.clone()))
            .collect();
        let snapshot = SnapshotRecord {
            snapshot_id: snapshot_id.clone(),
            path: layer_path.clone(),
            read_only: true,
            lower: snapshot_lower,
            refs: BTreeSet::new(),
        };
        let garbage = {
            let mut state = self.lock()?;
            if state.environments.contains_key(child) {
                drop(state);
                self.rollback_fork(&child_root, &layer_path);
                return Err(BackendError::new(format!(
                    "child filesystem environment {child} already exists"
                )));
            }
            state.environments.insert(child.to_string(), child_environment.clone());
            state.snapshots.insert(snapshot_id.clone(), snapshot.clone());
            self.refresh_references(&mut state)?
        };
        self.schedule_gc(garbage)?;

        let durable = (|| -> Result<(), BackendError> {
            self.persist_snapshot(&snapshot)?;
            self.persist_environment(&child_environment)?;
            Ok(())
        })();
        if let Err(error) = durable {
            {
                let mut state = self.lock()?;
                state.environments.remove(child);
                let _ = self.refresh_references(&mut state);
            }
            let _ = self.remove_environment_metadata(child);
            self.remove_snapshot_metadata(&snapshot_id);
            self.rollback_fork(&child_root, &layer_path);
            return Err(BackendError::new(format!(
                "filesystem fork metadata was not durable: {error}"
            )));
        }

        let mut result = Self::snapshot_result(child_environment.snapshot_id);
        result.confirmation.snapshot_created = true;
        result.confirmation.metadata_durable = true;
        result.confirmation.refs_updated = true;
        result.confirmation.writers_quiesced = false;
        Ok(result)
    }

    fn rollback_fork(&self, child_root: &Path, layer_path: &Path) {
        let _ = remove_path(
            &self.config.btrfs_path,
            child_root,
            self.mechanism == FilesystemMechanism::Btrfs,
        );
        let _ = remove_path(
            &self.config.btrfs_path,
            layer_path,
            self.mechanism == FilesystemMechanism::Btrfs,
        );
    }

    /// Take a read-only snapshot of the branch's current state.
    pub fn mark(
        &self,
        snapshot: &str,
        mark_id: &str,
        scope: &[PathScope],
    ) -> Result<FilesystemResult, BackendError> {
        let scopes = normalize_scopes(scope)?;
        let (environment, existing, existing_snapshot) = {
            let state = self.lock()?;
            let environment = Self::resolve_environment(&state, snapshot)?;
            let existing = environment.marks.get(mark_id).cloned();
            let record = existing
                .as_ref()
                .and_then(|id| state.snapshots.get(id))
                .cloned();
            (environment, existing, record)
        };
        if let Some(snapshot_id) = existing {
            let verified = existing_snapshot
                .as_ref()
                .is_some_and(|record| path_exists(&record.path))
                && path_exists(&self.snapshot_metadata_path(&snapshot_id));
            if !verified {
                return Err(BackendError::new(format!(
                    "replayed mark {mark_id} could not verify the snapshot and durable metadata"
                )));
            }
            let mut result = Self::snapshot_result(snapshot_id);
            result.confirmation.snapshot_created = true;
            result.confirmation.metadata_durable = true;
            return Ok(result);
        }
        if environment.state == EnvironmentState::Terminated {
            return Err(BackendError::new(
                "terminated filesystem environment cannot be marked",
            ));
        }
        let snapshot_id = Self::mark_snapshot_id(&environment, mark_id);
        let layer_path = self.layer_path_for(&snapshot_id);
        if path_exists(&layer_path) {
            return Err(BackendError::new(format!(
                "filesystem mark snapshot {snapshot_id} already exists"
            )));
        }

        let selected = environment.selected_mounts(&scopes)?;
        let selected_targets: BTreeSet<String> = selected
            .iter()
            .map(|mount| mount.target.display().to_string())
            .collect();
        let mut updated = environment.clone();
        let mut record_lower: BTreeMap<String, Vec<PathBuf>> = environment
            .mounts
            .iter()
            .map(|(key, mount)| {
                let mut chain = mount.lower.clone();
                if selected_targets.contains(key) {
                    chain.push(layer_path.clone());
                }
                (key.clone(), chain)
            })
            .collect();
        for mount in &selected {
            record_lower
                .entry(mount.target.display().to_string())
                .or_default();
        }
        let created = (|| -> Result<(), BackendError> {
            match self.mechanism {
                FilesystemMechanism::Btrfs => {
                    // One read-only snapshot per selected mount upper.  The
                    // live mount and uppers are not changed.
                    fs::create_dir_all(&layer_path).map_err(error)?;
                    for mount in &selected {
                        let layer_upper = mount.layer_upper(&layer_path);
                        if let Some(parent) = layer_upper.parent() {
                            fs::create_dir_all(parent).map_err(error)?;
                        }
                        self.btrfs_snapshot(&mount.upper, &layer_upper, true)?;
                    }
                }
                FilesystemMechanism::Overlay => {
                    // Independent clone of each selected mount upper only.
                    fs::create_dir_all(&layer_path).map_err(error)?;
                    for mount in &selected {
                        clone_upper(
                            &mount.upper,
                            &mount.layer_upper(&layer_path),
                            self.fork_cost == ForkCost::Reflink,
                        )?;
                    }
                }
            }
            Ok(())
        })();
        if let Err(error) = created {
            let _ = remove_path(
                &self.config.btrfs_path,
                &layer_path,
                self.mechanism == FilesystemMechanism::Btrfs,
            );
            return Err(error);
        }

        let record = SnapshotRecord {
            snapshot_id: snapshot_id.clone(),
            path: layer_path,
            read_only: true,
            lower: record_lower,
            refs: BTreeSet::new(),
        };
        let (garbage, durable) = {
            let mut state = self.lock()?;
            state.snapshots.insert(snapshot_id.clone(), record.clone());
            updated
                .marks
                .insert(mark_id.to_string(), snapshot_id.clone());
            state.environments.insert(updated.id.clone(), updated.clone());
            let garbage = self.refresh_references(&mut state)?;
            let durable = self.persist_snapshot(&record).is_ok()
                && self.persist_environment(&updated).is_ok();
            (garbage, durable)
        };
        if !durable {
            {
                let mut state = self.lock()?;
                state.snapshots.remove(&snapshot_id);
                state
                    .environments
                    .insert(environment.id.clone(), environment.clone());
                let garbage = self.refresh_references(&mut state)?;
                let _ = self.schedule_gc(garbage);
            }
            self.remove_snapshot_metadata(&snapshot_id);
            let _ = remove_path(
                &self.config.btrfs_path,
                &record.path,
                self.mechanism == FilesystemMechanism::Btrfs,
            );
            let _ = self.persist_environment(&environment);
            return Err(BackendError::new(
                "filesystem mark metadata was not durable",
            ));
        }
        self.schedule_gc(garbage)?;
        let mut result = Self::snapshot_result(snapshot_id);
        result.confirmation.snapshot_created = true;
        result.confirmation.metadata_durable = true;
        result.confirmation.refs_updated = true;
        result.confirmation.writers_quiesced = false;
        Ok(result)
    }

    fn btrfs_delta(
        &self,
        _environment: &Environment,
        mount: &MountUnit,
        base: &Path,
    ) -> Result<(View, Vec<(String, String)>, Vec<FilesystemChange>), BackendError> {
        let token = self.token();
        let snapshot_root = self
            .snapshots_root()
            .join(format!("diff-snapshot-{token}"));
        let current = snapshot_root.join(&mount.layer_key).join("upper");
        if let Some(parent) = current.parent() {
            fs::create_dir_all(parent).map_err(error)?;
        }
        self.btrfs_snapshot(&mount.upper, &current, true)?;
        let base_upper = mount.layer_upper(base);
        let dump = match self.btrfs_send_dump(&base_upper, &current) {
            Ok(dump) => dump,
            Err(error) => {
                let _ = remove_path(&self.config.btrfs_path, &snapshot_root, true);
                return Err(error);
            }
        };
        // The send stream is rooted at the mount upper itself.
        let changes = parse_send_dump(&dump, "");
        let source_upper = mount.upper.clone();
        let mut view = View::new();
        let mut redirects = Vec::new();
        for change in &changes {
            match change.kind.as_str() {
                "delete" => {
                    view.insert(change.path.clone(), whiteout_node());
                }
                "rename" => {
                    let old = change.old_path.clone().unwrap_or_default();
                    view.insert(old.clone(), whiteout_node());
                    if let Some(node) = layer_node_at(&source_upper, &change.path)? {
                        if let Some(target) = node.redirect.clone() {
                            redirects.push((target, change.path.clone()));
                        }
                        view.insert(change.path.clone(), node);
                    } else {
                        view.insert(change.path.clone(), whiteout_node());
                    }
                }
                _ => {
                    if let Some(node) = layer_node_at(&source_upper, &change.path)? {
                        if node.is_whiteout() {
                            view.insert(change.path.clone(), whiteout_node());
                        } else {
                            if let Some(target) = node.redirect.clone() {
                                redirects.push((target, change.path.clone()));
                            }
                            view.insert(change.path.clone(), node);
                        }
                    } else {
                        view.insert(change.path.clone(), whiteout_node());
                    }
                }
            }
        }
        let _ = remove_path(&self.config.btrfs_path, &snapshot_root, true);
        Ok((view, redirects, changes))
    }

    /// Compare the branch's own delta with the speculation snapshot it is
    /// based on.  Only delta layers after `base` plus the branch upper are
    /// scanned; no file content is read.
    pub fn diff(
        &self,
        base: &str,
        source: &str,
        target: Option<&str>,
        scope: &[PathScope],
    ) -> Result<FilesystemResult, BackendError> {
        let scopes = normalize_scopes(scope)?;
        if target.is_some() {
            return Err(BackendError::new(
                "filesystem diff target filtering is not implemented (Phase 2)",
            ));
        }
        let (base_record, source_environment) = {
            let state = self.lock()?;
            let base = Self::resolve_snapshot(&state, base)?;
            let source = Self::resolve_environment(&state, source)?;
            (base, source)
        };
        let mounts = source_environment.selected_mounts(&scopes)?;
        let mut changes = Vec::new();
        for mount in &mounts {
            let in_lower = !self.mount_uses_flattening(mount)
                && mount.lower.iter().any(|layer| layer == &base_record.path);
            let mount_changes = if in_lower {
                let (delta, redirects) =
                    self.delta_since(&source_environment, mount, &base_record.path)?;
                changes_from_delta(&delta, &redirects)
            } else if self.mechanism == FilesystemMechanism::Overlay {
                // A mark is an independent copy of the upper, not a layer in
                // the live lower chain.  Compare the branch's own upper with
                // that copy; cost is O(upper entries), never the lineage.
                let current = read_layer(&mount.upper)?;
                let mark = read_layer(&mount.layer_upper(&base_record.path))?;
                changes_from_upper_copy(&current, &mark)
            } else {
                let (_delta, _redirects, raw) =
                    self.btrfs_delta(&source_environment, mount, &base_record.path)?;
                raw
            };
            changes.extend(
                mount_changes
                    .into_iter()
                    .map(|change| prefix_change(&mount.target, change)),
            );
        }
        changes.sort_by(|left, right| left.path.cmp(&right.path));
        changes.dedup_by(|left, right| left.path == right.path && left.kind == right.kind);
        let document = DiffDocument {
            base: base.to_string(),
            source: source.to_string(),
            changes,
        };
        let path = self.diffs_root().join(format!(
            "diff-{}-{}-{}.json",
            std::process::id(),
            self.token(),
            now_nanos()
        ));
        self.write_json(&path, &document)?;
        let mut result = FilesystemResult::default();
        result.snapshot_id = Some(source_environment.snapshot_id);
        result.diff_ref = Some(path.display().to_string());
        result.confirmation.metadata_durable = true;
        Ok(result)
    }

    /// Three-way merge `source` into `target` using `base` as the common
    /// ancestor.  Data is staged outside the live upper and swapped only after
    /// the merge is fully materialized.
    pub fn accept(
        &self,
        base: &str,
        source: &str,
        target: &str,
        scope: &[PathScope],
        target_mount_pid: Option<u32>,
        target_mount_namespace: Option<u64>,
    ) -> Result<FilesystemResult, BackendError> {
        let scopes = normalize_scopes(scope)?;
        let target_environment = {
            let state = self.lock()?;
            Self::resolve_environment(&state, target)?
        };
        if target_environment.state != EnvironmentState::Suspended {
            return Err(BackendError::new(
                "target filesystem environment must be suspended before accept",
            ));
        }
        let safe_root = match target_mount_pid {
            Some(pid) => {
                let expected = target_mount_namespace.ok_or_else(|| {
                    BackendError::new(
                        "target environment has no recorded sandbox mount namespace; \
                         ProcessBackend must supply it from process_handle()",
                    )
                })?;
                let actual = fs::metadata(format!("/proc/{pid}/ns/mnt"))
                    .map_err(error)?
                    .ino();
                let host = fs::metadata("/proc/self/ns/mnt").map_err(error)?.ino();
                if actual == host {
                    return Err(BackendError::new(
                        "target_mount_pid is in the host mount namespace (likely the outer bwrap pid); \
                         pass the sandbox child pid from bwrap --json-status-fd",
                    ));
                }
                if actual != expected {
                    return Err(BackendError::new(
                        "target_mount_pid mount namespace does not match the recorded target environment",
                    ));
                }
                Some(SafeRoot::open(pid)?)
            }
            None => None,
        };
        // A None target_mount_pid is the caller's assertion that no process
        // currently has this environment mounted.  The filesystem backend does
        // not own process or mount-namespace liveness.

        let (base_record, source_environment) = {
            let state = self.lock()?;
            let base = Self::resolve_snapshot(&state, base)?;
            let source = Self::resolve_environment(&state, source)?;
            (base, source)
        };
        let mounts = source_environment.selected_mounts(&scopes)?;
        let mut conflicts = Vec::new();
        let mut prepared = Vec::new();
        for mount in &mounts {
            let key = mount.target.display().to_string();
            let target_mount = target_environment.mount_by_target(&mount.target).ok_or_else(|| {
                BackendError::new(format!(
                    "target environment does not manage mount {}",
                    mount.target.display()
                ))
            })?;
            let (delta, redirects) = if self.mechanism == FilesystemMechanism::Btrfs {
                let (delta, redirects, _) =
                    self.btrfs_delta(&source_environment, mount, &base_record.path)?;
                (delta, redirects)
            } else if self.mount_uses_flattening(mount) {
                let current = read_layer(&mount.upper)?;
                let base_view = read_layer(&mount.layer_upper(&base_record.path))?;
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
                (delta, redirects)
            } else {
                self.delta_since(&source_environment, mount, &base_record.path)?
            };
            let base_layers =
                Self::snapshot_mount_layers(&base_record, &source_environment, mount);
            let target_upper = read_layer(&target_mount.upper)?;
            conflicts.extend(
                merge_delta_layers(
                    &delta,
                    &redirects,
                    &base_layers,
                    target_mount,
                    &target_upper,
                )?
                .into_iter()
                .map(|conflict| prefix_conflict(&mount.target, conflict)),
            );
            let source_maps = if self.mechanism == FilesystemMechanism::Btrfs
                || self.mount_uses_flattening(mount)
            {
                vec![read_layer(&mount.upper)?]
            } else {
                let mut maps = self.read_chain(&mount.lower, mount)?;
                maps.push(read_layer(&mount.upper)?);
                maps
            };
            let _ = key;
            prepared.push((mount.clone(), target_mount.clone(), delta, redirects, source_maps));
        }
        if !conflicts.is_empty() {
            let mut result = FilesystemResult::default();
            result.conflicts = conflicts;
            return Ok(result);
        }

        for (mount, target_mount, delta, redirects, source_maps) in prepared {
            let replay_root = match &safe_root {
                Some(root) => {
                    let prefix = mount
                        .target
                        .strip_prefix("/")
                        .unwrap_or(&mount.target)
                        .to_string_lossy()
                        .into_owned();
                    ReplayRoot::Mount { root, prefix }
                }
                None => ReplayRoot::Upper(&target_mount.upper),
            };
            // Phase 1 replay is deliberately non-atomic.  A failure can leave
            // a partially applied target; the result is an error and
            // `published` stays false.  The caller must keep the target free
            // of concurrent writers for the replay window.
            replay_delta(&replay_root, &delta, &redirects, &source_maps)?;
        }

        let mut result = FilesystemResult::default();
        result.confirmation.published = true;
        result.confirmation.target_paused = false;
        result.confirmation.writers_quiesced = false;
        result.confirmation.metadata_durable = false;
        result.confirmation.refs_updated = false;
        Ok(result)
    }

    /// Release a branch or mark and schedule its data for removal.
    pub fn discard(
        &self,
        snapshot: &str,
        _scope: &[PathScope],
    ) -> Result<FilesystemResult, BackendError> {
        let (environment, mark, record, garbage) = {
            let mut state = self.lock()?;
            if let Some(environment) = state.environments.get(snapshot).cloned() {
                if environment.state != EnvironmentState::Terminated {
                    return Err(BackendError::new(
                        "filesystem environment must be terminated before discard",
                    ));
                }
                state.environments.remove(&environment.id);
                let garbage = self.refresh_references(&mut state)?;
                (Some(environment), None, None, garbage)
            } else if let Some(record) = state.snapshots.get(snapshot).cloned() {
                state.snapshots.remove(snapshot);
                for environment in state.environments.values_mut() {
                    environment.marks.retain(|_, value| value != snapshot);
                }
                self.remove_snapshot_metadata(snapshot);
                let garbage = self.refresh_references(&mut state)?;
                (None, Some(snapshot.to_string()), Some(record), garbage)
            } else {
                let environment = Self::resolve_environment(&state, snapshot)?;
                if environment.state != EnvironmentState::Terminated {
                    return Err(BackendError::new(
                        "filesystem environment must be terminated before discard",
                    ));
                }
                state.environments.remove(&environment.id);
                let garbage = self.refresh_references(&mut state)?;
                (Some(environment), None, None, garbage)
            }
        };
        if let Some(environment) = &environment {
            self.remove_environment_metadata(&environment.id)?;
        }
        self.schedule_gc(garbage)?;
        if let Some(environment) = environment {
            self.retire_path(&environment.root)?;
        }
        if let Some(record) = record {
            self.retire_path(&record.path)?;
        }
        let mut result = FilesystemResult::default();
        result.snapshot_id = Some(mark.unwrap_or_else(|| snapshot.to_string()));
        result.confirmation.refs_released = true;
        // True only after every path was moved out of the reachable
        // namespace; physical reclamation continues in the background.
        result.confirmation.gc_eligible = true;
        Ok(result)
    }

    fn environment(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, BackendState>, BackendError> {
        self.lock()
    }

    fn paths(&self, id: &str) -> PathBuf {
        self.environments_root().join(id)
    }

    /// A mount whose subtree contains the backend state directory cannot use
    /// snapshot layers below that same mount as overlay lowerdirs: OverlayFS
    /// rejects overlapping lower directories.  Such mounts are flattened at
    /// fork (parent upper copied into child upper) and their layers are only
    /// used as diff metadata.
    fn mount_uses_flattening(&self, mount: &MountUnit) -> bool {
        self.config.state_directory.starts_with(&mount.target)
    }

    fn launch_lowers(&self, mount: &MountUnit) -> Vec<PathBuf> {
        let mut lowers = vec![mount.target.clone()];
        if !self.mount_uses_flattening(mount) {
            for layer in &mount.lower {
                lowers.push(mount.layer_upper(layer));
            }
        }
        lowers
    }

    fn overlayable_mounts(&self) -> Vec<PathBuf> {
        if let Ok(cache) = self.overlay_targets.lock()
            && let Some(targets) = cache.as_ref()
        {
            return targets.clone();
        }
        let targets: Vec<PathBuf> = real_writable_mounts()
            .into_iter()
            .filter(|target| self.probe_mount_target(target))
            .collect();
        if let Ok(mut cache) = self.overlay_targets.lock() {
            *cache = Some(targets.clone());
        }
        targets
    }

    fn probe_mount_target(&self, target: &Path) -> bool {
        if target == Path::new("/") || !target.is_dir() {
            return false;
        }
        let token = self.token();
        let root = self
            .config
            .state_directory
            .join(".probe-mounts")
            .join(token);
        let upper = root.join("upper");
        let work = root.join("work");
        if fs::create_dir_all(&upper)
            .and_then(|_| fs::create_dir_all(&work))
            .is_err()
        {
            let _ = remove_directory_force(&root);
            return false;
        }
        let true_path = if Path::new("/usr/bin/true").exists() {
            "/usr/bin/true"
        } else {
            "/bin/true"
        };
        let mut command = Command::new(&self.config.bubblewrap_path);
        command
            .args(["--die-with-parent", "--unshare-user"])
            .arg("--ro-bind")
            .arg("/")
            .arg("/")
            .arg("--overlay-src")
            .arg(target)
            .arg("--overlay")
            .arg(&upper)
            .arg(&work)
            .arg(target)
            .arg("--")
            .arg(true_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let ok = matches!(command.status(), Ok(status) if status.success());
        let _ = remove_directory_force(&root);
        ok
    }

    /// Produce the filesystem part of the process launch.  This backend never
    /// spawns a process and never records pids or namespaces.
    pub fn launch_fragment(
        &self,
        context: &BackendContext,
        launch: &crate::contract::AgentLaunch,
    ) -> Result<LaunchFragment, BackendError> {
        if !self.usable {
            return Err(BackendError::new(
                "no usable filesystem data plane (bubblewrap overlay probe failed)",
            ));
        }
        let mut environment = {
            let state = self.environment()?;
            state
                .environments
                .get(&context.environment_id)
                .cloned()
                .ok_or_else(|| BackendError::new("unknown filesystem environment"))?
        };
        if environment.state != EnvironmentState::Available {
            return Err(BackendError::new(
                "filesystem environment is not launchable",
            ));
        }
        if environment.workspace_root.as_deref() != Some(launch.cwd.as_path()) {
            environment.workspace_root = Some(launch.cwd.clone());
            {
                let mut state = self.environment()?;
                state
                    .environments
                    .insert(environment.id.clone(), environment.clone());
            }
            if let Err(error) = self.persist_environment(&environment) {
                eprintln!("filesystem: failed to persist workspace root: {error}");
            }
        }
        // Add mount units that appeared after establish (or were absent from
        // migrated legacy metadata).  Every writable overlayable mount is a
        // normal isolation unit, not a special workspace/tmp case.
        let mut mounts_added = false;
        for target in self.overlayable_mounts() {
            if !target.is_dir() {
                continue;
            }
            let key = target.display().to_string();
            if environment.mounts.contains_key(&key) {
                continue;
            }
            let layer_key = format!("mounts/{}", short_hash(&key));
            environment.mounts.insert(
                key,
                MountUnit {
                    upper: environment.root.join(&layer_key).join("upper"),
                    work: environment.root.join(&layer_key).join("work"),
                    target,
                    lower: Vec::new(),
                    layer_key,
                },
            );
            mounts_added = true;
        }
        if environment.mount_for_path(&launch.cwd).is_none() {
            return Err(BackendError::new(format!(
                "workspace {} is not on an overlayable mounted filesystem; configure a separate writable mount or allow passthrough_rw",
                launch.cwd.display()
            )));
        }
        if mounts_added {
            let units: Vec<MountUnit> = environment.mounts.values().cloned().collect();
            Self::prepare_mount_dirs(&units)?;
            {
                let mut state = self.environment()?;
                state
                    .environments
                    .insert(environment.id.clone(), environment.clone());
            }
            if let Err(error) = self.persist_environment(&environment) {
                eprintln!("filesystem: failed to persist newly discovered mounts: {error}");
            }
        }

        let mut fragment = LaunchFragment {
            chdir: Some(launch.cwd.clone()),
            ..LaunchFragment::default()
        };
        fragment.mounts.push(MountEntry::RoBind {
            source: PathBuf::from("/"),
            target: PathBuf::from("/"),
        });
        fragment.mounts.push(MountEntry::Proc {
            target: PathBuf::from("/proc"),
        });
        fragment.mounts.push(MountEntry::Dev {
            target: PathBuf::from("/dev"),
        });
        let mut mounts = environment.selected_mounts(&environment.scopes)?;
        // Parents must be mounted before nested child mounts so a later
        // parent overlay cannot hide an already-mounted child overlay.
        mounts.sort_by_key(|mount| mount.target.components().count());
        let mut covered: BTreeSet<PathBuf> = BTreeSet::new();
        for mount in &mounts {
            if !mount.target.is_dir() {
                continue;
            }
            ensure_overlay_disjoint(&mount.target, &mount.upper)?;
            fs::create_dir_all(&mount.upper).map_err(error)?;
            reset_dir(&mount.work)?;
            let lowers = self.launch_lowers(mount);
            fragment.mounts.push(MountEntry::Overlay {
                lowers,
                upper: mount.upper.clone(),
                work: mount.work.clone(),
                target: mount.target.clone(),
            });
            covered.insert(mount.target.clone());
        }
        // Everything writable that we did not overlay stays read-only.
        for target in if self.config.isolate_unmanaged_mounts {
            real_writable_mounts()
        } else {
            Vec::new()
        } {
            if covered.contains(&target) {
                continue;
            }
            if self
                .config
                .passthrough_rw
                .iter()
                .any(|path| target.starts_with(path))
            {
                continue;
            }
            fragment.mounts.push(MountEntry::RoBind {
                source: target.clone(),
                target,
            });
        }
        for path in &self.config.passthrough_rw {
            if path.exists() {
                fragment.mounts.push(MountEntry::Bind {
                    source: path.clone(),
                    target: path.clone(),
                });
            }
        }
        // Hide our own state (upper/work/snapshots/metadata) from the Agent.
        fragment.mounts.push(MountEntry::Tmpfs {
            target: self.config.state_directory.clone(),
        });
        fragment.mounts.push(MountEntry::RemountRo {
            target: self.config.state_directory.clone(),
        });
        fragment.mounts.sort_by_key(|entry| match entry {
            MountEntry::RoBind { .. } | MountEntry::Proc { .. } | MountEntry::Dev { .. } => 0,
            MountEntry::Overlay { .. } => 1,
            MountEntry::Bind { .. } => 2,
            MountEntry::Tmpfs { .. }
            | MountEntry::RemountRo { .. }
            | MountEntry::Dir { .. } => 3,
        });
        Ok(fragment)
    }
}

impl Backend for FilesystemBackend {
    fn backend_id(&self) -> &str {
        "filesystem"
    }

    fn capabilities(&self) -> RuntimeCapabilities {
        RuntimeCapabilities {
            shared_environment: false,
            branch_environment: self.usable,
            tools: Vec::new(),
            filesystem_fast_delete: Some(self.fast_delete),
            filesystem_fork_cost: Some(self.fork_cost.as_str().to_string()),
            filesystem_btrfs_incremental_diff: Some(self.btrfs_incremental_diff),
            filesystem_passthrough: Some(
                self.config
                    .passthrough_rw
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect(),
            ),
            freeze_mechanism: None,
        }
    }

    fn establish(&self, id: &str) -> Result<(), BackendError> {
        validate_component(id)?;
        if !self.usable {
            return Err(BackendError::new(
                "no usable filesystem data plane (bubblewrap overlay probe failed)",
            ));
        }
        let root = self.paths(id);
        let scopes = vec![PathScope::Workspace, PathScope::Tmp];
        {
            let state = self.environment()?;
            if state.environments.contains_key(id) {
                return Err(BackendError::new("filesystem environment already exists"));
            }
        }
        fs::create_dir_all(&root).map_err(error)?;
        let mut mounts = BTreeMap::new();
        for target in self.overlayable_mounts() {
            if !target.is_dir() {
                continue;
            }
            let key = target.display().to_string();
            let layer_key = format!("mounts/{}", short_hash(&key));
            mounts.insert(
                key,
                MountUnit {
                    upper: root.join(&layer_key).join("upper"),
                    work: root.join(&layer_key).join("work"),
                    target,
                    lower: Vec::new(),
                    layer_key,
                },
            );
        }
        let units: Vec<MountUnit> = mounts.values().cloned().collect();
        if self.mechanism == FilesystemMechanism::Btrfs {
            for mount in &units {
                if let Some(parent) = mount.upper.parent() {
                    fs::create_dir_all(parent).map_err(error)?;
                }
                if let Err(error) = self.btrfs_create(&mount.upper) {
                    let _ = remove_path(&self.config.btrfs_path, &root, true);
                    return Err(error);
                }
            }
        }
        if let Err(error) = Self::prepare_mount_dirs(&units) {
            let _ = remove_path(
                &self.config.btrfs_path,
                &root,
                self.mechanism == FilesystemMechanism::Btrfs,
            );
            return Err(error);
        }
        let environment = Environment {
            id: id.to_string(),
            root,
            state: EnvironmentState::Available,
            snapshot_id: Self::snapshot_id_for_child(id),
            parent_snapshot_id: None,
            base_snapshot_id: None,
            scopes,
            mounts,
            lower: BTreeMap::new(),
            marks: BTreeMap::new(),
            workspace_root: None,
        };
        {
            let mut state = self.environment()?;
            state
                .environments
                .insert(id.to_string(), environment.clone());
        }
        if let Err(error) = self.persist_environment(&environment) {
            let mut state = self.environment()?;
            state.environments.remove(id);
            drop(state);
            let _ = remove_path(
                &self.config.btrfs_path,
                &environment.root,
                self.mechanism == FilesystemMechanism::Btrfs,
            );
            return Err(error);
        }
        Ok(())
    }

    fn branch(&self, parent: &str, child: &str) -> Result<(), BackendError> {
        let scopes = {
            let state = self.environment()?;
            Self::resolve_environment(&state, parent)?.scopes
        };
        self.fork(parent, child, &scopes).map(|_| ())
    }

    fn operate(
        &self,
        id: &str,
        command: &[String],
        cancel: &CancelHandle,
    ) -> Result<crate::contract::CommandOutput, BackendError> {
        if cancel.is_cancelled() {
            return Err(BackendError::interrupted());
        }
        if !self.usable {
            return Err(BackendError::new(
                "no usable filesystem data plane (bubblewrap overlay probe failed)",
            ));
        }
        let environment = {
            let state = self.environment()?;
            state
                .environments
                .get(id)
                .cloned()
                .ok_or_else(|| BackendError::new("unknown filesystem environment"))?
        };
        if environment.state != EnvironmentState::Available {
            return Err(BackendError::new(format!(
                "filesystem environment is not available: {:?}",
                environment.state
            )));
        }
        let mut spec = MountSpec {
            chdir: Some(PathBuf::from("/")),
            json_status_fd: None,
            ..MountSpec::default()
        };
        spec.entries.push(MountEntry::RoBind {
            source: PathBuf::from("/"),
            target: PathBuf::from("/"),
        });
        spec.entries.push(MountEntry::Proc {
            target: PathBuf::from("/proc"),
        });
        spec.entries.push(MountEntry::Dev {
            target: PathBuf::from("/dev"),
        });
        let mut mounts: Vec<MountUnit> = environment.mounts.values().cloned().collect();
        mounts.sort_by_key(|mount| mount.target.components().count());
        for mount in &mounts {
            if !mount.target.is_dir() {
                continue;
            }
            ensure_overlay_disjoint(&mount.target, &mount.upper)?;
            fs::create_dir_all(&mount.upper).map_err(error)?;
            reset_dir(&mount.work)?;
            let lowers = self.launch_lowers(mount);
            spec.entries.push(MountEntry::Overlay {
                lowers,
                upper: mount.upper.clone(),
                work: mount.work.clone(),
                target: mount.target.clone(),
            });
        }
        spec.entries.push(MountEntry::Tmpfs {
            target: self.config.state_directory.clone(),
        });
        spec.entries.push(MountEntry::RemountRo {
            target: self.config.state_directory.clone(),
        });
        let mut wrapped = Command::new(&self.config.bubblewrap_path);
        spec.apply(&mut wrapped);
        wrapped.arg("--").args(command);
        let output = wrapped.output().map_err(error)?;
        Ok(crate::contract::CommandOutput {
            exit_code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    fn promote(&self, id: &str) -> Result<(), BackendError> {
        let state = self.environment()?;
        let environment = state
            .environments
            .get(id)
            .ok_or_else(|| BackendError::new("unknown filesystem environment"))?;
        if environment.state != EnvironmentState::Available {
            return Err(BackendError::new(format!(
                "filesystem environment cannot be promoted from {:?}",
                environment.state
            )));
        }
        Ok(())
    }

    fn suspend(&self, id: &str) -> Result<(), BackendError> {
        let mut state = self.environment()?;
        let environment = state
            .environments
            .get_mut(id)
            .ok_or_else(|| BackendError::new("unknown filesystem environment"))?;
        if environment.state != EnvironmentState::Available {
            return Err(BackendError::new(format!(
                "filesystem environment cannot be suspended from {:?}",
                environment.state
            )));
        }
        environment.state = EnvironmentState::Suspended;
        let metadata = environment.clone();
        drop(state);
        self.persist_environment(&metadata)
    }

    fn resume(&self, id: &str) -> Result<(), BackendError> {
        let mut state = self.environment()?;
        let environment = state
            .environments
            .get_mut(id)
            .ok_or_else(|| BackendError::new("unknown filesystem environment"))?;
        if environment.state != EnvironmentState::Suspended {
            return Err(BackendError::new(format!(
                "filesystem environment cannot be resumed from {:?}",
                environment.state
            )));
        }
        environment.state = EnvironmentState::Available;
        let metadata = environment.clone();
        drop(state);
        self.persist_environment(&metadata)
    }

    fn terminate(&self, id: &str) -> Result<(), BackendError> {
        let mut state = self.environment()?;
        let environment = state
            .environments
            .get_mut(id)
            .ok_or_else(|| BackendError::new("unknown filesystem environment"))?;
        if !matches!(
            environment.state,
            EnvironmentState::Available | EnvironmentState::Suspended
        ) {
            return Err(BackendError::new(format!(
                "filesystem environment cannot be terminated from {:?}",
                environment.state
            )));
        }
        environment.state = EnvironmentState::Terminated;
        let metadata = environment.clone();
        drop(state);
        self.persist_environment(&metadata)
    }

    fn cleanup(&self, id: &str) -> Result<(), BackendError> {
        let (environment, garbage) = {
            let mut state = self.environment()?;
            let environment = state
                .environments
                .get(id)
                .cloned()
                .ok_or_else(|| BackendError::new("unknown filesystem environment"))?;
            if environment.state != EnvironmentState::Terminated {
                return Err(BackendError::new(
                    "filesystem cleanup requires a terminated environment",
                ));
            }
            remove_path(
                &self.config.btrfs_path,
                &environment.root,
                self.mechanism == FilesystemMechanism::Btrfs,
            )?;
            state.environments.remove(id);
            let garbage = self.refresh_references(&mut state)?;
            (environment, garbage)
        };
        self.remove_environment_metadata(&environment.id)?;
        self.schedule_gc(garbage)?;
        Ok(())
    }

    fn state(&self, id: &str) -> Option<EnvironmentState> {
        self.state.lock().ok()?.environments.get(id).map(|value| value.state)
    }

    fn environments(&self) -> Vec<String> {
        self.state
            .lock()
            .map(|state| state.environments.keys().cloned().collect())
            .unwrap_or_default()
    }

    fn filesystem_operation(
        &self,
        context: &BackendContext,
        operation: &Operation,
        cancel: &CancelHandle,
    ) -> Result<FilesystemResult, BackendError> {
        if cancel.is_cancelled() {
            return Err(BackendError::interrupted());
        }
        let name = Self::operation_name(operation);
        if let Some(record) = self.replay(&context.operation_id, name, context.generation)? {
            if record.status == "failed" {
                return Err(BackendError::new(
                    record
                        .reason
                        .unwrap_or_else(|| "filesystem operation failed".to_string()),
                ));
            }
            return Ok(record.result);
        }
        let result = match operation {
            Operation::FilesystemFork {
                parent_snapshot_id,
                child_snapshot_id,
                path_scope,
            } => self.fork(parent_snapshot_id, child_snapshot_id, path_scope),
            Operation::FilesystemMark {
                snapshot_id,
                mark_id,
                path_scope,
            } => self.mark(snapshot_id, mark_id, path_scope),
            Operation::FilesystemDiff {
                base_snapshot_id,
                source_snapshot_id,
                target_snapshot_id,
                path_scope,
            } => self.diff(
                base_snapshot_id,
                source_snapshot_id,
                target_snapshot_id.as_deref(),
                path_scope,
            ),
            Operation::FilesystemAccept {
                base_snapshot_id,
                source_snapshot_id,
                target_snapshot_id,
                path_scope,
            } => self.accept(
                base_snapshot_id,
                source_snapshot_id,
                target_snapshot_id,
                path_scope,
                context.target_mount_pid,
                context.target_mount_namespace,
            ),
            Operation::FilesystemDiscard {
                snapshot_id,
                path_scope,
            } => self.discard(snapshot_id, path_scope),
            _ => {
                return Err(BackendError::new(
                    "filesystem dispatch requires a filesystem operation",
                ))
            }
        };
        match result {
            Ok(result) => {
                if let Err(error) = self.remember(
                    &context.operation_id,
                    name,
                    context.generation,
                    &result,
                    None,
                ) {
                    eprintln!("filesystem: operation record was not persisted: {error}");
                }
                Ok(result)
            }
            Err(error) => {
                let _ = self.remember(
                    &context.operation_id,
                    name,
                    context.generation,
                    &FilesystemResult::default(),
                    Some(error.to_string()),
                );
                Err(error)
            }
        }
    }
}

fn error(value: impl std::fmt::Display) -> BackendError {
    BackendError::new(value.to_string())
}

fn normalize_scopes(scope: &[PathScope]) -> Result<Vec<PathScope>, BackendError> {
    // Empty means "all mounts"; PathScope is only an optional filter now.
    let mut scopes = Vec::new();
    for value in scope {
        if !scopes.contains(value) {
            scopes.push(*value);
        }
    }
    Ok(scopes)
}

fn validate_component(id: &str) -> Result<(), BackendError> {
    if id.is_empty()
        || id == "."
        || id == ".."
        || id.contains('/')
        || id.contains('\\')
        || id.contains('\0')
    {
        return Err(BackendError::new(format!(
            "invalid filesystem identifier {id:?}"
        )));
    }
    Ok(())
}

fn path_exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

fn reset_dir(path: &Path) -> Result<(), BackendError> {
    if path_exists(path) {
        remove_directory_force(path)?;
    }
    fs::create_dir_all(path).map_err(error)
}

/// OverlayFS work directories can contain a mode-000 `work` child.  Plain
/// `remove_dir_all` cannot descend into it, so make directories traversable
/// before unlinking their contents.
fn remove_directory_force(path: &Path) -> Result<(), BackendError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(value) if value.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(value) => return Err(error(value)),
    };
    if metadata.is_dir() {
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
        for entry in fs::read_dir(path).map_err(error)? {
            let entry = entry.map_err(error)?;
            remove_directory_force(&entry.path())?;
        }
        fs::remove_dir(path).map_err(error)?;
    } else {
        fs::remove_file(path).map_err(error)?;
    }
    Ok(())
}

fn ensure_overlay_disjoint(target: &Path, upper: &Path) -> Result<(), BackendError> {
    // An upper directory below its lower mount is supported (the state
    // directory is commonly below the workspace mount).  It is only unsafe
    // when it *is* the mount target, so that is the one case rejected.  The
    // state directory itself is masked with a tmpfs after the overlays.
    let target = fs::canonicalize(target).unwrap_or_else(|_| target.to_path_buf());
    let upper = fs::canonicalize(upper).unwrap_or_else(|_| upper.to_path_buf());
    if target == upper {
        return Err(BackendError::new(format!(
            "overlay upper {} must not be the mount target itself",
            upper.display()
        )));
    }
    Ok(())
}

fn remove_path(btrfs: &Path, path: &Path, use_btrfs: bool) -> Result<(), BackendError> {
    remove_path_with_mode(btrfs, path, use_btrfs, use_btrfs)
}

fn remove_path_with_mode(
    btrfs: &Path,
    path: &Path,
    use_btrfs: bool,
    fast_delete: bool,
) -> Result<(), BackendError> {
    if !path_exists(path) {
        return Ok(());
    }
    let mut btrfs_failed = None;
    if use_btrfs && fast_delete {
        let output = Command::new(btrfs)
            .args([
                std::ffi::OsStr::new("subvolume"),
                std::ffi::OsStr::new("delete"),
                path.as_os_str(),
            ])
            .output()
            .map_err(error)?;
        if output.status.success() {
            return Ok(());
        }
        btrfs_failed = Some(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    if path_exists(path) && use_btrfs {
        let _ = Command::new(btrfs)
            .args([
                std::ffi::OsStr::new("property"),
                std::ffi::OsStr::new("set"),
                path.as_os_str(),
                std::ffi::OsStr::new("ro"),
                std::ffi::OsStr::new("false"),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    if path_exists(path) {
        if use_btrfs {
            remove_btrfs_tree(btrfs, path)?;
        } else {
            remove_directory_force(path)?;
        }
    }
    if path_exists(path) {
        return Err(BackendError::new(format!(
            "could not remove filesystem path {}: {}",
            path.display(),
            btrfs_failed.unwrap_or_else(|| "path still exists".to_string())
        )));
    }
    Ok(())
}

/// Recursively delete a tree that may contain btrfs subvolumes.  Plain
/// `remove_dir_all` leaves nested subvolume roots behind, so every directory
/// gets a best-effort `btrfs subvolume delete` before `rmdir`.
fn remove_btrfs_tree(btrfs: &Path, path: &Path) -> Result<(), BackendError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(value) if value.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(value) => return Err(error(value)),
    };
    if metadata.is_dir() {
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries.flatten() {
                remove_btrfs_tree(btrfs, &entry.path())?;
            }
        }
        let _ = Command::new(btrfs)
            .args([
                std::ffi::OsStr::new("property"),
                std::ffi::OsStr::new("set"),
                path.as_os_str(),
                std::ffi::OsStr::new("ro"),
                std::ffi::OsStr::new("false"),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = Command::new(btrfs)
            .args([
                std::ffi::OsStr::new("subvolume"),
                std::ffi::OsStr::new("delete"),
                path.as_os_str(),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        match fs::remove_dir(path) {
            Ok(()) => Ok(()),
            Err(value) if value.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(value) => Err(error(value)),
        }
    } else {
        fs::remove_file(path).map_err(error)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Node {
    mode: u32,
    size: u64,
    mtime_sec: i64,
    mtime_nsec: i64,
    inode: u64,
    kind: NodeKind,
    /// Host path that currently supplies the file content.  Metadata scans
    /// keep this path instead of reading the content into memory.
    source: Option<PathBuf>,
    opaque: bool,
    redirect: Option<String>,
    metacopy: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum NodeKind {
    Dir,
    File,
    Symlink(PathBuf),
    /// A deletion marker (OverlayFS whiteout or an equivalent tombstone).
    Whiteout,
    Other(u32),
}

impl Node {
    fn is_whiteout(&self) -> bool {
        matches!(self.kind, NodeKind::Whiteout)
    }
}

type View = BTreeMap<String, Node>;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct FilesystemChange {
    path: String,
    kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    old_path: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct DiffDocument {
    base: String,
    source: String,
    changes: Vec<FilesystemChange>,
}

fn equivalent(left: Option<&Node>, right: Option<&Node>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => {
            left.mode == right.mode
                && left.size == right.size
                && left.mtime_sec == right.mtime_sec
                && left.mtime_nsec == right.mtime_nsec
                && left.kind == right.kind
                && left.opaque == right.opaque
                && left.redirect == right.redirect
                && left.metacopy == right.metacopy
        }
        _ => false,
    }
}

fn apply_layer(
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

fn read_layer(root: &Path) -> Result<View, BackendError> {
    let mut nodes = View::new();
    walk_layer(root, root, &mut nodes).map_err(|value| {
        BackendError::new(format!("read layer {}: {value}", root.display()))
    })?;
    Ok(nodes)
}

fn mount_relative_path(target: &Path, relative: &str) -> String {
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

fn prefix_change(target: &Path, mut change: FilesystemChange) -> FilesystemChange {
    change.path = mount_relative_path(target, &change.path);
    if let Some(old) = change.old_path.take() {
        change.old_path = Some(mount_relative_path(target, &old));
    }
    change
}

fn prefix_conflict(target: &Path, mut conflict: FilesystemConflict) -> FilesystemConflict {
    conflict.path = mount_relative_path(target, &conflict.path);
    conflict
}

fn whiteout_node() -> Node {
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

fn parse_send_dump(dump: &str, layer_key: &str) -> Vec<FilesystemChange> {
    let prefix = if layer_key.is_empty() {
        String::new()
    } else {
        format!("{layer_key}/upper/")
    };
    let mut changes = Vec::new();
    for line in dump.lines() {
        let line = line.trim();
        let mut parts = line.split_whitespace();
        let Some(command) = parts.next() else {
            continue;
        };
        let raw_path = parts.next().unwrap_or("");
        let normalize = |value: &str| -> Option<String> {
            let value = value.trim_start_matches("./");
            value.strip_prefix(&prefix).map(|value| value.to_string())
        };
        match command {
            "rename" => {
                let Some(old) = normalize(raw_path) else {
                    continue;
                };
                let Some(new) = parts
                    .find(|value| *value == "->")
                    .and_then(|_| parts.next())
                    .and_then(normalize)
                else {
                    continue;
                };
                changes.push(FilesystemChange {
                    path: new,
                    kind: "rename".to_string(),
                    old_path: Some(old),
                });
            }
            "unlink" | "rmdir" => {
                if let Some(path) = normalize(raw_path) {
                    changes.push(FilesystemChange {
                        path,
                        kind: "delete".to_string(),
                        old_path: None,
                    });
                }
            }
            "mknod" => {
                if let Some(path) = normalize(raw_path) {
                    let whiteout = line.contains(" c 0 0") || line.contains(" c 0,0");
                    changes.push(FilesystemChange {
                        path,
                        kind: if whiteout { "delete" } else { "modify" }.to_string(),
                        old_path: None,
                    });
                }
            }
            "mkdir" | "mkfile" | "symlink" | "link" | "write" | "truncate"
            | "chmod" | "chown" | "utimes" | "set_xattr" | "remove_xattr"
            | "mkfifo" => {
                if let Some(path) = normalize(raw_path) {
                    changes.push(FilesystemChange {
                        path,
                        kind: "modify".to_string(),
                        old_path: None,
                    });
                }
            }
            _ => {}
        }
    }
    changes
}

fn node_from_metadata(path: &Path, metadata: &fs::Metadata) -> Result<Node, BackendError> {
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

fn walk_layer(
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

fn resolve_node(maps: &[View], path: &str) -> Option<Node> {
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

fn layer_node_at(root: &Path, path: &str) -> Result<Option<Node>, BackendError> {
    let target = safe_join(root, path)?;
    match fs::symlink_metadata(&target) {
        Ok(metadata) => Ok(Some(node_from_metadata(&target, &metadata)?)),
        Err(value) if value.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(value) => Err(error(value)),
    }
}

#[derive(Debug)]
enum PathResolution {
    Node(Node),
    Hidden,
    NotPresent,
}

fn resolve_path_in_upper(
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

fn resolve_path_in_layers(
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
    match resolve_path_in_upper(&mount.target, &lookup)? {
        PathResolution::Node(node) => Ok(Some(node)),
        PathResolution::Hidden | PathResolution::NotPresent => Ok(None),
    }
}

fn redirected_lookup(upper: &Path, lookup: &str) -> Result<Option<String>, BackendError> {
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

fn resolve_path_in_environment(
    mount: &MountUnit,
    path: &str,
) -> Result<Option<Node>, BackendError> {
    match resolve_path_in_upper(&mount.upper, path)? {
        PathResolution::Node(node) => return Ok(Some(node)),
        PathResolution::Hidden => return Ok(None),
        PathResolution::NotPresent => {}
    }
    match resolve_path_in_layers(&mount.lower, mount, path)? {
        Some(node) => Ok(Some(node)),
        None => match resolve_path_in_upper(&mount.target, path)? {
            PathResolution::Node(node) => Ok(Some(node)),
            PathResolution::Hidden => Ok(None),
            PathResolution::NotPresent => Ok(None),
        },
    }
}

fn ancestors(path: &str) -> Vec<String> {
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

fn changes_from_delta(
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

fn changes_from_upper_copy(current: &View, mark: &View) -> Vec<FilesystemChange> {
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
    changes.sort_by(|left, right| left.path.cmp(&right.path).then_with(|| left.kind.cmp(&right.kind)));
    changes.dedup_by(|left, right| left.path == right.path && left.kind == right.kind);
    changes
}

fn node_summary(node: Option<&Node>) -> String {
    match node {
        None => "absent".to_string(),
        Some(node) => match &node.kind {
            NodeKind::Whiteout => "whiteout".to_string(),
            NodeKind::Dir => "dir".to_string(),
            NodeKind::File => format!("file size={} mtime={}.{}", node.size, node.mtime_sec, node.mtime_nsec),
            NodeKind::Symlink(target) => format!("symlink:{}", target.display()),
            NodeKind::Other(kind) => format!("special:0o{kind:o}"),
        },
    }
}

fn conflict(
    path: &str,
    kind: &str,
    base: Option<&Node>,
    ours: Option<&Node>,
    theirs: Option<&Node>,
) -> FilesystemConflict {
    FilesystemConflict {
        path: path.to_string(),
        kind: kind.to_string(),
        reason: "target and source changed the same path differently".to_string(),
        base: Some(node_summary(base)),
        ours: Some(node_summary(ours)),
        theirs: Some(node_summary(theirs)),
    }
}

fn target_upper_has_changes_under(upper: &View, path: &str) -> bool {
    let prefix = format!("{path}/");
    upper.keys().any(|key| key.starts_with(&prefix))
}

#[cfg(test)]
fn merge_delta(
    delta: &View,
    redirects: &[(String, String)],
    base_maps: &[View],
    target_maps: &[View],
    target_upper: &View,
) -> Vec<FilesystemConflict> {
    let mut conflicts = Vec::new();
    for (path, node) in delta {
        let base = resolve_node(base_maps, path);
        let target = resolve_node(target_maps, path);
        let target_changed = !equivalent(target.as_ref(), base.as_ref());
        if node.is_whiteout() {
            if target_changed && target.is_some() {
                conflicts.push(conflict(
                    path,
                    "delete_modify",
                    base.as_ref(),
                    target.as_ref(),
                    None,
                ));
            }
            continue;
        }
        if target_changed && !equivalent(target.as_ref(), Some(node)) {
            let kind = if target.is_none() {
                "modify_delete"
            } else if base.is_none() {
                "add_add"
            } else {
                "content"
            };
            conflicts.push(conflict(
                path,
                kind,
                base.as_ref(),
                target.as_ref(),
                Some(node),
            ));
        }
        if matches!(node.kind, NodeKind::Dir) && node.opaque
            && target_upper_has_changes_under(target_upper, path)
            && !conflicts.iter().any(|value| value.path == *path)
        {
            conflicts.push(conflict(
                path,
                "opaque_replace",
                base.as_ref(),
                target.as_ref(),
                Some(node),
            ));
        }
    }
    for (old, new) in redirects {
        let base_old = resolve_node(base_maps, old);
        let target_old = resolve_node(target_maps, old);
        let base_new = resolve_node(base_maps, new);
        let target_new = resolve_node(target_maps, new);
        if !equivalent(target_old.as_ref(), base_old.as_ref())
            || !equivalent(target_new.as_ref(), base_new.as_ref())
        {
            conflicts.push(conflict(
                new,
                "rename",
                base_new.as_ref(),
                target_new.as_ref(),
                delta.get(old),
            ));
        }
    }
    conflicts.sort_by(|left, right| left.path.cmp(&right.path));
    conflicts.dedup_by(|left, right| left.path == right.path && left.kind == right.kind);
    conflicts
}

fn merge_delta_layers(
    delta: &View,
    redirects: &[(String, String)],
    base_layers: &[PathBuf],
    target_mount: &MountUnit,
    target_upper: &View,
) -> Result<Vec<FilesystemConflict>, BackendError> {
    let mut conflicts = Vec::new();
    for (path, node) in delta {
        let base = resolve_path_in_layers(base_layers, target_mount, path)?;
        let ours = resolve_path_in_environment(target_mount, path)?;
        let target_changed = !equivalent(ours.as_ref(), base.as_ref());
        if node.is_whiteout() {
            if target_changed && ours.is_some() {
                conflicts.push(conflict(
                    path,
                    "delete_modify",
                    base.as_ref(),
                    ours.as_ref(),
                    None,
                ));
            }
            continue;
        }
        if target_changed && !equivalent(ours.as_ref(), Some(node)) {
            let kind = if ours.is_none() {
                "modify_delete"
            } else if base.is_none() {
                "add_add"
            } else {
                "content"
            };
            conflicts.push(conflict(
                path,
                kind,
                base.as_ref(),
                ours.as_ref(),
                Some(node),
            ));
        }
        if matches!(node.kind, NodeKind::Dir) && node.opaque
            && target_upper_has_changes_under(target_upper, path)
            && !conflicts.iter().any(|value| value.path == *path)
        {
            conflicts.push(conflict(
                path,
                "opaque_replace",
                base.as_ref(),
                ours.as_ref(),
                Some(node),
            ));
        }
    }
    for (old, new) in redirects {
        let base_old = resolve_path_in_layers(base_layers, target_mount, old)?;
        let ours_old = resolve_path_in_environment(target_mount, old)?;
        let base_new = resolve_path_in_layers(base_layers, target_mount, new)?;
        let ours_new = resolve_path_in_environment(target_mount, new)?;
        if !equivalent(ours_old.as_ref(), base_old.as_ref())
            || !equivalent(ours_new.as_ref(), base_new.as_ref())
        {
            conflicts.push(conflict(
                new,
                "rename",
                base_new.as_ref(),
                ours_new.as_ref(),
                delta.get(old),
            ));
        }
    }
    conflicts.sort_by(|left, right| left.path.cmp(&right.path));
    conflicts.dedup_by(|left, right| left.path == right.path && left.kind == right.kind);
    Ok(conflicts)
}

fn source_content_source(maps: &[View], path: &str) -> Option<PathBuf> {
    for map in maps.iter().rev() {
        if let Some(node) = map.get(path) {
            if node.is_whiteout() {
                return None;
            }
            if node.metacopy {
                continue;
            }
            return node.source.clone();
        }
    }
    None
}

const RESOLVE_NO_MAGICLINKS: u64 = 0x02;
const RESOLVE_IN_ROOT: u64 = 0x10;

#[repr(C)]
struct OpenHow {
    flags: u64,
    mode: u64,
    resolve: u64,
}

/// Directory-fd-rooted operations for replaying into a live target mount.
/// All paths are resolved with `openat2(RESOLVE_IN_ROOT | RESOLVE_NO_MAGICLINKS)`
/// and final components use `O_NOFOLLOW`; absolute symlinks cannot escape to
/// the caller's root.
struct SafeRoot {
    fd: OwnedFd,
}

impl SafeRoot {
    fn open(pid: u32) -> Result<Self, BackendError> {
        let path = CString::new(format!("/proc/{pid}/root"))
            .map_err(|_| BackendError::new("invalid target root path"))?;
        let fd = unsafe {
            libc::open(
                path.as_ptr(),
                libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(error(std::io::Error::last_os_error()));
        }
        Ok(Self {
            fd: unsafe { OwnedFd::from_raw_fd(fd) },
        })
    }

    fn openat2(
        &self,
        path: &str,
        flags: libc::c_int,
        mode: u32,
        resolve: u64,
    ) -> Result<OwnedFd, BackendError> {
        let path = CString::new(path)
            .map_err(|_| BackendError::new("invalid target-relative path"))?;
        let how = OpenHow {
            flags: flags as u64,
            mode: mode as u64,
            resolve,
        };
        let result = unsafe {
            libc::syscall(
                libc::SYS_openat2,
                self.fd.as_raw_fd() as libc::c_long,
                path.as_ptr() as libc::c_long,
                &how as *const OpenHow as libc::c_long,
                std::mem::size_of::<OpenHow>() as libc::c_long,
            )
        };
        if result < 0 {
            return Err(error(std::io::Error::last_os_error()));
        }
        Ok(unsafe { OwnedFd::from_raw_fd(result as libc::c_int) })
    }

    fn open_dir(&self, path: &str, create: bool) -> Result<OwnedFd, BackendError> {
        if path.is_empty() || path == "." {
            let fd = unsafe { libc::dup(self.fd.as_raw_fd()) };
            if fd < 0 {
                return Err(error(std::io::Error::last_os_error()));
            }
            return Ok(unsafe { OwnedFd::from_raw_fd(fd) });
        }
        let mut current: Option<OwnedFd> = None;
        let mut prefix = String::new();
        for component in path.split('/') {
            if component.is_empty() || component == "." {
                continue;
            }
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(component);
            match self.openat2(
                &prefix,
                libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC,
                0,
                RESOLVE_IN_ROOT | RESOLVE_NO_MAGICLINKS,
            ) {
                Ok(fd) => current = Some(fd),
                Err(value) if create && value.message.contains("No such file") => {
                    let parent = match &current {
                        Some(parent) => unsafe {
                            OwnedFd::from_raw_fd(libc::dup(parent.as_raw_fd()))
                        },
                        None => unsafe {
                            OwnedFd::from_raw_fd(libc::dup(self.fd.as_raw_fd()))
                        },
                    };
                    let name = CString::new(component)
                        .map_err(|_| BackendError::new("invalid target directory name"))?;
                    if unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) } != 0 {
                        return Err(error(std::io::Error::last_os_error()));
                    }
                    current = Some(self.openat2(
                        &prefix,
                        libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC,
                        0,
                        RESOLVE_IN_ROOT | RESOLVE_NO_MAGICLINKS,
                    )?);
                }
                Err(value) => return Err(value),
            }
        }
        current.ok_or_else(|| BackendError::new("target path has no components"))
    }

    fn parent(&self, path: &str, create: bool) -> Result<(OwnedFd, String), BackendError> {
        let path = normalize_relative_path(path)?;
        let (parent, name) = path
            .rsplit_once('/')
            .map(|(parent, name)| (parent.to_string(), name.to_string()))
            .unwrap_or_else(|| (String::new(), path));
        if name.is_empty() {
            return Err(BackendError::new("target path has no file name"));
        }
        Ok((self.open_dir(&parent, create)?, name))
    }

    fn remove(&self, path: &str) -> Result<(), BackendError> {
        let path = normalize_relative_path(path)?;
        if path.is_empty() {
            return Err(BackendError::new("refusing to remove target root"));
        }
        let (parent, name) = self.parent(&path, false)?;
        let c_name = CString::new(name.clone())
            .map_err(|_| BackendError::new("invalid target file name"))?;
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        let result = unsafe {
            libc::fstatat(
                parent.as_raw_fd(),
                c_name.as_ptr(),
                &mut stat,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if result != 0 {
            let value = std::io::Error::last_os_error();
            if value.kind() == std::io::ErrorKind::NotFound {
                return Ok(());
            }
            return Err(error(value));
        }
        if (stat.st_mode & libc::S_IFMT) == libc::S_IFDIR {
            if let Ok(child) = self.openat2(
                &path,
                libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0,
                RESOLVE_IN_ROOT | RESOLVE_NO_MAGICLINKS,
            ) {
                remove_dir_fd(&child)?;
            }
            if unsafe { libc::unlinkat(parent.as_raw_fd(), c_name.as_ptr(), libc::AT_REMOVEDIR) } != 0
            {
                return Err(error(std::io::Error::last_os_error()));
            }
        } else if unsafe { libc::unlinkat(parent.as_raw_fd(), c_name.as_ptr(), 0) } != 0 {
            return Err(error(std::io::Error::last_os_error()));
        }
        Ok(())
    }

    fn exists(&self, path: &str) -> Result<bool, BackendError> {
        let path = normalize_relative_path(path)?;
        let (parent, name) = match self.parent(&path, false) {
            Ok(value) => value,
            Err(_) => return Ok(false),
        };
        let c_name = CString::new(name).map_err(|_| BackendError::new("invalid target name"))?;
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        let result = unsafe {
            libc::fstatat(
                parent.as_raw_fd(),
                c_name.as_ptr(),
                &mut stat,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if result == 0 {
            Ok(true)
        } else if std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound {
            Ok(false)
        } else {
            Err(error(std::io::Error::last_os_error()))
        }
    }

    fn mkdir(&self, path: &str, mode: u32) -> Result<(), BackendError> {
        let path = normalize_relative_path(path)?;
        let (parent, name) = self.parent(&path, true)?;
        let c_name = CString::new(name.clone())
            .map_err(|_| BackendError::new("invalid target directory name"))?;
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        let exists = unsafe {
            libc::fstatat(
                parent.as_raw_fd(),
                c_name.as_ptr(),
                &mut stat,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } == 0;
        if exists {
            if (stat.st_mode & libc::S_IFMT) == libc::S_IFDIR {
                return Ok(());
            }
            drop(parent);
            self.remove(&path)?;
            let (parent, name) = self.parent(&path, true)?;
            let c_name = CString::new(name)
                .map_err(|_| BackendError::new("invalid target directory name"))?;
            if unsafe { libc::mkdirat(parent.as_raw_fd(), c_name.as_ptr(), 0o700) } != 0 {
                return Err(error(std::io::Error::last_os_error()));
            }
            return Ok(());
        }
        if unsafe { libc::mkdirat(parent.as_raw_fd(), c_name.as_ptr(), 0o700) } != 0 {
            return Err(error(std::io::Error::last_os_error()));
        }
        let _ = mode;
        Ok(())
    }

    fn write_file(&self, path: &str, source: &Path, mode: u32) -> Result<(), BackendError> {
        let path = normalize_relative_path(path)?;
        let (parent, name) = self.parent(&path, true)?;
        let c_name = CString::new(name.clone())
            .map_err(|_| BackendError::new("invalid target file name"))?;
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        let exists = unsafe {
            libc::fstatat(
                parent.as_raw_fd(),
                c_name.as_ptr(),
                &mut stat,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } == 0;
        if exists && (stat.st_mode & libc::S_IFMT) != libc::S_IFREG {
            drop(parent);
            self.remove(&path)?;
            return self.write_file(&path, source, mode);
        }
        drop(parent);
        let output = self.openat2(
            &path,
            libc::O_WRONLY
                | libc::O_CREAT
                | libc::O_TRUNC
                | libc::O_NOFOLLOW
                | libc::O_CLOEXEC,
            mode & 0o7777,
            RESOLVE_IN_ROOT | RESOLVE_NO_MAGICLINKS,
        )?;
        let mut input = File::open(source).map_err(error)?;
        let mut output = File::from(output);
        std::io::copy(&mut input, &mut output).map_err(error)?;
        let _ = unsafe { libc::fchmod(output.as_raw_fd(), mode & 0o7777) };
        Ok(())
    }

    fn symlink(&self, path: &str, target: &Path) -> Result<(), BackendError> {
        let path = normalize_relative_path(path)?;
        let (parent, name) = self.parent(&path, true)?;
        let c_name = CString::new(name).map_err(|_| BackendError::new("invalid symlink name"))?;
        let c_target = CString::new(target.as_os_str().as_bytes())
            .map_err(|_| BackendError::new("invalid symlink target"))?;
        if unsafe { libc::symlinkat(c_target.as_ptr(), parent.as_raw_fd(), c_name.as_ptr()) } != 0 {
            return Err(error(std::io::Error::last_os_error()));
        }
        Ok(())
    }
}

fn remove_dir_fd(fd: &OwnedFd) -> Result<(), BackendError> {
    let dup = unsafe { libc::dup(fd.as_raw_fd()) };
    if dup < 0 {
        return Err(error(std::io::Error::last_os_error()));
    }
    let directory = unsafe { libc::fdopendir(dup) };
    if directory.is_null() {
        let value = std::io::Error::last_os_error();
        unsafe { libc::close(dup) };
        return Err(error(value));
    }
    loop {
        let entry = unsafe { libc::readdir(directory) };
        if entry.is_null() {
            break;
        }
        let name = unsafe { std::ffi::CStr::from_ptr((*entry).d_name.as_ptr()) };
        let bytes = name.to_bytes();
        if bytes == b"." || bytes == b".." {
            continue;
        }
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe {
            libc::fstatat(
                fd.as_raw_fd(),
                name.as_ptr(),
                &mut stat,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            unsafe { libc::closedir(directory) };
            return Err(error(std::io::Error::last_os_error()));
        }
        if (stat.st_mode & libc::S_IFMT) == libc::S_IFDIR {
            let child = unsafe {
                libc::openat(
                    fd.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if child < 0 {
                unsafe { libc::closedir(directory) };
                return Err(error(std::io::Error::last_os_error()));
            }
            let child = unsafe { OwnedFd::from_raw_fd(child) };
            if let Err(value) = remove_dir_fd(&child) {
                unsafe { libc::closedir(directory) };
                return Err(value);
            }
            if unsafe { libc::unlinkat(fd.as_raw_fd(), name.as_ptr(), libc::AT_REMOVEDIR) } != 0 {
                unsafe { libc::closedir(directory) };
                return Err(error(std::io::Error::last_os_error()));
            }
        } else if unsafe { libc::unlinkat(fd.as_raw_fd(), name.as_ptr(), 0) } != 0 {
            unsafe { libc::closedir(directory) };
            return Err(error(std::io::Error::last_os_error()));
        }
    }
    unsafe { libc::closedir(directory) };
    Ok(())
}

fn remove_visible(path: &Path) -> Result<(), BackendError> {
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

enum ReplayRoot<'a> {
    Mount { root: &'a SafeRoot, prefix: String },
    Upper(&'a Path),
}


enum AppliedOp {
    CreatedFile(String),
    CreatedDir(String),
    CreatedSymlink(String),
    Other,
}

fn replay_delta(
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

fn replay_node(
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

fn stage_node(
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

const FICLONE: libc::c_ulong = 0x4004_9409;

fn reflink_file(source: &Path, target: &Path) -> std::io::Result<()> {
    use std::os::unix::io::AsRawFd;
    let source_file = File::open(source)?;
    let target_file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(target)?;
    let result = unsafe {
        libc::ioctl(
            target_file.as_raw_fd(),
            FICLONE,
            source_file.as_raw_fd(),
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// Clone only one upper directory.  Preserves overlay markers, whiteouts,
/// modes, symlinks and mtimes.  Never uses hard links.
fn clone_upper(source: &Path, target: &Path, use_reflink: bool) -> Result<(), BackendError> {
    if !path_exists(source) {
        return Ok(());
    }
    fs::create_dir_all(target).map_err(error)?;
    let mut entries: Vec<_> = fs::read_dir(source)
        .map_err(error)?
        .filter_map(Result::ok)
        .collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let from = entry.path();
        let to = target.join(entry.file_name());
        let metadata = fs::symlink_metadata(&from).map_err(error)?;
        let file_type = metadata.file_type();
        if file_type.is_dir() {
            clone_upper(&from, &to, use_reflink)?;
            fs::set_permissions(&to, fs::Permissions::from_mode(metadata.mode() & 0o7777))
                .map_err(error)?;
            copy_overlay_xattrs(&from, &to)?;
            set_times(&to, &metadata)?;
        } else if file_type.is_symlink() {
            let link = fs::read_link(&from).map_err(error)?;
            symlink(link, &to).map_err(error)?;
            set_times(&to, &metadata)?;
        } else if file_type.is_char_device() && metadata.rdev() == 0 {
            let c_path = CString::new(to.as_os_str().as_bytes())
                .map_err(|_| BackendError::new("invalid whiteout path"))?;
            if unsafe {
                libc::mknod(
                    c_path.as_ptr(),
                    libc::S_IFCHR | (metadata.mode() & 0o7777),
                    libc::makedev(0, 0),
                )
            } != 0
            {
                return Err(error(std::io::Error::last_os_error()));
            }
            fs::set_permissions(&to, fs::Permissions::from_mode(metadata.mode() & 0o7777))
                .map_err(error)?;
            set_times(&to, &metadata)?;
        } else if file_type.is_file() {
            let cloned = use_reflink && reflink_file(&from, &to).is_ok();
            if !cloned {
                fs::copy(&from, &to).map_err(error)?;
            }
            fs::set_permissions(&to, fs::Permissions::from_mode(metadata.mode() & 0o7777))
                .map_err(error)?;
            copy_overlay_xattrs(&from, &to)?;
            set_times(&to, &metadata)?;
        } else {
            let c_path = CString::new(to.as_os_str().as_bytes())
                .map_err(|_| BackendError::new("invalid special file path"))?;
            if unsafe {
                libc::mknod(
                    c_path.as_ptr(),
                    metadata.mode(),
                    metadata.rdev(),
                )
            } != 0
            {
                return Err(error(std::io::Error::last_os_error()));
            }
            set_times(&to, &metadata)?;
        }
    }
    Ok(())
}

fn copy_overlay_xattrs(source: &Path, target: &Path) -> Result<(), BackendError> {
    for name in ["opaque", "redirect", "metacopy", "whiteout"] {
        for namespace in ["trusted.overlay.", "user.overlay."] {
            let full = format!("{namespace}{name}");
            if let Some(value) = get_xattr(source, &full) {
                let _ = set_xattr(target, &full, &value);
            }
        }
    }
    Ok(())
}

fn set_times(path: &Path, metadata: &fs::Metadata) -> Result<(), BackendError> {
    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| BackendError::new("invalid timestamp path"))?;
    let times = [
        libc::timespec {
            tv_sec: metadata.atime(),
            tv_nsec: metadata.atime_nsec(),
        },
        libc::timespec {
            tv_sec: metadata.mtime(),
            tv_nsec: metadata.mtime_nsec(),
        },
    ];
    let result = unsafe {
        libc::utimensat(
            libc::AT_FDCWD,
            path.as_ptr(),
            times.as_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        // Timestamp preservation is best-effort on filesystems that reject it;
        // semantic content and modes are already copied.
        Ok(())
    }
}

fn copy_file(source: &Path, target: &Path) -> Result<(), BackendError> {
    let mut input = File::open(source).map_err(|value| {
        BackendError::new(format!("open source {}: {value}", source.display()))
    })?;
    let mut output = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(target)
        .map_err(|value| {
            BackendError::new(format!("open target {}: {value}", target.display()))
        })?;
    std::io::copy(&mut input, &mut output).map_err(error)?;
    output.sync_all().map_err(error)?;
    Ok(())
}

fn create_whiteout(root: &Path, path: &str) -> Result<(), BackendError> {
    let target = safe_join(root, path)?;
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(error)?;
    }
    let _ = fs::remove_file(&target);
    let _ = fs::remove_dir(&target);
    let c_path = CString::new(target.as_os_str().as_bytes())
        .map_err(|_| BackendError::new(format!("invalid whiteout path {path}")))?;
    let result = unsafe {
        libc::mknod(
            c_path.as_ptr(),
            libc::S_IFCHR | 0o600,
            libc::makedev(0, 0),
        )
    };
    if result != 0 {
        // Fall back to an xattr whiteout where the kernel honours userxattr.
        File::create(&target).map_err(error)?;
        set_xattr(&target, "user.overlay.whiteout", b"y")?;
    }
    Ok(())
}

fn set_xattr(path: &Path, name: &str, value: &[u8]) -> Result<(), BackendError> {
    let c_path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| BackendError::new(format!("invalid xattr path {}", path.display())))?;
    let c_name = CString::new(name).map_err(|_| BackendError::new("invalid xattr name"))?;
    let result = unsafe {
        libc::setxattr(
            c_path.as_ptr(),
            c_name.as_ptr(),
            value.as_ptr().cast(),
            value.len(),
            0,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(error(std::io::Error::last_os_error()))
    }
}

fn overlay_xattr(path: &Path, name: &str) -> Option<Vec<u8>> {
    for namespace in ["trusted.overlay.", "user.overlay."] {
        let full = format!("{namespace}{name}");
        if let Some(value) = get_xattr(path, &full) {
            return Some(value);
        }
    }
    None
}

fn has_overlay_xattr(path: &Path, name: &str) -> bool {
    overlay_xattr(path, name).is_some()
}

fn get_xattr(path: &Path, name: &str) -> Option<Vec<u8>> {
    let path = CString::new(path.as_os_str().as_bytes()).ok()?;
    let name = CString::new(name).ok()?;
    let size = unsafe {
        libc::getxattr(
            path.as_ptr(),
            name.as_ptr(),
            std::ptr::null_mut(),
            0,
        )
    };
    if size < 0 {
        return None;
    }
    if size == 0 {
        return Some(Vec::new());
    }
    let mut buffer = vec![0u8; size as usize];
    let read = unsafe {
        libc::getxattr(
            path.as_ptr(),
            name.as_ptr(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
        )
    };
    if read < 0 {
        return None;
    }
    buffer.truncate(read as usize);
    Some(buffer)
}

fn normalize_relative_path(value: &str) -> Result<String, BackendError> {
    let path = Path::new(value);
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => normalized.push(value),
            Component::CurDir => {}
            Component::RootDir => {}
            Component::ParentDir | Component::Prefix(_) => {
                return Err(BackendError::new(format!(
                    "overlay redirect path is not relative: {value}"
                )));
            }
        }
    }
    Ok(normalized.to_string_lossy().into_owned())
}

fn remove_subtree(view: &mut View, path: &str) {
    let prefix = format!("{path}/");
    view.retain(|key, _| key != path && !key.starts_with(&prefix));
}

fn remove_children(view: &mut View, path: &str) {
    let prefix = format!("{path}/");
    view.retain(|key, _| !key.starts_with(&prefix));
}

fn safe_join(root: &Path, relative: &str) -> Result<PathBuf, BackendError> {
    let path = Path::new(relative);
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(BackendError::new(format!(
            "unsafe filesystem relative path {relative:?}"
        )));
    }
    Ok(root.join(path))
}

fn short_hash(value: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(value.as_bytes());
    format!("{:x}", digest)[..16].to_string()
}

fn real_writable_mounts() -> Vec<PathBuf> {
    let Ok(text) = fs::read_to_string("/proc/self/mountinfo") else {
        return Vec::new();
    };
    let mut mounts = Vec::new();
    for line in text.lines() {
        let Some((left, right)) = line.split_once(" - ") else {
            continue;
        };
        let fields: Vec<&str> = left.split_whitespace().collect();
        if fields.len() < 6 {
            continue;
        }
        let target = unescape_mount_path(fields[4]);
        let options = fields[5];
        if !options.split(',').any(|value| value == "rw") {
            continue;
        }
        let fstype = right.split_whitespace().next().unwrap_or("");
        if matches!(
            fstype,
            "proc"
                | "sysfs"
                | "devtmpfs"
                | "devpts"
                | "cgroup"
                | "cgroup2"
                | "securityfs"
                | "pstore"
                | "bpf"
                | "configfs"
                | "debugfs"
                | "tracefs"
                | "fusectl"
                | "mqueue"
                | "hugetlbfs"
                | "autofs"
                | "binfmt_misc"
                | "efivarfs"
                | "fuse.portal"
                | "nsfs"
                | "squashfs"
                | "iso9660"
                | "ramfs"
                | "overlay"
                | "fuse"
                | "vfat"
        ) {
            continue;
        }
        if target == Path::new("/") || target.starts_with("/proc") || target.starts_with("/sys") || target.starts_with("/dev") {
            continue;
        }
        if !mounts.contains(&target) {
            mounts.push(target);
        }
    }
    mounts
}

fn unescape_mount_path(value: &str) -> PathBuf {
    let mut result = String::new();
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\'
            && index + 3 < bytes.len()
            && bytes[index + 1..index + 4]
                .iter()
                .all(|value| (b'0'..=b'7').contains(value))
        {
            let octal = std::str::from_utf8(&bytes[index + 1..index + 4]).unwrap_or("0");
            if let Ok(value) = u8::from_str_radix(octal, 8) {
                result.push(value as char);
                index += 4;
                continue;
            }
        }
        result.push(bytes[index] as char);
        index += 1;
    }
    PathBuf::from(result)
}

fn now_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::AgentLaunch;

    fn temp() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    fn btrfs_temp() -> tempfile::TempDir {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/tmp");
        fs::create_dir_all(&root).unwrap();
        tempfile::Builder::new()
            .prefix("fs-test-")
            .tempdir_in(root)
            .expect("tempdir under target/tmp")
    }

    fn write(root: &Path, relative: &str, content: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    fn set_test_xattr(path: &Path, name: &str, value: &[u8]) {
        set_xattr(path, name, value).unwrap();
    }

    fn new_backend(root: &Path) -> FilesystemBackend {
        let backend = FilesystemBackend::new(FilesystemConfig::new(root.join("state"))).unwrap();
        assert!(
            backend.usable,
            "bubblewrap overlay probe failed; filesystem tests require a working sandbox"
        );
        backend
    }

    fn new_overlay_backend(root: &Path) -> FilesystemBackend {
        let mut config = FilesystemConfig::new(root.join("state"));
        config.force_overlay = true;
        let backend = FilesystemBackend::new(config).unwrap();
        assert!(backend.usable, "bubblewrap overlay probe failed");
        assert_eq!(backend.mechanism(), FilesystemMechanism::Overlay);
        backend
    }

    fn mount_for(backend: &FilesystemBackend, environment_id: &str, path: &Path) -> MountUnit {
        let state = backend.lock().unwrap();
        state.environments[environment_id]
            .mount_for_path(path)
            .cloned()
            .unwrap_or_else(|| {
                panic!(
                    "no mount unit for {} in environment {}: {:?}",
                    path.display(),
                    environment_id,
                    state.environments[environment_id]
                        .mounts
                        .keys()
                        .collect::<Vec<_>>()
                )
            })
    }

    fn upper_at(backend: &FilesystemBackend, environment_id: &str, path: &Path) -> PathBuf {
        let mount = mount_for(backend, environment_id, path);
        let relative = path.strip_prefix(&mount.target).unwrap_or_else(|_| {
            panic!(
                "{} is not below mount {}",
                path.display(),
                mount.target.display()
            )
        });
        mount.upper.join(relative)
    }

    fn write_upper(
        backend: &FilesystemBackend,
        environment_id: &str,
        path: &Path,
        content: &str,
    ) {
        let target = upper_at(backend, environment_id, path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(target, content).unwrap();
    }

    fn read_upper_layer_at(
        backend: &FilesystemBackend,
        environment_id: &str,
        directory: &Path,
    ) -> View {
        let target = upper_at(backend, environment_id, directory);
        read_layer(&target).unwrap_or_default()
    }

    fn spawn_in_environment(
        backend: &FilesystemBackend,
        environment_id: &str,
        cwd: &Path,
        script: &str,
    ) -> (crate::process::ProcessBackend, u32, u64) {
        let launch = AgentLaunch {
            command: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
            cwd: cwd.to_path_buf(),
            log: cwd.join("agent.log"),
            instance_id: None,
            env: BTreeMap::new(),
        };
        let context = BackendContext {
            environment_id: environment_id.to_string(),
            ..BackendContext::default()
        };
        let process = crate::process::ProcessBackend::new(
            "process",
            backend.bubblewrap_path(),
        );
        let fragment = backend.launch_fragment(&context, &launch).unwrap();
        process.launch(&context, &launch, &fragment).unwrap();
        let handle = process.process_handle(environment_id).unwrap();
        (process, handle.inner_pid, handle.mount_namespace)
    }

    fn run_in_environment(
        backend: &FilesystemBackend,
        environment_id: &str,
        cwd: &Path,
        script: &str,
    ) -> bool {
        let (process, _, _) = spawn_in_environment(backend, environment_id, cwd, script);
        let status = process.wait(environment_id).unwrap();
        let _ = process.cleanup(environment_id);
        status.success()
    }

    #[test]
    fn overlay_diff_translates_whiteout_opaque_redirect_and_metacopy() {
        let root = temp();
        let base = root.path().join("base");
        let upper = root.path().join("upper");
        write(&base, "modified.txt", "base");
        write(&base, "deleted.txt", "gone");
        write(&base, "opaque/old.txt", "old");
        write(&base, "old/kept.txt", "same");
        write(&base, "meta.txt", "same");

        write(&upper, "modified.txt", "new");
        write(&upper, "deleted.txt", "ignored");
        set_test_xattr(&upper.join("deleted.txt"), "user.overlay.whiteout", b"y");
        write(&upper, "opaque/new.txt", "new");
        set_test_xattr(&upper.join("opaque"), "user.overlay.opaque", b"y");
        write(&upper, "new/kept.txt", "same");
        set_test_xattr(&upper.join("new"), "user.overlay.redirect", b"old");
        write(&upper, "meta.txt", "");
        set_test_xattr(&upper.join("meta.txt"), "user.overlay.metacopy", b"y");

        let mut delta = View::new();
        let redirects = apply_layer(&mut delta, &upper).unwrap();
        let changes = changes_from_delta(&delta, &redirects);

        assert!(changes.iter().any(|change| change.path == "modified.txt" && change.kind == "change"));
        assert!(changes.iter().any(|change| change.path == "deleted.txt" && change.kind == "delete"));
        assert!(changes.iter().any(|change| change.path == "opaque/new.txt" && change.kind == "change"));
        assert!(changes.iter().any(|change| change.path == "new" && change.kind == "rename" && change.old_path.as_deref() == Some("old")));
        // The delta layer contains a tombstone, not just an absent path.
        assert!(delta.get("deleted.txt").is_some_and(Node::is_whiteout));
        // Content was never loaded into the view.
        assert!(delta.values().all(|node| node.source.is_some() || node.is_whiteout()));
    }

    #[test]
    fn merge_delta_reports_conflicts_and_accepts_independent_changes() {
        let base = Node {
            mode: libc::S_IFREG | 0o644,
            size: 4,
            mtime_sec: 1,
            mtime_nsec: 0,
            inode: 1,
            kind: NodeKind::File,
            source: Some(PathBuf::from("/base/a")),
            opaque: false,
            redirect: None,
            metacopy: false,
        };
        let base_maps = vec![View::from([
            ("a".into(), base.clone()),
            ("b".into(), base.clone()),
            ("c".into(), base.clone()),
        ])];
        let theirs = Node {
            mode: base.mode,
            size: 5,
            mtime_sec: 2,
            mtime_nsec: 0,
            inode: 2,
            kind: NodeKind::File,
            source: Some(PathBuf::from("/source/a")),
            opaque: false,
            redirect: None,
            metacopy: false,
        };
        let delta = View::from([
            ("a".into(), theirs.clone()),
            ("b".into(), Node {
                mode: base.mode,
                size: 0,
                mtime_sec: 0,
                mtime_nsec: 0,
                inode: 0,
                kind: NodeKind::Whiteout,
                source: None,
                opaque: false,
                redirect: None,
                metacopy: false,
            }),
            ("d".into(), theirs.clone()),
        ]);
        let target = View::from([
            ("a".into(), Node { size: 6, mtime_sec: 3, ..theirs.clone() }),
            ("b".into(), Node { size: 7, mtime_sec: 4, ..base.clone() }),
            ("d".into(), theirs.clone()),
        ]);
        let target_maps = vec![target.clone()];
        let conflicts = merge_delta(&delta, &[], &base_maps, &target_maps, &target);
        assert!(conflicts.iter().any(|value| value.path == "a" && value.kind == "content"));
        assert!(conflicts.iter().any(|value| value.path == "b" && value.kind == "delete_modify"));
        assert!(conflicts.iter().all(|value| value.path != "d"));
    }

    #[test]
    fn filesystem_operation_replays_same_generation_and_rejects_stale_generation() {
        let root = temp();
        let backend = new_backend(root.path());
        backend.establish("root").unwrap();
        let operation = Operation::FilesystemFork {
            parent_snapshot_id: "root".into(),
            child_snapshot_id: "child".into(),
            path_scope: vec![PathScope::Workspace],
        };
        let context = BackendContext {
            operation_id: "fork-1".into(),
            generation: 7,
            environment_id: "root".into(),
            ..BackendContext::default()
        };
        let cancel = CancelHandle::default();
        let first = backend
            .filesystem_operation(&context, &operation, &cancel)
            .unwrap();
        drop(backend);

        let backend = FilesystemBackend::new(FilesystemConfig::new(root.path().join("state"))).unwrap();
        assert!(backend.state("root").is_some());
        assert!(backend.state("child").is_some());
        let second = backend
            .filesystem_operation(&context, &operation, &cancel)
            .unwrap();
        assert_eq!(first.snapshot_id, second.snapshot_id);
        assert!(second.confirmation.snapshot_created);

        let stale = BackendContext {
            generation: 6,
            ..context.clone()
        };
        let error = backend
            .filesystem_operation(&stale, &operation, &cancel)
            .unwrap_err();
        assert!(error.message.contains("stale generation"));
    }

    #[test]
    fn composite_runtime_reaches_the_filesystem_data_plane() {
        use crate::runtime::{BackendConnection, BackendPolicy, CompositeRuntime};
        use std::sync::Arc;

        let root = temp();
        let filesystem = Arc::new(new_backend(root.path()));
        let runtime = CompositeRuntime::new(
            "runtime",
            vec![BackendConnection::new("filesystem", filesystem).unwrap()],
            BackendPolicy::single("filesystem"),
        )
        .unwrap();
        runtime.establish("root").unwrap();
        let operation = Operation::FilesystemFork {
            parent_snapshot_id: "root".into(),
            child_snapshot_id: "child".into(),
            path_scope: vec![PathScope::Workspace],
        };
        let context = BackendContext {
            operation_id: "composite-fork".into(),
            generation: 1,
            environment_id: "root".into(),
            ..BackendContext::default()
        };
        let result = runtime
            .filesystem_operation(&context, &operation, &CancelHandle::default())
            .unwrap();
        assert!(result.confirmation.snapshot_created);
        assert!(result.confirmation.metadata_durable);
    }

    #[test]
    fn backend_overlay_fork_diff_accept_discard_round_trip() {
        round_trip(temp(), false);
    }

    #[test]
    fn backend_btrfs_fork_diff_accept_discard_round_trip() {
        round_trip(btrfs_temp(), true);
    }

    fn round_trip(root: tempfile::TempDir, expect_btrfs: bool) {
        let backend = FilesystemBackend::new(FilesystemConfig::new(root.path().join("state"))).unwrap();
        assert!(
            backend.usable,
            "bubblewrap overlay probe failed; filesystem tests require a working sandbox"
        );
        if expect_btrfs && backend.mechanism() != FilesystemMechanism::Btrfs {
            eprintln!(
                "SKIP backend_btrfs_fork_diff_accept_discard_round_trip: btrfs needs create+snapshot+incremental_diff; selected {}",
                backend.mechanism().as_str()
            );
            return;
        }
        backend.establish("root").unwrap();
        let workspace = root.path().join("base");
        fs::create_dir_all(&workspace).unwrap();
        write_upper(
            &backend,
            "root",
            &workspace.join("parent.txt"),
            "parent",
        );

        let fork = backend
            .fork("root", "child", &[PathScope::Workspace])
            .unwrap();
        assert!(fork.confirmation.snapshot_created);
        assert!(fork.confirmation.metadata_durable);
        assert!(!fork.confirmation.writers_quiesced);
        let base = FilesystemBackend::base_snapshot_id_for_child("child");
        write_upper(
            &backend,
            "child",
            &workspace.join("child.txt"),
            "child",
        );

        if expect_btrfs {
            let diff = backend.diff(&base, "child", None, &[PathScope::Workspace]);
            assert!(
                diff.is_err(),
                "btrfs diff must fail explicitly without generation privileges"
            );
            assert!(diff.unwrap_err().message.contains("btrfs incremental diff"));
            let accept = backend.accept(
                &base,
                "child",
                "root",
                &[PathScope::Workspace],
                None,
                None,
            );
            assert!(accept.is_err());
        } else {
            let diff = backend
                .diff(&base, "child", None, &[PathScope::Workspace])
                .unwrap();
            assert!(diff.confirmation.metadata_durable);
            let document: DiffDocument =
                serde_json::from_slice(&fs::read(diff.diff_ref.unwrap()).unwrap()).unwrap();
            let child_path = workspace.join("child.txt").display().to_string();
            assert!(document
                .changes
                .iter()
                .any(|change| change.path == child_path && change.kind == "change"));

            let mark = backend
                .mark("child", "before-more", &[PathScope::Workspace])
                .unwrap();
            assert!(mark.confirmation.snapshot_created);
            write_upper(
                &backend,
                "child",
                &workspace.join("after-mark.txt"),
                "after",
            );
            let mark_diff = backend
                .diff(
                    mark.snapshot_id.as_deref().unwrap(),
                    "child",
                    None,
                    &[PathScope::Workspace],
                )
                .unwrap();
            let mark_document: DiffDocument =
                serde_json::from_slice(&fs::read(mark_diff.diff_ref.unwrap()).unwrap()).unwrap();
            let after_path = workspace.join("after-mark.txt").display().to_string();
            let child_path = workspace.join("child.txt").display().to_string();
            assert!(mark_document
                .changes
                .iter()
                .any(|change| change.path == after_path && change.kind == "change"));
            assert!(mark_document
                .changes
                .iter()
                .all(|change| change.path != child_path));

            backend.suspend("root").unwrap();
            let accept = backend
                .accept(&base, "child", "root", &[PathScope::Workspace], None, None)
                .unwrap();
            assert!(accept.conflicts.is_empty());
            assert!(accept.confirmation.published);
            assert!(!accept.confirmation.target_paused);
            assert!(upper_at(&backend, "root", &workspace.join("child.txt")).exists());
        }

        backend.terminate("child").unwrap();
        let discard = backend
            .discard("child", &[PathScope::Workspace])
            .unwrap();
        assert!(discard.confirmation.refs_released);
        assert!(discard.confirmation.gc_eligible);
        assert!(backend.state("child").is_none());
        wait_removed(&root.path().join("environments").join("child"));

        backend.terminate("root").unwrap();
        backend.cleanup("root").unwrap();
        wait_removed(&root.path().join("environments").join("root"));
        wait_until(|| {
            !path_exists(&root.path().join("snapshots").join(&base))
                && !path_exists(&root.path().join("layers").join(&base))
        });
        wait_until(|| {
            fs::read_dir(root.path().join("trash"))
                .map(|mut entries| entries.next().is_none())
                .unwrap_or(true)
        });
    }

    /// Real bubblewrap end-to-end: a whiteout created by `rm` must not
    /// resurrect the Base file in a forked child, and accept must propagate
    /// the deletion to the target.
    #[test]
    fn real_overlay_whiteout_survives_fork_diff_and_accept() {
        let root = btrfs_temp();
        let base = root.path().join("base");
        write(&base, "x", "base");
        let backend = new_overlay_backend(root.path());
        backend.establish("root").unwrap();
        backend
            .fork("root", "child", &[PathScope::Workspace])
            .unwrap();

        assert!(run_in_environment(&backend, "child", &base, "rm -f x"));
        assert_eq!(fs::read(base.join("x")).unwrap(), b"base");
        let child_layer = read_upper_layer_at(&backend, "child", &base);
        assert!(child_layer.get("x").is_some_and(Node::is_whiteout));
        assert!(!run_in_environment(&backend, "child", &base, "test -e x"));

        let base_snapshot = FilesystemBackend::base_snapshot_id_for_child("child");
        let diff = backend
            .diff(&base_snapshot, "child", None, &[PathScope::Workspace])
            .unwrap();
        let document: DiffDocument =
            serde_json::from_slice(&fs::read(diff.diff_ref.unwrap()).unwrap()).unwrap();
        let deleted = base.join("x").display().to_string();
        assert!(document
            .changes
            .iter()
            .any(|change| change.path == deleted && change.kind == "delete"));

        backend.suspend("root").unwrap();
        let accept = backend
            .accept(
                &base_snapshot,
                "child",
                "root",
                &[PathScope::Workspace],
                None,
                None,
            )
            .unwrap();
        assert!(accept.conflicts.is_empty());
        assert!(accept.confirmation.published);
        assert!(!accept.confirmation.target_paused);

        backend.resume("root").unwrap();
        assert!(!run_in_environment(&backend, "root", &base, "test -e x"));
        assert_eq!(fs::read(base.join("x")).unwrap(), b"base");

        backend.terminate("child").unwrap();
        backend.discard("child", &[PathScope::Workspace]).unwrap();
        backend.terminate("root").unwrap();
        backend.cleanup("root").unwrap();
        wait_until(|| {
            fs::read_dir(root.path().join("trash"))
                .map(|mut entries| entries.next().is_none())
                .unwrap_or(true)
        });
    }

    #[test]
    fn live_target_accept_replays_through_inner_sandbox_pid() {
        let root = btrfs_temp();
        let base = root.path().join("base");
        write(&base, "x", "base");
        let backend = new_overlay_backend(root.path());
        backend.establish("root").unwrap();
        backend
            .fork("root", "child", &[PathScope::Workspace])
            .unwrap();
        assert!(run_in_environment(&backend, "child", &base, "rm -f x"));
        let base_snapshot = FilesystemBackend::base_snapshot_id_for_child("child");
        let (target, target_pid, target_ns) = spawn_in_environment(
            &backend,
            "root",
            &base,
            "while [ -e x ]; do sleep 0.05; done; exit 0",
        );
        backend.suspend("root").unwrap();

        // The outer bwrap pid is in the host mount namespace and must be rejected.
        let outer = std::process::id();
        let error = backend
            .accept(
                &base_snapshot,
                "child",
                "root",
                &[PathScope::Workspace],
                Some(outer),
                Some(target_ns),
            )
            .unwrap_err();
        assert!(error.message.contains("host mount namespace"));

        let accept = backend
            .accept(
                &base_snapshot,
                "child",
                "root",
                &[PathScope::Workspace],
                Some(target_pid),
                Some(target_ns),
            )
            .unwrap();
        assert!(accept.confirmation.published);

        let mut exited = false;
        for _ in 0..200 {
            if let Some(status) = target.try_wait("root").unwrap() {
                assert!(status.success());
                exited = true;
                break;
            }
            thread::sleep(std::time::Duration::from_millis(10));
        }
        if !exited {
            let _ = target.stop("root");
            panic!("target process did not observe the accepted deletion");
        }
        assert_eq!(fs::read(base.join("x")).unwrap(), b"base");
        backend.terminate("child").unwrap();
        backend.discard("child", &[PathScope::Workspace]).unwrap();
        backend.terminate("root").unwrap();
        backend.cleanup("root").unwrap();
    }

    #[test]
    fn authority_surface_comparison() {
        let root = btrfs_temp();
        let host_base = root.path().join("host-base");
        let sandbox_base = root.path().join("sandbox-base");
        for base in [&host_base, &sandbox_base] {
            fs::create_dir_all(base).unwrap();
            fs::create_dir_all(base.join("lowerdir")).unwrap();
            write(base, "lowerdir/kept.txt", "kept");
            write(base, "cwd-file", "cwd");
        }
        let token = format!("ascout-audit-{}", std::process::id());
        let report_name = "authority-report.txt";
        let watcher = r#"import ctypes, os, time
libc = ctypes.CDLL("libc.so.6", use_errno=True)
fd = libc.inotify_init1(os.O_NONBLOCK)
wd = libc.inotify_add_watch(fd, b"watch.txt", 0x2)
if wd < 0:
    print("watchfail")
    raise SystemExit
with open("watch.txt", "a") as handle:
    handle.write("x")
time.sleep(0.2)
try:
    data = os.read(fd, 4096)
except BlockingIOError:
    data = b""
print("ok" if data else "fail")
"#;
        for base in [&host_base, &sandbox_base] {
            fs::write(base.join("watch.py"), watcher).unwrap();
            fs::write(base.join("watch.txt"), b"watch").unwrap();
        }
        let script = format!(
            "set +e\n             echo cwd_write=$(echo x > cwd-new && echo ok || echo fail)\n             echo tmp_write=$(echo x > /tmp/{token} && echo ok || echo fail)\n             echo vartmp_write=$(echo x > /var/tmp/{token} && echo ok || echo fail)\n             echo home_cache_write=$(mkdir -p \"$HOME/.cache\" 2>/dev/null; echo x > \"$HOME/.cache/{token}\" && echo ok || echo fail)\n             echo mv_lower=$(mv lowerdir lowerdir2 && echo ok || echo fail)\n             echo hardlink=$(ln cwd-file hardlink && echo ok || echo fail)\n             echo symlink=$(ln -s cwd-file symlink && echo ok || echo fail)\n             echo otmpfile=$(python3 -c 'import os; fd=os.open(\".\",os.O_TMPFILE|os.O_WRONLY,0o600); os.write(fd,b\"x\"); print(\"ok\")' 2>/dev/null || echo fail)\n             echo mmap=$(python3 -c 'import mmap,os; f=os.open(\"cwd-file\",os.O_RDONLY); m=mmap.mmap(f,0,prot=mmap.PROT_READ); print(\"ok\" if m[0:] else \"fail\")' 2>/dev/null || echo fail)\n             echo flock=$(flock cwd-file true && echo ok || echo fail)\n             echo xattr=$(python3 -c 'import os; os.setxattr(\"cwd-file\",\"user.ascout\",b\"1\")' 2>/dev/null && echo ok || echo fail)\n             echo chmod=$(chmod 600 cwd-file && echo ok || echo fail)\n             echo utimes=$(touch -d '2000-01-01' cwd-file && echo ok || echo fail)\n             echo fsync=$(python3 -c 'import os; f=os.open(\"cwd-file\",os.O_RDWR); os.fsync(f)' 2>/dev/null && echo ok || echo fail)\n             echo inotify=$(python3 watch.py 2>/dev/null || echo fail)\n             echo stat=$(stat -c '%d:%i:%s' cwd-file 2>/dev/null || echo fail)\n             echo statfs=$(stat -f -c '%T' . 2>/dev/null || echo fail)\n             echo id=$(id -u):$(id -g)\n             echo devshm=$(echo x > /dev/shm/{token} && echo ok || echo fail)\n             echo hostname=$(hostname 2>/dev/null || echo fail)\n             echo ps=$(ps -e 2>/dev/null | wc -l)\n             echo home=$HOME\n             echo pwd=$(pwd)\n",
        );
        let script = format!("(\n{script}\n) > {report_name} 2>&1");
        let host = Command::new("/bin/sh")
            .arg("-c")
            .arg(&script)
            .current_dir(&host_base)
            .env("HOME", "/home/betmul")
            .output()
            .unwrap();
        assert!(host.status.success());
        let host_report = fs::read_to_string(host_base.join(report_name)).unwrap();

        // The host run above legitimately created the same path.  Remove it so
        // the sandbox check proves its write stayed in the branch upper.
        let _ = fs::remove_file(format!("/home/betmul/.cache/{token}"));
        let backend = new_overlay_backend(root.path());
        backend.establish("root").unwrap();
        let launch = AgentLaunch {
            command: "/bin/sh".into(),
            args: vec!["-c".into(), script],
            cwd: sandbox_base.clone(),
            log: sandbox_base.join("agent.log"),
            instance_id: None,
            env: BTreeMap::from([("HOME".to_string(), "/home/betmul".to_string())]),
        };
        let context = BackendContext {
            environment_id: "root".into(),
            ..BackendContext::default()
        };
        let process = crate::process::ProcessBackend::new("process", backend.bubblewrap_path());
        let fragment = backend.launch_fragment(&context, &launch).unwrap();
        process.launch(&context, &launch, &fragment).unwrap();
        process.wait("root").unwrap();
        let sandbox_report = fs::read_to_string(upper_at(
            &backend,
            "root",
            &sandbox_base.join(report_name),
        ))
        .unwrap_or_default();
        eprintln!("== host authority report ==\n{host_report}");
        eprintln!("== sandbox authority report ==\n{sandbox_report}");
        let map = |text: &str| -> BTreeMap<String, String> {
            text.lines()
                .filter_map(|line| line.split_once('='))
                .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
                .collect()
        };
        let host_map = map(&host_report);
        let sandbox_map = map(&sandbox_report);
        for key in [
            "cwd_write",
            "tmp_write",
            "vartmp_write",
            "hardlink",
            "symlink",
            "otmpfile",
            "mmap",
            "flock",
            "xattr",
            "chmod",
            "utimes",
            "fsync",
            "inotify",
            "devshm",
        ] {
            assert_eq!(
                host_map.get(key).map(String::as_str),
                Some("ok"),
                "host operation {key} failed"
            );
            assert_eq!(
                sandbox_map.get(key).map(String::as_str),
                Some("ok"),
                "sandbox operation {key} failed"
            );
        }
        assert_eq!(
            sandbox_map.get("home_cache_write").map(String::as_str),
            Some("ok"),
            "~/.cache must be writable inside the sandbox like on the host"
        );
        assert!(
            !Path::new(&format!("/home/betmul/.cache/{token}")).exists(),
            "sandbox write to ~/.cache reached the host"
        );
        let _ = fs::remove_file(format!("/tmp/{token}"));
        let _ = fs::remove_file(format!("/var/tmp/{token}"));
        let _ = fs::remove_file(format!("/dev/shm/{token}"));
        let _ = fs::remove_file(format!("/home/betmul/.cache/{token}"));
        process.cleanup("root").unwrap();
        backend.terminate("root").unwrap();
        backend.cleanup("root").unwrap();
    }

    #[test]
    fn process_freeze_thaw_stops_and_resumes_writer() {
        let root = btrfs_temp();
        let base = root.path().join("base");
        fs::create_dir_all(&base).unwrap();
        let backend = new_overlay_backend(root.path());
        backend.establish("root").unwrap();
        let (process, _, _) = spawn_in_environment(
            &backend,
            "root",
            &base,
            "i=0; while :; do i=$((i+1)); echo $i > counter; sleep 0.02; done",
        );
        let counter = upper_at(&backend, "root", &base.join("counter"));
        wait_until(|| counter.exists());
        eprintln!(
            "freeze mechanism: {}",
            process.freeze_mechanism().as_str()
        );
        process.freeze("root").unwrap();
        let frozen = fs::read_to_string(&counter).unwrap();
        thread::sleep(std::time::Duration::from_millis(150));
        let still = fs::read_to_string(&counter).unwrap();
        assert_eq!(frozen, still, "frozen process kept writing");
        process.thaw("root").unwrap();
        thread::sleep(std::time::Duration::from_millis(150));
        let resumed = fs::read_to_string(&counter).unwrap();
        assert_ne!(still, resumed, "thawed process did not resume");
        process.stop("root").unwrap();
        process.cleanup("root").unwrap();
        backend.terminate("root").unwrap();
        backend.cleanup("root").unwrap();
    }

    #[test]
    fn process_group_fallback_freezes_and_reports_mechanism() {
        let root = btrfs_temp();
        let base = root.path().join("base");
        fs::create_dir_all(&base).unwrap();
        let backend = new_overlay_backend(root.path());
        backend.establish("root").unwrap();
        let launch = AgentLaunch {
            command: "/bin/sh".into(),
            args: vec![
                "-c".into(),
                "i=0; while :; do i=$((i+1)); echo $i > counter; sleep 0.02; done".into(),
            ],
            cwd: base.clone(),
            log: base.join("agent.log"),
            instance_id: None,
            env: BTreeMap::new(),
        };
        let context = BackendContext {
            environment_id: "root".into(),
            ..BackendContext::default()
        };
        let process = crate::process::ProcessBackend::new_process_group(
            "process",
            backend.bubblewrap_path(),
        );
        assert_eq!(
            process.freeze_mechanism(),
            crate::process::FreezeMechanism::ProcessGroup
        );
        let fragment = backend.launch_fragment(&context, &launch).unwrap();
        process.launch(&context, &launch, &fragment).unwrap();
        let counter = upper_at(&backend, "root", &base.join("counter"));
        wait_until(|| counter.exists());
        process.freeze("root").unwrap();
        let frozen = fs::read_to_string(&counter).unwrap();
        thread::sleep(std::time::Duration::from_millis(150));
        let still = fs::read_to_string(&counter).unwrap();
        assert_eq!(frozen, still);
        process.thaw("root").unwrap();
        thread::sleep(std::time::Duration::from_millis(150));
        assert_ne!(still, fs::read_to_string(&counter).unwrap());
        process.stop("root").unwrap();
        process.cleanup("root").unwrap();
        backend.terminate("root").unwrap();
        backend.cleanup("root").unwrap();
    }

    #[test]
    fn state_directory_is_hidden_inside_sandbox() {
        let root = btrfs_temp();
        let base = root.path().join("base");
        fs::create_dir_all(&base).unwrap();
        let backend = new_overlay_backend(root.path());
        backend.establish("root").unwrap();
        let state = root.path().join("state");
        let script = format!(
            "test -z \"$(ls -A '{}' 2>/dev/null)\" &&              ! test -e '{}/environments' &&              ! test -e '{}/metadata' &&              ! test -e '{}/snapshots' &&              ! (cd '{}' && touch escape) &&              test -z \"$(ls -A '{}/../state' 2>/dev/null)\" &&              test -z \"$(ls -A /proc/self/root{} 2>/dev/null)\"",
            state.display(),
            state.display(),
            state.display(),
            state.display(),
            state.display(),
            state.display(),
            state.display(),
        );
        assert!(
            run_in_environment(&backend, "root", &base, &script),
            "state directory was visible or writable inside the sandbox"
        );
        backend.terminate("root").unwrap();
        backend.cleanup("root").unwrap();
    }

    #[test]
    fn home_cache_and_config_writes_stay_in_home_upper() {
        let root = btrfs_temp();
        let base = root.path().join("base");
        fs::create_dir_all(&base).unwrap();
        let backend = new_overlay_backend(root.path());
        backend.establish("root").unwrap();
        let token = format!("ascout-home-{}", std::process::id());
        let cache = PathBuf::from(format!("/home/betmul/.cache/{token}"));
        let config = PathBuf::from(format!("/home/betmul/.config/{token}"));
        let _ = fs::remove_file(&cache);
        let _ = fs::remove_file(&config);
        let script = format!(
            "mkdir -p \"$HOME/.cache\" \"$HOME/.config\" &&              echo cache > \"$HOME/.cache/{token}\" &&              echo config > \"$HOME/.config/{token}\""
        );
        assert!(
            run_in_environment(&backend, "root", &base, &script),
            "sandbox could not write $HOME/.cache or $HOME/.config"
        );
        assert!(!cache.exists(), "sandbox write to ~/.cache reached the host");
        assert!(!config.exists(), "sandbox write to ~/.config reached the host");
        assert!(
            upper_at(&backend, "root", &cache).exists(),
            "~/.cache write did not reach the /home branch upper"
        );
        assert!(
            upper_at(&backend, "root", &config).exists(),
            "~/.config write did not reach the /home branch upper"
        );
        backend.terminate("root").unwrap();
        backend.cleanup("root").unwrap();
    }

    #[test]
    fn writes_outside_cwd_reach_branch_uppers_not_host() {
        let root = btrfs_temp();
        let base = root.path().join("base");
        fs::create_dir_all(&base).unwrap();
        let backend = new_overlay_backend(root.path());
        backend.establish("root").unwrap();
        let token = format!("ascout-outside-{}", std::process::id());
        let tmp = format!("/tmp/{token}");
        let vartmp = format!("/var/tmp/{token}");
        let srv = format!("/srv/{token}");
        let script = format!(
            "echo tmp > {tmp}; echo vartmp > {vartmp}; echo srv > {srv}"
        );
        assert!(run_in_environment(&backend, "root", &base, &script));
        assert!(!Path::new(&tmp).exists(), "host /tmp was written");
        assert!(!Path::new(&vartmp).exists(), "host /var/tmp was written");
        assert!(!Path::new(&srv).exists(), "host /srv was written");
        let state = backend.lock().unwrap();
        let upper_for = |target: &str| {
            state.environments["root"]
                .mount_by_target(Path::new(target))
                .unwrap_or_else(|| panic!("missing mount unit {target}"))
                .upper
                .clone()
        };
        assert!(upper_for("/tmp").join(&token).exists());
        assert!(upper_for("/var/tmp").join(&token).exists());
        assert!(upper_for("/srv").join(&token).exists());
        drop(state);
        backend.terminate("root").unwrap();
        backend.cleanup("root").unwrap();
    }

    #[test]
    fn extra_mount_fork_diff_accept_discard_round_trip() {
        let root = btrfs_temp();
        let base = root.path().join("base");
        fs::create_dir_all(&base).unwrap();
        let backend = new_overlay_backend(root.path());
        let candidates = backend.overlayable_mounts();
        let Some(target) = ["/var/tmp", "/srv"]
            .into_iter()
            .map(PathBuf::from)
            .find(|target| candidates.iter().any(|candidate| candidate == target))
        else {
            eprintln!(
                "SKIP extra_mount_fork_diff_accept_discard_round_trip: no second writable overlayable mount (candidates: {:?})",
                candidates
            );
            return;
        };
        if unsafe { libc::access(CString::new(target.as_os_str().as_bytes()).unwrap().as_ptr(), libc::W_OK) }
            != 0
        {
            eprintln!(
                "SKIP extra_mount_fork_diff_accept_discard_round_trip: {} is not writable",
                target.display()
            );
            return;
        }
        eprintln!(
            "extra_mount_fork_diff_accept_discard_round_trip: using {}",
            target.display()
        );
        let token = format!("ascout-f1-{}", std::process::id());
        let parent_file = target.join(format!("{token}-a"));
        let child_file = target.join(format!("{token}-b"));
        let lower_file = target.join(format!("{token}-lower"));
        let _ = fs::remove_file(&parent_file);
        let _ = fs::remove_file(&child_file);
        let _ = fs::remove_file(&lower_file);
        fs::write(&lower_file, b"lower").unwrap();

        backend.establish("root").unwrap();
        let parent_script = format!("echo a > {}", parent_file.display());
        assert!(
            run_in_environment(&backend, "root", &base, &parent_script),
            "parent could not write {}",
            parent_file.display()
        );
        assert!(!parent_file.exists(), "parent write reached the host");
        backend.fork("root", "child", &[]).unwrap();

        assert!(
            run_in_environment(
                &backend,
                "child",
                &base,
                &format!("test -f {}", parent_file.display()),
            ),
            "child did not inherit the parent write in {}",
            target.display()
        );
        let child_script = format!(
            "echo b > {}; rm -f {}",
            child_file.display(),
            lower_file.display()
        );
        assert!(run_in_environment(&backend, "child", &base, &child_script));
        assert!(!child_file.exists(), "child write reached the host");
        assert!(lower_file.exists(), "child deleted the host lower file");

        let base_snapshot = FilesystemBackend::base_snapshot_id_for_child("child");
        let diff = backend.diff(&base_snapshot, "child", None, &[]).unwrap();
        let document: DiffDocument =
            serde_json::from_slice(&fs::read(diff.diff_ref.unwrap()).unwrap()).unwrap();
        assert!(
            document
                .changes
                .iter()
                .any(|change| change.path == child_file.display().to_string()
                    && change.kind == "change"),
            "diff did not report {}: {:?}",
            child_file.display(),
            document.changes
        );
        assert!(
            document
                .changes
                .iter()
                .any(|change| change.path == lower_file.display().to_string()
                    && change.kind == "delete"),
            "diff did not report delete of {}: {:?}",
            lower_file.display(),
            document.changes
        );

        backend.suspend("root").unwrap();
        let accept = backend
            .accept(&base_snapshot, "child", "root", &[], None, None)
            .unwrap();
        assert!(accept.conflicts.is_empty(), "{:?}", accept.conflicts);
        assert!(accept.confirmation.published);
        backend.resume("root").unwrap();
        assert!(
            run_in_environment(
                &backend,
                "root",
                &base,
                &format!(
                    "test -f {} && ! test -e {}",
                    child_file.display(),
                    lower_file.display()
                ),
            ),
            "target did not observe accept of {} and deletion of {}",
            child_file.display(),
            lower_file.display()
        );
        assert!(!child_file.exists(), "accept wrote the host target mount");
        assert!(lower_file.exists(), "accept deleted the host lower file");

        // mark must also snapshot every mount, not only workspace/tmp.
        let mark = backend.mark("child", "extra-mark", &[]).unwrap();
        let mark_file = target.join(format!("{token}-mark"));
        let _ = fs::remove_file(&mark_file);
        assert!(run_in_environment(
            &backend,
            "child",
            &base,
            &format!("echo marked > {}", mark_file.display()),
        ));
        let mark_diff = backend
            .diff(mark.snapshot_id.as_deref().unwrap(), "child", None, &[])
            .unwrap();
        let mark_document: DiffDocument =
            serde_json::from_slice(&fs::read(mark_diff.diff_ref.unwrap()).unwrap()).unwrap();
        assert!(
            mark_document
                .changes
                .iter()
                .any(|change| change.path == mark_file.display().to_string()),
            "mark diff did not cover {}: {:?}",
            mark_file.display(),
            mark_document.changes
        );
        assert!(
            mark_document
                .changes
                .iter()
                .all(|change| change.path != child_file.display().to_string()),
            "mark diff included a pre-mark change: {:?}",
            mark_document.changes
        );

        let child_upper = {
            let state = backend.lock().unwrap();
            let mount = state.environments["child"]
                .mount_by_target(&target)
                .expect("child mount unit")
                .clone();
            mount.upper.join(format!("{token}-b"))
        };
        assert!(child_upper.exists(), "child upper did not contain the write");
        backend.terminate("child").unwrap();
        backend.discard("child", &[]).unwrap();
        wait_until(|| !child_upper.exists());
        assert!(!child_upper.exists(), "discard left extra-mount upper data");
        let _ = fs::remove_file(&parent_file);
        let _ = fs::remove_file(&child_file);
        let _ = fs::remove_file(&lower_file);
        let _ = fs::remove_file(&mark_file);
        backend.terminate("root").unwrap();
        backend.cleanup("root").unwrap();
    }

    #[test]
    fn live_replay_blocks_absolute_symlink_escape() {
        let root = btrfs_temp();
        let base = root.path().join("base");
        fs::create_dir_all(&base).unwrap();
        let sensitive = base.join("sensitive");
        fs::create_dir_all(&sensitive).unwrap();
        write(&sensitive, "secret", "host");
        std::os::unix::fs::symlink(&sensitive, base.join("escape")).unwrap();
        let mut config = FilesystemConfig::new(root.path().join("state"));
        config.force_overlay = true;
        config.isolate_unmanaged_mounts = false;
        let backend = FilesystemBackend::new(config).unwrap();
        assert!(backend.usable);
        backend.establish("root").unwrap();
        let (target, target_pid, _) =
            spawn_in_environment(&backend, "root", &base, "sleep 5");
        let workspace = {
            let state = backend.lock().unwrap();
            state.environments["root"].workspace_root.clone().unwrap()
        };
        let safe = SafeRoot::open(target_pid).unwrap();
        let source = root.path().join("source-file");
        fs::write(&source, b"attack").unwrap();
        let relative = workspace.strip_prefix("/").unwrap();
        let target_path = format!(
            "{}/escape/pwn",
            relative.to_string_lossy()
        );
        let result = safe.write_file(&target_path, &source, 0o644);
        // The absolute symlink must resolve inside the sandbox root, not the
        // caller's root.  Writing through it either fails or stays inside the
        // sandbox; it must never reach the host sensitive directory.
        assert!(
            result.is_err() || !sensitive.join("pwn").exists(),
            "write escaped to the host through an absolute symlink"
        );
        assert!(!sensitive.join("pwn").exists());
        assert_eq!(fs::read(sensitive.join("secret")).unwrap(), b"host");
        let _ = target.stop("root");
        backend.terminate("root").unwrap();
        backend.cleanup("root").unwrap();
    }

    /// A still-running parent writes after fork.  Its upper and mount are not
    /// touched by fork, so the child's independent upper copy must not see the
    /// later write.
    #[test]
    fn overlay_fork_isolates_parent_writes_after_the_copy() {
        let root = btrfs_temp();
        let base = root.path().join("base");
        fs::create_dir_all(&base).unwrap();
        let backend = new_overlay_backend(root.path());
        backend.establish("root").unwrap();
        let (parent, _, _) = spawn_in_environment(
            &backend,
            "root",
            &base,
            "touch ready; sleep 0.5; echo late > after.txt; sleep 3",
        );
        let parent_upper = upper_at(&backend, "root", &base);
        wait_until(|| parent_upper.join("ready").exists());

        backend
            .fork("root", "child", &[PathScope::Workspace])
            .unwrap();
        wait_until(|| parent_upper.join("after.txt").exists());

        assert!(
            run_in_environment(&backend, "child", &base, "test ! -e after.txt"),
            "child saw a parent write that happened after fork"
        );

        let _ = parent.stop("root");
        backend.terminate("child").unwrap();
        backend.discard("child", &[PathScope::Workspace]).unwrap();
        backend.terminate("root").unwrap();
        backend.cleanup("root").unwrap();
        wait_until(|| {
            fs::read_dir(root.path().join("trash"))
                .map(|mut entries| entries.next().is_none())
                .unwrap_or(true)
        });
    }

    #[test]
    fn legacy_scope_metadata_is_migrated_to_mount_units() {
        let root = btrfs_temp();
        let backend = new_overlay_backend(root.path());
        backend.establish("root").unwrap();
        let metadata_path = root
            .path()
            .join("state/metadata/environments/root.json");
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&metadata_path).unwrap()).unwrap();
        value.as_object_mut().unwrap().remove("mounts");
        value["lower"] = serde_json::json!({ "tmp": ["/legacy/layer"] });
        value["workspace_root"] = serde_json::Value::Null;
        fs::write(&metadata_path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
        drop(backend);

        let backend = FilesystemBackend::new(FilesystemConfig::new(root.path().join("state"))).unwrap();
        let state = backend.lock().unwrap();
        let environment = &state.environments["root"];
        let tmp = environment
            .mount_by_target(Path::new("/tmp"))
            .expect("tmp mount migrated");
        assert_eq!(tmp.layer_key, "tmp");
        assert_eq!(tmp.lower, vec![PathBuf::from("/legacy/layer")]);
        drop(state);
        drop(backend);

        // A legacy workspace layer without workspace_root cannot be mapped to
        // an absolute mount point and must fail loudly instead of being lost.
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&metadata_path).unwrap()).unwrap();
        value.as_object_mut().unwrap().remove("mounts");
        value["lower"] = serde_json::json!({ "workspace": ["/legacy/layer"] });
        value["workspace_root"] = serde_json::Value::Null;
        fs::write(&metadata_path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
        let error = match FilesystemBackend::new(FilesystemConfig::new(root.path().join("state"))) {
            Ok(_) => panic!("legacy workspace metadata without workspace_root must fail"),
            Err(error) => error,
        };
        assert!(
            error.message.contains("workspace_root"),
            "unexpected migration error: {}",
            error.message
        );
    }

    fn priv_enabled() -> bool {
        std::env::var("ASCOUT_PRIV_TESTS").ok().as_deref() == Some("1")
    }

    fn priv_state_root() -> PathBuf {
        let root = std::env::var_os("ASCOUT_PRIV_STATE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/var/tmp/ascout-priv-check"));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn priv_btrfs_fork_diff_accept_discard() {
        if !priv_enabled() {
            eprintln!("SKIP priv_btrfs_fork_diff_accept_discard: ASCOUT_PRIV_TESTS != 1");
            return;
        }
        let root = tempfile::Builder::new()
            .prefix("priv-btrfs-")
            .tempdir_in(priv_state_root())
            .unwrap();
        let backend = FilesystemBackend::new(FilesystemConfig::new(root.path().join("state"))).unwrap();
        assert_eq!(
            backend.mechanism(),
            FilesystemMechanism::Btrfs,
            "privileged environment must select the btrfs data plane"
        );
        assert!(backend.fast_delete(), "privileged btrfs must support strict delete");
        backend.establish("root").unwrap();
        let workspace = root.path().join("base");
        fs::create_dir_all(&workspace).unwrap();
        write_upper(&backend, "root", &workspace.join("parent.txt"), "parent");
        backend.fork("root", "child", &[PathScope::Workspace]).unwrap();
        write_upper(&backend, "child", &workspace.join("child.txt"), "child");
        let base = FilesystemBackend::base_snapshot_id_for_child("child");
        let diff = backend
            .diff(&base, "child", None, &[PathScope::Workspace])
            .unwrap();
        let document: DiffDocument =
            serde_json::from_slice(&fs::read(diff.diff_ref.unwrap()).unwrap()).unwrap();
        let child_path = workspace.join("child.txt").display().to_string();
        assert!(
            document
                .changes
                .iter()
                .any(|change| change.path == child_path && change.kind == "change"),
            "btrfs diff did not report {}: {:?}",
            workspace.join("child.txt").display(),
            document.changes
        );
        backend.suspend("root").unwrap();
        let accept = backend
            .accept(&base, "child", "root", &[PathScope::Workspace], None, None)
            .unwrap();
        assert!(accept.confirmation.published);
        assert!(upper_at(&backend, "root", &workspace.join("child.txt")).exists());
        backend.resume("root").unwrap();
        backend.terminate("child").unwrap();
        let child_root = backend.paths("child");
        backend.discard("child", &[PathScope::Workspace]).unwrap();
        wait_until(|| !child_root.exists());
        assert!(!child_root.exists(), "priv discard left child data");
        backend.terminate("root").unwrap();
        backend.cleanup("root").unwrap();
    }

    #[test]
    fn priv_direct_root_overlay_is_available_in_initial_userns() {
        if !priv_enabled() {
            eprintln!("SKIP priv_direct_root_overlay_is_available_in_initial_userns: ASCOUT_PRIV_TESTS != 1");
            return;
        }
        let root = tempfile::Builder::new()
            .prefix("priv-root-")
            .tempdir_in(priv_state_root())
            .unwrap();
        let upper = root.path().join("upper");
        let work = root.path().join("work");
        fs::create_dir_all(&upper).unwrap();
        fs::create_dir_all(&work).unwrap();
        if unsafe { libc::geteuid() } != 0 {
            eprintln!("SKIP priv_direct_root_overlay_is_available_in_initial_userns: not root");
            return;
        }
        let status = Command::new("unshare")
            .args(["--mount", "--propagation", "private", "bwrap"])
            .args(["--die-with-parent", "--ro-bind", "/", "/"])
            .args(["--proc", "/proc", "--dev", "/dev"])
            .arg("--overlay-src")
            .arg("/")
            .arg("--overlay")
            .arg(&upper)
            .arg(&work)
            .arg("/")
            .arg("--chdir")
            .arg("/")
            .arg("--")
            .arg("/usr/bin/true")
            .status()
            .unwrap();
        assert!(status.success(), "privileged direct lower=/ overlay failed");
    }

    fn wait_removed(path: &Path) {
        wait_until(|| !path_exists(path));
        assert!(
            !path_exists(path),
            "path was not removed: {}",
            path.display()
        );
    }

    fn wait_until(mut condition: impl FnMut() -> bool) {
        for _ in 0..500 {
            if condition() {
                return;
            }
            thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(condition(), "condition was not met in time");
    }
}
