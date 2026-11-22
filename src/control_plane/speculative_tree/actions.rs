
use super::*;
use crate::protocol::contract::RuntimeCapabilities;
use serde_json::json;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolPending {
    pub chance_id: String,
    pub branch_id: String,
    pub operation_id: String,
    pub tool_name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControlOperation {
    Select(Vec<String>),
    Predict(ToolPending),
    Launch(ToolPending),
    Establish(AgentCommand),
    Validate(String),
    Dispose(String, String),
    SuspendAcceptance(String),
    ResumeAcceptance(String),
    Promote(String, Value),
    Terminate(String),
    Cleanup(String),
    Rebalance,
}

impl ControlOperation {
    pub fn references(&self, branch: &str) -> bool {
        self.branch() == Some(branch)
            || matches!(self, Self::Establish(AgentCommand::StartCandidate {parent_branch_id, ..}) if parent_branch_id == branch)
    }

    pub fn branch(&self) -> Option<&str> {
        match self {
            Self::Select(_) | Self::Rebalance => None,
            Self::Predict(tool) | Self::Launch(tool) => Some(&tool.branch_id),
            Self::Establish(command) => {
                let AgentCommand::StartCandidate { branch_id, .. } = command else {
                    unreachable!()
                };
                Some(branch_id)
            }
            Self::Validate(id) | Self::Dispose(id, _) | Self::SuspendAcceptance(id) | Self::ResumeAcceptance(id)
            | Self::Promote(id, _) | Self::Terminate(id) | Self::Cleanup(id) => {
                Some(id)
            }
        }
    }

    pub fn chance_id(&self) -> Option<&str> {
        match self {
            Self::Predict(tool) | Self::Launch(tool) => Some(&tool.chance_id),
            _ => None,
        }
    }
}

#[derive(Serialize)]
pub enum ControlResult {
    Selected(Vec<String>),
    Predictions(Vec<Value>),
    Abstain(Option<String>),
    BranchModel(Value),
    Launch(bool),
    Outcome(Value, Value),
    Disposition(bool, String, Option<String>, Option<u64>),
    AcceptanceDisposition(
        bool,
        String,
        Option<String>,
        Option<u64>,
        bool,
        Option<Value>,
    ),
    Acceptance(
        Option<crate::protocol::contract::Acceptance>,
        Option<crate::protocol::contract::ExecutionAccess>,
        Option<String>,
    ),
    Rebalance(Vec<(String, String)>),
    Completed,
    Environment(String, Option<crate::protocol::contract::ExecutionAccess>),
    EnvironmentFailed(String, String),
}

#[derive(Default)]
pub struct ControlActions {
    pub commands: Vec<AgentCommand>,
    pub operations: Vec<(u64, ControlOperation)>,
    pub records: Vec<Value>,
}

impl SpeculativeTree {
    pub(in crate::control_plane) fn schedule(
        &mut self,
        operation: ControlOperation,
        actions: &mut ControlActions,
    ) {
        let id = self.next_work;
        self.next_work += 1;
        if matches!(
            operation,
            ControlOperation::Select(_)
                | ControlOperation::Validate(_)
                | ControlOperation::Dispose(_, _)
                | ControlOperation::Rebalance
                | ControlOperation::Predict(_)
                | ControlOperation::Launch(_)
        ) {
            let context = self
                .branches
                .keys()
                .filter(|branch| {
                    matches!(operation, ControlOperation::Launch(_))
                        && (self.authority_branch_id.as_ref() == Some(*branch)
                            || self.pending_candidates().contains(branch))
                        || matches!(&operation, ControlOperation::Validate(candidate) | ControlOperation::Dispose(candidate, _)
                            if self.chance_parent(&self.branches[candidate]) == Some(branch.as_str()))
                        || operation
                            .branch()
                            .is_none_or(|target| target == branch.as_str())
                })
                .map(|branch| {
                    (
                        branch.clone(),
                        self.policy_branch_context(branch, &operation),
                    )
                })
                .collect();
            self.policy_dependencies
                .insert(id, self.policy_dependencies_for(&operation));
            self.policy_work_context.insert(id, context);
        }
        self.pending_work.insert(id, operation.clone());
        actions.operations.push((id, operation));
    }

    fn discard_expired_validation(
        &mut self,
        work: &ControlOperation,
        actions: &mut ControlActions,
    ) -> io::Result<()> {
        if let ControlOperation::Launch(tool) = work {
            self.chance_mut(&tool.chance_id)?.launch_epoch = None;
        }
        if let ControlOperation::Validate(candidate) = work
            && self
                .branches
                .get(candidate)
                .is_some_and(|branch| self.authority_advanced_past(branch))
        {
            actions.commands.extend(self.discard_candidate(
                candidate,
                DiscardedReason::MatchRejected,
                "Validation waiting point expired",
            )?);
        }
        Ok(())
    }

    fn policy_dependencies_for(&self, operation: &ControlOperation) -> Value {
        match operation {
            ControlOperation::Predict(_) => {
                json!({"capabilities": self.capability_view, "revision": self.capability_revision})
            }
            ControlOperation::Launch(tool) => {
                json!({"parent": tool.branch_id, "candidates": self.pending_candidates()})
            }
            ControlOperation::Validate(_) | ControlOperation::Dispose(_, _) => json!({"authority": self.authority_branch_id}),
            _ => Value::Null,
        }
    }

    fn policy_branch_context(&self, id: &str, operation: &ControlOperation) -> Value {
        let branch = &self.branches[id];
        let lifecycle = json!({"revision":self.policy_revisions.get(id),"generation":branch.generation,"status":branch.status,"role":branch.role});
        match operation {
            ControlOperation::Select(ids) => {
                return json!({"lifecycle":lifecycle,"chances":ids.iter().filter_map(|key|self.chances.get(key)).filter(|chance|chance.parent_branch_id==id).map(|chance|json!({"chance":chance,"revision":self.policy_chance_revisions.get(&chance.chance_id)})).collect::<Vec<_>>()});
            }
            ControlOperation::Predict(tool) => {
                return json!({"generation":branch.generation,"assumptions":branch.assumption_set,"chance":self.chances.get(&tool.chance_id).map(|chance|json!({"id":chance.chance_id,"kind":chance.speculation_kind,"stage":chance.stage,"revision":self.policy_chance_revisions.get(&tool.chance_id).map(|r|r.1)}))});
            }
            ControlOperation::Launch(_) => return json!(branch),
            ControlOperation::Rebalance => {
                return json!({"lifecycle":lifecycle,"chance_id":branch.chance_id});
            }
            _ => {}
        }
        json!({"revision": self.policy_revisions.get(id), "record_revision": self.policy_record_revisions.get(id),
            "chance_revisions": self.chances.values().filter(|chance| branch.chance_id.as_ref() == Some(&chance.chance_id)).map(|chance| self.policy_chance_revisions.get(&chance.chance_id).map(|revisions| revisions.1)).collect::<Vec<_>>(),
            "branch": branch,
            "chances": self.chances.values().filter(|chance| chance.parent_branch_id == id || branch.chance_id.as_ref() == Some(&chance.chance_id)).map(|chance| {
                let mut chance = chance.clone();
                chance.launch_epoch = None;
                chance
            }).collect::<Vec<_>>(),
            "usage": self.model_usage.get(id), "facts": self.action_facts.get(id),
            "generation": branch.generation, "status": branch.status,
            "position": branch.execution_position, "assumptions": branch.assumption_set,
            "proposal": self.branch_proposal(id)})
    }

    pub fn speculative_admission(&mut self, message: AgentMessage) -> io::Result<ControlActions> {
        let AgentMessage::SpeculativeAdmissionRequest {
            branch_id,
            policy_digest,
            action_id: operation_id,
            name: tool_name,
            arguments,
            checkpoint,
            task_id,
            action_kind,
            parent_action_id,
            timestamp_ms,
            fast_start,
        } = message
        else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "expected speculative admission request",
            ));
        };
        if task_id != self.execution_id {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "action task does not match tree",
            ));
        }
        if self.local_policy_digest.as_deref() != Some(policy_digest.as_str()) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "speculative admission does not use the delivered local Policy",
            ));
        }
        self.register_action(
            &branch_id,
            operation_id.clone(),
            action_kind,
            tool_name.clone(),
            parent_action_id,
        )?;
        let mut actions = ControlActions::default();
        if self.branches[&branch_id].status == BranchStatus::Promoting {
            // Admission can cross the acceptance pause in transit. The Agent
            // still waits for acceptance before executing this action.
            actions.commands.push(self.resolve_pending(
                &branch_id, &operation_id, json!({"phase":"execute", "speculation_skipped":true}),
            )?);
            return Ok(actions);
        }
        let branch_live = self.branches.get(&branch_id).is_some_and(|branch| {
            branch.status.is_live_authority() || branch.status.is_live_candidate()
        });
        if self.stopping
            || !branch_live
            || self.branches[&branch_id]
                .pending_operations
                .iter()
                .any(|op| op.operation_id == operation_id)
        {
            return Ok(actions);
        }
        let local_id = if let Some(fast) = &fast_start {
            let id = fast["chance_id"].as_str().filter(|id| {
                id.strip_prefix("chance-local-").is_some_and(|suffix| {
                    suffix.len() == 32 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
            }).ok_or_else(|| invalid_input("invalid local Chance identity"))?;
            if action_kind != ActionKind::ToolExecution || checkpoint.is_none()
                || fast.get("prediction").is_none() || self.chances.contains_key(id)
                || self.authority_branch_id.as_deref() != Some(&branch_id)
                || !self.pending_candidates().is_empty() || self.acceptance_in_flight()
            {
                return Err(invalid_input("local speculation crossed a conflicting action"));
            }
            Some(id.to_owned())
        } else {
            None
        };
        self.mark_operation_requested(&branch_id, &operation_id, OperationKind::Tool)?;
        let id = self.admit_chance_with_id(
            &branch_id,
            &operation_id,
            &tool_name,
            arguments,
            checkpoint,
            Some(timestamp_ms),
            local_id,
        )?;
        if let Some(fast) = fast_start {
            self.change_chance_stage(&id, ChanceStage::Running, "Selected by Agent Adapter")?;
            self.change_authority_state(&id, AuthorityState::Fetching)?;
            self.complete_predict(self.tool_pending(&id)?, Ok(ControlResult::Predictions(vec![json!({"payload":fast["prediction"], "provenance":fast["provenance"]})])), &mut actions)?;
        }
        Ok(actions)
    }

    pub fn complete_work(
        &mut self,
        id: u64,
        result: Result<ControlResult, String>,
    ) -> io::Result<ControlActions> {
        use sha2::{Digest, Sha256};
        let fingerprint: [u8; 32] =
            Sha256::digest(serde_json::to_vec(&result).expect("control results serialize")).into();
        let mut actions = ControlActions::default();
        let Some(work) = self.pending_work.remove(&id) else {
            if self
                .completed_work
                .get(&id)
                .is_some_and(|first| first != &fingerprint)
            {
                self.pending_records.push(json!({
                    "message": "feedback_conflict", "work_id": id,
                    "reason": "conflicting complete_work feedback"
                }));
            }
            return Ok(actions);
        };
        self.completed_work.insert(id, fingerprint);
        while self.completed_work.len() > 256 {
            self.completed_work.pop_first();
        }
        let evaluated_work = work.clone();
        let context = self.policy_work_context.remove(&id);
        if self
            .policy_dependencies
            .remove(&id)
            .is_some_and(|original| original != self.policy_dependencies_for(&work))
        {
            self.discard_expired_validation(&work, &mut actions)?;
            return Ok(actions);
        }
        if matches!(
            work,
            ControlOperation::Launch(_) | ControlOperation::Validate(_) | ControlOperation::Dispose(_, _)
        ) && context.as_ref().is_some_and(|context| {
            context.iter().any(|(branch, original)| {
                !self.branches.contains_key(branch)
                    || *original != self.policy_branch_context(branch, &evaluated_work)
            })
        }) {
            self.discard_expired_validation(&work, &mut actions)?;
            return Ok(actions);
        }
        if let Some(branch) = work.branch()
            && context.as_ref().is_some_and(|context| {
                context.get(branch).is_none_or(|original| {
                    !self.branches.contains_key(branch)
                        || *original != self.policy_branch_context(branch, &evaluated_work)
                })
            })
        {
            self.discard_expired_validation(&work, &mut actions)?;
            return Ok(actions);
        }
        let result = match (result, context.as_ref()) {
            (Ok(ControlResult::Rebalance(entries)), Some(context)) => Ok(ControlResult::Rebalance(
                entries
                    .into_iter()
                    .filter(|(branch, _)| {
                        self.branches.contains_key(branch)
                            && context.get(branch).is_some_and(|original| {
                                *original == self.policy_branch_context(branch, &evaluated_work)
                            })
                    })
                    .collect(),
            )),
            (Ok(ControlResult::Selected(ids)), Some(context)) => Ok(ControlResult::Selected(
                ids.into_iter()
                    .filter(|id| {
                        self.chances.get(id).is_some_and(|chance| {
                            context
                                .get(&chance.parent_branch_id)
                                .is_some_and(|original| {
                                    *original
                                        == self.policy_branch_context(
                                            &chance.parent_branch_id,
                                            &evaluated_work,
                                        )
                                })
                        })
                    })
                    .collect(),
            )),
            (result, _) => result,
        };
        let branch = work.branch().map(str::to_string);
        match work {
            ControlOperation::Select(mut ids) => {
                if let Some(context) = &context {
                    ids.retain(|id| {
                        self.chances.get(id).is_some_and(|chance| {
                            context
                                .get(&chance.parent_branch_id)
                                .is_some_and(|original| {
                                    *original
                                        == self.policy_branch_context(
                                            &chance.parent_branch_id,
                                            &evaluated_work,
                                        )
                                })
                        })
                    });
                }
                self.complete_select(ids, result, &mut actions)?;
            }
            ControlOperation::Predict(tool) => {
                self.complete_predict(tool, result, &mut actions)?;
            }
            ControlOperation::Launch(tool) => {
                self.complete_launch(tool, result, &mut actions)?;
            }
            ControlOperation::Rebalance => {
                self.complete_rebalance(result, &mut actions)?;
            }
            ControlOperation::Establish(command) => {
                self.complete_establish(command, result, &mut actions, &branch.unwrap())?;
            }
            ControlOperation::Validate(candidate) => {
                self.complete_validate(id, candidate, None, result, &mut actions)?;
            }
            ControlOperation::Dispose(candidate, resolution) => {
                self.complete_validate(id, candidate, Some(&resolution), result, &mut actions)?;
            }
            ControlOperation::SuspendAcceptance(candidate) => {
                if result.is_ok() {
                    if let Some((id, record)) = self.pending_acceptance.as_mut()
                        && *id == candidate
                    {
                        record["suspended"] = json!(true);
                        if !self.stopping {
                            if self.acceptance_target(self.branch(&candidate)?).is_some() {
                                actions.commands.push(self.make_prepare_acceptance(&candidate, true)?);
                            } else {
                                actions.commands.extend(self.cancel_acceptance(&candidate, "Acceptance target is no longer eligible")?);
                            }
                        }
                    } else {
                        return Err(io::Error::other("acceptance target paused without a pending acceptance"));
                    }
                } else if let Err(reason) = result {
                    actions.commands.extend(self.cancel_acceptance(&candidate, &reason)?);
                }
            }
            ControlOperation::ResumeAcceptance(parent) => {
                result.map_err(io::Error::other)?;
                if !self.stopping {
                    actions.commands.push(self.make_cancel_acceptance(&parent)?);
                }
            }
            ControlOperation::Promote(candidate, record) => {
                self.complete_promote(candidate, record, result, &mut actions)?;
            }
            work @ (ControlOperation::Terminate(_) | ControlOperation::Cleanup(_)) => {
                self.complete_terminate_or_cleanup(work, result, &mut actions, &branch.unwrap())?;
            }
        }
        Ok(actions)
    }

    fn tool_pending(&self, chance_id: &str) -> io::Result<ToolPending> {
        let chance = self.chance(chance_id)?;
        Ok(ToolPending {
            chance_id: chance.chance_id.clone(),
            branch_id: chance.parent_branch_id.clone(),
            operation_id: chance.operation_id.clone(),
            tool_name: chance.tool_name.clone(),
            arguments: chance.arguments.clone(),
        })
    }

    fn chance_is(&self, chance_id: &str, stage: &ChanceStage) -> bool {
        self.chances
            .get(chance_id)
            .is_some_and(|chance| &chance.stage == stage && self.chance_waiting_live(chance))
    }

    /// Unselected Chances stay Pending while the authority fetches the real tool result.
    fn complete_select(
        &mut self,
        ids: Vec<String>,
        result: Result<ControlResult, String>,
        actions: &mut ControlActions,
    ) -> io::Result<()> {
        let selected = match result {
            Ok(ControlResult::Selected(selected)) => selected,
            Err(error) => {
                for id in ids {
                    if self.chance_is(&id, &ChanceStage::Pending) {
                        actions.records.push(
                            json!({"message":"policy_error", "chance_id":id, "reason":error}),
                        );
                        self.change_chance_stage(
                            &id,
                            ChanceStage::Discarded(DiscardedReason::Failed),
                            "select failed",
                        )?;
                        self.authorize_authority(&id, actions)?;
                    }
                }
                return Ok(());
            }
            _ => unreachable!(),
        };
        for id in &selected {
            if ids.contains(id) && self.chance_is(id, &ChanceStage::Pending) {
                self.change_chance_stage(id, ChanceStage::Running, "Selected")?;
                let tool = self.tool_pending(id)?;
                self.schedule(ControlOperation::Predict(tool), actions);
            }
        }
        for id in ids.iter().filter(|id| !selected.contains(id)) {
            if self.chance_is(id, &ChanceStage::Pending) {
                self.authorize_authority(id, actions)?;
            }
        }
        Ok(())
    }

    fn complete_predict(
        &mut self,
        tool: ToolPending,
        result: Result<ControlResult, String>,
        actions: &mut ControlActions,
    ) -> io::Result<()> {
        let chance_id = tool.chance_id.as_str();
        if !self.chance_is(chance_id, &ChanceStage::Running) {
            return Ok(());
        }
        match result {
            Ok(ControlResult::BranchModel(prediction)) => {
                let command = self.start_model_candidate(chance_id, prediction)?;
                if let AgentCommand::StartCandidate { branch_id, .. } = &command {
                    actions.records.push(json!({"message":"candidate_created","branch_id":branch_id,"parent":tool.branch_id,"chance_id":chance_id,"fork_at":self.branches[branch_id].execution_position.transcript_index}));
                }
                self.schedule(ControlOperation::Establish(command), actions);
            }
            Ok(ControlResult::Abstain(reason)) => {
                self.change_chance_stage(chance_id, ChanceStage::Discarded(DiscardedReason::NotSelected),
                    reason.as_deref().unwrap_or("Policy abstained"))?;
                self.authorize_authority(chance_id, actions)?;
            }
            Ok(ControlResult::Predictions(predictions)) => {
                for prediction in predictions {
                    let command = self.start_candidate_with_provenance(chance_id,
                        prediction["payload"].clone(), prediction.get("provenance").cloned())?;
                    let child = match &command {
                        AgentCommand::StartCandidate { branch_id, .. } => branch_id.clone(),
                        _ => unreachable!(),
                    };
                    actions.records.push(json!({
                        "message": "candidate_created",
                        "branch_id": child,
                        "parent": tool.branch_id,
                        "chance_id": chance_id,
                        "fork_at": self.branches[&child].execution_position.transcript_index
                    }));
                    if !chance_id.starts_with("chance-local-") {
                        self.schedule(ControlOperation::Establish(command), actions);
                    }
                }
            }
            Err(error) => {
                actions
                    .records
                    .push(json!({"message":"policy_error", "chance_id":chance_id, "reason":error}));
                self.change_chance_stage(
                    chance_id,
                    ChanceStage::Discarded(DiscardedReason::Failed),
                    "predict failed",
                )?;
                self.authorize_authority(chance_id, actions)?;
            }
            _ => unreachable!(),
        }
        Ok(())
    }

    fn complete_launch(
        &mut self,
        tool: ToolPending,
        result: Result<ControlResult, String>,
        actions: &mut ControlActions,
    ) -> io::Result<()> {
        let chance_id = tool.chance_id.as_str();
        let still_waiting = self.chance_is(chance_id, &ChanceStage::Running)
            && self.chance(chance_id)?.authority == AuthorityState::NotRequested;
        match result {
            Ok(ControlResult::Launch(true)) if still_waiting => {
                self.authorize_authority(chance_id, actions)?;
            }
            Ok(ControlResult::Launch(_)) => {}
            Err(error) => actions
                .records
                .push(json!({"message":"policy_error", "chance_id":chance_id, "reason":error})),
            _ => unreachable!(),
        }
        Ok(())
    }

    fn authorize_authority(&mut self, id: &str, actions: &mut ControlActions) -> io::Result<()> {
        self.change_authority_state(id, AuthorityState::Fetching)?;
        let chance = self.chance(id)?.clone();
        actions.commands.push(self.resolve_pending(
            &chance.parent_branch_id,
            &chance.operation_id,
            json!({"phase":"execute"}),
        )?);
        Ok(())
    }

    /// `Blocked(Rebalanced)` requires an adapter hold receipt, even after a pause command.
    fn complete_rebalance(
        &mut self,
        result: Result<ControlResult, String>,
        actions: &mut ControlActions,
    ) -> io::Result<()> {
        let rebalance = match result {
            Ok(ControlResult::Rebalance(rebalance)) => rebalance,
            Err(error) => {
                actions.records.push(
                    json!({"message":"policy_error", "decision":"rebalance", "reason":error}),
                );
                return Ok(());
            }
            _ => unreachable!(),
        };
        if self.stopping {
            return Ok(());
        }
        for (branch_id, action) in rebalance {
            let Some(status) = self.branches.get(&branch_id).map(|branch| branch.status) else {
                continue;
            };
            match (action.as_str(), status) {
                ("pause", BranchStatus::Running) => {
                    actions.commands.push(AgentCommand::Pause { branch_id })
                }
                ("resume", BranchStatus::Blocked(BlockedReason::Rebalanced)) => {
                    actions.commands.push(AgentCommand::Resume { branch_id })
                }
                ("preempt", status) if status.is_live_candidate() => {
                    if !self.pending_work.values().any(|work| work.branch() == Some(&branch_id)) {
                        self.schedule(ControlOperation::Dispose(branch_id, "preempted".into()), actions);
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn complete_establish(
        &mut self,
        mut command: AgentCommand,
        result: Result<ControlResult, String>,
        actions: &mut ControlActions,
        branch: &str,
    ) -> io::Result<()> {
        let result = match result {
            Ok(ControlResult::EnvironmentFailed(environment_id, reason)) => {
                self.bind_environment(branch, environment_id)?;
                Err(reason)
            }
            other => other,
        };
        match result {
            Ok(ControlResult::Environment(environment_id, access)) => {
                self.bind_environment(branch, environment_id.clone())?;
                if let AgentCommand::StartCandidate {
                    environment_id: target,
                    execution_access,
                    ..
                } = &mut command
                {
                    *target = Some(environment_id.clone());
                    *execution_access = access;
                }
                self.record_runtime_state(branch, RuntimeExecutionState::Ready)?;
                if !self.stopping && branch.starts_with("candidate-chance-local-")
                    && self.branches[branch].status.is_live_candidate()
                {
                    if let AgentCommand::StartCandidate { operation_id, execution_access, .. } = command {
                        actions.commands.push(self.resolve_pending(branch, &operation_id,
                            json!({"phase":"bind_environment", "environment_id":environment_id, "execution_access":execution_access}))?);
                    }
                } else if !self.stopping && self.branches[branch].status == BranchStatus::Created {
                    actions.commands.push(command);
                }
            }
            Err(reason) => {
                self.record_runtime_state(branch, RuntimeExecutionState::Failed)?;
                actions.records.push(json!({"message":"runtime_operation_failed","branch_id":branch,"reason":reason}));
                actions.commands.extend(self.discard_candidate(
                    branch,
                    DiscardedReason::Failed,
                    &format!("Runtime environment establishment failed: {reason}"),
                )?);
            }
            _ => unreachable!(),
        }
        Ok(())
    }

    pub(super) fn complete_tool_outcome(
        &mut self,
        tool: ToolPending,
        result: Result<ControlResult, String>,
        actions: &mut ControlActions,
    ) -> io::Result<()> {
        let ToolPending {
            chance_id,
            branch_id,
            operation_id,
            ..
        } = tool;
        let live = self
            .chances
            .get(&chance_id)
            .is_some_and(|chance| self.chance_waiting_live(chance));
        let (outcome, evidence) = match result {
            Ok(ControlResult::Outcome(outcome, evidence)) => (outcome, evidence),
            Err(error) => {
                self.change_authority_state(&chance_id, AuthorityState::Unavailable)?;
                self.change_chance_stage(
                    &chance_id,
                    ChanceStage::Discarded(DiscardedReason::Failed),
                    "Authority value unavailable",
                )?;
                self.discard_chance_candidates(&chance_id, DiscardedReason::Failed,
                    "Authority value unavailable", actions)?;
                actions.records.push(json!({"message":"authority_tool_failed", "chance_id":chance_id, "reason":error}));
                return self.release_tool_continuation(&chance_id, actions);
            }
            _ => unreachable!(),
        };
        self.record_authority_outcome(&chance_id, outcome.clone(), Some(evidence.clone()))?;
        self.record_public_nle_map_action(&branch_id, &operation_id, &evidence)?;
        // Tool Candidates already own a Proposal, so they can run through validation.
        let chance = self.chance_mut(&chance_id)?;
        chance.tool_continuation_pending = live;
        if !chance.stage.is_terminal() && chance.branch_ids.is_empty() {
            self.change_chance_stage(
                &chance_id,
                ChanceStage::Discarded(DiscardedReason::NotSelected),
                "Authority value arrived without a candidate",
            )?;
        } else if !self.chance(&chance_id)?.stage.is_terminal() {
            self.change_chance_stage(
                &chance_id,
                ChanceStage::Resolving,
                "Authority value arrived",
            )?;
        }
        self.complete_operation(&branch_id, Some(&operation_id), OperationStatus::Succeeded)?;
        if live {
            if self.branch(&branch_id)?.status
                == BranchStatus::Blocked(BlockedReason::ExtentReached)
            {
                self.change_status(
                    &branch_id,
                    BranchStatus::Running,
                    "Authority value delivered",
                )?;
            }
        }
        self.release_tool_continuation(&chance_id, actions)
    }

    pub(super) fn discard_chance_candidates(
        &mut self, id: &str, why: DiscardedReason, reason: &str, actions: &mut ControlActions,
    ) -> io::Result<()> {
        for branch in self.chance(id)?.branch_ids.clone() {
            if self.branches.get(&branch).is_some_and(|b| b.status.is_live_candidate()) {
                actions.commands.extend(self.discard_candidate(&branch, why, reason)?);
            }
        }
        Ok(())
    }

    pub(super) fn release_tool_continuation(&mut self, id: &str, actions: &mut ControlActions) -> io::Result<()> {
        let chance = self.chance(id)?;
        if !chance.tool_continuation_pending {
            return Ok(());
        }
        let parent = chance.parent_branch_id.clone();
        let operation = chance.operation_id.clone();
        let parent_retired = self.branch(&parent)?.status.is_discarding_or_later();
        if parent_retired || chance.stage.is_terminal() {
            if !parent_retired {
                actions.commands.push(self.resolve_pending(
                    &parent,
                    &operation,
                    json!({"phase":"continue"}),
                )?);
            }
            self.chance_mut(id)?.tool_continuation_pending = false;
        }
        Ok(())
    }

    fn establish_local_candidate(&mut self, branch: &str, actions: &mut ControlActions) -> io::Result<()> {
        let prediction = self.branch_proposal(branch).ok_or_else(|| invalid_input("missing local Proposal"))?.payload.clone();
        let command = self.candidate_start_command(branch, prediction)?;
        self.record_runtime_state(branch, RuntimeExecutionState::EnvironmentRequested)?;
        self.schedule(ControlOperation::Establish(command), actions);
        Ok(())
    }

    fn complete_validate(
        &mut self,
        validation_id: u64,
        candidate: String,
        resolution: Option<&str>,
        result: Result<ControlResult, String>,
        actions: &mut ControlActions,
    ) -> io::Result<()> {
        if self.stopping || match resolution {
            None => !self.candidate_due(&candidate),
            Some(_) => !self.branch(&candidate)?.status.is_live_candidate(),
        } {
            return Ok(());
        }
        let (compatible, verdict, notification, validated_at_ms, transfer, part, error) =
            match result {
                Ok(ControlResult::AcceptanceDisposition(
                    compatible,
                    verdict,
                    notification,
                    validated_at_ms,
                    full,
                    part,
                )) => (
                    compatible,
                    verdict,
                    notification,
                    validated_at_ms,
                    Some(full),
                    part,
                    None,
                ),
                Ok(ControlResult::Disposition(
                    compatible,
                    verdict,
                    notification,
                    validated_at_ms,
                )) => (
                    compatible,
                    verdict,
                    notification,
                    validated_at_ms,
                    None,
                    None,
                    None,
                ),
                Err(error) => (false, "error".into(), None, None, None, None, Some(error)),
                _ => unreachable!(),
            };
        let proposal = self.branch_proposal(&candidate).cloned();
        let chance_id = self
            .branch(&candidate)?
            .chance_id
            .clone()
            .ok_or_else(|| io::Error::other("candidate Chance is missing"))?;
        let unbound_local = candidate.starts_with("candidate-chance-local-") && self.branch(&candidate)?.environment_id.is_none();
        let salvage = part.as_ref().is_some_and(|part| part["intent"] == "Salvage");
        if verdict == "wait" || (verdict == "indeterminate" && !salvage) {
            return Ok(());
        }
        if proposal.is_none() {
            let record = json!({
                "branch_id": candidate,
                "hypothesis": Value::Null,
                "verdict": "incompatible",
                "policy_error": Value::Null,
                "disposition": "Discard"
            });
            actions.commands.extend(self.discard_candidate(
                &candidate,
                DiscardedReason::Failed,
                "Candidate did not produce a Proposal before validation",
            )?);
            actions.records.push(record);
            return Ok(());
        }
        if resolution.is_none() && error.is_none()
            && let Some(proposal) = &proposal
        {
            self.record_validation(
                &chance_id,
                &proposal.proposal_id,
                validation_id,
                validated_at_ms,
            );
        }
        let record = json!({"branch_id":candidate, "hypothesis":proposal.as_ref().map(|p|&p.payload), "verdict":verdict, "policy_error":error});
        if self.can_accept(&candidate, compatible || (salvage
            && !self.ancestor_assumptions_unresolved(self.branch(&candidate)?)), transfer.is_some()) {
            if unbound_local && transfer == Some(false) {
                self.establish_local_candidate(&candidate, actions)?;
                return Ok(());
            }
            let previous_status = self.branch(&candidate)?.status;
            if resolution.is_none() {
                self.chance_mut(&chance_id)?.validation_id = Some(validation_id);
            }
            self.record_runtime_state(&candidate, RuntimeExecutionState::PromotionRequested)?;
            self.change_status(
                &candidate,
                BranchStatus::Promoting,
                if salvage { "Runtime salvage requested" } else { "Runtime acceptance requested" },
            )?;
            let mut record = record;
            if let Some(notification) = notification {
                record["notification"] = json!(notification);
            }
            if unbound_local {
                let mut command = self.make_promote_command(&candidate, record["notification"].as_str().map(str::to_owned))?;
                if let AgentCommand::Promote { reuse_parent_environment, .. } = &mut command {
                    *reuse_parent_environment = true;
                }
                self.record_runtime_state(&candidate, RuntimeExecutionState::Promoted)?;
                record["disposition"] = json!("Promote");
                record["outcome"] = json!("full");
                self.pending_acceptance = Some((candidate, record.clone()));
                actions.records.push(record);
                actions.commands.push(command);
            } else if let Some(full) = transfer {
                record["previous_status"] = json!(previous_status);
                record["full"] = json!(full);
                record["part"] = part.unwrap_or(Value::Null);
                self.pending_acceptance = Some((candidate.clone(), record));
                actions
                    .commands
                    .push(self.make_prepare_acceptance(&candidate, false)?);
            } else {
                self.schedule(ControlOperation::Promote(candidate, record), actions);
            }
        } else if compatible && transfer.is_none() && self.ancestor_assumptions_unresolved(self.branch(&candidate)?) {
            // A matching nested Proposal waits for ancestor validation before it can commit.
            actions.records.push(json!({
                "branch_id": candidate,
                "hypothesis": self.branch_proposal(&candidate).map(|proposal| &proposal.payload),
                "verdict": verdict,
                "disposition": "Wait",
                "reason": "ancestor Proposal is not confirmed"
            }));
        } else {
            let why = if error.is_some() {
                DiscardedReason::Failed
            } else if resolution == Some("preempted") {
                DiscardedReason::BudgetExhausted
            } else {
                DiscardedReason::MatchRejected
            };
            actions.commands.extend(self.discard_candidate(
                &candidate,
                why,
                &format!(
                    "Validation disposition: {verdict}; {}",
                    error.as_deref().unwrap_or("no policy error")
                ),
            )?);
            let mut record = record;
            record["disposition"] = json!("Discard");
            actions.records.push(record);
        }
        Ok(())
    }

    fn complete_promote(
        &mut self,
        candidate: String,
        mut record: Value,
        result: Result<ControlResult, String>,
        actions: &mut ControlActions,
    ) -> io::Result<()> {
        if record.get("full").is_some() {
            let ControlResult::Acceptance(outcome, access, reason) =
                result.map_err(io::Error::other)?
            else {
                return Err(io::Error::other("Runtime acceptance receipt missing"));
            };
            if let Some(outcome) = outcome {
                use crate::protocol::contract::Acceptance;
                let part = !matches!(outcome, Acceptance::Full);
                let notification = match &outcome {
                    Acceptance::Full => record["notification"].as_str().map(str::to_owned),
                    Acceptance::PartResults { results } => Some(results.iter().map(|id| {
                        let descriptions = record["part"]["results"].as_array().unwrap();
                        let result = descriptions.iter().find(|result| result["id"] == *id).unwrap();
                        format!("Accepted result: {}", result["notification"].as_str().unwrap())
                    }).collect::<Vec<_>>().join("\n")),
                    Acceptance::Part { scopes } => {
                        let selected = &record["part"]["scopes"];
                        let result = record["part"]["notification"]
                            .as_str()
                            .filter(|_| *selected == json!(scopes));
                        Some(format!(
                            "Speculative acceptance committed filesystem scopes {}.{}",
                            json!(scopes),
                            result
                                .map(|text| format!(" Accepted result: {text}"))
                                .unwrap_or_default()
                        ))
                    }
                };
                if !part && access.is_none() {
                    return Err(io::Error::other(
                        "full acceptance is missing target execution access",
                    ));
                }
                if !part {
                    self.record_runtime_state(&candidate, RuntimeExecutionState::Promoted)?;
                }
                let mut command = self.make_promote_command(&candidate, notification)?;
                if let AgentCommand::Promote {
                    execution_access,
                    parent_branch_id,
                    part: command_part,
                    ..
                } = &mut command
                {
                    let parent = self.chance_parent(self.branch(&candidate)?);
                    *parent_branch_id = parent.filter(|parent| Some(*parent) != self.authority_branch_id.as_deref()).map(str::to_owned);
                    *execution_access = access;
                    *command_part = part;
                }
                record["disposition"] = if part {
                    json!(record["part"]["intent"].as_str().unwrap_or("Reuse"))
                } else {
                    json!("Promote")
                };
                record["outcome"] = json!(outcome);
                self.pending_acceptance = Some((candidate.clone(), record.clone()));
                actions.commands.push(command);
            } else {
                self.record_runtime_state(&candidate, RuntimeExecutionState::Failed)?;
                record["disposition"] = json!("Discard");
                record["runtime_promote_error"] = json!(reason);
                actions.commands.extend(
                    self.cancel_acceptance(&candidate, "Runtime did not accept the transfer")?,
                );
            }
            actions.records.push(record);
            return Ok(());
        }
        let succeeded = result.is_ok();
        self.record_runtime_state(
            &candidate,
            if succeeded {
                RuntimeExecutionState::Promoted
            } else {
                RuntimeExecutionState::Failed
            },
        )?;
        if !self.stopping && self.branches[&candidate].status == BranchStatus::Promoting {
            let notification = record
                .get("notification")
                .and_then(Value::as_str)
                .map(str::to_owned);
            actions.commands.extend(self.finish_runtime_promotion(
                &candidate,
                succeeded,
                notification,
            )?);
            record["disposition"] = json!(if succeeded { "Promote" } else { "Discard" });
            record["runtime_promote_error"] = json!(result.err());
            actions.records.push(record);
        }
        Ok(())
    }

    fn complete_terminate_or_cleanup(
        &mut self,
        work: ControlOperation,
        result: Result<ControlResult, String>,
        actions: &mut ControlActions,
        branch: &str,
    ) -> io::Result<()> {
        if let Err(error) = result {
            self.record_runtime_state(branch, RuntimeExecutionState::Failed)?;
            return Err(io::Error::other(error));
        }
        let state = if matches!(work, ControlOperation::Terminate(_)) {
            RuntimeExecutionState::Terminated
        } else {
            RuntimeExecutionState::Cleaned
        };
        self.record_runtime_state(branch, state)?;
        if state == RuntimeExecutionState::Terminated {
            self.schedule(ControlOperation::Cleanup(branch.to_string()), actions);
        }
        Ok(())
    }

    fn progress_chances(&mut self, actions: &mut ControlActions) -> io::Result<()> {
        let selectable = self.selectable_chances();
        if !selectable.is_empty()
            && !self
                .pending_work
                .values()
                .any(|work| matches!(work, ControlOperation::Select(_)))
        {
            self.schedule(ControlOperation::Select(selectable), actions);
        }
        let ids: Vec<String> = self.chances.keys().cloned().collect();
        for id in ids {
            self.release_tool_continuation(&id, actions)?;
            let chance = self.chance(&id)?;
            if chance.speculation_kind == Some(ActionKind::ModelInference)
                && chance.authority == AuthorityState::Arrived
                && (chance.branch_ids.is_empty() || chance.stage.is_terminal())
                && self
                    .branch(&chance.parent_branch_id)?
                    .pending_operations
                    .iter()
                    .any(|op| {
                        op.operation_id == chance.operation_id
                            && op.status == OperationStatus::Requested
                    })
            {
                let parent = chance.parent_branch_id.clone();
                let operation = chance.operation_id.clone();
                actions.commands.push(self.resolve_pending(
                    &parent,
                    &operation,
                    json!({"phase":"continue"}),
                )?);
                self.complete_operation(&parent, Some(&operation), OperationStatus::Succeeded)?;
            }
            let chance = self.chance(&id)?;
            if !self.chance_waiting_live(chance)
                || self
                    .pending_work
                    .values()
                    .any(|work| work.chance_id() == Some(id.as_str()))
            {
                continue;
            }
            if chance.authority == AuthorityState::NotRequested && chance.stage.is_terminal() {
                self.authorize_authority(&id, actions)?;
            }
            let chance = self.chance(&id)?;
            match chance.authority {
                AuthorityState::NotRequested
                    if chance.stage == ChanceStage::Running
                        && chance.launch_epoch != Some(self.epoch) =>
                {
                    let tool = self.tool_pending(&id)?;
                    self.chance_mut(&id)?.launch_epoch = Some(self.epoch);
                    self.schedule(ControlOperation::Launch(tool), actions);
                }
                AuthorityState::Fetching => {}
                _ => {}
            }
        }
        Ok(())
    }

    pub fn pending_work(&self) -> bool {
        !self.pending_work.is_empty()
    }

    pub fn progress(&mut self, capabilities: &RuntimeCapabilities) -> io::Result<ControlActions> {
        let mut actions = ControlActions::default();
        if self.recovering {
            if let Some(command) = self.next_prompt()? {
                actions.commands.push(command);
            }
            return Ok(actions);
        }
        self.check_command_deadlines()?;
        if self.stopping {
            actions.commands.extend(self.shutdown_cleanup()?);
        } else if let Some((candidate, record)) = self.pending_acceptance.as_mut() {
            if let Some(parent) = record["resume_parent"].as_str().map(str::to_owned) {
                self.pending_acceptance = None;
                self.schedule(ControlOperation::ResumeAcceptance(parent), &mut actions);
            } else if record["interrupted"] == true
                && record.get("outcome").is_none()
                && !self.pending_work.values().any(|work| matches!(work, ControlOperation::SuspendAcceptance(id) if id == candidate))
                && record["parked"] == true
            {
                let candidate = candidate.clone();
                actions.commands.extend(self.cancel_acceptance(&candidate, "Execution interrupted before acceptance")?);
            } else if record["prepared"] == true && record.get("outcome").is_none() {
                let (candidate, record) = self.pending_acceptance.take().unwrap();
                self.schedule(ControlOperation::Promote(candidate, record), &mut actions);
            } else if record["parked"] == true && record.get("suspend_requested").is_none() {
                record["suspend_requested"] = json!(true);
                let candidate = candidate.clone();
                self.schedule(ControlOperation::SuspendAcceptance(candidate), &mut actions);
            }
        }
        self.sweep_chances(&mut actions)?;
        if !self.stopping {
            self.progress_chances(&mut actions)?;
            for candidate in self.pending_candidates() {
                if self.pending_work.values().any(|work| work.branch() == Some(candidate.as_str())) {
                    continue;
                }
                let branch = &self.branches[&candidate];
                if candidate.starts_with("candidate-chance-local-")
                    && branch.environment_id.is_none()
                    && branch.execution_state.runtime == RuntimeExecutionState::EnvironmentRequested
                {
                    self.establish_local_candidate(&candidate, &mut actions)?;
                } else if self.candidate_due(&candidate) {
                    self.schedule(ControlOperation::Validate(candidate), &mut actions);
                } else if branch.status == BranchStatus::Blocked(BlockedReason::ExtentReached)
                    && self.branch_proposal(&candidate).is_some()
                    && self.extent_disposals.insert(candidate.clone())
                {
                    self.schedule(ControlOperation::Dispose(candidate, "extent_reached".into()), &mut actions);
                }
            }
            if self.rebalance_epoch != Some(self.epoch)
                && !self
                    .pending_work
                    .values()
                    .any(|work| matches!(work, ControlOperation::Rebalance))
                && self.branches.values().any(|branch| {
                    branch.role == BranchRole::Candidate
                        && matches!(
                            branch.status,
                            BranchStatus::Running
                                | BranchStatus::Blocked(BlockedReason::Rebalanced)
                        )
                })
            {
                self.rebalance_epoch = Some(self.epoch);
                self.schedule(ControlOperation::Rebalance, &mut actions);
            }
            if let Some(command) = self.next_prompt()? {
                actions.commands.push(command);
            }
        }
        if capabilities.branch_environment {
            let clean: Vec<_> = self
                .branches
                .values()
                .filter(|branch| {
                    branch.status == BranchStatus::Cleaned
                        && !self.has_live_environment_dependents(&branch.branch_id)
                        && !self.pending_acceptance.as_ref().is_some_and(|(id, _)| {
                            self.chance_parent(&self.branches[id]) == Some(&branch.branch_id)
                        })
                        && !self.pending_work.values().any(|work| match work {
                            ControlOperation::SuspendAcceptance(id) | ControlOperation::Promote(id, _) => {
                                self.chance_parent(&self.branches[id]) == Some(&branch.branch_id)
                            }
                            _ => false,
                        })
                        && !matches!(
                            branch.execution_state.runtime,
                            RuntimeExecutionState::Cleaned
                                | RuntimeExecutionState::TerminationRequested
                        )
                        && !self
                            .pending_work
                            .values()
                            .any(|work| work.branch() == Some(branch.branch_id.as_str()))
                })
                .map(|branch| branch.branch_id.clone())
                .collect();
            for branch in clean {
                self.record_runtime_state(&branch, RuntimeExecutionState::TerminationRequested)?;
                self.schedule(ControlOperation::Terminate(branch), &mut actions);
            }
        }
        Ok(actions)
    }
}
