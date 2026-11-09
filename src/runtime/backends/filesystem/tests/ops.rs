//! fork/mark/diff/accept/discard 端到端测试。

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

        let backend = FilesystemBackend::new(FilesystemConfig::new(root.path().join("state"))).unwrap();
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
    fn composite_runtime_reaches_the_filesystem_data_plane() {
        use crate::runtime::{BackendConnection, BackendPolicy, CompositeRuntime};
        use std::sync::Arc;

        let root = temp();
        let filesystem = Arc::new(new_backend(root.path()));
        let runtime = CompositeRuntime::new(
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
            operation_id: "composite-fork".into(),
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
    fn safe_root_mkdir_preserves_source_mode() {
        use std::os::unix::fs::MetadataExt;
        let safe = SafeRoot::open(std::process::id()).unwrap();
        let token = format!("ascout-mode-{}", std::process::id());
        let relative = format!("tmp/{token}");
        safe.mkdir(&relative, 0o750).unwrap();
        let metadata = fs::metadata(format!("/{relative}")).unwrap();
        assert_eq!(metadata.mode() & 0o777, 0o750);
        fs::remove_dir(format!("/{relative}")).unwrap();
    }

    #[test]
    fn btrfs_snapshot_copy_fork_is_end_to_end() {
        let root = btrfs_temp();
        let backend = FilesystemBackend::new(FilesystemConfig::new(root.path().join("state"))).unwrap();
        if backend.fork_cost() != ForkCost::BtrfsSnapshot || backend.mechanism() != FilesystemMechanism::Overlay {
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
            run_in_environment(&backend, "child", &base, "test -f fork.txt && test ! -e late.txt"),
            "child did not observe the fork-time snapshot"
        );
        assert!(
            run_in_environment(&backend, "child", &base, "rm -f fork.txt && test ! -e fork.txt"),
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
        assert!(error.message.contains("injected clone failure"), "{}", error.message);
        assert!(backend.state("child").is_none());
        assert!(!root
            .path()
            .join("state/layers")
            .join(FilesystemBackend::base_snapshot_id_for_child("child"))
            .exists());
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
        let backend = FilesystemBackend::new(FilesystemConfig::new(root.path().join("state"))).unwrap();
        assert!(!orphan.exists(), "backend start must remove orphan staging");
        drop(backend);
    }

    #[test]
    fn accept_work_does_not_scale_with_unrelated_upper_size() {
        let Some(small) = accept_read_layer_entries(20) else {
            return;
        };
        let Some(large) = accept_read_layer_entries(5000) else {
            return;
        };
        assert!(
            large <= small + 50,
            "accept scanned unrelated upper entries: small={small}, large={large}"
        );
    }

    fn accept_read_layer_entries(unrelated: usize) -> Option<usize> {
        let workspace_root = btrfs_temp();
        let base = workspace_root.path().join("base");
        fs::create_dir_all(&base).unwrap();
        let state = match separate_state_tempdir(&base) {
            Some(value) => value,
            None => {
                eprintln!(
                    "SKIP accept_work_does_not_scale_with_unrelated_upper_size: no writable mount separate from workspace"
                );
                return None;
            }
        };
        let mut config = FilesystemConfig::new(state.path().join("state"));
        config.force_overlay = true;
        let backend = FilesystemBackend::new(config).unwrap();
        backend.establish("root").unwrap();
        let workspace_mount = mount_for(&backend, "root", &base);
        if backend.mount_uses_flattening(&workspace_mount) {
            eprintln!(
                "SKIP accept_work_does_not_scale_with_unrelated_upper_size: workspace mount is flattened"
            );
            return None;
        }
        for index in 0..unrelated {
            write_upper(
                &backend,
                "root",
                &base.join(format!("unrelated-{index}")),
                "x",
            );
        }
        backend.fork("root", "child", &[]).unwrap();
        let snapshot = FilesystemBackend::base_snapshot_id_for_child("child");
        for index in 0..10.min(unrelated) {
            write_upper(
                &backend,
                "child",
                &base.join(format!("unrelated-{index}")),
                "changed",
            );
        }
        backend.suspend("root").unwrap();
        reset_read_layer_entries();
        let accepted = backend
            .accept(&snapshot, "child", "root", &[], None, None)
            .unwrap();
        let entries = read_layer_entries();
        assert!(accepted.confirmation.published);
        backend.resume("root").unwrap();
        backend.terminate("child").unwrap();
        backend.discard("child", &[]).unwrap();
        backend.terminate("root").unwrap();
        backend.cleanup("root").unwrap();
        Some(entries)
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
                error.message.contains("injected replay failure")
                    || error.message.contains("rollback"),
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
    fn opaque_removal_error_aborts_replay() {
        use std::os::unix::fs::PermissionsExt;
        if unsafe { libc::geteuid() } == 0 {
            eprintln!("SKIP opaque_removal_error_aborts_replay: running as root");
            return;
        }
        let root = temp();
        let upper = root.path().join("upper");
        fs::create_dir_all(upper.join("opaque")).unwrap();
        fs::write(upper.join("opaque/old.txt"), b"old").unwrap();
        fs::set_permissions(&upper, fs::Permissions::from_mode(0o500)).unwrap();
        let mut delta = View::new();
        delta.insert(
            "opaque".into(),
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
        let mut journal = ReplayJournal::new(root.path().join("journal")).unwrap();
        let error = replay_delta(
            &ReplayRoot::Upper(&upper),
            &delta,
            &[],
            &[],
            &mut journal,
            &upper,
            None,
        )
        .unwrap_err();
        assert!(error.message.contains("replay"), "{}", error.message);
        fs::set_permissions(&upper, fs::Permissions::from_mode(0o700)).unwrap();
        let _ = journal.rollback();
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
        assert!(error.message.contains("injected replay failure"), "{}", error.message);
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
        let backend = FilesystemBackend::new(FilesystemConfig::new(root.path().join("state"))).unwrap();
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
        write_upper(
            &backend,
            "root",
            &workspace.join("parent.txt"),
            "parent",
        );

        let fork = backend
            .fork("root", "child", &[PathScope::Workspace])
            .unwrap();
        assert!(fork.confirmation.snapshot_created);
        assert!(fork.confirmation.metadata_durable);
        assert!(!fork.confirmation.writers_quiesced);
        let base = FilesystemBackend::base_snapshot_id_for_child("child");
        write_upper(
            &backend,
            "child",
            &workspace.join("child.txt"),
            "child",
        );

        if expect_btrfs {
            let diff = backend.diff(&base, "child", &[PathScope::Workspace]);
            assert!(
                diff.is_err(),
                "btrfs diff must fail explicitly without generation privileges"
            );
            assert!(diff.unwrap_err().message.contains("btrfs incremental diff"));
            let accept = backend.accept(
                &base,
                "child",
                "root",
                &[PathScope::Workspace],
                None,
                None,
            );
            assert!(accept.is_err());
        } else {
            let diff = backend
                .diff(&base, "child", &[PathScope::Workspace])
                .unwrap();
            assert!(diff.confirmation.metadata_durable);
            let document: DiffDocument =
                serde_json::from_slice(&fs::read(diff.diff_ref.unwrap()).unwrap()).unwrap();
            let child_path = workspace.join("child.txt").display().to_string();
            assert!(document
                .changes
                .iter()
                .any(|change| change.path == child_path && change.kind == "change"));

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
            assert!(mark_document
                .changes
                .iter()
                .any(|change| change.path == after_path && change.kind == "change"));
            assert!(mark_document
                .changes
                .iter()
                .all(|change| change.path != child_path));

            backend.suspend("root").unwrap();
            let accept = backend
                .accept(&base, "child", "root", &[PathScope::Workspace], None, None)
                .unwrap();
            assert!(accept.conflicts.is_empty());
            assert!(accept.confirmation.published);
            assert!(!accept.confirmation.target_paused);
            assert!(upper_at(&backend, "root", &workspace.join("child.txt")).exists());
        }

        backend.terminate("child").unwrap();
        let discard = backend
            .discard("child", &[PathScope::Workspace])
            .unwrap();
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

    /// Real bubblewrap end-to-end: a whiteout created by `rm` must not
    /// resurrect the Base file in a forked child, and accept must propagate
    /// the deletion to the target.
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
        assert!(document
            .changes
            .iter()
            .any(|change| change.path == deleted && change.kind == "delete"));

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
        let (target, target_pid, target_ns) = spawn_in_environment(
            &backend,
            "root",
            &base,
            "while [ -e x ]; do sleep 0.05; done; exit 0",
        );
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

        let mut exited = false;
        for _ in 0..200 {
            if let Some(status) = target.try_wait("root").unwrap() {
                assert!(status.success());
                exited = true;
                break;
            }
            thread::sleep(std::time::Duration::from_millis(10));
        }
        if !exited {
            let _ = target.stop("root");
            panic!("target process did not observe the accepted deletion");
        }
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
        if unsafe { libc::access(CString::new(target.as_os_str().as_bytes()).unwrap().as_ptr(), libc::W_OK) }
            != 0
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
            document
                .changes
                .iter()
                .any(|change| change.path == child_file.display().to_string()
                    && change.kind == "change"),
            "diff did not report {}: {:?}",
            child_file.display(),
            document.changes
        );
        assert!(
            document
                .changes
                .iter()
                .any(|change| change.path == lower_file.display().to_string()
                    && change.kind == "delete"),
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

        // mark must also snapshot every mount, not only workspace/tmp.
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
        assert!(child_upper.exists(), "child upper did not contain the write");
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

    /// A still-running parent writes after fork.  Its upper and mount are not
    /// touched by fork, so the child's independent upper copy must not see the
    /// later write.
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
        wait_until(|| parent_upper.join("ready").exists());

        backend
            .fork("root", "child", &[PathScope::Workspace])
            .unwrap();
        wait_until(|| parent_upper.join("after.txt").exists());

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


    fn merged_view_entries(
        backend: &FilesystemBackend,
        environment_id: &str,
        path: &Path,
    ) -> BTreeSet<String> {
        let (layers, upper) = {
            let state = backend.lock().unwrap();
            let environment = state.environments.get(environment_id).unwrap();
            let mount = environment.mount_for_path(path).unwrap();
            (
                mount
                    .lower
                    .iter()
                    .map(|layer| mount.layer_upper(layer))
                    .collect::<Vec<_>>(),
                mount.upper.clone(),
            )
        };
        let mut view = View::new();
        for layer in &layers {
            apply_layer(&mut view, layer).unwrap();
        }
        apply_layer(&mut view, &upper).unwrap();
        view.into_keys().collect()
    }

    fn producer_for(backend: &FilesystemBackend, base: &Path) -> crate::runtime::backends::filesystem::ops::fork::ForkProducer {
        let mount = mount_for(backend, "root", base);
        let producers = backend.fork_producers(std::slice::from_ref(&mount));
        *producers
            .get(&mount.target.display().to_string())
            .expect("non-empty mount has a producer")
    }

    #[test]
    fn adaptive_fork_producer_matches_upper_size_and_child_view() {
        use crate::runtime::backends::filesystem::ops::fork::ForkProducer;

        let root = btrfs_temp();
        let base = root.path().join("base");
        fs::create_dir_all(&base).unwrap();
        let state = match separate_state_tempdir(&base) {
            Some(value) => value,
            None => {
                eprintln!("SKIP adaptive_fork_producer_matches_upper_size_and_child_view: no writable state mount separate from workspace");
                return;
            }
        };
        let backend = FilesystemBackend::new(FilesystemConfig::new(state.path().join("state"))).unwrap();
        if backend.mechanism() != FilesystemMechanism::Overlay
            || backend.fork_cost() != ForkCost::BtrfsSnapshot
        {
            eprintln!("SKIP adaptive_fork_producer_matches_upper_size_and_child_view: btrfs snapshot copies unavailable");
            return;
        }
        backend.establish("root").unwrap();
        for index in 0..10 {
            write_upper(&backend, "root", &base.join(format!("small-{index}")), "small");
        }
        assert_eq!(producer_for(&backend, &base), ForkProducer::Clone);
        let small_parent = merged_view_entries(&backend, "root", &base);
        backend.fork("root", "child-small", &[]).unwrap();
        assert_eq!(
            merged_view_entries(&backend, "child-small", &base),
            small_parent,
            "clone producer changed the child view"
        );

        for index in 10..600 {
            write_upper(&backend, "root", &base.join(format!("large-{index}")), "large");
        }
        assert_eq!(producer_for(&backend, &base), ForkProducer::Snapshot);
        let large_parent = merged_view_entries(&backend, "root", &base);
        backend.fork("root", "child-snapshot", &[]).unwrap();
        let snapshot_view = merged_view_entries(&backend, "child-snapshot", &base);
        assert_eq!(snapshot_view, large_parent, "snapshot producer changed the child view");

        // Force the same parent view through the clone producer.  Empty
        // mounts normally carry `volume_is_subvolume=false`; flipping the live
        // mount exercises the clone path without changing the view.
        {
            let mut state = backend.lock().unwrap();
            for mount in state.environments.get_mut("root").unwrap().mounts.values_mut() {
                mount.volume_is_subvolume = false;
            }
        }
        assert_eq!(producer_for(&backend, &base), ForkProducer::Clone);
        backend.fork("root", "child-clone", &[]).unwrap();
        let clone_view = merged_view_entries(&backend, "child-clone", &base);
        assert_eq!(clone_view, large_parent, "forced clone producer changed the child view");
        assert_eq!(clone_view, snapshot_view, "producers produced different child views");

        backend.terminate("child-small").unwrap();
        backend.discard("child-small", &[]).unwrap();
        backend.terminate("child-snapshot").unwrap();
        backend.discard("child-snapshot", &[]).unwrap();
        backend.terminate("child-clone").unwrap();
        backend.discard("child-clone", &[]).unwrap();
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
        let diff = backend
            .diff(&base_snapshot, "child", &[])
            .unwrap();
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
