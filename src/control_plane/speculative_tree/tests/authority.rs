use super::super::*;
use super::support::*;
use crate::protocol::contract::RuntimeCapabilities;
use serde_json::json;

fn admit(tree: &mut SpeculativeTree, operation: &str, kind: ActionKind) {
    tree.speculative_admission(AgentMessage::SpeculativeAdmissionRequest {
        task_id: "task-1".into(),
        policy_digest: "test-policy".into(),
        branch_id: "authority".into(),
        action_id: operation.into(),
        action_kind: kind,
        name: "test".into(),
        arguments: json!({}),
        checkpoint: None,
        parent_action_id: None,
        fast_start: None,
        timestamp_ms: 1,
    })
    .unwrap();
}

fn selection_tree() -> SpeculativeTree {
    let mut tree = tree();
    tree.execution_id = "task-1".into();
    tree.bind_local_policy("test-policy");
    tree.create_authority("authority", "").unwrap();
    tree.handle_message(AgentMessage::BranchStarted {
        branch_id: "authority".into(),
    })
    .unwrap();
    tree
}

#[test]
fn fetching_chance_is_reselected_when_rebalance_frees_capacity() {
    for kind in [ActionKind::ToolExecution, ActionKind::ModelInference] {
        let mut tree = selection_tree();
        let capabilities = RuntimeCapabilities::default();
        admit(&mut tree, "old-op", ActionKind::ToolExecution);
        tree.change_chance_stage("chance-1", ChanceStage::Running, "test setup")
            .unwrap();
        tree.change_authority_state("chance-1", AuthorityState::Fetching)
            .unwrap();
        for _ in 0..2 {
            let AgentCommand::StartCandidate { branch_id, .. } =
                tree.start_candidate("chance-1", json!("guess")).unwrap()
            else {
                unreachable!()
            };
            tree.handle_message(AgentMessage::BranchStarted {
                branch_id: branch_id.clone(),
            })
            .unwrap();
            tree.branch_mut(&branch_id).unwrap().yield_only = true;
        }
        let candidates = tree.pending_candidates();
        assert_eq!(candidates.len(), 2);
        admit(&mut tree, "waiting-op", kind);
        let actions = tree.progress(&capabilities).unwrap();
        let (select, _) = actions
            .operations
            .iter()
            .find(|(_, op)| {
                matches!(op,
            ControlOperation::Select(ids) if ids == &["chance-2"])
            })
            .unwrap();
        let (rebalance, _) = actions
            .operations
            .iter()
            .find(|(_, op)| matches!(op, ControlOperation::Rebalance))
            .unwrap();
        let actions = tree
            .complete_work(*select, Ok(ControlResult::Selected(vec![])))
            .unwrap();
        assert!(
            matches!(actions.commands.as_slice(), [AgentCommand::ResolvePending { outcome: Some(outcome), .. }]
            if *outcome == json!({"phase":"execute"}))
        );
        assert_eq!(tree.chances["chance-2"].authority, AuthorityState::Fetching);
        assert_eq!(tree.selectable_chances(), ["chance-2"]);
        assert!(
            !tree
                .progress(&capabilities)
                .unwrap()
                .operations
                .iter()
                .any(|(_, op)| matches!(op, ControlOperation::Select(_)))
        );
        tree.change_chance_stage("chance-1", ChanceStage::Resolving, "prior chance advanced")
            .unwrap();
        let actions = tree.progress(&capabilities).unwrap();
        let (select, _) = actions
            .operations
            .iter()
            .find(|(_, op)| {
                matches!(op,
            ControlOperation::Select(ids) if ids == &["chance-2"])
            })
            .unwrap();
        let epoch = tree.epoch;
        let actions = tree
            .complete_work(*select, Ok(ControlResult::Selected(vec![])))
            .unwrap();
        assert!(actions.commands.is_empty());
        assert_eq!(tree.epoch, epoch);
        let preempt = tree
            .complete_work(
                *rebalance,
                Ok(ControlResult::Rebalance(vec![(
                    candidates[0].clone(),
                    "preempt".into(),
                )])),
            )
            .unwrap();
        let (dispose, _) = preempt.operations.iter().find(|(_, op)| matches!(op, ControlOperation::Dispose(_, reason) if reason == "preempted")).unwrap();
        tree.complete_work(
            *dispose,
            Ok(ControlResult::Disposition(
                false,
                "incompatible".into(),
                None,
                None,
            )),
        )
        .unwrap();
        assert_eq!(tree.pending_candidates().len(), 1);
        let actions = tree.progress(&capabilities).unwrap();
        let (select, _) = actions
            .operations
            .iter()
            .find(|(_, op)| {
                matches!(op,
            ControlOperation::Select(ids) if ids == &["chance-2"])
            })
            .unwrap();
        let actions = tree
            .complete_work(
                *select,
                Ok(ControlResult::Selected(vec!["chance-2".into()])),
            )
            .unwrap();
        let (predict, _) = actions
            .operations
            .iter()
            .find(|(_, op)| matches!(op, ControlOperation::Predict(_)))
            .unwrap();
        let result = if kind == ActionKind::ModelInference {
            ControlResult::BranchModel(json!({}))
        } else {
            ControlResult::Predictions(vec![json!({"payload": "guess"})])
        };
        let actions = tree.complete_work(*predict, Ok(result)).unwrap();
        assert!(
            actions
                .operations
                .iter()
                .any(|(_, op)| matches!(op, ControlOperation::Establish(_)))
        );
        assert_eq!(tree.chances["chance-2"].authority, AuthorityState::Fetching);
        assert!(actions.commands.is_empty());
    }
}

#[test]
fn model_authority_arrival_discards_unselected_chance_and_releases_wait() {
    let mut tree = selection_tree();
    let capabilities = RuntimeCapabilities::default();
    admit(&mut tree, "model-op", ActionKind::ModelInference);
    let actions = tree.progress(&capabilities).unwrap();
    tree.complete_work(actions.operations[0].0, Ok(ControlResult::Selected(vec![])))
        .unwrap();
    tree.handle_message(AgentMessage::ActionFinished {
        task_id: "task-1".into(),
        branch_id: "authority".into(),
        action_id: "model-op".into(),
        action_kind: ActionKind::ModelInference,
        status: ActionStatus::Succeeded,
        timestamp_ms: 2,
        result: json!({"tool_calls": []}),
        evidence: None,
    })
    .unwrap();
    assert_eq!(
        tree.chances["chance-1"].stage,
        ChanceStage::Discarded(DiscardedReason::NotSelected)
    );
    assert_eq!(tree.chances["chance-1"].authority, AuthorityState::Arrived);
    let actions = tree.progress(&capabilities).unwrap();
    assert!(
        matches!(actions.commands.as_slice(), [AgentCommand::ResolvePending { operation_id, outcome: Some(outcome), .. }]
        if operation_id == "model-op" && *outcome == json!({"phase":"continue"}))
    );
    assert!(tree.progress(&capabilities).unwrap().commands.is_empty());
}

#[test]
fn authority_continuation_waits_for_promotion_and_prunes_only_after_confirmation() {
    let mut tree = tree();
    tree.workspace_root =
        std::env::temp_dir().join(format!("ascout-continuation-{}", std::process::id()));
    tree.create_authority("authority", "").unwrap();
    tree.handle_message(AgentMessage::BranchStarted {
        branch_id: "authority".into(),
    })
    .unwrap();
    let chance = tree
        .admit_chance("authority", "op", "tool", json!({}), None, None)
        .unwrap();
    let first = tree.start_candidate(&chance, json!("first")).unwrap();
    let AgentCommand::StartCandidate {
        branch_id: winner, ..
    } = first
    else {
        unreachable!()
    };
    let second = tree.start_candidate(&chance, json!("second")).unwrap();
    let AgentCommand::StartCandidate {
        branch_id: sibling, ..
    } = second
    else {
        unreachable!()
    };
    for id in [&winner, &sibling] {
        tree.handle_message(AgentMessage::BranchStarted {
            branch_id: id.clone(),
        })
        .unwrap();
    }
    tree.handle_message(AgentMessage::TurnCompleted {
        branch_id: winner.clone(),
    })
    .unwrap();
    let commands = tree.accept_candidate(&winner).unwrap();
    assert_eq!(tree.authority_branch_id.as_deref(), Some("authority"));
    assert_eq!(tree.branches[&sibling].status, BranchStatus::Running);
    tree.queue_prompt("next".into());
    assert!(tree.next_prompt().unwrap().is_none());
    let AgentCommand::Promote {
        command_id,
        generation,
        ..
    } = &commands[0]
    else {
        unreachable!()
    };
    let commands = tree
        .handle_message(AgentMessage::CommandResult {
            command_id: command_id.clone(),
            branch_id: winner.clone(),
            operation_id: None,
            operation: CommandOperation::Promote,
            status: CommandStatus::Applied,
            generation: *generation,
            reason: None,
        })
        .unwrap();
    assert!(commands.iter().any(|command| matches!(command, AgentCommand::Discard { branch_id, .. } if branch_id == &sibling)));
    // Promotion does not let the next prompt bypass writer drain and materialization.
    assert!(tree.task_finishing());
    assert!(tree.next_prompt().unwrap().is_none());
    tree.complete_task();
    assert!(
        matches!(tree.next_prompt().unwrap(), Some(AgentCommand::Prompt { branch_id, .. }) if branch_id == winner)
    );
    assert!(tree.authority_running());
    assert!(
        tree.branches[&winner]
            .pending_operations
            .iter()
            .all(|operation| operation.requested_at.is_some() && operation.deadline_at.is_some())
    );
}

#[test]
fn unavailable_tool_result_discards_candidates_and_releases_parent() {
    let mut tree = tree();
    tree.create_authority("authority", "").unwrap();
    tree.handle_message(AgentMessage::BranchStarted {
        branch_id: "authority".into(),
    })
    .unwrap();
    let chance = tree
        .admit_chance("authority", "op", "tool", json!({}), None, None)
        .unwrap();
    let AgentCommand::StartCandidate { branch_id, .. } =
        tree.start_candidate(&chance, json!("guess")).unwrap()
    else {
        unreachable!()
    };
    tree.chance_mut(&chance).unwrap().tool_continuation_pending = true;
    let mut actions = ControlActions::default();
    tree.complete_tool_outcome(
        ToolPending {
            chance_id: chance.clone(),
            branch_id: "authority".into(),
            operation_id: "op".into(),
            tool_name: "tool".into(),
            arguments: json!({}),
        },
        Err("transport interrupted".into()),
        &mut actions,
    )
    .unwrap();
    assert_eq!(
        tree.chance(&chance).unwrap().stage,
        ChanceStage::Discarded(DiscardedReason::Failed)
    );
    assert_eq!(
        tree.chance(&chance).unwrap().authority,
        AuthorityState::Unavailable
    );
    assert!(!tree.chance(&chance).unwrap().tool_continuation_pending);
    assert_eq!(tree.branches[&branch_id].status, BranchStatus::Discarding);
    assert!(actions.commands.iter().any(
        |command| matches!(command, AgentCommand::Discard { branch_id: id, .. } if id == &branch_id)
    ));
    assert!(actions.commands.iter().any(|command| matches!(command, AgentCommand::ResolvePending { operation_id, outcome: Some(value), .. } if operation_id == "op" && value["phase"] == "continue")));
}

#[test]
fn authority_branch_failure_fails_the_run_without_discard() {
    let mut tree = tree();
    tree.branches.insert(
        "authority".into(),
        BranchRecord {
            branch_id: "authority".into(),
            environment_id: Some("environment-authority".into()),
            parent_branch_id: None,
            role: BranchRole::Authority,
            status: BranchStatus::Running,
            generation: 1,
            execution_position: Default::default(),
            chance_id: None,
            assumption_set: Vec::new(),
            yield_only: false,
            deferred_environment: false,
            yielded_action_ids: Default::default(),
            pending_operations: Vec::new(),
            execution_state: Default::default(),
        },
    );
    tree.authority_branch_id = Some("authority".into());

    let commands = tree
        .handle_message(AgentMessage::BranchFailed {
            branch_id: "authority".into(),
            reason: "429".into(),
        })
        .unwrap();

    assert!(commands.is_empty());
    assert_eq!(tree.branches["authority"].status, BranchStatus::Running);
    assert!(tree.authority_failure().is_some());
}

#[test]
fn late_or_duplicate_branch_started_is_ignored() {
    let mut tree = tree();
    for status in [
        BranchStatus::Running,
        BranchStatus::Discarding,
        BranchStatus::Discarded,
        BranchStatus::Cleaning,
        BranchStatus::Cleaned,
    ] {
        tree.branches.insert(
            "candidate".into(),
            BranchRecord {
                branch_id: "candidate".into(),
                environment_id: Some("environment-candidate".into()),
                parent_branch_id: None,
                role: BranchRole::Candidate,
                status,
                generation: 1,

                execution_position: Default::default(),
                chance_id: None,
                assumption_set: Vec::new(),
                yield_only: false,
                deferred_environment: false,
                yielded_action_ids: Default::default(),
                pending_operations: Vec::new(),
                execution_state: Default::default(),
            },
        );
        let commands = tree
            .handle_message(AgentMessage::BranchStarted {
                branch_id: "candidate".into(),
            })
            .unwrap();
        assert!(commands.is_empty());
        assert_eq!(tree.branches["candidate"].status, status);
    }
}
