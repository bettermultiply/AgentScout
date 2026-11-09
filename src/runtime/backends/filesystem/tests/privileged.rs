//! 高权限测试（默认 SKIP）。

use super::super::*;
use super::support::*;

    #[test]
    fn priv_btrfs_fork_diff_accept_discard() {
        if !priv_enabled() {
            eprintln!("SKIP priv_btrfs_fork_diff_accept_discard: ASCOUT_PRIV_TESTS != 1");
            return;
        }
        let state_root = priv_state_root();
        let fs_type = Command::new("stat")
            .args(["-f", "-c", "%T"])
            .arg(&state_root)
            .output()
            .ok()
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
            .unwrap_or_default();
        assert_eq!(fs_type, "btrfs", "ASCOUT_PRIV_TESTS=1 requires a btrfs state directory");
        let root = tempfile::Builder::new()
            .prefix("priv-btrfs-")
            .tempdir_in(state_root)
            .unwrap();
        let backend = FilesystemBackend::new(FilesystemConfig::new(root.path().join("state"))).unwrap();
        assert_eq!(
            backend.mechanism(),
            FilesystemMechanism::Btrfs,
            "privileged environment must select the btrfs data plane"
        );
        assert!(backend.fast_delete(), "privileged btrfs must support strict delete");
        backend.establish("root").unwrap();
        let workspace = root.path().join("base");
        fs::create_dir_all(&workspace).unwrap();
        write_upper(&backend, "root", &workspace.join("parent.txt"), "parent");
        backend.fork("root", "child", &[PathScope::Workspace]).unwrap();
        write_upper(&backend, "child", &workspace.join("child.txt"), "child");
        let base = FilesystemBackend::base_snapshot_id_for_child("child");
        let diff = backend
            .diff(&base, "child", &[PathScope::Workspace])
            .unwrap();
        let document: DiffDocument =
            serde_json::from_slice(&fs::read(diff.diff_ref.unwrap()).unwrap()).unwrap();
        let child_path = workspace.join("child.txt").display().to_string();
        assert!(
            document
                .changes
                .iter()
                .any(|change| change.path == child_path && change.kind == "change"),
            "btrfs diff did not report {}: {:?}",
            workspace.join("child.txt").display(),
            document.changes
        );
        backend.suspend("root").unwrap();
        let accept = backend
            .accept(&base, "child", "root", &[PathScope::Workspace], None, None)
            .unwrap();
        assert!(accept.confirmation.published);
        assert!(upper_at(&backend, "root", &workspace.join("child.txt")).exists());
        backend.resume("root").unwrap();
        backend.terminate("child").unwrap();
        let child_root = backend.paths("child");
        backend.discard("child", &[PathScope::Workspace]).unwrap();
        wait_until(|| !child_root.exists());
        assert!(!child_root.exists(), "priv discard left child data");
        backend.terminate("root").unwrap();
        backend.cleanup("root").unwrap();
    }

    #[test]
    fn priv_direct_root_overlay_is_available_in_initial_userns() {
        if !priv_enabled() {
            eprintln!("SKIP priv_direct_root_overlay_is_available_in_initial_userns: ASCOUT_PRIV_TESTS != 1");
            return;
        }
        assert_eq!(unsafe { libc::geteuid() }, 0, "ASCOUT_PRIV_TESTS=1 requires root for direct lower=/ overlay");
        let root = tempfile::Builder::new()
            .prefix("priv-root-")
            .tempdir_in(priv_state_root())
            .unwrap();
        let upper = root.path().join("upper");
        let work = root.path().join("work");
        fs::create_dir_all(&upper).unwrap();
        fs::create_dir_all(&work).unwrap();
        let status = Command::new("unshare")
            .args(["--mount", "--propagation", "private", "bwrap"])
            .args(["--die-with-parent", "--ro-bind", "/", "/"])
            .args(["--proc", "/proc", "--dev", "/dev"])
            .arg("--overlay-src")
            .arg("/")
            .arg("--overlay")
            .arg(&upper)
            .arg(&work)
            .arg("/")
            .arg("--chdir")
            .arg("/")
            .arg("--")
            .arg("/usr/bin/true")
            .status()
            .unwrap();
        assert!(status.success(), "privileged direct lower=/ overlay failed");
    }
