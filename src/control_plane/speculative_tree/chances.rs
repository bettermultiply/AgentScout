
use super::*;
use crate::control_plane::observation::ObservationContent;

impl SpeculativeTree {
    pub fn chance(&self, chance_id: &str) -> io::Result<&ChanceRecord> {
        self.chances
            .get(chance_id)
            .ok_or_else(|| invalid_input("chance does not exist"))
    }

    pub(crate) fn chance_mut(&mut self, chance_id: &str) -> io::Result<&mut ChanceRecord> {
        let revision = self.policy_chance_revisions.entry(chance_id.to_owned()).or_default();
        revision.0 = revision.0.checked_add(1).expect("Chance revision exhausted");
        self.chances
            .get_mut(chance_id)
            .ok_or_else(|| invalid_input("chance does not exist"))
    }

    pub fn admit_chance(
        &mut self,
        parent: &str,
        operation_id: &str,
        tool_name: &str,
        arguments: Value,
        checkpoint: Option<Value>,
        speculation_started_at_ms: Option<u64>,
    ) -> io::Result<String> {
        self.admit_chance_with_id(parent, operation_id, tool_name, arguments, checkpoint, speculation_started_at_ms, None)
    }

    pub(super) fn admit_chance_with_id(
        &mut self,
        parent: &str,
        operation_id: &str,
        tool_name: &str,
        arguments: Value,
        checkpoint: Option<Value>,
        speculation_started_at_ms: Option<u64>,
        local_id: Option<String>,
    ) -> io::Result<String> {
        let parent_chance_id = self.branch(parent)?.chance_id.clone();
        let speculation_kind = self
            .active_actions
            .get(parent)
            .and_then(|actions| actions.get(operation_id))
            .map(|action| action.action_kind);
        let deferred_environment = local_id.is_some();
        let chance_id = if let Some(id) = local_id {
            id
        } else if let Some(scope) = self.reopened_execution_scope() {
            format!("chance-{scope}-{}", self.next_chance)
        } else {
            format!("chance-{}", self.next_chance)
        };
        self.next_chance = self
            .next_chance
            .checked_add(1)
            .ok_or_else(|| invalid_input("chance counter exhausted"))?;
        self.chances.insert(
            chance_id.clone(),
            ChanceRecord {
                chance_id: chance_id.clone(),
                parent_branch_id: parent.to_string(),
                parent_chance_id: parent_chance_id.clone(),
                operation_id: operation_id.to_string(),
                tool_name: tool_name.to_string(),
                arguments,
                checkpoint,
                stage: ChanceStage::Pending,
                authority: AuthorityState::NotRequested,
                proposals: Vec::new(),
                branch_ids: Vec::new(),
                authority_outcome: None,
                authority_evidence: None,
                speculation_started_at_ms,
                validation_id: None,
                speculation_kind,
                tool_continuation_pending: false,
                deferred_environment,
                continuation_released_by_yield: false,
                launch_epoch: None,
            },
        );
        self.epoch += 1;
        self.pending_observations
            .push(ObservationContent::ChanceCreated {
                chance_id: chance_id.clone(),
                parent_branch_id: parent.to_string(),
                parent_chance_id,
                operation_id: operation_id.to_string(),
                tool_name: tool_name.to_string(),
                stage: ChanceStage::Pending,
                authority_state: AuthorityState::NotRequested,
                speculation_started_at_ms,
            });
        Ok(chance_id)
    }

    pub(crate) fn change_chance_stage(
        &mut self,
        chance_id: &str,
        stage: ChanceStage,
        reason: &str,
    ) -> io::Result<()> {
        let current = self.chance(chance_id)?;
        if current.stage == stage || current.stage.is_terminal() {
            return Ok(());
        }
        let chance = self.chance_mut(chance_id)?;
        chance.stage = stage.clone();
        let validation_id = if matches!(stage, ChanceStage::Accepted(_)) {
            chance.validation_id
        } else {
            None
        };
        let revision = self.policy_chance_revisions.entry(chance_id.to_owned()).or_default();
        revision.1 = revision.1.checked_add(1).expect("Chance stage revision exhausted");
        self.epoch += 1;
        self.pending_observations
            .push(ObservationContent::ChanceStageChanged {
                chance_id: chance_id.into(),
                stage,
                reason: Some(reason.into()),
                validation_id,
            });
        Ok(())
    }

    pub(crate) fn change_authority_state(
        &mut self,
        chance_id: &str,
        state: AuthorityState,
    ) -> io::Result<()> {
        if self.chance(chance_id)?.authority == state {
            return Ok(());
        }
        let chance = self.chance_mut(chance_id)?;
        chance.authority = state;
        self.epoch += 1;
        self.pending_observations
            .push(ObservationContent::AuthorityStateChanged {
                chance_id: chance_id.into(),
                state,
            });
        Ok(())
    }

    pub fn selectable_chances(&self) -> Vec<String> {
        self.chances
            .values()
            .filter(|chance| {
                chance.stage == ChanceStage::Pending
                    && chance.authority == AuthorityState::NotRequested
            })
            .map(|chance| chance.chance_id.clone())
            .collect()
    }

    pub(crate) fn chance_waiting_live(&self, chance: &ChanceRecord) -> bool {
        !self.stopping
            && self
                .branches
                .get(&chance.parent_branch_id)
                .is_some_and(|branch| {
                    // Keep in-flight wait points until the Promote receipt or disposal.
                    branch.status.is_live_authority()
                        || branch.status.is_live_candidate()
                        || branch.status == BranchStatus::Promoting
                })
            && self.branches[&chance.parent_branch_id]
                .pending_operations
                .iter()
                .any(|op| {
                    op.operation_id == chance.operation_id
                        && op.status == OperationStatus::Requested
                })
    }

    pub(super) fn assumption_confirmed(&self, assumption: &Assumption) -> bool {
        self.chances.get(&assumption.chance_id).is_some_and(|chance| {
            matches!(&chance.stage, ChanceStage::Accepted(id) if id == &assumption.proposal_id)
        })
    }

    pub(crate) fn ancestor_assumptions_unresolved(&self, branch: &BranchRecord) -> bool {
        // A model Branch can block descendant commit before it has a Proposal to record.
        let mut parent = branch.parent_branch_id.as_deref();
        while let Some(id) = parent {
            let Some(ancestor) = self.branches.get(id) else {
                break;
            };
            // A retired authority has Candidate role for history but no unresolved Chance.
            if ancestor.role == BranchRole::Candidate
                && ancestor.chance_id.is_some()
                && self.branch_proposal(id).is_none()
            {
                return true;
            }
            parent = ancestor.parent_branch_id.as_deref();
        }
        branch.assumption_set.iter().any(|assumption| {
            Some(&assumption.chance_id) != branch.chance_id.as_ref()
                && !self.assumption_confirmed(assumption)
        })
    }

    pub(crate) fn invalidate_chances_on_branch(
        &mut self,
        branch_id: &str,
        reason: &str,
    ) -> io::Result<()> {
        let ids: Vec<String> = self
            .chances
            .values()
            .filter(|chance| chance.parent_branch_id == branch_id && !chance.stage.is_terminal())
            .map(|chance| chance.chance_id.clone())
            .collect();
        for id in ids {
            self.change_authority_state(&id, AuthorityState::Unavailable)?;
            self.change_chance_stage(
                &id,
                ChanceStage::Discarded(DiscardedReason::AncestorInvalidated),
                reason,
            )?;
        }
        Ok(())
    }

    pub fn own_assumption<'a>(&self, branch: &'a BranchRecord) -> Option<&'a Assumption> {
        branch
            .assumption_set
            .iter()
            .find(|assumption| Some(&assumption.chance_id) == branch.chance_id.as_ref())
    }

    pub fn branch_proposal(&self, branch_id: &str) -> Option<&ProposalRecord> {
        let branch = self.branches.get(branch_id)?;
        let assumption = self.own_assumption(branch)?;
        self.chances
            .get(&assumption.chance_id)?
            .proposals
            .iter()
            .find(|proposal| proposal.proposal_id == assumption.proposal_id)
    }

    pub(super) fn settle_discarded_chance(
        &mut self,
        chance_id: &str,
        reason: DiscardedReason,
        text: &str,
    ) -> io::Result<()> {
        let Some(chance) = self.chances.get(chance_id) else {
            return Ok(());
        };
        if chance.stage.is_terminal() {
            return Ok(());
        }
        let live = chance.branch_ids.iter().any(|id| {
            self.branches.get(id).is_some_and(|branch| {
                branch.status.is_live_candidate()
                    || matches!(
                        branch.status,
                        BranchStatus::Promoting | BranchStatus::Promoted
                    )
            })
        });
        if !live {
            self.change_chance_stage(chance_id, ChanceStage::Discarded(reason), text)?;
        }
        Ok(())
    }

    pub(super) fn settle_branch_chance(&mut self, branch_id: &str, text: &str) -> io::Result<()> {
        if let Some(chance_id) = self.branch(branch_id)?.chance_id.clone() {
            self.settle_discarded_chance(&chance_id, DiscardedReason::Failed, text)?;
        }
        Ok(())
    }

    pub(crate) fn record_validation(
        &mut self,
        chance_id: &str,
        proposal_id: &str,
        validation_id: u64,
        validated_at_ms: Option<u64>,
    ) {
        self.pending_observations
            .push(ObservationContent::ProposalValidated {
                chance_id: chance_id.into(),
                proposal_id: proposal_id.into(),
                validation_id,
                validated_at_ms,
            });
    }

    pub fn record_authority_outcome(
        &mut self,
        chance_id: &str,
        outcome: Value,
        evidence: Option<Value>,
    ) -> io::Result<()> {
        let chance = self.chance_mut(chance_id)?;
        chance.authority_outcome = Some(outcome);
        chance.authority_evidence = evidence;
        self.change_authority_state(chance_id, AuthorityState::Arrived)
    }

    pub(crate) fn record_public_nle_map_action(
        &mut self,
        branch_id: &str,
        action_id: &str,
        evidence: &Value,
    ) -> io::Result<()> {
        let Some(action) = evidence.get("action").and_then(Value::as_u64) else {
            return Ok(());
        };
        let Some(before) = evidence.get("before") else {
            return Ok(());
        };
        let Some(after) = evidence.get("after") else {
            return Ok(());
        };
        let public_observation = |observation: &Value| {
            let chars = observation.get("chars")?.as_array()?;
            let blstats = observation.get("blstats")?.as_array()?;
            if !chars.iter().all(Value::is_string) || !blstats.iter().all(Value::is_number) {
                return None;
            }
            let mut value = serde_json::Map::new();
            value.insert("chars".into(), Value::Array(chars.clone()));
            value.insert("blstats".into(), Value::Array(blstats.clone()));
            if let Some(message) = observation.get("message").and_then(Value::as_str) {
                value.insert("message".into(), Value::String(message.into()));
            }
            Some(Value::Object(value))
        };
        let (Some(before), Some(after)) = (public_observation(before), public_observation(after))
        else {
            return Ok(());
        };
        self.pending_observations.push(
            crate::control_plane::observation::ObservationContent::NleMapAction {
                branch_id: branch_id.into(),
                action_id: action_id.into(),
                action,
                before,
                after,
            },
        );
        Ok(())
    }

    pub(crate) fn sweep_chances(&mut self, actions: &mut ControlActions) -> io::Result<()> {
        let ids: Vec<String> = self.chances.keys().cloned().collect();
        for id in ids {
            let chance = self.chance(&id)?;
            let waiting = matches!(
                chance.authority,
                AuthorityState::NotRequested | AuthorityState::Fetching
            );
            if waiting && !self.chance_waiting_live(chance) {
                self.change_authority_state(&id, AuthorityState::Unavailable)?;
                self.change_chance_stage(
                    &id,
                    ChanceStage::Discarded(DiscardedReason::Failed),
                    "Waiting point is no longer live",
                )?;
            }
            if let ChanceStage::Discarded(reason) = self.chance(&id)?.stage {
                self.discard_chance_candidates(&id, reason, "Chance discarded", actions)?;
                self.release_tool_continuation(&id, actions)?;
            }
        }
        let removable: Vec<String> = self
            .chances
            .values()
            .filter(|chance| {
                chance.stage.is_terminal()
                    && chance.authority != AuthorityState::Fetching
                    && !chance.tool_continuation_pending
                    // The after-model hold still needs the result before action completion.
                    && !(chance.speculation_kind == Some(ActionKind::ModelInference)
                        && chance.authority == AuthorityState::Arrived
                        && self.chance_waiting_live(chance))
                    && !chance
                        .branch_ids
                        .iter()
                        .any(|id| self.branches.contains_key(id))
                    && !self.branches.values().any(|branch| {
                        branch.assumption_set.iter().any(|assumption| assumption.chance_id == chance.chance_id)
                    })
                    && !self
                        .pending_work
                        .values()
                        .any(|work| work.chance_id() == Some(chance.chance_id.as_str()))
            })
            .map(|chance| chance.chance_id.clone())
            .collect();
        for id in removable {
            self.policy_chance_revisions.remove(&id);
            self.chances.remove(&id);
        }
        Ok(())
    }
}
