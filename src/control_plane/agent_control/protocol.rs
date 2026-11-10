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
                ..
            } => self.handle_action_finished(
                &task_id,
                &branch_id,
                &action_id,
                action_kind,
                status,
                timestamp_ms,
            ),
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
                self.model_usage.entry(branch_id).or_default().push(ModelUsageFact {
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
        // 命中结算需要权威侧工具结果真实的接入侧到达时刻。这里只保留
        // 已完成 action 的时刻，不改变 action 的生命周期。
        self.tool_finished_at_ms
            .entry(branch_id.to_string())
            .or_default()
            .insert(operation_id.to_string(), timestamp_ms);
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
        self.register_action(branch_id, action_id, action_kind, name, parent_action_id)
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
        self.active_actions.remove(branch_id);
        let branch = self.branch_mut(branch_id)?;
        if branch.execution_state.agent != AgentExecutionState::Released {
            branch.execution_state.agent = AgentExecutionState::Stopped;
        }
        self.branch_mut(branch_id)?.execution_state.turn_active = false;
        Ok(vec![])
    }

    fn handle_agent_session_released(&mut self, branch_id: &str) -> io::Result<Vec<AgentCommand>> {
        self.active_actions.remove(branch_id);
        self.branch_mut(branch_id)?.execution_state.agent = AgentExecutionState::Released;
        self.branch_mut(branch_id)?.execution_state.turn_active = false;
        Ok(vec![])
    }

    fn handle_branch_failed(
        &mut self,
        branch_id: &str,
        reason: String,
    ) -> io::Result<Vec<AgentCommand>> {
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

    /// 只有控制面可以接受 Candidate；接受后旧权威失去权威身份。
    pub(crate) fn finish_runtime_promotion(
        &mut self,
        candidate: &str,
        success: bool,
    ) -> io::Result<Vec<AgentCommand>> {
        if success {
            Ok(vec![self.make_promote_command(candidate)?])
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
