//! nested Chance 测试。

use super::super::*;
use super::support::*;
use crate::protocol::contract::RuntimeCapabilities;
use serde_json::json;

#[test]
fn nested_chance_waits_on_a_live_candidate_and_is_invalidated_with_it() {
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
    assert!(
        tree.accept_candidate(&nested_branch).is_err(),
        "an unconfirmed ancestor Proposal blocks commit eligibility"
    );

    // A matching nested Proposal is not discarded while the ancestor is still
    // unconfirmed; it waits for commit eligibility.
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
        tree.chances[&ancestor].stage,
        ChanceStage::Discarded(DiscardedReason::MatchRejected)
    );
    assert_eq!(
        tree.branches[&nested_branch].status,
        BranchStatus::Discarding
    );
}
