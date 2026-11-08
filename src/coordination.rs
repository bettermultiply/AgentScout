use std::io;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::agent_control::{
    AgentCommand, AgentMessage, BranchPhase, BranchStatus, OperationKind, OperationStatus,
    ProposalRecord, RuntimeExecutionState, SpeculativeTree,
};
use crate::contract::{Operation, RuntimeCapabilities};
use crate::control::ControlSession;
use crate::policy::{CompiledPolicy, Decision, PolicyEngine};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolPending {
    pub branch_id: String,
    pub operation_id: String,
    pub tool_name: String,
    pub arguments: Value,
    pub checkpoint: Option<Value>,
    pub launch: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControlOperation {
    Predict(ToolPending),
    Establish(AgentCommand),
    Invoke(ToolPending),
    Validate(String),
    Promote(String, Value),
    Terminate(String),
    Cleanup(String),
}

impl ControlOperation {
    pub fn references(&self, branch: &str) -> bool {
        self.branch() == branch
            || matches!(self, Self::Establish(AgentCommand::StartCandidate {parent_branch_id, ..}) if parent_branch_id == branch)
    }

    pub fn branch(&self) -> &str {
        match self {
            Self::Predict(tool) | Self::Invoke(tool) => &tool.branch_id,
            Self::Establish(command) => {
                let AgentCommand::StartCandidate { branch_id, .. } = command else {
                    unreachable!()
                };
                branch_id
            }
            Self::Validate(id) | Self::Promote(id, _) | Self::Terminate(id) | Self::Cleanup(id) => {
                id
            }
        }
    }
}

pub enum ControlResult {
    Predictions(Vec<Value>),
    PredictionsWithoutAuthority(Vec<Value>),
    Outcome(Value, Value),
    Disposition(bool, String),
    Completed,
}

#[derive(Default)]
pub struct ControlActions {
    pub commands: Vec<AgentCommand>,
    pub operations: Vec<(u64, ControlOperation)>,
    pub records: Vec<Value>,
}

impl SpeculativeTree {
    fn schedule(&mut self, operation: ControlOperation, actions: &mut ControlActions) {
        let id = self.next_work;
        self.next_work += 1;
        self.pending_work.insert(id, operation.clone());
        actions.operations.push((id, operation));
    }

    fn waiting_point_live(&self, tool: &ToolPending) -> bool {
        !self.stopping
            && self.can_invoke_tool(&tool.branch_id)
            && self.branches[&tool.branch_id]
                .pending_operations
                .iter()
                .any(|op| {
                    op.operation_id == tool.operation_id && op.status == OperationStatus::Requested
                })
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
        let fact = AgentMessage::ActionStarted {
            task_id,
            branch_id: branch_id.clone(),
            action_id: operation_id.clone(),
            action_kind,
            parent_action_id,
            result_pending: true,
            name: tool_name.clone(),
            arguments: arguments.clone(),
            checkpoint: checkpoint.clone(),
        };
        self.handle_message(fact)?;
        let mut actions = ControlActions::default();
        if self.stopping
            || !self.can_invoke_tool(&branch_id)
            || self.branches[&branch_id]
                .pending_operations
                .iter()
                .any(|op| op.operation_id == operation_id)
        {
            return Ok(actions);
        }
        // Admission was decided by the adapter that sent this explicit request.
        // The control-plane phase starts at selection; it does not re-run or
        // claim ownership of the local admission decision.
        self.branches
            .get_mut(&branch_id)
            .expect("admission branch exists")
            .execution_state
            .phase = BranchPhase::Select;
        self.mark_operation_requested(&branch_id, &operation_id, OperationKind::Tool)?;
        self.schedule(
            ControlOperation::Predict(ToolPending {
                branch_id,
                operation_id,
                tool_name,
                arguments,
                checkpoint,
                launch: true,
            }),
            &mut actions,
        );
        Ok(actions)
    }

    pub fn complete_work(
        &mut self,
        id: u64,
        result: Result<ControlResult, String>,
    ) -> io::Result<ControlActions> {
        let mut actions = ControlActions::default();
        let Some(work) = self.pending_work.remove(&id) else {
            return Ok(actions);
        };
        let branch = work.branch().to_string();
        match work {
            ControlOperation::Predict(tool) => {
                let ToolPending {
                    branch_id,
                    operation_id,
                    tool_name,
                    checkpoint,
                    ..
                } = &tool;
                if self.waiting_point_live(&tool) {
                    self.branches
                        .get_mut(branch_id)
                        .expect("live waiting branch")
                        .execution_state
                        .phase = BranchPhase::Predict;
                    let launch_unsupported =
                        matches!(&result, Ok(ControlResult::PredictionsWithoutAuthority(_)));
                    // An empty prediction set is not a failed speculation: it means this boundary has no
                    // hypothesis to test. Execute the tool as an ordinary authority action instead of
                    // leaving the adapter's tool call waiting for a result that will never arrive.
                    let mut direct_invocation = false;
                    match result {
                        Ok(ControlResult::Predictions(predictions)) if predictions.is_empty() => {
                            direct_invocation = true;
                            self.tool_invocations.push(tool.clone());
                        }
                        Ok(ControlResult::Predictions(predictions)) => {
                            for prediction in predictions {
                                let command = self.start_candidate(
                                    branch_id,
                                    operation_id,
                                    tool_name,
                                    prediction,
                                    checkpoint.clone(),
                                )?;
                                let child = match &command {
                                    AgentCommand::StartCandidate { branch_id, .. } => branch_id,
                                    _ => unreachable!(),
                                };
                                actions.records.push(json!({"message":"candidate_created", "branch_id":child, "parent":branch_id, "fork_at":self.branches[child].execution_position.transcript_index}));
                                self.schedule(ControlOperation::Establish(command), &mut actions);
                            }
                        }
                        Ok(ControlResult::PredictionsWithoutAuthority(predictions)) => {
                            for prediction in predictions {
                                let command = self.start_candidate(
                                    branch_id,
                                    operation_id,
                                    tool_name,
                                    prediction,
                                    checkpoint.clone(),
                                )?;
                                let child = match &command {
                                    AgentCommand::StartCandidate { branch_id, .. } => branch_id,
                                    _ => unreachable!(),
                                };
                                actions.records.push(json!({"message":"candidate_created", "branch_id":child, "parent":branch_id, "launch":"unsupported"}));
                                self.schedule(ControlOperation::Establish(command), &mut actions);
                            }
                            actions.records.push(json!({"message":"launch_not_started", "branch_id":branch_id, "operation_id":operation_id, "status":"unsupported", "reason":"validator/evidence-only path is unsupported"}));
                        }
                        Err(error) => actions.records.push(
                            json!({"message":"policy_error", "branch_id":branch, "reason":error}),
                        ),
                        _ => unreachable!(),
                    }
                    // An empty prediction set includes the truthful
                    // launch=false prototype result: the Proposal is not
                    // cancelled, but validator/evidence-only execution is
                    // unsupported. Do not retry the same point forever.
                    if !direct_invocation
                        && !launch_unsupported
                        && !self.branches.values().any(|candidate| {
                            candidate.parent_branch_id.as_deref() == Some(branch_id)
                                && candidate.proposal_id.is_some()
                        })
                    {
                        actions.records.push(json!({
                            "message": "launch_not_started",
                            "branch_id": branch_id,
                            "operation_id": operation_id,
                            "status": "unsupported",
                            "reason": "validator/evidence-only path is unsupported; Proposal retained outside Branch execution"
                        }));
                    } else if !direct_invocation && !launch_unsupported {
                        // Retain the waiting point until all child environments exist.
                        self.tool_invocations.push(tool);
                    }
                }
            }
            ControlOperation::Establish(command) => match result {
                Ok(ControlResult::Completed) => {
                    self.record_runtime_state(&branch, RuntimeExecutionState::Ready)?;
                    if !self.stopping && self.branches[&branch].status == BranchStatus::Created {
                        actions.commands.push(command);
                    }
                }
                Err(reason) => {
                    self.record_runtime_state(&branch, RuntimeExecutionState::Failed)?;
                    actions.records.push(json!({"message":"runtime_operation_failed","branch_id":branch,"reason":reason}));
                    actions
                        .commands
                        .extend(self.resolve_candidate(&branch, false)?);
                }
                _ => unreachable!(),
            },
            ControlOperation::Invoke(tool) => {
                let live = self.waiting_point_live(&tool);
                let ToolPending {
                    branch_id,
                    operation_id,
                    launch: _,
                    ..
                } = tool;
                let (outcome, evidence) =
                    result
                        .map_err(io::Error::other)
                        .and_then(|value| match value {
                            ControlResult::Outcome(outcome, evidence) => Ok((outcome, evidence)),
                            _ => unreachable!(),
                        })?;
                self.record_candidate_outcome(
                    &branch_id,
                    &operation_id,
                    outcome.clone(),
                    Some(evidence),
                )?;
                self.branches
                    .get_mut(&branch_id)
                    .expect("candidate parent exists")
                    .execution_state
                    .phase = BranchPhase::Effect;
                self.complete_operation(
                    &branch_id,
                    Some(&operation_id),
                    OperationStatus::Succeeded,
                )?;
                if live {
                    actions.commands.push(self.resolve_pending(
                        &branch_id,
                        &operation_id,
                        outcome,
                    )?);
                }
            }
            ControlOperation::Validate(candidate) => {
                if !self.stopping && self.candidate_due(&candidate) {
                    self.branches
                        .get_mut(&candidate)
                        .expect("candidate exists")
                        .execution_state
                        .phase = BranchPhase::Dispose;
                    let (compatible, verdict, error) = match result {
                        Ok(ControlResult::Disposition(compatible, verdict)) => {
                            (compatible, verdict, None)
                        }
                        Err(error) => (false, "error".into(), Some(error)),
                        _ => unreachable!(),
                    };
                    let proposal = self.branches[&candidate]
                        .proposal_id
                        .as_ref()
                        .and_then(|id| self.proposals.get(id))
                        .ok_or_else(|| io::Error::other("candidate Proposal is missing"))?;
                    let record = json!({"branch_id":candidate, "hypothesis":proposal.payload, "verdict":verdict, "policy_error":error});
                    if self.can_promote(&candidate, compatible) {
                        self.record_runtime_state(
                            &candidate,
                            RuntimeExecutionState::PromotionRequested,
                        )?;
                        self.branches.get_mut(&candidate).unwrap().status = BranchStatus::Promoting;
                        self.schedule(ControlOperation::Promote(candidate, record), &mut actions);
                    } else {
                        actions
                            .commands
                            .extend(self.resolve_candidate(&candidate, false)?);
                        let mut record = record;
                        record["disposition"] = json!("Discard");
                        actions.records.push(record);
                    }
                }
            }
            ControlOperation::Promote(candidate, mut record) => {
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
                    actions
                        .commands
                        .extend(self.finish_runtime_promotion(&candidate, succeeded)?);
                    record["disposition"] = json!(if succeeded { "Promote" } else { "Discard" });
                    record["runtime_promote_error"] = json!(result.err());
                    actions.records.push(record);
                }
            }
            work @ (ControlOperation::Terminate(_) | ControlOperation::Cleanup(_)) => {
                if let Err(error) = result {
                    self.record_runtime_state(&branch, RuntimeExecutionState::Failed)?;
                    return Err(io::Error::other(error));
                }
                let state = if matches!(work, ControlOperation::Terminate(_)) {
                    RuntimeExecutionState::Terminated
                } else {
                    RuntimeExecutionState::Cleaned
                };
                self.record_runtime_state(&branch, state)?;
                if state == RuntimeExecutionState::Terminated {
                    self.schedule(ControlOperation::Cleanup(branch), &mut actions);
                }
            }
        }
        Ok(actions)
    }

    pub fn pending_work(&self) -> bool {
        !self.pending_work.is_empty()
    }

    pub fn progress(&mut self, capabilities: &RuntimeCapabilities) -> io::Result<ControlActions> {
        let mut actions = ControlActions::default();
        self.check_command_deadlines()?;
        if !self.stopping {
            for tool in std::mem::take(&mut self.tool_invocations) {
                let waiting = self.pending_work.values().any(|work| matches!(work, ControlOperation::Establish(AgentCommand::StartCandidate {parent_branch_id, operation_id, ..}) if parent_branch_id == &tool.branch_id && operation_id == &tool.operation_id));
                if waiting {
                    self.tool_invocations.push(tool);
                } else if self.waiting_point_live(&tool)
                    && capabilities
                        .tools
                        .iter()
                        .any(|declared| declared.name == tool.tool_name)
                {
                    if tool.launch {
                        if let Some(branch) = self.branches.get_mut(&tool.branch_id) {
                            branch.execution_state.phase = BranchPhase::Launch;
                        }
                        self.schedule(ControlOperation::Invoke(tool), &mut actions);
                    } else {
                        actions.records.push(json!({
                            "message": "authority_launch_not_started",
                            "branch_id": tool.branch_id,
                            "operation_id": tool.operation_id,
                            "reason": "policy launch=false; validator/evidence-only path unsupported"
                        }));
                        // launch=false is handled at prediction time and is
                        // never converted into an authority Invoke.
                    }
                }
            }
            for candidate in self.pending_candidates() {
                if self.candidate_due(&candidate)
                    && !self
                        .pending_work
                        .values()
                        .any(|work| work.branch() == candidate)
                {
                    if self.candidate_outcome(&candidate).is_some() {
                        self.branches
                            .get_mut(&candidate)
                            .expect("candidate exists")
                            .execution_state
                            .phase = BranchPhase::Match;
                        self.schedule(ControlOperation::Validate(candidate), &mut actions);
                    } else {
                        actions
                            .commands
                            .extend(self.resolve_candidate(&candidate, false)?);
                        actions.records.push(json!({"branch_id":candidate,"disposition":"Discard","verdict":"timeout"}));
                    }
                }
            }
            if let Some(command) = self.next_prompt()? {
                actions.commands.push(command);
            }
        }
        if capabilities.branch_environment && !capabilities.shared_environment {
            let clean: Vec<_> = self
                .branches
                .values()
                .filter(|branch| {
                    branch.status == BranchStatus::Cleaned
                        && !matches!(
                            branch.execution_state.runtime,
                            RuntimeExecutionState::Cleaned
                                | RuntimeExecutionState::TerminationRequested
                        )
                        && !self
                            .pending_work
                            .values()
                            .any(|work| work.branch() == branch.branch_id)
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

impl AgentController {
    pub fn perform(
        &self,
        tree: &SpeculativeTree,
        policy: &CompiledPolicy,
        runtime: &ControlSession,
        capabilities: &RuntimeCapabilities,
        work: &ControlOperation,
    ) -> Result<ControlResult, String> {
        let environment = if capabilities.shared_environment {
            "authority"
        } else {
            work.branch()
        };
        let call = |environment: &str, operation| {
            let result = runtime
                .call(environment, operation)
                .map_err(|error| error.to_string())?;
            if result.status != crate::contract::ResultStatus::Completed {
                return Err(format!("Runtime request failed: {:?}", result.reason));
            }
            Ok(result)
        };
        match work {
            ControlOperation::Predict(tool) => {
                let observation = if capabilities
                    .tools
                    .iter()
                    .any(|declared| declared.name == tool.tool_name)
                {
                    call(
                        environment,
                        Operation::ToolContext {
                            tool_name: tool.tool_name.clone(),
                        },
                    )
                    .ok()
                    .and_then(|result| result.output)
                    .and_then(|output| serde_json::from_str(&output.stdout).ok())
                } else {
                    None
                };
                self.predict(
                    tree,
                    policy,
                    &tool.operation_id,
                    &tool.tool_name,
                    &tool.arguments,
                    observation.as_ref(),
                )
                .map(|predictions| {
                    if self.launch_allowed(tree, policy, tool) {
                        ControlResult::Predictions(predictions)
                    } else {
                        ControlResult::PredictionsWithoutAuthority(predictions)
                    }
                })
                .map_err(|error| error.to_string())
            }
            ControlOperation::Validate(candidate) => self
                .disposition(
                    policy,
                    tree.branches[candidate]
                        .proposal_id
                        .as_ref()
                        .and_then(|id| tree.proposals.get(id))
                        .ok_or("candidate Proposal is missing")?,
                    &tree
                        .candidate_evidence(candidate)
                        .cloned()
                        .unwrap_or(Value::Null),
                )
                .map(|(compatible, verdict)| ControlResult::Disposition(compatible, verdict))
                .map_err(|error| error.to_string()),
            ControlOperation::Invoke(tool) => {
                let output = call(
                    environment,
                    Operation::InvokeTool {
                        tool_name: tool.tool_name.clone(),
                        arguments: tool.arguments.clone(),
                    },
                )?
                .output
                .ok_or("Runtime tool result missing")?;
                let mut value: Value =
                    serde_json::from_str(&output.stdout).map_err(|error| error.to_string())?;
                let result = value
                    .get_mut("result")
                    .map(Value::take)
                    .ok_or("Runtime result missing")?;
                let evidence = value
                    .get_mut("evidence")
                    .map(Value::take)
                    .ok_or("Runtime evidence missing")?;
                Ok(ControlResult::Outcome(result, evidence))
            }
            ControlOperation::Establish(command) => {
                if capabilities.branch_environment && !capabilities.shared_environment {
                    let AgentCommand::StartCandidate {
                        parent_branch_id,
                        branch_id,
                        ..
                    } = command
                    else {
                        unreachable!()
                    };
                    call(
                        parent_branch_id,
                        Operation::BranchEnvironment {
                            child_environment_id: branch_id.clone(),
                        },
                    )?;
                }
                Ok(ControlResult::Completed)
            }
            ControlOperation::Promote(_, _) => {
                call(environment, Operation::PromoteEnvironment).map(|_| ControlResult::Completed)
            }
            ControlOperation::Terminate(_) => {
                call(environment, Operation::TerminateEnvironment).map(|_| ControlResult::Completed)
            }
            ControlOperation::Cleanup(_) => {
                call(environment, Operation::CleanupEnvironment).map(|_| ControlResult::Completed)
            }
        }
    }
}

#[derive(Default)]
pub struct AgentController;

impl AgentController {
    pub fn predict(
        &self,
        tree: &SpeculativeTree,
        policy: &CompiledPolicy,
        operation_id: &str,
        tool_name: &str,
        arguments: &Value,
        observation: Option<&Value>,
    ) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
        let engine = PolicyEngine::default();
        let edge = json!({"edge_id": operation_id, "tool_name": tool_name, "arguments": arguments});
        let mut frontier = tree
            .pending_work
            .values()
            .filter_map(|work| match work {
                ControlOperation::Predict(pending) => Some(json!({
                    "edge_id": pending.operation_id,
                    "tool_name": pending.tool_name,
                    "arguments": pending.arguments,
                })),
                _ => None,
            })
            .collect::<Vec<_>>();
        frontier.extend(tree.tool_invocations.iter().map(|pending| {
            json!({
                "edge_id": pending.operation_id,
                "tool_name": pending.tool_name,
                "arguments": pending.arguments,
            })
        }));
        if !frontier.iter().any(|item| item["edge_id"] == operation_id) {
            frontier.push(edge.clone());
        }
        let selected = engine.evaluate(
            policy,
            Decision::Select,
            &json!({"frontier_view": frontier, "scheduling_context": {}}),
        )?;
        if !selected
            .as_array()
            .is_some_and(|ids| ids.contains(&json!(operation_id)))
        {
            return Ok(Vec::new());
        }
        let result = engine.evaluate(policy, Decision::Predict, &json!({
            "edge_context": edge, "process_context": {"operation_id": operation_id, "arguments": arguments, "observation": observation},
            "assumption_set": [], "capability_context": {"tool": tool_name, "runtime_capabilities": tree.capability_view,
                "prediction_types": ["string", "boolean", "enum"]}
        }))?;
        let proposals = result["proposals"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| item.get("payload").cloned())
            .collect();
        Ok(proposals)
    }

    fn launch_allowed(
        &self,
        tree: &SpeculativeTree,
        policy: &CompiledPolicy,
        tool: &ToolPending,
    ) -> bool {
        let authority = tree
            .authority_branch_id
            .as_ref()
            .and_then(|id| tree.branches.get(id));
        let candidates: Vec<_> = tree
            .pending_candidates()
            .iter()
            .map(|id| tree.branches[id].clone())
            .collect();
        PolicyEngine::default()
            .evaluate(
                policy,
                Decision::Launch,
                &json!({
                    "edge_context": {"edge_id": tool.operation_id, "tool_name": tool.tool_name},
                    "authority_state": authority,
                    "candidate_view": candidates
                }),
            )
            .map(|value| value == true)
            .unwrap_or(false)
    }

    pub fn disposition(
        &self,
        policy: &CompiledPolicy,
        proposal: &ProposalRecord,
        outcome: &Value,
    ) -> Result<(bool, String), Box<dyn std::error::Error>> {
        let engine = PolicyEngine::default();
        let result = engine.evaluate(
            policy,
            Decision::Match,
            &json!({
                "proposal": {"payload": proposal.payload}, "evidence": outcome,
                "match_context": {"operation_id": proposal.operation_id}
            }),
        )?;
        let verdict = result["verdict"].as_str().unwrap_or("error").to_string();
        let compatible = verdict == "compatible";
        let disposition = engine.evaluate(
            policy,
            Decision::Dispose,
            &json!({
                "branch": {"operation_id": proposal.operation_id},
                "resolution": if compatible {"accepted"} else {"rejected"},
                "context": {"match": result}
            }),
        )?;
        let promote = disposition["actions"]
            .as_array()
            .is_some_and(|actions| actions.iter().any(|action| action["intent"] == "Promote"));
        Ok((compatible && promote, verdict))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_control::{BranchRole, CommandOperation, CommandStatus};

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
            action_kind: crate::agent_control::ActionKind::ToolExecution,
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
            action_kind: crate::agent_control::ActionKind::ToolExecution,
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
}
