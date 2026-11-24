
use super::super::*;
use super::support::*;
use crate::protocol::contract::RuntimeCapabilities;
use serde_json::json;

fn nested_tree() -> (SpeculativeTree, String, String) {
    let mut tree = tree();
    tree.execution_id = "task-1".into();
    tree.bind_local_policy("test-policy");
    tree.create_authority("authority", "").unwrap();
    tree.handle_message(AgentMessage::BranchStarted {
        branch_id: "authority".into(),
    })
    .unwrap();
    tree.speculative_admission(AgentMessage::SpeculativeAdmissionRequest {
        task_id: "task-1".into(),
        policy_digest: "test-policy".into(),
        branch_id: "authority".into(),
        action_id: "op-1".into(),
        action_kind: ActionKind::ToolExecution,
        parent_action_id: None,
        name: "tool".into(),
        arguments: json!({}),
        checkpoint: None,
        fast_start: None,
        timestamp_ms: 0,
    })
    .unwrap();
    let ancestor = "chance-1".to_string();
    let AgentCommand::StartCandidate { branch_id, .. } = tree
        .start_candidate(&ancestor, json!("ancestor guess"))
        .unwrap()
    else {
        unreachable!()
    };
    tree.handle_message(AgentMessage::BranchStarted {
        branch_id: branch_id.clone(),
    })
    .unwrap();

    tree.speculative_admission(AgentMessage::SpeculativeAdmissionRequest {
        task_id: "task-1".into(),
        policy_digest: "test-policy".into(),
        branch_id: branch_id.clone(),
        action_id: "op-2".into(),
        action_kind: ActionKind::ToolExecution,
        parent_action_id: None,
        name: "tool".into(),
        arguments: json!({}),
        checkpoint: None,
        fast_start: None,
        timestamp_ms: 1,
    })
    .unwrap();
    let nested = "chance-2";
    assert!(
        tree.chances.contains_key(nested),
        "a candidate Branch may admit a nested Chance"
    );

    tree.progress(&RuntimeCapabilities::default()).unwrap();
    assert_eq!(tree.chances[nested].stage, ChanceStage::Pending);
    assert_ne!(tree.chances[nested].authority, AuthorityState::Unavailable);

    let AgentCommand::StartCandidate {
        branch_id: nested_branch,
        ..
    } = tree.start_candidate(nested, json!("nested guess")).unwrap()
    else {
        unreachable!()
    };
    tree.handle_message(AgentMessage::BranchStarted {
        branch_id: nested_branch.clone(),
    })
    .unwrap();
    assert!(
        tree.branches[&nested_branch]
            .assumption_set
            .iter()
            .any(|assumption| assumption.chance_id == ancestor)
    );
    assert!(
        tree.branches[&nested_branch]
            .assumption_set
            .iter()
            .any(|assumption| assumption.chance_id == nested)
    );
    (tree, branch_id, nested_branch)
}

#[test]
fn nested_chance_waits_on_a_live_candidate_and_is_invalidated_with_it() {
    let (mut tree, branch_id, nested_branch) = nested_tree();
    let ancestor = "chance-1";
    let nested = "chance-2";
    assert!(
        tree.accept_candidate(&nested_branch).is_err(),
        "an unconfirmed ancestor Proposal blocks commit eligibility"
    );

    tree.record_authority_outcome(nested, json!("nested guess"), Some(json!({})))
        .unwrap();
    let actions = tree.progress(&RuntimeCapabilities::default()).unwrap();
    let validate = actions
        .operations
        .iter()
        .find_map(|(id, work)| {
            matches!(work, ControlOperation::Validate(branch) if branch == &nested_branch)
                .then_some(*id)
        })
        .expect("nested candidate is validated");
    let actions = tree
        .complete_work(
            validate,
            Ok(ControlResult::Disposition(
                true,
                "compatible".into(),
                None,
                Some(123),
            )),
        )
        .unwrap();
    assert!(
        !actions.commands.iter().any(
            |command| matches!(command, AgentCommand::Discard { branch_id, .. } if branch_id == &nested_branch)
        )
    );
    assert!(!matches!(
        tree.chances[nested].stage,
        ChanceStage::Discarded(_)
    ));

    tree.discard_candidate(
        &branch_id,
        DiscardedReason::MatchRejected,
        "ancestor Proposal rejected",
    )
    .unwrap();
    assert_eq!(
        tree.chances[nested].stage,
        ChanceStage::Discarded(DiscardedReason::AncestorInvalidated)
    );
    assert_eq!(
        tree.chances[ancestor].stage,
        ChanceStage::Discarded(DiscardedReason::MatchRejected)
    );
    assert_eq!(
        tree.branches[&nested_branch].status,
        BranchStatus::Discarding
    );
}

fn work_matching(tree: &mut SpeculativeTree, predicate: impl Fn(&ControlOperation) -> bool) -> u64 {
    tree.progress(&RuntimeCapabilities::default())
        .unwrap()
        .operations
        .into_iter()
        .find_map(|(id, operation)| predicate(&operation).then_some(id))
        .expect("expected work")
}

fn applied(tree: &mut SpeculativeTree, command: &AgentCommand) -> Vec<AgentCommand> {
    let (branch_id, command_id, generation, operation) = match command {
        AgentCommand::PrepareAcceptance {
            branch_id,
            command_id,
            generation,
            ..
        } => (
            branch_id,
            command_id,
            generation,
            CommandOperation::PrepareAcceptance,
        ),
        AgentCommand::Promote {
            branch_id,
            command_id,
            generation,
            ..
        } => (branch_id, command_id, generation, CommandOperation::Promote),
        AgentCommand::CancelAcceptance {
            branch_id,
            command_id,
            generation,
        } => (
            branch_id,
            command_id,
            generation,
            CommandOperation::CancelAcceptance,
        ),
        _ => panic!("unexpected command {command:?}"),
    };
    tree.handle_message(AgentMessage::CommandResult {
        branch_id: branch_id.clone(),
        command_id: command_id.clone(),
        generation: *generation,
        operation,
        operation_id: None,
        status: CommandStatus::Applied,
        reason: None,
    })
    .unwrap()
}

fn accept_transfer(
    tree: &mut SpeculativeTree,
    candidate: &str,
    parent: &str,
    chance: &str,
    full: bool,
) -> Vec<AgentCommand> {
    use crate::protocol::contract::{Acceptance, ExecutionAccess};
    let operation_id = tree.chances[chance].operation_id.clone();
    let cutoff = tree.branches[parent]
        .pending_operations
        .iter()
        .find(|op| op.operation_id == operation_id)
        .unwrap()
        .requested_at
        .unwrap();
    tree.chance_mut(chance).unwrap().launch_epoch = Some(tree.epoch);
    assert!(tree.acceptance_progress_compatible(&tree.branches[candidate]));
    let validate = work_matching(
        tree,
        |op| matches!(op, ControlOperation::Validate(id) if id == candidate),
    );
    let part = (!full).then(|| json!({"results":[{"id":"report","files":["/work/report"],"notification":"local report"}]}));
    let actions = tree
        .complete_work(
            validate,
            Ok(ControlResult::AcceptanceDisposition(
                true,
                "compatible".into(),
                None,
                Some(cutoff),
                full,
                part,
                None,
            )),
        )
        .unwrap();
    assert!(
        matches!(actions.commands.as_slice(), [AgentCommand::PrepareAcceptance { authority_branch_id, target_suspended: false, part, .. }] if authority_branch_id == parent && *part != full),
        "{candidate} -> {parent}: {:?} {:?}",
        actions.commands,
        actions.records
    );
    applied(tree, &actions.commands[0]);
    let suspend = work_matching(
        tree,
        |op| matches!(op, ControlOperation::SuspendAcceptance(id) if id == candidate),
    );
    let actions = tree
        .complete_work(suspend, Ok(ControlResult::Completed))
        .unwrap();
    assert!(
        matches!(actions.commands.as_slice(), [AgentCommand::PrepareAcceptance { authority_branch_id, target_suspended: true, part, .. }] if authority_branch_id == parent && *part != full)
    );
    applied(tree, &actions.commands[0]);
    let transfer = work_matching(
        tree,
        |op| matches!(op, ControlOperation::Promote(id, _) if id == candidate),
    );
    let outcome = if full {
        Acceptance::Full
    } else {
        Acceptance::PartResults {
            results: vec!["report".into()],
        }
    };
    let access = ExecutionAccess {
        environment_id: tree.branches[parent].environment_id().unwrap().into(),
        transport: "unix".into(),
        address: "/tmp/fake.sock".into(),
    };
    let actions = tree
        .complete_work(
            transfer,
            Ok(ControlResult::Acceptance(
                Some(outcome),
                full.then_some(access),
                None,
            )),
        )
        .unwrap();
    let expected_parent = (parent != "authority").then_some(parent);
    assert!(
        matches!(actions.commands.as_slice(), [AgentCommand::Promote { parent_branch_id, part, notification, .. }] if parent_branch_id.as_deref() == expected_parent && *part != full && (full || notification.as_deref() == Some("Accepted result: local report")))
    );
    applied(tree, &actions.commands[0])
}

#[test]
fn nested_transfer_preserves_outer_choice_and_descendants() {
    for (full, outer_accepts) in [(false, false), (false, true), (true, false), (true, true)] {
        let (mut tree, parent, child) = nested_tree();
        let descendant_chance = admit(&mut tree, &child, "op-3");
        let AgentCommand::StartCandidate {
            branch_id: descendant,
            ..
        } = tree
            .start_candidate(&descendant_chance, json!("descendant"))
            .unwrap()
        else {
            unreachable!()
        };
        tree.handle_message(AgentMessage::BranchStarted {
            branch_id: descendant.clone(),
        })
        .unwrap();
        for (branch, environment) in [
            ("authority", "A"),
            (&parent, "B"),
            (&child, "C"),
            (&descendant, "D"),
        ] {
            tree.branch_mut(branch).unwrap().environment_id = Some(environment.into());
        }
        let commands = accept_transfer(&mut tree, &child, &parent, "chance-2", full);
        let continuation = if full { &child } else { &parent };
        assert_eq!(tree.authority_branch_id(), Some("authority"));
        assert!(!tree.chances["chance-1"].stage.is_terminal());
        assert!(matches!(
            tree.chances["chance-2"].stage,
            ChanceStage::Accepted(_)
        ));
        assert_eq!(tree.branches[continuation].environment_id(), Some("B"));
        assert_eq!(
            tree.branch_proposal(continuation).unwrap().payload,
            json!("ancestor guess")
        );
        assert_eq!(tree.branches[&descendant].environment_id(), Some("D"));
        assert_eq!(tree.branches[&descendant].status.is_live_candidate(), full);
        if full {
            assert_eq!(tree.branches[&parent].environment_id(), Some("C"));
            assert!(tree.has_live_environment_dependents(&parent));
        }
        let release = commands.iter().find(|cmd| matches!(cmd, AgentCommand::CancelAcceptance { branch_id, .. } if branch_id == continuation)).unwrap();
        applied(&mut tree, release);
        if outer_accepts {
            accept_transfer(&mut tree, continuation, "authority", "chance-1", true);
            if full {
                assert_eq!(
                    tree.acceptance_target(&tree.branches[&descendant]),
                    Some(child.as_str())
                );
            }
            assert_eq!(tree.authority_branch_id(), Some(continuation.as_str()));
            assert_eq!(tree.branches[continuation].environment_id(), Some("A"));
            assert_eq!(tree.branches["authority"].environment_id(), Some("B"));
            assert!(matches!(
                tree.chances["chance-1"].stage,
                ChanceStage::Accepted(_)
            ));
        } else {
            tree.discard_candidate(
                continuation,
                DiscardedReason::MatchRejected,
                "outer Proposal rejected",
            )
            .unwrap();
            assert_eq!(tree.authority_branch_id(), Some("authority"));
            assert_eq!(tree.branches[continuation].status, BranchStatus::Discarding);
            assert_eq!(tree.branches[&descendant].status, BranchStatus::Discarding);
        }
    }
}

#[test]
fn yield_preserves_active_work_and_task_end_discards_candidate() {
    let (mut tree, candidate, _) = nested_tree();
    tree.record_authority_outcome("chance-1", json!("different result"), None).unwrap();
    tree.chance_mut("chance-1").unwrap().tool_continuation_pending = true;
    tree.handle_message(AgentMessage::ActionStarted {
        task_id: "task-1".into(), branch_id: candidate.clone(), action_id: "verify".into(),
        action_kind: ActionKind::ToolExecution, name: "bash".into(), arguments: json!({"command":"pytest"}),
        parent_action_id: None, checkpoint: None, timestamp_ms: 10,
    }).unwrap();
    let mut actions = ControlActions::default();
    tree.schedule(ControlOperation::Validate(candidate.clone()), &mut actions);
    let id = actions.operations[0].0;
    let delivery = tree.complete_work(id, Ok(ControlResult::AcceptanceDisposition(
        false, "incompatible".into(), None, None, false,
        Some(json!({"intent":"Yield", "results":[]})), None,
    ))).unwrap();
    assert!(matches!(delivery.commands.as_slice(), [AgentCommand::ResolvePending { branch_id, outcome: Some(outcome), .. }]
        if branch_id == "authority" && outcome["phase"] == "continue"));
    assert!(tree.branches[&candidate].yield_only);
    assert_eq!(tree.branches[&candidate].status, BranchStatus::Running);
    assert!(tree.active_actions[&candidate].contains_key("verify"));
    assert!(!tree.can_promote(&candidate, true));
    assert!(tree.accept_candidate(&candidate).is_err());
    assert!(!tree.candidate_due(&candidate));

    tree.finish_task();
    let actions = tree.progress(&RuntimeCapabilities::default()).unwrap();
    assert!(actions.commands.iter().any(|command| matches!(command, AgentCommand::Discard { branch_id, .. } if branch_id == &candidate)));
    assert_eq!(tree.branches[&candidate].status, BranchStatus::Discarding);
    assert!(!tree.task_writers_drained());
}

#[test]
fn running_yield_candidates_batch_two_tools_at_the_authority_model_boundary() {
    let (mut tree, candidate, _) = nested_tree();
    tree.branches.get_mut(&candidate).unwrap().yield_only = true;
    for id in ["first", "second"] {
        tree.handle_message(AgentMessage::ActionStarted {
            task_id: "task-1".into(), branch_id: candidate.clone(), action_id: id.into(),
            action_kind: ActionKind::ToolExecution, name: "bash".into(), arguments: json!({}),
            parent_action_id: None, checkpoint: None, timestamp_ms: 10,
        }).unwrap();
        tree.handle_message(AgentMessage::ActionFinished {
            task_id: "task-1".into(), branch_id: candidate.clone(), action_id: id.into(),
            action_kind: ActionKind::ToolExecution, status: ActionStatus::Succeeded,
            result: json!(id), evidence: None, timestamp_ms: 11,
        }).unwrap();
        assert!(!tree.candidate_due(&candidate));
    }
    tree.model_inference_boundary("task-1", "authority", "next-model").unwrap();
    let capabilities = RuntimeCapabilities::default();
    let actions = tree.progress(&capabilities).unwrap();
    let validation = actions.operations.iter().find(|(_, op)| matches!(op, ControlOperation::Validate(id) if id == &candidate)).unwrap().0;
    assert!(!actions.commands.iter().any(|op| matches!(op, AgentCommand::ResolvePending { operation_id, .. } if operation_id == "next-model")));
    let delivery = tree.complete_work(validation, Ok(ControlResult::AcceptanceDisposition(
        false, "incompatible".into(), None, None, false,
        Some(json!({"intent":"Yield", "results":[
            {"id":"first", "files":[], "notification":"first output"},
            {"id":"second", "files":[], "notification":"second output"}
        ]})), None,
    ))).unwrap();
    assert!(matches!(delivery.commands.as_slice(), [AgentCommand::ResolvePending { outcome: Some(outcome), .. }]
        if outcome["notification"] == "first output\nsecond output"));
    let actions = tree.progress(&capabilities).unwrap();
    assert!(actions.commands.iter().any(|op| matches!(op, AgentCommand::ResolvePending { operation_id, outcome: Some(outcome), .. }
        if operation_id == "next-model" && outcome["phase"] == "execute")));
    assert!(!tree.candidate_due(&candidate));
}

#[test]
fn nested_transfer_uses_its_parent_outcome_and_progress() {
    let (mut tree, parent, child) = nested_tree();
    tree.record_authority_outcome("chance-1", json!("A result"), None)
        .unwrap();
    assert!(tree.candidate_outcome(&child).is_none());
    assert!(!tree.candidate_due(&child));
    tree.change_authority_state("chance-2", AuthorityState::Fetching)
        .unwrap();
    tree.handle_message(AgentMessage::ActionStarted {
        task_id: "task-1".into(),
        branch_id: parent.clone(),
        action_id: "op-2".into(),
        action_kind: ActionKind::ToolExecution,
        name: "tool".into(),
        arguments: json!({}),
        parent_action_id: None,
        checkpoint: None,
        timestamp_ms: 10,
    })
    .unwrap();
    tree.handle_message(AgentMessage::ActionFinished {
        task_id: "task-1".into(),
        branch_id: parent.clone(),
        action_id: "op-2".into(),
        action_kind: ActionKind::ToolExecution,
        status: ActionStatus::Succeeded,
        result: json!("B result"),
        evidence: Some(json!({"source":"B"})),
        timestamp_ms: 11,
    })
    .unwrap();
    assert_eq!(tree.candidate_outcome(&child), Some(&json!("B result")));
    assert_eq!(
        tree.candidate_evidence(&child),
        Some(&json!({"source":"B"}))
    );
    assert!(tree.candidate_due(&child));
    assert!(tree.acceptance_progress_compatible(&tree.branches[&child]));
    assert!(!tree.can_accept(&child, false, true));
    assert!(tree.can_accept(&child, true, true));
    tree.branch_mut(&child).unwrap().assumption_set[0].proposal_id = "different proposal".into();
    assert!(!tree.can_accept(&child, true, true));
    tree.branch_mut(&child).unwrap().assumption_set[0] =
        tree.branches[&parent].assumption_set[0].clone();
    admit(&mut tree, &parent, "later-tool");
    assert!(!tree.acceptance_progress_compatible(&tree.branches[&child]));
    assert!(!tree.can_accept(&child, true, true));
}

fn admit(tree: &mut SpeculativeTree, branch: &str, action: &str) -> String {
    tree.speculative_admission(AgentMessage::SpeculativeAdmissionRequest {
        task_id: "task-1".into(),
        policy_digest: "test-policy".into(),
        branch_id: branch.into(),
        action_id: action.into(),
        action_kind: ActionKind::ToolExecution,
        parent_action_id: None,
        name: "tool".into(),
        arguments: json!({}),
        checkpoint: None,
        fast_start: None,
        timestamp_ms: 2,
    })
    .unwrap();
    tree.chances
        .values()
        .find(|chance| chance.operation_id == action)
        .unwrap()
        .chance_id
        .clone()
}
