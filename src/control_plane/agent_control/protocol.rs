//! SpeculativeTree 的Agent 协议消息处理。

use super::*;

impl SpeculativeTree {
    /// 接收 Pi 报告的启动事实。
    pub fn handle_message(&mut self, message: AgentMessage) -> io::Result<Vec<AgentCommand>> {
        match message {
            AgentMessage::AgentConnected { agent_id, .. } if agent_id == self.agent_id => {
                Ok(vec![])
            }
            AgentMessage::AgentConnected { .. } => {
                Err(invalid_input("agent id does not match controller"))
            }
            AgentMessage::AgentReady { agent_id } | AgentMessage::AgentReleased { agent_id } => {
                if agent_id != self.agent_id {
                    return Err(invalid_input("agent id does not match controller"));
                }
                Ok(vec![])
            }
            AgentMessage::AgentDisconnected { agent_id } => {
                if agent_id != self.agent_id {
                    return Err(invalid_input("agent id does not match controller"));
                }
                Err(io::Error::other(
                    "Agent disconnected; session release is not confirmed",
                ))
            }
            AgentMessage::AgentFailed { agent_id, reason } => {
                if agent_id != self.agent_id {
                    return Err(invalid_input("agent id does not match controller"));
                }
                Err(io::Error::other(reason))
            }
            AgentMessage::BranchCreated { branch_id } | AgentMessage::BranchReady { branch_id } => {
                self.branch(&branch_id)?;
                Ok(vec![])
            }
            AgentMessage::BranchSuspended { branch_id } => {
                let branch = self.branch_mut(&branch_id)?;
                if matches!(
                    branch.status,
                    BranchStatus::Running
                        | BranchStatus::Promoted
                        | BranchStatus::AwaitingValidation
                ) {
                    branch.execution_state.agent = AgentExecutionState::Stopped;
                }
                Ok(vec![])
            }
            AgentMessage::BranchResumed { branch_id } => {
                let branch = self.branch_mut(&branch_id)?;
                if matches!(
                    branch.status,
                    BranchStatus::Running
                        | BranchStatus::Promoted
                        | BranchStatus::AwaitingValidation
                ) {
                    branch.execution_state.agent = AgentExecutionState::Running;
                }
                Ok(vec![])
            }
            AgentMessage::BranchStarted { branch_id } => {
                let branch = self.branch_mut(&branch_id)?;
                // A start fact already in transit may arrive after cancellation
                // or after a duplicate start report. It must not resurrect a
                // discarded Branch or abort Parent fallback.
                if branch.status == BranchStatus::Created {
                    branch.status = BranchStatus::Running;
                    branch.execution_state.agent = AgentExecutionState::Running;
                    for operation in &mut branch.pending_operations {
                        if matches!(
                            operation.kind,
                            OperationKind::StartAgent | OperationKind::StartCandidate
                        ) {
                            operation.status = OperationStatus::Succeeded;
                        }
                    }
                }
                Ok(vec![])
            }
            AgentMessage::ActionFinished {
                task_id,
                branch_id,
                action_id: operation_id,
                result: outcome,
                action_kind,
                status,
                evidence,
            } => {
                if task_id != self.execution_id {
                    return Err(invalid_input("action task does not match tree"));
                }
                self.branch(&branch_id)?;
                let Some(action) = self
                    .active_actions
                    .get(&branch_id)
                    .and_then(|actions| actions.get(&operation_id))
                else {
                    return Ok(vec![]); // Duplicate/late completion cannot advance the Tree.
                };
                if action.action_kind != action_kind {
                    return Err(invalid_input("action kind does not match start"));
                }
                self.active_actions
                    .get_mut(&branch_id)
                    .unwrap()
                    .remove(&operation_id);
                if action_kind != ActionKind::ToolExecution {
                    let branch = self.branch_mut(&branch_id)?;
                    if action_kind == ActionKind::ModelInference
                        && status == ActionStatus::Succeeded
                        && branch.role == BranchRole::Candidate
                        && branch.status == BranchStatus::Running
                    {
                        branch.status = BranchStatus::AwaitingValidation;
                    }
                    return Ok(vec![]);
                }
                self.complete_operation(
                    &branch_id,
                    Some(&operation_id),
                    if status == ActionStatus::Succeeded {
                        OperationStatus::Succeeded
                    } else {
                        OperationStatus::Failed
                    },
                )?;
                for branch in self
                    .branches
                    .values_mut()
                    .filter(|branch| branch.parent_branch_id.as_deref() == Some(&branch_id))
                {
                    for operation in &mut branch.pending_operations {
                        // Runtime evidence is authoritative; an Agent's textual
                        // rendering of that result must not overwrite it.
                        if operation.kind == OperationKind::StartCandidate
                            && operation.operation_id == operation_id
                            && operation.evidence.is_none()
                        {
                            operation.outcome = Some(outcome.clone());
                            operation.evidence = evidence.clone();
                        }
                    }
                }
                Ok(vec![])
            }
            AgentMessage::TurnCompleted { branch_id } => {
                self.branch_mut(&branch_id)?.execution_state.turn_active = false;
                Ok(vec![])
            }
            AgentMessage::Transcript { branch_id, .. } => {
                self.branch_mut(&branch_id)?
                    .execution_position
                    .transcript_index += 1;
                Ok(vec![])
            }
            AgentMessage::Output { branch_id, .. }
            | AgentMessage::Usage { branch_id, .. }
            | AgentMessage::AdapterCapabilityGap { branch_id, .. } => {
                self.branch(&branch_id)?;
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
            } => {
                if task_id != self.execution_id {
                    return Err(invalid_input("action task does not match tree"));
                }
                let branch = self.branch(&branch_id)?;
                if matches!(
                    branch.status,
                    BranchStatus::Discarding
                        | BranchStatus::Discarded
                        | BranchStatus::Cleaning
                        | BranchStatus::Cleaned
                ) {
                    return Ok(vec![]);
                }
                let actions = self.active_actions.entry(branch_id).or_default();
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
            AgentMessage::SpeculativeAdmissionRequest { .. } => Err(invalid_input(
                "speculative admission must be handled by the coordinator",
            )),
            AgentMessage::BranchReleased { branch_id } => {
                self.active_actions.remove(&branch_id);
                let branch = self.branch_mut(&branch_id)?;
                if branch.execution_state.agent != AgentExecutionState::Released {
                    branch.execution_state.agent = AgentExecutionState::Stopped;
                }
                self.branch_mut(&branch_id)?.execution_state.turn_active = false;
                Ok(vec![])
            }
            AgentMessage::AgentSessionReleased { branch_id } => {
                self.active_actions.remove(&branch_id);
                self.branch_mut(&branch_id)?.execution_state.agent = AgentExecutionState::Released;
                self.branch_mut(&branch_id)?.execution_state.turn_active = false;
                Ok(vec![])
            }
            AgentMessage::BranchFailed { branch_id, reason } => {
                let branch = self.branch(&branch_id)?.clone();
                if matches!(
                    branch.status,
                    BranchStatus::Discarding
                        | BranchStatus::Discarded
                        | BranchStatus::Cleaning
                        | BranchStatus::Cleaned
                ) {
                    return Ok(vec![]);
                }
                self.branch_mut(&branch_id)?.execution_state.agent = AgentExecutionState::Failed;
                self.branch_mut(&branch_id)?.execution_state.turn_active = false;
                self.branch_mut(&branch_id)?.execution_state.failure = Some(reason.clone());
                // A failure of the current authority is not a candidate
                // discard: there is no older authority to fall back to.
                if self.authority_branch_id.as_deref() == Some(branch_id.as_str()) {
                    return Ok(vec![]);
                }
                // Promote may already be in flight. Do not discard until the
                // Agent has answered whether the authority switch happened.
                if branch.status == BranchStatus::Promoting {
                    return Ok(vec![]);
                }
                self.resolve_candidate(&branch_id, false)
            }
            AgentMessage::CommandResult {
                command_id,
                branch_id,
                operation_id,
                operation,
                status,
                generation,
                ..
            } => {
                let Some(pending) = self.pending_commands.get(&command_id).cloned() else {
                    return Ok(vec![]);
                };
                if pending.branch_id != branch_id
                    || pending.operation != operation
                    || pending.operation_id != operation_id
                    || pending.generation != generation
                {
                    return Ok(vec![]);
                }
                if matches!(status, CommandStatus::Accepted | CommandStatus::Started) {
                    return Ok(vec![]);
                }
                self.pending_commands.remove(&command_id);
                let operation_status = if status == CommandStatus::Applied {
                    OperationStatus::Succeeded
                } else {
                    OperationStatus::Failed
                };
                self.complete_operation(&branch_id, Some(&command_id), operation_status)?;
                let branch = self.branch(&branch_id)?.clone();
                match (pending.operation, status) {
                    (CommandOperation::Promote, CommandStatus::Applied)
                        if branch.status == BranchStatus::Promoting =>
                    {
                        let old_authority = self
                            .authority_branch_id
                            .clone()
                            .ok_or_else(|| invalid_input("tree has no authority branch"))?;
                        if old_authority == branch_id {
                            return Ok(vec![]);
                        }
                        self.branch_mut(&branch_id)?.role = BranchRole::Authority;
                        self.branch_mut(&branch_id)?.status = BranchStatus::Promoted;
                        self.authority_branch_id = Some(branch_id.clone());
                        // A model failure that arrived while Promote was in
                        // flight is now a failure of the new authority, not
                        // a reason to discard the candidate.
                        let old = self.branch_mut(&old_authority)?;
                        old.role = BranchRole::Candidate;
                        old.status = BranchStatus::Discarding;
                        let mut commands = vec![self.make_discard_command(&old_authority)?];
                        // Only the confirmed switch invalidates the old authority's other continuations.
                        let obsolete: Vec<_> = self
                            .pending_candidates()
                            .into_iter()
                            .filter(|id| {
                                self.branches[id].parent_branch_id.as_deref()
                                    == Some(&old_authority)
                            })
                            .collect();
                        for id in obsolete {
                            if self.pending_candidates().contains(&id) {
                                commands.extend(self.resolve_candidate(&id, false)?);
                            }
                        }
                        Ok(commands)
                    }
                    (CommandOperation::Promote, _) if branch.status == BranchStatus::Promoting => {
                        self.branch_mut(&branch_id)?.status = BranchStatus::Discarding;
                        Ok(vec![self.make_discard_command(&branch_id)?])
                    }
                    (CommandOperation::Discard, CommandStatus::Applied)
                        if branch.status == BranchStatus::Discarding =>
                    {
                        self.branch_mut(&branch_id)?.execution_state.agent =
                            AgentExecutionState::Stopped;
                        self.branch_mut(&branch_id)?.execution_state.turn_active = false;
                        self.branch_mut(&branch_id)?.status = BranchStatus::Cleaning;
                        Ok(vec![self.make_cleanup_command(&branch_id)?])
                    }
                    (CommandOperation::Cleanup, CommandStatus::Applied)
                        if branch.status == BranchStatus::Cleaning =>
                    {
                        self.branch_mut(&branch_id)?.execution_state.agent =
                            AgentExecutionState::Released;
                        self.branch_mut(&branch_id)?.execution_state.turn_active = false;
                        self.branch_mut(&branch_id)?.status = BranchStatus::Cleaned;
                        // Cleanup is the terminal receipt for this session; no
                        // earlier in-flight command may keep it live or revive it.
                        self.pending_commands
                            .retain(|_, command| command.branch_id != branch_id);
                        for operation in &mut self.branch_mut(&branch_id)?.pending_operations {
                            if matches!(
                                operation.status,
                                OperationStatus::Planned | OperationStatus::Requested
                            ) {
                                operation.status = OperationStatus::Failed;
                            }
                        }
                        Ok(vec![])
                    }
                    _ => Ok(vec![]),
                }
            }
        }
    }

    /// 只有控制面可以接受 Candidate；接受后旧权威失去权威身份。
    pub(crate) fn finish_runtime_promotion(
        &mut self,
        candidate: &str,
        success: bool,
    ) -> io::Result<Vec<AgentCommand>> {
        if success {
            Ok(vec![self.make_promote_command(candidate)?])
        } else {
            self.branch_mut(candidate)?.status = BranchStatus::Discarding;
            Ok(vec![self.make_discard_command(candidate)?])
        }
    }

}
