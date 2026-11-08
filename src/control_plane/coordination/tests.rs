//! 控制面协调测试。

use super::*;

    
    use crate::control_plane::agent_control::{BranchRole, CommandOperation, CommandStatus};

    fn task() -> (SpeculativeTree, tempfile::TempDir) {
        let root = tempfile::tempdir().unwrap();
        let mut tree = SpeculativeTree::with_workspace("agent", root.path()).unwrap();
        tree.execution_id = String::new();
        tree.bind_local_policy("test-policy");
        tree.create_authority("authority", "").unwrap();
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
        }
    }

    #[test]
    fn interrupt_invalidates_a_prediction_already_in_flight() {
        let (mut tree, _root) = task();
        let predict = tree.speculative_admission(waiting()).unwrap().operations[0].0;
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
        let command = tree
            .start_candidate("authority", "op", "tool", json!("guess"), None)
            .unwrap();
        let AgentCommand::StartCandidate { branch_id, .. } = command else {
            unreachable!()
        };
        tree.handle_message(AgentMessage::BranchStarted {
            branch_id: branch_id.clone(),
        })
        .unwrap();
        tree.record_candidate_outcome("authority", "op", json!({"payload":"guess"}), None)
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
        })
        .unwrap();
        let actions = tree
            .complete_work(
                validation,
                Ok(ControlResult::Disposition(true, "compatible".into())),
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
    fn branch_cannot_be_archived_while_its_environment_is_still_being_created() {
        let (mut tree, _root) = task();
        let command = tree
            .start_candidate("authority", "op", "tool", json!("guess"), None)
            .unwrap();
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
