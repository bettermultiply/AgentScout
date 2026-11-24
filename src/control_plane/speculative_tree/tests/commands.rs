
use super::super::*;
use super::support::*;

#[test]
fn command_results_require_exact_pending_identity_and_are_consumed() {
    let mut tree = tree();
    tree.branches.insert(
        "parent".into(),
        BranchRecord {
            branch_id: "parent".into(),
            environment_id: Some("environment-parent".into()),
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
    tree.branches.insert(
        "candidate".into(),
        BranchRecord {
            branch_id: "candidate".into(),
            environment_id: Some("environment-candidate".into()),
            parent_branch_id: Some("parent".into()),
            role: BranchRole::Candidate,
            status: BranchStatus::Running,
            generation: 2,

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
#[test]
fn text_prompts_wait_for_authority_and_cached_commands_survive_authority_change() {
    use crate::control_plane::observation::ObservationContent;
    use serde_json::json;
    let mut tree = tree();
    tree.create_authority("authority", "").unwrap();
    let mut candidate = tree.branches["authority"].clone();
    candidate.branch_id = "candidate".into();
    candidate.role = BranchRole::Candidate;
    tree.branches.insert("candidate".into(), candidate);
    tree.drain_observations();
    tree.handle_message(AgentMessage::UiPromptRequested {
        branch_id: "candidate".into(),
        prompt_id: "question".into(),
        prompt: UiPrompt {
            kind: "editor".into(),
            title: "Edit".into(),
            message_text: None,
            options: vec![],
            placeholder: None,
            prefill: Some("one\ntwo".into()),
        },
    })
    .unwrap();
    assert!(tree.drain_observations().is_empty());
    assert!(
        tree.answer_ui_prompt("question".into(), json!("answer"), false)
            .is_err()
    );
    tree.handle_message(AgentMessage::CommandsListed {
        branch_id: "authority".into(),
        request_id: "startup".into(),
        commands: json!([{"name":"loaded"}]),
    })
    .unwrap();
    tree.change_authority("candidate").unwrap();
    assert!(tree.drain_observations().iter().any(|o| matches!(o, ObservationContent::UiPromptRequested { prompt_id, .. } if prompt_id == "question")));
    assert!(
        tree.answer_ui_prompt("question".into(), json!(true), false)
            .is_err()
    );
    assert!(
        matches!(tree.answer_ui_prompt("question".into(), json!("one\ntwo"), false).unwrap(), AgentCommand::UiPromptAnswer { answer, cancelled: false, .. } if answer == json!("one\ntwo"))
    );
    assert!(
        tree.answer_ui_prompt("question".into(), json!("late"), false)
            .is_err()
    );
    assert!(tree.get_commands("reconnect".into()).unwrap().is_none());
    assert!(tree.drain_observations().iter().any(|o| matches!(o, ObservationContent::CommandsListed { branch_id, commands, .. } if branch_id == "candidate" && commands[0]["name"] == "loaded")));
    tree.handle_message(AgentMessage::UiPromptRequested {
        branch_id: "candidate".into(),
        prompt_id: "cancel".into(),
        prompt: UiPrompt {
            kind: "input".into(),
            title: "Cancel".into(),
            message_text: None,
            options: vec![],
            placeholder: None,
            prefill: None,
        },
    })
    .unwrap();
    tree.change_authority("authority").unwrap();
    assert!(tree.drain_ui_commands().iter().any(|c| matches!(c, AgentCommand::UiPromptAnswer { prompt_id, cancelled: true, .. } if prompt_id == "cancel")));
    assert!(
        tree.answer_ui_prompt("cancel".into(), json!("late"), false)
            .is_err()
    );
}
