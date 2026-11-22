
use std::path::Path;

use super::*;

impl FilesystemBackend {
    pub(super) fn overlayable_mounts(&self) -> Vec<PathBuf> {
        if let Ok(cache) = self.overlay_targets.lock()
            && let Some(targets) = cache.as_ref()
        {
            return targets.clone();
        }
        let targets: Vec<PathBuf> = self
            .config
            .isolated_paths
            .clone()
            .unwrap_or_else(real_writable_mounts)
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

    /// OverlayFS rejects overlapping lowerdirs. The internal opt-in uses Flatten instead.
    pub(super) fn state_overlap_mounts(&self) -> Vec<PathBuf> {
        self.overlayable_mounts()
            .into_iter()
            .filter(|target| self.config.state_directory.starts_with(target))
            .collect()
    }

    pub(super) fn ensure_state_directory_outside_isolated_mounts(
        &self,
        targets: impl IntoIterator<Item = PathBuf>,
    ) -> Result<(), BackendError> {
        let mut overlaps: Vec<PathBuf> = targets
            .into_iter()
            .filter(|target| self.config.state_directory.starts_with(target))
            .collect();
        overlaps.sort();
        overlaps.dedup();
        if overlaps.is_empty() || self.config.allow_overlapping_state_directory {
            return Ok(());
        }
        Err(BackendError::new(format!(
            "filesystem state directory {} is inside isolated mount {}; snapshot layers stored there cannot be lowerdirs of that same mount (Overlay directories may not overlap). Move the state directory outside the isolated mounts, for example onto a separately mounted filesystem",
            self.config.state_directory.display(),
            overlaps
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )))
    }

    pub(super) fn launch_lowers(&self, mount: &MountUnit) -> Vec<PathBuf> {
        let mut lowers = vec![
            mount
                .saved_base
                .as_ref()
                .or(mount.base.as_ref())
                .unwrap_or(&mount.target)
                .clone(),
        ];
        for layer in &mount.lower {
            lowers.push(mount.layer_upper(layer));
        }
        lowers
    }

    pub fn launch_fragment(
        &self,
        context: &BackendContext,
        launch: &crate::protocol::contract::AgentLaunch,
    ) -> Result<LaunchFragment, BackendError> {
        let environment = self.launch_environment(context, launch)?;
        self.fragment_for_environment(&environment, launch, false)
    }

    pub(super) fn holder_fragment(
        &self,
        context: &BackendContext,
        launch: &crate::protocol::contract::AgentLaunch,
    ) -> Result<LaunchFragment, BackendError> {
        let environment = self.launch_environment(context, launch)?;
        self.fragment_for_environment(&environment, launch, true)
    }

    fn launch_environment(
        &self,
        context: &BackendContext,
        launch: &crate::protocol::contract::AgentLaunch,
    ) -> Result<Environment, BackendError> {
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
        let mut isolated = self.overlayable_mounts();
        isolated.extend(
            environment
                .mounts
                .values()
                .map(|mount| mount.target.clone()),
        );
        self.ensure_state_directory_outside_isolated_mounts(isolated)?;
        let workspace = self
            .config
            .sandbox_target(&launch.cwd)
            .unwrap_or_else(|| launch.cwd.clone());
        if environment.workspace_root.as_deref() != Some(workspace.as_path()) {
            environment.workspace_root = Some(workspace.clone());
            self.persist_environment(&environment).map_err(|error| {
                BackendError::new(format!("failed to persist workspace root: {error}"))
            })?;
            let mut state = self.environment()?;
            state
                .environments
                .insert(environment.id.clone(), environment.clone());
        }
        let before = environment.mounts.len();
        self.discover_mounts(&mut environment)?;
        if environment.mount_for_path(&workspace).is_none() {
            return Err(BackendError::new(format!(
                "workspace {} is not on an overlayable mounted filesystem; configure a separate writable mount or allow passthrough_rw",
                workspace.display()
            )));
        }
        if environment.mounts.len() > before {
            self.provision_discovered_mounts(&mut environment)?;
        }
        Ok(environment)
    }

    fn discover_mounts(&self, environment: &mut Environment) -> Result<(), BackendError> {
        for target in self.overlayable_mounts() {
            if !target.is_dir() {
                continue;
            }
            let sandbox = self
                .config
                .sandbox_target(&target)
                .unwrap_or_else(|| target.clone());
            let key = sandbox.display().to_string();
            if environment.mounts.contains_key(&key) {
                continue;
            }
            let layer_key = if self.uses_btrfs_storage() {
                format!("mounts/{}/volume", short_hash(&key))
            } else {
                format!("mounts/{}", short_hash(&key))
            };
            environment.mounts.insert(
                key,
                MountUnit {
                    target: sandbox.clone(),
                    base: (sandbox != target).then_some(target),
                    saved_base: None,
                    upper: environment.root.join(&layer_key).join("upper"),
                    work: environment.root.join(&layer_key).join("work"),
                    lower: Vec::new(),
                    layer_key,
                    volume_is_subvolume: self.uses_btrfs_storage(),
                },
            );
        }
        Ok(())
    }

    fn provision_discovered_mounts(
        &self,
        environment: &mut Environment,
    ) -> Result<(), BackendError> {
        if environment.mounts.is_empty() {
            return Ok(());
        }
        let units: Vec<MountUnit> = environment.mounts.values().cloned().collect();
        if self.uses_btrfs_storage() {
            for mount in &units {
                if !mount.volume_is_subvolume {
                    continue;
                }
                let volume = mount.volume()?;
                if !path_exists(volume) {
                    if let Some(parent) = volume.parent() {
                        fs::create_dir_all(parent).map_err(error)?;
                    }
                    self.btrfs_create(volume)?;
                }
            }
        }
        Self::prepare_mount_dirs(&units)?;
        self.persist_environment(environment).map_err(|error| {
            BackendError::new(format!(
                "failed to persist newly discovered mounts: {error}"
            ))
        })?;
        let mut state = self.environment()?;
        state
            .environments
            .insert(environment.id.clone(), environment.clone());
        Ok(())
    }

    fn fragment_for_environment(
        &self,
        environment: &Environment,
        launch: &crate::protocol::contract::AgentLaunch,
        native: bool,
    ) -> Result<LaunchFragment, BackendError> {
        let mut fragment = LaunchFragment {
            chdir: Some(
                self.config
                    .sandbox_target(&launch.cwd)
                    .unwrap_or_else(|| launch.cwd.clone()),
            ),
            ..LaunchFragment::default()
        };
        let holder_dir = self
            .config
            .state_directory
            .join("holder-sockets")
            .join(&environment.id);
        fs::create_dir_all(&holder_dir).map_err(error)?;
        // `/run` is read-only in bubblewrap's host-root view.
        fragment.holder_bind = Some((holder_dir, PathBuf::from("/dev/ascout-holder")));
        fragment
            .mounts
            .extend(crate::runtime::launch_plan::base_mount_entries());
        let mut mounts = environment.selected_mounts(&environment.scopes)?;
        // A later parent overlay would hide a child mounted first.
        mounts.sort_by_key(|mount| mount.target.components().count());
        let mut covered: BTreeSet<PathBuf> = BTreeSet::new();
        for mount in &mounts {
            if !mount.target.is_dir() && mount.base.is_none() {
                continue;
            }
            ensure_overlay_disjoint(&mount.target, &mount.upper)?;
            fs::create_dir_all(&mount.upper).map_err(error)?;
            reset_dir(&mount.work)?;
            ensure_overlay_work(&mount.upper, &mount.work)?;
            fragment.mounts.push(MountEntry::Overlay {
                lowers: self.launch_lowers(mount),
                upper: mount.upper.clone(),
                work: mount.work.clone(),
                target: mount.target.clone(),
            });
            covered.insert(mount.target.clone());
        }
        if !native {
            self.append_unmanaged_mounts(&mut fragment, &covered);
        }
        for path in &self.config.passthrough_rw {
            if path.exists() {
                fragment.mounts.push(MountEntry::Bind {
                    source: path.clone(),
                    target: path.clone(),
                });
            }
        }
        self.append_state_masks(&mut fragment);
        fragment.mounts.sort_by_key(|entry| match entry {
            MountEntry::RoBind { .. } | MountEntry::Proc { .. } | MountEntry::Dev { .. } => 0,
            MountEntry::Bind { .. } => 1,
            MountEntry::Overlay { .. } => 2,
            MountEntry::Tmpfs { .. } | MountEntry::RemountRo { .. } | MountEntry::Dir { .. } => 3,
        });
        Ok(fragment)
    }

    fn append_unmanaged_mounts(&self, fragment: &mut LaunchFragment, covered: &BTreeSet<PathBuf>) {
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
    }

    fn append_state_masks(&self, fragment: &mut LaunchFragment) {
        fragment.mounts.push(MountEntry::Tmpfs {
            target: self.config.state_directory.clone(),
        });
        fragment.mounts.push(MountEntry::RemountRo {
            target: self.config.state_directory.clone(),
        });
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
        if target == Path::new("/")
            || target.starts_with("/proc")
            || target.starts_with("/sys")
            || target.starts_with("/dev")
        {
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
