//! AgentController：把控制结果落到 Tree。

use super::*;

impl AgentController {
    pub fn perform(
        &self,
        tree: &SpeculativeTree,
        policy: &CompiledPolicy,
        runtime: &dyn Runtime,
        capabilities: &RuntimeCapabilities,
        work: &ControlOperation,
    ) -> Result<ControlResult, String> {
        let environment = if capabilities.shared_environment {
            "authority"
        } else {
            work.branch().unwrap_or("authority")
        };
        let call = |environment: &str, operation| {
            let result = runtime
                .call(environment, operation)
                .map_err(|error| error.to_string())?;
            if result.status != crate::protocol::contract::ResultStatus::Completed {
                return Err(format!("Runtime request failed: {:?}", result.reason));
            }
            Ok(result)
        };
        match work {
            ControlOperation::Select(ids) => self
                .select(tree, policy, ids)
                .map(ControlResult::Selected)
                .map_err(|error| error.to_string()),
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
                self.predict(tree, policy, tool, observation.as_ref())
                    .map(ControlResult::Predictions)
                    .map_err(|error| error.to_string())
            }
            ControlOperation::Launch(tool) => self
                .launch_allowed(tree, policy, tool)
                .map(ControlResult::Launch)
                .map_err(|error| error.to_string()),
            ControlOperation::Rebalance => self
                .rebalance(tree, policy)
                .map(ControlResult::Rebalance)
                .map_err(|error| error.to_string()),
            ControlOperation::Validate(candidate) => {
                let branch = &tree.branches[candidate];
                let chance = branch
                    .chance_id
                    .as_ref()
                    .and_then(|id| tree.chances.get(id))
                    .ok_or("candidate Chance is missing")?;
                self.disposition(
                    policy,
                    tree.branch_proposal(candidate)
                        .ok_or("candidate Proposal is missing")?,
                    &chance.operation_id,
                    &tree.candidate_evidence(candidate).cloned().unwrap_or(Value::Null),
                    &tree
                        .candidate_outcome(candidate)
                        .cloned()
                        .unwrap_or(Value::Null),
                )
                .map(|(compatible, verdict, notification, validated_at_ms)| {
                    ControlResult::Disposition(compatible, verdict, notification, validated_at_ms)
                })
                .map_err(|error| error.to_string())
            }
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
    /// `select` sees every Pending Chance and returns an ordered subset.
    /// Chances it leaves out stay Pending.
    pub fn select(
        &self,
        tree: &SpeculativeTree,
        policy: &CompiledPolicy,
        ids: &[String],
    ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let frontier: Vec<Value> = ids
            .iter()
            .filter_map(|id| tree.chances.get(id))
            .map(|chance| {
                json!({
                    "chance_id": chance.chance_id,
                    "tool_name": chance.tool_name,
                    "arguments": chance.arguments,
                })
            })
            .collect();
        let selected = PolicyEngine::default().evaluate(
            policy,
            Decision::Select,
            &json!({"frontier_view": frontier, "scheduling_context": {}}),
        )?;
        Ok(selected
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|id| id.as_str().map(str::to_string))
            .collect())
    }

    pub fn predict(
        &self,
        tree: &SpeculativeTree,
        policy: &CompiledPolicy,
        tool: &ToolPending,
        observation: Option<&Value>,
    ) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
        let assumptions: Vec<Value> = tree
            .branches
            .get(&tool.branch_id)
            .into_iter()
            .flat_map(|branch| &branch.assumption_set)
            .map(|assumption| {
                json!({
                    "chance_id": assumption.chance_id,
                    "proposal_id": assumption.proposal_id,
                })
            })
            .collect();
        let result = PolicyEngine::default().evaluate(
            policy,
            Decision::Predict,
            &json!({
                "chance_context": {
                    "chance_id": tool.chance_id,
                    "tool_name": tool.tool_name,
                    "arguments": tool.arguments,
                },
                "process_context": {
                    "operation_id": tool.operation_id,
                    "arguments": tool.arguments,
                    "observation": observation,
                },
                "assumption_set": assumptions,
                "capability_context": {
                    "tool": tool.tool_name,
                    "runtime_capabilities": tree.capability_view,
                    "prediction_types": ["string", "boolean", "enum"],
                },
            }),
        )?;
        Ok(result["proposals"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| item.get("payload").cloned())
            .collect())
    }

    /// `launch` decides whether the authority value is fetched now. `false`
    /// only defers the fetch.
    pub(super) fn launch_allowed(
        &self,
        tree: &SpeculativeTree,
        policy: &CompiledPolicy,
        tool: &ToolPending,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let authority = tree
            .authority_branch_id
            .as_ref()
            .and_then(|id| tree.branches.get(id));
        let candidates: Vec<_> = tree
            .pending_candidates()
            .iter()
            .map(|id| tree.branches[id].clone())
            .collect();
        let allowed = PolicyEngine::default().evaluate(
            policy,
            Decision::Launch,
            &json!({
                "chance_context": {"chance_id": tool.chance_id, "tool_name": tool.tool_name},
                "authority_state": authority,
                "candidate_view": candidates
            }),
        )?;
        Ok(allowed == true)
    }

    /// `rebalance` adjusts started candidate work: pause, resume or preempt.
    pub fn rebalance(
        &self,
        tree: &SpeculativeTree,
        policy: &CompiledPolicy,
    ) -> Result<Vec<(String, String)>, Box<dyn std::error::Error>> {
        let view = |branch: &crate::control_plane::agent_control::BranchRecord| {
            json!({"branch_id": branch.branch_id, "chance_id": branch.chance_id})
        };
        let candidates = || {
            tree.branches
                .values()
                .filter(|branch| branch.role == BranchRole::Candidate)
        };
        let active: Vec<_> = candidates()
            .filter(|branch| branch.status == BranchStatus::Running)
            .map(view)
            .collect();
        let paused: Vec<_> = candidates()
            .filter(|branch| branch.status == BranchStatus::Blocked(BlockedReason::Rebalanced))
            .map(view)
            .collect();
        let actions = PolicyEngine::default().evaluate(
            policy,
            Decision::Rebalance,
            &json!({"active_view": active, "paused_view": paused, "scheduling_context": {}}),
        )?;
        Ok(actions
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| {
                Some((
                    item["branch_id"].as_str()?.to_string(),
                    item["action"].as_str()?.to_string(),
                ))
            })
            .collect())
    }

    pub fn disposition(
        &self,
        policy: &CompiledPolicy,
        proposal: &ProposalRecord,
        operation_id: &str,
        evidence: &Value,
        outcome: &Value,
    ) -> Result<(bool, String, Option<String>, Option<u64>), Box<dyn std::error::Error>> {
        let engine = PolicyEngine::default();
        let result = engine.evaluate(
            policy,
            Decision::Match,
            &json!({
                "proposal": {"payload": proposal.payload}, "evidence": evidence,
                "match_context": {"operation_id": operation_id}
            }),
        )?;
        // Match is the validation operation; Dispose and Promote may take
        // arbitrarily longer. Capture here before either is invoked.
        let validated_at_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .and_then(|duration| u64::try_from(duration.as_millis()).ok());
        let verdict = result["verdict"].as_str().unwrap_or("error").to_string();
        let compatible = verdict == "compatible";
        let disposition = engine.evaluate(
            policy,
            Decision::Dispose,
            &json!({
                "branch": {"operation_id": operation_id},
                "resolution": if compatible {"accepted"} else {"rejected"},
                "context": {
                    "match": result,
                    "authority_outcome": outcome
                }
            }),
        )?;
        let notification = disposition["actions"]
            .as_array()
            .and_then(|actions| actions.iter().find(|action| action["intent"] == "Promote"))
            .and_then(|action| action.get("notification"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let promote = disposition["actions"]
            .as_array()
            .is_some_and(|actions| actions.iter().any(|action| action["intent"] == "Promote"));
        Ok((compatible && promote, verdict, notification, validated_at_ms))
    }
}
