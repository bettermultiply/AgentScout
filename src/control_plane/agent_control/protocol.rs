//! SpeculativeTree 的Agent 协议消息处理。

use super::*;

impl SpeculativeTree {
    /// 接收 Pi 报告的启动事实。
    pub fn handle_message(&mut self, message: AgentMessage) -> io::Result<Vec<AgentCommand>> {
        match message {
            AgentMessage::SessionHistory { .. } => Ok(Vec::new()),
            AgentMessage::UiPromptRequested {
                branch_id,
                prompt_id,
                prompt,
            } => {
                // A prompt that cannot be shown is answered with a cancel
                // command so the Agent stops waiting. Only messages that
                // threaten control-plane state fail the connection.
                let cancel = |reason: &str| {
                    Ok(vec![AgentCommand::UiPromptAnswer {
                        branch_id: branch_id.clone(),
                        prompt_id: prompt_id.clone(),
                        answer: Value::Null,
                        cancelled: true,
                        reason: Some(reason.into()),
                    }])
                };
                if !matches!(
                    prompt.kind.as_str(),
                    "confirm" | "select" | "input" | "editor"
                ) {
                    return cancel("Unsupported text prompt kind");
                }
                let Ok(branch) = self.branch(&branch_id) else {
                    return cancel("Unknown Branch");
                };
                if self.stopping
                    || matches!(
                        branch.status,
                        BranchStatus::Discarding
                            | BranchStatus::Discarded
                            | BranchStatus::Cleaning
                            | BranchStatus::Cleaned
                    )
                {
                    return cancel("Branch is closing");
                }
                if self.ui_prompts.contains_key(&prompt_id) {
                    // The Agent reuses prompt ids; close the recorded prompt
                    // first so the reused id does not stay answerable.
                    self.close_ui_prompt(&prompt_id, true, Some("Duplicate prompt_id".into()));
                    return cancel("Duplicate prompt_id");
                }
                let visible = self.authority_branch_id.as_ref() == Some(&branch_id);
                if visible {
                    self.pending_observations.push(
                        crate::control_plane::observation::ObservationContent::UiPromptRequested {
                            branch_id: branch_id.clone(),
                            prompt_id: prompt_id.clone(),
                            prompt: prompt.clone(),
                        },
                    );
                }
                self.ui_prompts
                    .insert(prompt_id, (branch_id, prompt, visible));
                Ok(vec![])
            }
            AgentMessage::UiPromptCancelled {
                branch_id,
                prompt_id,
            } => {
                if self
                    .ui_prompts
                    .get(&prompt_id)
                    .is_some_and(|(owner, _, _)| owner == &branch_id)
                {
                    self.close_ui_prompt(&prompt_id, true, Some("Agent cancelled prompt".into()));
                }
                Ok(vec![])
            }
            AgentMessage::UiNotification {
                branch_id,
                text,
                level,
            } => {
                if self.authority_branch_id.as_ref() == Some(&branch_id) {
                    self.pending_observations.push(
                        crate::control_plane::observation::ObservationContent::UiNotification {
                            branch_id,
                            text,
                            level,
                        },
                    );
                }
                Ok(vec![])
            }
            AgentMessage::CommandsListed {
                branch_id,
                request_id,
                commands,
            } => {
                if (request_id == "startup"
                    || self.command_queries.remove(&request_id).as_ref() == Some(&branch_id))
                    && self.authority_branch_id.as_ref() == Some(&branch_id)
                {
                    if self.command_catalog.is_some() {
                        return Ok(vec![]);
                    }
                    self.command_catalog = Some(commands.clone());
                    self.pending_observations.push(
                        crate::control_plane::observation::ObservationContent::CommandsListed {
                            branch_id,
                            request_id,
                            commands,
                        },
                    );
                }
                Ok(vec![])
            }
            AgentMessage::TurnFailed { .. } | AgentMessage::ModelChanged { .. } => Ok(vec![]),
            AgentMessage::RecoverySuspended
            | AgentMessage::RecoverySaved { .. }
            | AgentMessage::RecoveryLoaded { .. } => Ok(vec![]),
            AgentMessage::AgentConnected { agent_id, .. } if agent_id == self.agent_id => {
                Ok(vec![])
            }
            AgentMessage::AgentConnected { .. } => {
                Err(invalid_input("agent id does not match controller"))
            }
            AgentMessage::AgentReady { agent_id, .. }
            | AgentMessage::AgentReleased { agent_id, .. } => {
                self.check_agent(&agent_id)?;
                Ok(vec![])
            }
            AgentMessage::AgentDisconnected { agent_id } => {
                self.check_agent(&agent_id)?;
                Err(io::Error::other(
                    "Agent disconnected; session release is not confirmed",
                ))
            }
            AgentMessage::AgentFailed { agent_id, reason } => {
                self.check_agent(&agent_id)?;
                Err(io::Error::other(reason))
            }
            AgentMessage::BranchCreated { branch_id } | AgentMessage::BranchReady { branch_id } => {
                self.branch(&branch_id)?;
                Ok(vec![])
            }
            AgentMessage::BranchSuspended { branch_id, reason } => {
                self.handle_branch_suspended(&branch_id, reason)
            }
            AgentMessage::BranchResumed { branch_id } => self.handle_branch_resumed(&branch_id),
            AgentMessage::BranchStarted { branch_id } => self.handle_branch_started(&branch_id),
            AgentMessage::ActionFinished {
                task_id,
                branch_id,
                action_id,
                action_kind,
                status,
                timestamp_ms,
                result,
                evidence,
            } => {
                if task_id != self.execution_id {
                    return Err(invalid_input("action task does not match tree"));
                }
                self.branch(&branch_id)?;
                if let Some(previous) = self
                    .action_facts
                    .get(&branch_id)
                    .and_then(|facts| facts.get(&action_id))
                {
                    if previous.status != status
                        || previous.action_kind != action_kind
                        || previous.result.as_ref() != Some(&result)
                        || previous.evidence != evidence
                    {
                        self.pending_records.push(serde_json::json!({
                            "message": "feedback_conflict", "branch_id": branch_id,
                            "operation_id": action_id, "reason": "conflicting ActionFinished feedback"
                        }));
                    }
                    return Ok(vec![]);
                }
                let tool_calls = result.get("tool_calls").cloned().or_else(|| {
                    evidence
                        .as_ref()
                        .and_then(|value| value.get("tool_calls"))
                        .cloned()
                });
                self.action_facts
                    .entry(branch_id.clone())
                    .or_default()
                    .insert(
                        action_id.clone(),
                        ActionFinishedFact {
                            action_id: action_id.clone(),
                            action_kind,
                            status,
                            timestamp_ms,
                            result: Some(result.clone()),
                            evidence: evidence.clone(),
                            tool_calls: tool_calls.clone(),
                        },
                    );
                let live = self
                    .active_actions
                    .get(&branch_id)
                    .is_some_and(|a| a.contains_key(&action_id));
                let mut pending_commands = Vec::new();
                if live && action_kind == ActionKind::ToolExecution {
                    let ids: Vec<_> = self
                        .chances
                        .values()
                        .filter(|chance| {
                            chance.parent_branch_id == branch_id
                                && chance.operation_id == action_id
                                && chance.authority == AuthorityState::Fetching
                        })
                        .map(|chance| chance.chance_id.clone())
                        .collect();
                    for id in ids {
                        let chance = self.chance(&id)?.clone();
                        let mut actions = crate::control_plane::coordination::ControlActions::default();
                        let outcome = if status == ActionStatus::Succeeded {
                            Ok(crate::control_plane::coordination::ControlResult::Outcome(
                                result.clone(),
                                evidence.clone().unwrap_or(Value::Null),
                            ))
                        } else {
                            Err(format!("Authority tool {status:?}"))
                        };
                        self.complete_tool_outcome(
                            crate::control_plane::coordination::ToolPending {
                                chance_id: id,
                                branch_id: branch_id.clone(),
                                operation_id: action_id.clone(),
                                tool_name: chance.tool_name,
                                arguments: chance.arguments,
                            },
                            outcome,
                            &mut actions,
                        )?;
                        pending_commands.extend(actions.commands);
                        self.pending_records.extend(actions.records);
                    }
                }
                if live
                    && action_kind == ActionKind::ModelInference
                    && status == ActionStatus::Succeeded
                {
                    let ids: Vec<_> = self
                        .chances
                        .values()
                        .filter(|c| {
                            c.parent_branch_id == branch_id
                                && c.operation_id == action_id
                                && (!c.stage.is_terminal()
                                    || c.authority == AuthorityState::Fetching)
                        })
                        .map(|c| c.chance_id.clone())
                        .collect();
                    for id in ids {
                        self.record_authority_outcome(&id, result.clone(), evidence.clone())?;
                        self.change_chance_stage(
                            &id,
                            ChanceStage::Resolving,
                            "Model authority arrived",
                        )?;
                        // The authority fact closes the only window in which
                        // an uncommitted Candidate may produce a Proposal.
                        // Cancel those model continuations before processing
                        // any later Candidate completion.  The returned
                        // commands are sent through the normal connection
                        // queue; this is deliberately not a direct Pi call.
                        let unfinished: Vec<_> = self
                            .chance(&id)?
                            .branch_ids
                            .iter()
                            .filter(|candidate| {
                                self.branch_proposal(candidate).is_none()
                                    && self.branches.get(*candidate).is_some_and(|branch| {
                                        branch.status.is_live_candidate()
                                            && branch.pending_operations.iter().any(|operation| {
                                                operation.kind == OperationKind::StartCandidate
                                                    && operation.status
                                                        == OperationStatus::Succeeded
                                            })
                                    })
                            })
                            .cloned()
                            .collect();
                        let mut cancellation_commands = Vec::new();
                        for candidate in unfinished {
                            self.pending_records.push(serde_json::json!({
                                "branch_id": candidate,
                                "hypothesis": Value::Null,
                                "verdict": "incompatible",
                                "policy_error": Value::Null,
                                "disposition": "Discard"
                            }));
                            cancellation_commands.extend(self.discard_candidate(
                                &candidate,
                                DiscardedReason::Failed,
                                "Authority value arrived before Candidate Proposal was ready",
                            )?);
                        }
                        // `handle_message` returns one command vector below;
                        // keep these receipts alongside the authority fact.
                        pending_commands.extend(cancellation_commands);
                    }
                    let candidate_chance = self.branch(&branch_id)?.chance_id.clone();
                    if let Some(id) = candidate_chance {
                        if status == ActionStatus::Succeeded
                            && self.branch_proposal(&branch_id).is_none()
                            && self.chance(&id)?.operation_id == action_id
                            && self.chance(&id)?.speculation_kind
                                == Some(ActionKind::ModelInference)
                            && !self.chance(&id)?.stage.is_terminal()
                            && self.branch(&branch_id)?.status.is_live_candidate()
                            && self.chance(&id)?.authority != AuthorityState::Arrived
                        {
                            let proposal_id = format!("proposal-{branch_id}");
                            self.chance_mut(&id)?.proposals.push(ProposalRecord {
                                proposal_id: proposal_id.clone(),
                                payload: result.clone(),
                            });
                            self.branch_mut(&branch_id)?
                                .assumption_set
                                .push(Assumption {
                                    chance_id: id.clone(),
                                    proposal_id: proposal_id.clone(),
                                });
                            self.pending_observations.push(crate::control_plane::observation::ObservationContent::ProposalAdded { chance_id: id, proposal_id, payload: result });
                        }
                    }
                }
                pending_commands.extend(self.handle_action_finished(
                    &task_id,
                    &branch_id,
                    &action_id,
                    action_kind,
                    status,
                    timestamp_ms,
                )?);
                // The model-fact path above may have queued cancellation
                // commands.  Keep the command transport serialized by
                // returning them to the control-plane execution loop.
                Ok(pending_commands)
            }
            AgentMessage::TurnCompleted { branch_id } => {
                let active = self.branch(&branch_id)?.execution_state.turn_active;
                self.branch_mut(&branch_id)?.execution_state.turn_active = false;
                if active
                    && !self.recovering
                    && !self.stopping
                    && self.authority_branch_id.as_deref() == Some(&branch_id)
                {
                    self.task_finishing = true;
                }
                Ok(vec![])
            }
            AgentMessage::Transcript {
                branch_id, entry, ..
            } => {
                self.transcripts
                    .entry(branch_id.clone())
                    .or_default()
                    .push(entry);
                self.branch_mut(&branch_id)?
                    .execution_position
                    .transcript_index += 1;
                Ok(vec![])
            }
            AgentMessage::Output { branch_id, .. }
            | AgentMessage::AdapterCapabilityGap { branch_id, .. } => {
                self.branch(&branch_id)?;
                Ok(vec![])
            }
            AgentMessage::Usage {
                branch_id,
                action_id,
                model,
                usage,
                started_at_ms,
                finished_at_ms,
                ..
            } => {
                self.branch(&branch_id)?;
                self.model_usage
                    .entry(branch_id)
                    .or_default()
                    .push(ModelUsageFact {
                        action_id,
                        model,
                        started_at_ms,
                        finished_at_ms,
                        duration_ms: finished_at_ms.checked_sub(started_at_ms),
                        cost: usage["cost"]["total"].as_f64(),
                    });
                Ok(vec![])
            }
            AgentMessage::ActionStarted {
                task_id,
                branch_id,
                action_id,
                action_kind,
                name,
                parent_action_id,
                ..
            } => self.handle_action_started(
                &task_id,
                &branch_id,
                action_id,
                action_kind,
                name,
                parent_action_id,
            ),
            AgentMessage::SpeculativeAdmissionRequest { .. } => Err(invalid_input(
                "speculative admission must be handled by the coordinator",
            )),
            AgentMessage::BranchReleased { branch_id } => self.handle_branch_released(&branch_id),
            AgentMessage::AgentSessionReleased { branch_id } => {
                self.handle_agent_session_released(&branch_id)
            }
            AgentMessage::BranchFailed { branch_id, reason } => {
                self.handle_branch_failed(&branch_id, reason)
            }
            AgentMessage::CommandResult {
                command_id,
                branch_id,
                operation_id,
                operation,
                status,
                generation,
                reason,
            } => self.handle_command_result(
                &command_id,
                &branch_id,
                operation_id.as_deref(),
                operation,
                status,
                generation,
                reason.as_deref(),
            ),
        }
    }

    fn check_agent(&self, agent_id: &str) -> io::Result<()> {
        if agent_id != self.agent_id {
            return Err(invalid_input("agent id does not match controller"));
        }
        Ok(())
    }

    /// The Agent Adapter reports that it actually holds this Branch at a
    /// boundary. Only the latest blocking Decision is recorded.
    fn handle_branch_suspended(
        &mut self,
        branch_id: &str,
        reason: BlockedReason,
    ) -> io::Result<Vec<AgentCommand>> {
        let branch = self.branch_mut(branch_id)?;
        if matches!(
            branch.status,
            BranchStatus::Running
                | BranchStatus::Promoted
                | BranchStatus::Finished
                | BranchStatus::Blocked(_)
        ) {
            branch.execution_state.agent = AgentExecutionState::Stopped;
            self.change_status(
                branch_id,
                BranchStatus::Blocked(reason),
                &format!("Blocked: {reason:?}"),
            )?;
        }
        Ok(vec![])
    }

    fn handle_branch_resumed(&mut self, branch_id: &str) -> io::Result<Vec<AgentCommand>> {
        let branch = self.branch_mut(branch_id)?;
        if matches!(
            branch.status,
            BranchStatus::Running
                | BranchStatus::Promoted
                | BranchStatus::Finished
                | BranchStatus::Blocked(_)
        ) {
            branch.execution_state.agent = AgentExecutionState::Running;
            if matches!(branch.status, BranchStatus::Blocked(_)) {
                self.change_status(branch_id, BranchStatus::Running, "Block released")?;
            }
        }
        Ok(vec![])
    }

    fn handle_branch_started(&mut self, branch_id: &str) -> io::Result<Vec<AgentCommand>> {
        let branch = self.branch_mut(branch_id)?;
        // A start fact already in transit may arrive after cancellation
        // or after a duplicate start report. It must not resurrect a
        // discarded Branch or abort Parent fallback.
        if branch.status == BranchStatus::Created {
            branch.execution_state.agent = AgentExecutionState::Running;
            for operation in &mut branch.pending_operations {
                if matches!(
                    operation.kind,
                    OperationKind::StartAgent | OperationKind::StartCandidate
                ) {
                    operation.status = OperationStatus::Succeeded;
                }
            }
            self.change_status(
                branch_id,
                BranchStatus::Running,
                "Agent Branch start confirmed",
            )?;
        }
        Ok(vec![])
    }

    fn handle_action_finished(
        &mut self,
        task_id: &str,
        branch_id: &str,
        operation_id: &str,
        action_kind: ActionKind,
        status: ActionStatus,
        timestamp_ms: u64,
    ) -> io::Result<Vec<AgentCommand>> {
        if task_id != self.execution_id {
            return Err(invalid_input("action task does not match tree"));
        }
        self.branch(branch_id)?;
        let Some(action) = self
            .active_actions
            .get(branch_id)
            .and_then(|actions| actions.get(operation_id))
        else {
            return Ok(vec![]); // Duplicate/late completion cannot advance the Tree.
        };
        if action.action_kind != action_kind {
            return Err(invalid_input("action kind does not match start"));
        }
        self.active_actions
            .get_mut(branch_id)
            .unwrap()
            .remove(operation_id);
        if action_kind != ActionKind::ToolExecution {
            let branch = self.branch_mut(branch_id)?;
            if action_kind == ActionKind::ModelInference
                && status == ActionStatus::Succeeded
                && branch.role == BranchRole::Candidate
                && branch.status == BranchStatus::Running
            {
                self.change_status(
                    branch_id,
                    BranchStatus::Finished,
                    "Candidate model inference succeeded",
                )?;
            }
            return Ok(vec![]);
        }
        // 命中结算需要权威侧工具结果真实的 Instrumentation 到达时刻。这里只保留
        // 已完成 action 的时刻，不改变 action 的生命周期。
        self.tool_finished_at_ms
            .entry(branch_id.to_string())
            .or_default()
            .insert(operation_id.to_string(), timestamp_ms);
        if self.recovering && status == ActionStatus::Cancelled {
            // Native cancellation ends a Pi frame, not the Chance's real
            // Runtime operation. Its result/receipt is collected separately.
            return Ok(vec![]);
        }
        self.complete_operation(
            branch_id,
            Some(operation_id),
            if status == ActionStatus::Succeeded {
                OperationStatus::Succeeded
            } else {
                OperationStatus::Failed
            },
        )?;
        Ok(vec![])
    }

    fn handle_action_started(
        &mut self,
        task_id: &str,
        branch_id: &str,
        action_id: String,
        action_kind: ActionKind,
        name: String,
        parent_action_id: Option<String>,
    ) -> io::Result<Vec<AgentCommand>> {
        if task_id != self.execution_id {
            return Err(invalid_input("action task does not match tree"));
        }
        let commands =
            self.register_action(branch_id, action_id, action_kind, name, parent_action_id)?;
        if action_kind == ActionKind::ModelInference
            && let Some(chance_id) = self.branch(branch_id)?.chance_id.clone()
            && self.chance(&chance_id)?.authority == AuthorityState::Arrived
            && self.branch_proposal(branch_id).is_none()
            && self.branch(branch_id)?.role == BranchRole::Candidate
            && self.branch(branch_id)?.status.is_live_candidate()
        {
            // The Candidate's model call started only after the authority
            // value had already arrived. It can no longer produce a winning
            // Proposal; cancel the request now that it has a real start fact
            // and a truthful cancellation/usage can be observed.
            self.pending_records.push(serde_json::json!({
                "branch_id": branch_id,
                "hypothesis": Value::Null,
                "verdict": "incompatible",
                "policy_error": Value::Null,
                "disposition": "Discard"
            }));
            return self.discard_candidate(
                branch_id,
                DiscardedReason::Failed,
                "Authority value arrived before Candidate Proposal was ready",
            );
        }
        Ok(commands)
    }

    /// Pair an admitted action with its later ActionFinished. This registers
    /// the action's identity only; it records no start time. Tool start time
    /// comes exclusively from the adapter's ActionStarted fact.
    pub(crate) fn register_action(
        &mut self,
        branch_id: &str,
        action_id: String,
        action_kind: ActionKind,
        name: String,
        parent_action_id: Option<String>,
    ) -> io::Result<Vec<AgentCommand>> {
        let branch = self.branch(branch_id)?;
        if matches!(
            branch.status,
            BranchStatus::Discarding
                | BranchStatus::Discarded
                | BranchStatus::Cleaning
                | BranchStatus::Cleaned
        ) {
            return Ok(vec![]);
        }
        let actions = self
            .active_actions
            .entry(branch_id.to_string())
            .or_default();
        if actions.contains_key(&action_id) {
            return Ok(vec![]);
        }
        actions.insert(
            action_id,
            ActionRecord {
                action_kind,
                name,
                parent_action_id,
            },
        );
        Ok(vec![])
    }

    fn handle_branch_released(&mut self, branch_id: &str) -> io::Result<Vec<AgentCommand>> {
        self.cancel_ui_prompts(Some(branch_id), "Branch released");
        self.active_actions.remove(branch_id);
        let branch = self.branch_mut(branch_id)?;
        if branch.execution_state.agent != AgentExecutionState::Released {
            branch.execution_state.agent = AgentExecutionState::Stopped;
        }
        self.branch_mut(branch_id)?.execution_state.turn_active = false;
        Ok(vec![])
    }

    fn handle_agent_session_released(&mut self, branch_id: &str) -> io::Result<Vec<AgentCommand>> {
        self.cancel_ui_prompts(Some(branch_id), "Agent session released");
        self.active_actions.remove(branch_id);
        self.branch_mut(branch_id)?.execution_state.agent = AgentExecutionState::Released;
        self.branch_mut(branch_id)?.execution_state.turn_active = false;
        Ok(vec![])
    }

    pub fn delivery_failed(
        &mut self,
        command: &AgentCommand,
        reason: String,
    ) -> io::Result<Vec<AgentCommand>> {
        let value = serde_json::to_value(command)?;
        let branch = value["branch_id"].as_str().unwrap_or_default();
        if matches!(
            command,
            AgentCommand::StartBranch { .. } | AgentCommand::StartCandidate { .. }
        ) && self.branches.get(branch).is_some_and(|b| {
            b.pending_operations.iter().any(|op| {
                matches!(
                    op.kind,
                    OperationKind::StartAgent | OperationKind::StartCandidate
                ) && op.status == OperationStatus::Succeeded
            })
        }) {
            return Ok(vec![]);
        }
        if let Some(id) = value["command_id"].as_str() {
            if let Some(pending) = self.pending_commands.get(id).cloned() {
                return self.handle_command_result(
                    id,
                    branch,
                    pending.operation_id.as_deref(),
                    pending.operation,
                    CommandStatus::Failed,
                    pending.generation,
                    Some(&reason),
                );
            }
            // A business result may already have consumed this command.
            if !matches!(command, AgentCommand::StartCandidate { .. }) { return Ok(vec![]); }
        }
        self.handle_branch_failed(branch, reason)
    }

    fn handle_branch_failed(
        &mut self,
        branch_id: &str,
        reason: String,
    ) -> io::Result<Vec<AgentCommand>> {
        self.cancel_ui_prompts(Some(branch_id), "Branch failed");
        let branch = self.branch(branch_id)?.clone();
        if matches!(
            branch.status,
            BranchStatus::Discarding
                | BranchStatus::Discarded
                | BranchStatus::Cleaning
                | BranchStatus::Cleaned
        ) {
            return Ok(vec![]);
        }
        self.branch_mut(branch_id)?.execution_state.agent = AgentExecutionState::Failed;
        self.branch_mut(branch_id)?.execution_state.turn_active = false;
        self.branch_mut(branch_id)?.execution_state.failure = Some(reason.clone());
        // A failure of the current authority is not a candidate
        // discard: there is no older authority to fall back to.
        if self.authority_branch_id.as_deref() == Some(branch_id) {
            return Ok(vec![]);
        }
        // Promote may already be in flight. Do not discard until the
        // Agent has answered whether the authority switch happened.
        if branch.status == BranchStatus::Promoting {
            return Ok(vec![]);
        }
        self.discard_candidate(
            branch_id,
            DiscardedReason::Failed,
            &format!("Agent Branch failed: {reason}"),
        )
    }

    fn handle_command_result(
        &mut self,
        command_id: &str,
        branch_id: &str,
        operation_id: Option<&str>,
        operation: CommandOperation,
        status: CommandStatus,
        generation: u64,
        reason: Option<&str>,
    ) -> io::Result<Vec<AgentCommand>> {
        let Some(pending) = self.pending_commands.get(command_id).cloned() else {
            if !matches!(status, CommandStatus::Accepted | CommandStatus::Started)
                && self.branches.get(branch_id).is_some_and(|branch| {
                    branch.generation == generation
                        && branch.pending_operations.iter().any(|operation| {
                            operation.operation_id == command_id
                                && matches!(
                                    operation.status,
                                    OperationStatus::Succeeded | OperationStatus::Failed
                                )
                        })
                })
            {
                self.complete_operation(
                    branch_id,
                    Some(command_id),
                    if status == CommandStatus::Applied {
                        OperationStatus::Succeeded
                    } else {
                        OperationStatus::Failed
                    },
                )?;
            }
            return Ok(vec![]);
        };
        if pending.branch_id != branch_id
            || pending.operation != operation
            || pending.operation_id.as_deref() != operation_id
            || pending.generation != generation
        {
            return Ok(vec![]);
        }
        if matches!(status, CommandStatus::Accepted | CommandStatus::Started) {
            return Ok(vec![]);
        }
        self.pending_commands.remove(command_id);
        let operation_status = if status == CommandStatus::Applied {
            OperationStatus::Succeeded
        } else {
            OperationStatus::Failed
        };
        self.complete_operation(branch_id, Some(command_id), operation_status)?;
        self.apply_command_result(branch_id, pending.operation, status, reason)
    }

    /// Keep the command/status dispatch visible; each transition body is a
    /// named method.
    fn apply_command_result(
        &mut self,
        branch_id: &str,
        operation: CommandOperation,
        status: CommandStatus,
        reason: Option<&str>,
    ) -> io::Result<Vec<AgentCommand>> {
        let branch_status = self.branch(branch_id)?.status;
        match (operation, status) {
            (CommandOperation::Abort, status) if status != CommandStatus::Applied => {
                Err(io::Error::other(format!(
                    "Agent abort {status:?}: {}",
                    reason.unwrap_or("no reason reported")
                )))
            }
            (CommandOperation::Promote, CommandStatus::Applied)
                if branch_status == BranchStatus::Promoting =>
            {
                self.apply_promote(branch_id)
            }
            (CommandOperation::Promote, _) if branch_status == BranchStatus::Promoting => self
                .apply_promote_failure(
                    branch_id,
                    &format!(
                        "Agent promotion {status:?}: {}",
                        reason.unwrap_or("no reason reported")
                    ),
                ),
            (CommandOperation::Discard, CommandStatus::Applied)
                if branch_status == BranchStatus::Discarding =>
            {
                self.apply_discard(branch_id)
            }
            (CommandOperation::Cleanup, CommandStatus::Applied)
                if branch_status == BranchStatus::Cleaning =>
            {
                self.apply_cleanup(branch_id)
            }
            _ => Ok(vec![]),
        }
    }

    fn apply_promote(&mut self, branch_id: &str) -> io::Result<Vec<AgentCommand>> {
        let old_authority = self
            .authority_branch_id
            .clone()
            .ok_or_else(|| invalid_input("tree has no authority branch"))?;
        if old_authority == branch_id {
            return Ok(vec![]);
        }
        self.change_authority(branch_id)?;
        self.change_status(branch_id, BranchStatus::Promoted, "Agent promotion applied")?;
        if !self.recovering && !self.branch(branch_id)?.execution_state.turn_active {
            self.task_finishing = true;
        }
        if let Some(chance_id) = self.branch(branch_id)?.chance_id.clone()
            && let Some(assumption) = self.own_assumption(self.branch(branch_id)?).cloned()
        {
            self.change_chance_stage(
                &chance_id,
                ChanceStage::Accepted(assumption.proposal_id),
                "Promotion applied",
            )?;
        }
        // A model failure that arrived while Promote was in flight is now a
        // failure of the new authority, not a reason to discard the candidate.
        self.change_status(
            &old_authority,
            BranchStatus::Discarding,
            "Authority replaced",
        )?;
        let mut commands = vec![self.make_discard_command(&old_authority)?];
        // Only the confirmed switch invalidates the old authority's other
        // continuations.
        let obsolete: Vec<_> = self
            .pending_candidates()
            .into_iter()
            .filter(|id| self.branches[id].parent_branch_id.as_deref() == Some(&old_authority))
            .collect();
        for id in obsolete {
            if self.pending_candidates().contains(&id) {
                commands.extend(self.discard_candidate(
                    &id,
                    DiscardedReason::AncestorInvalidated,
                    "Sibling authority promotion confirmed",
                )?);
            }
        }
        Ok(commands)
    }

    fn apply_promote_failure(
        &mut self,
        branch_id: &str,
        reason: &str,
    ) -> io::Result<Vec<AgentCommand>> {
        self.change_status(branch_id, BranchStatus::Discarding, reason)?;
        self.settle_branch_chance(branch_id, reason)?;
        Ok(vec![self.make_discard_command(branch_id)?])
    }

    fn apply_discard(&mut self, branch_id: &str) -> io::Result<Vec<AgentCommand>> {
        self.branch_mut(branch_id)?.execution_state.agent = AgentExecutionState::Stopped;
        self.branch_mut(branch_id)?.execution_state.turn_active = false;
        self.change_status(branch_id, BranchStatus::Discarded, "Agent discard applied")?;
        self.change_status(
            branch_id,
            BranchStatus::Cleaning,
            "Agent session cleanup requested after discard",
        )?;
        Ok(vec![self.make_cleanup_command(branch_id)?])
    }

    fn apply_cleanup(&mut self, branch_id: &str) -> io::Result<Vec<AgentCommand>> {
        self.branch_mut(branch_id)?.execution_state.agent = AgentExecutionState::Released;
        self.branch_mut(branch_id)?.execution_state.turn_active = false;
        self.change_status(
            branch_id,
            BranchStatus::Cleaned,
            "Agent session cleanup applied",
        )?;
        // Cleanup is the terminal receipt for this session; no earlier
        // in-flight command may keep it live or revive it.
        self.pending_commands
            .retain(|_, command| command.branch_id != branch_id);
        for operation in &mut self.branch_mut(branch_id)?.pending_operations {
            if matches!(
                operation.status,
                OperationStatus::Planned | OperationStatus::Requested
            ) {
                operation.status = OperationStatus::Failed;
            }
        }
        Ok(vec![])
    }

    /// 只有 control plane 可以接受 Candidate；接受后旧权威失去权威身份。
    pub(crate) fn finish_runtime_promotion(
        &mut self,
        candidate: &str,
        success: bool,
        notification: Option<String>,
    ) -> io::Result<Vec<AgentCommand>> {
        if success {
            Ok(vec![self.make_promote_command(candidate, notification)?])
        } else {
            self.change_status(
                candidate,
                BranchStatus::Discarding,
                "Runtime promotion failed",
            )?;
            self.settle_branch_chance(candidate, "Runtime promotion failed")?;
            Ok(vec![self.make_discard_command(candidate)?])
        }
    }
}
