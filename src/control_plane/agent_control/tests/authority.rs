//! authority 测试。

use super::super::*;
use super::support::*;
use serde_json::json;

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
    // A finished winner ends this task. Promotion confirmation alone must
    // not let the next prompt bypass writer drain and materialization.
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
fn authority_branch_failure_fails_the_run_without_discard() {
    let mut tree = tree();
    tree.branches.insert(
        "authority".into(),
        BranchRecord {
            branch_id: "authority".into(),
            parent_branch_id: None,
            role: BranchRole::Authority,
            status: BranchStatus::Running,
            generation: 1,
            execution_position: Default::default(),
            chance_id: None,
            assumption_set: Vec::new(),
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
                parent_branch_id: None,
                role: BranchRole::Candidate,
                status,
                generation: 1,

                execution_position: Default::default(),
                chance_id: None,
                assumption_set: Vec::new(),
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
