//! Firecracker Backend 中不依赖真实特权的逻辑测试。

use super::*;
use super::backend_impl::{MAX_ENVIRONMENT_ID_BYTES, validate_environment_id};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

struct FakeApi {
    fail_snapshot: bool,
    fail_pause: bool,
    fail_resume: bool,
    requests: Mutex<Vec<(String, String)>>,
}

impl FakeApi {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            fail_snapshot: false,
            fail_pause: false,
            fail_resume: false,
            requests: Mutex::new(Vec::new()),
        })
    }

    fn failing(snapshot: bool, pause: bool, resume: bool) -> Arc<Self> {
        Arc::new(Self {
            fail_snapshot: snapshot,
            fail_pause: pause,
            fail_resume: resume,
            requests: Mutex::new(Vec::new()),
        })
    }
}

impl FirecrackerApi for FakeApi {
    fn request(
        &self,
        method: &str,
        _socket: &Path,
        path: &str,
        body: &Value,
    ) -> Result<(), BackendError> {
        self.requests
            .lock()
            .unwrap()
            .push((method.to_string(), path.to_string()));
        if self.fail_snapshot && path == "/snapshot/create" {
            return Err(BackendError::new("injected snapshot failure"));
        }
        let state = body.get("state").and_then(Value::as_str);
        if self.fail_pause && path == "/vm" && state == Some("Paused") {
            return Err(BackendError::new("injected pause failure"));
        }
        if self.fail_resume && path == "/vm" && state == Some("Resumed") {
            return Err(BackendError::new("injected resume failure"));
        }
        Ok(())
    }
}

fn config(root: &Path) -> FirecrackerConfig {
    let assets = root.join("assets");
    fs::create_dir_all(&assets).unwrap();
    for name in ["firecracker", "kernel", "rootfs.ext4", "guest-access"] {
        let path = assets.join(name);
        if !path.exists() {
            fs::write(&path, b"stub").unwrap();
        }
    }
    FirecrackerConfig {
        control_socket: PathBuf::new(),
        adapter_instance: String::new(),
        firecracker_path: assets.join("firecracker"),
        kernel_path: assets.join("kernel"),
        rootfs_path: assets.join("rootfs.ext4"),
        guest_access_path: assets.join("guest-access"),
        state_directory: root.join("state"),
    }
}

fn backend(root: &Path, api: Arc<dyn FirecrackerApi>) -> FirecrackerBackend {
    FirecrackerBackend::with_api(config(root), api).unwrap()
}

fn test_network() -> EnvironmentNetwork {
    EnvironmentNetwork {
        namespace: "fc-test".into(),
        veth_host: "vh-test".into(),
        veth_subnet: "10.213.255.252/30".into(),
        upstream: "lo".into(),
        teardown_enabled: false,
    }
}

fn insert_environment(
    backend: &FirecrackerBackend,
    id: &str,
    directory: PathBuf,
    state: EnvironmentState,
) {
    let environment = Environment {
        state,
        child: None,
        directory,
        api_socket: PathBuf::from(format!("/tmp/{id}.sock")),
        vsock_socket: PathBuf::from(format!("/tmp/{id}.vsock")),
        network: test_network(),
    };
    backend
        .environments
        .lock()
        .unwrap()
        .insert(id.to_string(), Arc::new(Mutex::new(environment)));
}

#[test]
fn capabilities_declare_independent_branch_environments() {
    let root = tempfile::tempdir().unwrap();
    let capabilities = backend(root.path(), FakeApi::new()).capabilities();
    assert!(capabilities.branch_environment);
    assert!(!capabilities.shared_environment);
    assert!(capabilities.filesystem_fast_delete.is_none());
}

#[test]
fn config_without_adapter_level_fields_deserializes() {
    let root = tempfile::tempdir().unwrap();
    let config = config(root.path());
    let value = json!({
        "firecracker_path": config.firecracker_path,
        "kernel_path": config.kernel_path,
        "rootfs_path": config.rootfs_path,
        "guest_access_path": config.guest_access_path,
        "state_directory": config.state_directory,
    });
    let decoded: FirecrackerConfig = serde_json::from_value(value).unwrap();
    assert!(decoded.control_socket.as_os_str().is_empty());
    assert!(decoded.adapter_instance.is_empty());
}

#[test]
fn environment_ids_cannot_escape_the_state_directory() {
    for id in ["", ".", "..", "../victim", "a/b", "a\\b", "a\0b"] {
        assert!(validate_environment_id(id).is_err(), "accepted {id:?}");
    }
    assert!(validate_environment_id(&"a".repeat(MAX_ENVIRONMENT_ID_BYTES + 1)).is_err());
    assert!(validate_environment_id("candidate-1").is_ok());

    let root = tempfile::tempdir().unwrap();
    let backend = backend(root.path(), FakeApi::new());
    let error = backend.establish("../victim").unwrap_err();
    assert!(error.message.contains("invalid path character"), "{error}");
    assert!(!root.path().join("victim").exists());
    assert!(!root.path().join("state").join("victim").exists());
}

#[test]
fn establish_rejects_too_long_socket_paths_before_copying_rootfs() {
    let root = tempfile::tempdir().unwrap();
    let mut config = config(root.path());
    config.state_directory = root.path().join("s".repeat(120));
    let backend = FirecrackerBackend::with_api(config, FakeApi::new()).unwrap();
    let error = backend.establish("env").unwrap_err();
    assert!(error.message.contains("socket path is too long"), "{error}");
    assert!(!backend.config.state_directory.join("env").exists());
    assert!(backend.environments.lock().unwrap().is_empty());
}

#[test]
fn stale_environment_directory_is_removed_before_establish() {
    let root = tempfile::tempdir().unwrap();
    let backend = backend(root.path(), FakeApi::new());
    let directory = backend.config.state_directory.join("stale");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("leftover"), b"x").unwrap();
    // establish 在创建环境前调用同一段残留处理逻辑；这里直接验证该逻辑，
    // 避免单元测试触及真实的网络和 sudo 权限。
    backend.remove_stale_directory(&directory, "stale").unwrap();
    assert!(!directory.exists(), "stale directory was not removed");
}

#[test]
fn live_environment_directory_is_not_removed() {
    let root = tempfile::tempdir().unwrap();
    let backend = backend(root.path(), FakeApi::new());
    let directory = backend.config.state_directory.join("live");
    fs::create_dir_all(&directory).unwrap();
    let script = root.path().join("firecracker");
    fs::write(&script, "#!/bin/sh\nsleep 30\n").unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    let api_socket = directory.join("api.sock");
    let mut child = Command::new(&script)
        .arg("--id")
        .arg("live")
        .arg("--api-sock")
        .arg(&api_socket)
        .spawn()
        .unwrap();
    fs::write(directory.join("pid"), child.id().to_string()).unwrap();
    std::thread::sleep(Duration::from_millis(50));

    let error = backend.remove_stale_directory(&directory, "live").unwrap_err();
    assert!(error.message.contains("live Firecracker process"), "{error}");
    assert!(directory.exists());
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn resume_failure_marks_parent_suspended_and_returns_error() {
    let root = tempfile::tempdir().unwrap();
    let backend = backend(root.path(), FakeApi::failing(false, false, true));
    let parent_directory = backend.config.state_directory.join("parent");
    fs::create_dir_all(&parent_directory).unwrap();
    fs::write(parent_directory.join("rootfs.ext4"), b"rootfs").unwrap();
    insert_environment(
        &backend,
        "parent",
        parent_directory,
        EnvironmentState::Available,
    );

    let error = backend.branch("parent", "child").unwrap_err();
    assert!(error.message.contains("resume"), "{error}");
    let state = backend
        .environments
        .lock()
        .unwrap()
        .get("parent")
        .unwrap()
        .lock()
        .unwrap()
        .state;
    assert_eq!(state, EnvironmentState::Suspended);
    assert!(!backend.config.state_directory.join("child").exists());
}

#[test]
fn snapshot_and_resume_failures_are_both_reported() {
    let root = tempfile::tempdir().unwrap();
    let backend = backend(root.path(), FakeApi::failing(true, false, true));
    let parent_directory = backend.config.state_directory.join("parent");
    fs::create_dir_all(&parent_directory).unwrap();
    fs::write(parent_directory.join("rootfs.ext4"), b"rootfs").unwrap();
    insert_environment(
        &backend,
        "parent",
        parent_directory,
        EnvironmentState::Available,
    );

    let error = backend.branch("parent", "child").unwrap_err();
    assert!(error.message.contains("snapshot failed"), "{error}");
    assert!(error.message.contains("resume"), "{error}");
    let state = backend
        .environments
        .lock()
        .unwrap()
        .get("parent")
        .unwrap()
        .lock()
        .unwrap()
        .state;
    assert_eq!(state, EnvironmentState::Suspended);
}

#[test]
fn cleanup_removes_environment_only_after_directory_removal() {
    let root = tempfile::tempdir().unwrap();
    let backend = backend(root.path(), FakeApi::new());
    let directory = backend.config.state_directory.join("gone");
    fs::create_dir_all(&directory).unwrap();
    insert_environment(&backend, "gone", directory.clone(), EnvironmentState::Terminated);

    backend.cleanup("gone").unwrap();
    assert!(!directory.exists());
    assert!(backend.environments.lock().unwrap().is_empty());
}

#[test]
fn cleanup_treats_missing_directory_as_success() {
    let root = tempfile::tempdir().unwrap();
    let backend = backend(root.path(), FakeApi::new());
    insert_environment(
        &backend,
        "missing",
        backend.config.state_directory.join("missing"),
        EnvironmentState::Terminated,
    );

    backend.cleanup("missing").unwrap();
    assert!(backend.environments.lock().unwrap().is_empty());
}

#[test]
fn cleanup_failure_keeps_environment_for_retry() {
    let root = tempfile::tempdir().unwrap();
    let backend = backend(root.path(), FakeApi::new());
    let directory = backend.config.state_directory.join("locked");
    fs::create_dir_all(&directory).unwrap();
    // A non-directory at the target path makes remove_dir_all fail.
    let file = directory.join("not-a-directory");
    fs::write(&file, b"x").unwrap();
    // Make the state directory itself unremovable by removing the tree first
    // and replacing it with a file is not what cleanup sees; instead use a
    // path under a read-only directory is user-dependent. Exercise the retry
    // state through a missing child plus a directory that reappears as file.
    fs::remove_dir_all(&directory).unwrap();
    fs::write(&directory, b"file").unwrap();
    insert_environment(
        &backend,
        "locked",
        directory.clone(),
        EnvironmentState::Terminated,
    );

    let error = backend.cleanup("locked").unwrap_err();
    assert!(error.message.contains("failed to remove"), "{error}");
    let state = backend
        .environments
        .lock()
        .unwrap()
        .get("locked")
        .unwrap()
        .lock()
        .unwrap()
        .state;
    assert_eq!(state, EnvironmentState::Terminated);
    assert!(backend.environments.lock().unwrap().contains_key("locked"));
}

#[test]
fn network_registry_releases_on_cleanup() {
    let root = tempfile::tempdir().unwrap();
    let backend = backend(root.path(), FakeApi::new());
    backend.networks.lock().unwrap().allocate("env").unwrap();
    let directory = backend.config.state_directory.join("env");
    fs::create_dir_all(&directory).unwrap();
    insert_environment(&backend, "env", directory, EnvironmentState::Terminated);
    backend.cleanup("env").unwrap();
    assert!(backend.networks.lock().unwrap().allocation("env").is_none());
}
