use std::path::Path;

use ascout::control_plane::policy::{CompiledPolicy, Decision, PolicyEngine};
use serde_json::json;

#[test]
fn serial_baseline_compiles_to_a_pure_opportunity_rejection() {
    let compiled = PolicyEngine::default()
        .compile(Path::new("assets/policy/serial-baseline/policy.yaml"))
        .unwrap();
    let no_worker = PolicyEngine::new(
        "/usr/bin/python3".into(),
        "/nonexistent/bwrap".into(),
        std::time::Duration::from_secs(1),
    )
    .unwrap();
    assert!(compiled.pure_decision(Decision::OpportunityPass).is_some());
    assert_eq!(
        no_worker
            .evaluate(
                &compiled,
                Decision::OpportunityPass,
                &json!({"op":{"kind":"bash","has_predictor":true,"produces_result":true,"result_pending":true}}),
            )
            .unwrap(),
        false
    );
}

// Regression: bubblewrap has no --preserve-fds option. Exercise the real
// compiler and sandbox rather than substituting a process or backend.
#[test]
fn policy_compilation_uses_supported_bubblewrap_pipe_setup() {
    let engine = PolicyEngine::default();
    let policy = engine
        .compile(Path::new("assets/policy/policy/policy.yaml"))
        .unwrap();
    assert_eq!(
        engine
            .evaluate(
                &policy,
                Decision::Extent,
                &json!({
                    "process_state": {"steps_completed": 5, "hook_count": 5, "phase": "reasoning"}
                })
            )
            .unwrap(),
        "stop"
    );
}

#[test]
fn policy_model_grant_is_validated_and_covered_by_the_artifact_digest() {
    let missing_tree =
        CompiledPolicy::from_bytes(include_bytes!("fixtures/predictor-fact-policy.json"))
            .unwrap_err();
    assert_eq!(missing_tree.kind(), std::io::ErrorKind::InvalidInput);

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("policy.yaml");
    std::fs::copy(
        "assets/policy/policy/decisions.py",
        dir.path().join("decisions.py"),
    )
    .unwrap();
    let base = include_str!("../assets/policy/policy/policy.yaml");
    let engine = PolicyEngine::default();
    assert!(
        engine
            .compile(Path::new("assets/policy/policy/policy.yaml"))
            .unwrap()
            .policy_model_grant()
            .is_none()
    );
    let grant = json!({"decisions":["predict"], "models":["fake/small"], "max_requests":16, "timeout_ms":300000});
    let write_policy = |grant: &serde_json::Value| {
        std::fs::write(&path, format!("{base}\npolicy_model_grant: {grant}\n")).unwrap();
    };
    write_policy(&grant);
    let compiled = engine.compile(&path).unwrap();
    let mut artifact = serde_json::to_value(&compiled).unwrap();
    assert_eq!(artifact["policy_model_grant"], grant);
    assert!(CompiledPolicy::from_bytes(&serde_json::to_vec(&artifact).unwrap()).is_ok());
    artifact["policy_model_grant"]["max_requests"] = json!(15);
    let err = CompiledPolicy::from_bytes(&serde_json::to_vec(&artifact).unwrap()).unwrap_err();
    assert!(err.to_string().contains("digest mismatch"));
    artifact
        .as_object_mut()
        .unwrap()
        .remove("policy_model_grant");
    assert!(CompiledPolicy::from_bytes(&serde_json::to_vec(&artifact).unwrap()).is_err());

    for (key, value) in [
        ("decisions", json!(["admit.opportunity_pass"])),
        ("decisions", json!([])),
        ("decisions", json!(["predict", "predict"])),
        ("models", json!([])),
        ("models", json!(["fake/ "])),
        ("models", json!(["missing-provider"])),
        ("max_requests", json!(0)),
        ("max_requests", json!(17)),
        ("timeout_ms", json!(99)),
        ("timeout_ms", json!(300001)),
        ("unknown", json!(true)),
    ] {
        let mut invalid = grant.clone();
        invalid[key] = value;
        write_policy(&invalid);
        assert!(
            engine.compile(&path).is_err(),
            "accepted invalid grant: {invalid}"
        );
    }
}

#[test]
fn pure_decisions_validate_and_run_without_a_sandbox_process() {
    let sandbox = PolicyEngine::default();
    let compiled = sandbox
        .compile(Path::new("assets/policy/policy/policy.yaml"))
        .unwrap();
    let compiled = CompiledPolicy::from_bytes(&serde_json::to_vec(&compiled).unwrap()).unwrap();
    let no_worker = PolicyEngine::new(
        "/usr/bin/python3".into(),
        "/nonexistent/bwrap".into(),
        std::time::Duration::from_secs(1),
    )
    .unwrap();
    let cases = [
        (
            Decision::OpportunityPass,
            json!({"op":{"kind":"tool_execution","has_predictor":true}}),
            json!(false),
        ),
        (
            Decision::FeasibilityPass,
            json!({"op":{"kind":"tool_execution","has_predictor":true},"state":{},"contract_context":{}}),
            json!(true),
        ),
        (
            Decision::Extent,
            json!({"process_state":{"steps_completed":5,"hook_count":5,"phase":"reasoning"}}),
            json!("stop"),
        ),
        (
            Decision::Effect,
            json!({"context":{"capabilities":[]},"effect_descriptor":{}}),
            json!({"mode":"block"}),
        ),
        (
            Decision::Select,
            json!({"frontier_view": [{"chance_id":"one"}],"scheduling_context":{}}),
            json!([]),
        ),
        (
            Decision::Launch,
            json!({"chance_context":{},"candidate_view":[],"authority_state":{}}),
            json!(true),
        ),
    ];
    for (decision, input, expected) in &cases {
        assert!(compiled.pure_decision(*decision).is_some(), "{decision:?}");
        let result = no_worker.evaluate(&compiled, *decision, input);
        assert_eq!(
            result.unwrap_or_else(|error| panic!("{decision:?}: {error}")),
            *expected
        );
    }
    assert!(
        no_worker
            .evaluate(&compiled, Decision::Effect, &json!({}))
            .unwrap_err()
            .to_string()
            .contains("missing Decision input")
    );
    assert!(compiled.pure_decision(Decision::Predict).is_none());
    for (decision, data, _) in &cases {
        let count = 10_000;
        let start = std::time::Instant::now();
        for _ in 0..count {
            std::hint::black_box(
                no_worker
                    .evaluate(&compiled, *decision, std::hint::black_box(data))
                    .unwrap(),
            );
        }
        eprintln!(
            "{decision:?}: {:.2} µs/evaluation",
            start.elapsed().as_secs_f64() * 1e6 / f64::from(count)
        );
    }
}

#[test]
fn compile_rejects_opportunity_capabilities_and_defers_complex_python() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("policy.yaml");
    let source_path = dir.path().join("decisions.py");
    let source = include_str!("../assets/policy/policy/decisions.py");
    let signature = "def opportunity_pass(data, *, policy_config, capabilities):";
    std::fs::write(&path, include_str!("../assets/policy/policy/policy.yaml")).unwrap();
    std::fs::write(
        &source_path,
        source.replace(
            signature,
            &format!("{signature}\n    return capabilities.model_complete({{}})"),
        ),
    )
    .unwrap();
    let sandbox = PolicyEngine::default();
    let error = sandbox.compile(&path).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("admit.opportunity_pass cannot reference capabilities.model_complete"),
        "{error}"
    );

    for body in [
        "import math\n    return math.isfinite(1)",
        "for item in data:\n        return bool(item)\n    return False",
        "try:\n        return True\n    except Exception:\n        return False",
        "return unsupported_call(data)",
    ] {
        std::fs::write(
            &source_path,
            source.replace(signature, &format!("{signature}\n    {body}")),
        )
        .unwrap();
        let error = sandbox.compile(&path).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(
            error
                .to_string()
                .contains("admit.opportunity_pass must compile to a pure in-memory expression"),
            "{error}"
        );
    }

    let input = json!({"context":{},"effect_descriptor":{}});
    let effect = "def effect(data, *, policy_config, capabilities):";
    let no_worker = PolicyEngine::new(
        "/usr/bin/python3".into(),
        "/nonexistent/bwrap".into(),
        std::time::Duration::from_secs(1),
    )
    .unwrap();
    for (body, expected_error) in [
        ("return data['missing']", "KeyError"),
        ("return data['context'] + 1", "TypeError"),
        ("return []", "unsupported effect result"),
    ] {
        std::fs::write(
            &source_path,
            source.replace(effect, &format!("{effect}\n    {body}")),
        )
        .unwrap();
        let compiled = sandbox.compile(&path).unwrap();
        let error = no_worker
            .evaluate(&compiled, Decision::Effect, &input)
            .unwrap_err();
        assert!(error.to_string().contains(expected_error), "{error}");
    }
    for body in [
        "import re\n    return {'mode': 'allow' if re.match('a', 'a') else 'block'}",
        "return capabilities.model_complete({})",
    ] {
        std::fs::write(
            &source_path,
            source.replace(effect, &format!("{effect}\n    {body}")),
        )
        .unwrap();
        let compiled = sandbox.compile(&path).unwrap();
        assert!(compiled.pure_decision(Decision::Effect).is_none());
        let error = no_worker
            .evaluate(&compiled, Decision::Effect, &input)
            .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
        if body.starts_with("import") {
            assert_eq!(
                sandbox
                    .evaluate(&compiled, Decision::Effect, &input)
                    .unwrap(),
                json!({"mode":"allow"})
            );
        }
    }
}
