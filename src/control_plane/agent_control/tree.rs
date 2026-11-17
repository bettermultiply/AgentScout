//! SpeculativeTree 的基础访问与内部索引。

use super::*;

impl BranchRecord {
    pub fn branch_id(&self) -> &str {
        &self.branch_id
    }
    pub fn environment_id(&self) -> Option<&str> {
        self.environment_id.as_deref()
    }
    pub fn parent_branch_id(&self) -> Option<&str> {
        self.parent_branch_id.as_deref()
    }
    pub fn role(&self) -> BranchRole {
        self.role
    }
    pub fn status(&self) -> BranchStatus {
        self.status
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn execution_position(&self) -> &ExecutionPosition {
        &self.execution_position
    }
    pub fn chance_id(&self) -> Option<&str> {
        self.chance_id.as_deref()
    }
    pub fn assumption_set(&self) -> &[Assumption] {
        &self.assumption_set
    }
    pub fn pending_operations(&self) -> &[PendingOperation] {
        &self.pending_operations
    }
    pub fn execution_state(&self) -> &BranchExecutionState {
        &self.execution_state
    }
}

impl SpeculativeTree {
    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }
    pub fn execution_id(&self) -> &str {
        &self.execution_id
    }
    pub fn history_root_branch_id(&self) -> Option<&str> {
        self.history_root_branch_id.as_deref()
    }
    pub fn authority_branch_id(&self) -> Option<&str> {
        self.authority_branch_id.as_deref()
    }
    pub fn branches(&self) -> &BTreeMap<String, BranchRecord> {
        &self.branches
    }
    pub fn chances(&self) -> &BTreeMap<String, ChanceRecord> {
        &self.chances
    }
    pub fn active_actions(&self) -> &BTreeMap<String, BTreeMap<String, ActionRecord>> {
        &self.active_actions
    }
    pub fn policy_binding(&self) -> Option<&Value> {
        self.policy_binding.as_ref()
    }
    pub fn runtime_binding(&self) -> Option<&Value> {
        self.runtime_binding.as_ref()
    }
    pub fn capability_view(&self) -> Option<&Value> {
        self.capability_view.as_ref()
    }
    pub fn local_policy_digest(&self) -> Option<&str> {
        self.local_policy_digest.as_deref()
    }
    pub fn workspace_root(&self) -> &std::path::Path {
        &self.workspace_root
    }

    /// Set the recording identity before execution starts; it cannot change under live work.
    pub fn set_execution_id(&mut self, execution_id: String) -> io::Result<()> {
        if self.authority_branch_id.is_some() && !self.recovering {
            return Err(invalid_input(
                "execution identity cannot change during execution",
            ));
        }
        self.execution_id = execution_id;
        Ok(())
    }

    pub fn set_agent_launch(&mut self, launch: crate::protocol::contract::AgentLaunch) {
        self.agent_launch = Some(launch);
    }

    pub fn agent_launch_for(&self, branch: &str) -> Option<crate::protocol::contract::AgentLaunch> {
        let mut launch = self.agent_launch.clone()?;
        launch.instance_id = Some(branch.to_string());
        launch
            .env
            .insert("ASCOUT_AGENT_BRANCH".into(), branch.into());
        Some(launch)
    }

    pub fn recovering(&self) -> bool {
        self.recovering
    }

    pub fn task_finishing(&self) -> bool {
        self.task_finishing
    }

    pub fn load_recovery(&mut self) {
        self.recovering = true;
    }

    pub(crate) fn pause_recovery(&mut self, at: u64) -> AgentCommand {
        self.recovering = true;
        self.recovery_paused_at.get_or_insert(at);
        AgentCommand::SuspendRecovery {
            branch_id: self.authority_branch_id.clone().unwrap_or_default(),
        }
    }

    pub(crate) fn continue_recovery(&mut self, at: u64) {
        if let Some(paused) = self.recovery_paused_at.take() {
            let elapsed = at.saturating_sub(paused);
            for branch in self.branches.values_mut() {
                for operation in &mut branch.pending_operations {
                    if let Some(deadline) = &mut operation.deadline_at {
                        *deadline = deadline.saturating_add(elapsed);
                    }
                }
            }
        }
        self.recovering = false;
    }

    pub(crate) fn finish_task(&mut self) {
        self.task_finishing = true;
    }

    pub(crate) fn cancel_task(&mut self) -> io::Result<Vec<AgentCommand>> {
        self.recovering = false;
        self.task_finishing = true;
        self.interrupt()
    }

    pub(crate) fn complete_task(&mut self) {
        self.task_finishing = false;
    }

    pub fn task_writers_drained(&self) -> bool {
        self.active_actions
            .values()
            .all(|actions| actions.is_empty())
            && !self.pending_work()
            && !self.recovery_pending_commands()
            && self.branches.values().all(|branch| {
                branch.role == BranchRole::Authority
                    || (branch.status == BranchStatus::Cleaned
                        && branch.execution_state.runtime == RuntimeExecutionState::Cleaned)
            })
    }
    pub fn drain_observations(
        &mut self,
    ) -> Vec<crate::control_plane::observation::ObservationContent> {
        std::mem::take(&mut self.pending_observations)
    }

    pub fn drain_records(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.pending_records)
    }

    pub(crate) fn change_status(
        &mut self,
        id: &str,
        status: BranchStatus,
        reason: &str,
    ) -> io::Result<()> {
        if matches!(
            status,
            BranchStatus::Discarding
                | BranchStatus::Discarded
                | BranchStatus::Cleaning
                | BranchStatus::Cleaned
        ) {
            self.cancel_ui_prompts(Some(id), "Branch is closing");
        }
        let branch = self.branch_mut(id)?;
        if branch.status == status {
            return Ok(());
        }
        branch.status = status;
        self.epoch += 1;
        if matches!(status, BranchStatus::Discarding | BranchStatus::Discarded) {
            self.invalidate_chances_on_branch(id, reason)?;
        }
        self.pending_observations.push(
            crate::control_plane::observation::ObservationContent::StatusChanged {
                branch_id: id.into(),
                status,
                reason: Some(reason.into()),
            },
        );
        if status == BranchStatus::Promoted {
            self.queue_settlement(id, true)?;
        } else if status == BranchStatus::Discarded {
            self.queue_settlement(id, false)?;
        }
        Ok(())
    }

    pub(super) fn change_authority(&mut self, id: &str) -> io::Result<()> {
        self.branch(id)?;
        if self.authority_branch_id.as_deref() == Some(id) {
            return Ok(());
        }
        let previous = self.authority_branch_id.replace(id.into());
        if let Some(old) = &previous {
            self.branch_mut(old)?.role = BranchRole::Candidate;
            // Only the current authority's prompts can be answered. A prompt
            // already shown for the outgoing authority would otherwise stay
            // visible while every answer is rejected.
            self.cancel_ui_prompts(Some(old), "Branch is no longer the authority");
        }
        self.branch_mut(id)?.role = BranchRole::Authority;
        self.pending_observations.push(
            crate::control_plane::observation::ObservationContent::AuthorityChanged {
                branch_id: id.into(),
                previous_authority: previous,
            },
        );
        for (prompt_id, (owner, prompt, visible)) in &mut self.ui_prompts {
            if owner == id && !*visible {
                *visible = true;
                self.pending_observations.push(
                    crate::control_plane::observation::ObservationContent::UiPromptRequested {
                        branch_id: id.into(),
                        prompt_id: prompt_id.clone(),
                        prompt: prompt.clone(),
                    },
                );
            }
        }
        Ok(())
    }
    pub fn with_workspace(
        agent_id: impl Into<String>,
        root: impl Into<PathBuf>,
    ) -> io::Result<Self> {
        let mut tree = Self::new(agent_id.into());
        tree.workspace_root = root.into();
        fs::create_dir_all(tree.workspace())?;
        Ok(tree)
    }

    pub fn workspace(&self) -> PathBuf {
        self.workspace_root.join("workspace")
    }

    /// Runtime tools may only execute for the live authority recorded by the Tree.
    /// Candidate and late messages must never be routed into another Branch's world.
    pub fn can_invoke_tool(&self, branch_id: &str) -> bool {
        self.authority_branch_id.as_deref() == Some(branch_id)
            && self.branches.get(branch_id).is_some_and(|branch| {
                branch.status.is_live_authority()
                    && branch.execution_state.agent == AgentExecutionState::Running
            })
    }

    pub fn authority_ready(&self) -> bool {
        self.authority_branch_id
            .as_ref()
            .and_then(|id| self.branches.get(id))
            .is_some_and(|branch| {
                branch.execution_state.agent == AgentExecutionState::Running
                    && !matches!(
                        branch.status,
                        BranchStatus::Cleaning | BranchStatus::Cleaned
                    )
            })
    }

    pub fn authority_running(&self) -> bool {
        self.authority_branch_id
            .as_ref()
            .and_then(|id| self.branches.get(id))
            .is_some_and(|branch| branch.execution_state.turn_active)
    }

    pub fn next_deadline(&self) -> Option<u64> {
        if self.stopping {
            return self.shutdown_deadline;
        }
        self.branches
            .values()
            .flat_map(|branch| &branch.pending_operations)
            .filter(|op| {
                if op.kind == OperationKind::StartCandidate {
                    self.pending_candidates().contains(&op.branch_id)
                        && !self
                            .pending_work
                            .values()
                            .any(|work| work.branch() == Some(op.branch_id.as_str()))
                } else {
                    op.status == OperationStatus::Requested
                }
            })
            .filter_map(|op| op.deadline_at)
            .min()
    }

    pub(super) fn insert_branch(
        &mut self,
        branch_id: &str,
        parent_branch_id: Option<String>,
        role: BranchRole,
        status: BranchStatus,
        chance_id: Option<String>,
        (speculation_kind, speculation_started_at_ms): (Option<ActionKind>, Option<u64>),
    ) -> io::Result<()> {
        if self.branches.contains_key(branch_id) {
            return Err(invalid_input("branch id already exists"));
        }
        let generation = self.next_generation;
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .ok_or_else(|| invalid_input("generation counter exhausted"))?;
        self.branches.insert(
            branch_id.to_string(),
            BranchRecord {
                branch_id: branch_id.to_string(),
                // Runtime assigns this identity after the establish request.
                // An unbound Branch must not carry a control-plane placeholder.
                environment_id: None,
                parent_branch_id,
                role,
                status,
                generation,
                execution_position: Default::default(),
                chance_id: chance_id.clone(),
                assumption_set: Vec::new(),
                pending_operations: Vec::new(),
                execution_state: Default::default(),
            },
        );
        self.pending_observations.push(
            crate::control_plane::observation::ObservationContent::BranchCreated {
                branch_id: branch_id.into(),
                parent_branch_id: self.branch(branch_id)?.parent_branch_id.clone(),
                role,
                chance_id,
                speculation_kind,
                speculation_started_at_ms,
            },
        );
        if let Some(started_at_ms) = speculation_started_at_ms {
            self.speculations.insert(
                branch_id.to_string(),
                SpeculationRecord {
                    started_at_ms,
                    kind: speculation_kind,
                    settled: false,
                },
            );
        }
        self.epoch += 1;
        self.pending_observations.push(
            crate::control_plane::observation::ObservationContent::StatusChanged {
                branch_id: branch_id.into(),
                status,
                reason: Some("Branch created".into()),
            },
        );
        self.emit_environment(branch_id, RuntimeExecutionState::Unknown)?;
        Ok(())
    }

    pub fn branch(&self, branch_id: &str) -> io::Result<&BranchRecord> {
        self.branches
            .get(branch_id)
            .ok_or_else(|| invalid_input("branch does not exist"))
    }

    pub(super) fn branch_mut(&mut self, branch_id: &str) -> io::Result<&mut BranchRecord> {
        self.branches
            .get_mut(branch_id)
            .ok_or_else(|| invalid_input("branch does not exist"))
    }

    pub(super) fn command_id(&mut self, operation: CommandOperation) -> io::Result<String> {
        let id = format!(
            "command-{}-{}",
            self.next_command_id,
            operation_name(operation)
        );
        self.next_command_id = self
            .next_command_id
            .checked_add(1)
            .ok_or_else(|| invalid_input("command counter exhausted"))?;
        Ok(id)
    }

    pub(super) fn register(
        &mut self,
        branch_id: &str,
        operation: CommandOperation,
        operation_id: Option<String>,
        generation: u64,
    ) -> io::Result<String> {
        self.branch(branch_id)?;
        let id = self.command_id(operation)?;
        self.pending_commands.insert(
            id.clone(),
            PendingCommand {
                branch_id: branch_id.to_string(),
                operation,
                operation_id,
                generation,
            },
        );
        let kind = match operation {
            CommandOperation::Abort => OperationKind::Abort,
            CommandOperation::Promote => OperationKind::Promote,
            CommandOperation::Discard => OperationKind::Discard,
            CommandOperation::Cleanup => OperationKind::Cleanup,
            CommandOperation::ResolvePending => OperationKind::ResolvePending,
        };
        self.mark_operation_requested(branch_id, &id, kind)?;
        Ok(id)
    }

    pub(super) fn make_abort_command(&mut self, branch_id: &str) -> io::Result<AgentCommand> {
        let generation = self.branch(branch_id)?.generation;
        let command_id = self.register(branch_id, CommandOperation::Abort, None, generation)?;
        Ok(AgentCommand::Abort {
            branch_id: branch_id.into(),
            command_id,
            generation,
        })
    }

    pub(super) fn make_promote_command(
        &mut self,
        branch_id: &str,
        notification: Option<String>,
    ) -> io::Result<AgentCommand> {
        let generation = self.branch(branch_id)?.generation;
        let command_id = self.register(branch_id, CommandOperation::Promote, None, generation)?;
        Ok(AgentCommand::Promote {
            branch_id: branch_id.to_string(),
            notification,
            command_id,
            generation,
        })
    }

    pub(super) fn make_discard_command(&mut self, branch_id: &str) -> io::Result<AgentCommand> {
        let generation = self.branch(branch_id)?.generation;
        let command_id = self.register(branch_id, CommandOperation::Discard, None, generation)?;
        Ok(AgentCommand::Discard {
            branch_id: branch_id.to_string(),
            command_id,
            generation,
        })
    }

    pub(super) fn make_cleanup_command(&mut self, branch_id: &str) -> io::Result<AgentCommand> {
        let generation = self.branch(branch_id)?.generation;
        let command_id = self.register(branch_id, CommandOperation::Cleanup, None, generation)?;
        Ok(AgentCommand::Cleanup {
            branch_id: branch_id.to_string(),
            workspace: self.workspace(),
            command_id,
            generation,
        })
    }
}
