//! SpeculativeTree 的候选、晋升与回收。

use super::*;

impl SpeculativeTree {
    /// Create the candidate, proposal, position and start intent as one Tree operation.
    pub fn start_candidate(
        &mut self,
        parent: &str,
        operation_id: &str,
        tool_name: &str,
        prediction: Value,
        checkpoint: Option<Value>,
    ) -> io::Result<AgentCommand> {
        if self.authority_branch_id.as_deref() != Some(parent) {
            return Err(invalid_input(
                "candidate parent is not the current authority",
            ));
        }
        let sequence = self.next_generation;
        let branch_id = format!("candidate-{sequence}");
        let transcript_index = self.branch(parent)?.execution_position.transcript_index;
        if !matches!(
            self.branch(parent)?.status,
            BranchStatus::Created | BranchStatus::Running | BranchStatus::Promoted
        ) {
            return Err(invalid_input("parent branch cannot create a candidate"));
        }
        self.insert_branch(
            &branch_id,
            Some(parent.to_string()),
            BranchRole::Candidate,
            BranchStatus::Created,
        )?;
        self.record_candidate_context(
            &branch_id,
            operation_id,
            prediction.clone(),
            sequence,
            transcript_index,
        )?;
        self.mark_operation_requested(&branch_id, operation_id, OperationKind::StartCandidate)?;
        let branch = self.branch_mut(&branch_id)?;
        branch.pending_operations.last_mut().unwrap().tool_name = Some(tool_name.to_string());
        branch.execution_state.runtime = RuntimeExecutionState::EnvironmentRequested;
        branch.execution_state.turn_active = true;
        Ok(AgentCommand::StartCandidate {
            branch_id: branch_id.clone(),
            parent_branch_id: parent.to_string(),
            operation_id: operation_id.to_string(),
            tool_name: tool_name.to_string(),
            workspace: self.workspace(),
            prediction,
            checkpoint,
            command_id: format!("start-{branch_id}"),
            generation: self.branch(&branch_id)?.generation,
        })
    }

    pub fn pending_candidates(&self) -> Vec<String> {
        self.branches
            .values()
            .filter(|branch| {
                branch.role == BranchRole::Candidate
                    && matches!(
                        branch.status,
                        BranchStatus::Created
                            | BranchStatus::Running
                            | BranchStatus::AwaitingValidation
                    )
                    && branch.proposal_id.is_some()
            })
            .map(|branch| branch.branch_id.clone())
            .collect()
    }

    pub fn candidate_outcome(&self, candidate: &str) -> Option<&Value> {
        let branch = self.branches.get(candidate)?;
        branch.execution_state.validation_outcome.as_ref()
    }

    /// Runtime evidence gathered for the candidate's real action. This is the
    /// verifier input (`before`/`after` observations), not the model-facing
    /// tool result.
    pub fn candidate_evidence(&self, candidate: &str) -> Option<&Value> {
        let branch = self.branches.get(candidate)?;
        branch.execution_state.validation_evidence.as_ref()
    }

    pub fn candidate_due(&self, candidate: &str) -> bool {
        let Some(branch) = self.branches.get(candidate) else {
            return false;
        };
        if !self.pending_candidates().iter().any(|id| id == candidate)
            || self
                .branches
                .values()
                .any(|branch| branch.status == BranchStatus::Promoting)
        {
            return false;
        }
        (self.candidate_outcome(candidate).is_some()
            && matches!(
                branch.status,
                BranchStatus::Running | BranchStatus::AwaitingValidation
            ))
            || branch.pending_operations.iter().any(|operation| {
                operation.kind == OperationKind::StartCandidate
                    && operation
                        .deadline_at
                        .is_some_and(|deadline| deadline <= now_ms())
            })
    }

    /// A candidate may only win while the authority is still at the waiting
    /// point the candidate forked from. Once the authority has started another
    /// tool operation, promoting the candidate would roll the authority back
    /// past work that has already been executed.
    pub(super) fn authority_advanced_past(&self, candidate: &BranchRecord) -> bool {
        let Some(proposal_id) = candidate.proposal_id.as_ref() else {
            return true;
        };
        let Some(proposal) = self.proposals.get(proposal_id) else {
            return true;
        };
        let Some(authority) = self
            .authority_branch_id
            .as_deref()
            .and_then(|id| self.branches.get(id))
        else {
            return true;
        };
        authority.pending_operations.iter().any(|operation| {
            operation.kind == OperationKind::Tool
                && operation.status == OperationStatus::Requested
                && operation.operation_id != proposal.operation_id
        })
    }

    pub fn can_promote(&self, candidate: &str, compatible: bool) -> bool {
        compatible
            && self.branches.get(candidate).is_some_and(|branch| {
                branch.role == BranchRole::Candidate
                    && matches!(
                        branch.status,
                        BranchStatus::Running | BranchStatus::AwaitingValidation
                    )
                    && branch.parent_branch_id == self.authority_branch_id
                    && !self.authority_advanced_past(branch)
            })
            && !self
                .branches
                .values()
                .any(|branch| branch.status == BranchStatus::Promoting)
    }

    /// 把 Candidate 的预测和执行位置写入 Tree；候选运行数据随后只从
    /// BranchRecord 读取，不再需要控制循环维护第二份 Proposal 状态。
    pub fn record_candidate_context(
        &mut self,
        branch_id: &str,
        operation_id: impl Into<String>,
        prediction: Value,
        sequence: u64,
        transcript_index: usize,
    ) -> io::Result<()> {
        let operation_id = operation_id.into();
        {
            let branch = self.branch_mut(branch_id)?;
            if branch.role != BranchRole::Candidate {
                return Err(invalid_input("only a candidate can have a proposal"));
            }
            branch.execution_position = ExecutionPosition {
                sequence,
                operation_id: Some(operation_id.clone()),
                transcript_index,
            };
            branch.execution_state.phase = BranchPhase::Predict;
        }
        let proposal_id = format!("proposal-{branch_id}");
        if self.proposals.contains_key(&proposal_id) {
            return Err(invalid_input("proposal id already exists"));
        }
        self.proposals.insert(
            proposal_id.clone(),
            ProposalRecord {
                proposal_id: proposal_id.clone(),
                operation_id: operation_id.clone(),
                payload: prediction,
            },
        );
        let branch = self.branch_mut(branch_id)?;
        branch.proposal_id = Some(proposal_id.clone());
        branch.assumption_set = vec![proposal_id];
        branch.pending_operations.push(PendingOperation {
            operation_id,
            branch_id: branch_id.to_string(),
            kind: OperationKind::StartCandidate,
            status: OperationStatus::Planned,
            outcome: None,
            evidence: None,
            requested_at: None,
            deadline_at: None,
            tool_name: None,
        });
        branch.execution_state.agent = AgentExecutionState::StartRequested;
        Ok(())
    }

    /// 将外部结果写回拥有该 Proposal 的 Candidate。
    /// 控制循环只提供事件，不在本地保存结果副本。
    pub fn record_candidate_outcome(
        &mut self,
        parent_branch_id: &str,
        operation_id: &str,
        outcome: Value,
        evidence: Option<Value>,
    ) -> io::Result<()> {
        let candidate_ids: Vec<String> = self
            .branches
            .values()
            .filter(|branch| {
                branch.parent_branch_id.as_deref() == Some(parent_branch_id)
                    && branch
                        .proposal_id
                        .as_ref()
                        .and_then(|id| self.proposals.get(id))
                        .is_some_and(|proposal| proposal.operation_id == operation_id)
            })
            .map(|branch| branch.branch_id.clone())
            .collect();
        for candidate_id in candidate_ids {
            let branch = self.branch_mut(&candidate_id)?;
            branch.execution_state.validation_outcome = Some(outcome.clone());
            branch.execution_state.validation_evidence = evidence.clone();
        }
        Ok(())
    }

    /// Return terminal Branches that may be handed to the Ledger.
    pub fn evictable_branches(&self) -> Vec<String> {
        self.branches
            .values()
            .filter(|branch| {
                !self
                    .pending_work
                    .values()
                    .any(|work| work.references(&branch.branch_id))
            })
            .filter(|branch| {
                branch.branch_id != self.authority_branch_id.as_deref().unwrap_or_default()
                    && matches!(branch.status, BranchStatus::Cleaned)
                    && !self
                        .pending_commands
                        .values()
                        .any(|command| command.branch_id == branch.branch_id)
                    && !self.branches.values().any(|child| {
                        child.parent_branch_id.as_deref() == Some(branch.branch_id.as_str())
                    })
            })
            .map(|branch| branch.branch_id.clone())
            .collect()
    }

    /// 从活动 Tree 中取出终止 Branch，并把它的所有权交给调用方的 Ledger。
    ///
    /// 从这一步开始，核心数据结构已经不再拥有该 Branch；Ledger 决定
    /// 记录对象何时释放。调用方应立即把返回值传给 Ledger。
    pub fn take_archivable_branch(&mut self, branch_id: &str) -> io::Result<BranchRecord> {
        if !self.evictable_branches().iter().any(|id| id == branch_id) {
            return Err(invalid_input("branch is not safe to archive"));
        }
        self.active_actions.remove(branch_id);
        self.branches
            .remove(branch_id)
            .ok_or_else(|| invalid_input("branch does not exist"))
    }

    pub fn resolve_candidate(
        &mut self,
        branch_id: &str,
        compatible: bool,
    ) -> io::Result<Vec<AgentCommand>> {
        let candidate = self
            .branches
            .get(branch_id)
            .ok_or_else(|| invalid_input("candidate branch does not exist"))?
            .clone();
        if candidate.role != BranchRole::Candidate
            || !(candidate.status == BranchStatus::Running
                || candidate.status == BranchStatus::AwaitingValidation
                || (!compatible && candidate.status == BranchStatus::Created))
        {
            return Err(invalid_input("branch is not a running candidate"));
        }

        if !compatible {
            let mut ids = BTreeSet::from([branch_id.to_string()]);
            loop {
                let before = ids.len();
                for branch in self.branches.values() {
                    if branch
                        .parent_branch_id
                        .as_ref()
                        .is_some_and(|parent| ids.contains(parent))
                    {
                        ids.insert(branch.branch_id.clone());
                    }
                }
                if before == ids.len() {
                    break;
                }
            }
            let mut commands = Vec::new();
            for id in ids {
                if matches!(
                    self.branch(&id)?.status,
                    BranchStatus::Created
                        | BranchStatus::Running
                        | BranchStatus::AwaitingValidation
                ) {
                    self.branch_mut(&id)?.status = BranchStatus::Discarding;
                    commands.push(self.make_discard_command(&id)?);
                }
            }
            return Ok(commands);
        }

        let old_authority = self
            .authority_branch_id
            .clone()
            .ok_or_else(|| invalid_input("tree has no authority branch"))?;
        if candidate.parent_branch_id.as_deref() != Some(&old_authority) {
            return Err(invalid_input(
                "candidate is not a child of current authority",
            ));
        }
        if self
            .branches
            .values()
            .any(|branch| branch.status == BranchStatus::Promoting)
        {
            return Err(invalid_input("a promotion is already in progress"));
        }
        if !matches!(
            self.branch(&old_authority)?.status,
            BranchStatus::Running | BranchStatus::Promoted
        ) {
            return Err(invalid_input("current authority is not live"));
        }
        let promoted = self.branch_mut(branch_id)?;
        promoted.status = BranchStatus::Promoting;
        Ok(vec![self.make_promote_command(branch_id)?])
    }

}
