//! Filesystem 测试共享辅助。

use super::super::*;
use crate::protocol::contract::AgentLaunch;
use crate::runtime::backends::filesystem::mounts::real_writable_mounts;

    pub(super) fn temp() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    pub(super) fn btrfs_temp() -> tempfile::TempDir {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/tmp");
        fs::create_dir_all(&root).unwrap();
        tempfile::Builder::new()
            .prefix("fs-test-")
            .tempdir_in(root)
            .expect("tempdir under target/tmp")
    }

    /// Choose a state-directory tempdir on a different writable mount than
    /// the workspace.  This keeps the lower-chain tests independent of where
    /// the repository itself is located.
    pub(super) fn separate_state_tempdir(workspace: &Path) -> Option<tempfile::TempDir> {
        for target in real_writable_mounts() {
            if workspace.starts_with(&target) {
                continue;
            }
            if let Ok(directory) = tempfile::Builder::new()
                .prefix("ascout-state-")
                .tempdir_in(&target)
            {
                return Some(directory);
            }
        }
        None
    }

    /// Config for tests whose state directory deliberately sits inside an
    /// isolated mount.  They exercise the retained flatten fallback; production
    /// layouts must keep the state directory outside every isolated mount.
    pub(super) fn overlapping_state_config(state: impl Into<PathBuf>) -> FilesystemConfig {
        let mut config = FilesystemConfig::new(state);
        config.allow_overlapping_state_directory = true;
        config
    }

    /// State-directory tempdir on a writable mount that is not in `isolated`
    /// and does not contain the workspace.  Used by tests that need the default
    /// (non-overlapping) layout.
    pub(super) fn tempdir_outside_mounts(
        workspace: &Path,
        isolated: &[PathBuf],
    ) -> Option<tempfile::TempDir> {
        for target in real_writable_mounts() {
            if workspace.starts_with(&target) {
                continue;
            }
            if isolated.iter().any(|mount| mount == &target) {
                continue;
            }
            if let Ok(directory) = tempfile::Builder::new()
                .prefix("ascout-state-")
                .tempdir_in(&target)
            {
                return Some(directory);
            }
        }
        None
    }

    pub(super) fn write(root: &Path, relative: &str, content: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    pub(super) fn set_test_xattr(path: &Path, name: &str, value: &[u8]) {
        set_xattr(path, name, value).unwrap();
    }

    pub(super) fn new_backend(root: &Path) -> FilesystemBackend {
        let backend = FilesystemBackend::new(overlapping_state_config(root.join("state"))).unwrap();
        assert!(
            backend.usable,
            "bubblewrap overlay probe failed; filesystem tests require a working sandbox"
        );
        backend
    }

    pub(super) fn new_overlay_backend(root: &Path) -> FilesystemBackend {
        let mut config = overlapping_state_config(root.join("state"));
        config.force_overlay = true;
        let backend = FilesystemBackend::new(config).unwrap();
        assert!(backend.usable, "bubblewrap overlay probe failed");
        assert_eq!(backend.mechanism(), FilesystemMechanism::Overlay);
        backend
    }

    /// Overlay backend whose state directory is on a different mount than
    /// `workspace`, so the workspace mount is not forced to flatten by the
    /// overlap rule and the per-fork layout threshold decides.  The state
    /// mount itself still overlaps (it holds the state directory), so the
    /// internal switch stays enabled.
    pub(super) fn layout_backend(
        workspace: &Path,
        flatten_entry_limit: usize,
    ) -> Option<(tempfile::TempDir, FilesystemBackend)> {
        let state = separate_state_tempdir(workspace)?;
        let mut config = FilesystemConfig::new(state.path().join("state"));
        config.force_overlay = true;
        config.allow_overlapping_state_directory = true;
        config.fork_flatten_entry_limit = flatten_entry_limit;
        let backend = FilesystemBackend::new(config).unwrap();
        assert!(backend.usable, "bubblewrap overlay probe failed");
        assert_eq!(backend.mechanism(), FilesystemMechanism::Overlay);
        Some((state, backend))
    }

    pub(super) fn mount_for(backend: &FilesystemBackend, environment_id: &str, path: &Path) -> MountUnit {
        let state = backend.lock().unwrap();
        state.environments[environment_id]
            .mount_for_path(path)
            .cloned()
            .unwrap_or_else(|| {
                panic!(
                    "no mount unit for {} in environment {}: {:?}",
                    path.display(),
                    environment_id,
                    state.environments[environment_id]
                        .mounts
                        .keys()
                        .collect::<Vec<_>>()
                )
            })
    }

    pub(super) fn upper_at(backend: &FilesystemBackend, environment_id: &str, path: &Path) -> PathBuf {
        let mount = mount_for(backend, environment_id, path);
        let relative = path.strip_prefix(&mount.target).unwrap_or_else(|_| {
            panic!(
                "{} is not below mount {}",
                path.display(),
                mount.target.display()
            )
        });
        mount.upper.join(relative)
    }

    pub(super) fn write_upper(
        backend: &FilesystemBackend,
        environment_id: &str,
        path: &Path,
        content: &str,
    ) {
        let target = upper_at(backend, environment_id, path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(target, content).unwrap();
    }

    pub(super) fn read_upper_layer_at(
        backend: &FilesystemBackend,
        environment_id: &str,
        directory: &Path,
    ) -> View {
        let target = upper_at(backend, environment_id, directory);
        read_layer(&target).unwrap_or_default()
    }

    pub(super) fn spawn_in_environment(
        backend: &FilesystemBackend,
        environment_id: &str,
        cwd: &Path,
        script: &str,
    ) -> (crate::runtime::process::ProcessBackend, u32, u64) {
        spawn_in_environment_with_env(backend, environment_id, cwd, script, BTreeMap::new())
    }

    pub(super) fn spawn_in_environment_with_env(
        backend: &FilesystemBackend,
        environment_id: &str,
        cwd: &Path,
        script: &str,
        env: BTreeMap<String, String>,
    ) -> (crate::runtime::process::ProcessBackend, u32, u64) {
        let launch = AgentLaunch {
            command: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
            cwd: cwd.to_path_buf(),
            log: cwd.join("agent.log"),
            instance_id: None,
            env,
        };
        let context = BackendContext {
            environment_id: environment_id.to_string(),
            ..BackendContext::default()
        };
        let process = crate::runtime::process::ProcessBackend::new(
            "process",
            backend.bubblewrap_path(),
        );
        let fragment = backend.launch_fragment(&context, &launch).unwrap();
        process.launch(&context, &launch, &fragment).unwrap();
        let handle = process.process_handle(environment_id).unwrap();
        (process, handle.inner_pid, handle.mount_namespace)
    }

    pub(super) fn run_in_environment(
        backend: &FilesystemBackend,
        environment_id: &str,
        cwd: &Path,
        script: &str,
    ) -> bool {
        run_in_environment_with_env(backend, environment_id, cwd, script, BTreeMap::new())
    }

    pub(super) fn run_in_environment_with_env(
        backend: &FilesystemBackend,
        environment_id: &str,
        cwd: &Path,
        script: &str,
        env: BTreeMap<String, String>,
    ) -> bool {
        let (process, _, _) =
            spawn_in_environment_with_env(backend, environment_id, cwd, script, env);
        let status = process.wait(environment_id).unwrap();
        if !status.success() {
            eprintln!(
                "sandbox command failed: env={environment_id} status={status} log={}",
                fs::read_to_string(cwd.join("agent.log")).unwrap_or_default()
            );
        }
        let _ = process.cleanup(environment_id);
        status.success()
    }

    pub(super) fn priv_enabled() -> bool {
        std::env::var("ASCOUT_PRIV_TESTS").ok().as_deref() == Some("1")
    }

    pub(super) fn priv_state_root() -> PathBuf {
        let root = std::env::var_os("ASCOUT_PRIV_STATE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/var/tmp/ascout-priv-check"));
        fs::create_dir_all(&root).unwrap();
        root
    }

    pub(super) fn trash_is_empty(path: &Path) -> bool {
        match fs::read_dir(path) {
            Ok(mut entries) => entries.next().is_none(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
            Err(_) => false,
        }
    }

    pub(super) fn wait_removed(path: &Path) {
        wait_until(|| !path_exists(path));
        assert!(
            !path_exists(path),
            "path was not removed: {}",
            path.display()
        );
    }

    pub(super) fn wait_until(mut condition: impl FnMut() -> bool) {
        for _ in 0..500 {
            if condition() {
                return;
            }
            thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(condition(), "condition was not met in time");
    }
