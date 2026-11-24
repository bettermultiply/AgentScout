
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
            Self::Establish(command) => Some(command.branch_id()),
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
                .is_some_and(|branch| !branch.yield_only && self.authority_advanced_past(branch))
        {
            actions.commands.extend(self.discard_candidate(
                candidate,
                DiscardedReason::MatchRejected,
                "Validation waiting point expired",
            )?);
        }
        Ok(())
    }

    pub(super) fn yield_snapshot(&self, candidate: &str) -> Value {
        let branch = &self.branches[candidate];
        let tool_count = self.action_facts.get(candidate).into_iter().flat_map(|facts| facts.values())
            .filter(|fact| fact.action_kind == ActionKind::ToolExecution
                && matches!(fact.status, ActionStatus::Succeeded | ActionStatus::Failed)).count();
        json!([branch.status, branch.execution_state.turn_active, tool_count])
    }

    pub(super) fn yield_candidate_stopped(&self, branch: &BranchRecord) -> bool {
        branch.status == BranchStatus::Blocked(BlockedReason::ExtentReached)
            || !branch.execution_state.turn_active
    }

    pub(super) fn yield_pending(&self, candidate: &str) -> bool {
        let current = self.yield_snapshot(candidate);
        let previous = self.deferred_validations.get(candidate).and_then(|snapshot| snapshot[2].as_u64());
        current[2].as_u64().unwrap_or(0) > previous.unwrap_or(0)
    }

    pub(crate) fn model_inference_boundary(&mut self, task: &str, branch: &str, action: &str) -> io::Result<ControlActions> {
        if task != self.execution_id || self.authority_branch_id.as_deref() != Some(branch) {
            return Err(invalid_input("model inference boundary must belong to Authority"));
        }
        if !self.stopping {
            self.model_inference_boundary = Some((branch.into(), action.into(), None));
        }
        Ok(ControlActions::default())
    }

    fn flush_model_inference_boundary(&mut self, actions: &mut ControlActions) -> io::Result<()> {
        let Some((parent, ..)) = &self.model_inference_boundary else { return Ok(()) };
        if self.authority_branch_id.as_deref() != Some(parent) {
            self.model_inference_boundary = None;
            return Ok(());
        }
        if self.has_pending_yield(parent) { return Ok(()) }
        let (parent, operation, chance) = self.model_inference_boundary.take().unwrap();
        self.release_authority(&parent, &operation, chance.as_deref(), actions)
    }

    fn has_pending_yield(&self, parent: &str) -> bool {
        self.branches.values().any(|candidate| candidate.yield_only
            && (candidate.status.is_decidable() || candidate.status == BranchStatus::Blocked(BlockedReason::Rebalanced))
            && self.chance_parent(candidate) == Some(parent) && self.yield_pending(&candidate.branch_id))
    }

    pub(crate) fn validation_snapshot(&self, candidate: &str) -> Value {
        let operation = ControlOperation::Validate(candidate.to_owned());
        let parent = self.chance_parent(&self.branches[candidate]);
        json!({
            "dependencies": self.policy_dependencies_for(&operation),
            "candidate": self.policy_branch_context(candidate, &operation),
            "parent": parent.map(|id| self.policy_branch_context(id, &operation)),
            "outcome": self.candidate_outcome(candidate),
        })
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
                let mut context = json!(chance);
                if let Value::Object(fields) = &mut context {
                    fields.remove("launch_epoch");
                }
                context
            }).collect::<Vec<_>>(),
            "usage": self.model_usage.get(id), "facts": self.action_facts.get(id), "active": self.active_actions.get(id),
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
        self.register_action(&branch_id, operation_id.clone(), ActionRecord {
            action_kind, name: tool_name.clone(), parent_action_id, arguments: None,
        })?;
        let mut actions = ControlActions::default();
        let branch = self.branch(&branch_id)?;
        // Admission can cross the acceptance pause in transit. The Agent
        // still waits for acceptance before executing this action.
        if branch.yield_only || branch.status == BranchStatus::Promoting {
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
        let context = self.policy_work_context.remove(&id);
        if self.work_context_expired(id, &work, context.as_ref()) {
            self.discard_expired_validation(&work, &mut actions)?;
            return Ok(actions);
        }
        let (work, result) = self.filter_work_results(work, result, context.as_ref());
        match work {
            ControlOperation::Select(ids) => {
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
                let AgentCommand::StartCandidate { branch_id, .. } = &command else {
                    return Err(io::Error::other(
                        "establishment work is missing a StartCandidate command",
                    ));
                };
                let branch = branch_id.clone();
                self.complete_establish(command, result, &mut actions, &branch)?;
            }
            ControlOperation::Validate(candidate) => {
                self.complete_validate(id, candidate, None, result, &mut actions)?;
            }
            ControlOperation::Dispose(candidate, resolution) => {
                self.complete_validate(id, candidate, Some(&resolution), result, &mut actions)?;
            }
            ControlOperation::SuspendAcceptance(candidate) => {
                if result.is_ok() {
                    let part = if let Some((id, record)) = self.pending_acceptance.as_mut()
                        && *id == candidate
                    {
                        record["suspended"] = json!(true);
                        record["full"] == false
                    } else {
                        return Err(io::Error::other(
                            "acceptance target paused without a pending acceptance",
                        ));
                    };
                    if !self.stopping {
                        if self.acceptance_target(self.branch(&candidate)?).is_some() {
                            actions
                                .commands
                                .push(self.make_prepare_acceptance(&candidate, true, part)?);
                        } else {
                            actions.commands.extend(self.cancel_acceptance(
                                &candidate,
                                "Acceptance target is no longer eligible",
                            )?);
                        }
                    }
                } else if let Err(reason) = result {
                    actions
                        .commands
                        .extend(self.cancel_acceptance(&candidate, &reason)?);
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
            ControlOperation::Terminate(branch) => {
                self.complete_terminate_or_cleanup(
                    &branch,
                    RuntimeExecutionState::Terminated,
                    result,
                    &mut actions,
                )?;
            }
            ControlOperation::Cleanup(branch) => {
                self.complete_terminate_or_cleanup(
                    &branch,
                    RuntimeExecutionState::Cleaned,
                    result,
                    &mut actions,
                )?;
            }
        }
        Ok(actions)
    }

    fn work_context_expired(
        &mut self,
        id: u64,
        work: &ControlOperation,
        context: Option<&BTreeMap<String, Value>>,
    ) -> bool {
        if self
            .policy_dependencies
            .remove(&id)
            .is_some_and(|original| original != self.policy_dependencies_for(work))
        {
            return true;
        }
        let Some(context) = context else {
            return false;
        };
        let changed = |branch: &str, original: &Value| {
            !self.branches.contains_key(branch)
                || *original != self.policy_branch_context(branch, work)
        };
        if matches!(
            work,
            ControlOperation::Launch(_)
                | ControlOperation::Validate(_)
                | ControlOperation::Dispose(_, _)
        ) && context
            .iter()
            .any(|(branch, original)| changed(branch, original))
        {
            return true;
        }
        work.branch().is_some_and(|branch| {
            context
                .get(branch)
                .is_none_or(|original| changed(branch, original))
        })
    }

    fn filter_work_results(
        &self,
        work: ControlOperation,
        mut result: Result<ControlResult, String>,
        context: Option<&BTreeMap<String, Value>>,
    ) -> (ControlOperation, Result<ControlResult, String>) {
        let Some(context) = context else {
            return (work, result);
        };
        let branch_unchanged = |branch: &str| {
            self.branches.contains_key(branch)
                && context
                    .get(branch)
                    .is_some_and(|original| *original == self.policy_branch_context(branch, &work))
        };
        let chance_unchanged = |id: &String| {
            self.chances
                .get(id)
                .is_some_and(|chance| branch_unchanged(&chance.parent_branch_id))
        };
        match &mut result {
            Ok(ControlResult::Rebalance(entries)) => {
                entries.retain(|(branch, _)| branch_unchanged(branch))
            }
            Ok(ControlResult::Selected(ids)) => ids.retain(chance_unchanged),
            _ => {}
        }
        let work = match &work {
            ControlOperation::Select(ids) => ControlOperation::Select(
                ids.iter()
                    .filter(|id| chance_unchanged(id))
                    .cloned()
                    .collect(),
            ),
            _ => work,
        };
        (work, result)
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
            Ok(ControlResult::BranchModel(mut prediction)) => {
                let count = prediction.get("candidates").and_then(Value::as_u64).unwrap_or(1).clamp(1, 4);
                if let Some(fields) = prediction.as_object_mut() {
                    fields.remove("candidates");
                }
                for _ in 0..count {
                    let command = self.start_model_candidate(chance_id, prediction.clone())?;
                    if let AgentCommand::StartCandidate { branch_id, .. } = &command {
                        actions.records.push(json!({"message":"candidate_created","branch_id":branch_id,"parent":tool.branch_id,"chance_id":chance_id,"fork_at":self.branches[branch_id].execution_position.transcript_index}));
                    }
                    self.schedule(ControlOperation::Establish(command), actions);
                }
            }
            Ok(ControlResult::Abstain(reason)) => {
                self.change_chance_stage(chance_id, ChanceStage::Discarded(DiscardedReason::NotSelected),
                    reason.as_deref().unwrap_or("Policy abstained"))?;
                self.authorize_authority(chance_id, actions)?;
            }
            Ok(ControlResult::Predictions(predictions)) => {
                for prediction in predictions {
                    let model_options = Value::Object(["model", "thinking", "candidate_guidance"].into_iter()
                        .filter_map(|key| prediction.get(key).map(|value| (key.into(), value.clone())))
                        .collect());
                    let model_options = if model_options.as_object().is_some_and(|options| options.is_empty()) {
                        Value::Null
                    } else { model_options };
                    let command = self.start_candidate_with_provenance(chance_id,
                        prediction["payload"].clone(), prediction.get("provenance").cloned(), model_options)?;
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
                    if !self.chance(&chance_id)?.deferred_environment {
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
        let chance = self.chance(id)?.clone();
        if chance.speculation_kind == Some(ActionKind::ModelInference)
            && self.authority_branch_id.as_deref() == Some(&chance.parent_branch_id)
            && self.has_pending_yield(&chance.parent_branch_id)
        {
            self.model_inference_boundary = Some((chance.parent_branch_id, chance.operation_id, Some(id.to_owned())));
            return Ok(());
        }
        if self.model_inference_boundary.as_ref().is_some_and(|(parent, operation, _)| {
            parent == &chance.parent_branch_id && operation == &chance.operation_id
        }) {
            self.model_inference_boundary = None;
        }
        self.release_authority(&chance.parent_branch_id, &chance.operation_id, Some(id), actions)
    }

    fn release_authority(
        &mut self,
        parent: &str,
        operation: &str,
        chance: Option<&str>,
        actions: &mut ControlActions,
    ) -> io::Result<()> {
        let outcome = if let Some(id) = chance.filter(|id| {
            self.chances.get(*id).is_some_and(|chance| !chance.stage.is_terminal())
        }) {
            self.change_authority_state(id, AuthorityState::Fetching)?;
            json!({"phase":"execute"})
        } else {
            self.complete_operation(parent, Some(operation), OperationStatus::Succeeded)?;
            json!({"phase":"execute", "speculation_skipped":true})
        };
        actions.commands.push(self.resolve_pending(parent, operation, outcome)?);
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
                let candidate = self.branch(branch)?;
                if !self.stopping && candidate.deferred_environment && candidate.status.is_live_candidate()
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
        if let Ok(ControlResult::AcceptanceDisposition(.., Some(audit))) = &result {
            actions
                .records
                .push(json!({"message": "policy_audit", "branch_id": candidate, "audit": audit}));
        }
        if self.stopping
            || match resolution {
                None => !self.candidate_due(&candidate),
                Some(_) => {
                    let branch = self.branch(&candidate)?;
                    !branch.status.is_live_candidate()
                        || branch.yield_only
                            && self.yield_candidate_stopped(branch)
                            && self.yield_pending(&candidate)
                            && !self.candidate_due(&candidate)
                }
            }
        {
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
                    _,
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
                _ => return Err(io::Error::other("unexpected validation result")),
            };
        let branch = self.branch(&candidate)?;
        let proposal = self.branch_proposal(&candidate).cloned();
        let chance_id = branch
            .chance_id
            .clone()
            .ok_or_else(|| io::Error::other("candidate Chance is missing"))?;
        let salvage = part
            .as_ref()
            .is_some_and(|part| part["intent"] == "Salvage");
        let yield_only = branch.yield_only;
        let stopped = self.yield_candidate_stopped(branch);
        let mut record = json!({"branch_id":candidate, "hypothesis":proposal.as_ref().map(|p|&p.payload), "verdict":verdict, "policy_error":error, "accepted":compatible});
        if let Some(part) = part.as_ref().filter(|part| part["intent"] == "Yield")
            && (!self.task_finishing || yield_only)
            && resolution != Some("preempted")
            && self.apply_yield(&candidate, &chance_id, part, actions)?
        {
            record["disposition"] = json!("Yield");
            record["part"] = part.clone();
            actions.records.push(record);
            if stopped || self.task_finishing {
                actions.commands.extend(self.discard_candidate(
                    &candidate,
                    DiscardedReason::MatchRejected,
                    "Candidate stopped after Yield",
                )?);
            } else {
                let snapshot = if self.branch(&candidate)?.yield_only {
                    self.yield_snapshot(&candidate)
                } else {
                    self.validation_snapshot(&candidate)
                };
                self.deferred_validations
                    .insert(candidate.clone(), snapshot);
            }
            return Ok(());
        }
        // A retired candidate can still hand off notifications without reopening acceptance.
        let retired_salvage = if yield_only && !compatible {
            part.as_ref().filter(|part| {
                part["intent"] == "Salvage" && part["results"].as_array().is_some_and(|results| {
                    results.iter().all(|result| result["files"] == json!([]))
                })
            })
        } else {
            None
        };
        if let Some(part) = retired_salvage
            && self.apply_yield(&candidate, &chance_id, part, actions)?
        {
            record["disposition"] = json!("Salvage");
            record["part"] = part.clone();
            actions.records.push(record);
            self.queue_disposition_settlement(&candidate, false, "Salvage")?;
            actions.commands.extend(self.discard_candidate(
                &candidate, DiscardedReason::MatchRejected, "Delivered remaining candidate results",
            )?);
            return Ok(());
        }
        if yield_only
            && !stopped
            && transfer != Some(true)
            && error.is_none()
            && resolution != Some("preempted")
        {
            self.deferred_validations
                .insert(candidate.clone(), self.yield_snapshot(&candidate));
            return Ok(());
        }
        if !yield_only && (verdict == "wait" || (verdict == "indeterminate" && !salvage)) {
            let snapshot = self.validation_snapshot(&candidate);
            self.deferred_validations.insert(candidate, snapshot);
            return Ok(());
        }
        self.deferred_validations.remove(&candidate);
        let Some(proposal) = proposal else {
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
        };
        if resolution.is_none() && error.is_none() {
            self.record_validation(
                &chance_id,
                &proposal.proposal_id,
                validation_id,
                validated_at_ms,
            );
        }
        let branch = self.branch(&candidate)?;
        let ancestor_unresolved = (salvage || (compatible && transfer.is_none()))
            && self.ancestor_assumptions_unresolved(branch);
        if self.can_accept(
            &candidate,
            compatible || (salvage && !ancestor_unresolved),
            transfer.is_some(),
        ) {
            if let Some(notification) = notification {
                record["notification"] = json!(notification);
            }
            return self.request_candidate_acceptance(
                candidate,
                &chance_id,
                resolution.is_none().then_some(validation_id),
                transfer,
                part,
                record,
                actions,
            );
        }
        if compatible && transfer.is_none() && ancestor_unresolved {
            // A matching nested Proposal waits for ancestor validation before it can commit.
            actions.records.push(json!({
                "branch_id": candidate,
                "hypothesis": proposal.payload,
                "verdict": verdict,
                "disposition": "Wait",
                "reason": "ancestor Proposal is not confirmed"
            }));
            return Ok(());
        }
        self.discard_after_validation(
            &candidate,
            resolution,
            &verdict,
            error.as_deref(),
            record,
            actions,
        )
    }

    fn request_candidate_acceptance(
        &mut self,
        candidate: String,
        chance_id: &str,
        validation_id: Option<u64>,
        transfer: Option<bool>,
        part: Option<Value>,
        mut record: Value,
        actions: &mut ControlActions,
    ) -> io::Result<()> {
        let branch = self.branch(&candidate)?;
        let previous_status = branch.status;
        let unbound_local = branch.deferred_environment && branch.environment_id.is_none();
        let salvage = part
            .as_ref()
            .is_some_and(|part| part["intent"] == "Salvage");
        if unbound_local && transfer == Some(false) {
            self.establish_local_candidate(&candidate, actions)?;
            return Ok(());
        }
        if let Some(validation_id) = validation_id {
            self.chance_mut(chance_id)?.validation_id = Some(validation_id);
        }
        self.record_runtime_state(&candidate, RuntimeExecutionState::PromotionRequested)?;
        self.change_status(
            &candidate,
            BranchStatus::Promoting,
            if salvage {
                "Runtime salvage requested"
            } else {
                "Runtime acceptance requested"
            },
        )?;
        if unbound_local {
            let mut command = self.make_promote_command(
                &candidate,
                record["notification"].as_str().map(str::to_owned),
            )?;
            if let AgentCommand::Promote {
                reuse_parent_environment,
                ..
            } = &mut command
            {
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
            let partial = record["full"] == false;
            self.pending_acceptance = Some((candidate.clone(), record));
            actions
                .commands
                .push(self.make_prepare_acceptance(&candidate, false, partial)?);
        } else {
            self.schedule(ControlOperation::Promote(candidate, record), actions);
        }
        Ok(())
    }

    fn discard_after_validation(
        &mut self,
        candidate: &str,
        resolution: Option<&str>,
        verdict: &str,
        error: Option<&str>,
        mut record: Value,
        actions: &mut ControlActions,
    ) -> io::Result<()> {
        let why = if error.is_some() {
            DiscardedReason::Failed
        } else if resolution == Some("preempted") {
            DiscardedReason::BudgetExhausted
        } else {
            DiscardedReason::MatchRejected
        };
        actions.commands.extend(self.discard_candidate(
            candidate,
            why,
            &format!(
                "Validation disposition: {verdict}; {}",
                error.unwrap_or("no policy error")
            ),
        )?);
        record["disposition"] = json!("Discard");
        actions.records.push(record);
        Ok(())
    }

    fn apply_yield(
        &mut self, candidate: &str, chance_id: &str, part: &Value, actions: &mut ControlActions,
    ) -> io::Result<bool> {
        let branch = self.branch(candidate)?;
        let chance = self.chance(chance_id)?;
        let parent = chance.parent_branch_id.clone();
        let target = self.branch(&parent)?;
        if !(target.status.is_live_authority() || target.status.is_decidable())
            || self.ancestor_assumptions_unresolved(branch)
        {
            return Ok(false);
        }
        let operation = chance.operation_id.clone();
        let arrived = chance.authority == AuthorityState::Arrived;
        let release_model = arrived && chance.speculation_kind == Some(ActionKind::ModelInference)
            && self.chance_waiting_live(chance);
        let release_tool = chance.tool_continuation_pending;
        let yield_only = arrived || self.authority_advanced_past(branch);
        let mut ids = BTreeSet::new();
        let notifications: Vec<_> = part["results"].as_array().into_iter().flatten().filter_map(|result| {
            let id = result["id"].as_str()?;
            let notification = result["notification"].as_str()?;
            (result["files"] == json!([]) && !branch.yielded_action_ids.contains(id)
                && ids.insert(id.to_owned())).then_some(notification)
        }).collect();
        if !notifications.is_empty() || release_model || release_tool || (yield_only && !branch.yield_only) {
            actions.commands.push(self.resolve_pending(&parent, &operation,
                json!({"phase":"continue", "notification": notifications.join("\n"),
                    "yield_candidate": yield_only.then_some(candidate)}))?);
        }
        if release_model {
            self.complete_operation(&parent, Some(&operation), OperationStatus::Succeeded)?;
        }
        if release_model || release_tool {
            let chance = self.chance_mut(chance_id)?;
            chance.tool_continuation_pending = false;
            chance.continuation_released_by_yield = true;
        }
        let branch = self.branch_mut(candidate)?;
        branch.yield_only |= yield_only;
        branch.yielded_action_ids.extend(ids);
        if branch.status == BranchStatus::Finished && branch.execution_state.turn_active {
            self.change_status(candidate, BranchStatus::Running, "Candidate turn continues after Yield")?;
        }
        Ok(true)
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
                    Acceptance::PartResults { results } => {
                        let mut notifications = Vec::with_capacity(results.len());
                        for id in results {
                            let descriptions = record["part"]["results"].as_array().ok_or_else(|| {
                                io::Error::other("partial acceptance is missing result descriptions")
                            })?;
                            let description = descriptions.iter().find(|result| result["id"] == *id)
                                .ok_or_else(|| io::Error::other(format!("partial acceptance returned unknown result {id}")))?;
                            let notification = description["notification"].as_str().ok_or_else(|| {
                                io::Error::other(format!("accepted result {id} is missing a notification"))
                            })?;
                            notifications.push(format!("Accepted result: {notification}"));
                        }
                        Some(notifications.join("\n"))
                    }
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
                    json!("Salvage")
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
        branch: &str,
        state: RuntimeExecutionState,
        result: Result<ControlResult, String>,
        actions: &mut ControlActions,
    ) -> io::Result<()> {
        if let Err(error) = result {
            self.record_runtime_state(branch, RuntimeExecutionState::Failed)?;
            return Err(io::Error::other(error));
        }
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
                || self.model_inference_boundary.as_ref().is_some_and(|(parent, operation, _)| {
                    parent == &chance.parent_branch_id && operation == &chance.operation_id
                })
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
                let (candidate, record) = self.pending_acceptance.take().ok_or_else(|| {
                    io::Error::other("prepared acceptance is missing its pending record")
                })?;
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
                if branch.yield_only
                    && (self.task_finishing || self.yield_candidate_stopped(branch))
                    && !self.yield_pending(&candidate)
                {
                    actions.commands.extend(self.discard_candidate(&candidate, DiscardedReason::MatchRejected, "Candidate has no remaining results")?);
                    continue;
                }
                if branch.deferred_environment
                    && branch.environment_id.is_none()
                    && branch.execution_state.runtime == RuntimeExecutionState::EnvironmentRequested
                {
                    self.establish_local_candidate(&candidate, &mut actions)?;
                } else if self.candidate_due(&candidate) {
                    // Apply each verdict before checking the next sibling's live count.
                    let chance = &branch.chance_id;
                    if !self.pending_work.values().any(|work| matches!(work,
                        ControlOperation::Validate(id) if &self.branches[id].chance_id == chance)) {
                        self.schedule(ControlOperation::Validate(candidate), &mut actions);
                    }
                } else if !branch.yield_only && branch.status == BranchStatus::Blocked(BlockedReason::ExtentReached)
                    && self.branch_proposal(&candidate).is_some()
                    && self.extent_disposals.insert(candidate.clone())
                {
                    self.schedule(ControlOperation::Dispose(candidate, "extent_reached".into()), &mut actions);
                }
            }
            self.flush_model_inference_boundary(&mut actions)?;
            if self.rebalance_epoch != Some(self.epoch)
                && !self
                    .pending_work
                    .values()
                    .any(|work| matches!(work, ControlOperation::Rebalance))
                && self.branches.values().any(|branch| {
                    branch.is_running_candidate()
                        || branch.role == BranchRole::Candidate
                            && branch.status == BranchStatus::Blocked(BlockedReason::Rebalanced)
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
