use super::super::*;

#[test]
fn establish_does_not_copy_mount_content() {
    let root = tempfile::Builder::new()
        .prefix("materialize-")
        .tempdir_in("/tmp/opencode")
        .unwrap();
    let workspace = root.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    fs::write(workspace.join("unrelated"), "must not be copied").unwrap();
    let mut config = FilesystemConfig::new(root.path().join("state"));
    config.force_overlay = true;
    config.isolated_paths = Some(vec![workspace.clone()]);
    let backend = FilesystemBackend::new(config).unwrap();
    backend.establish("authority").unwrap();
    let state = backend.lock().unwrap();
    assert!(
        state.environments["authority"]
            .mounts
            .values()
            .all(|m| m.saved_base.is_none()),
        "establish copied the execution lower"
    );
    assert!(
        !root.path().join("state/materialization-bases").exists(),
        "establish archived the entire mount"
    );
}

#[test]
fn materialize_final_view_permissions_types_and_retry() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let root = tempfile::Builder::new()
        .prefix("materialize-")
        .tempdir_in("/tmp/opencode")
        .unwrap();
    let workspace = root.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    fs::write(workspace.join("file"), "before").unwrap();
    fs::write(workspace.join("untouched"), "keep").unwrap();
    fs::create_dir(workspace.join("deleted")).unwrap();
    fs::write(workspace.join("deleted/child"), "delete").unwrap();
    fs::create_dir(workspace.join("directory")).unwrap();
    fs::write(workspace.join("directory/kept"), "keep").unwrap();
    let original = fs::metadata(workspace.join("file")).unwrap();
    let mut config = FilesystemConfig::new(root.path().join("state"));
    config.force_overlay = true;
    config.isolated_paths = Some(vec![workspace.clone()]);
    let backend = FilesystemBackend::new(config).unwrap();
    backend.establish("authority").unwrap();
    let script = format!(
        "printf after > '{0}/file'; chmod 600 '{0}/file'; chmod 700 '{0}/directory'; rm -r '{0}/deleted'; mkdir '{0}/new'; echo nested > '{0}/new/child'; ln -s file '{0}/link'",
        workspace.display()
    );
    let output = backend
        .operate(
            "authority",
            &["/bin/sh".into(), "-ec".into(), script],
            &CancelHandle::default(),
        )
        .unwrap();
    assert_eq!(output.exit_code, 0, "{}", output.stderr);
    // A hostile middle symlink must be replaced, not followed into outside.
    let outside = root.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("child"), "safe").unwrap();
    symlink(&outside, workspace.join("new")).unwrap();
    backend.materialize_authority("authority").unwrap();
    assert_eq!(fs::read_to_string(workspace.join("file")).unwrap(), "after");
    assert_eq!(
        fs::metadata(workspace.join("file"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(workspace.join("file")).unwrap().mtime_nsec(),
        original.mtime_nsec()
    );
    assert_eq!(fs::read_to_string(outside.join("child")).unwrap(), "safe");
    assert_eq!(
        fs::read_to_string(workspace.join("new/child")).unwrap(),
        "nested\n"
    );
    assert_eq!(
        fs::read_to_string(workspace.join("directory/kept")).unwrap(),
        "keep"
    );
    assert!(!workspace.join("deleted").exists());
    assert_eq!(
        fs::read_link(workspace.join("link")).unwrap(),
        PathBuf::from("file")
    );
    assert_eq!(
        fs::read_to_string(workspace.join("untouched")).unwrap(),
        "keep"
    );
}

#[test]
fn materialize_export_overlapping_subtree_keeps_each_file() {
    let root = tempfile::Builder::new()
        .prefix("materialize-")
        .tempdir_in("/tmp/opencode")
        .unwrap();
    let view = root.path().join("view");
    let package = root.path().join("package");
    fs::create_dir_all(view.join("dir")).unwrap();
    fs::create_dir(&package).unwrap();
    fs::write(view.join("dir/a"), "a").unwrap();
    fs::write(view.join("dir/b"), "b").unwrap();
    fs::write(view.join("last"), "last").unwrap();
    fs::write(package.join("selection.json"), r#"[{"path":"dir","subtree":true},{"path":"dir/a","subtree":false},{"path":"last","subtree":false}]"#).unwrap();
    run_helper("export", &view, &package).unwrap();
    let nodes: serde_json::Value =
        serde_json::from_slice(&fs::read(package.join("nodes.json")).unwrap()).unwrap();
    for (path, expected) in [("dir/a", "a"), ("dir/b", "b"), ("last", "last")] {
        assert_eq!(
            fs::read_to_string(
                package
                    .join("data")
                    .join(nodes[path]["data"].as_str().unwrap())
            )
            .unwrap(),
            expected
        );
    }
}

fn run_helper(operation: &str, root: &Path, package: &Path) -> Result<(), BackendError> {
    let mut command = Command::new("python3");
    command
        .args(["-c", include_str!("../materialize.py"), operation])
        .arg(root)
        .arg(package);
    let output = command.output().map_err(error)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(BackendError::new(String::from_utf8_lossy(&output.stderr)))
    }
}

#[test]
fn materialize_plan_failure_retains_receipts_and_reload_uses_new_root() {
    let root = tempfile::Builder::new()
        .prefix("materialize-")
        .tempdir_in("/tmp/opencode")
        .unwrap();
    let workspace = root.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    let mut config = FilesystemConfig::new(root.path().join("state"));
    config.force_overlay = true;
    config.isolated_paths = Some(vec![workspace.clone()]);
    let backend = FilesystemBackend::new(config.clone()).unwrap();
    backend.establish("authority").unwrap();
    backend
        .operate(
            "authority",
            &[
                "sh".into(),
                "-c".into(),
                format!("echo first > {}/a", workspace.display()),
            ],
            &CancelHandle::default(),
        )
        .unwrap();
    let plans = config.state_directory.join("tool-plans");
    let retained = config.state_directory.join("retained-plans");
    fs::rename(&plans, &retained).unwrap();
    fs::write(&plans, "injected plan failure").unwrap();
    assert!(backend.materialize_authority("authority").is_err());
    fs::write(workspace.join("a"), "host after publication").unwrap();
    fs::remove_file(&plans).unwrap();
    fs::rename(&retained, &plans).unwrap();
    backend.materialize_authority("authority").unwrap();
    assert_eq!(
        fs::read_to_string(workspace.join("a")).unwrap(),
        "host after publication"
    );
    let expected = backend.lock().unwrap().environments["authority"]
        .root
        .clone();
    drop(backend);
    let restored = FilesystemBackend::new(config).unwrap();
    assert_eq!(
        restored.lock().unwrap().environments["authority"].root,
        expected
    );
    assert!(expected.is_dir());
}

#[test]
fn materialize_partial_retry_skips_completed_opaque_directory() {
    use std::os::unix::fs::symlink;
    let root = tempfile::Builder::new()
        .prefix("materialize-")
        .tempdir_in("/tmp/opencode")
        .unwrap();
    let view = root.path().join("view");
    let host = root.path().join("host");
    let package = root.path().join("package");
    fs::create_dir_all(view.join("a")).unwrap();
    fs::create_dir_all(host.join("a")).unwrap();
    fs::create_dir(&package).unwrap();
    fs::write(view.join("a/file"), "new").unwrap();
    fs::write(host.join("a/old"), "old").unwrap();
    symlink("new", view.join("z")).unwrap();
    symlink("old", host.join("z")).unwrap();
    fs::write(
        package.join("selection.json"),
        r#"[{"path":"a","subtree":true},{"path":"z","subtree":false}]"#,
    )
    .unwrap();
    run_helper("export", &view, &package).unwrap();
    assert!(run_helper("publish", &host, &package).is_err());
    assert!(!host.join("a/old").exists());
    fs::write(host.join("a/after-success"), "keep").unwrap();
    fs::remove_file(host.join("z")).unwrap();
    run_helper("publish", &host, &package).unwrap();
    assert_eq!(fs::read_to_string(host.join("a/file")).unwrap(), "new");
    assert_eq!(
        fs::read_to_string(host.join("a/after-success")).unwrap(),
        "keep"
    );
    assert_eq!(fs::read_link(host.join("z")).unwrap(), PathBuf::from("new"));
}

#[test]
fn materialize_changes_unreadable_directory_mode_and_retries_without_following_symlink() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let root = tempfile::Builder::new()
        .prefix("materialize-permission-")
        .tempdir_in("/tmp/opencode")
        .unwrap();
    let host = root.path().join("host");
    let package = root.path().join("package");
    let outside = root.path().join("outside");
    for path in [&host, &package, &outside] {
        fs::create_dir(path).unwrap();
    }
    fs::create_dir(host.join("restricted")).unwrap();
    fs::set_permissions(host.join("restricted"), fs::Permissions::from_mode(0o300)).unwrap();
    fs::create_dir(outside.join("restricted")).unwrap();
    fs::set_permissions(
        outside.join("restricted"),
        fs::Permissions::from_mode(0o300),
    )
    .unwrap();
    symlink(&outside, host.join("zlink")).unwrap();
    fs::write(package.join("nodes.json"), r#"{"restricted":{"kind":"directory","mode":493,"subtree":false},"zlink":{"kind":"directory","mode":493,"subtree":false}}"#).unwrap();
    fs::write(package.join("root.json"), r#"{"mode":493}"#).unwrap();
    fs::write(
        package.join("receipts.json"),
        r#"{"paths":["zlink"],"modes":[],"published":false}"#,
    )
    .unwrap();
    let output = Command::new("python3")
        .args(["-c", include_str!("../materialize.py"), "publish"])
        .arg(&host)
        .arg(&package)
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "symlink must remain a rejected directory boundary"
    );
    assert_eq!(
        fs::metadata(host.join("restricted"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o755,
        "failure occurs after restricted mode receipt"
    );
    let receipts: serde_json::Value =
        serde_json::from_slice(&fs::read(package.join("receipts.json")).unwrap()).unwrap();
    assert!(
        receipts["modes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| path == "restricted"),
        "completed directory mode is receipted"
    );
    fs::remove_file(host.join("zlink")).unwrap();
    fs::remove_file(package.join("nodes.json")).unwrap();
    fs::write(
        package.join("nodes.json"),
        r#"{"restricted":{"kind":"directory","mode":493,"subtree":false}}"#,
    )
    .unwrap();
    let retry = Command::new("python3")
        .args(["-c", include_str!("../materialize.py"), "publish"])
        .arg(&host)
        .arg(&package)
        .output()
        .unwrap();
    assert!(
        retry.status.success(),
        "{}",
        String::from_utf8_lossy(&retry.stderr)
    );
    assert_eq!(
        fs::metadata(host.join("restricted"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
    assert_eq!(
        fs::metadata(outside.join("restricted"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o300
    );
}

#[test]
fn materialize_writes_through_unreadable_parent_without_changing_its_mode_on_failure() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::Builder::new()
        .prefix("materialize-permission-")
        .tempdir_in("/tmp/opencode")
        .unwrap();
    let host = root.path().join("host");
    let package = root.path().join("package");
    fs::create_dir_all(host.join("restricted")).unwrap();
    fs::create_dir_all(package.join("data")).unwrap();
    fs::write(package.join("data/0"), "branch data").unwrap();
    fs::set_permissions(&host, fs::Permissions::from_mode(0o300)).unwrap();
    fs::set_permissions(host.join("restricted"), fs::Permissions::from_mode(0o300)).unwrap();
    fs::write(package.join("nodes.json"), r#"{"restricted":{"kind":"directory","mode":192,"subtree":false},"restricted/file":{"kind":"file","mode":384,"data":"0"},"restricted/z":{"kind":"file","mode":384,"data":"missing"}}"#).unwrap();
    fs::write(package.join("root.json"), r#"{"mode":192}"#).unwrap();
    assert!(run_helper("publish", &host, &package).is_err());
    assert_eq!(
        fs::read_to_string(host.join("restricted/file")).unwrap(),
        "branch data"
    );
    for path in [&host, &host.join("restricted")] {
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o300
        );
    }
    fs::write(host.join("restricted/file"), "host after receipt").unwrap();
    fs::write(package.join("data/missing"), "retry").unwrap();
    run_helper("publish", &host, &package).unwrap();
    assert_eq!(
        fs::read_to_string(host.join("restricted/file")).unwrap(),
        "host after receipt"
    );
    assert_eq!(
        fs::read_to_string(host.join("restricted/z")).unwrap(),
        "retry"
    );
    assert_eq!(
        fs::metadata(&host).unwrap().permissions().mode() & 0o777,
        0o300
    );
    // Restore only the test fixture so TempDir can remove it.
    fs::set_permissions(&host, fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(host.join("restricted"), fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn materialize_export_does_not_follow_stale_symlink_descendant() {
    use std::os::unix::fs::symlink;
    let root = tempfile::Builder::new()
        .prefix("materialize-")
        .tempdir_in("/tmp/opencode")
        .unwrap();
    let view = root.path().join("view");
    let outside = root.path().join("outside");
    let package = root.path().join("package");
    for path in [&view, &outside, &package] {
        fs::create_dir(path).unwrap();
    }
    fs::write(outside.join("secret"), "outside").unwrap();
    symlink(&outside, view.join("link")).unwrap();
    fs::write(
        package.join("selection.json"),
        r#"[{"path":"link","subtree":false},{"path":"link/secret","subtree":false}]"#,
    )
    .unwrap();
    run_helper("export", &view, &package).unwrap();
    let nodes: serde_json::Value =
        serde_json::from_slice(&fs::read(package.join("nodes.json")).unwrap()).unwrap();
    assert!(nodes.get("link/secret").is_none());
    assert_eq!(fs::read_dir(package.join("data")).unwrap().count(), 0);
}

#[test]
fn materialize_retired_root_failure_keeps_execution_plan_on_old_generation() {
    let root = tempfile::Builder::new()
        .prefix("materialize-retain-")
        .tempdir_in("/tmp/opencode")
        .unwrap();
    let workspace = root.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    let mut config = FilesystemConfig::new(root.path().join("state"));
    config.force_overlay = true;
    config.isolated_paths = Some(vec![workspace]);
    let backend = FilesystemBackend::new(config.clone()).unwrap();
    backend.establish("authority").unwrap();
    let old = backend.lock().unwrap().environments["authority"]
        .root
        .clone();
    let snapshots = config.state_directory.join("metadata/snapshots");
    fs::create_dir_all(&snapshots).unwrap();
    fs::rename(&snapshots, snapshots.with_extension("retained")).unwrap();
    fs::write(&snapshots, "injected snapshot failure").unwrap();
    assert!(backend.materialize_authority("authority").is_err());
    let metadata: EnvironmentMetadata =
        serde_json::from_slice(&fs::read(backend.environment_metadata_path("authority")).unwrap())
            .unwrap();
    assert_eq!(metadata.root.as_ref(), Some(&old));
    let plan: serde_json::Value = serde_json::from_slice(
        &fs::read(config.state_directory.join("tool-plans/authority.json")).unwrap(),
    )
    .unwrap();
    assert!(plan.to_string().contains(&old.display().to_string()));
}

#[test]
fn candidate_continues_after_parent_cleanup_and_publishes_without_old_delta_replay() {
    let root = tempfile::Builder::new()
        .prefix("materialize-candidate-")
        .tempdir_in("/tmp/opencode")
        .unwrap();
    let workspace = root.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    let mut config = FilesystemConfig::new(root.path().join("state"));
    config.force_overlay = true;
    config.fork_flatten_entry_limit = 0;
    config.isolated_paths = Some(vec![workspace.clone()]);
    let backend = FilesystemBackend::new(config).unwrap();
    backend.establish("parent").unwrap();
    let run = |id: &str, script: &str| {
        let result = backend
            .operate(
                id,
                &[
                    "sh".into(),
                    "-ec".into(),
                    format!("cd '{}'; {script}", workspace.display()),
                ],
                &CancelHandle::default(),
            )
            .unwrap();
        assert_eq!(result.exit_code, 0, "{}", result.stderr);
        result.stdout
    };
    run("parent", "echo inherited > a");
    backend.fork("parent", "candidate", &[]).unwrap();
    run("candidate", "echo candidate > b");
    backend.terminate("parent").unwrap();
    backend.cleanup("parent").unwrap();
    assert_eq!(
        run("candidate", "cat a; echo continued >> b"),
        "inherited\n"
    );
    backend.materialize_authority("candidate").unwrap();
    assert_eq!(
        fs::read_to_string(workspace.join("b")).unwrap(),
        "candidate\ncontinued\n"
    );
    fs::write(workspace.join("a"), "host next task").unwrap();
    assert_eq!(run("candidate", "cat a; echo next > c"), "host next task");
    backend.materialize_authority("candidate").unwrap();
    assert_eq!(
        fs::read_to_string(workspace.join("a")).unwrap(),
        "host next task"
    );
}
