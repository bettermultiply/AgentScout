
use super::*;

impl SpeculativeTree {
    pub fn start_candidate(
        &mut self,
        chance_id: &str,
        prediction: Value,
    ) -> io::Result<AgentCommand> {
        self.start_candidate_with_provenance(chance_id, prediction, None, Value::Null)
    }

    pub(super) fn start_candidate_with_provenance(
        &mut self,
        chance_id: &str,
        prediction: Value,
        provenance: Option<Value>,
        model_options: Value,
    ) -> io::Result<AgentCommand> {
        self.create_candidate(chance_id, prediction, provenance, model_options, false)
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
        self.create_candidate(chance_id, prediction, None, Value::Null, true)
    }

    fn create_candidate(
        &mut self,
        chance_id: &str,
        prediction: Value,
        provenance: Option<Value>,
        model_options: Value,
        deferred: bool,
    ) -> io::Result<AgentCommand> {
        let chance = self.chance(chance_id)?.clone();
        let parent = chance.parent_branch_id.as_str();
        let sequence = self.next_generation;
        let branch_id = if chance.deferred_environment {
            format!("candidate-{chance_id}")
        } else if let Some(scope) = self.reopened_execution_scope() {
            format!("candidate-{scope}-{sequence}")
        } else {
            format!("candidate-{sequence}")
        };
        let transcript_index = self.branch(parent)?.execution_position.transcript_index;
        let parent_assumptions = self.branch(parent)?.assumption_set.clone();
        let parent_status = self.branch(parent)?.status;
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
        if let Some(history) = self.transcripts.get(parent).cloned() {
            self.transcripts.insert(branch_id.clone(), history);
        }
        let chance_record = self.chance_mut(chance_id)?;
        if !deferred {
            chance_record.proposals.push(ProposalRecord {
                proposal_id: proposal_id.clone(),
                payload: prediction.clone(),
                provenance,
                model_options,
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
        branch.deferred_environment = chance.deferred_environment;
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
        if !chance.deferred_environment {
            self.record_runtime_state(&branch_id, RuntimeExecutionState::EnvironmentRequested)?;
        }
        self.candidate_start_command(&branch_id, prediction)
    }

    pub(super) fn candidate_start_command(&self, branch_id: &str, prediction: Value) -> io::Result<AgentCommand> {
        let branch = self.branch(branch_id)?;
        let chance = self.chance(branch.chance_id.as_deref().ok_or_else(|| invalid_input("missing Candidate Chance"))?)?;
        let parent = &chance.parent_branch_id;
        Ok(AgentCommand::StartCandidate {
            environment_id: branch.environment_id.clone(),
            execution_access: None,
            branch_id: branch_id.into(),
            parent_branch_id: parent.clone(),
            operation_id: chance.operation_id.clone(),
            tool_name: chance.tool_name.clone(),
            workspace: self.workspace(),
            prediction,
            model_options: self.branch_proposal(branch_id).map_or(Value::Null, |proposal| proposal.model_options.clone()),
            checkpoint: chance.checkpoint.clone(),
            command_id: format!("start-{branch_id}"),
            generation: branch.generation,
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

    pub fn candidate_outcome(&self, candidate: &str) -> Option<&Value> {
        let chance = self
            .chances
            .get(self.branches.get(candidate)?.chance_id.as_ref()?)?;
        chance.authority_outcome.as_ref()
    }

    /// Verifier input (`before`/`after` observations) is separate from the tool result.
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
            || self.pending_acceptance.is_some()
            || self.pending_work.values().any(|work| matches!(work, ControlOperation::ResumeAcceptance(_)))
            || self
                .pending_commands
                .values()
                .any(|command| command.operation == CommandOperation::CancelAcceptance)
            || self
                .branches
                .values()
                .any(|branch| branch.status == BranchStatus::Promoting)
        {
            return false;
        }
        if branch.yield_only {
            return (branch.status.is_decidable() || branch.status == BranchStatus::Blocked(BlockedReason::Rebalanced))
                && (self.yield_candidate_stopped(branch)
                    || self.chance_parent(branch).is_some_and(|parent| {
                        self.model_inference_boundary.as_ref().is_some_and(|(owner, _, _)| owner == parent)
                            || self.authority_branch_id.as_deref() == Some(parent)
                                && self.chances.values().any(|chance| {
                                    chance.parent_branch_id == parent
                                        && chance.speculation_kind == Some(ActionKind::ModelInference)
                                        && chance.authority == AuthorityState::NotRequested
                                        && self.chance_waiting_live(chance)
                                })
                    }))
                && self.yield_pending(candidate);
        }
        let chance = self
            .chances
            .get(branch.chance_id.as_ref().unwrap());
        let held_proposal = chance.is_some_and(|chance| {
            chance.authority == AuthorityState::NotRequested
                && chance.launch_epoch.is_some()
                && self.branch_proposal(candidate).is_some()
                && !self.pending_work.values().any(|work| {
                    matches!(work,
                    ControlOperation::Launch(tool)
                    if tool.chance_id == chance.chance_id)
                })
        });
        (self.candidate_outcome(candidate).is_some() || held_proposal)
            && branch.status.is_decidable()
            && !self.deferred_validations.get(candidate)
                .is_some_and(|saved| saved == &self.validation_snapshot(candidate))
    }

    /// Promote only at the original wait point to avoid rolling back executed authority work.
    pub(crate) fn authority_advanced_past(&self, candidate: &BranchRecord) -> bool {
        let Some(chance) = candidate
            .chance_id
            .as_ref()
            .and_then(|id| self.chances.get(id))
        else {
            return true;
        };
        let Some(authority) = self.branches.get(&chance.parent_branch_id) else {
            return true;
        };
        authority.pending_operations.iter().any(|operation| {
            operation.kind == OperationKind::Tool
                && operation.status == OperationStatus::Requested
                && operation.operation_id != chance.operation_id
                && self
                    .active_actions
                    .get(&authority.branch_id)
                    .and_then(|actions| actions.get(&operation.operation_id))
                    .is_none_or(|action| action.action_kind == ActionKind::ToolExecution)
        })
    }

    pub(crate) fn acceptance_progress_compatible(&self, candidate: &BranchRecord) -> bool {
        let Some(chance) = candidate
            .chance_id
            .as_ref()
            .and_then(|id| self.chances.get(id))
        else {
            return false;
        };
        let parent = &chance.parent_branch_id;
        let origin_finished = self
            .action_facts
            .get(parent)
            .and_then(|facts| facts.get(&chance.operation_id));
        let cutoff = origin_finished
            .map(|origin| origin.timestamp_ms)
            .or_else(|| {
                // A speculative-first operation can be validated by Policy before
                // it starts. The original Agent call must still be held.
                if chance.authority != AuthorityState::NotRequested {
                    return None;
                }
                self.branches
                    .get(parent)?
                    .pending_operations
                    .iter()
                    .find(|operation| {
                        operation.operation_id == chance.operation_id
                            && operation.status == OperationStatus::Requested
                    })?
                    .requested_at
            });
        let Some(cutoff) = cutoff else { return false };
        !self.authority_advanced_past(candidate)
            && !self
                .active_actions
                .get(parent)
                .is_some_and(|actions| actions.keys().any(|id| id != &chance.operation_id))
            && !self.action_facts.get(parent).is_some_and(|facts| {
                facts
                    .values()
                    .any(|fact| fact.action_id != chance.operation_id && fact.timestamp_ms > cutoff)
            })
    }

    pub(crate) fn chance_parent(&self, candidate: &BranchRecord) -> Option<&str> {
        let chance = self.chances.get(candidate.chance_id.as_ref()?)?;
        Some(&chance.parent_branch_id)
    }

    pub(crate) fn acceptance_target(&self, candidate: &BranchRecord) -> Option<&str> {
        if candidate.yield_only {
            return None;
        }
        let chance = self.chances.get(candidate.chance_id.as_ref()?)?;
        if chance.stage.is_terminal() || chance.continuation_released_by_yield {
            return None;
        }
        let parent = self.branches.get(&chance.parent_branch_id)?;
        if self.authority_branch_id.as_ref() == Some(&parent.branch_id) {
            return (parent.status.is_live_authority()
                && !self.ancestor_assumptions_unresolved(candidate))
                .then_some(&parent.branch_id);
        }

        if parent.role != BranchRole::Candidate
            || !parent.status.is_decidable()
            || self.branch_proposal(&parent.branch_id).is_none()
            || !self.active_branch_ids().contains(&parent.branch_id)
        {
            return None;
        }
        let unresolved = |assumption: &&Assumption| !self.assumption_confirmed(assumption);
        let inherited = candidate.assumption_set.iter().filter(|assumption| {
            Some(&assumption.chance_id) != candidate.chance_id.as_ref()
        }).filter(unresolved);
        if !inherited.clone().all(|assumption| parent.assumption_set.contains(assumption))
            || !parent.assumption_set.iter().filter(unresolved).all(|assumption| inherited.clone().any(|item| item == assumption))
            || parent.assumption_set.iter().any(|assumption| {
                self.chances.get(&assumption.chance_id).is_none_or(|chance| match &chance.stage {
                    ChanceStage::Accepted(id) => id != &assumption.proposal_id,
                    ChanceStage::Discarded(_) => true,
                    _ => false,
                })
            })
        {
            return None;
        }
        Some(&parent.branch_id)
    }

    pub fn can_promote(&self, candidate: &str, compatible: bool) -> bool {
        self.can_accept(candidate, compatible, false)
    }

    pub(super) fn can_accept(&self, candidate: &str, compatible: bool, transfer: bool) -> bool {
        compatible
            && self.pending_acceptance.is_none()
            && !self.pending_work.values().any(|work| matches!(work, ControlOperation::ResumeAcceptance(_)))
            && !self.pending_commands.values().any(|command| command.operation == CommandOperation::CancelAcceptance)
            && self.branch_proposal(candidate).is_some()
            && self.branches.get(candidate).is_some_and(|branch| {
                branch.role == BranchRole::Candidate
                    && branch.status.is_decidable()
                    && self.acceptance_target(branch).is_some_and(|parent| {
                        transfer || Some(parent) == self.authority_branch_id.as_deref()
                    })
                    && !self.authority_advanced_past(branch)
            })
            && !self
                .branches
                .values()
                .any(|branch| branch.status == BranchStatus::Promoting)
    }

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
                    && (branch.environment_id.is_none()
                        || branch.execution_state.runtime == RuntimeExecutionState::Cleaned)
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

    /// 取出终止 Branch 后，调用方须立即将其移交 Ledger。
    pub fn take_archivable_branch(&mut self, branch_id: &str) -> io::Result<BranchRecord> {
        if !self.evictable_branches().iter().any(|id| id == branch_id) {
            return Err(invalid_input("branch is not safe to archive"));
        }
        self.policy_record_revisions.remove(branch_id);
        self.policy_revisions.remove(branch_id);
        self.active_actions.remove(branch_id);
        self.action_facts.remove(branch_id);
        self.speculations.remove(branch_id);
        self.extent_disposals.remove(branch_id);
        self.deferred_validations.remove(branch_id);
        self.model_usage.remove(branch_id);
        self.tool_finished_at_ms.remove(branch_id);
        self.branches
            .remove(branch_id)
            .ok_or_else(|| invalid_input("branch does not exist"))
    }

    pub fn discard_candidate(
        &mut self,
        branch_id: &str,
        why: DiscardedReason,
        reason: &str,
    ) -> io::Result<Vec<AgentCommand>> {
        self.discard_candidate_subtree(branch_id, why, reason, false)
    }

    pub(super) fn discard_candidate_subtree(
        &mut self,
        branch_id: &str,
        why: DiscardedReason,
        reason: &str,
        after_acceptance: bool,
    ) -> io::Result<Vec<AgentCommand>> {
        let candidate = self
            .branches
            .get(branch_id)
            .ok_or_else(|| invalid_input("candidate branch does not exist"))?;
        if candidate.role != BranchRole::Candidate
            || !(candidate.status.is_live_candidate()
                || (after_acceptance && candidate.status == BranchStatus::Promoting))
        {
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
            let status = self.branch(&id)?.status;
            if status.is_live_candidate()
                || (after_acceptance && id == branch_id && status == BranchStatus::Promoting)
            {
                self.change_status(&id, BranchStatus::Discarding, reason)?;
                self.deferred_validations.remove(&id);
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
    #[cfg(test)]
    pub fn accept_candidate(&mut self, branch_id: &str) -> io::Result<Vec<AgentCommand>> {
        let candidate = self
            .branches
            .get(branch_id)
            .ok_or_else(|| invalid_input("candidate branch does not exist"))?
            .clone();
        if candidate.yield_only || candidate.role != BranchRole::Candidate || !candidate.status.is_decidable() {
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
