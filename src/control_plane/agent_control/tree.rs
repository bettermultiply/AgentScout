//! SpeculativeTree 的基础访问与内部索引。

use super::*;

impl SpeculativeTree {
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
                matches!(
                    branch.status,
                    BranchStatus::Running | BranchStatus::Promoted
                ) && branch.execution_state.agent == AgentExecutionState::Running
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
                            .any(|work| work.branch() == op.branch_id)
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
                parent_branch_id,
                role,
                status,
                generation,
                execution_position: Default::default(),
                proposal_id: None,
                assumption_set: Vec::new(),
                pending_operations: Vec::new(),
                execution_state: Default::default(),
            },
        );
        Ok(())
    }

    pub(super) fn branch(&self, branch_id: &str) -> io::Result<&BranchRecord> {
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
            CommandOperation::Promote => OperationKind::Promote,
            CommandOperation::Discard => OperationKind::Discard,
            CommandOperation::Cleanup => OperationKind::Cleanup,
            CommandOperation::ResolvePending => OperationKind::ResolvePending,
        };
        self.mark_operation_requested(branch_id, &id, kind)?;
        Ok(id)
    }

    pub(super) fn make_promote_command(&mut self, branch_id: &str) -> io::Result<AgentCommand> {
        let generation = self.branch(branch_id)?.generation;
        let command_id = self.register(branch_id, CommandOperation::Promote, None, generation)?;
        Ok(AgentCommand::Promote {
            branch_id: branch_id.to_string(),
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
