//! SpeculativeTree 的命令队列与生命周期。

use super::*;

impl SpeculativeTree {
    pub fn drain_ui_commands(&mut self) -> Vec<AgentCommand> {
        std::mem::take(&mut self.ui_commands)
    }

    pub(super) fn close_ui_prompt(&mut self, id: &str, cancelled: bool, reason: Option<String>) {
        if let Some((branch_id, _, visible)) = self.ui_prompts.remove(id) {
            if visible {
                self.pending_observations.push(
                    crate::control_plane::observation::ObservationContent::UiPromptResolved {
                        branch_id,
                        prompt_id: id.into(),
                        cancelled,
                        reason,
                    },
                );
            }
        }
    }

    pub(super) fn cancel_ui_prompts(&mut self, branch: Option<&str>, reason: &str) {
        let ids: Vec<_> = self
            .ui_prompts
            .iter()
            .filter(|(_, (owner, _, _))| branch.is_none_or(|branch| owner == branch))
            .map(|(id, (owner, _, _))| (id.clone(), owner.clone()))
            .collect();
        for (prompt_id, branch_id) in ids {
            self.close_ui_prompt(&prompt_id, true, Some(reason.into()));
            self.ui_commands.push(AgentCommand::UiPromptAnswer {
                branch_id,
                prompt_id,
                answer: Value::Null,
                cancelled: true,
                reason: Some(reason.into()),
            });
        }
    }

    pub fn answer_ui_prompt(
        &mut self,
        prompt_id: String,
        answer: Value,
        cancelled: bool,
    ) -> io::Result<AgentCommand> {
        let (branch_id, prompt, _) = self
            .ui_prompts
            .get(&prompt_id)
            .ok_or_else(|| invalid_input("Unknown, already answered or cancelled prompt_id"))?;
        if self.stopping || self.authority_branch_id.as_ref() != Some(branch_id) {
            return Err(invalid_input(
                "Prompt does not belong to the current authority",
            ));
        }
        if !cancelled
            && !match prompt.kind.as_str() {
                "confirm" => answer.is_boolean(),
                "select" => answer
                    .as_str()
                    .is_some_and(|s| prompt.options.iter().any(|o| o == s)),
                "input" | "editor" => answer.is_string(),
                _ => false,
            }
        {
            return Err(invalid_input("Invalid answer for this prompt"));
        }
        let branch_id = branch_id.clone();
        self.close_ui_prompt(&prompt_id, cancelled, None);
        Ok(AgentCommand::UiPromptAnswer {
            branch_id,
            prompt_id,
            answer,
            cancelled,
            reason: None,
        })
    }

    pub fn get_commands(&mut self, request_id: String) -> io::Result<Option<AgentCommand>> {
        let branch_id = self
            .authority_branch_id
            .clone()
            .ok_or_else(|| invalid_input("No authority session"))?;
        if let Some(commands) = &self.command_catalog {
            self.pending_observations.push(
                crate::control_plane::observation::ObservationContent::CommandsListed {
                    branch_id,
                    request_id,
                    commands: commands.clone(),
                },
            );
        }
        Ok(None)
    }
    pub fn recovery_pending_commands(&self) -> bool {
        !self.pending_commands.is_empty()
    }
    pub fn queue_prompt(&mut self, prompt: String) {
        if !prompt.trim().is_empty() {
            self.queued_prompts.push_back(prompt);
        }
    }

    pub fn next_prompt(&mut self) -> io::Result<Option<AgentCommand>> {
        if self.recovering { return Ok(None); }
        let Some(prompt) = self.queued_prompts.front().cloned() else {
            return Ok(None);
        };
        let command = self.continue_authority(prompt)?;
        if command.is_some() {
            self.queued_prompts.pop_front();
        }
        Ok(command)
    }

    /// A prompt starts a turn on the confirmed authority, never on an intended winner.
    pub fn input_ready(&self) -> bool {
        !self.task_finishing
            && !self.recovering
            && !self.stopping
            && self.queued_prompts.is_empty()
            && self.authority_ready()
            && !self.authority_running()
            && !self
                .branches
                .values()
                .any(|branch| branch.status == BranchStatus::Promoting)
    }

    pub fn continue_authority(&mut self, prompt: String) -> io::Result<Option<AgentCommand>> {
        if self.task_finishing
            || self.recovering
            || self.stopping
            || !self.authority_ready()
            || self.authority_running()
            || self
                .branches
                .values()
                .any(|branch| branch.status == BranchStatus::Promoting)
        {
            return Ok(None);
        }
        let branch_id = self.authority_branch_id.clone().unwrap();
        self.branch_mut(&branch_id)?.execution_state.turn_active = true;
        Ok(Some(AgentCommand::Prompt { branch_id, prompt }))
    }

    pub fn is_stopping(&self) -> bool {
        self.stopping
    }

    pub fn check_command_deadlines(&mut self) -> io::Result<()> {
        if self
            .shutdown_deadline
            .is_some_and(|deadline| deadline <= now_ms())
        {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Agent shutdown incomplete: cleanup or connection drain timed out",
            ));
        }
        if let Some(operation) = self
            .branches
            .values()
            .flat_map(|branch| &branch.pending_operations)
            .find(|operation| {
                operation.kind == OperationKind::Tool
                    && operation.status == OperationStatus::Requested
                    && operation
                        .deadline_at
                        .is_some_and(|deadline| deadline <= now_ms())
            })
        {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "Tool {} timed out for {}",
                    operation.operation_id, operation.branch_id
                ),
            ));
        }
        if let Some(branch) = self.branches.values().find(|branch| {
            branch.status == BranchStatus::Created
                && branch.pending_operations.iter().any(|operation| {
                    operation.kind == OperationKind::StartAgent
                        && operation
                            .deadline_at
                            .is_some_and(|deadline| deadline <= now_ms())
                })
        }) {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("Agent startup timed out for {}", branch.branch_id),
            ));
        }
        let expired = self
            .pending_commands
            .iter()
            .find(|(id, command)| {
                self.branches[&command.branch_id]
                    .pending_operations
                    .iter()
                    .any(|operation| {
                        operation.operation_id == **id
                            && operation
                                .deadline_at
                                .is_some_and(|deadline| deadline <= now_ms())
                    })
            })
            .map(|(id, command)| (id.clone(), command.clone()));
        if let Some((command_id, pending)) = expired {
            self.complete_operation(
                &pending.branch_id,
                Some(&command_id),
                OperationStatus::Failed,
            )?;
            // A timed-out authority switch is ambiguous. Never resume the old
            // authority as if the Agent had confirmed rejection.
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "{} confirmation timed out for {}",
                    operation_name(pending.operation),
                    pending.branch_id
                ),
            ));
        }
        Ok(())
    }

    /// Cancellation affects all pending candidates; their subtrees are discarded by resolve_candidate.
    pub fn interrupt(&mut self) -> io::Result<Vec<AgentCommand>> {
        self.cancel_ui_prompts(None, "Execution interrupted");
        self.queued_prompts.clear();
        for operation in self
            .branches
            .values_mut()
            .flat_map(|branch| &mut branch.pending_operations)
        {
            if operation.kind == OperationKind::Tool
                && operation.status == OperationStatus::Requested
            {
                operation.status = OperationStatus::Failed;
            }
        }
        let mut commands = Vec::new();
        for candidate in self.pending_candidates() {
            if self.pending_candidates().contains(&candidate) {
                commands.extend(self.discard_candidate(
                    &candidate,
                    DiscardedReason::Failed,
                    "User interrupted execution",
                )?);
            }
        }
        // A switch already in flight cannot be assumed rejected. Abort the
        // intended winner too, while retaining Promote's receipt until known.
        let promoting: Vec<_> = self
            .branches
            .values()
            .filter(|branch| branch.status == BranchStatus::Promoting)
            .map(|branch| branch.branch_id.clone())
            .collect();
        for branch in promoting {
            commands.push(self.make_abort_command(&branch)?);
        }
        if let Some(branch_id) = self.authority_branch_id.clone() {
            commands.push(self.make_abort_command(&branch_id)?);
        }
        Ok(commands)
    }

    pub fn begin_shutdown(&mut self) -> io::Result<Vec<AgentCommand>> {
        self.cancel_ui_prompts(None, "Session shutdown");
        if self.stopping {
            return Ok(Vec::new());
        }
        self.stopping = true;
        self.shutdown_deadline = Some(now_ms() + 10_000);
        self.queued_prompts.clear();
        let ids: Vec<_> = self
            .branches
            .values()
            .filter(|branch| branch.status != BranchStatus::Cleaned)
            .filter(|branch| {
                !self.pending_commands.values().any(|command| {
                    command.branch_id == branch.branch_id
                        && command.operation == CommandOperation::Cleanup
                })
            })
            .map(|branch| branch.branch_id.clone())
            .collect();
        ids.into_iter().map(|id| self.begin_cleanup(&id)).collect()
    }

    pub fn cleanup_pending(&self) -> io::Result<bool> {
        if self.branches.values().any(|branch| {
            branch.pending_operations.iter().any(|operation| {
                operation.kind == OperationKind::Cleanup
                    && operation.status == OperationStatus::Failed
            })
        }) {
            return Err(io::Error::other("Agent cleanup failed"));
        }
        Ok(self
            .pending_commands
            .values()
            .any(|command| command.operation == CommandOperation::Cleanup))
    }

    pub fn resolve_pending(
        &mut self,
        branch_id: &str,
        operation_id: &str,
        outcome: Value,
    ) -> io::Result<AgentCommand> {
        let generation = self.branch(branch_id)?.generation;
        let command_id = self.register(
            branch_id,
            CommandOperation::ResolvePending,
            Some(operation_id.to_string()),
            generation,
        )?;
        Ok(AgentCommand::ResolvePending {
            branch_id: branch_id.to_string(),
            operation_id: operation_id.to_string(),
            accepted: false,
            outcome: Some(outcome),
            command_id,
            generation,
        })
    }

    pub fn mark_operation_requested(
        &mut self,
        branch_id: &str,
        operation_id: &str,
        kind: OperationKind,
    ) -> io::Result<()> {
        let branch = self.branch_mut(branch_id)?;
        if let Some(operation) = branch
            .pending_operations
            .iter_mut()
            .find(|operation| operation.operation_id == operation_id)
        {
            operation.status = OperationStatus::Requested;
            operation.requested_at = Some(now_ms());
            operation.deadline_at = Some(now_ms() + 120_000);
        } else {
            branch.pending_operations.push(PendingOperation {
                operation_id: operation_id.to_string(),
                branch_id: branch_id.to_string(),
                kind,
                status: OperationStatus::Requested,
                requested_at: Some(now_ms()),
                deadline_at: Some(now_ms() + 120_000),
            });
        }
        Ok(())
    }

    /// 保存 Runtime 实际返回的独占 Environment 身份；不据此宣告建立成功。
    pub fn bind_environment(&mut self, branch_id: &str, environment_id: String) -> io::Result<()> {
        self.branch(branch_id)?;
        if environment_id.is_empty()
            || self
                .branches
                .values()
                .any(|record| record.environment_id.as_deref() == Some(&environment_id))
        {
            return Err(io::Error::other(
                "Runtime returned an empty or already bound Environment identity",
            ));
        }
        if self.branch(branch_id)?.environment_id.is_some() {
            return Err(io::Error::other(
                "Branch already has an Environment identity",
            ));
        }
        self.branch_mut(branch_id)?.environment_id = Some(environment_id);
        Ok(())
    }

    /// Runtime facts are independent of Agent session cleanup. In particular,
    /// releasing a session does not clean its Runtime Environment.
    pub fn record_runtime_state(
        &mut self,
        branch_id: &str,
        state: RuntimeExecutionState,
    ) -> io::Result<()> {
        if self.branch(branch_id)?.execution_state.runtime == state {
            return Ok(());
        }
        self.branch_mut(branch_id)?.execution_state.runtime = state;
        self.emit_environment(branch_id, state)?;
        Ok(())
    }

    pub(super) fn emit_environment(
        &mut self,
        branch_id: &str,
        state: RuntimeExecutionState,
    ) -> io::Result<()> {
        use crate::control_plane::observation::ObservationContent;
        let independent_instance = match state {
            RuntimeExecutionState::Unknown
            | RuntimeExecutionState::EnvironmentRequested
            | RuntimeExecutionState::Failed => None,
            RuntimeExecutionState::Cleaned => Some(false),
            _ => Some(true),
        };
        let configuration = BTreeMap::new();
        self.pending_observations
            .push(ObservationContent::EnvironmentChanged {
                branch_id: branch_id.into(),
                state,
                independent_instance,
                parent_environment: None,
                configuration,
            });
        if state == RuntimeExecutionState::Cleaned {
            self.pending_observations
                .push(ObservationContent::EnvironmentCleaned {
                    branch_id: branch_id.into(),
                    succeeded: true,
                    reason: Some("Runtime environment cleanup confirmed".into()),
                });
        }
        Ok(())
    }

    /// Orderly process exit also releases the current Authority's session.
    /// Runtime environment cleanup remains a separate confirmed operation.
    pub fn begin_cleanup(&mut self, branch_id: &str) -> io::Result<AgentCommand> {
        self.branch(branch_id)?;
        let command = self.make_cleanup_command(branch_id)?;
        self.change_status(
            branch_id,
            BranchStatus::Cleaning,
            "Agent session cleanup requested",
        )?;
        Ok(command)
    }

    pub fn complete_operation(
        &mut self,
        branch_id: &str,
        operation_id: Option<&str>,
        status: OperationStatus,
    ) -> io::Result<()> {
        let branch = self.branch_mut(branch_id)?;
        if let Some(operation) = branch
            .pending_operations
            .iter_mut()
            .find(|operation| operation_id.is_none_or(|id| operation.operation_id == id))
        {
            if matches!(
                operation.status,
                OperationStatus::Succeeded | OperationStatus::Failed
            ) {
                if operation.status != status {
                    let operation_id = operation.operation_id.clone();
                    let previous = operation.status;
                    self.pending_records.push(serde_json::json!({
                        "message": "feedback_conflict", "branch_id": branch_id,
                        "operation_id": operation_id,
                        "reason": format!("conflicting terminal feedback: {previous:?} then {status:?}")
                    }));
                }
                return Ok(());
            }
            operation.status = status;
        }
        Ok(())
    }

    /// Set only when the authority Branch itself can no longer continue.
    pub fn authority_failure(&self) -> Option<&str> {
        self.authority_branch_id
            .as_ref()
            .and_then(|id| self.branches.get(id))
            .and_then(|branch| branch.execution_state.failure.as_deref())
    }

    /// 由 control plane 登记初始权威 Branch，并要求 Pi 启动它。
    pub fn create_authority(
        &mut self,
        branch_id: impl Into<String>,
        prompt: impl Into<String>,
    ) -> io::Result<AgentCommand> {
        let branch_id = branch_id.into();
        if self.authority_branch_id.is_some() {
            return Err(invalid_input("tree already has an authority"));
        }
        self.insert_branch(
            &branch_id,
            None,
            BranchRole::Authority,
            BranchStatus::Created,
            None,
            (None, None),
        )?;
        self.change_authority(&branch_id)?;
        self.history_root_branch_id = Some(branch_id.clone());
        self.branch_mut(&branch_id)?.execution_state.agent = AgentExecutionState::StartRequested;
        let prompt = prompt.into();
        self.branch_mut(&branch_id)?.execution_state.turn_active = !prompt.is_empty();
        self.mark_operation_requested(&branch_id, "start-agent", OperationKind::StartAgent)?;
        Ok(AgentCommand::StartBranch {
            environment_id: self.branches[&branch_id].environment_id.clone(),
            execution_access: None,
            role: BranchRole::Authority,
            workspace: self.workspace(),
            branch_id,
            prompt,
        })
    }
}
