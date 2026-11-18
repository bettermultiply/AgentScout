//! control plane 协调测试。

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
fn held_authority_prediction_failure_or_empty_releases_tool_and_model_once() {
    use crate::control_plane::agent_control::ActionKind;
    for kind in [ActionKind::ToolExecution, ActionKind::ModelInference] {
        for failure in [false, true] {
            let (mut tree, _root) = task();
            let mut message = waiting();
            if let AgentMessage::SpeculativeAdmissionRequest { action_kind, .. } = &mut message { *action_kind = kind; }
            tree.speculative_admission(message).unwrap();
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
            let actions = tree
                .complete_work(
                    predict,
                    if failure {
                        Err("predict failed".into())
                    } else {
                        Ok(ControlResult::Predictions(vec![]))
                    },
                )
                .unwrap();
            assert_eq!(actions.commands.iter().filter(|c| matches!(c, AgentCommand::ResolvePending { outcome: Some(value), .. } if value["phase"] == "execute")).count(), 1, "{kind:?}, failure={failure}");
            assert_eq!(tree.chances["chance-1"].authority, AuthorityState::Fetching);
            assert!(tree.progress(&RuntimeCapabilities::default()).unwrap().commands.is_empty());
            if kind == ActionKind::ModelInference {
                tree.handle_message(AgentMessage::ActionFinished {
                    task_id: String::new(),
                    branch_id: "authority".into(),
                    action_id: "op".into(),
                    action_kind: kind,
                    status: crate::control_plane::agent_control::ActionStatus::Succeeded,
                    result: json!({"tool_calls":[]}),
                    evidence: None,
                    timestamp_ms: 1,
                })
                .unwrap();
                assert_eq!(
                    tree.chances["chance-1"].authority,
                    AuthorityState::Arrived,
                    "fallback result must remain paired after predict failure"
                );
                let actions = tree.progress(&RuntimeCapabilities::default()).unwrap();
                assert!(actions.commands.iter().any(|c| matches!(c, AgentCommand::ResolvePending { outcome: Some(value), .. } if value["phase"] == "continue")));
                assert!(!tree.progress(&RuntimeCapabilities::default()).unwrap().commands.iter().any(|c| matches!(c, AgentCommand::ResolvePending { .. })));
            }
        }
    }
}

#[test]
fn held_authority_candidate_can_be_validated_without_parent_result() {
    use crate::control_plane::agent_control::{ActionKind, ActionStatus};
    for kind in [ActionKind::ToolExecution, ActionKind::ModelInference] {
        let (mut tree, _root) = task();
        let mut message = waiting();
        if let AgentMessage::SpeculativeAdmissionRequest { action_kind, .. } = &mut message { *action_kind = kind; }
        tree.speculative_admission(message).unwrap();
        tree.change_chance_stage("chance-1", ChanceStage::Running, "selected")
            .unwrap();
        let command = if kind == ActionKind::ModelInference {
            tree.start_model_candidate("chance-1", json!({"predict":"branch_model","model":"fake"}))
        } else {
            tree.start_candidate("chance-1", json!("guess"))
        }
        .unwrap();
        let AgentCommand::StartCandidate { branch_id, .. } = command else {
            unreachable!()
        };
        tree.handle_message(AgentMessage::BranchStarted {
            branch_id: branch_id.clone(),
        })
        .unwrap();
        if kind == ActionKind::ModelInference {
            assert!(
                !tree.candidate_due(&branch_id),
                "unfinished alternate model is not a Proposal"
            );
            tree.handle_message(AgentMessage::ActionStarted {
                task_id: String::new(),
                branch_id: branch_id.clone(),
                action_id: "op".into(),
                action_kind: kind,
                parent_action_id: None,
                name: "model".into(),
                arguments: json!({}),
                checkpoint: None,
                timestamp_ms: 1,
            })
            .unwrap();
            tree.handle_message(AgentMessage::ActionFinished {
                task_id: String::new(),
                branch_id: branch_id.clone(),
                action_id: "op".into(),
                action_kind: kind,
                status: ActionStatus::Succeeded,
                result: json!({"tool_calls":[]}),
                evidence: None,
                timestamp_ms: 2,
            })
            .unwrap();
        }
        let launch = tree.progress(&RuntimeCapabilities::default()).unwrap().operations.into_iter().find(|(_, work)| matches!(work, ControlOperation::Launch(_))).unwrap().0;
        tree.complete_work(launch, Ok(ControlResult::Launch(false))).unwrap();
        assert!(tree.candidate_due(&branch_id), "{kind:?} candidate stalled with authority held");
        tree.handle_message(AgentMessage::BranchFailed {
            branch_id,
            reason: "candidate execution failed".into(),
        }).unwrap();
        let actions = tree.progress(&RuntimeCapabilities::default()).unwrap();
        assert_eq!(actions.commands.iter().filter(|command| matches!(command,
            AgentCommand::ResolvePending { outcome: Some(value), .. }
            if value["phase"] == "execute")).count(), 1);
        assert!(!tree.progress(&RuntimeCapabilities::default()).unwrap().commands.iter()
            .any(|command| matches!(command, AgentCommand::ResolvePending { .. })));
    }
}

#[test]
fn held_authority_four_launch_paths_promote_or_fallback_once() {
    use crate::control_plane::agent_control::{ActionKind, ActionStatus};
    for kind in [ActionKind::ToolExecution, ActionKind::ModelInference] {
        for eager in [false, true] {
            for accepted in [false, true] {
                let (mut tree, _root) = task();
                let mut message = waiting();
                if let AgentMessage::SpeculativeAdmissionRequest { action_kind, .. } = &mut message { *action_kind = kind; }
                tree.speculative_admission(message).unwrap();
                tree.change_chance_stage("chance-1", ChanceStage::Running, "selected")
                    .unwrap();
                let command = if kind == ActionKind::ModelInference {
                    tree.start_model_candidate(
                        "chance-1",
                        json!({"predict":"branch_model","model":"fake"}),
                    )
                } else {
                    tree.start_candidate("chance-1", json!("guess"))
                }
                .unwrap();
                let AgentCommand::StartCandidate { branch_id, .. } = command else {
                    unreachable!()
                };
                tree.handle_message(AgentMessage::BranchStarted {
                    branch_id: branch_id.clone(),
                })
                .unwrap();
                if kind == ActionKind::ModelInference {
                    tree.handle_message(AgentMessage::ActionStarted {
                        task_id: String::new(),
                        branch_id: branch_id.clone(),
                        action_id: "op".into(),
                        action_kind: kind,
                        parent_action_id: None,
                        name: "model".into(),
                        arguments: json!({}),
                        checkpoint: None,
                        timestamp_ms: 1,
                    })
                    .unwrap();
                    tree.handle_message(AgentMessage::ActionFinished {
                        task_id: String::new(),
                        branch_id: branch_id.clone(),
                        action_id: "op".into(),
                        action_kind: kind,
                        status: ActionStatus::Succeeded,
                        result: json!({"tool_calls":[]}),
                        evidence: None,
                        timestamp_ms: 2,
                    })
                    .unwrap();
                }
                let launch = tree
                    .progress(&RuntimeCapabilities::default())
                    .unwrap()
                    .operations
                    .into_iter()
                    .find(|(_, work)| matches!(work, ControlOperation::Launch(_)))
                    .unwrap()
                    .0;
                let actions = tree
                    .complete_work(launch, Ok(ControlResult::Launch(eager)))
                    .unwrap();
                let executes = |actions: &ControlActions| {
                    actions.commands.iter().filter(|c| matches!(c, AgentCommand::ResolvePending { outcome: Some(value), .. } if value["phase"] == "execute")).count()
                };
                assert_eq!(executes(&actions), usize::from(eager));
                assert_eq!(tree.chances["chance-1"].authority, if eager { AuthorityState::Fetching } else { AuthorityState::NotRequested });
                if eager { tree.record_authority_outcome("chance-1", json!("real"), None).unwrap(); }
                assert!(tree.candidate_due(&branch_id));
                let mut actions = ControlActions::default();
                tree.schedule(ControlOperation::Validate(branch_id.clone()), &mut actions);
                let actions = tree
                    .complete_work(
                        actions.operations[0].0,
                        Ok(ControlResult::Disposition(
                            accepted,
                            if accepted {
                                "compatible"
                            } else {
                                "incompatible"
                            }
                            .into(),
                            None,
                            Some(3),
                        )),
                    )
                    .unwrap();
                if accepted {
                    let promotion = actions
                        .operations
                        .iter()
                        .find(|(_, work)| matches!(work, ControlOperation::Promote(..)))
                        .unwrap()
                        .0;
                    let actions = tree
                        .complete_work(promotion, Ok(ControlResult::Completed))
                        .unwrap();
                    let command = actions
                        .commands
                        .iter()
                        .find(|c| matches!(c, AgentCommand::Promote { .. }))
                        .unwrap();
                    let AgentCommand::Promote {
                        command_id,
                        generation,
                        ..
                    } = command
                    else {
                        unreachable!()
                    };
                    let commands = tree
                        .handle_message(AgentMessage::CommandResult {
                            branch_id: branch_id.clone(),
                            command_id: command_id.clone(),
                            generation: *generation,
                            operation: CommandOperation::Promote,
                            operation_id: None,
                            status: CommandStatus::Applied,
                            reason: None,
                        })
                        .unwrap();
                    assert_eq!(
                        tree.authority_branch_id.as_deref(),
                        Some(branch_id.as_str())
                    );
                    assert!(commands.iter().any(|c| matches!(c, AgentCommand::Discard { branch_id, .. } if branch_id == "authority")));
                    assert!(!commands.iter().any(|c| matches!(c, AgentCommand::ResolvePending { .. })), "Promote must cancel parent, never inject candidate result");
                } else {
                    let actions = tree.progress(&RuntimeCapabilities::default()).unwrap();
                    assert_eq!(executes(&actions), usize::from(!eager));
                    assert_eq!(executes(&tree.progress(&RuntimeCapabilities::default()).unwrap()), 0);
                }
            }
        }
    }
}

#[test]
fn consumed_work_keeps_first_fingerprint_and_reports_only_conflicts() {
    let (mut tree, _root) = task();
    let mut actions = ControlActions::default();
    tree.schedule(ControlOperation::Select(vec![]), &mut actions);
    let id = actions.operations[0].0;
    tree.complete_work(id, Ok(ControlResult::Selected(vec![]))).unwrap();
    let first = tree.completed_work[&id];
    tree.complete_work(id, Ok(ControlResult::Selected(vec![]))).unwrap();
    assert!(!tree.pending_records.iter().any(|r| r["reason"] == "conflicting complete_work feedback"));
    tree.complete_work(id, Err("late failure".into())).unwrap();
    assert_eq!(tree.completed_work[&id], first);
    assert!(tree.pending_records.iter().any(|r| r["reason"] == "conflicting complete_work feedback"));
    assert!(!tree.pending_work.contains_key(&id));
}

#[test]
fn select_result_cannot_select_a_chance_outside_original_request() {
    let (mut tree, _root) = task();
    tree.speculative_admission(waiting()).unwrap();
    let mut actions = ControlActions::default();
    tree.schedule(ControlOperation::Select(vec![]), &mut actions);
    let result = tree
        .complete_work(
            actions.operations[0].0,
            Ok(ControlResult::Selected(vec!["chance-1".into()])),
        )
        .unwrap();
    assert!(result.operations.is_empty());
    assert_eq!(tree.chances["chance-1"].stage, ChanceStage::Pending);
}

#[test]
fn conflicting_operation_feedback_preserves_first_terminal_state() {
    let (mut tree, _root) = task();
    tree.complete_operation("authority", Some("start-agent"), OperationStatus::Succeeded)
        .unwrap();
    tree.complete_operation("authority", Some("start-agent"), OperationStatus::Failed)
        .unwrap();
    assert_eq!(
        tree.branches["authority"].pending_operations[0].status,
        OperationStatus::Succeeded
    );
}

#[test]
fn rebalance_ignores_changed_branch_but_keeps_unrelated_result() {
    let (mut tree, _root) = task();
    tree.speculative_admission(waiting()).unwrap();
    let command = tree
        .start_candidate("chance-1", json!({"result": "x"}))
        .unwrap();
    let AgentCommand::StartCandidate { branch_id, .. } = command else {
        unreachable!()
    };
    tree.branches.get_mut(&branch_id).unwrap().status = BranchStatus::Running;
    let mut actions = ControlActions::default();
    tree.schedule(ControlOperation::Rebalance, &mut actions);
    // An unrelated Branch changes. The unchanged Candidate's result is valid.
    tree.branches.get_mut("authority").unwrap().generation += 1;
    let completed = tree
        .complete_work(
            actions.operations[0].0,
            Ok(ControlResult::Rebalance(vec![(
                branch_id.clone(),
                "pause".into(),
            )])),
        )
        .unwrap();
    assert_eq!(completed.commands.len(), 1);
    // A transition back to the original value still invalidates the decision.
    tree.schedule(ControlOperation::Rebalance, &mut actions);
    tree.change_status(
        &branch_id,
        BranchStatus::Blocked(BlockedReason::Rebalanced),
        "test pause",
    )
    .unwrap();
    tree.change_status(&branch_id, BranchStatus::Running, "test resume")
        .unwrap();
    let completed = tree
        .complete_work(
            actions.operations[1].0,
            Ok(ControlResult::Rebalance(vec![(
                branch_id.clone(),
                "pause".into(),
            )])),
        )
        .unwrap();
    assert!(completed.commands.is_empty());

    tree.schedule(ControlOperation::Rebalance, &mut actions);
    tree.branches.get_mut(&branch_id).unwrap().generation += 1;
    let completed = tree
        .complete_work(
            actions.operations[2].0,
            Ok(ControlResult::Rebalance(vec![(branch_id, "pause".into())])),
        )
        .unwrap();
    assert!(completed.commands.is_empty());
}

#[test]
fn missing_materialization_authority_is_visible_and_retryable() {
    for missing in ["environment", "branch", "authority"] {
        let (mut tree, _root) = task();
        tree.finish_task();
        match missing {
            "environment" => tree.branches.get_mut("authority").unwrap().environment_id = None,
            "branch" => {
                tree.branches.remove("authority");
            }
            _ => tree.authority_branch_id = None,
        }
        let mut lifecycle = task::Task::default();
        assert!(
            lifecycle
                .start_materialization(&tree, |_| Ok(()))
                .unwrap()
                .is_none()
        );
        let mut records = Vec::new();
        let (_, ready, result) = lifecycle
            .materialize(&mut tree, false, |v| {
                records.push(v);
                Ok(())
            })
            .unwrap()
            .unwrap();
        assert!(!ready);
        assert!(result.unwrap_err().contains("missing"));
        assert!(lifecycle.materialization_failed);
        assert!(
            tree.task_finishing(),
            "missing state cannot pretend publication succeeded"
        );
        assert_eq!(records[0]["message"], "materialization_failed");
        lifecycle.retry_materialization();
        tree.authority_branch_id = Some("authority".into());
        if missing == "branch" {
            continue;
        }
        tree.branches.get_mut("authority").unwrap().environment_id = Some("restored".into());
        assert_eq!(
            lifecycle.start_materialization(&tree, |_| Ok(())).unwrap(),
            Some("restored".into())
        );
    }
}

#[test]
fn materialization_is_single_flight_and_waits_for_abort_receipt_before_next_task() {
    use crate::control_plane::gateway::ui::Request;
    let (mut tree, root) = task();
    let mut session = session::Session::new(root.path(), None);
    tree.finish_task();
    let mut lifecycle = task::Task::default();
    let mut records = Vec::new();
    assert!(
        lifecycle
            .start_materialization(&tree, |v| {
                records.push(v);
                Ok(())
            })
            .unwrap()
            .is_some()
    );
    assert!(
        lifecycle
            .start_materialization(&tree, |_| Ok(()))
            .unwrap()
            .is_none()
    );
    let abort = lifecycle.cancel(&mut tree, false).unwrap();
    assert_eq!(abort.len(), 1);
    assert!(lifecycle.cancel(&mut tree, false).unwrap().is_empty());
    assert!(
        session
            .request(&mut tree, Request::Pause, 1)
            .unwrap()
            .commands
            .is_empty()
    );
    assert!(!tree.recovering());
    tree.queue_prompt("next task".into());
    assert!(tree.next_prompt().unwrap().is_none());
    // A Runtime result alone cannot release the task before Abort is applied.
    lifecycle.materialized(Err("publication failed".into()));
    assert!(
        lifecycle
            .materialize(&mut tree, false, |_| Ok(()))
            .unwrap()
            .is_none()
    );
    let AgentCommand::Abort {
        command_id,
        generation,
        ..
    } = &abort[0]
    else {
        panic!("Abort expected")
    };
    tree.handle_message(AgentMessage::TurnCompleted {
        branch_id: "authority".into(),
    })
    .unwrap();
    assert!(
        lifecycle
            .materialize(&mut tree, false, |_| Ok(()))
            .unwrap()
            .is_none()
    );
    tree.handle_message(AgentMessage::CommandResult {
        branch_id: "authority".into(),
        command_id: command_id.clone(),
        generation: *generation,
        operation: CommandOperation::Abort,
        operation_id: None,
        status: CommandStatus::Applied,
        reason: None,
    })
    .unwrap();
    assert!(
        lifecycle
            .materialize(&mut tree, false, |_| Ok(()))
            .unwrap()
            .unwrap()
            .2
            .is_err()
    );
    assert!(tree.task_finishing());
    assert!(
        lifecycle
            .start_materialization(&tree, |_| Ok(()))
            .unwrap()
            .is_none()
    );
    lifecycle.retry_materialization();
    assert!(
        lifecycle
            .start_materialization(&tree, |_| Ok(()))
            .unwrap()
            .is_some()
    );
    // Shutdown while that retry is in flight records intent, not another call.
    assert!(lifecycle.cancel(&mut tree, true).unwrap().is_empty());
    assert!(lifecycle.exit_requested);
    assert!(
        lifecycle
            .start_materialization(&tree, |_| Ok(()))
            .unwrap()
            .is_none()
    );
    assert_eq!(records.len(), 1);
}

#[test]
fn continue_command_is_not_a_chat_prompt_and_pause_cannot_block_termination() {
    use crate::control_plane::gateway::ui::Request;
    let (mut tree, root) = task();
    let mut session = session::Session::new(root.path(), None);
    session
        .request(
            &mut tree,
            Request::Prompt {
                prompt: "/continue".into(),
            },
            1,
        )
        .unwrap();
    assert_eq!(
        session.input_sequence(),
        0,
        "idle continue is not a user turn"
    );
    assert!(tree.next_prompt().unwrap().is_none());
    tree.continue_authority("work".into()).unwrap();
    session.request(&mut tree, Request::Interrupt, 2).unwrap();
    session.request(&mut tree, Request::Pause, 3).unwrap();
    assert!(
        !tree.recovering(),
        "pause must not block termination/materialization"
    );
}

#[test]
fn ordinary_prompt_is_queued_while_paused_until_continue() {
    use crate::control_plane::gateway::ui::Request;
    let (mut tree, root) = task();
    let mut session = session::Session::new(root.path(), None);
    session.request(&mut tree, Request::Pause, 1).unwrap();
    session
        .request(
            &mut tree,
            Request::Prompt {
                prompt: "queued".into(),
            },
            2,
        )
        .unwrap();
    assert!(tree.recovering());
    assert!(tree.next_prompt().unwrap().is_none());
    session.agent(&mut tree, &AgentMessage::RecoveryLoaded { error: None }).unwrap();
    let commands = session.request(&mut tree, Request::Continue, 3).unwrap();
    assert!(commands.commands.iter().any(|command| matches!(command, AgentCommand::Continue { .. })));
    assert!(matches!(tree.next_prompt().unwrap(), Some(AgentCommand::Prompt { prompt, .. }) if prompt == "queued"));
}

#[test]
fn repeated_interrupt_and_idle_continue_do_not_start_another_turn() {
    use crate::control_plane::gateway::ui::Request;
    let (mut tree, root) = task();
    let mut session = session::Session::new(root.path(), None);
    assert!(
        session
            .request(&mut tree, Request::Interrupt, 1)
            .unwrap()
            .commands
            .is_empty()
    );
    assert!(!tree.task_finishing());
    tree.continue_authority("work".into()).unwrap();
    assert!(
        !session
            .request(&mut tree, Request::Interrupt, 2)
            .unwrap()
            .commands
            .is_empty()
    );
    assert!(
        session
            .request(&mut tree, Request::Interrupt, 3)
            .unwrap()
            .commands
            .is_empty()
    );
    assert!(
        session
            .request(&mut tree, Request::Continue, 4)
            .unwrap()
            .commands
            .is_empty()
    );
    assert_eq!(session.input_sequence(), 0);
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
fn next_model_admission_does_not_expire_tool_validation() {
    let (mut tree, _root) = task();
    tree.speculative_admission(waiting()).unwrap();
    let AgentCommand::StartCandidate { branch_id, .. } = tree.start_candidate("chance-1", json!("guess")).unwrap() else { unreachable!() };
    let mut model = waiting();
    if let AgentMessage::SpeculativeAdmissionRequest { action_id, action_kind, name, .. } = &mut model {
        *action_id = "next-model".into();
        *action_kind = crate::control_plane::agent_control::ActionKind::ModelInference;
        *name = "model".into();
    }
    tree.speculative_admission(model).unwrap();
    assert!(!tree.authority_advanced_past(&tree.branches[&branch_id]));
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
    let result = SpeculationCoordinator
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
fn cancelled_creation_keeps_late_resource_for_cleanup() {
    let (mut tree, root) = task();
    let chance = tree.admit_chance("authority", "op", "tool", json!({}), None, None).unwrap();
    let command = tree.start_candidate(&chance, json!("guess")).unwrap();
    let AgentCommand::StartCandidate { branch_id: child, .. } = &command else { unreachable!() };
    let child = child.clone();
    let mut scheduled = ControlActions::default();
    tree.schedule(ControlOperation::Establish(command), &mut scheduled);
    let mut session = session::Session::new(root.path(), None);
    session
        .request(
            &mut tree,
            crate::control_plane::gateway::ui::Request::Shutdown,
            0,
        )
        .unwrap();
    let actions = tree
        .complete_work(
            scheduled.operations[0].0,
            Ok(ControlResult::Environment("late-resource".into(), None)),
        )
        .unwrap();
    assert_eq!(
        tree.branches[&child].environment_id.as_deref(),
        Some("late-resource")
    );
    assert!(
        !actions
            .commands
            .iter()
            .any(|c| matches!(c, AgentCommand::StartCandidate { .. }))
    );
    assert!(!tree.evictable_branches().contains(&child));
    let AgentCommand::Cleanup {
        command_id,
        generation,
        ..
    } = tree.begin_cleanup(&child).unwrap()
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
    let capabilities = RuntimeCapabilities {
        branch_environment: true,
        ..Default::default()
    };
    let progress = tree.progress(&capabilities).unwrap();
    let (terminate, _) = progress.operations.iter().find(|(_, op)| matches!(op, ControlOperation::Terminate(id) if id == &child)).expect("late resource must be terminated");
    let cleanup = tree.complete_work(*terminate, Ok(ControlResult::Completed)).unwrap();
    let (cleanup, _) = cleanup.operations.iter().find(|(_, op)| matches!(op, ControlOperation::Cleanup(id) if id == &child)).expect("terminated resource must be cleaned");
    tree.complete_work(*cleanup, Ok(ControlResult::Completed)).unwrap();
    assert_eq!(tree.branches[&child].execution_state.runtime, RuntimeExecutionState::Cleaned);
}

#[test]
fn failed_delivery_uses_branch_failure_and_ignores_completed_start() {
    let (mut tree, _root) = task();
    let chance = tree.admit_chance("authority", "op", "tool", json!({}), None, None).unwrap();
    let command = tree.start_candidate(&chance, json!("guess")).unwrap();
    let AgentCommand::StartCandidate { branch_id: child, .. } = &command else { unreachable!() };
    let child = child.clone();
    let actions = tree.delivery_failed(&command, "disconnected".into()).unwrap();
    assert!(!actions.is_empty());
    assert!(tree.branches[&child].execution_state.failure.is_some());
    let authority = AgentCommand::StartBranch {
        branch_id: "authority".into(),
        environment_id: None,
        execution_access: None,
        role: BranchRole::Authority,
        workspace: "/tmp/fake".into(),
        prompt: String::new(),
    };
    // task() already consumed BranchStarted: a delayed send timeout is not a new failure.
    assert!(tree.delivery_failed(&authority, "receipt lost".into()).unwrap().is_empty());
    assert!(tree.authority_failure().is_none());
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
                        execution_access: None,
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
    let result = SpeculationCoordinator
        .perform(
            &tree,
            &policy,
            &PolicyEngine::default(),
            &session,
            &capabilities,
            &ControlOperation::Establish(command),
        )
        .unwrap();
    assert!(matches!(&result, ControlResult::Environment(id, _) if id == "runtime-returned-child"));
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
    assert_eq!(calls.len(), 2);
    assert_eq!(
        Some(calls[0].0.as_str()),
        tree.branches["authority"].environment_id.as_deref()
    );
    assert_eq!(calls[0].1, Operation::BranchEnvironment { agent: None });
    assert_eq!(calls[1].0, "runtime-returned-child");
    assert_eq!(calls[1].1, Operation::BindExecution { branch_id: child });
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
