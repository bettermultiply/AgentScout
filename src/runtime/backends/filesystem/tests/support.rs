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

    pub(super) fn dir_is_empty(path: &Path) -> bool {
        match fs::read_dir(path) {
            Ok(mut entries) => entries.next().is_none(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
            Err(_) => false,
        }
    }

    /// Parent directories between a mount point and a file that a Chain delta
    /// scan necessarily reports: writing a nested path copies the parent
    /// directories into the child upper.  These are structural, not semantic
    /// changes, but the assertion below still accounts for every entry so an
    /// unexpected directory or file change fails.
    fn ancestor_directories(mount: &Path, file: &Path) -> Vec<PathBuf> {
        let mut directories = Vec::new();
        let mut current = file.parent();
        while let Some(directory) = current {
            if directory == mount || !directory.starts_with(mount) {
                break;
            }
            directories.push(directory.to_path_buf());
            current = directory.parent();
        }
        directories
    }

    /// One fork generation on the mount that holds `base`: write base files,
    /// fork, assert the layout selected by `flatten_entry_limit`, let a real
    /// process modify/delete/create, assert the exact diff, accept back into
    /// `root`, verify the real view, then discard and wait for reclamation.
    ///
    /// `flatten_entry_limit == 0` always selects Chain; `usize::MAX` always
    /// selects Flatten.  The helper is deliberately data-plane agnostic: the
    /// privileged tests run it on btrfs and an unprivileged test runs it on
    /// overlay, so the shared code itself is executed locally.
    pub(super) fn layout_diff_accept_scenario(
        backend: &FilesystemBackend,
        base: &Path,
        flatten_entry_limit: usize,
    ) {
        assert!(
            flatten_entry_limit == 0 || flatten_entry_limit == usize::MAX,
            "layout scenario only defines the two forced layouts, got {flatten_entry_limit}"
        );
        let expect_chain = flatten_entry_limit == 0;
        let layout_name = if expect_chain { "Chain" } else { "Flatten" };
        let keep = base.join("keep");
        let remove = base.join("remove");
        let newfile = base.join("newfile");

        // Both files exist only in the parent view at fork time; the child
        // must later change one and delete the other.
        write_upper(backend, "root", &keep, "base-keep");
        write_upper(backend, "root", &remove, "base-remove");
        let parent = mount_for(backend, "root", base);
        assert!(
            parent.lower.is_empty(),
            "root environment must start without lower layers: {:?}",
            parent.lower
        );

        backend.fork("root", "child", &[]).unwrap();
        let child = mount_for(backend, "child", base);
        if expect_chain {
            assert_eq!(
                child.lower.len(),
                parent.lower.len() + 1,
                "Chain layout must add exactly one snapshot layer to the child lower chain: parent={:?} child={:?}",
                parent.lower,
                child.lower
            );
            assert_eq!(
                &child.lower[..parent.lower.len()],
                &parent.lower[..],
                "Chain layout must keep the parent lower chain as a prefix"
            );
            assert!(
                dir_is_empty(&child.upper),
                "Chain layout must start the child upper empty: {}",
                child.upper.display()
            );
        } else {
            assert_eq!(
                child.lower, parent.lower,
                "Flatten layout must not add a layer to the child lower chain"
            );
            assert!(
                upper_at(backend, "child", &keep).exists(),
                "Flatten layout must copy the parent upper into the child upper: {}",
                keep.display()
            );
            assert!(
                upper_at(backend, "child", &remove).exists(),
                "Flatten layout must copy the parent upper into the child upper: {}",
                remove.display()
            );
        }

        let base_id = FilesystemBackend::base_snapshot_id_for_child("child");
        let layer_path = {
            let state = backend.lock().unwrap();
            state.snapshots[&base_id].path.clone()
        };
        let child_root = backend.paths("child");

        assert!(
            run_in_environment(
                backend,
                "child",
                base,
                "test -f keep && test -f remove && \
                 printf modified > keep && rm -f remove && printf new > newfile"
            ),
            "child process could not exercise the {layout_name} layout (limit={flatten_entry_limit})"
        );

        let diff = backend.diff(&base_id, "child", &[]).unwrap();
        let document: DiffDocument =
            serde_json::from_slice(&fs::read(diff.diff_ref.unwrap()).unwrap()).unwrap();
        let changes: BTreeSet<(String, String)> = document
            .changes
            .iter()
            .map(|change| (change.path.clone(), change.kind.clone()))
            .collect();
        let expected: BTreeSet<(String, String)> = [
            (keep.display().to_string(), "change".to_string()),
            (remove.display().to_string(), "delete".to_string()),
            (newfile.display().to_string(), "change".to_string()),
        ]
        .into_iter()
        .collect();
        let mut expected = expected;
        if expect_chain {
            // Chain diffs scan the child upper directly, so the directories
            // copied up for the nested paths are also reported.  Account for
            // exactly those; any other entry still fails the comparison.
            for file in [&keep, &remove, &newfile] {
                expected.extend(
                    ancestor_directories(&child.target, file)
                        .into_iter()
                        .map(|directory| (directory.display().to_string(), "change".to_string())),
                );
            }
        }
        assert_eq!(
            changes, expected,
            "diff must report exactly the child's three changes for limit={flatten_entry_limit}"
        );

        backend.suspend("root").unwrap();
        let accepted = backend
            .accept(&base_id, "child", "root", &[], None, None)
            .unwrap();
        assert!(
            accepted.conflicts.is_empty(),
            "layout scenario accept reported conflicts: {:?}",
            accepted.conflicts
        );
        assert!(accepted.confirmation.published);
        backend.resume("root").unwrap();
        assert!(
            run_in_environment(
                backend,
                "root",
                base,
                "test \"$(cat keep)\" = modified && test ! -e remove && \
                 test \"$(cat newfile)\" = new"
            ),
            "accepted child changes are not visible in the root real view"
        );

        backend.terminate("child").unwrap();
        backend.discard("child", &[]).unwrap();
        wait_until_for(
            || !child_root.exists() && !layer_path.exists(),
            1500,
        );
        assert!(!child_root.exists(), "discard left child data at {}", child_root.display());
        assert!(!layer_path.exists(), "discard left the snapshot layer at {}", layer_path.display());

        backend.terminate("root").unwrap();
        let root_root = backend.paths("root");
        backend.discard("root", &[]).unwrap();
        wait_until_for(|| !root_root.exists() && trash_is_empty(&backend.trash_root()), 1500);
        assert!(!root_root.exists(), "discard left root data at {}", root_root.display());
        assert!(
            trash_is_empty(&backend.trash_root()),
            "cleanup left retired subvolumes/entries in {}",
            backend.trash_root().display()
        );
        assert!(
            dir_is_empty(&backend.environments_root()),
            "cleanup left environment data in {}",
            backend.environments_root().display()
        );
        assert!(
            dir_is_empty(&backend.snapshots_root()),
            "cleanup left snapshot data in {}",
            backend.snapshots_root().display()
        );
        assert!(
            dir_is_empty(&backend.layers_root()),
            "cleanup left snapshot layers in {}",
            backend.layers_root().display()
        );
    }

    /// Two fork generations on one mount: a large root upper selects Chain,
    /// then a small child upper selects Flatten.  This mirrors
    /// `mixed_chain_and_flatten_layout_diff_and_accept_preserve_parent_view`
    /// on the btrfs data plane.
    pub(super) fn mixed_generations_scenario(backend: &FilesystemBackend, base: &Path) {
        let keep = base.join("keep");
        let remove = base.join("remove");
        let newfile = base.join("newfile");
        write_upper(backend, "root", &keep, "base-keep");
        write_upper(backend, "root", &remove, "base-remove");
        for index in 0..40 {
            write_upper(
                backend,
                "root",
                &base.join(format!("filler-{index}")),
                "filler",
            );
        }

        backend.fork("root", "child1", &[]).unwrap();
        let child1 = mount_for(backend, "child1", base);
        assert_eq!(
            child1.lower.len(), 1,
            "large root upper must chain: child1 lower={:?}",
            child1.lower
        );
        assert!(
            dir_is_empty(&child1.upper),
            "chained child1 upper must start empty"
        );
        write_upper(
            backend,
            "child1",
            &base.join("child1-only"),
            "child1-only",
        );

        backend.fork("child1", "grandchild", &[]).unwrap();
        let grandchild = mount_for(backend, "grandchild", base);
        assert_eq!(
            grandchild.lower, child1.lower,
            "small child1 upper must flatten without adding a layer"
        );
        assert!(
            upper_at(backend, "grandchild", &base.join("child1-only")).exists(),
            "flattened grandchild upper must carry child1's own write"
        );
        assert!(
            !upper_at(backend, "grandchild", &keep).exists(),
            "root files must come from the inherited lower layer, not the flattened upper"
        );

        assert!(
            run_in_environment(
                backend,
                "grandchild",
                base,
                "test -f keep && test -f remove && test -f child1-only && \
                 printf modified > keep && rm -f remove && printf new > newfile"
            ),
            "grandchild did not see the mixed Chain + Flatten view"
        );

        let base_id = FilesystemBackend::base_snapshot_id_for_child("grandchild");
        let grandchild_root = backend.paths("grandchild");
        let diff = backend.diff(&base_id, "grandchild", &[]).unwrap();
        let document: DiffDocument =
            serde_json::from_slice(&fs::read(diff.diff_ref.unwrap()).unwrap()).unwrap();
        let changes: BTreeSet<(String, String)> = document
            .changes
            .iter()
            .map(|change| (change.path.clone(), change.kind.clone()))
            .collect();
        let expected: BTreeSet<(String, String)> = [
            (keep.display().to_string(), "change".to_string()),
            (remove.display().to_string(), "delete".to_string()),
            (newfile.display().to_string(), "change".to_string()),
        ]
        .into_iter()
        .collect();
        assert_eq!(
            changes, expected,
            "mixed-layout diff must report exactly the three grandchild changes"
        );

        backend.suspend("child1").unwrap();
        let accepted = backend
            .accept(&base_id, "grandchild", "child1", &[], None, None)
            .unwrap();
        assert!(accepted.conflicts.is_empty(), "{:?}", accepted.conflicts);
        assert!(accepted.confirmation.published);
        backend.resume("child1").unwrap();
        assert!(
            run_in_environment(
                backend,
                "child1",
                base,
                "test \"$(cat keep)\" = modified && test ! -e remove && \
                 test \"$(cat newfile)\" = new && test -f child1-only"
            ),
            "accepted changes are not visible in child1"
        );
        assert!(
            run_in_environment(
                backend,
                "root",
                base,
                "test \"$(cat keep)\" = base-keep && test -f remove && test ! -e newfile"
            ),
            "accept into child1 changed the parent root view"
        );

        backend.terminate("grandchild").unwrap();
        backend.discard("grandchild", &[]).unwrap();
        backend.terminate("child1").unwrap();
        backend.discard("child1", &[]).unwrap();
        backend.terminate("root").unwrap();
        backend.discard("root", &[]).unwrap();
        wait_until_for(
            || !grandchild_root.exists() && trash_is_empty(&backend.trash_root()),
            1500,
        );
        assert!(!grandchild_root.exists(), "discard left grandchild data");
        assert!(
            trash_is_empty(&backend.trash_root()),
            "cleanup left retired subvolumes/entries in {}",
            backend.trash_root().display()
        );
        assert!(
            dir_is_empty(&backend.environments_root()),
            "cleanup left environment data in {}",
            backend.environments_root().display()
        );
        assert!(
            dir_is_empty(&backend.snapshots_root()),
            "cleanup left snapshot data in {}",
            backend.snapshots_root().display()
        );
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

    /// Privileged tests need a workspace on a *different* mount than the
    /// state directory so the workspace mount is not forced to Flatten by the
    /// overlap rule.  Unlike the state directory there is no safe default.
    pub(super) fn priv_workspace_root() -> PathBuf {
        let root = std::env::var_os("ASCOUT_PRIV_WORKSPACE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                panic!(
                    "ASCOUT_PRIV_TESTS=1 requires ASCOUT_PRIV_WORKSPACE_DIR to name a writable \
                     directory on a mount different from ASCOUT_PRIV_STATE_DIR"
                )
            });
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

    pub(super) fn wait_until(condition: impl FnMut() -> bool) {
        wait_until_for(condition, 500);
    }

    pub(super) fn wait_until_for(mut condition: impl FnMut() -> bool, attempts: usize) {
        for _ in 0..attempts {
            if condition() {
                return;
            }
            thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(condition(), "condition was not met in time");
    }
