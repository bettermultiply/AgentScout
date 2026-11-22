
use super::super::*;
use super::support::*;
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::FileTypeExt;

#[test]
fn filesystem_operation_replays_same_generation_and_rejects_stale_generation() {
    let root = temp();
    let backend = new_backend(root.path());
    backend.establish("root").unwrap();
    let operation = Operation::FilesystemFork {
        parent_snapshot_id: "root".into(),
        child_snapshot_id: "child".into(),
        path_scope: vec![PathScope::Workspace],
    };
    let context = BackendContext {
        operation_id: "fork-1".into(),
        generation: 7,
        environment_id: "root".into(),
        ..BackendContext::default()
    };
    let cancel = CancelHandle::default();
    let first = backend
        .filesystem_operation(&context, &operation, &cancel)
        .unwrap();
    drop(backend);

    let backend =
        FilesystemBackend::new(overlapping_state_config(root.path().join("state"))).unwrap();
    assert!(backend.state("root").is_some());
    assert!(backend.state("child").is_some());
    let second = backend
        .filesystem_operation(&context, &operation, &cancel)
        .unwrap();
    assert_eq!(first.snapshot_id, second.snapshot_id);
    assert!(second.confirmation.snapshot_created);

    let stale = BackendContext {
        generation: 6,
        ..context.clone()
    };
    let error = backend
        .filesystem_operation(&stale, &operation, &cancel)
        .unwrap_err();
    assert!(error.message.contains("stale generation"));
}

#[test]
fn base_runtime_reaches_the_filesystem_data_plane() {
    use crate::runtime::{BackendConnection, BackendPolicy, BaseRuntime};
    use std::sync::Arc;

    let root = temp();
    let filesystem = Arc::new(new_backend(root.path()));
    let runtime = BaseRuntime::with_policy(
        "runtime",
        vec![BackendConnection::new("filesystem", filesystem).unwrap()],
        BackendPolicy::single("filesystem"),
    )
    .unwrap();
    runtime.establish("root").unwrap();
    let operation = Operation::FilesystemFork {
        parent_snapshot_id: "root".into(),
        child_snapshot_id: "child".into(),
        path_scope: vec![PathScope::Workspace],
    };
    let context = BackendContext {
        operation_id: "runtime-fork".into(),
        generation: 1,
        environment_id: "root".into(),
        ..BackendContext::default()
    };
    let result = runtime
        .filesystem_operation(&context, &operation, &CancelHandle::default())
        .unwrap();
    assert!(result.confirmation.snapshot_created);
    assert!(result.confirmation.metadata_durable);
}

#[test]
fn btrfs_snapshot_copy_fork_is_end_to_end() {
    let root = btrfs_temp();
    let backend =
        FilesystemBackend::new(overlapping_state_config(root.path().join("state"))).unwrap();
    if backend.fork_cost() != ForkCost::BtrfsSnapshot
        || backend.mechanism() != FilesystemMechanism::Overlay
    {
        eprintln!(
            "SKIP btrfs_snapshot_copy_fork_is_end_to_end: state directory is not btrfs or snapshot copies unavailable (mechanism={}, fork_cost={})",
            backend.mechanism().as_str(),
            backend.fork_cost().as_str()
        );
        return;
    }
    backend.establish("root").unwrap();
    let base = root.path().join("base");
    fs::create_dir_all(&base).unwrap();
    write(&base, "fork.txt", "at-fork");
    let parent_dir = upper_at(&backend, "root", &base.join("xattr-dir"));
    fs::create_dir_all(&parent_dir).unwrap();
    set_test_xattr(&parent_dir, "user.overlay.opaque", b"y");
    backend.fork("root", "child", &[]).unwrap();
    let child_dir = upper_at(&backend, "child", &base.join("xattr-dir"));
    assert!(
        get_xattr(&child_dir, "user.overlay.opaque").is_some(),
        "btrfs snapshot lost user.overlay.* xattr"
    );
    let parent_upper = upper_at(&backend, "root", &base);
    write_upper(&backend, "root", &base.join("late.txt"), "late");
    assert!(
        run_in_environment(
            &backend,
            "child",
            &base,
            "test -f fork.txt && test ! -e late.txt"
        ),
        "child did not observe the fork-time snapshot"
    );
    assert!(
        run_in_environment(
            &backend,
            "child",
            &base,
            "rm -f fork.txt && test ! -e fork.txt"
        ),
        "child whiteout failed"
    );
    let whiteout = upper_at(&backend, "child", &base.join("fork.txt"));
    assert!(
        fs::symlink_metadata(&whiteout)
            .map(|metadata| metadata.file_type().is_char_device())
            .unwrap_or(false),
        "child whiteout is not a character device"
    );
    assert!(base.join("fork.txt").exists(), "child deleted host base");
    wait_until(|| parent_upper.join("late.txt").exists());
    let child_root = backend.paths("child");
    backend.terminate("child").unwrap();
    backend.discard("child", &[]).unwrap();
    wait_until(|| !child_root.exists());
    assert!(!child_root.exists(), "discard left btrfs child subvolumes");
    let trash = root.path().join("state/trash");
    wait_until(|| trash_is_empty(&trash));
    assert!(
        trash_is_empty(&trash),
        "background removal left btrfs subvolumes in trash"
    );
    backend.terminate("root").unwrap();
    backend.cleanup("root").unwrap();
}

#[test]
fn failed_clone_and_orphan_staging_are_not_valid_layers() {
    let root = temp();
    let backend = new_overlay_backend(root.path());
    backend.establish("root").unwrap();
    let base = root.path().join("base");
    fs::create_dir_all(&base).unwrap();
    for index in 0..4 {
        write_upper(&backend, "root", &base.join(format!("f{index}")), "x");
    }
    set_clone_failure_after(Some(2));
    let error = backend.fork("root", "child", &[]).unwrap_err();
    set_clone_failure_after(None);
    assert!(
        error.message.contains("injected clone failure"),
        "{}",
        error.message
    );
    assert!(backend.state("child").is_none());
    assert!(
        !root
            .path()
            .join("state/layers")
            .join(FilesystemBackend::base_snapshot_id_for_child("child"))
            .exists()
    );
    let layers = root.path().join("state/layers");
    let orphans: Vec<_> = fs::read_dir(&layers)
        .unwrap()
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(".tmp-"))
        .collect();
    assert!(orphans.is_empty(), "failed clone left staging dirs");
    backend.terminate("root").unwrap();
    backend.cleanup("root").unwrap();
    drop(backend);

    let orphan = layers.join(".tmp-orphan");
    fs::create_dir_all(&orphan).unwrap();
    fs::write(orphan.join("partial"), b"x").unwrap();
    let backend =
        FilesystemBackend::new(overlapping_state_config(root.path().join("state"))).unwrap();
    assert!(!orphan.exists(), "backend start must remove orphan staging");
    drop(backend);
}

#[test]
fn accept_replay_rolls_back_every_injected_failure() {
    let root = temp();
    let token = format!("ascout-rollback-{}", std::process::id());
    let modify = PathBuf::from(format!("/tmp/{token}-modify"));
    let delete = PathBuf::from(format!("/tmp/{token}-delete"));
    let create = PathBuf::from(format!("/tmp/{token}-create"));
    let backend = new_overlay_backend(root.path());
    backend.establish("root").unwrap();
    if !backend
        .overlayable_mounts()
        .iter()
        .any(|target| target == Path::new("/tmp"))
    {
        eprintln!(
            "SKIP accept replay rollback: /tmp is not an overlayable mount (candidates: {:?})",
            backend.overlayable_mounts()
        );
        return;
    }
    write_upper(&backend, "root", &modify, "old");
    write_upper(&backend, "root", &delete, "keep");
    backend.fork("root", "child", &[]).unwrap();
    let snapshot = FilesystemBackend::base_snapshot_id_for_child("child");
    write_upper(&backend, "child", &modify, "new");
    write_upper(&backend, "child", &create, "new");
    let child_upper = upper_at(&backend, "child", Path::new("/tmp"));
    create_whiteout(&child_upper, delete.file_name().unwrap().to_str().unwrap()).unwrap();
    backend.suspend("root").unwrap();

    for fail_after in 0..3 {
        backend.inject_replay_failure_after(Some(fail_after));
        let error = backend
            .accept(&snapshot, "child", "root", &[], None, None)
            .expect_err("injected replay failure must not publish");
        assert!(
            error.message.contains("injected replay failure") || error.message.contains("rollback"),
            "unexpected error: {}",
            error.message
        );
        assert_eq!(
            fs::read_to_string(upper_at(&backend, "root", &modify)).unwrap(),
            "old"
        );
        assert_eq!(
            fs::read_to_string(upper_at(&backend, "root", &delete)).unwrap(),
            "keep"
        );
        assert!(!upper_at(&backend, "root", &create).exists());
    }
    backend.inject_replay_failure_after(None);
    let accepted = backend
        .accept(&snapshot, "child", "root", &[], None, None)
        .unwrap();
    assert!(accepted.confirmation.published);
    assert_eq!(
        fs::read_to_string(upper_at(&backend, "root", &modify)).unwrap(),
        "new"
    );
    assert!(upper_at(&backend, "root", &create).exists());
    let journal = root.path().join("state/replay-journal");
    assert!(
        !journal.exists() || fs::read_dir(&journal).unwrap().next().is_none(),
        "successful accept must clean the replay journal"
    );
    backend.resume("root").unwrap();
    backend.terminate("child").unwrap();
    backend.discard("child", &[]).unwrap();
    backend.terminate("root").unwrap();
    backend.cleanup("root").unwrap();
}

#[test]
fn replay_directory_replacement_rolls_back() {
    let root = temp();
    let upper = root.path().join("upper");
    fs::create_dir_all(upper.join("replace")).unwrap();
    fs::write(upper.join("replace/old.txt"), b"old").unwrap();
    let source = root.path().join("source.txt");
    fs::write(&source, b"new").unwrap();
    let mut delta = View::new();
    delta.insert(
        "replace".into(),
        Node {
            mode: libc::S_IFDIR | 0o755,
            size: 0,
            mtime_sec: 0,
            mtime_nsec: 0,
            inode: 0,
            kind: NodeKind::Dir,
            source: None,
            opaque: true,
            redirect: None,
            metacopy: false,
        },
    );
    delta.insert(
        "replace/new.txt".into(),
        Node {
            mode: libc::S_IFREG | 0o644,
            size: 3,
            mtime_sec: 0,
            mtime_nsec: 0,
            inode: 0,
            kind: NodeKind::File,
            source: Some(source),
            opaque: false,
            redirect: None,
            metacopy: false,
        },
    );
    let mut journal = ReplayJournal::new(root.path().join("journal")).unwrap();
    let error = replay_delta(
        &ReplayRoot::Upper(&upper),
        &delta,
        &[],
        &[],
        &mut journal,
        &upper,
        Some(1),
    )
    .unwrap_err();
    assert!(
        error.message.contains("injected replay failure"),
        "{}",
        error.message
    );
    let (failures, _) = journal.rollback();
    assert!(failures.is_empty(), "rollback failed: {failures:?}");
    assert_eq!(fs::read(upper.join("replace/old.txt")).unwrap(), b"old");
    assert!(!upper.join("replace/new.txt").exists());
}

#[test]
fn accept_replay_reports_rollback_failure() {
    let root = temp();
    let token = format!("ascout-rollback-fail-{}", std::process::id());
    let modify = PathBuf::from(format!("/tmp/{token}-modify"));
    let added = PathBuf::from(format!("/tmp/{token}-added"));
    let backend = new_overlay_backend(root.path());
    backend.establish("root").unwrap();
    if !backend
        .overlayable_mounts()
        .iter()
        .any(|target| target == Path::new("/tmp"))
    {
        eprintln!(
            "SKIP accept replay rollback: /tmp is not an overlayable mount (candidates: {:?})",
            backend.overlayable_mounts()
        );
        return;
    }
    write_upper(&backend, "root", &modify, "old");
    backend.fork("root", "child", &[]).unwrap();
    let snapshot = FilesystemBackend::base_snapshot_id_for_child("child");
    write_upper(&backend, "child", &modify, "new");
    write_upper(&backend, "child", &added, "new");
    backend.suspend("root").unwrap();
    backend.inject_replay_failure_after(Some(1));
    backend.inject_rollback_failure(true);
    let error = backend
        .accept(&snapshot, "child", "root", &[], None, None)
        .expect_err("injected failure must fail accept");
    assert!(
        error.message.contains("rollback failed"),
        "rollback failure was not reported: {}",
        error.message
    );
    assert!(
        error
            .cleanup_failures
            .iter()
            .any(|value| value.contains("rollback")),
        "missing rollback cleanup failure: {:?}",
        error.cleanup_failures
    );
    backend.inject_replay_failure_after(None);
    backend.inject_rollback_failure(false);
    backend.resume("root").unwrap();
    backend.terminate("child").unwrap();
    backend.discard("child", &[]).unwrap();
    backend.terminate("root").unwrap();
    backend.cleanup("root").unwrap();
}

#[test]
fn backend_overlay_fork_diff_accept_discard_round_trip() {
    round_trip(temp(), false);
}

#[test]
fn backend_btrfs_fork_diff_accept_discard_round_trip() {
    round_trip(btrfs_temp(), true);
}

fn round_trip(root: tempfile::TempDir, expect_btrfs: bool) {
    let backend =
        FilesystemBackend::new(overlapping_state_config(root.path().join("state"))).unwrap();
    assert!(
        backend.usable,
        "bubblewrap overlay probe failed; filesystem tests require a working sandbox"
    );
    if expect_btrfs && backend.mechanism() != FilesystemMechanism::Btrfs {
        eprintln!(
            "SKIP backend_btrfs_fork_diff_accept_discard_round_trip: btrfs needs create+snapshot+incremental_diff; selected {}",
            backend.mechanism().as_str()
        );
        return;
    }
    backend.establish("root").unwrap();
    let workspace = root.path().join("base");
    fs::create_dir_all(&workspace).unwrap();
    write_upper(&backend, "root", &workspace.join("parent.txt"), "parent");

    let fork = backend
        .fork("root", "child", &[PathScope::Workspace])
        .unwrap();
    assert!(fork.confirmation.snapshot_created);
    assert!(fork.confirmation.metadata_durable);
    assert!(!fork.confirmation.writers_quiesced);
    let base = FilesystemBackend::base_snapshot_id_for_child("child");
    write_upper(&backend, "child", &workspace.join("child.txt"), "child");

    if expect_btrfs {
        let diff = backend.diff(&base, "child", &[PathScope::Workspace]);
        assert!(
            diff.is_err(),
            "btrfs diff must fail explicitly without generation privileges"
        );
        assert!(diff.unwrap_err().message.contains("btrfs incremental diff"));
        let accept = backend.accept(&base, "child", "root", &[PathScope::Workspace], None, None);
        assert!(accept.is_err());
    } else {
        let diff = backend
            .diff(&base, "child", &[PathScope::Workspace])
            .unwrap();
        assert!(diff.confirmation.metadata_durable);
        let document: DiffDocument =
            serde_json::from_slice(&fs::read(diff.diff_ref.unwrap()).unwrap()).unwrap();
        let child_path = workspace.join("child.txt").display().to_string();
        assert!(
            document
                .changes
                .iter()
                .any(|change| change.path == child_path && change.kind == "change")
        );

        let mark = backend
            .mark("child", "before-more", &[PathScope::Workspace])
            .unwrap();
        assert!(mark.confirmation.snapshot_created);
        write_upper(
            &backend,
            "child",
            &workspace.join("after-mark.txt"),
            "after",
        );
        let mark_diff = backend
            .diff(
                mark.snapshot_id.as_deref().unwrap(),
                "child",
                &[PathScope::Workspace],
            )
            .unwrap();
        let mark_document: DiffDocument =
            serde_json::from_slice(&fs::read(mark_diff.diff_ref.unwrap()).unwrap()).unwrap();
        let after_path = workspace.join("after-mark.txt").display().to_string();
        let child_path = workspace.join("child.txt").display().to_string();
        assert!(
            mark_document
                .changes
                .iter()
                .any(|change| change.path == after_path && change.kind == "change")
        );
        assert!(
            mark_document
                .changes
                .iter()
                .all(|change| change.path != child_path)
        );

        write_upper(
            &backend,
            "root",
            &workspace.join("authority-progress.txt"),
            "keep",
        );
        backend.suspend("root").unwrap();
        let accept = backend
            .accept_environment_state(&BackendContext::new("root"), "child", true, None)
            .unwrap();
        assert_eq!(
            accept.acceptance,
            Some(crate::protocol::contract::Acceptance::Full)
        );
        assert!(accept.conflicts.is_empty());
        assert!(accept.confirmation.published);
        assert!(upper_at(&backend, "root", &workspace.join("child.txt")).exists());
        assert_eq!(
            fs::read_to_string(upper_at(
                &backend,
                "root",
                &workspace.join("authority-progress.txt")
            ))
            .unwrap(),
            "keep"
        );

        let conflicting_path = workspace.join("child.txt");
        write_upper(&backend, "root", &conflicting_path, "authority wins");
        let target = BackendContext::new("root");
        let rejected = backend
            .accept_environment_state(&target, "child", true, None)
            .unwrap();
        assert!(rejected.acceptance.is_none());
        assert!(!rejected.conflicts.is_empty());
        let downgraded = backend
            .accept_environment_state(&target, "child", true, Some(&crate::protocol::contract::PartSelection::Scopes(vec![])))
            .unwrap();
        assert_eq!(
            downgraded.acceptance,
            Some(crate::protocol::contract::Acceptance::Part { scopes: vec![] })
        );
        assert_eq!(
            fs::read_to_string(upper_at(&backend, "root", &conflicting_path)).unwrap(),
            "authority wins"
        );
    }

    backend.terminate("child").unwrap();
    let discard = backend.discard("child", &[PathScope::Workspace]).unwrap();
    assert!(discard.confirmation.refs_released);
    assert!(discard.confirmation.gc_eligible);
    assert!(backend.state("child").is_none());
    wait_removed(&root.path().join("environments").join("child"));

    backend.terminate("root").unwrap();
    backend.cleanup("root").unwrap();
    wait_removed(&root.path().join("environments").join("root"));
    wait_until(|| {
        !path_exists(&root.path().join("snapshots").join(&base))
            && !path_exists(&root.path().join("layers").join(&base))
    });
    wait_until(|| trash_is_empty(&root.path().join("trash")));
}

#[test]
fn real_overlay_whiteout_survives_fork_diff_and_accept() {
    let root = btrfs_temp();
    let base = root.path().join("base");
    write(&base, "x", "base");
    let backend = new_overlay_backend(root.path());
    backend.establish("root").unwrap();
    backend
        .fork("root", "child", &[PathScope::Workspace])
        .unwrap();

    assert!(run_in_environment(&backend, "child", &base, "rm -f x"));
    assert_eq!(fs::read(base.join("x")).unwrap(), b"base");
    let child_layer = read_upper_layer_at(&backend, "child", &base);
    assert!(child_layer.get("x").is_some_and(Node::is_whiteout));
    assert!(!run_in_environment(&backend, "child", &base, "test -e x"));

    let base_snapshot = FilesystemBackend::base_snapshot_id_for_child("child");
    let diff = backend
        .diff(&base_snapshot, "child", &[PathScope::Workspace])
        .unwrap();
    let document: DiffDocument =
        serde_json::from_slice(&fs::read(diff.diff_ref.unwrap()).unwrap()).unwrap();
    let deleted = base.join("x").display().to_string();
    assert!(
        document
            .changes
            .iter()
            .any(|change| change.path == deleted && change.kind == "delete")
    );

    backend.suspend("root").unwrap();
    let accept = backend
        .accept(
            &base_snapshot,
            "child",
            "root",
            &[PathScope::Workspace],
            None,
            None,
        )
        .unwrap();
    assert!(accept.conflicts.is_empty());
    assert!(accept.confirmation.published);
    assert!(!accept.confirmation.target_paused);

    backend.resume("root").unwrap();
    assert!(!run_in_environment(&backend, "root", &base, "test -e x"));
    assert_eq!(fs::read(base.join("x")).unwrap(), b"base");

    backend.terminate("child").unwrap();
    backend.discard("child", &[PathScope::Workspace]).unwrap();
    backend.terminate("root").unwrap();
    backend.cleanup("root").unwrap();
    wait_until(|| trash_is_empty(&root.path().join("trash")));
}

#[test]
fn btrfs_mount_copy_up_rename_stays_within_subvolume() {
    let root = btrfs_temp();
    let mut backend = new_overlay_backend(root.path());
    let probe = root.path().join("subvolume-probe");
    if let Err(error) = backend.btrfs_create(&probe) {
        eprintln!("SKIP btrfs_mount_copy_up_rename_stays_within_subvolume: {error}");
        return;
    }
    remove_path(&backend.config.btrfs_path, &probe, true).unwrap();
    // Exercise btrfs layout on hosts without privileged send.
    backend.mechanism = FilesystemMechanism::Btrfs;
    // Use fixture lowers to avoid reading root-only entries on host mounts.
    let lowers = tempfile::Builder::new()
        .prefix("control-plane-refactor-lowers-")
        .tempdir_in("/tmp/opencode")
        .unwrap();
    let targets = vec![lowers.path().join("first"), lowers.path().join("second")];
    for target in &targets {
        fs::create_dir(target).unwrap();
    }
    assert!(!targets.is_empty(), "test needs a non-overlapping mount");
    *backend.overlay_targets.lock().unwrap() = Some(vec![targets[0].clone()]);
    let result = (|| -> Result<(), BackendError> {
        backend.establish("root")?;
        *backend.overlay_targets.lock().unwrap() = Some(targets.clone());
        let launch = crate::protocol::contract::AgentLaunch {
            command: "/bin/true".into(),
            args: Vec::new(),
            cwd: targets[0].clone(),
            log: root.path().join("unused.log"),
            instance_id: None,
            env: BTreeMap::new(),
        };
        backend.launch_fragment(&BackendContext::new("root"), &launch)?;
        assert_eq!(
            backend.lock()?.environments["root"].mounts.len(),
            targets.len()
        );
        backend.fork("root", "empty", &[])?;
        let empty_mounts = backend.lock()?.environments["empty"].mounts.clone();
        for mount in empty_mounts.values() {
            assert!(upper_is_empty(&mount.upper));
            assert!(mount.lower.is_empty());
            fs::write(mount.work.join("copy-up"), b"empty-child").map_err(error)?;
            fs::rename(mount.work.join("copy-up"), mount.upper.join("keep")).map_err(error)?;
        }
        for (parent, child, limit) in [("root", "chain", 0), ("chain", "flatten", usize::MAX)] {
            let mounts = {
                let state = backend.lock()?;
                state.environments[parent]
                    .mounts
                    .values()
                    .cloned()
                    .collect::<Vec<_>>()
            };
            for mount in &mounts {
                fs::write(mount.work.join("copy-up"), b"copy-up").map_err(error)?;
                fs::rename(mount.work.join("copy-up"), mount.upper.join("keep")).map_err(
                    |value| {
                        BackendError::new(format!(
                            "work {} -> upper {} rename failed: {value}",
                            mount.work.display(),
                            mount.upper.display()
                        ))
                    },
                )?;
            }
            backend.config.fork_flatten_entry_limit = limit;
            backend.fork(parent, child, &[])?;
            let mounts = backend.lock()?.environments[child].mounts.clone();
            for mount in mounts.values() {
                assert_eq!(upper_is_empty(&mount.upper), limit == 0);
                assert_eq!(mount.lower.len(), 1);
            }
        }
        let mounts = backend.lock()?.environments["flatten"].mounts.clone();
        for mount in mounts.values() {
            fs::write(mount.work.join("copy-up"), b"last").map_err(error)?;
            fs::rename(mount.work.join("copy-up"), mount.upper.join("keep")).map_err(error)?;
        }
        backend.mark("flatten", "rename-domain", &[])?;
        backend.load_metadata()?;
        let legacy = backend.paths("legacy").join("mount");
        fs::create_dir_all(&legacy).map_err(error)?;
        backend.btrfs_create(&legacy.join("upper"))?;
        fs::create_dir(legacy.join("work")).map_err(error)?;
        fs::write(legacy.join("upper/keep"), b"preserved").map_err(error)?;
        fs::write(legacy.join("work/untouched"), b"preserved-work").map_err(error)?;
        let mut environment = backend.lock()?.environments["root"].clone();
        environment.id = "legacy".into();
        environment.root = backend.paths("legacy");
        let mut mount = environment.mounts.values().next().unwrap().clone();
        mount.upper = legacy.join("upper");
        mount.work = legacy.join("work");
        environment.mounts = BTreeMap::from([(mount.target.display().to_string(), mount)]);
        backend.persist_environment(&environment)?;
        let rejected = backend.load_metadata().unwrap_err();
        assert!(rejected.message.contains("EXDEV"), "{rejected}");
        assert_eq!(fs::read(legacy.join("upper/keep")).unwrap(), b"preserved");
        assert_eq!(
            fs::read(legacy.join("work/untouched")).unwrap(),
            b"preserved-work"
        );
        Ok(())
    })();
    let cleanup = remove_path(
        &backend.config.btrfs_path,
        &backend.config.state_directory,
        true,
    );
    assert!(result.is_ok(), "{result:?}");
    cleanup.unwrap();
}

#[test]
fn live_target_accept_replays_through_inner_sandbox_pid() {
    let root = btrfs_temp();
    let base = root.path().join("base");
    write(&base, "x", "base");
    let backend = new_overlay_backend(root.path());
    backend.establish("root").unwrap();
    backend
        .fork("root", "child", &[PathScope::Workspace])
        .unwrap();
    assert!(run_in_environment(&backend, "child", &base, "rm -f x"));
    let base_snapshot = FilesystemBackend::base_snapshot_id_for_child("child");
    let started = std::time::Instant::now();
    let (target, target_pid, target_ns) = spawn_in_environment(
        &backend,
        "root",
        &base,
        "test -e x || exit 20; printf ready > .accept-ready; \
             while [ -e x ]; do sleep 0.05; done; echo ASCOUT_ACCEPT_DELETION_OBSERVED; exit 0",
    );
    let spawned = started.elapsed();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let ready = upper_at(&backend, "root", &base.join(".accept-ready"));
    while !ready.exists() {
        if target.try_wait("root").unwrap().is_some() || std::time::Instant::now() >= deadline {
            let _ = target.stop("root");
            panic!(
                "target did not confirm its initial view: spawn={spawned:?} elapsed={:?} pid={target_pid} log={}",
                started.elapsed(),
                fs::read_to_string(base.join("agent.log")).unwrap_or_default()
            );
        }
        thread::sleep(std::time::Duration::from_millis(10));
    }
    let observing = started.elapsed();
    backend.suspend("root").unwrap();

    // The outer bwrap pid is in the host mount namespace and must be rejected.
    let outer = std::process::id();
    let error = backend
        .accept(
            &base_snapshot,
            "child",
            "root",
            &[PathScope::Workspace],
            Some(outer),
            Some(target_ns),
        )
        .unwrap_err();
    assert!(error.message.contains("host mount namespace"));

    let accept = backend
        .accept(
            &base_snapshot,
            "child",
            "root",
            &[PathScope::Workspace],
            Some(target_pid),
            Some(target_ns),
        )
        .unwrap();
    assert!(accept.confirmation.published);
    let published = started.elapsed();
    let visible = fs::metadata(format!("/proc/{target_pid}/root{}/x", base.display()));
    assert!(
        visible
            .as_ref()
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound),
        "accepted deletion is still visible in the recorded sandbox: {visible:?}"
    );

    let mut exited = false;
    while std::time::Instant::now() < deadline {
        if let Some(status) = target.try_wait("root").unwrap() {
            assert!(status.success());
            exited = true;
            break;
        }
        thread::sleep(std::time::Duration::from_millis(10));
    }
    if !exited {
        let _ = target.stop("root");
        panic!(
            "target did not exit after accepted deletion: spawn={spawned:?} observing={observing:?} \
                 published={published:?} elapsed={:?} pid={target_pid} namespace={target_ns} log={}",
            started.elapsed(),
            fs::read_to_string(base.join("agent.log")).unwrap_or_default()
        );
    }
    eprintln!(
        "live-target timing: spawn={spawned:?} observing={observing:?} published={published:?} exited={:?}",
        started.elapsed()
    );
    target.cleanup("root").unwrap();
    assert_eq!(fs::read(base.join("x")).unwrap(), b"base");
    backend.terminate("child").unwrap();
    backend.discard("child", &[PathScope::Workspace]).unwrap();
    backend.terminate("root").unwrap();
    backend.cleanup("root").unwrap();
}

#[test]
fn extra_mount_fork_diff_accept_discard_round_trip() {
    let root = btrfs_temp();
    let base = root.path().join("base");
    fs::create_dir_all(&base).unwrap();
    let backend = new_overlay_backend(root.path());
    let candidates = backend.overlayable_mounts();
    let Some(target) = ["/var/tmp", "/srv"]
        .into_iter()
        .map(PathBuf::from)
        .find(|target| candidates.iter().any(|candidate| candidate == target))
    else {
        eprintln!(
            "SKIP extra_mount_fork_diff_accept_discard_round_trip: no second writable overlayable mount (candidates: {:?})",
            candidates
        );
        return;
    };
    if unsafe {
        libc::access(
            CString::new(target.as_os_str().as_bytes())
                .unwrap()
                .as_ptr(),
            libc::W_OK,
        )
    } != 0
    {
        eprintln!(
            "SKIP extra_mount_fork_diff_accept_discard_round_trip: {} is not writable",
            target.display()
        );
        return;
    }
    eprintln!(
        "extra_mount_fork_diff_accept_discard_round_trip: using {}",
        target.display()
    );
    let token = format!("ascout-f1-{}", std::process::id());
    let parent_file = target.join(format!("{token}-a"));
    let child_file = target.join(format!("{token}-b"));
    let lower_file = target.join(format!("{token}-lower"));
    let _ = fs::remove_file(&parent_file);
    let _ = fs::remove_file(&child_file);
    let _ = fs::remove_file(&lower_file);
    fs::write(&lower_file, b"lower").unwrap();

    backend.establish("root").unwrap();
    let parent_script = format!("echo a > {}", parent_file.display());
    assert!(
        run_in_environment(&backend, "root", &base, &parent_script),
        "parent could not write {}",
        parent_file.display()
    );
    assert!(!parent_file.exists(), "parent write reached the host");
    backend.fork("root", "child", &[]).unwrap();

    assert!(
        run_in_environment(
            &backend,
            "child",
            &base,
            &format!("test -f {}", parent_file.display()),
        ),
        "child did not inherit the parent write in {}",
        target.display()
    );
    let child_script = format!(
        "echo b > {}; rm -f {}",
        child_file.display(),
        lower_file.display()
    );
    assert!(run_in_environment(&backend, "child", &base, &child_script));
    assert!(!child_file.exists(), "child write reached the host");
    assert!(lower_file.exists(), "child deleted the host lower file");

    let base_snapshot = FilesystemBackend::base_snapshot_id_for_child("child");
    let diff = backend.diff(&base_snapshot, "child", &[]).unwrap();
    let document: DiffDocument =
        serde_json::from_slice(&fs::read(diff.diff_ref.unwrap()).unwrap()).unwrap();
    assert!(
        document.changes.iter().any(
            |change| change.path == child_file.display().to_string() && change.kind == "change"
        ),
        "diff did not report {}: {:?}",
        child_file.display(),
        document.changes
    );
    assert!(
        document.changes.iter().any(
            |change| change.path == lower_file.display().to_string() && change.kind == "delete"
        ),
        "diff did not report delete of {}: {:?}",
        lower_file.display(),
        document.changes
    );

    backend.suspend("root").unwrap();
    let accept = backend
        .accept(&base_snapshot, "child", "root", &[], None, None)
        .unwrap();
    assert!(accept.conflicts.is_empty(), "{:?}", accept.conflicts);
    assert!(accept.confirmation.published);
    backend.resume("root").unwrap();
    assert!(
        run_in_environment(
            &backend,
            "root",
            &base,
            &format!(
                "test -f {} && ! test -e {}",
                child_file.display(),
                lower_file.display()
            ),
        ),
        "target did not observe accept of {} and deletion of {}",
        child_file.display(),
        lower_file.display()
    );
    assert!(!child_file.exists(), "accept wrote the host target mount");
    assert!(lower_file.exists(), "accept deleted the host lower file");

    let mark = backend.mark("child", "extra-mark", &[]).unwrap();
    let mark_file = target.join(format!("{token}-mark"));
    let _ = fs::remove_file(&mark_file);
    assert!(run_in_environment(
        &backend,
        "child",
        &base,
        &format!("echo marked > {}", mark_file.display()),
    ));
    let mark_diff = backend
        .diff(mark.snapshot_id.as_deref().unwrap(), "child", &[])
        .unwrap();
    let mark_document: DiffDocument =
        serde_json::from_slice(&fs::read(mark_diff.diff_ref.unwrap()).unwrap()).unwrap();
    assert!(
        mark_document
            .changes
            .iter()
            .any(|change| change.path == mark_file.display().to_string()),
        "mark diff did not cover {}: {:?}",
        mark_file.display(),
        mark_document.changes
    );
    assert!(
        mark_document
            .changes
            .iter()
            .all(|change| change.path != child_file.display().to_string()),
        "mark diff included a pre-mark change: {:?}",
        mark_document.changes
    );

    let child_upper = {
        let state = backend.lock().unwrap();
        let mount = state.environments["child"]
            .mount_by_target(&target)
            .expect("child mount unit")
            .clone();
        mount.upper.join(format!("{token}-b"))
    };
    assert!(
        child_upper.exists(),
        "child upper did not contain the write"
    );
    backend.terminate("child").unwrap();
    backend.discard("child", &[]).unwrap();
    wait_until(|| !child_upper.exists());
    assert!(!child_upper.exists(), "discard left extra-mount upper data");
    let _ = fs::remove_file(&parent_file);
    let _ = fs::remove_file(&child_file);
    let _ = fs::remove_file(&lower_file);
    let _ = fs::remove_file(&mark_file);
    backend.terminate("root").unwrap();
    backend.cleanup("root").unwrap();
}

#[test]
fn overlay_fork_isolates_parent_writes_after_the_copy() {
    let root = btrfs_temp();
    let base = root.path().join("base");
    fs::create_dir_all(&base).unwrap();
    let backend = new_overlay_backend(root.path());
    backend.establish("root").unwrap();
    let (parent, _, _) = spawn_in_environment(
        &backend,
        "root",
        &base,
        "touch ready; sleep 0.5; echo late > after.txt; sleep 3",
    );
    let parent_upper = upper_at(&backend, "root", &base);
    wait_until_for(|| parent_upper.join("ready").exists(), 3000);

    backend
        .fork("root", "child", &[PathScope::Workspace])
        .unwrap();
    wait_until_for(|| parent_upper.join("after.txt").exists(), 3000);

    assert!(
        run_in_environment(&backend, "child", &base, "test ! -e after.txt"),
        "child saw a parent write that happened after fork"
    );

    let _ = parent.stop("root");
    backend.terminate("child").unwrap();
    backend.discard("child", &[PathScope::Workspace]).unwrap();
    backend.terminate("root").unwrap();
    backend.cleanup("root").unwrap();
    wait_until(|| trash_is_empty(&root.path().join("trash")));
}

#[test]
fn layout_diff_accept_scenario_runs_on_overlay_for_both_limits() {
    for limit in [0usize, usize::MAX] {
        let root = btrfs_temp();
        let base = root.path().join("base");
        fs::create_dir_all(&base).unwrap();
        let Some((_state, backend)) = layout_backend(&base, limit) else {
            eprintln!(
                "SKIP layout_diff_accept_scenario_runs_on_overlay_for_both_limits: no writable mount separate from workspace"
            );
            return;
        };
        backend.establish("root").unwrap();
        layout_diff_accept_scenario(&backend, &base, limit);
        drop(backend);
    }
}

#[test]
fn mixed_chain_and_flatten_layout_diff_and_accept_preserve_parent_view() {
    let root = btrfs_temp();
    let base = root.path().join("base");
    fs::create_dir_all(&base).unwrap();
    let Some((_state, backend)) = layout_backend(&base, 32) else {
        eprintln!(
            "SKIP mixed_chain_and_flatten_layout_diff_and_accept_preserve_parent_view: no writable mount separate from workspace"
        );
        return;
    };
    backend.establish("root").unwrap();
    write_upper(&backend, "root", &base.join("keep"), "base-keep");
    write_upper(&backend, "root", &base.join("remove"), "base-remove");
    for index in 0..40 {
        write_upper(
            &backend,
            "root",
            &base.join(format!("filler-{index}")),
            "filler",
        );
    }

    backend.fork("root", "child1", &[]).unwrap();
    let child1 = mount_for(&backend, "child1", &base);
    assert_eq!(child1.lower.len(), 1, "root upper is large: child1 chains");
    write_upper(&backend, "child1", &base.join("child1-only"), "child1-only");

    backend.fork("child1", "grandchild", &[]).unwrap();
    let grandchild = mount_for(&backend, "grandchild", &base);
    assert_eq!(
        grandchild.lower, child1.lower,
        "child1 upper is small: grandchild flattens without adding S2 to its lower"
    );
    assert!(
        upper_at(&backend, "grandchild", &base.join("child1-only")).exists(),
        "flattened grandchild upper must carry child1's own write"
    );
    assert!(
        !upper_at(&backend, "grandchild", &base.join("keep")).exists(),
        "root files must come from the inherited S1 lower, not the flattened upper"
    );

    assert!(
        run_in_environment(
            &backend,
            "grandchild",
            &base,
            "test -f keep && test -f remove && test -f child1-only && \
                 printf modified > keep && rm -f remove && printf new > newfile"
        ),
        "grandchild did not see the mixed lower-chain + flattened view"
    );

    let grandchild_base = FilesystemBackend::base_snapshot_id_for_child("grandchild");
    let diff = backend.diff(&grandchild_base, "grandchild", &[]).unwrap();
    let document: DiffDocument =
        serde_json::from_slice(&fs::read(diff.diff_ref.unwrap()).unwrap()).unwrap();
    let changes: BTreeSet<(String, String)> = document
        .changes
        .iter()
        .map(|change| (change.path.clone(), change.kind.clone()))
        .collect();
    let expected: BTreeSet<(String, String)> = [
        (
            base.join("keep").display().to_string(),
            "change".to_string(),
        ),
        (
            base.join("remove").display().to_string(),
            "delete".to_string(),
        ),
        (
            base.join("newfile").display().to_string(),
            "change".to_string(),
        ),
    ]
    .into_iter()
    .collect();
    assert_eq!(
        changes, expected,
        "mixed-layout diff must report exactly the three grandchild changes"
    );

    backend.suspend("child1").unwrap();
    let accepted = backend
        .accept(&grandchild_base, "grandchild", "child1", &[], None, None)
        .unwrap();
    assert!(accepted.conflicts.is_empty(), "{:?}", accepted.conflicts);
    assert!(accepted.confirmation.published);
    backend.resume("child1").unwrap();
    assert!(
        run_in_environment(
            &backend,
            "child1",
            &base,
            "test \"$(cat keep)\" = modified && test ! -e remove && \
                 test \"$(cat newfile)\" = new && test -f child1-only"
        ),
        "accepted changes are not visible in child1"
    );
    assert!(
        run_in_environment(
            &backend,
            "root",
            &base,
            "test \"$(cat keep)\" = base-keep && test -f remove && test ! -e newfile"
        ),
        "accept into child1 changed the parent root"
    );

    backend.terminate("grandchild").unwrap();
    backend.discard("grandchild", &[]).unwrap();
    backend.terminate("child1").unwrap();
    backend.discard("child1", &[]).unwrap();
    backend.terminate("root").unwrap();
    backend.cleanup("root").unwrap();
}

#[test]
fn mixed_layout_accept_reports_conflict_and_writes_nothing() {
    let root = btrfs_temp();
    let base = root.path().join("base");
    fs::create_dir_all(&base).unwrap();
    let Some((_state, backend)) = layout_backend(&base, 32) else {
        eprintln!(
            "SKIP mixed_layout_accept_reports_conflict_and_writes_nothing: no writable mount separate from workspace"
        );
        return;
    };
    backend.establish("root").unwrap();
    write_upper(&backend, "root", &base.join("keep"), "base-keep");
    for index in 0..40 {
        write_upper(
            &backend,
            "root",
            &base.join(format!("filler-{index}")),
            "filler",
        );
    }
    backend.fork("root", "child1", &[]).unwrap();
    write_upper(&backend, "child1", &base.join("child1-only"), "child1-only");
    backend.fork("child1", "grandchild", &[]).unwrap();
    write_upper(
        &backend,
        "grandchild",
        &base.join("keep"),
        "grandchild-keep",
    );
    write_upper(&backend, "grandchild", &base.join("newfile"), "new");
    write_upper(&backend, "child1", &base.join("keep"), "target-keep");

    let grandchild_base = FilesystemBackend::base_snapshot_id_for_child("grandchild");
    backend.suspend("child1").unwrap();
    let accepted = backend
        .accept(&grandchild_base, "grandchild", "child1", &[], None, None)
        .unwrap();
    assert!(
        accepted
            .conflicts
            .iter()
            .any(|conflict| conflict.path == base.join("keep").display().to_string()),
        "accept did not report the conflicting path: {:?}",
        accepted.conflicts
    );
    assert!(
        !accepted.confirmation.published,
        "conflicting accept must not publish"
    );
    assert_eq!(
        fs::read_to_string(upper_at(&backend, "child1", &base.join("keep"))).unwrap(),
        "target-keep",
        "conflicting accept overwrote the target"
    );
    assert!(
        !upper_at(&backend, "child1", &base.join("newfile")).exists(),
        "conflicting accept applied an unrelated change"
    );
    backend.resume("child1").unwrap();
    backend.terminate("grandchild").unwrap();
    backend.discard("grandchild", &[]).unwrap();
    backend.terminate("child1").unwrap();
    backend.discard("child1", &[]).unwrap();
    backend.terminate("root").unwrap();
    backend.cleanup("root").unwrap();
}

#[test]
fn fork_refuses_chain_deeper_than_max_lower_depth_before_creating_anything() {
    let root = btrfs_temp();
    let base = root.path().join("base");
    fs::create_dir_all(&base).unwrap();
    let Some((_state, backend)) = layout_backend(&base, 0) else {
        eprintln!(
            "SKIP fork_refuses_chain_deeper_than_max_lower_depth_before_creating_anything: no writable mount separate from workspace"
        );
        return;
    };
    backend.establish("root").unwrap();
    write_upper(&backend, "root", &base.join("gen-0"), "x");
    // The guard uses recorded depth, so populate metadata without creating forks.
    let target = mount_for(&backend, "root", &base).target;
    {
        let mut state = backend.lock().unwrap();
        let environment = state.environments.get_mut("root").unwrap();
        let mount = environment
            .mounts
            .get_mut(&target.display().to_string())
            .unwrap();
        for index in 0..MAX_LOWER_DEPTH {
            let layer = backend.layers_root().join(format!("fake-layer-{index}"));
            fs::create_dir_all(mount.layer_upper(&layer)).unwrap();
            mount.lower.push(layer);
        }
    }
    let parent_depth = mount_for(&backend, "root", &base).lower.len();
    assert_eq!(parent_depth, MAX_LOWER_DEPTH);

    let error = backend
        .fork("root", "chain-overflow", &[])
        .expect_err("fork beyond MAX_LOWER_DEPTH must fail");
    assert!(
        error.message.contains("lower"),
        "error must explain the lower chain: {}",
        error.message
    );
    assert!(
        error.message.contains(&MAX_LOWER_DEPTH.to_string()),
        "error must state the limit {}: {}",
        MAX_LOWER_DEPTH,
        error.message
    );
    assert!(
        error.message.contains(&(MAX_LOWER_DEPTH + 1).to_string()),
        "error must state the resulting depth {}: {}",
        MAX_LOWER_DEPTH + 1,
        error.message
    );
    assert!(backend.state("chain-overflow").is_none());
    assert!(
        !backend.paths("chain-overflow").exists(),
        "failed fork left a child environment root"
    );
    assert!(
        !backend.environment_metadata_path("chain-overflow").exists(),
        "failed fork left child environment metadata"
    );
    assert!(
        !backend
            .layer_path_for(&FilesystemBackend::base_snapshot_id_for_child(
                "chain-overflow"
            ))
            .exists(),
        "failed fork left a layer directory"
    );
    for index in 0..MAX_LOWER_DEPTH {
        let _ = remove_directory_force(&backend.layers_root().join(format!("fake-layer-{index}")));
    }
    backend.terminate("root").unwrap();
    backend.cleanup("root").unwrap();
}

#[test]
fn empty_upper_fork_base_is_treated_as_empty_during_diff_and_accept() {
    let root = btrfs_temp();
    let base = root.path().join("base");
    fs::create_dir_all(&base).unwrap();
    let backend = new_overlay_backend(root.path());
    backend.establish("root").unwrap();
    backend.fork("root", "child", &[]).unwrap();
    let late = base.join("late.txt");
    write_upper(&backend, "child", &late, "late");

    let base_snapshot = FilesystemBackend::base_snapshot_id_for_child("child");
    let diff = backend.diff(&base_snapshot, "child", &[]).unwrap();
    let document: crate::runtime::backends::filesystem::DiffDocument =
        serde_json::from_slice(&fs::read(diff.diff_ref.unwrap()).unwrap()).unwrap();
    assert!(
        document
            .changes
            .iter()
            .any(|change| change.path == late.display().to_string()),
        "diff missed a write on a mount whose base upper was empty"
    );

    backend.suspend("root").unwrap();
    backend
        .accept(&base_snapshot, "child", "root", &[], None, None)
        .unwrap();
    assert_eq!(
        fs::read_to_string(upper_at(&backend, "root", &late)).unwrap(),
        "late"
    );
    backend.resume("root").unwrap();
    backend.terminate("child").unwrap();
    backend.discard("child", &[]).unwrap();
    backend.terminate("root").unwrap();
    backend.cleanup("root").unwrap();
}
