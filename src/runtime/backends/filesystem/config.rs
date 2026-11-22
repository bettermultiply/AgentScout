
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct FilesystemConfig {
    pub state_directory: PathBuf,
    pub bubblewrap_path: PathBuf,
    pub btrfs_path: PathBuf,
    pub force_overlay: bool,
    pub isolated_paths: Option<Vec<PathBuf>>,
    /// Changes the sandbox path only. The isolated host directory stays the lower and publication target.
    pub isolated_targets: Vec<(PathBuf, PathBuf)>,
    pub passthrough_rw: Vec<PathBuf>,
    pub isolate_unmanaged_mounts: bool,
    /// Tests and diagnostics only. Allow overlap by falling back to Flatten.
    pub allow_overlapping_state_directory: bool,
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
