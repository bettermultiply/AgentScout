//! commands 测试。

use super::super::*;
use super::support::*;

#[test]
fn command_results_require_exact_pending_identity_and_are_consumed() {
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
    let stale = tree
        .handle_message(AgentMessage::CommandResult {
            command_id: command_id.clone(),
            branch_id: "candidate".into(),
            operation_id: None,
            operation: CommandOperation::Promote,
            status: CommandStatus::Applied,
            generation: generation + 1,
            reason: None,
        })
        .unwrap();
    assert!(stale.is_empty());
    assert_eq!(tree.authority_branch_id.as_deref(), Some("parent"));
    tree.handle_message(AgentMessage::CommandResult {
        command_id: command_id.clone(),
        branch_id: "candidate".into(),
        operation_id: None,
        operation: CommandOperation::Promote,
        status: CommandStatus::Applied,
        generation,
        reason: None,
    })
    .unwrap();
    let duplicate = tree
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
    assert!(duplicate.is_empty());
    assert_eq!(tree.authority_branch_id.as_deref(), Some("candidate"));
}
