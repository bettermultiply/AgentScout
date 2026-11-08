//! probe 相关实现。

use super::*;

impl FilesystemBackend {
    pub(super) fn probe_overlay(&self) -> bool {
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

    pub(super) fn probe_reflink(&self) -> bool {
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
    pub(super) fn probe_btrfs(&self) -> (bool, bool) {
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

    pub(super) fn probe_btrfs_incremental(&self) -> bool {
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

    pub(super) fn cleanup_btrfs_probe(&self, root: &Path) {
        let _ = remove_path(&self.config.btrfs_path, &root.join("snap"), true);
        let _ = remove_path(&self.config.btrfs_path, &root.join("sub"), true);
        let _ = remove_directory_force(root);
    }

}
