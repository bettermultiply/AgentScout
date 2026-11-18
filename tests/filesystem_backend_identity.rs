use std::sync::Arc;
use std::time::Duration;

use ascout::control_plane::gateway::runtime::ControlServer;
use ascout::control_plane::policy::PolicyEngine;
use ascout::runtime::BaseRuntime;
use ascout::runtime::proxy::RuntimeProxy;
use ascout::runtime::backends::filesystem::{FilesystemBackend, FilesystemConfig};

#[test]
fn filesystem_hello_confirms_policy_runtime_backend_identity() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::copy(
        "assets/policy/policy/decisions.py",
        directory.path().join("decisions.py"),
    )
    .unwrap();
    let yaml = std::fs::read_to_string("assets/policy/policy/policy.yaml").unwrap();
    std::fs::write(
        directory.path().join("policy.yaml"),
        format!(
            "{yaml}\nruntime: {}\n",
            directory.path().join("runtime.json").display()
        ),
    )
    .unwrap();
    std::fs::write(
        directory.path().join("runtime.json"),
        r#"{"backend":"filesystem"}"#,
    )
    .unwrap();
    let engine = PolicyEngine::default();
    let policy = engine
        .compile(&directory.path().join("policy.yaml"))
        .unwrap();
    assert_eq!(
        policy.runtime_config_path(),
        Some(directory.path().join("runtime.json").as_path())
    );

    let filesystem = Arc::new(
        FilesystemBackend::new(FilesystemConfig::new(directory.path().join("state"))).unwrap(),
    );
    let runtime = Arc::new(BaseRuntime::new("runtime", filesystem));
    let server = ControlServer::bind("127.0.0.1:0").unwrap();
    let address = server.local_addr().unwrap().to_string();
    let adapter = std::thread::spawn(move || {
        RuntimeProxy {
            adapter_instance: "filesystem-regression".into(),
            runtime,
        }
        .run(&address)
        .unwrap()
    });
    let session = server.accept().unwrap();
    let hello = session.wait_for_hello(Duration::from_secs(5)).unwrap();
    session.shutdown().unwrap();
    adapter.join().unwrap();

    assert_eq!(hello.capabilities.backends, vec!["filesystem"]);
    engine
        .confirm_backends(&policy, &hello.capabilities)
        .unwrap();
    let mut missing = hello.capabilities;
    missing.backends.clear();
    let error = engine.confirm_backends(&policy, &missing).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Policy requires unavailable backend: filesystem")
    );
}
