//! mountinfo 候选、逐挂载 overlay 与 LaunchFragment 产出。

use std::path::Path;

use super::*;

impl FilesystemBackend {
    pub(super) fn overlayable_mounts(&self) -> Vec<PathBuf> {
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

    pub(super) fn probe_mount_target(&self, target: &Path) -> bool {
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

    /// A mount whose subtree contains the backend state directory cannot use
    /// snapshot layers below that same mount as overlay lowerdirs: OverlayFS
    /// rejects overlapping lower directories.  Such mounts are flattened at
    /// fork (parent upper copied into child upper) and their layers are only
    /// used as diff metadata.
    pub(super) fn mount_uses_flattening(&self, mount: &MountUnit) -> bool {
        self.config.state_directory.starts_with(&mount.target)
    }

    pub(super) fn launch_lowers(&self, mount: &MountUnit) -> Vec<PathBuf> {
        let mut lowers = vec![mount.target.clone()];
        if !self.mount_uses_flattening(mount) {
            for layer in &mount.lower {
                lowers.push(mount.layer_upper(layer));
            }
        }
        lowers
    }

    /// Produce the filesystem part of the process launch.  This backend never
    /// spawns a process and never records pids or namespaces.
    pub fn launch_fragment(
        &self,
        context: &BackendContext,
        launch: &crate::protocol::contract::AgentLaunch,
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


pub(super) fn real_writable_mounts() -> Vec<PathBuf> {
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


pub(super) fn unescape_mount_path(value: &str) -> PathBuf {
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

