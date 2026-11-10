//! SpeculativeTree 中的 Chance：被 `admit` 接纳的投机机会。
//!
//! Chance 在 admit 通过时创建并拥有不透明 id，直到收敛（接受或抛弃）。它持有
//! Proposal、关联的 Branch、权威值的取得状态和验证依据；Branch 只记录自己
//! 属于哪个 Chance。

use super::*;
use crate::control_plane::observation::ObservationContent;

impl SpeculativeTree {
    pub fn chance(&self, chance_id: &str) -> io::Result<&ChanceRecord> {
        self.chances
            .get(chance_id)
            .ok_or_else(|| invalid_input("chance does not exist"))
    }

    pub(crate) fn chance_mut(&mut self, chance_id: &str) -> io::Result<&mut ChanceRecord> {
        self.chances
            .get_mut(chance_id)
            .ok_or_else(|| invalid_input("chance does not exist"))
    }

    /// `admit` 通过：创建处于 `Pending` 的 Chance 并生成不透明 id。
    pub fn admit_chance(
        &mut self,
        parent: &str,
        operation_id: &str,
        tool_name: &str,
        arguments: Value,
        checkpoint: Option<Value>,
        speculation_started_at_ms: Option<u64>,
    ) -> io::Result<String> {
        let parent_chance_id = self.branch(parent)?.chance_id.clone();
        let speculation_kind = self
            .active_actions
            .get(parent)
            .and_then(|actions| actions.get(operation_id))
            .map(|action| action.action_kind);
        let chance_id = format!("chance-{}", self.next_chance);
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
                speculation_kind,
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
            });
        Ok(chance_id)
    }

    pub(crate) fn change_chance_stage(
        &mut self,
        chance_id: &str,
        stage: ChanceStage,
        reason: &str,
    ) -> io::Result<()> {
        let chance = self.chance_mut(chance_id)?;
        if chance.stage == stage || chance.stage.is_terminal() {
            return Ok(());
        }
        chance.stage = stage.clone();
        self.epoch += 1;
        self.pending_observations
            .push(ObservationContent::ChanceStageChanged {
                chance_id: chance_id.into(),
                stage,
                reason: Some(reason.into()),
            });
        Ok(())
    }

    pub(crate) fn change_authority_state(
        &mut self,
        chance_id: &str,
        state: AuthorityState,
    ) -> io::Result<()> {
        let chance = self.chance_mut(chance_id)?;
        if chance.authority == state {
            return Ok(());
        }
        chance.authority = state;
        self.epoch += 1;
        self.pending_observations
            .push(ObservationContent::AuthorityStateChanged {
                chance_id: chance_id.into(),
                state,
            });
        Ok(())
    }

    /// 已 `admit`、尚未被 `select` 处理、也没有开始取权威值的 Chance。
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

    /// Chance 的权威等待点是否仍然成立：父 Branch 仍在权威路径上（暂时被
    /// extent、effect 或 rebalance 停住不算失效），且该 operation 仍在等待结果。
    pub(crate) fn chance_waiting_live(&self, chance: &ChanceRecord) -> bool {
        !self.stopping
            && self.authority_branch_id.as_deref() == Some(chance.parent_branch_id.as_str())
            && self
                .branches
                .get(&chance.parent_branch_id)
                .is_some_and(|branch| branch.status.is_live_authority())
            && self.branches[&chance.parent_branch_id]
                .pending_operations
                .iter()
                .any(|op| {
                    op.operation_id == chance.operation_id
                        && op.status == OperationStatus::Requested
                })
    }

    /// 这个 Chance 上，Branch 自己携带的 Proposal 引用。
    pub fn own_assumption<'a>(&self, branch: &'a BranchRecord) -> Option<&'a Assumption> {
        branch
            .assumption_set
            .iter()
            .find(|assumption| Some(&assumption.chance_id) == branch.chance_id.as_ref())
    }

    /// 取出 Branch 所属 Chance 上的自身 Proposal。
    pub fn branch_proposal(&self, branch_id: &str) -> Option<&ProposalRecord> {
        let branch = self.branches.get(branch_id)?;
        let assumption = self.own_assumption(branch)?;
        self.chances
            .get(&assumption.chance_id)?
            .proposals
            .iter()
            .find(|proposal| proposal.proposal_id == assumption.proposal_id)
    }

    /// Chance 的最后一个候选 Branch 离开投机后，把 Chance 记为抛弃。
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
                    || matches!(branch.status, BranchStatus::Promoting | BranchStatus::Promoted)
            })
        });
        if !live {
            self.change_chance_stage(chance_id, ChanceStage::Discarded(reason), text)?;
        }
        Ok(())
    }

    /// 一个已经失败的 Branch 离开投机后，按需把它的 Chance 记为 `Failed`。
    pub(super) fn settle_branch_chance(&mut self, branch_id: &str, text: &str) -> io::Result<()> {
        if let Some(chance_id) = self.branch(branch_id)?.chance_id.clone() {
            self.settle_discarded_chance(&chance_id, DiscardedReason::Failed, text)?;
        }
        Ok(())
    }

    /// 权威值和验证依据属于 Chance。
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

    /// 丢弃失去权威等待点的 Chance，并释放已经收敛且无人引用的记录。
    pub(crate) fn sweep_chances(&mut self) -> io::Result<()> {
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
        }
        let removable: Vec<String> = self
            .chances
            .values()
            .filter(|chance| {
                chance.stage.is_terminal()
                    && chance.authority != AuthorityState::Fetching
                    && !chance
                        .branch_ids
                        .iter()
                        .any(|id| self.branches.contains_key(id))
                    && !self
                        .pending_work
                        .values()
                        .any(|work| work.chance_id() == Some(chance.chance_id.as_str()))
            })
            .map(|chance| chance.chance_id.clone())
            .collect();
        for id in removable {
            self.chances.remove(&id);
        }
        Ok(())
    }
}
