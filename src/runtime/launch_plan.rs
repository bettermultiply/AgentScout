
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Default)]
pub struct LaunchFragment {
    pub mounts: Vec<MountEntry>,
    pub chdir: Option<PathBuf>,
    pub env: BTreeMap<String, String>,
    pub holder_bind: Option<(PathBuf, PathBuf)>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum MountEntry {
    RoBind {
        source: PathBuf,
        target: PathBuf,
    },
    Bind {
        source: PathBuf,
        target: PathBuf,
    },
    Overlay {
        /// Bottom to top.
        lowers: Vec<PathBuf>,
        upper: PathBuf,
        work: PathBuf,
        target: PathBuf,
    },
    Proc {
        target: PathBuf,
    },
    Dev {
        target: PathBuf,
    },
    Tmpfs {
        target: PathBuf,
    },
    RemountRo {
        target: PathBuf,
    },
    Dir {
        target: PathBuf,
    },
}

#[derive(Debug, Clone, Default)]
pub struct MountSpec {
    pub entries: Vec<MountEntry>,
    pub chdir: Option<PathBuf>,
    pub json_status_fd: Option<libc::c_int>,
}

impl MountEntry {
    pub fn target(&self) -> Option<&Path> {
        match self {
            MountEntry::RoBind { target, .. }
            | MountEntry::Bind { target, .. }
            | MountEntry::Overlay { target, .. }
            | MountEntry::Proc { target }
            | MountEntry::Dev { target }
            | MountEntry::Tmpfs { target }
            | MountEntry::RemountRo { target }
            | MountEntry::Dir { target } => Some(target),
        }
    }
}

/// A tmpfs root with read-only host binds allows mount targets absent from the host root.
pub fn sandbox_root_needs_assembly(entries: &[MountEntry]) -> bool {
    entries
        .iter()
        .filter_map(MountEntry::target)
        .any(|target| !target.exists())
}

fn apply_sandbox_root_assembly(command: &mut Command, entries: &[MountEntry]) {
    command.arg("--tmpfs").arg("/");
    let replaced: BTreeSet<OsString> = entries
        .iter()
        .filter_map(|entry| match entry {
            MountEntry::RoBind { .. } | MountEntry::Proc { .. } | MountEntry::Dev { .. } => None,
            other => other
                .target()
                .and_then(|target| target.components().next())
                .map(|component| component.as_os_str().to_os_string()),
        })
        .collect();
    let Ok(read_dir) = fs::read_dir("/") else {
        return;
    };
    let mut names: Vec<OsString> = read_dir
        .filter_map(|entry| entry.ok().map(|entry| entry.file_name()))
        .filter(|name| name != "dev" && name != "proc")
        .filter(|name| !replaced.contains(name))
        .collect();
    names.sort();
    for name in names {
        let path = PathBuf::from("/").join(&name);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                if let Ok(link) = fs::read_link(&path) {
                    command.arg("--symlink").arg(link).arg(&path);
                }
            }
            Ok(_) => {
                command.arg("--ro-bind").arg(&path).arg(&path);
            }
            Err(_) => {}
        }
    }
    command.args(["--dev", "/dev", "--proc", "/proc"]);
}

impl MountSpec {
    pub fn apply(&self, command: &mut Command) {
        command.arg("--die-with-parent").arg("--unshare-user");
        let assemble_root = sandbox_root_needs_assembly(&self.entries);
        if assemble_root {
            apply_sandbox_root_assembly(command, &self.entries);
        }
        for entry in &self.entries {
            if assemble_root
                && matches!(entry, MountEntry::RoBind { source, target } if source == Path::new("/") && target == Path::new("/"))
            {
                continue;
            }
            if let Some(target) = entry.target() {
                if !target.exists() {
                    command.arg("--dir").arg(target);
                }
            }
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
