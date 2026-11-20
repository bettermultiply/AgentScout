//! SpeculationCoordinator：推动处理、调用 Policy 并发出操作。
//! 不拥有 Tree 或第二份投机状态，可以服务多棵树。

use super::*;
use crate::control_plane::gateway::agent::protocol::{ActionKind, ActionStatus};

impl SpeculationCoordinator {
    pub fn perform(
        &self,
        tree: &SpeculativeTree,
        policy: &CompiledPolicy,
        engine: &PolicyEngine,
        runtime: &dyn Runtime,
        capabilities: &RuntimeCapabilities,
        work: &ControlOperation,
    ) -> Result<ControlResult, String> {
        let environment_for = |branch: &str| -> Result<&str, String> {
            tree.branch(branch)
                .ok()
                .and_then(|record| record.environment_id())
                .ok_or_else(|| format!("unknown Branch {branch}"))
        };
        let authority = tree.authority_branch_id().unwrap_or("authority");
        let environment = || environment_for(work.branch().unwrap_or(authority));
        let call = |environment: &str, operation| {
            super::execution::runtime_call(runtime, environment, operation)
                .map_err(|error| error.to_string())
        };
        match work {
            ControlOperation::Select(ids) => self
                .select(tree, policy, engine, ids)
                .map(ControlResult::Selected)
                .map_err(|error| error.to_string()),
            ControlOperation::Predict(tool) => {
                let observation = if capabilities
                    .tools
                    .iter()
                    .any(|declared| declared.name == tool.tool_name)
                {
                    call(
                        environment()?,
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
                self.predict(tree, policy, engine, tool, observation.as_ref())
                    .map_err(|error| error.to_string())
            }
            ControlOperation::Launch(tool) => self
                .launch_allowed(tree, policy, engine, tool)
                .map(ControlResult::Launch)
                .map_err(|error| error.to_string()),
            ControlOperation::Rebalance => self
                .rebalance(tree, policy, engine)
                .map(ControlResult::Rebalance)
                .map_err(|error| error.to_string()),
            ControlOperation::Validate(candidate) => {
                let branch = &tree.branches[candidate];
                let chance = branch
                    .chance_id
                    .as_ref()
                    .and_then(|id| tree.chances.get(id))
                    .ok_or("candidate Chance is missing")?;
                // A Candidate without a Proposal has nothing to compare. Do not
                // invent a null Proposal and send it through Policy Match: the
                // missing Proposal is itself the truthful terminal fact.
                let Some(proposal) = tree.branch_proposal(candidate) else {
                    return Ok(ControlResult::Disposition(
                        false,
                        "incompatible".into(),
                        None,
                        None,
                    ));
                };
                self.disposition(
                    policy,
                    engine,
                    tree,
                    candidate,
                    proposal,
                    &chance.operation_id,
                    &tree
                        .candidate_evidence(candidate)
                        .cloned()
                        .unwrap_or(Value::Null),
                    &tree
                        .candidate_outcome(candidate)
                        .cloned()
                        .unwrap_or(Value::Null),
                )
                .map(
                    |(compatible, verdict, notification, validated_at_ms, full, part)| {
                        ControlResult::AcceptanceDisposition(
                            compatible,
                            verdict,
                            notification,
                            validated_at_ms,
                            full,
                            part,
                        )
                    },
                )
                .map_err(|error| error.to_string())
            }
            ControlOperation::Establish(command) => {
                if capabilities.branch_environment {
                    let AgentCommand::StartCandidate {
                        parent_branch_id,
                        branch_id,
                        ..
                    } = command
                    else {
                        unreachable!()
                    };
                    let result = runtime
                        .branch(
                            environment_for(parent_branch_id)?,
                            tree.agent_launch_for(branch_id),
                        )
                        .map_err(|error| error.to_string())?;
                    if result.status != crate::protocol::contract::ResultStatus::Completed {
                        let reason = result
                            .reason
                            .unwrap_or_else(|| "Runtime branch failed".into());
                        if !result.environment_id.is_empty()
                            && result.environment_id != environment_for(parent_branch_id)?
                        {
                            return Ok(ControlResult::EnvironmentFailed(
                                result.environment_id,
                                reason,
                            ));
                        }
                        return Err(reason);
                    }
                    if result.environment_id.is_empty()
                        || tree.branches().values().any(|record| {
                            record.environment_id() == Some(result.environment_id.as_str())
                        })
                    {
                        return Err(
                            "Runtime returned an empty or already bound Environment identity"
                                .into(),
                        );
                    }
                    return Ok(ControlResult::Environment(
                        result.environment_id,
                        result.execution_access,
                    ));
                } else {
                    return Err("Runtime cannot create a Candidate Environment".into());
                }
            }
            ControlOperation::SuspendAcceptance(candidate) => {
                let branch = tree.branch(candidate).map_err(|error| error.to_string())?;
                let parent = tree.acceptance_target(branch).ok_or("acceptance target is no longer eligible")?;
                call(environment_for(parent)?, Operation::SuspendEnvironment)
                    .map(|_| ControlResult::Completed)
            }
            ControlOperation::ResumeAcceptance(parent) => {
                call(environment_for(parent)?, Operation::ResumeEnvironment)
                    .map(|_| ControlResult::Completed)
            }
            ControlOperation::Promote(candidate, record) if record.get("full").is_some() => {
                use crate::protocol::contract::{Acceptance, PartResult, PartSelection, ResultStatus};
                let branch = tree.branch(candidate).map_err(|error| error.to_string())?;
                let parent = tree.chance_parent(branch).ok_or("missing acceptance parent")?;
                let target = environment_for(parent)?;
                let transfer = || {
                if tree.acceptance_target(branch) != Some(parent)
                    || !tree.acceptance_progress_compatible(branch)
                {
                    return Ok(ControlResult::Acceptance(None, None, Some("acceptance target is no longer eligible".into())));
                }
                let full = record["full"] == true;
                let part = if let Some(results) = record["part"]["results"].as_array() {
                    let results = results.iter().map(|result| serde_json::from_value::<PartResult>(json!({
                        "id": result["id"], "files": result["files"],
                    }))).collect::<Result<Vec<_>, _>>().map_err(|error| error.to_string())?;
                    if !PartSelection::valid_results(&results) {
                        return Err("invalid part result dependencies".into());
                    }
                    Some(PartSelection::Results { results })
                } else {
                    record["part"].get("scopes").cloned()
                        .map(serde_json::from_value).transpose()
                        .map_err(|error| error.to_string())?.map(PartSelection::Scopes)
                };
                let result = runtime
                    .call(
                        target,
                        Operation::AcceptEnvironment {
                            source_environment_id: environment()?.into(),
                            full,
                            part: part.clone(),
                            target_suspended: true,
                        },
                    )
                    .map_err(|error| error.to_string())?;
                match result.status {
                    ResultStatus::Completed if result.environment_id == target => {
                        let filesystem =
                            result.filesystem.ok_or("Runtime acceptance data missing")?;
                        match &filesystem.acceptance {
                            Some(Acceptance::Full) if !full => {
                                return Err("Runtime returned unapproved full acceptance".into());
                            }
                            Some(Acceptance::Part { scopes })
                                if !matches!(&part, Some(PartSelection::Scopes(approved))
                                    if scopes.iter().all(|scope| approved.contains(scope))) =>
                            {
                                return Err("Runtime returned unapproved part acceptance".into());
                            }
                            Some(Acceptance::PartResults { results })
                                if !matches!(&part, Some(PartSelection::Results { results: approved })
                                    if !results.is_empty()
                                        && results.iter().collect::<std::collections::BTreeSet<_>>().len() == results.len()
                                        && results.iter().all(|id| approved.iter().any(|result| &result.id == id))) =>
                            {
                                return Err("Runtime returned unapproved part results".into());
                            }
                            _ => {}
                        }
                        if result
                            .execution_access
                            .as_ref()
                            .is_some_and(|access| access.environment_id != target)
                        {
                            return Err("Runtime acceptance execution access has the wrong Environment identity".into());
                        }
                        let reason = (!filesystem.conflicts.is_empty())
                            .then(|| format!("Filesystem conflicts: {:?}", filesystem.conflicts));
                        Ok(ControlResult::Acceptance(
                            filesystem.acceptance,
                            result.execution_access,
                            reason,
                        ))
                    }
                    ResultStatus::Failed
                    | ResultStatus::Unsupported
                    | ResultStatus::NotApplicable => {
                        Ok(ControlResult::Acceptance(None, None, result.reason))
                    }
                    _ => Err(format!(
                        "Runtime acceptance outcome is uncertain: {:?}",
                        result.reason
                    )),
                }
                };
                let result = transfer();
                call(target, Operation::ResumeEnvironment)
                    .map_err(|error| format!("Could not resume acceptance target: {error}"))?;
                result
            }
            ControlOperation::Promote(_, _) => call(environment()?, Operation::PromoteEnvironment)
                .map(|_| ControlResult::Completed),
            ControlOperation::Terminate(_) => {
                if tree.branches[work.branch().unwrap()]
                    .environment_id
                    .is_none()
                {
                    return Ok(ControlResult::Completed);
                }
                call(environment()?, Operation::TerminateEnvironment)
                    .map(|_| ControlResult::Completed)
            }
            ControlOperation::Cleanup(_) => {
                if tree.branches[work.branch().unwrap()]
                    .environment_id
                    .is_none()
                {
                    return Ok(ControlResult::Completed);
                }
                call(environment()?, Operation::CleanupEnvironment)
                    .map(|_| ControlResult::Completed)
            }
        }
    }
}

#[derive(Default)]
pub struct SpeculationCoordinator;

impl SpeculationCoordinator {
    /// `select` sees every Pending Chance and returns an ordered subset.
    /// Chances it leaves out stay Pending.
    pub fn select(
        &self,
        tree: &SpeculativeTree,
        policy: &CompiledPolicy,
        engine: &PolicyEngine,
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
        let selected = engine.evaluate(
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
        engine: &PolicyEngine,
        tool: &ToolPending,
        observation: Option<&Value>,
    ) -> Result<ControlResult, Box<dyn std::error::Error>> {
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
        let result = engine.evaluate(
            policy,
            Decision::Predict,
            &json!({
                "chance_context": {
                    "chance_id": tool.chance_id,
                    "tool_name": tool.tool_name,
                    "arguments": tool.arguments,
                    "kind": tree
                        .chances
                        .get(&tool.chance_id)
                        .and_then(|chance| chance.speculation_kind)
                        .map(|kind| format!("{kind:?}").to_lowercase()),
                },
                "process_context": {
                    "operation_id": tool.operation_id,
                    "arguments": tool.arguments,
                    "observation": observation,
                },
                "assumption_set": assumptions,
                "capability_context": {
                    "tool": tool.tool_name,
                    "runtime_capabilities": tree.capability_view(),
                    "prediction_types": ["string", "boolean", "enum"],
                },
            }),
        )?;
        if result["predict"] == "branch_model" {
            return Ok(ControlResult::BranchModel(result));
        }
        Ok(ControlResult::Predictions(
            result["proposals"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|item| item.get("payload").cloned())
                .collect(),
        ))
    }

    /// `launch` decides whether the authority value is fetched now. `false`
    /// only defers the fetch.
    pub(super) fn launch_allowed(
        &self,
        tree: &SpeculativeTree,
        policy: &CompiledPolicy,
        engine: &PolicyEngine,
        tool: &ToolPending,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let authority = tree.branches.get(&tool.branch_id);
        let candidates: Vec<_> = tree
            .pending_candidates()
            .iter()
            .map(|id| tree.branches[id].clone())
            .collect();
        let allowed = engine.evaluate(
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
        engine: &PolicyEngine,
    ) -> Result<Vec<(String, String)>, Box<dyn std::error::Error>> {
        let view = |branch: &crate::control_plane::speculative_tree::BranchRecord| json!({"branch_id": branch.branch_id, "chance_id": branch.chance_id});
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
        let actions = engine.evaluate(
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
        engine: &PolicyEngine,
        tree: &SpeculativeTree,
        candidate_id: &str,
        proposal: &ProposalRecord,
        operation_id: &str,
        evidence: &Value,
        outcome: &Value,
    ) -> Result<
        (
            bool,
            String,
            Option<String>,
            Option<u64>,
            bool,
            Option<Value>,
        ),
        Box<dyn std::error::Error>,
    > {
        let parent = tree.chance_parent(&tree.branches[candidate_id]);
        let result = engine.evaluate(
            policy,
            Decision::Match,
            &json!({
                "proposal": {"payload": proposal.payload}, "evidence": evidence,
                "match_context": {
                    "operation_id": operation_id,
                    "proposal_pending": proposal.payload.is_null(),
                    "authority": match_facts(tree, parent, operation_id, Some(outcome)),
                    "candidate": match_facts(tree, Some(candidate_id), operation_id, Some(&proposal.payload)),
                }
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
        let candidate_execution = tree
            .transcripts
            .get(candidate_id)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let parent_execution = parent
            .and_then(|parent| tree.transcripts.get(parent))
            .map(Vec::as_slice)
            .unwrap_or_default();
        let inherited = candidate_execution
            .iter()
            .zip(parent_execution)
            .take_while(|(candidate, parent)| candidate == parent)
            .count();
        let mut candidate_results: Vec<_> = tree.action_facts.get(candidate_id).into_iter()
            .flat_map(|facts| facts.values())
            .filter(|fact| fact.status == ActionStatus::Succeeded
                && fact.action_kind == ActionKind::ToolExecution)
            .map(|fact| json!({"action_id": fact.action_id, "result": fact.result, "evidence": fact.evidence}))
            .collect();
        let candidate_execution = &candidate_execution[inherited..];
        candidate_results.extend(
            candidate_execution
                .iter()
                .filter(|entry| entry["role"] == "assistant" && entry["stopReason"] == "stop")
                .map(|entry| json!({"result": entry["content"]})),
        );
        let disposition = engine.evaluate(
            policy,
            Decision::Dispose,
            &json!({
                "branch": {"operation_id": operation_id},
                "resolution": if compatible {"accepted"} else if verdict == "indeterminate" {"indeterminate"} else if verdict == "error" {"error"} else {"rejected"},
                "context": {
                    "match": result,
                    "authority_outcome": outcome,
                    "candidate_proposal": proposal.payload,
                    "candidate_execution": candidate_execution,
                    "candidate_results": candidate_results
                }
            }),
        )?;
        let notification = disposition["actions"]
            .as_array()
            .and_then(|actions| actions.iter().find(|action| action["intent"] == "Promote"))
            .and_then(|action| action.get("notification"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let action = disposition["actions"]
            .as_array()
            .and_then(|actions| actions.iter().find(|action| action["intent"] == "Promote"));
        Ok((
            compatible && action.is_some(),
            verdict,
            notification,
            validated_at_ms,
            action.is_some_and(|action| action["full"] != false),
            action.and_then(|action| action.get("part")).cloned(),
        ))
    }
}

fn match_facts(
    tree: &SpeculativeTree,
    branch_id: Option<&str>,
    operation_id: &str,
    result: Option<&Value>,
) -> Value {
    let branch = branch_id.and_then(|id| tree.branches.get(id));
    let usage = branch_id.and_then(|id| tree.model_usage.get(id));
    let fact = branch_id
        .and_then(|id| tree.action_facts.get(id))
        .and_then(|facts| facts.get(operation_id));
    let fact_result = fact.and_then(|item| item.result.as_ref());
    let fact_evidence = fact.and_then(|item| item.evidence.as_ref());
    let selected_result = result
        .filter(|value| !value.is_null())
        .or(fact_result)
        .or_else(|| branch_id.and_then(|id| tree.candidate_outcome(id)));
    let selected_evidence =
        fact_evidence.or_else(|| branch_id.and_then(|id| tree.candidate_evidence(id)));
    let selected_tool_calls = fact
        .and_then(|item| item.tool_calls.as_ref())
        .or_else(|| selected_result.and_then(|item| item.get("tool_calls")))
        .or_else(|| selected_evidence.and_then(|item| item.get("tool_calls")));
    let selected_action_id = fact
        .map(|item| item.action_id.as_str())
        .unwrap_or(operation_id);
    let selected_model = usage.and_then(|items| {
        items
            .iter()
            .find(|item| item.action_id.as_deref() == Some(selected_action_id))
            .map(|item| item.model.as_str())
    });
    let speculation_kind = branch_id
        .and_then(|id| tree.branches.get(id))
        .and_then(|branch| branch.chance_id.as_ref())
        .and_then(|id| tree.chances.get(id))
        .and_then(|chance| chance.speculation_kind);
    json!({
        "branch": branch,
        "action_id": selected_action_id,
        "action_kind": fact.map(|item| item.action_kind),
        "status": fact.map(|item| item.status),
        "speculation_kind": speculation_kind,
        "model": selected_model,
        "result": selected_result,
        "evidence": selected_evidence,
        "usage": usage,
        "tool_calls": selected_tool_calls,
        "pairing": {"operation_id": operation_id, "action_id": selected_action_id, "branch_id": branch_id},
    })
}
