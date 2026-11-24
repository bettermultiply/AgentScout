
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use super::*;

// Linux btrfs UAPI (linux/btrfs.h). The v2 union is unused here.
#[repr(C)]
struct VolumeArgs {
    fd: i64,
    name: [u8; 4088],
}

#[repr(C)]
struct SnapshotArgs {
    fd: i64,
    transid: u64,
    flags: u64,
    unused: [u64; 4],
    name: [u8; 4040],
}

fn volume_target(path: &Path, name: &mut [u8]) -> std::io::Result<fs::File> {
    let basename = path
        .file_name()
        .ok_or(std::io::ErrorKind::InvalidInput)?
        .as_bytes();
    if basename.len() >= name.len() || basename.contains(&0) {
        return Err(std::io::ErrorKind::InvalidInput.into());
    }
    name[..basename.len()].copy_from_slice(basename);
    fs::File::open(path.parent().ok_or(std::io::ErrorKind::InvalidInput)?)
}

fn ioctl_result(result: libc::c_int) -> Result<(), BackendError> {
    if result == 0 {
        Ok(())
    } else {
        Err(error(std::io::Error::last_os_error()))
    }
}

fn btrfs_send_dump_with(
    btrfs_path: &Path,
    base: &Path,
    snapshot: &Path,
) -> Result<String, BackendError> {
    let mut send = Command::new(btrfs_path)
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
    let stderr = send.stderr.take();
    let stderr_thread = stderr.map(|mut stderr| {
        std::thread::spawn(move || {
            use std::io::Read;
            let mut text = String::new();
            let _ = stderr.read_to_string(&mut text);
            text
        })
    });
    let dump = match Command::new(btrfs_path)
        .args([
            std::ffi::OsStr::new("receive"),
            std::ffi::OsStr::new("--dump"),
        ])
        .stdin(Stdio::from(stdout))
        .output()
    {
        Ok(dump) => dump,
        Err(value) => {
            let _ = send.kill();
            let _ = send.wait();
            return Err(error(value));
        }
    };
    let status = send.wait().map_err(error)?;
    let send_error = stderr_thread
        .and_then(|thread| thread.join().ok())
        .unwrap_or_default();
    if !status.success() || !dump.status.success() {
        return Err(BackendError::new(format!(
            "btrfs send/receive --dump failed: send={status}; send_err={}; receive_err={}",
            send_error.trim(),
            String::from_utf8_lossy(&dump.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&dump.stdout).into_owned())
}

impl FilesystemBackend {
    pub(super) fn btrfs_send_dump(
        &self,
        base: &Path,
        snapshot: &Path,
    ) -> Result<String, BackendError> {
        btrfs_send_dump_with(&self.config.btrfs_path, base, snapshot)
    }

    pub(super) fn btrfs_delete_strict(&self, path: &Path) -> Result<(), BackendError> {
        self.run_btrfs(&[
            std::ffi::OsStr::new("subvolume"),
            std::ffi::OsStr::new("delete"),
            path.as_os_str(),
        ])
    }

    pub(super) fn run_btrfs(&self, arguments: &[&std::ffi::OsStr]) -> Result<(), BackendError> {
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

    pub(super) fn btrfs_create(&self, path: &Path) -> Result<(), BackendError> {
        if self.config.btrfs_path == Path::new("btrfs") {
            let mut args = VolumeArgs {
                fd: 0,
                name: [0; 4088],
            };
            let parent = volume_target(path, &mut args.name).map_err(error)?;
            let request = libc::_IOW::<VolumeArgs>(0x94, 14);
            return ioctl_result(unsafe { libc::ioctl(parent.as_raw_fd(), request, &args) });
        }
        self.run_btrfs(&[
            std::ffi::OsStr::new("subvolume"),
            std::ffi::OsStr::new("create"),
            path.as_os_str(),
        ])
    }

    pub(super) fn btrfs_snapshot(
        &self,
        source: &Path,
        target: &Path,
        read_only: bool,
    ) -> Result<(), BackendError> {
        if self.config.btrfs_path == Path::new("btrfs") {
            let source = fs::File::open(source).map_err(error)?;
            let mut args = SnapshotArgs {
                fd: i64::from(source.as_raw_fd()),
                transid: 0,
                flags: if read_only { 1 << 1 } else { 0 }, // BTRFS_SUBVOL_RDONLY
                unused: [0; 4],
                name: [0; 4040],
            };
            let parent = volume_target(target, &mut args.name).map_err(error)?;
            let request = libc::_IOW::<SnapshotArgs>(0x94, 23);
            return ioctl_result(unsafe { libc::ioctl(parent.as_raw_fd(), request, &args) });
        }
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

    pub(super) fn probe_btrfs_snapshot_copies(&self) -> bool {
        let root = self.config.state_directory.join(".probe-btrfs-snapshot");
        let _ = remove_path(&self.config.btrfs_path, &root, true);
        if fs::create_dir_all(&root).is_err() {
            return false;
        }
        let sub = root.join("sub");
        let snap = root.join("snap");
        let upper = root.join("upper");
        let work = root.join("work");
        let target = root.join("target");
        let ok = self.btrfs_create(&sub).is_ok()
            && fs::write(sub.join("file"), b"snapshot-copy").is_ok()
            && self.btrfs_snapshot(&sub, &snap, true).is_ok()
            && fs::create_dir_all(&upper).is_ok()
            && fs::create_dir_all(&work).is_ok()
            && fs::create_dir_all(&target).is_ok();
        let mounted = if ok {
            let command = if Path::new("/usr/bin/true").exists() {
                "/usr/bin/true"
            } else {
                "/bin/true"
            };
            let mut entries = crate::runtime::launch_plan::base_mount_entries();
            entries.push(MountEntry::Overlay {
                lowers: vec![snap.clone()],
                upper: upper.clone(),
                work: work.clone(),
                target: target.clone(),
            });
            let spec = MountSpec {
                entries,
                chdir: Some(PathBuf::from("/")),
                json_status_fd: None,
            };
            let mut probe = Command::new(&self.config.bubblewrap_path);
            spec.apply(&mut probe);
            matches!(
                probe
                    .arg("--")
                    .arg(command)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status(),
                Ok(status) if status.success()
            )
        } else {
            false
        };
        let deleted = if mounted {
            // The slow fallback must still delete a read-only snapshot.
            let _ = self.btrfs_delete_strict(&snap);
            let _ = self.btrfs_delete_strict(&sub);
            let _ = remove_path_with_mode(&self.config.btrfs_path, &root, true, false);
            !path_exists(&root)
        } else {
            false
        };
        let _ = remove_path(&self.config.btrfs_path, &root, true);
        mounted && deleted
    }

    pub(super) fn btrfs_delta(
        &self,
        _environment: &Environment,
        mount: &MountUnit,
        base: &Path,
    ) -> Result<(View, Vec<(String, String)>, Vec<FilesystemChange>), BackendError> {
        let token = self.token();
        let snapshot_root = self.snapshots_root().join(format!("diff-snapshot-{token}"));
        let current = snapshot_root.join(&mount.layer_key);
        if let Some(parent) = current.parent() {
            fs::create_dir_all(parent).map_err(error)?;
        }
        self.btrfs_snapshot(mount.volume()?, &current, true)?;
        let base_volume = base.join(&mount.layer_key);
        let dump = match self.btrfs_send_dump(&base_volume, &current) {
            Ok(dump) => dump,
            Err(error) => {
                let _ = remove_path(&self.config.btrfs_path, &snapshot_root, true);
                return Err(error);
            }
        };
        let delta = send_dump_upper_delta(&dump, &mount.upper);
        let _ = remove_path(&self.config.btrfs_path, &snapshot_root, true);
        delta
    }
}

fn send_dump_upper_delta(
    dump: &str,
    upper: &Path,
) -> Result<(View, Vec<(String, String)>, Vec<FilesystemChange>), BackendError> {
    let changes = parse_send_dump(dump)
        .into_iter()
        .filter_map(|change| {
            let path = change.path.strip_prefix("upper/").map(str::to_string);
            let old_path = change
                .old_path
                .and_then(|path| path.strip_prefix("upper/").map(str::to_string));
            match (path, old_path) {
                (Some(path), old_path) => Some(FilesystemChange {
                    path,
                    kind: if change.kind == "rename" && old_path.is_none() {
                        "change".to_string()
                    } else {
                        change.kind
                    },
                    old_path,
                }),
                (None, Some(path)) => Some(FilesystemChange {
                    path,
                    kind: "delete".to_string(),
                    old_path: None,
                }),
                (None, None) => None,
            }
        })
        .collect();
    delta_from_changes(changes, upper)
}

fn delta_from_changes(
    changes: Vec<FilesystemChange>,
    upper: &Path,
) -> Result<(View, Vec<(String, String)>, Vec<FilesystemChange>), BackendError> {
    let mut view = View::new();
    let mut redirects = Vec::new();
    // Tombstones first: a rename cycle may reuse an old name as a destination.
    for change in &changes {
        if change.kind == "delete" {
            view.insert(change.path.clone(), whiteout_node());
        }
        if let Some(old) = &change.old_path {
            view.insert(old.clone(), whiteout_node());
        }
    }
    for change in &changes {
        if change.kind == "delete" {
            continue;
        }
        if let Some(node) = layer_node_at(upper, &change.path)? {
            if let Some(target) = node.redirect.clone() {
                redirects.push((target, change.path.clone()));
            }
            if (change.old_path.is_some() || node.opaque) && matches!(node.kind, NodeKind::Dir) {
                // Send can omit unchanged descendants needed when replaying a replacement.
                for (path, child) in read_layer(&upper.join(&change.path))? {
                    view.insert(format!("{}/{path}", change.path), child);
                }
            }
            view.insert(change.path.clone(), node);
        } else {
            view.insert(change.path.clone(), whiteout_node());
        }
    }
    let mut output = changes_from_delta(&view, &redirects);
    output.extend(changes.into_iter().filter(|c| c.old_path.is_some()));
    Ok((view, redirects, output))
}

pub(super) fn parse_send_dump(dump: &str) -> Vec<FilesystemChange> {
    // btrfs receive --dump prefixes paths with the snapshot name, independent of the layer key.
    let mut root = String::new();
    // Map current paths to base paths (None for new inodes), including descendants of renames.
    let mut entries: BTreeMap<String, Option<String>> = BTreeMap::new();
    let mut deleted = BTreeSet::new();
    for line in dump.lines() {
        let mut parts = line.split_whitespace();
        let Some(command) = parts.next() else {
            continue;
        };
        let raw_path = parts.next().unwrap_or("");
        if command == "snapshot" || command == "subvol" {
            root = format!(
                "{}/",
                raw_path
                    .strip_prefix("./")
                    .unwrap_or(raw_path)
                    .trim_end_matches('/')
            );
            continue;
        }
        let normalize = |value: &str| -> Option<String> {
            let value = value.strip_prefix("./").unwrap_or(value);
            let path = value.strip_prefix(&root)?.trim_end_matches('/');
            (!root.is_empty() && !path.is_empty()).then(|| path.to_string())
        };
        let Some(path) = normalize(raw_path) else {
            continue;
        };
        match command {
            "rename" => {
                let Some(new) = parts.find_map(|v| v.strip_prefix("dest=").and_then(normalize))
                else {
                    continue;
                };
                let origin = stream_origin(&entries, &path);
                entries.remove(&path);
                let descendants: Vec<_> = entries
                    .keys()
                    .filter(|p| p.starts_with(&format!("{path}/")))
                    .cloned()
                    .collect();
                for old in descendants {
                    let origin = entries.remove(&old).unwrap();
                    entries.insert(format!("{new}{}", &old[path.len()..]), origin);
                }
                if let Some(Some(replaced)) = entries.insert(new, origin) {
                    deleted.insert(replaced);
                }
            }
            "unlink" | "rmdir" => {
                let origin = stream_origin(&entries, &path);
                entries.remove(&path);
                if let Some(origin) = origin {
                    deleted.insert(origin);
                }
            }
            "mkdir" | "mkfile" | "mknod" | "symlink" | "mkfifo" | "mksock" => {
                entries.insert(path, None);
            }
            "link" | "write" | "encoded_write" | "clone" | "truncate" | "chmod" | "chown"
            | "utimes" | "set_xattr" | "remove_xattr" | "update_extent" | "fileattr" => {
                let origin = stream_origin(&entries, &path);
                entries.entry(path).or_insert(origin);
            }
            _ => {}
        }
    }
    let mut changes = Vec::new();
    for path in deleted {
        if !entries.contains_key(&path) {
            changes.push(FilesystemChange {
                path,
                kind: "delete".into(),
                old_path: None,
            });
        }
    }
    for (path, origin) in entries {
        let old_path = origin.filter(|old| old != &path);
        changes.push(FilesystemChange {
            path,
            kind: if old_path.is_some() {
                "rename"
            } else {
                "change"
            }
            .into(),
            old_path,
        });
    }
    changes
}

fn stream_origin(entries: &BTreeMap<String, Option<String>>, path: &str) -> Option<String> {
    if let Some(origin) = entries.get(path) {
        return origin.clone();
    }
    let mut parent = path;
    while let Some((prefix, _)) = parent.rsplit_once('/') {
        if let Some(origin) = entries.get(prefix) {
            return origin
                .as_ref()
                .map(|old| format!("{old}{}", &path[prefix.len()..]));
        }
        parent = prefix;
    }
    Some(path.to_string())
}
