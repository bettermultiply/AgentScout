//! btrfs 子卷、快照、send dump 与增量 diff。

use std::path::Path;

use super::*;

impl FilesystemBackend {
    pub(super) fn btrfs_send_dump(&self, base: &Path, snapshot: &Path) -> Result<String, BackendError> {
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

