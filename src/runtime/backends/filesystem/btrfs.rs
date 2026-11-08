//! btrfs 子卷、快照、send dump 与增量 diff。

use std::path::Path;

use super::*;

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
    pub(super) fn btrfs_send_dump(&self, base: &Path, snapshot: &Path) -> Result<String, BackendError> {
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
        self.run_btrfs(&[
            std::ffi::OsStr::new("subvolume"),
            std::ffi::OsStr::new("create"),
            path.as_os_str(),
        ])
    }

    pub(super) fn btrfs_snapshot(&self, source: &Path, target: &Path, read_only: bool) -> Result<(), BackendError> {
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
            // Exercise the same btrfs-aware recursive removal used by the
            // background trash path.  fast_delete is only a cost signal; this
            // fallback must actually clean a read-only snapshot.
            let _ = self.btrfs_delete_strict(&snap);
            let _ = self.btrfs_delete_strict(&sub);
            let _ = remove_path_with_mode(
                &self.config.btrfs_path,
                &root,
                true,
                false,
            );
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

}

pub(super) fn parse_send_dump(dump: &str, layer_key: &str) -> Vec<FilesystemChange> {
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


#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn send_dump_drains_large_send_stderr() {
        let root = tempfile::tempdir().unwrap();
        let script = root.path().join("fake-btrfs.sh");
        std::fs::write(
            &script,
            r#"#!/bin/sh
if [ "$1" = "send" ]; then
  i=0
  while [ "$i" -lt 20000 ]; do
    echo "send stderr line $i" >&2
    i=$((i+1))
  done
  echo "send payload"
else
  cat >/dev/null
  echo "receive dump"
fi
"#,
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let dump = btrfs_send_dump_with(&script, root.path(), root.path()).unwrap();
        assert!(dump.contains("receive dump"));
    }
}
