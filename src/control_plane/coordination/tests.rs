//! 控制面协调测试。

use super::*;

    
    use crate::control_plane::agent_control::{BranchRole, CommandOperation, CommandStatus};
    use crate::control_plane::policy::CompiledPolicy;
    use crate::control_plane::session::ControlServer;
    use crate::protocol::contract::{
        AdapterMessage, ControlMessage, EnvironmentState, OperationResult, ResultStatus,
        RuntimeCapabilities,
    };
    use crate::protocol::transport;
    use std::os::unix::net::UnixStream;

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
            timestamp_ms: 0,
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
            .start_candidate("authority", "op", "tool", json!("guess"), None, None)
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
            timestamp_ms: 0,
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
            .start_candidate("authority", "op", "tool", json!("guess"), None, None)
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
    fn firecracker_style_capabilities_route_establish_through_branch_environment() {
        let (mut tree, _root) = task();
        let command = tree
            .start_candidate("authority", "op", "tool", json!("guess"), None, None)
            .unwrap();
        let child = match &command {
            AgentCommand::StartCandidate { branch_id, .. } => branch_id.clone(),
            _ => unreachable!(),
        };
        let policy: CompiledPolicy = serde_json::from_value(json!({
            "version": 1,
            "python": "test",
            "files": {},
            "runtime": null,
            "decisions": {},
            "digest": "test"
        }))
        .unwrap();

        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("control.sock");
        let server = ControlServer::bind(&socket).unwrap();
        let recorded = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let client_recorded = recorded.clone();
        let client = std::thread::spawn(move || {
            let mut stream = UnixStream::connect(&socket).unwrap();
            loop {
                match transport::read_message::<_, ControlMessage>(&mut stream).unwrap() {
                    Some(ControlMessage::Request(request)) => {
                        client_recorded.lock().unwrap().push((
                            request.environment_id.clone(),
                            request.operation.clone(),
                        ));
                        let result = OperationResult {
                            request_id: request.request_id.clone(),
                            environment_id: request.environment_id.clone(),
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
                        transport::write_message(
                            &mut stream,
                            &AdapterMessage::Result(result),
                        )
                        .unwrap();
                    }
                    None => break,
                }
            }
        });
        let session = server.accept().unwrap();
        let capabilities = RuntimeCapabilities {
            branch_environment: true,
            shared_environment: false,
            ..RuntimeCapabilities::default()
        };
        let result = AgentController
            .perform(
                &tree,
                &policy,
                &session,
                &capabilities,
                &ControlOperation::Establish(command),
            )
            .unwrap();
        assert!(matches!(result, ControlResult::Completed));
        assert_eq!(
            *recorded.lock().unwrap(),
            vec![(
                "authority".to_string(),
                Operation::BranchEnvironment {
                    child_environment_id: child
                }
            )]
        );
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
