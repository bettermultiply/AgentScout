//! SpeculativeTree 的候选、晋升与回收。

use super::*;

impl SpeculativeTree {
    /// Create one candidate Branch for a new Proposal on `chance_id`: the
    /// Proposal, the Branch, its position and the start intent are one Tree
    /// operation.
    pub fn start_candidate(
        &mut self,
        chance_id: &str,
        prediction: Value,
    ) -> io::Result<AgentCommand> {
        self.create_candidate(chance_id, prediction, false)
    }

    pub fn start_model_candidate(
        &mut self,
        chance_id: &str,
        prediction: Value,
    ) -> io::Result<AgentCommand> {
        if self.chance(chance_id)?.speculation_kind != Some(ActionKind::ModelInference) {
            return Err(invalid_input(
                "branch_model requires a model inference boundary",
            ));
        }
        self.create_candidate(chance_id, prediction, true)
    }

    fn create_candidate(
        &mut self,
        chance_id: &str,
        prediction: Value,
        deferred: bool,
    ) -> io::Result<AgentCommand> {
        let chance = self.chance(chance_id)?.clone();
        let parent = chance.parent_branch_id.as_str();
        let sequence = self.next_generation;
        let branch_id = if let Some(scope) = self.reopened_execution_scope() {
            format!("candidate-{scope}-{sequence}")
        } else {
            format!("candidate-{sequence}")
        };
        let transcript_index = self.branch(parent)?.execution_position.transcript_index;
        let parent_assumptions = self.branch(parent)?.assumption_set.clone();
        let parent_status = self.branch(parent)?.status;
        // The Chance's authority value is produced by its parent Branch itself,
        // so a nested Chance on a live candidate is created like one on the
        // authority. AssumptionSet still blocks commit until ancestors resolve.
        if !parent_status.is_live_authority() && !parent_status.is_live_candidate() {
            return Err(invalid_input("parent branch cannot create a candidate"));
        }
        let proposal_id = format!("proposal-{branch_id}");
        self.insert_branch(
            &branch_id,
            Some(parent.to_string()),
            BranchRole::Candidate,
            BranchStatus::Created,
            Some(chance_id.to_string()),
            (chance.speculation_kind, chance.speculation_started_at_ms),
        )?;
        // The logical transcript belongs to the control plane. A Branch
        // inherits its parent's transcript and appends only its own entries;
        // native Pi session files remain the persistence/assembly form.
        if let Some(history) = self.transcripts.get(parent).cloned() {
            self.transcripts.insert(branch_id.clone(), history);
        }
        let chance_record = self.chance_mut(chance_id)?;
        if !deferred {
            chance_record.proposals.push(ProposalRecord {
                proposal_id: proposal_id.clone(),
                payload: prediction.clone(),
            });
        }
        chance_record.branch_ids.push(branch_id.clone());
        if !deferred {
            self.pending_observations.push(
                crate::control_plane::observation::ObservationContent::ProposalAdded {
                    chance_id: chance_id.into(),
                    proposal_id: proposal_id.clone(),
                    payload: prediction.clone(),
                },
            );
        }
        let branch = self.branch_mut(&branch_id)?;
        branch.execution_position = ExecutionPosition {
            sequence,
            operation_id: Some(chance.operation_id.clone()),
            transcript_index,
        };
        branch.assumption_set = parent_assumptions;
        if !deferred {
            branch.assumption_set.push(Assumption {
                chance_id: chance_id.to_string(),
                proposal_id,
            });
        }
        branch.pending_operations.push(PendingOperation {
            operation_id: chance.operation_id.clone(),
            branch_id: branch_id.clone(),
            kind: OperationKind::StartCandidate,
            status: OperationStatus::Planned,
            requested_at: None,
            deadline_at: None,
        });
        branch.execution_state.agent = AgentExecutionState::StartRequested;
        self.mark_operation_requested(
            &branch_id,
            &chance.operation_id,
            OperationKind::StartCandidate,
        )?;
        self.branch_mut(&branch_id)?.execution_state.turn_active = true;
        self.record_runtime_state(&branch_id, RuntimeExecutionState::EnvironmentRequested)?;
        Ok(AgentCommand::StartCandidate {
            environment_id: self.branches[&branch_id].environment_id.clone(),
            execution_access: None,
            branch_id: branch_id.clone(),
            parent_branch_id: parent.to_string(),
            operation_id: chance.operation_id,
            tool_name: chance.tool_name,
            workspace: self.workspace(),
            prediction,
            checkpoint: chance.checkpoint,
            command_id: format!("start-{branch_id}"),
            generation: self.branch(&branch_id)?.generation,
            history: self.transcripts.get(parent).cloned().unwrap_or_default(),
        })
    }

    pub fn pending_candidates(&self) -> Vec<String> {
        self.branches
            .values()
            .filter(|branch| {
                branch.role == BranchRole::Candidate
                    && branch.status.is_live_candidate()
                    && branch.chance_id.is_some()
            })
            .map(|branch| branch.branch_id.clone())
            .collect()
    }

    /// The authority's real result for the Chance this candidate belongs to.
    pub fn candidate_outcome(&self, candidate: &str) -> Option<&Value> {
        let chance = self
            .chances
            .get(self.branches.get(candidate)?.chance_id.as_ref()?)?;
        chance.authority_outcome.as_ref()
    }

    /// Runtime evidence gathered for the Chance's real action. This is the
    /// verifier input (`before`/`after` observations), not the model-facing
    /// tool result.
    pub fn candidate_evidence(&self, candidate: &str) -> Option<&Value> {
        let chance = self
            .chances
            .get(self.branches.get(candidate)?.chance_id.as_ref()?)?;
        chance.authority_evidence.as_ref()
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
        let chance = branch.chance_id.as_ref().and_then(|id| self.chances.get(id));
        // With the authority held, Policy may judge a real Proposal using its
        // own acceptance rule. Never fabricate a Proposal for an unfinished
        // alternate-model inference. An in-flight launch must settle first.
        let held_proposal = chance.is_some_and(|chance| {
            chance.authority == AuthorityState::NotRequested
                && chance.launch_epoch.is_some()
                && self.branch_proposal(candidate).is_some()
                && !self.pending_work.values().any(|work| matches!(work,
                    crate::control_plane::coordination::ControlOperation::Launch(tool)
                    if tool.chance_id == chance.chance_id))
        });
        (self.candidate_outcome(candidate).is_some() || held_proposal)
            && branch.status.is_decidable()
    }

    /// A candidate may only win while the authority is still at the waiting
    /// point the candidate forked from. Once the authority has started another
    /// tool operation, promoting the candidate would roll the authority back
    /// past work that has already been executed.
    pub(crate) fn authority_advanced_past(&self, candidate: &BranchRecord) -> bool {
        let Some(chance) = candidate
            .chance_id
            .as_ref()
            .and_then(|id| self.chances.get(id))
        else {
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
                && operation.operation_id != chance.operation_id
                && self.active_actions.get(&authority.branch_id)
                    .and_then(|actions| actions.get(&operation.operation_id))
                    .is_none_or(|action| action.action_kind == ActionKind::ToolExecution)
        })
    }

    pub fn can_promote(&self, candidate: &str, compatible: bool) -> bool {
        compatible
            && self.branch_proposal(candidate).is_some()
            && self.branches.get(candidate).is_some_and(|branch| {
                branch.role == BranchRole::Candidate
                    && branch.status.is_decidable()
                    && branch.parent_branch_id == self.authority_branch_id
                    && !self.ancestor_assumptions_unresolved(branch)
                    && !self.authority_advanced_past(branch)
            })
            && !self
                .branches
                .values()
                .any(|branch| branch.status == BranchStatus::Promoting)
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
        self.policy_record_revisions.remove(branch_id);
        self.policy_revisions.remove(branch_id);
        self.active_actions.remove(branch_id);
        self.action_facts.remove(branch_id);
        self.speculations.remove(branch_id);
        self.model_usage.remove(branch_id);
        self.tool_finished_at_ms.remove(branch_id);
        self.branches
            .remove(branch_id)
            .ok_or_else(|| invalid_input("branch does not exist"))
    }

    /// Discard a candidate and every Branch descended from it. When this was
    /// the last live Branch of its Chance, the Chance is recorded as discarded
    /// with `why`.
    pub fn discard_candidate(
        &mut self,
        branch_id: &str,
        why: DiscardedReason,
        reason: &str,
    ) -> io::Result<Vec<AgentCommand>> {
        let candidate = self
            .branches
            .get(branch_id)
            .ok_or_else(|| invalid_input("candidate branch does not exist"))?;
        if candidate.role != BranchRole::Candidate || !candidate.status.is_live_candidate() {
            return Err(invalid_input("branch is not a live candidate"));
        }
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
        let mut chance_ids = BTreeSet::new();
        for id in ids {
            if self.branch(&id)?.status.is_live_candidate() {
                self.change_status(&id, BranchStatus::Discarding, reason)?;
                commands.push(self.make_discard_command(&id)?);
                chance_ids.extend(self.branch(&id)?.chance_id.clone());
            }
        }
        for chance_id in chance_ids {
            self.settle_discarded_chance(&chance_id, why, reason)?;
        }
        Ok(commands)
    }

    /// Accept a candidate: request the authority switch. The Chance becomes
    /// `Accepted` only when the Agent confirms the promotion.
    pub fn accept_candidate(&mut self, branch_id: &str) -> io::Result<Vec<AgentCommand>> {
        let candidate = self
            .branches
            .get(branch_id)
            .ok_or_else(|| invalid_input("candidate branch does not exist"))?
            .clone();
        if candidate.role != BranchRole::Candidate || !candidate.status.is_decidable() {
            return Err(invalid_input("branch is not a decidable candidate"));
        }
        if self.ancestor_assumptions_unresolved(&candidate) {
            return Err(invalid_input(
                "candidate has unconfirmed ancestor proposals",
            ));
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
        if !self.branch(&old_authority)?.status.is_live_authority() {
            return Err(invalid_input("current authority is not live"));
        }
        self.change_status(
            branch_id,
            BranchStatus::Promoting,
            "Candidate accepted; promotion requested",
        )?;
        Ok(vec![self.make_promote_command(branch_id, None)?])
    }
}
