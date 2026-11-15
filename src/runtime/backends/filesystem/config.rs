//! 文件系统 Backend 配置与数据面类别。

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct FilesystemConfig {
    pub state_directory: PathBuf,
    pub bubblewrap_path: PathBuf,
    pub btrfs_path: PathBuf,
    /// Force the portable overlay data plane even when btrfs is available.
    /// Used by tests that exercise overlay semantics on a btrfs host.
    pub force_overlay: bool,
    /// Explicit directory isolation, with all other host paths read-only.
    /// None retains automatic writable-mount discovery.
    pub isolated_paths: Option<Vec<PathBuf>>,
    /// Sandbox path of an isolated path.  An isolated host directory is still
    /// the overlay lower and the publication destination; this only changes
    /// where the Agent sees that directory inside the sandbox.  A workload that
    /// names its workspace `/app` maps the host workspace there.
    pub isolated_targets: Vec<(PathBuf, PathBuf)>,
    /// Paths explicitly allowed to pass through read-write to the host.
    pub passthrough_rw: Vec<PathBuf>,
    /// Make unmanaged writable mounts read-only inside the sandbox.
    pub isolate_unmanaged_mounts: bool,
    /// Internal switch for tests and diagnostics only: allow the state
    /// directory to sit inside an isolated mount and fall back to flattening.
    /// Production deployments must keep the state directory outside every
    /// isolated mount; `FilesystemConfig` is built in code and is not
    /// deserialized from external configuration.
    pub allow_overlapping_state_directory: bool,
    /// Per-fork layout threshold.  An upper with fewer entries is flattened
    /// into the child upper; a larger one adds a snapshot layer to the child
    /// lower chain.  `0` always uses Chain and `usize::MAX` always uses
    /// Flatten.  Tests and benchmarks override this; production uses the
    /// default constant.
    pub fork_flatten_entry_limit: usize,
}

impl FilesystemConfig {
    pub fn new(state_directory: impl Into<PathBuf>) -> Self {
        Self {
            state_directory: state_directory.into(),
            bubblewrap_path: PathBuf::from("bwrap"),
            btrfs_path: PathBuf::from("btrfs"),
            force_overlay: false,
            isolated_paths: None,
            isolated_targets: Vec::new(),
            passthrough_rw: Vec::new(),
            isolate_unmanaged_mounts: true,
            allow_overlapping_state_directory: false,
            fork_flatten_entry_limit: super::DEFAULT_FORK_FLATTEN_ENTRY_LIMIT,
        }
    }

    /// The path the Agent sees for an isolated host directory, when this
    /// deployment maps it somewhere else inside the sandbox.
    pub fn sandbox_target(&self, host: &std::path::Path) -> Option<PathBuf> {
        self.isolated_targets
            .iter()
            .filter(|(source, _)| host.starts_with(source))
            .max_by_key(|(source, _)| source.as_os_str().len())
            .map(|(source, target)| match host.strip_prefix(source) {
                Ok(relative) if relative.as_os_str().is_empty() => target.clone(),
                Ok(relative) => target.join(relative),
                Err(_) => target.clone(),
            })
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
