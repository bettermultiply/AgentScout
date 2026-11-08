//! Neutral launch-plan types shared by Backend composition and ProcessBackend.
//!
//! Backends contribute mount entries; ProcessBackend is the only component
//! that translates them into bubblewrap arguments.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Clone, Default)]
pub struct LaunchFragment {
    pub mounts: Vec<MountEntry>,
    pub chdir: Option<PathBuf>,
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub enum MountEntry {
    RoBind { source: PathBuf, target: PathBuf },
    Bind { source: PathBuf, target: PathBuf },
    Overlay {
        /// Bottom to top.
        lowers: Vec<PathBuf>,
        upper: PathBuf,
        work: PathBuf,
        target: PathBuf,
    },
    Proc { target: PathBuf },
    Dev { target: PathBuf },
    Tmpfs { target: PathBuf },
    RemountRo { target: PathBuf },
    Dir { target: PathBuf },
}

#[derive(Debug, Clone, Default)]
pub struct MountSpec {
    pub entries: Vec<MountEntry>,
    pub chdir: Option<PathBuf>,
    pub json_status_fd: Option<libc::c_int>,
}

impl MountSpec {
    pub fn apply(&self, command: &mut Command) {
        command.arg("--die-with-parent").arg("--unshare-user");
        for entry in &self.entries {
            match entry {
                MountEntry::RoBind { source, target } => {
                    command.arg("--ro-bind").arg(source).arg(target);
                }
                MountEntry::Bind { source, target } => {
                    command.arg("--bind").arg(source).arg(target);
                }
                MountEntry::Overlay {
                    lowers,
                    upper,
                    work,
                    target,
                } => {
                    for lower in lowers {
                        command.arg("--overlay-src").arg(lower);
                    }
                    command.arg("--overlay").arg(upper).arg(work).arg(target);
                }
                MountEntry::Proc { target } => {
                    command.arg("--proc").arg(target);
                }
                MountEntry::Dev { target } => {
                    command.arg("--dev").arg(target);
                }
                MountEntry::Tmpfs { target } => {
                    command.arg("--tmpfs").arg(target);
                }
                MountEntry::RemountRo { target } => {
                    command.arg("--remount-ro").arg(target);
                }
                MountEntry::Dir { target } => {
                    command.arg("--dir").arg(target);
                }
            }
        }
        if let Some(chdir) = &self.chdir {
            command.arg("--chdir").arg(chdir);
        }
        if let Some(fd) = self.json_status_fd {
            command.arg("--json-status-fd").arg(fd.to_string());
        }
    }
}
/// Standard base mounts used by isolation/probe code.
pub fn base_mount_entries() -> Vec<MountEntry> {
    vec![
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
    ]
}
