
use super::super::mounts::real_writable_mounts;
use super::super::*;
use super::support::*;

fn path_fs_type(path: &Path) -> String {
    Command::new("stat")
        .args(["-f", "-c", "%T"])
        .arg(path)
        .output()
        .ok()
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .unwrap_or_default()
}

fn mount_of(path: &Path) -> String {
    if let Ok(output) = Command::new("findmnt")
        .args(["-n", "-o", "TARGET,FSTYPE", "-T"])
        .arg(path)
        .output()
        && output.status.success()
    {
        let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !value.is_empty() {
            return value;
        }
    }
    real_writable_mounts()
        .into_iter()
        .filter(|target| path.starts_with(target))
        .max_by_key(|target| target.as_os_str().len())
        .map(|target| target.display().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn layout_diagnostic(backend: &FilesystemBackend, workspace: &Path) -> String {
    let state = &backend.config.state_directory;
    let isolated = backend.overlayable_mounts();
    format!(
        "invalid privileged layout:\n\
             - ASCOUT_PRIV_STATE_DIR state directory: {} (mount: {})\n\
             - ASCOUT_PRIV_WORKSPACE_DIR workspace: {} (mount: {})\n\
             - isolated mounts: {:?}\n\
             The state directory must be on btrfs but outside every mount the backend \
             isolates; the workspace must be on a different writable mount (for example \
             /tmp). On a host where every writable btrfs mount is isolated, put the state \
             directory directly on the btrfs filesystem mounted at /.",
        state.display(),
        mount_of(state),
        workspace.display(),
        mount_of(workspace),
        isolated
    )
}

fn btrfs_layout_backend(
    state_base: &Path,
    workspace_base: &Path,
    flatten_entry_limit: usize,
) -> (
    tempfile::TempDir,
    tempfile::TempDir,
    FilesystemBackend,
    PathBuf,
) {
    let state = tempfile::Builder::new()
        .prefix("priv-btrfs-state-")
        .tempdir_in(state_base)
        .expect("create privileged state tempdir");
    let workspace = tempfile::Builder::new()
        .prefix("priv-btrfs-workspace-")
        .tempdir_in(workspace_base)
        .expect("create privileged workspace tempdir");
    let base = workspace.path().join("base");
    fs::create_dir_all(&base).unwrap();
    let mut config = FilesystemConfig::new(state.path().join("state"));
    config.fork_flatten_entry_limit = flatten_entry_limit;
    let backend = FilesystemBackend::new(config).unwrap();
    assert_eq!(
        backend.mechanism(),
        FilesystemMechanism::Btrfs,
        "ASCOUT_PRIV_TESTS=1 requires the btrfs data plane (mechanism={}, fork_cost={})",
        backend.mechanism().as_str(),
        backend.fork_cost().as_str()
    );
    (state, workspace, backend, base)
}

fn assert_layout_is_separate(backend: &FilesystemBackend, workspace: &Path) {
    let state = &backend.config.state_directory;
    let state_mount = mount_of(state);
    let workspace_mount = mount_of(workspace);
    assert_ne!(
        state_mount,
        workspace_mount,
        "state directory and workspace must be on different mounts\n{}",
        layout_diagnostic(backend, workspace)
    );
    let overlaps = backend.state_overlap_mounts();
    assert!(
        overlaps.is_empty(),
        "state directory is inside an isolated mount\n{}",
        layout_diagnostic(backend, workspace)
    );
    backend.establish("root").unwrap_or_else(|error| {
        panic!(
            "default config refused to establish the privileged layout: {error}\n{}",
            layout_diagnostic(backend, workspace)
        )
    });
}

fn run_btrfs_layout_test(test_name: &str, flatten_entry_limit: usize) {
    if !priv_enabled() {
        eprintln!("SKIP {test_name}: ASCOUT_PRIV_TESTS != 1");
        return;
    }
    assert_eq!(
        unsafe { libc::geteuid() },
        0,
        "{test_name}: ASCOUT_PRIV_TESTS=1 requires root for the btrfs data plane"
    );
    let state_base = priv_state_root();
    let fs_type = path_fs_type(&state_base);
    assert_eq!(
        fs_type,
        "btrfs",
        "{test_name}: ASCOUT_PRIV_STATE_DIR={} must be on btrfs, found {fs_type}",
        state_base.display()
    );
    let workspace_base = priv_workspace_root();
    let (state, workspace, backend, base) =
        btrfs_layout_backend(&state_base, &workspace_base, flatten_entry_limit);
    assert_layout_is_separate(&backend, &base);
    eprintln!(
        "{test_name}: mechanism={} fast_delete={} limit={} state={} workspace={}",
        backend.mechanism().as_str(),
        backend.fast_delete(),
        flatten_entry_limit,
        backend.config.state_directory.display(),
        base.display()
    );
    layout_diff_accept_scenario(&backend, &base, flatten_entry_limit);
    drop(backend);
    drop(workspace);
    drop(state);
}

#[test]
fn priv_btrfs_chain_layout() {
    run_btrfs_layout_test("priv_btrfs_chain_layout", 0);
}

#[test]
fn priv_btrfs_flatten_layout() {
    run_btrfs_layout_test("priv_btrfs_flatten_layout", usize::MAX);
}

#[test]
fn priv_btrfs_mixed_generations() {
    if !priv_enabled() {
        eprintln!("SKIP priv_btrfs_mixed_generations: ASCOUT_PRIV_TESTS != 1");
        return;
    }
    assert_eq!(
        unsafe { libc::geteuid() },
        0,
        "priv_btrfs_mixed_generations: ASCOUT_PRIV_TESTS=1 requires root for the btrfs data plane"
    );
    let state_base = priv_state_root();
    let fs_type = path_fs_type(&state_base);
    assert_eq!(
        fs_type,
        "btrfs",
        "priv_btrfs_mixed_generations: ASCOUT_PRIV_STATE_DIR={} must be on btrfs, found {fs_type}",
        state_base.display()
    );
    let workspace_base = priv_workspace_root();
    let (state, workspace, backend, base) = btrfs_layout_backend(&state_base, &workspace_base, 32);
    assert_layout_is_separate(&backend, &base);
    mixed_generations_scenario(&backend, &base);
    drop(backend);
    drop(workspace);
    drop(state);
}
