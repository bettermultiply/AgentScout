//! promotion 测试。

use super::super::*;
use super::support::*;

#[test]
fn candidate_shares_run_workspace_and_promotes() {
    let mut tree = tree();
    tree.branches.insert(
        "parent".into(),
        BranchRecord {
            branch_id: "parent".into(),
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
    tree.branches.insert(
        "candidate".into(),
        BranchRecord {
            branch_id: "candidate".into(),
            parent_branch_id: Some("parent".into()),
            role: BranchRole::Candidate,
            status: BranchStatus::Running,
            generation: 2,

            execution_position: Default::default(),
            chance_id: None,
            assumption_set: Vec::new(),
            pending_operations: Vec::new(),
            execution_state: Default::default(),
        },
    );
    tree.authority_branch_id = Some("parent".into());
    let commands = tree.accept_candidate("candidate").unwrap();
    assert!(
        commands
            .iter()
            .any(|command| matches!(command, AgentCommand::Promote { .. }))
    );
    assert_eq!(tree.authority_branch_id.as_deref(), Some("parent"));
    assert_eq!(tree.branches["parent"].status, BranchStatus::Running);
    let promote = commands
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
    let commands = tree
        .handle_message(AgentMessage::CommandResult {
            command_id: promote.0,
            branch_id: "candidate".into(),
            operation_id: None,
            operation: CommandOperation::Promote,
            status: CommandStatus::Applied,
            generation: promote.1,
            reason: None,
        })
        .unwrap();
    assert!(
        commands
            .iter()
            .any(|command| matches!(command, AgentCommand::Discard { .. }))
    );
    assert_eq!(tree.authority_branch_id.as_deref(), Some("candidate"));
}

#[test]
fn mismatch_discards_and_cleans_candidate() {
    let mut tree = tree();
    tree.branches.insert(
        "parent".into(),
        BranchRecord {
            branch_id: "parent".into(),
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
    tree.branches.insert(
        "candidate".into(),
        BranchRecord {
            branch_id: "candidate".into(),
            parent_branch_id: Some("parent".into()),
            role: BranchRole::Candidate,
            status: BranchStatus::Running,
            generation: 2,

            execution_position: Default::default(),
            chance_id: None,
            assumption_set: Vec::new(),
            pending_operations: Vec::new(),
            execution_state: Default::default(),
        },
    );
    tree.authority_branch_id = Some("parent".into());
    let commands = tree
        .discard_candidate("candidate", DiscardedReason::MatchRejected, "test")
        .unwrap();
    assert!(
        commands
            .iter()
            .any(|command| matches!(command, AgentCommand::Discard { .. }))
    );
    let discard = commands
        .into_iter()
        .find_map(|command| match command {
            AgentCommand::Discard {
                command_id,
                generation,
                ..
            } => Some((command_id, generation)),
            _ => None,
        })
        .unwrap();
    let commands = tree
        .handle_message(AgentMessage::CommandResult {
            command_id: discard.0,
            branch_id: "candidate".into(),
            operation_id: None,
            operation: CommandOperation::Discard,
            status: CommandStatus::Applied,
            generation: discard.1,
            reason: None,
        })
        .unwrap();
    let cleanup = commands
        .into_iter()
        .find_map(|command| match command {
            AgentCommand::Cleanup {
                command_id,
                generation,
                ..
            } => Some((command_id, generation)),
            _ => None,
        })
        .unwrap();
    tree.handle_message(AgentMessage::CommandResult {
        command_id: cleanup.0,
        branch_id: "candidate".into(),
        operation_id: None,
        operation: CommandOperation::Cleanup,
        status: CommandStatus::Applied,
        generation: cleanup.1,
        reason: None,
    })
    .unwrap();
    assert_eq!(tree.branches["candidate"].status, BranchStatus::Cleaned);
}

#[test]
fn failed_promotion_discards_without_switching_authority() {
    let mut tree = tree();
    tree.branches.insert(
        "parent".into(),
        BranchRecord {
            branch_id: "parent".into(),
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
    tree.branches.insert(
        "candidate".into(),
        BranchRecord {
            branch_id: "candidate".into(),
            parent_branch_id: Some("parent".into()),
            role: BranchRole::Candidate,
            status: BranchStatus::Running,
            generation: 2,

            execution_position: Default::default(),
            chance_id: None,
            assumption_set: Vec::new(),
            pending_operations: Vec::new(),
            execution_state: Default::default(),
        },
    );
    tree.authority_branch_id = Some("parent".into());
    let command = tree.accept_candidate("candidate").unwrap().remove(0);
    let (command_id, generation) = match command {
        AgentCommand::Promote {
            command_id,
            generation,
            ..
        } => (command_id, generation),
        _ => unreachable!(),
    };
    let commands = tree
        .handle_message(AgentMessage::CommandResult {
            command_id,
            branch_id: "candidate".into(),
            operation_id: None,
            operation: CommandOperation::Promote,
            status: CommandStatus::Failed,
            generation,
            reason: Some("failed".into()),
        })
        .unwrap();
    assert!(
        commands
            .iter()
            .any(|command| matches!(command, AgentCommand::Discard { .. }))
    );
    assert_eq!(tree.authority_branch_id.as_deref(), Some("parent"));
    assert_eq!(tree.branches["candidate"].status, BranchStatus::Discarding);
}

#[test]
fn branch_failure_during_promotion_waits_for_promote_result() {
    let mut tree = tree();
    tree.branches.insert(
        "parent".into(),
        BranchRecord {
            branch_id: "parent".into(),
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
    tree.branches.insert(
        "candidate".into(),
        BranchRecord {
            branch_id: "candidate".into(),
            parent_branch_id: Some("parent".into()),
            role: BranchRole::Candidate,
            status: BranchStatus::Running,
            generation: 2,

            execution_position: Default::default(),
            chance_id: None,
            assumption_set: Vec::new(),
            pending_operations: Vec::new(),
            execution_state: Default::default(),
        },
    );
    tree.authority_branch_id = Some("parent".into());
    let command = tree.accept_candidate("candidate").unwrap().remove(0);
    let (command_id, generation) = match command {
        AgentCommand::Promote {
            command_id,
            generation,
            ..
        } => (command_id, generation),
        _ => unreachable!(),
    };

    let commands = tree
        .handle_message(AgentMessage::BranchFailed {
            branch_id: "candidate".into(),
            reason: "429".into(),
        })
        .unwrap();

    assert!(commands.is_empty());
    assert_eq!(tree.branches["candidate"].status, BranchStatus::Promoting);
    assert_eq!(tree.authority_branch_id.as_deref(), Some("parent"));
    assert!(tree.authority_failure().is_none());

    let commands = tree
        .handle_message(AgentMessage::CommandResult {
            command_id,
            branch_id: "candidate".into(),
            operation_id: None,
            operation: CommandOperation::Promote,
            status: CommandStatus::Applied,
            generation,
            reason: None,
        })
        .unwrap();

    assert_eq!(tree.authority_branch_id.as_deref(), Some("candidate"));
    assert_eq!(tree.branches["candidate"].status, BranchStatus::Promoted);
    assert!(tree.authority_failure().is_some());
    assert!(commands.iter().any(|command| matches!(
        command,
        AgentCommand::Discard { branch_id, .. } if branch_id == "parent"
    )));
}

#[test]
fn late_promote_result_after_failed_promotion_is_ignored() {
    let mut tree = tree();
    tree.branches.insert(
        "parent".into(),
        BranchRecord {
            branch_id: "parent".into(),
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
    tree.branches.insert(
        "candidate".into(),
        BranchRecord {
            branch_id: "candidate".into(),
            parent_branch_id: Some("parent".into()),
            role: BranchRole::Candidate,
            status: BranchStatus::Running,
            generation: 2,

            execution_position: Default::default(),
            chance_id: None,
            assumption_set: Vec::new(),
            pending_operations: Vec::new(),
            execution_state: Default::default(),
        },
    );
    tree.authority_branch_id = Some("parent".into());
    let command = tree.accept_candidate("candidate").unwrap().remove(0);
    let (command_id, generation) = match command {
        AgentCommand::Promote {
            command_id,
            generation,
            ..
        } => (command_id, generation),
        _ => unreachable!(),
    };
    tree.handle_message(AgentMessage::BranchFailed {
        branch_id: "candidate".into(),
        reason: "429".into(),
    })
    .unwrap();
    tree.handle_message(AgentMessage::CommandResult {
        command_id: command_id.clone(),
        branch_id: "candidate".into(),
        operation_id: None,
        operation: CommandOperation::Promote,
        status: CommandStatus::Failed,
        generation,
        reason: Some("promote rejected".into()),
    })
    .unwrap();
    assert_eq!(tree.branches["candidate"].status, BranchStatus::Discarding);
    assert_eq!(tree.authority_branch_id.as_deref(), Some("parent"));

    let commands = tree
        .handle_message(AgentMessage::CommandResult {
            command_id,
            branch_id: "candidate".into(),
            operation_id: None,
            operation: CommandOperation::Promote,
            status: CommandStatus::Applied,
            generation,
            reason: None,
        })
        .unwrap();

    assert!(commands.is_empty());
    assert_eq!(tree.branches["candidate"].status, BranchStatus::Discarding);
    assert_eq!(tree.authority_branch_id.as_deref(), Some("parent"));
    assert!(tree.authority_failure().is_none());
}

#[test]
fn candidate_completion_only_marks_readiness_and_cleanup_failure_is_not_cleaned() {
    let mut tree = tree();
    tree.branches.insert(
        "candidate".into(),
        BranchRecord {
            branch_id: "candidate".into(),
            parent_branch_id: None,
            role: BranchRole::Candidate,
            status: BranchStatus::Running,
            generation: 1,
            execution_position: Default::default(),
            chance_id: None,
            assumption_set: Vec::new(),
            pending_operations: Vec::new(),
            execution_state: Default::default(),
        },
    );
    tree.handle_message(AgentMessage::ActionStarted {
        task_id: tree.execution_id.clone(),
        branch_id: "candidate".into(),
        action_id: "inference".into(),
        action_kind: ActionKind::ModelInference,
        parent_action_id: None,
        name: "model".into(),
        arguments: Value::Null,
        checkpoint: None,
        timestamp_ms: 0,
    })
    .unwrap();
    tree.handle_message(AgentMessage::ActionFinished {
        task_id: tree.execution_id.clone(),
        branch_id: "candidate".into(),
        action_id: "inference".into(),
        action_kind: ActionKind::ModelInference,
        status: ActionStatus::Succeeded,
        result: Value::Null,
        evidence: None,
        timestamp_ms: 0,
    })
    .unwrap();
    assert_eq!(tree.branches["candidate"].status, BranchStatus::Finished);
    tree.branches.get_mut("candidate").unwrap().status = BranchStatus::Cleaning;
    let command = tree.make_cleanup_command("candidate").unwrap();
    let (command_id, generation) = match command {
        AgentCommand::Cleanup {
            command_id,
            generation,
            ..
        } => (command_id, generation),
        _ => unreachable!(),
    };
    tree.handle_message(AgentMessage::CommandResult {
        command_id,
        branch_id: "candidate".into(),
        operation_id: None,
        operation: CommandOperation::Cleanup,
        status: CommandStatus::Failed,
        generation,
        reason: Some("busy".into()),
    })
    .unwrap();
    assert_eq!(tree.branches["candidate"].status, BranchStatus::Cleaning);
    let command = tree.begin_cleanup("candidate").unwrap();
    let AgentCommand::Cleanup {
        command_id,
        generation,
        ..
    } = command
    else {
        unreachable!()
    };
    tree.handle_message(AgentMessage::CommandResult {
        command_id,
        branch_id: "candidate".into(),
        operation_id: None,
        operation: CommandOperation::Cleanup,
        status: CommandStatus::Applied,
        generation,
        reason: None,
    })
    .unwrap();
    assert_eq!(
        tree.branches["candidate"].execution_state.agent,
        AgentExecutionState::Released
    );
    // A late start fact must not resurrect a released Agent session.
    tree.handle_message(AgentMessage::BranchStarted {
        branch_id: "candidate".into(),
    })
    .unwrap();
    assert_eq!(
        tree.branches["candidate"].execution_state.agent,
        AgentExecutionState::Released
    );
    assert_eq!(
        tree.branches["candidate"].execution_state.runtime,
        RuntimeExecutionState::Unknown
    );
}
