//! AgentController：把控制结果落到 Tree。

use super::*;

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
            if result.status != crate::protocol::contract::ResultStatus::Completed {
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

    pub(super) fn launch_allowed(
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

