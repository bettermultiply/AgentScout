use std::path::Path;

use ascout::policy::{Decision, PolicyEngine};
use serde_json::json;

// Regression: bubblewrap has no --preserve-fds option. Exercise the real
// compiler and sandbox rather than substituting a process or backend.
#[test]
fn policy_compilation_uses_supported_bubblewrap_pipe_setup() {
    let engine = PolicyEngine::default();
    let policy = engine
        .compile(Path::new("assets/policy/policy.yaml"))
        .unwrap();
    assert_eq!(
        engine
            .evaluate(
                &policy,
                Decision::Extent,
                &json!({
                    "process_state": {"steps_completed": 5, "phase": "reasoning"}
                })
            )
            .unwrap(),
        "stop"
    );
}
