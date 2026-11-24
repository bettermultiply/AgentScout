
use super::*;
use crate::control_plane::gateway::agent::protocol::{ActionKind, ActionStatus};
use crate::control_plane::policy::ModelProviderProcess;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

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
        let call = |branch, operation| {
            super::execution::runtime_call(runtime, Self::environment_for(tree, branch)?, operation)
                .map(|_| ControlResult::Completed)
                .map_err(|error| error.to_string())
        };
        match work {
            ControlOperation::Select(ids) => self
                .select(tree, policy, engine, ids)
                .map(ControlResult::Selected)
                .map_err(|error| error.to_string()),
            ControlOperation::Predict(tool) => {
                self.perform_predict(tree, policy, engine, runtime, capabilities, tool)
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
                self.perform_validate_or_dispose(tree, policy, engine, candidate, None)
            }
            ControlOperation::Dispose(candidate, resolution) => {
                self.perform_validate_or_dispose(tree, policy, engine, candidate, Some(resolution))
            }
            ControlOperation::Establish(command) => {
                Self::perform_establish(tree, runtime, capabilities, command)
            }
            ControlOperation::SuspendAcceptance(candidate) => {
                let branch = tree.branch(candidate).map_err(|error| error.to_string())?;
                let parent = tree
                    .transfer_target(branch, tree.pending_partial_acceptance(candidate))
                    .ok_or("acceptance target is no longer eligible")?;
                call(parent, Operation::SuspendEnvironment)
            }
            ControlOperation::ResumeAcceptance(parent) => {
                call(parent, Operation::ResumeEnvironment)
            }
            ControlOperation::Promote(candidate, record) if record.get("full").is_some() => {
                Self::perform_accept(tree, runtime, candidate, record)
            }
            ControlOperation::Promote(candidate, _) => {
                call(candidate, Operation::PromoteEnvironment)
            }
            ControlOperation::Terminate(branch) => Self::perform_environment_lifecycle(
                tree,
                runtime,
                branch,
                Operation::TerminateEnvironment,
            ),
            ControlOperation::Cleanup(branch) => Self::perform_environment_lifecycle(
                tree,
                runtime,
                branch,
                Operation::CleanupEnvironment,
            ),
        }
    }

    fn environment_for<'a>(tree: &'a SpeculativeTree, branch: &str) -> Result<&'a str, String> {
        tree.branch(branch)
            .ok()
            .and_then(|record| record.environment_id())
            .ok_or_else(|| format!("unknown Branch {branch}"))
    }

    fn perform_predict(
        &self,
        tree: &SpeculativeTree,
        policy: &CompiledPolicy,
        engine: &PolicyEngine,
        runtime: &dyn Runtime,
        capabilities: &RuntimeCapabilities,
        tool: &ToolPending,
    ) -> Result<ControlResult, String> {
        let observation = if capabilities
            .tools
            .iter()
            .any(|declared| declared.name == tool.tool_name)
        {
            super::execution::runtime_call(
                runtime,
                Self::environment_for(tree, &tool.branch_id)?,
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

    fn perform_validate_or_dispose(
        &self,
        tree: &SpeculativeTree,
        policy: &CompiledPolicy,
        engine: &PolicyEngine,
        candidate: &str,
        resolution: Option<&str>,
    ) -> Result<ControlResult, String> {
        let branch = tree.branch(candidate).map_err(|error| error.to_string())?;
        let chance = branch
            .chance_id
            .as_ref()
            .and_then(|id| tree.chances.get(id))
            .ok_or("candidate Chance is missing")?;
        let Some(proposal) = tree.branch_proposal(candidate) else {
            return Ok(ControlResult::Disposition(
                false,
                "incompatible".into(),
                None,
                None,
            ));
        };
        self.disposition_for_resolution(
            policy,
            engine,
            tree,
            candidate,
            proposal,
            &chance.operation_id,
            tree.candidate_evidence(candidate).unwrap_or(&Value::Null),
            tree.candidate_outcome(candidate).unwrap_or(&Value::Null),
            resolution,
        )
        .map(
            |(compatible, verdict, notification, validated_at_ms, full, part, audit)| {
                ControlResult::AcceptanceDisposition(
                    compatible,
                    verdict,
                    notification,
                    validated_at_ms,
                    full,
                    part,
                    audit,
                )
            },
        )
        .map_err(|error| error.to_string())
    }

    fn perform_establish(
        tree: &SpeculativeTree,
        runtime: &dyn Runtime,
        capabilities: &RuntimeCapabilities,
        command: &AgentCommand,
    ) -> Result<ControlResult, String> {
        if !capabilities.branch_environment {
            return Err("Runtime cannot create a Candidate Environment".into());
        }
        let AgentCommand::StartCandidate {
            parent_branch_id,
            branch_id,
            ..
        } = command
        else {
            return Err("Establish requires a StartCandidate command".into());
        };
        let parent = Self::environment_for(tree, parent_branch_id)?;
        let result = runtime
            .branch(parent, tree.agent_launch_for(branch_id))
            .map_err(|error| error.to_string())?;
        if result.status != crate::protocol::contract::ResultStatus::Completed {
            let reason = result
                .reason
                .unwrap_or_else(|| "Runtime branch failed".into());
            if !result.environment_id.is_empty() && result.environment_id != parent {
                return Ok(ControlResult::EnvironmentFailed(
                    result.environment_id,
                    reason,
                ));
            }
            return Err(reason);
        }
        if result.environment_id.is_empty()
            || tree
                .branches()
                .values()
                .any(|record| record.environment_id() == Some(result.environment_id.as_str()))
        {
            return Err("Runtime returned an empty or already bound Environment identity".into());
        }
        Ok(ControlResult::Environment(
            result.environment_id,
            result.execution_access,
        ))
    }

    fn perform_accept(
        tree: &SpeculativeTree,
        runtime: &dyn Runtime,
        candidate: &str,
        record: &Value,
    ) -> Result<ControlResult, String> {
        use crate::protocol::contract::{Acceptance, PartResult, PartSelection, ResultStatus};
        let branch = tree.branch(candidate).map_err(|error| error.to_string())?;
        let parent = tree
            .chance_parent(branch)
            .ok_or("missing acceptance parent")?;
        let target = Self::environment_for(tree, parent)?;
        let transfer = || {
            let full = record["full"] == true;
            if tree.transfer_target(branch, !full) != Some(parent) {
                return Ok(ControlResult::Acceptance(
                    None,
                    None,
                    Some("acceptance target is no longer eligible".into()),
                ));
            }
            let part = if let Some(results) = record["part"]["results"].as_array() {
                let results = results
                    .iter()
                    .map(|result| {
                        serde_json::from_value::<PartResult>(json!({
                            "id": result["id"], "files": result["files"],
                        }))
                    })
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| error.to_string())?;
                if !PartSelection::valid_results(&results) {
                    return Err("invalid part result dependencies".into());
                }
                Some(PartSelection::Results { results })
            } else {
                record["part"]
                    .get("scopes")
                    .cloned()
                    .map(serde_json::from_value)
                    .transpose()
                    .map_err(|error| error.to_string())?
                    .map(PartSelection::Scopes)
            };
            let result = runtime
                .call(
                    target,
                    Operation::AcceptEnvironment {
                        source_environment_id: Self::environment_for(tree, candidate)?.into(),
                        full,
                        part: part.clone(),
                        target_suspended: true,
                    },
                )
                .map_err(|error| error.to_string())?;
            match result.status {
                ResultStatus::Completed if result.environment_id == target => {
                    let filesystem = result.filesystem.ok_or("Runtime acceptance data missing")?;
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
                ResultStatus::Failed | ResultStatus::Unsupported | ResultStatus::NotApplicable => {
                    Ok(ControlResult::Acceptance(None, None, result.reason))
                }
                _ => Err(format!(
                    "Runtime acceptance outcome is uncertain: {:?}",
                    result.reason
                )),
            }
        };
        let result = transfer();
        super::execution::runtime_call(runtime, target, Operation::ResumeEnvironment)
            .map_err(|error| format!("Could not resume acceptance target: {error}"))?;
        result
    }

    fn perform_environment_lifecycle(
        tree: &SpeculativeTree,
        runtime: &dyn Runtime,
        branch: &str,
        operation: Operation,
    ) -> Result<ControlResult, String> {
        let branch = tree.branch(branch).map_err(|error| error.to_string())?;
        if let Some(environment) = branch.environment_id() {
            super::execution::runtime_call(runtime, environment, operation)
                .map_err(|error| error.to_string())?;
        }
        Ok(ControlResult::Completed)
    }
}

#[derive(Clone, Default)]
pub struct SpeculationCoordinator {
    provider: Option<Arc<ModelProviderProcess>>,
    cancelled: Arc<AtomicBool>,
}

impl SpeculationCoordinator {
    pub fn with_provider(provider: ModelProviderProcess) -> Self {
        Self {
            provider: Some(Arc::new(provider)),
            ..Self::default()
        }
    }

    pub(super) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub(super) fn interrupt(&mut self) {
        self.cancel();
        self.cancelled = Arc::default();
    }

    fn evaluate(
        &self,
        engine: &PolicyEngine,
        policy: &CompiledPolicy,
        decision: Decision,
        data: &Value,
    ) -> io::Result<Value> {
        let span = tracing::info_span!(target: "ascout_otel", "ascout.policy.evaluate",
            decision = decision.as_str(), result = tracing::field::Empty, allowed = tracing::field::Empty,
            result_count = tracing::field::Empty);
        crate::telemetry::parent(&span, crate::telemetry::current());
        let result = span.in_scope(|| self.evaluate_inner(engine, policy, decision, data));
        if let Ok(value) = &result {
            if let Some(allowed) = value.as_bool() { span.record("allowed", allowed); }
            if let Some(items) = value.as_array() { span.record("result_count", items.len()); }
            let outcome = value["actions"][0]["intent"].as_str()
                .or_else(|| value["verdict"].as_str()).or_else(|| value.as_str());
            if let Some(outcome @ ("Promote" | "PartialPromote" | "Discard" | "Retain" | "compatible" | "incompatible" | "indeterminate" | "continue" | "stop")) = outcome {
                span.record("result", outcome);
            }
        }
        crate::telemetry::finish(&span, &result);
        result
    }

    fn evaluate_inner(
        &self, engine: &PolicyEngine, policy: &CompiledPolicy,
        decision: Decision, data: &Value,
    ) -> io::Result<Value> {
        if let Some(grant) = policy
            .policy_model_grant()
            .filter(|grant| grant.decisions.contains(&decision))
        {
            let Some(provider) = self.provider.as_ref() else {
                return engine.evaluate(policy, decision, data);
            };
            engine.evaluate_with_provider_cancellable(
                policy,
                decision,
                data,
                &grant.models,
                grant.max_requests,
                provider,
                &self.cancelled,
                Some(Duration::from_millis(grant.timeout_ms)),
            )
        } else {
            engine.evaluate(policy, decision, data)
        }
    }
    pub fn select(
        &self,
        tree: &SpeculativeTree,
        policy: &CompiledPolicy,
        engine: &PolicyEngine,
        ids: &[String],
    ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let mut frontier: Vec<_> = ids.iter().filter_map(|id| tree.chances.get(id)).collect();
        frontier.sort_by_key(|chance| (chance.speculation_started_at_ms.unwrap_or(u64::MAX),
            chance.chance_id.strip_prefix("chance-").and_then(|value| value.parse::<u64>().ok())));
        let frontier: Vec<Value> = frontier.into_iter().map(|chance| {
                json!({
                    "chance_id": chance.chance_id,
                    "tool_name": chance.tool_name,
                    "arguments": chance.arguments,
                    "assumption_depth": tree.branches[&chance.parent_branch_id].assumption_set.len(),
                    "speculation_kind": chance.speculation_kind,
                })
            })
            .collect();
        let active = tree.pending_candidates();
        let blocked = active.iter().filter(|id| {
            matches!(tree.branches[*id].status, BranchStatus::Blocked(_))
        }).count();
        let pending = tree.chances.values().filter(|chance| {
            chance.stage == crate::control_plane::speculative_tree::ChanceStage::Pending
        }).count();
        let branches: Vec<_> = active.iter().map(|id| candidate_summary(tree, &tree.branches[id])).collect();
        let selected = self.evaluate(
            engine,
            policy,
            Decision::Select,
            &json!({"frontier_view": frontier, "scheduling_context": {
                "active_candidate_count": active.len(),
                "blocked_candidate_count": blocked,
                "pending_chance_count": pending,
                "active_branches": branches,
            }}),
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
        let mut data = json!({
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
                    "workspace": tree.workspace(),
                    "observation": observation,
                    "active_branches": tree.pending_candidates().iter()
                        .map(|id| candidate_summary(tree, &tree.branches[id])).collect::<Vec<_>>(),
                    "authority_active_tools": active_tools(tree, tree.authority_branch_id.as_deref()),
                    "steps_completed": completed_rounds(tree, &tool.branch_id),
                },
                "assumption_set": assumptions,
                "capability_context": {
                    "tool": tool.tool_name,
                    "runtime_capabilities": tree.capability_view(),
                    "prediction_types": ["string", "boolean", "enum"],
                },
        });
        if declares_input(policy, Decision::Predict, "history") {
            data["history"] = json!(branch_history(tree, &tool.branch_id, &tool.operation_id, policy.history_max_entries()));
        }
        let result = self.evaluate(engine, policy, Decision::Predict, &data)?;
        if result["predict"] == "branch_model" {
            return Ok(ControlResult::BranchModel(result));
        }
        if result["abstain"] == true {
            return Ok(ControlResult::Abstain(result["reason"].as_str().map(str::to_owned)));
        }
        let Value::Object(mut result) = result else {
            return Err("Predict result is not an object".into());
        };
        let Some(Value::Array(proposals)) = result.remove("proposals") else {
            return Err("Predict result is missing a proposals array".into());
        };
        Ok(ControlResult::Predictions(proposals))
    }

    pub(super) fn launch_allowed(
        &self,
        tree: &SpeculativeTree,
        policy: &CompiledPolicy,
        engine: &PolicyEngine,
        tool: &ToolPending,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let authority = tree.branches.get(&tool.branch_id).map_or_else(|| json!({}), |branch| {
            json!({
                "branch_id": branch.branch_id,
                "status": branch.status,
                "assumption_depth": branch.assumption_set.len(),
                "turn_active": branch.execution_state.turn_active,
                "steps_completed": completed_rounds(tree, &tool.branch_id),
            })
        });
        let candidates: Vec<_> = tree
            .pending_candidates()
            .iter()
            .map(|id| candidate_summary(tree, &tree.branches[id]))
            .collect();
        let allowed = self.evaluate(
            engine,
            policy,
            Decision::Launch,
            &json!({
                "chance_context": {
                    "chance_id": tool.chance_id,
                    "tool_name": tool.tool_name,
                    "arguments": tool.arguments,
                    "kind": tree.chances.get(&tool.chance_id).and_then(|chance| chance.speculation_kind),
                },
                "authority_state": authority,
                "candidate_view": candidates
            }),
        )?;
        Ok(allowed == true)
    }

    pub fn rebalance(
        &self,
        tree: &SpeculativeTree,
        policy: &CompiledPolicy,
        engine: &PolicyEngine,
    ) -> Result<Vec<(String, String)>, Box<dyn std::error::Error>> {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_millis() as u64);
        let view = |branch: &crate::control_plane::speculative_tree::BranchRecord| {
            let facts = tree.action_facts.get(&branch.branch_id);
            let count = |status| facts.into_iter().flat_map(|facts| facts.values())
                .filter(|fact| fact.action_kind == ActionKind::ToolExecution && fact.status == status).count();
            let started_at_ms = tree.speculations.get(&branch.branch_id).map_or(0, |branch| branch.started_at_ms);
            let speculation_kind = branch.chance_id.as_ref().and_then(|id| tree.chances.get(id))
                .and_then(|chance| chance.speculation_kind);
            json!({"branch_id": branch.branch_id, "chance_id": branch.chance_id,
                "started_at_ms": started_at_ms,
                "speculation_kind": speculation_kind,
                "steps_completed": completed_rounds(tree, &branch.branch_id),
                "yield_only": branch.yield_only,
                "tool_counts": {"succeeded": count(ActionStatus::Succeeded), "failed": count(ActionStatus::Failed)},
                "tokens_used": tree.model_usage.get(&branch.branch_id).into_iter().flatten().map(|usage| usage.tokens_used).sum::<u64>(),
                "age_ms": now.saturating_sub(started_at_ms),
            })
        };
        let candidates = || {
            tree.branches
                .values()
                .filter(|branch| branch.role == BranchRole::Candidate)
        };
        let active: Vec<_> = candidates()
            .filter(|branch| branch.is_running_candidate())
            .map(view)
            .collect();
        let paused: Vec<_> = candidates()
            .filter(|branch| branch.status == BranchStatus::Blocked(BlockedReason::Rebalanced))
            .map(view)
            .collect();
        let authority = tree.authority_branch_id.as_deref();
        let recent_actions = authority.map_or_else(Vec::new, |id| recent_completed_actions(tree, id, "", 30));
        let actions = self.evaluate(
            engine,
            policy,
            Decision::Rebalance,
            &json!({"active_view": active, "paused_view": paused,
                "scheduling_context": {
                    "waiting_chance": !tree.selectable_chances().is_empty(),
                    "authority_recent_actions": recent_actions,
                    "authority_active_tools": active_tools(tree, authority),
                }}),
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
            Option<Value>,
        ),
        Box<dyn std::error::Error>,
    > {
        self.disposition_for_resolution(policy, engine, tree, candidate_id, proposal, operation_id, evidence, outcome, None)
    }

    fn disposition_for_resolution(
        &self,
        policy: &CompiledPolicy,
        engine: &PolicyEngine,
        tree: &SpeculativeTree,
        candidate_id: &str,
        proposal: &ProposalRecord,
        operation_id: &str,
        evidence: &Value,
        outcome: &Value,
        resolution: Option<&str>,
    ) -> Result<
        (
            bool,
            String,
            Option<String>,
            Option<u64>,
            bool,
            Option<Value>,
            Option<Value>,
        ),
        Box<dyn std::error::Error>,
    > {
        let branch = tree.branch(candidate_id)?;
        let parent = tree.chance_parent(branch);
        let proposal_view = json!({"id": proposal.proposal_id, "payload": proposal.payload, "provenance": proposal.provenance, "model_options": proposal.model_options});
        let mut data = json!({
                "proposal": proposal_view, "evidence": evidence,
                "match_context": {
                    "operation_id": operation_id,
                    "proposal_pending": proposal.payload.is_null(),
                    "authority": match_facts(tree, parent, operation_id, Some(outcome)),
                    "candidate": match_facts(tree, Some(candidate_id), operation_id, Some(&proposal.payload)),
                }
        });
        if declares_input(policy, Decision::Match, "outcome") {
            data["outcome"] = outcome.clone();
        }
        let mut result = Value::Null;
        // Match is the validation operation; Dispose and Promote may take
        // arbitrarily longer. Capture here before either is invoked.
        let validated_at_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .and_then(|duration| u64::try_from(duration.as_millis()).ok());
        let candidate_execution = tree
            .transcripts
            .get(candidate_id)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let parent_execution = parent
            .and_then(|parent| tree.transcripts.get(parent))
            .map(Vec::as_slice)
            .unwrap_or_default();
        let task_prompt: String = parent_execution
            .iter()
            .find(|entry| entry["role"] == "user")
            .map(|entry| {
                let content = &entry["content"];
                match content.as_array() {
                    Some(parts) => parts
                        .iter()
                        .filter(|part| part["type"] == "text")
                        .filter_map(|part| part["text"].as_str())
                        .collect::<Vec<_>>()
                        .join("\n"),
                    None => content.as_str().unwrap_or_default().to_owned(),
                }
            })
            .unwrap_or_default()
            .chars()
            .take(6000)
            .collect();
        let chance_id = &branch.chance_id;
        let sibling_candidate_count = tree
            .pending_candidates()
            .iter()
            .filter(|id| {
                id.as_str() != candidate_id
                    && tree
                        .branch(id)
                        .is_ok_and(|sibling| &sibling.chance_id == chance_id)
            })
            .count();
        data["match_context"]["task_prompt"] = json!(task_prompt);
        data["match_context"]["sibling_candidate_count"] = json!(sibling_candidate_count);
        let inherited = candidate_execution
            .iter()
            .zip(parent_execution)
            .take_while(|(candidate, parent)| candidate == parent)
            .count();
        let candidate_execution = &candidate_execution[inherited..];
        data["match_context"]["candidate_results"] =
            Value::Array(candidate_results(tree, candidate_id, candidate_execution));
        data["match_context"]["workspace"] = json!(tree.workspace());
        if declares_input(policy, Decision::Predict, "history") {
            if let Some(parent) = parent {
                data["match_context"]["authority_history"] =
                    json!(branch_history(tree, parent, "", policy.history_max_entries()));
            }
        }
        if resolution.is_none() {
            result = self.evaluate(engine, policy, Decision::Match, &data)?;
        }
        let yield_only = branch.yield_only;
        let verdict = result["verdict"]
            .as_str()
            .unwrap_or("indeterminate")
            .to_string();
        let explicit_resolution = resolution.is_some();
        let resolution = resolution.unwrap_or(match (yield_only, verdict.as_str()) {
            (true, _) => "rejected",
            (_, "compatible") => "accepted",
            (_, "indeterminate") => "indeterminate",
            (_, "error") => "error",
            _ => "rejected",
        });
        let accepted = resolution == "accepted";
        let disposition = self.evaluate(
            engine,
            policy,
            Decision::Dispose,
            &json!({
                "branch": {"operation_id": operation_id, "branch_id": candidate_id, "proposal": proposal_view},
                "resolution": resolution,
                "context": {
                    "match": result,
                    "workspace": tree.workspace(),
                    "authority_outcome": outcome,
                    "candidate_proposal": proposal_view,
                    "candidate": data["match_context"]["candidate"],
                    "candidate_execution": candidate_execution,
                    "candidate_results": data["match_context"]["candidate_results"],
                    "authority_history": data["match_context"]["authority_history"]
                }
            }),
        )?;
        let action = &disposition["actions"][0];
        let notification = action["notification"].as_str().map(str::to_owned);
        let full = action["intent"] == "Promote";
        let part = resolve_partial_acceptance(tree, candidate_id, action, notification.as_deref(), accepted)?;
        let verdict = if explicit_resolution && action.is_null() {
            "wait".into()
        } else if explicit_resolution && part.is_none() {
            "discard".into()
        } else {
            verdict
        };
        let audit = result["details"]["audit"].as_str().map(|finding| {
            json!({
                "finding": finding,
                "model": result["details"]["audit_model"],
                "prompt_chars": result["details"]["audit_prompt_chars"],
                "reply_chars": result["details"]["audit_reply_chars"],
            })
        });
        Ok((
            accepted && (full || part.is_some()),
            verdict,
            notification,
            validated_at_ms,
            full,
            part,
            audit,
        ))
    }
}

fn completed_rounds(tree: &SpeculativeTree, branch: &str) -> usize {
    let Some(facts) = tree.action_facts.get(branch) else { return 0 };
    let active = tree.active_actions.get(branch);
    let active_tools = active.is_some_and(|actions| actions.values().any(|action| action.action_kind == ActionKind::ToolExecution));
    let mut models: Vec<_> = facts.values().filter(|fact| fact.action_kind == ActionKind::ModelInference).collect();
    models.sort_by_key(|fact| (fact.timestamp_ms, &fact.action_id));
    let finished_tool = |id: &str| !active.is_some_and(|actions| actions.contains_key(id)) && facts.get(id)
        .is_some_and(|fact| fact.action_kind == ActionKind::ToolExecution && fact.status != ActionStatus::Cancelled);
    let leading_tools = facts.values().filter(|fact| fact.action_kind == ActionKind::ToolExecution
        && fact.status != ActionStatus::Cancelled
        && models.first().is_none_or(|model| fact.timestamp_ms <= model.timestamp_ms)).count();
    let injected_round = tree.branch_proposal(branch).is_some_and(|proposal| {
        proposal.payload["kind"] == "predicted_tool_calls"
            && proposal.payload["tool_calls"].as_array().is_some_and(|calls| !calls.is_empty() && leading_tools >= calls.len())
            && (!models.is_empty() || !active_tools)
    });
    usize::from(injected_round) + models.iter().enumerate().filter(|(index, model)| {
        model.status != ActionStatus::Cancelled && match model.tool_calls.as_ref().and_then(Value::as_array) {
            Some(calls) => calls.iter().all(|call| call["id"].as_str().is_some_and(finished_tool)),
            None => *index + 1 < models.len() || !active_tools,
        }
    }).count()
}

fn candidate_results(
    tree: &SpeculativeTree,
    candidate_id: &str,
    candidate_execution: &[Value],
) -> Vec<Value> {
    let mut completed: Vec<_> = tree
        .action_facts
        .get(candidate_id)
        .into_iter()
        .flat_map(|facts| facts.values())
        .filter(|fact| {
            matches!(fact.status, ActionStatus::Succeeded | ActionStatus::Failed)
                && matches!(fact.action_kind, ActionKind::ToolExecution | ActionKind::ModelInference)
        })
        .collect();
    completed.sort_by(|a, b| (a.timestamp_ms, &a.action_id).cmp(&(b.timestamp_ms, &b.action_id)));
    let legacy_transcript = !completed.iter().any(|fact| fact.action_kind == ActionKind::ModelInference
        && fact.result.as_ref().is_some_and(|result| result.get("content").is_some()));
    let mut candidate_results: Vec<_> = completed
        .into_iter()
        .map(|fact| {
            let mut entry = json!({"action_id": fact.action_id, "timestamp_ms": fact.timestamp_ms,
                "action_kind": fact.action_kind, "started_at_ms": fact.started_at_ms,
                "status": fact.status, "result": fact.result, "evidence": fact.evidence});
            for key in ["tool_name", "arguments", "details"] {
                if let Some(value) = fact.result.as_ref().and_then(|result| result.get(key)) {
                    entry[key] = value.clone();
                }
            }
            entry
        })
        .collect();
    if legacy_transcript {
        candidate_results.extend(candidate_execution
            .iter()
            .filter(|entry| entry["role"] == "assistant" && entry["stopReason"] == "stop")
            .map(|entry| json!({"result": entry["content"]})));
    }
    candidate_results
}

fn resolve_partial_acceptance(
    tree: &SpeculativeTree,
    candidate_id: &str,
    action: &Value,
    notification: Option<&str>,
    accepted: bool,
) -> Result<Option<Value>, Box<dyn std::error::Error>> {
    if !matches!(action["intent"].as_str(), Some("Salvage" | "Yield")) {
        return Ok(None);
    }
    let mut part = action.clone();
    if let Some(results) = part["results"].as_array_mut() {
        for result in results {
            let fact = tree
                .action_facts
                .get(candidate_id)
                .and_then(|facts| facts.get(result["id"].as_str()?))
                .filter(|fact| {
                    let notification_only = result["files"] == json!([]);
                    match fact.action_kind {
                        ActionKind::ModelInference => notification_only && fact.status == ActionStatus::Succeeded,
                        ActionKind::ToolExecution => fact.status == ActionStatus::Succeeded
                            || !accepted && fact.status == ActionStatus::Failed && notification_only,
                        _ => false,
                    }
                })
                .ok_or("partial disposition references an ineligible action result")?;
            if result["files"] == json!([]) {
                continue;
            }
            let fact = fact.result.as_ref().ok_or("missing tool result")?;
            let files = match fact["tool_name"].as_str() {
                Some("edit" | "write" | "read") => {
                    let path = fact["arguments"]["path"]
                        .as_str()
                        .ok_or("missing result file")?;
                    let path = path.strip_prefix("./").unwrap_or(path);
                    vec![tree.workspace().join(path)]
                }
                Some("bash") => Vec::new(),
                _ => {
                    return Err(
                        "cannot determine the file dependencies of the selected result".into(),
                    );
                }
            };
            result["files"] = json!(files);
        }
    } else if let Some(scopes) = action["scopes"].as_array().filter(|scopes| {
        scopes
            .iter()
            .all(|scope| scope.as_str().is_some_and(|path| path.starts_with('/')))
    }) {
        part = json!({"intent": action["intent"], "results": [{"id": "selected-paths",
            "files": scopes, "notification": notification.unwrap_or("Selected files transferred")}]});
    }
    Ok(Some(part))
}

fn candidate_summary(tree: &SpeculativeTree, branch: &crate::control_plane::speculative_tree::BranchRecord) -> Value {
    let chance = branch.chance_id.as_ref().and_then(|id| tree.chances.get(id));
    json!({
        "branch_id": branch.branch_id,
        "chance_id": branch.chance_id,
        "status": branch.status,
        "speculation_kind": chance.and_then(|chance| chance.speculation_kind),
        "proposal_provenance": tree.branch_proposal(&branch.branch_id).and_then(|proposal| proposal.provenance.as_ref()),
        "yield_only": branch.yield_only,
        "active_tools": active_tools(tree, Some(&branch.branch_id)),
        "assumption_depth": branch.assumption_set.len(),
    })
}

fn declares_input(policy: &CompiledPolicy, decision: Decision, input: &str) -> bool {
    policy
        .decision_inputs(decision)
        .is_some_and(|inputs| inputs.iter().any(|name| name == input))
}

fn branch_history(tree: &SpeculativeTree, branch: &str, operation: &str, max_entries: usize) -> Vec<Value> {
    let transcript = tree.transcripts.get(branch).into_iter().flatten().rev().take(max_entries).map(|entry| (
        entry["timestamp"].as_u64().unwrap_or(0),
        json!({"role": entry["role"].as_str().map(summary), "custom_type": entry["customType"].as_str().map(summary),
            "content": content(&entry["content"])})
    ));
    let actions = recent_completed_actions(tree, branch, operation, max_entries);
    let mut history: Vec<_> = transcript.chain(actions.into_iter().rev().map(|entry| (entry["timestamp_ms"].as_u64().unwrap_or(0), entry))).collect();
    history.sort_by_key(|(timestamp, _)| std::cmp::Reverse(*timestamp));
    history.truncate(max_entries);
    history.into_iter().rev().map(|(timestamp, mut entry)| {
        entry["timestamp_ms"] = json!(timestamp);
        entry
    }).collect()
}

fn summary(text: &str) -> &str {
    &text[..text.floor_char_boundary(1024)]
}

fn content(value: &Value) -> String {
    if let Some(text) = value.as_str() {
        return summary(text).to_owned();
    }
    value.as_array().into_iter().flatten().take(4)
        .filter_map(|part| part["text"].as_str())
        .map(summary).collect::<Vec<_>>().join("\n")
}

fn recent_completed_actions(tree: &SpeculativeTree, branch: &str, operation: &str, max_entries: usize) -> Vec<Value> {
    let mut facts: Vec<_> = tree
        .action_facts
        .get(branch)
        .into_iter()
        .flat_map(|facts| facts.values())
        .filter(|fact| fact.action_id != operation && matches!(fact.status, ActionStatus::Succeeded | ActionStatus::Failed))
        .collect();
    facts.sort_by_key(|fact| std::cmp::Reverse((fact.timestamp_ms, &fact.action_id)));
    facts.truncate(max_entries);
    facts.into_iter().rev().map(|fact| {
        let result = fact.result.as_ref().unwrap_or(&Value::Null);
        json!({"action_id": summary(&fact.action_id), "kind": fact.action_kind,
            "timestamp_ms": fact.timestamp_ms, "started_at_ms": fact.started_at_ms, "status": fact.status,
            "tool_name": result["tool_name"].as_str().map(summary),
            "arguments": result.get("arguments"), "is_error": result.get("is_error"),
            "content": content(result.get("content").unwrap_or(&result["payload"]))})
    }).collect()
}

fn active_tools(tree: &SpeculativeTree, branch: Option<&str>) -> Vec<Value> {
    branch.and_then(|id| tree.active_actions.get(id)).into_iter().flat_map(|actions| actions.values())
        .filter(|action| action.action_kind == ActionKind::ToolExecution && action.arguments.is_some())
        .map(|action| json!({"tool_name": action.name, "arguments": action.arguments})).collect()
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
        "yield_only": branch.is_some_and(|branch| branch.yield_only),
        "yielded_action_ids": branch.map(|branch| &branch.yielded_action_ids),
        "active_tools": active_tools(tree, branch_id),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control_plane::speculative_tree::{ActionFinishedFact, ActionRecord};

    fn finish(tree: &mut SpeculativeTree, id: &str, kind: ActionKind, time: u64, result: Value) {
        let tool_calls = result.get("tool_calls").cloned();
        tree.action_facts.entry("child".into()).or_default().insert(id.into(), ActionFinishedFact {
            action_id: id.into(), action_kind: kind, status: ActionStatus::Succeeded,
            timestamp_ms: time, started_at_ms: None, result: Some(result), evidence: None, tool_calls,
        });
    }

    #[test]
    fn counts_complete_rounds_and_projects_inflight_tools() {
        let mut tree = SpeculativeTree::new("fixture".into());
        finish(&mut tree, "model-1", ActionKind::ModelInference, 1, json!({"tool_calls": [{"id":"one"},{"id":"two"}]}));
        finish(&mut tree, "one", ActionKind::ToolExecution, 2, json!(null));
        assert_eq!(completed_rounds(&tree, "child"), 0);
        tree.active_actions.entry("child".into()).or_default().insert("two".into(), ActionRecord {
            action_kind: ActionKind::ToolExecution, name: "bash".into(),
            arguments: Some(json!({"command":"pytest"})), parent_action_id: None, started_at_ms: None,
        });
        assert_eq!(active_tools(&tree, Some("child")), vec![json!({"tool_name":"bash","arguments":{"command":"pytest"}})]);
        assert_eq!(completed_rounds(&tree, "child"), 0);
        tree.active_actions.get_mut("child").unwrap().remove("two");
        finish(&mut tree, "two", ActionKind::ToolExecution, 3, json!(null));
        assert_eq!(completed_rounds(&tree, "child"), 1);
        finish(&mut tree, "model-2", ActionKind::ModelInference, 4, json!({"tool_calls": []}));
        assert_eq!(completed_rounds(&tree, "child"), 2);
    }

    #[test]
    fn model_outputs_can_be_selected_only_as_successful_notifications() {
        let mut tree = SpeculativeTree::new("fixture".into());
        let result = json!({"content":[{"type":"thinking","thinking":"reason"},{"type":"text","text":"answer"}]});
        finish(&mut tree, "model", ActionKind::ModelInference, 1, result.clone());
        let projected = candidate_results(&tree, "child", &[]);
        assert_eq!(projected[0]["action_kind"], "model_inference");
        assert_eq!(projected[0]["result"], result);
        for intent in ["Salvage", "Yield"] {
            let mut action = json!({"intent":intent,"results":[{"id":"model","notification":"answer","files":[]}]});
            assert_eq!(resolve_partial_acceptance(&tree, "child", &action, None, false).unwrap(), Some(action.clone()));
            action["results"][0]["files"] = json!(["edited.txt"]);
            assert!(resolve_partial_acceptance(&tree, "child", &action, None, false).is_err());
        }
        tree.action_facts.get_mut("child").unwrap().get_mut("model").unwrap().status = ActionStatus::Failed;
        let failed = json!({"intent":"Salvage","results":[{"id":"model","notification":"answer","files":[]}]});
        assert!(resolve_partial_acceptance(&tree, "child", &failed, None, false).is_err());
    }
}
