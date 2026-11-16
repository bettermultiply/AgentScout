//! 控制面协调测试。

use super::*;

use crate::control_plane::agent_control::{BranchRole, CommandOperation, CommandStatus};
use crate::control_plane::gateway::runtime::ControlServer;
use crate::control_plane::policy::CompiledPolicy;
use crate::protocol::contract::{
    AdapterMessage, ControlMessage, EnvironmentState, OperationResult, ResultStatus,
    RuntimeCapabilities,
};
use crate::protocol::transport;
use std::net::TcpStream;

fn task() -> (SpeculativeTree, tempfile::TempDir) {
    let root = tempfile::tempdir().unwrap();
    let mut tree = SpeculativeTree::with_workspace("agent", root.path()).unwrap();
    tree.execution_id = String::new();
    tree.bind_local_policy("test-policy");
    tree.create_authority("authority", "").unwrap();
    tree.branches.get_mut("authority").unwrap().environment_id = Some("runtime-authority".into());
    tree.handle_message(AgentMessage::BranchStarted {
        branch_id: "authority".into(),
    })
    .unwrap();
    (tree, root)
}

fn waiting() -> AgentMessage {
    AgentMessage::SpeculativeAdmissionRequest {
        task_id: String::new(),
        policy_digest: "test-policy".into(),
        parent_action_id: None,
        action_kind: crate::control_plane::agent_control::ActionKind::ToolExecution,
        branch_id: "authority".into(),
        action_id: "op".into(),
        name: "tool".into(),
        arguments: json!({}),
        checkpoint: None,
        timestamp_ms: 0,
    }
}

#[test]
fn promoting_branch_keeps_its_inflight_model_waiting_point_live() {
    use crate::control_plane::agent_control::{ActionKind, ActionStatus};
    let (mut tree, _root) = task();
    tree.speculative_admission(waiting()).unwrap();
    let AgentCommand::StartCandidate { branch_id, .. } =
        tree.start_candidate("chance-1", json!("guess")).unwrap()
    else {
        unreachable!()
    };
    tree.handle_message(AgentMessage::BranchStarted {
        branch_id: branch_id.clone(),
    })
    .unwrap();
    let mut model = waiting();
    if let AgentMessage::SpeculativeAdmissionRequest {
        branch_id: owner,
        action_id,
        action_kind,
        name,
        ..
    } = &mut model
    {
        *owner = branch_id.clone();
        *action_id = "inflight-model".into();
        *action_kind = ActionKind::ModelInference;
        *name = "model".into();
    }
    tree.speculative_admission(model).unwrap();
    tree.change_authority_state("chance-2", AuthorityState::Fetching)
        .unwrap();
    tree.branches.get_mut(&branch_id).unwrap().status = BranchStatus::Promoting;
    tree.sweep_chances().unwrap();
    assert_eq!(
        tree.chances.get("chance-2").map(|chance| &chance.authority),
        Some(&AuthorityState::Fetching),
        "promotion must not invalidate or remove the in-flight waiting point"
    );
    tree.handle_message(AgentMessage::ActionFinished {
        task_id: String::new(),
        branch_id: branch_id.clone(),
        action_id: "inflight-model".into(),
        action_kind: ActionKind::ModelInference,
        status: ActionStatus::Succeeded,
        timestamp_ms: 1,
        result: json!({"tool_calls":[]}),
        evidence: None,
    })
    .unwrap();
    assert_eq!(tree.chances["chance-2"].authority, AuthorityState::Arrived);
    let actions = tree.progress(&RuntimeCapabilities::default()).unwrap();
    assert!(actions.commands.iter().any(|command| matches!(command, AgentCommand::ResolvePending { branch_id: owner, operation_id, outcome: Some(outcome), .. } if owner == &branch_id && operation_id == "inflight-model" && outcome["phase"] == "continue")));
}

#[test]
fn interrupt_invalidates_a_prediction_already_in_flight() {
    let (mut tree, _root) = task();
    tree.speculative_admission(waiting()).unwrap();
    let select = tree
        .progress(&RuntimeCapabilities::default())
        .unwrap()
        .operations[0]
        .0;
    let predict = tree
        .complete_work(select, Ok(ControlResult::Selected(vec!["chance-1".into()])))
        .unwrap()
        .operations[0]
        .0;
    tree.interrupt().unwrap();
    let actions = tree
        .complete_work(
            predict,
            Ok(ControlResult::Predictions(vec![json!("guess")])),
        )
        .unwrap();
    assert!(actions.operations.is_empty());
    assert_eq!(tree.branches.len(), 1);
}

#[test]
fn validation_cannot_promote_after_parent_enters_a_later_operation() {
    let (mut tree, _root) = task();
    tree.speculative_admission(waiting()).unwrap();
    let command = tree.start_candidate("chance-1", json!("guess")).unwrap();
    let AgentCommand::StartCandidate { branch_id, .. } = command else {
        unreachable!()
    };
    tree.handle_message(AgentMessage::BranchStarted {
        branch_id: branch_id.clone(),
    })
    .unwrap();
    tree.record_authority_outcome("chance-1", json!({"payload":"guess"}), None)
        .unwrap();
    let mut actions = ControlActions::default();
    tree.schedule(ControlOperation::Validate(branch_id.clone()), &mut actions);
    let validation = actions.operations[0].0;
    let AgentMessage::SpeculativeAdmissionRequest {
        branch_id: parent,
        name: tool_name,
        arguments,
        checkpoint,
        ..
    } = waiting()
    else {
        unreachable!()
    };
    tree.speculative_admission(AgentMessage::SpeculativeAdmissionRequest {
        task_id: String::new(),
        policy_digest: "test-policy".into(),
        parent_action_id: None,
        action_kind: crate::control_plane::agent_control::ActionKind::ToolExecution,
        branch_id: parent,
        action_id: "later".into(),
        name: tool_name,
        arguments,
        checkpoint,
        timestamp_ms: 0,
    })
    .unwrap();
    let actions = tree
        .complete_work(
            validation,
            Ok(ControlResult::Disposition(
                true,
                "compatible".into(),
                None,
                Some(123),
            )),
        )
        .unwrap();
    assert!(
        !actions
            .operations
            .iter()
            .any(|(_, work)| matches!(work, ControlOperation::Promote(..)))
    );
    assert!(actions.commands.iter().any(
        |command| matches!(command,AgentCommand::Discard {branch_id:id,..} if id==&branch_id)
    ));
}

#[test]
fn notification_reaches_agent_only_after_runtime_success_and_completion_is_consumed() {
    for success in [false, true] {
        let (mut tree, _root) = task();
        tree.speculative_admission(waiting()).unwrap();
        let AgentCommand::StartCandidate { branch_id, .. } =
            tree.start_candidate("chance-1", json!("guess")).unwrap()
        else {
            unreachable!()
        };
        tree.handle_message(AgentMessage::BranchStarted {
            branch_id: branch_id.clone(),
        })
        .unwrap();
        tree.record_authority_outcome("chance-1", json!("real result"), None)
            .unwrap();
        let mut actions = ControlActions::default();
        tree.schedule(ControlOperation::Validate(branch_id.clone()), &mut actions);
        let validation = actions.operations.pop().unwrap().0;
        let mut actions = tree
            .complete_work(
                validation,
                Ok(ControlResult::Disposition(
                    true,
                    "compatible".into(),
                    Some("Policy text".into()),
                    Some(123),
                )),
            )
            .unwrap();
        assert!(
            !actions
                .commands
                .iter()
                .any(|c| matches!(c, AgentCommand::Promote { .. }))
        );
        let promotion = actions.operations.pop().unwrap().0;
        let actions = tree
            .complete_work(
                promotion,
                if success {
                    Ok(ControlResult::Completed)
                } else {
                    Err("Runtime failed".into())
                },
            )
            .unwrap();
        let notices: Vec<_> = actions
            .commands
            .iter()
            .filter_map(|c| match c {
                AgentCommand::Promote { notification, .. } => Some(notification.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            notices,
            if success {
                vec![Some("Policy text".into())]
            } else {
                vec![]
            }
        );
        assert!(
            tree.complete_work(promotion, Ok(ControlResult::Completed))
                .unwrap()
                .commands
                .is_empty()
        );
        let observations = tree.drain_observations();
        use crate::control_plane::observation::ObservationContent;
        assert!(observations.iter().any(|event| matches!(event, ObservationContent::ChanceCreated { chance_id, speculation_started_at_ms: Some(0), .. } if chance_id == "chance-1")));
        let proposal_id = tree
            .branch_proposal(&branch_id)
            .unwrap()
            .proposal_id
            .clone();
        assert!(observations.iter().any(|event| matches!(event, ObservationContent::ProposalValidated { chance_id, proposal_id: id, validation_id, validated_at_ms: Some(123) } if chance_id == "chance-1" && id == &proposal_id && *validation_id == validation)));
        assert!(!observations.iter().any(|event| matches!(
            event,
            ObservationContent::ChanceStageChanged {
                stage: ChanceStage::Accepted(_),
                ..
            }
        )));
        if success {
            let (command_id, generation) = actions
                .commands
                .into_iter()
                .find_map(|command| match command {
                    AgentCommand::Promote {
                        command_id,
                        generation,
                        ..
                    } => Some((command_id, generation)),
                    _ => None,
                })
                .unwrap();
            tree.handle_message(AgentMessage::CommandResult {
                command_id,
                generation,
                branch_id: branch_id.clone(),
                operation_id: None,
                operation: CommandOperation::Promote,
                status: CommandStatus::Applied,
                reason: None,
            })
            .unwrap();
            assert!(tree.drain_observations().iter().any(
                |event| matches!(event, ObservationContent::ChanceStageChanged {
                    stage: ChanceStage::Accepted(id), validation_id: Some(id_of_validation), ..
                } if id == &proposal_id && *id_of_validation == validation)
            ));
        }
    }
}

#[test]
fn candidate_authority_outcome_is_bound_to_its_chance_not_latest_operation() {
    let (mut tree, _root) = task();
    let first = tree
        .admit_chance("authority", "first", "tool", json!({}), None, None)
        .unwrap();
    let AgentCommand::StartCandidate {
        branch_id: first_candidate,
        ..
    } = tree.start_candidate(&first, json!("first guess")).unwrap()
    else {
        unreachable!()
    };
    tree.record_authority_outcome(
        &first,
        json!({"actual":"first real"}),
        Some(json!({"evidence":"first"})),
    )
    .unwrap();
    let second = tree
        .admit_chance("authority", "second", "tool", json!({}), None, None)
        .unwrap();
    let AgentCommand::StartCandidate {
        branch_id: second_candidate,
        ..
    } = tree
        .start_candidate(&second, json!("second guess"))
        .unwrap()
    else {
        unreachable!()
    };
    tree.record_authority_outcome(
        &second,
        json!({"actual":"second real"}),
        Some(json!({"evidence":"second"})),
    )
    .unwrap();
    assert_eq!(
        tree.candidate_outcome(&first_candidate),
        Some(&json!({"actual":"first real"}))
    );
    assert_eq!(
        tree.candidate_evidence(&first_candidate),
        Some(&json!({"evidence":"first"}))
    );
    assert_eq!(
        tree.candidate_outcome(&second_candidate),
        Some(&json!({"actual":"second real"}))
    );
    assert_eq!(
        tree.candidate_evidence(&second_candidate),
        Some(&json!({"evidence":"second"}))
    );
}

#[test]
fn dispose_receives_authority_outcome_without_replacing_match_evidence() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("policy.yaml"),
        include_str!("../../../assets/policy/policy/policy.yaml"),
    )
    .unwrap();
    let base = include_str!("../../../assets/policy/policy/decisions.py")
        .split("def match_proposal(")
        .next()
        .unwrap();
    let source = format!(
        "{}\n{}",
        base,
        r#"
def match_proposal(data, *, policy_config, capabilities):
    assert data["evidence"] == {"verifier": "evidence"}
    return {"verdict": "compatible"}
def dispose(data, *, policy_config, capabilities):
    assert data["context"]["match"]["verdict"] == "compatible"
    return {"actions": [{"intent": "Promote", "notification": data["context"]["authority_outcome"]["actual"]}]}
"#
    );
    std::fs::write(root.path().join("decisions.py"), source).unwrap();
    let policy = PolicyEngine::default()
        .compile(&root.path().join("policy.yaml"))
        .unwrap();
    let engine = PolicyEngine::default();
    let (mut tree, _workspace) = task();
    tree.speculative_admission(waiting()).unwrap();
    let AgentCommand::StartCandidate { branch_id, .. } =
        tree.start_candidate("chance-1", json!("guess")).unwrap()
    else {
        unreachable!()
    };
    let result = AgentController
        .disposition(
            &policy,
            &engine,
            &tree,
            &branch_id,
            tree.branch_proposal(&branch_id).unwrap(),
            "op",
            &json!({"verifier":"evidence"}),
            &json!({"actual":"Policy authoritative text"}),
        )
        .unwrap();
    assert_eq!(
        (&result.0, &result.1, &result.2),
        (
            &true,
            &"compatible".to_string(),
            &Some("Policy authoritative text".to_string())
        )
    );
    assert!(result.3.is_some());
}

#[test]
fn branch_cannot_be_archived_while_its_environment_is_still_being_created() {
    let (mut tree, _root) = task();
    let chance = tree
        .admit_chance("authority", "op", "tool", json!({}), None, None)
        .unwrap();
    let command = tree.start_candidate(&chance, json!("guess")).unwrap();
    let child = match &command {
        AgentCommand::StartCandidate { branch_id, .. } => branch_id.clone(),
        _ => unreachable!(),
    };
    tree.schedule(
        ControlOperation::Establish(command),
        &mut ControlActions::default(),
    );
    let cleanup = tree.begin_cleanup(&child).unwrap();
    let AgentCommand::Cleanup {
        command_id,
        generation,
        ..
    } = cleanup
    else {
        unreachable!()
    };
    tree.handle_message(AgentMessage::CommandResult {
        command_id,
        branch_id: child.clone(),
        operation_id: None,
        operation: CommandOperation::Cleanup,
        status: CommandStatus::Applied,
        generation,
        reason: None,
    })
    .unwrap();
    assert_eq!(tree.branches[&child].role, BranchRole::Candidate);
    assert!(!tree.evictable_branches().contains(&child));
}

#[test]
fn failed_creation_preserves_runtime_identity_without_starting_candidate() {
    let (mut tree, _root) = task();
    let chance = tree
        .admit_chance("authority", "op", "tool", json!({}), None, None)
        .unwrap();
    let command = tree.start_candidate(&chance, json!("guess")).unwrap();
    let child = match &command {
        AgentCommand::StartCandidate { branch_id, .. } => branch_id.clone(),
        _ => unreachable!(),
    };
    let mut scheduled = ControlActions::default();
    tree.schedule(ControlOperation::Establish(command), &mut scheduled);
    let actions = tree
        .complete_work(
            scheduled.operations[0].0,
            Ok(ControlResult::EnvironmentFailed(
                "runtime-partial-child".into(),
                "launch failed".into(),
            )),
        )
        .unwrap();
    assert_eq!(
        tree.branches[&child].environment_id.as_deref(),
        Some("runtime-partial-child")
    );
    assert_eq!(
        tree.branches[&child].execution_state.runtime,
        RuntimeExecutionState::Failed
    );
    assert!(
        !actions
            .commands
            .iter()
            .any(|command| matches!(command, AgentCommand::StartCandidate { .. }))
    );
    let restored: SpeculativeTree =
        serde_json::from_value(serde_json::to_value(&tree).unwrap()).unwrap();
    assert_eq!(
        restored.branches[&child].environment_id,
        tree.branches[&child].environment_id
    );
}

#[test]
fn firecracker_style_capabilities_route_establish_through_branch_environment() {
    let (mut tree, _root) = task();
    let chance = tree
        .admit_chance("authority", "op", "tool", json!({}), None, None)
        .unwrap();
    let command = tree.start_candidate(&chance, json!("guess")).unwrap();
    let child = match &command {
        AgentCommand::StartCandidate { branch_id, .. } => branch_id.clone(),
        _ => unreachable!(),
    };
    let policy: CompiledPolicy = serde_json::from_value(json!({
        "source": "/fixture/policy.yaml",
        "definition": "",
        "runtime_config_path": null,
        "version": 1,
        "python": "test",
        "files": {},
        "runtime": null,
        "decisions": {},
        "digest": "test"
    }))
    .unwrap();

    let server = ControlServer::bind("127.0.0.1:0").unwrap();
    let address = server.local_addr().unwrap();
    let recorded = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let client_recorded = recorded.clone();
    let client = std::thread::spawn(move || {
        let mut stream = TcpStream::connect(address).unwrap();
        loop {
            match transport::read_message::<_, ControlMessage>(&mut stream).unwrap() {
                Some(ControlMessage::Request(request)) => {
                    client_recorded
                        .lock()
                        .unwrap()
                        .push((request.environment_id.clone(), request.operation.clone()));
                    let result = OperationResult {
                        request_id: request.request_id.clone(),
                        environment_id: "runtime-returned-child".into(),
                        status: ResultStatus::Completed,
                        environment_state: Some(EnvironmentState::Available),
                        can_accept_operations: true,
                        follow_up: Vec::new(),
                        resources: Vec::new(),
                        remaining: Vec::new(),
                        backend_states: Vec::new(),
                        outputs: Vec::new(),
                        reason: None,
                        output: None,
                        filesystem: None,
                    };
                    transport::write_message(&mut stream, &AdapterMessage::Result(result)).unwrap();
                }
                None => break,
            }
        }
    });
    let session = server.accept().unwrap();
    let capabilities = RuntimeCapabilities {
        branch_environment: true,
        ..RuntimeCapabilities::default()
    };
    let mut scheduled = ControlActions::default();
    tree.schedule(ControlOperation::Establish(command.clone()), &mut scheduled);
    assert!(tree.branches[&child].environment_id.is_none());
    let result = AgentController
        .perform(
            &tree,
            &policy,
            &PolicyEngine::default(),
            &session,
            &capabilities,
            &ControlOperation::Establish(command),
        )
        .unwrap();
    assert!(matches!(&result, ControlResult::Environment(id) if id == "runtime-returned-child"));
    let actions = tree
        .complete_work(scheduled.operations[0].0, Ok(result))
        .unwrap();
    assert_eq!(
        tree.branches[&child].environment_id.as_deref(),
        Some("runtime-returned-child")
    );
    assert!(
        matches!(&actions.commands[0], AgentCommand::StartCandidate { environment_id: Some(id), .. } if id == "runtime-returned-child")
    );
    let calls = recorded.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        Some(calls[0].0.as_str()),
        tree.branches["authority"].environment_id.as_deref()
    );
    assert_eq!(calls[0].1, Operation::BranchEnvironment { agent: None });
    session.shutdown().unwrap();
    client.join().unwrap();
}

#[test]
fn expired_tool_wait_is_an_error_instead_of_an_immediate_timer_forever() {
    let (mut tree, _root) = task();
    tree.speculative_admission(waiting()).unwrap();
    tree.branches
        .get_mut("authority")
        .unwrap()
        .pending_operations
        .iter_mut()
        .find(|operation| operation.kind == OperationKind::Tool)
        .unwrap()
        .deadline_at = Some(0);
    assert!(tree.check_command_deadlines().is_err());
}
