use super::super::*;
use super::support::{mount_for, run_in_environment};

#[test]
fn workspace_is_mounted_at_its_sandbox_target_and_publishes_back() {
    let root = tempfile::Builder::new()
        .prefix("workspace-target-")
        .tempdir_in("/tmp/opencode")
        .unwrap();
    let workspace = root.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    fs::write(workspace.join("NOTE_FROM_ATTACKER.txt"), "note").unwrap();
    let mut config = FilesystemConfig::new(root.path().join("state"));
    config.force_overlay = true;
    config.allow_overlapping_state_directory = true;
    config.isolated_paths = Some(vec![workspace.clone()]);
    config.isolated_targets = vec![(workspace.clone(), PathBuf::from("/app"))];
    let backend = FilesystemBackend::new(config).unwrap();
    assert!(backend.usable, "bubblewrap overlay probe failed");
    backend.establish("authority").unwrap();

    let mount = mount_for(&backend, "authority", Path::new("/app"));
    assert_eq!(mount.target, PathBuf::from("/app"));
    assert_eq!(mount.base.as_deref(), Some(workspace.as_path()));

    let launched = run_in_environment(
        &backend,
        "authority",
        &workspace,
        "set -e; test \"$(cat /app/NOTE_FROM_ATTACKER.txt)\" = note; test \"$(pwd)\" = /app; mkdir -p /app/recovered; echo restored > /app/recovered/file",
    );
    assert!(
        launched,
        "agent operations must run at the sandbox workspace path"
    );
    assert!(
        mount.upper.join("recovered/file").is_file(),
        "sandbox writes stay in the branch upper"
    );
    assert!(
        !workspace.join("recovered").exists(),
        "sandbox writes never touch the host workspace before materialization"
    );

    backend.materialize_authority("authority").unwrap();
    assert_eq!(
        fs::read_to_string(workspace.join("recovered/file")).unwrap(),
        "restored\n"
    );
    assert!(
        !Path::new("/app").exists(),
        "the sandbox path is never created on the host"
    );
}
