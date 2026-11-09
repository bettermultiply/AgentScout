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
        let delta = send_dump_delta(&dump, &mount.upper);
        let _ = remove_path(&self.config.btrfs_path, &snapshot_root, true);
        delta
    }
}

fn send_dump_delta(
    dump: &str,
    upper: &Path,
) -> Result<(View, Vec<(String, String)>, Vec<FilesystemChange>), BackendError> {
    let changes = parse_send_dump(dump);
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
            if change.old_path.is_some() && matches!(node.kind, NodeKind::Dir) {
                // An unchanged descendant of a moved directory has no separate
                // send command. Replay needs that subtree, never unrelated upper.
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
    // receive --dump prefixes *every* path with the snapshot name.  It is
    // unrelated to the backend's layer key (and can itself be "upper").
    let mut root = String::new();
    // Current stream path -> original base path; None denotes a new inode.
    // Track identities, not an append-only command log: send creates temporary
    // names and can move directories (including already touched descendants).
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
        // Replacements at the same name are changes, not a delete + change.
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn send_dump_snapshot_prefix_and_temporary_renames() {
        // Format captured in log.txt:199-218 (snapshot name differs from layer key).
        let dump = "snapshot ./snap2 uuid=example\n\
            unlink ./snap2/base.txt\n\
            mkfile ./snap2/o258-840402-0\n\
            rename ./snap2/o258-840402-0 dest=./snap2/modified.txt\n\
            write ./snap2/modified.txt offset=0 len=8\n\
            mkdir ./snap2/o259-840402-0\n\
            rename ./snap2/o259-840402-0 dest=./snap2/renamed\n\
            mkfile ./snap2/o260-840402-0\n\
            rename ./snap2/o260-840402-0 dest=./snap2/renamed/nested.txt\n\
            utimes ./snap2/ atime=example\n";
        let changes = parse_send_dump(dump);
        let actual: Vec<_> = changes
            .iter()
            .map(|c| (c.path.as_str(), c.kind.as_str(), c.old_path.as_deref()))
            .collect();
        assert_eq!(
            actual,
            vec![
                ("base.txt", "delete", None),
                ("modified.txt", "change", None),
                ("renamed", "change", None),
                ("renamed/nested.txt", "change", None),
            ]
        );
        let upper_dump = dump.replace("snap2", "upper");
        let changes = parse_send_dump(&upper_dump);
        assert!(
            changes
                .iter()
                .any(|c| c.path == "modified.txt" && c.kind == "change")
        );
        assert!(
            !changes
                .iter()
                .any(|c| c.path.starts_with("upper/") || c.path.starts_with('o'))
        );
    }

    #[test]
    fn send_dump_real_rename_delete_and_created_then_removed() {
        let dump = "snapshot ./upper uuid=example\n\
            chmod ./upper/old/file mode=600\n\
            rename ./upper/old dest=./upper/o300-1-0\n\
            rename ./upper/o300-1-0 dest=./upper/new\n\
            unlink ./upper/new/file\n\
            unlink ./upper/new/untouched\n\
            unlink ./upper/deleted\n\
            mkfile ./upper/o301-1-0\n\
            rename ./upper/o301-1-0 dest=./upper/ephemeral\n\
            unlink ./upper/ephemeral\n\
            unlink ./upper/replaced\n\
            mkfile ./upper/o302-1-0\n\
            rename ./upper/o302-1-0 dest=./upper/replaced\n\
            chmod ./upper/upper/kept mode=644\n";
        let changes = parse_send_dump(dump);
        assert!(changes.iter().any(|c| c.path == "new"
            && c.kind == "rename"
            && c.old_path.as_deref() == Some("old")));
        assert!(
            changes
                .iter()
                .any(|c| c.path == "old/file" && c.kind == "delete")
        );
        assert!(
            changes
                .iter()
                .any(|c| c.path == "old/untouched" && c.kind == "delete")
        );
        assert!(
            changes
                .iter()
                .any(|c| c.path == "deleted" && c.kind == "delete")
        );
        assert!(
            changes
                .iter()
                .any(|c| c.path == "upper/kept" && c.kind == "change")
        );
        assert_eq!(changes.iter().filter(|c| c.path == "replaced").count(), 1);
        assert!(changes.iter().any(|c| c.path == "replaced" && c.kind == "change"));
        assert!(!changes.iter().any(|c| c.path.contains("o300-")
            || c.path.contains("o301-")
            || c.path == "ephemeral"));
    }

    #[test]
    fn send_dump_delta_uses_final_paths_for_accept_and_moved_subtrees() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("new")).unwrap();
        fs::write(root.path().join("new/unchanged"), b"kept").unwrap();
        fs::write(root.path().join("created"), b"created").unwrap();
        fs::write(root.path().join("unrelated"), b"not in delta").unwrap();
        let dump = "snapshot ./upper uuid=example\n\
            mkfile ./upper/o261-840421-0\n\
            rename ./upper/o261-840421-0 dest=./upper/created\n\
            rename ./upper/old dest=./upper/new\n\
            unlink ./upper/deleted\n";
        let (delta, _, changes) = send_dump_delta(dump, root.path()).unwrap();
        assert_eq!(
            delta.keys().map(String::as_str).collect::<Vec<_>>(),
            vec!["created", "deleted", "new", "new/unchanged", "old"]
        );
        assert!(!delta["created"].is_whiteout());
        assert_eq!(
            fs::read(delta["created"].source.as_ref().unwrap()).unwrap(),
            b"created"
        );
        assert!(delta["old"].is_whiteout());
        assert!(delta["deleted"].is_whiteout());
        assert_eq!(
            fs::read(delta["new/unchanged"].source.as_ref().unwrap()).unwrap(),
            b"kept"
        );
        assert!(
            changes
                .iter()
                .any(|c| c.path == "created" && c.kind == "change")
        );
        assert!(
            changes
                .iter()
                .any(|c| c.path == "new" && c.old_path.as_deref() == Some("old"))
        );
    }

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
