use ascout::control_plane::policy::{Decision, ModelProviderProcess, PolicyEngine};
use serde_json::json;
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

#[test]
fn compiled_policy_fake_provider_schema_and_revocation() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::copy(
        "assets/policy/policy/policy.yaml",
        directory.path().join("policy.yaml"),
    )
    .unwrap();
    let source = std::fs::read_to_string("assets/policy/policy/decisions.py").unwrap().replace(
        "if (data[\"context\"].get(\"capabilities\") or {}).get(\"isolated\"):\n        return {\"mode\": \"allow\"}\n    return {\"mode\": \"block\"}",
        "return {\"mode\":capabilities.model_complete({\"provider\":\"fake\",\"model\":\"small\",\"prompt\":\"allow\",\"max_tokens\":4})[\"text\"]}");
    std::fs::write(directory.path().join("decisions.py"), source).unwrap();
    let engine = PolicyEngine::new(
        Path::new("/usr/bin/python3").into(),
        Path::new("/usr/bin/bwrap").into(),
        Duration::from_secs(1),
    )
    .unwrap();
    let policy = engine
        .compile(&directory.path().join("policy.yaml"))
        .unwrap();
    let data = json!({"effect_descriptor":{},"context":{}});
    let models = vec!["fake/small".into()];
    let provider = ModelProviderProcess {
        executable: "node".into(),
        arguments: vec!["tests/fixtures/policy-provider-service.mjs".into()],
        environment: Default::default(),
    };
    for _ in 0..2 {
        assert_eq!(
            engine
                .evaluate_with_provider(&policy, Decision::Effect, &data, &models, 1, &provider)
                .unwrap(),
            json!({"mode":"allow"})
        );
    }
    assert!(engine.evaluate(&policy, Decision::Effect, &data).is_err());
    assert!(
        engine
            .evaluate_with_provider(&policy, Decision::Effect, &data, &models, 0, &provider)
            .is_err()
    );
    assert!(
        engine
            .evaluate_with_capabilities(
                &policy,
                Decision::Effect,
                &data,
                &models,
                1,
                &mut |_, _| Ok(json!({"text":"invalid"}))
            )
            .is_err()
    );
    let blocked = ModelProviderProcess {
        executable: "/usr/bin/python3".into(),
        arguments: vec!["-c".into(), "import time;time.sleep(60)".into()],
        environment: Default::default(),
    };
    let cancelled = AtomicBool::new(false);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            std::thread::sleep(Duration::from_millis(100));
            cancelled.store(true, Ordering::Release);
        });
        let error = engine
            .evaluate_with_provider_cancellable(
                &policy,
                Decision::Effect,
                &data,
                &models,
                1,
                &blocked,
                &cancelled,
                None,
            )
            .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
    });
    assert_eq!(
        engine
            .evaluate_with_provider(&policy, Decision::Effect, &data, &models, 1, &blocked)
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::TimedOut
    );
    assert_eq!(
        engine
            .evaluate_with_provider(&policy, Decision::Effect, &data, &models, 1, &provider)
            .unwrap(),
        json!({"mode":"allow"})
    );
}

#[test]
fn predict_distinguishes_abstention_from_an_empty_batch() {
    let engine = PolicyEngine::default();
    let dir = tempfile::tempdir().unwrap();
    let definition = dir.path().join("policy.yaml");
    let code = dir.path().join("decisions.py");
    let source = include_str!("../assets/policy/policy/decisions.py");
    std::fs::write(&definition, include_str!("../assets/policy/policy/policy.yaml")).unwrap();
    let start = source.find("def predict(").unwrap();
    let end = start + source[start + 1..].find("\ndef ").unwrap() + 1;
    let input = json!({"chance_context": {}, "process_context": {}, "assumption_set": [], "capability_context": {}});
    for (expression, valid) in [
        (r#"{"abstain": True, "reason": "no usable evidence"}"#, true),
        (r#"{"generation": "batch", "proposals": []}"#, false),
    ] {
        std::fs::write(&code, format!("{}def predict(data, *, policy_config, capabilities):\n    return {expression}\n\n{}",
            &source[..start], &source[end..])).unwrap();
        let policy = engine.compile(&definition).unwrap();
        let result = engine.evaluate(&policy, Decision::Predict, &input);
        assert_eq!(result.is_ok(), valid, "{result:?}");
    }
}
