//! SpeculativeTree 的控制操作与动作投影。

use super::*;

impl SpeculativeTree {
    pub(super) fn schedule(&mut self, operation: ControlOperation, actions: &mut ControlActions) {
        let id = self.next_work;
        self.next_work += 1;
        self.pending_work.insert(id, operation.clone());
        actions.operations.push((id, operation));
    }

    pub fn speculative_admission(&mut self, message: AgentMessage) -> io::Result<ControlActions> {
        let AgentMessage::SpeculativeAdmissionRequest {
            branch_id,
            policy_digest,
            action_id: operation_id,
            name: tool_name,
            arguments,
            checkpoint,
            task_id,
            action_kind,
            parent_action_id,
            timestamp_ms,
        } = message
        else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "expected speculative admission request",
            ));
        };
        if task_id != self.execution_id {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "action task does not match tree",
            ));
        }
        if self.local_policy_digest.as_deref() != Some(policy_digest.as_str()) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "speculative admission does not use the delivered local Policy",
            ));
        }
        // Admission says the action exists and will be paired with its
        // ActionFinished; it does not say the tool has started. The adapter
        // measures the real start time in ActionStarted, so no start time is
        // synthesized here from the admission timestamp.
        self.register_action(
            &branch_id,
            operation_id.clone(),
            action_kind,
            tool_name.clone(),
            parent_action_id,
        )?;
        let actions = ControlActions::default();
        // A Chance may be admitted on any Branch that still exists in the
        // speculation, not only on the current authority: the nested Chance's
        // authority value is that Branch's own real execution result.
        let branch_live = self.branches.get(&branch_id).is_some_and(|branch| {
            branch.status.is_live_authority() || branch.status.is_live_candidate()
        });
        if self.stopping
            || !branch_live
            || self.branches[&branch_id]
                .pending_operations
                .iter()
                .any(|op| op.operation_id == operation_id)
        {
            return Ok(actions);
        }
        // Admission was decided by the adapter that sent this explicit request.
        // The control plane does not re-run it; it only records the accepted
        // opportunity as a Pending Chance. `select` runs from `progress`.
        self.mark_operation_requested(&branch_id, &operation_id, OperationKind::Tool)?;
        self.admit_chance(
            &branch_id,
            &operation_id,
            &tool_name,
            arguments,
            checkpoint,
            Some(timestamp_ms),
        )?;
        Ok(actions)
    }

    pub fn complete_work(
        &mut self,
        id: u64,
        result: Result<ControlResult, String>,
    ) -> io::Result<ControlActions> {
        let mut actions = ControlActions::default();
        let Some(work) = self.pending_work.remove(&id) else {
            return Ok(actions);
        };
        let branch = work.branch().map(str::to_string);
        match work {
            ControlOperation::Select(ids) => {
                self.complete_select(ids, result, &mut actions)?;
            }
            ControlOperation::Predict(tool) => {
                self.complete_predict(tool, result, &mut actions)?;
            }
            ControlOperation::Launch(tool) => {
                self.complete_launch(tool, result, &mut actions)?;
            }
            ControlOperation::Rebalance => {
                self.complete_rebalance(result, &mut actions)?;
            }
            ControlOperation::Establish(command) => {
                self.complete_establish(command, result, &mut actions, &branch.unwrap())?;
            }
            ControlOperation::Invoke(tool) => {
                self.complete_invoke(tool, result, &mut actions)?;
            }
            ControlOperation::Validate(candidate) => {
                self.complete_validate(id, candidate, result, &mut actions)?;
            }
            ControlOperation::Promote(candidate, record) => {
                self.complete_promote(candidate, record, result, &mut actions)?;
            }
            work @ (ControlOperation::Terminate(_) | ControlOperation::Cleanup(_)) => {
                self.complete_terminate_or_cleanup(work, result, &mut actions, &branch.unwrap())?;
            }
        }
        Ok(actions)
    }

    fn tool_pending(&self, chance_id: &str) -> io::Result<ToolPending> {
        let chance = self.chance(chance_id)?;
        Ok(ToolPending {
            chance_id: chance.chance_id.clone(),
            branch_id: chance.parent_branch_id.clone(),
            operation_id: chance.operation_id.clone(),
            tool_name: chance.tool_name.clone(),
            arguments: chance.arguments.clone(),
        })
    }

    fn chance_is(&self, chance_id: &str, stage: &ChanceStage) -> bool {
        self.chances
            .get(chance_id)
            .is_some_and(|chance| &chance.stage == stage && self.chance_waiting_live(chance))
    }

    /// Chances `select` leaves out stay Pending, but the Agent's pending tool
    /// call still needs its real result: the authority value is fetched
    /// without speculation.
    fn complete_select(
        &mut self,
        ids: Vec<String>,
        result: Result<ControlResult, String>,
        actions: &mut ControlActions,
    ) -> io::Result<()> {
        let selected = match result {
            Ok(ControlResult::Selected(selected)) => selected,
            Err(error) => {
                for id in ids {
                    if self.chance_is(&id, &ChanceStage::Pending) {
                        actions.records.push(json!({"message":"policy_error", "chance_id":id, "reason":error}));
                        self.change_chance_stage(
                            &id,
                            ChanceStage::Discarded(DiscardedReason::Failed),
                            "select failed",
                        )?;
                        self.change_authority_state(&id, AuthorityState::Fetching)?;
                    }
                }
                return Ok(());
            }
            _ => unreachable!(),
        };
        for id in &selected {
            if self.chance_is(id, &ChanceStage::Pending) {
                self.change_chance_stage(id, ChanceStage::Running, "Selected")?;
                let tool = self.tool_pending(id)?;
                self.schedule(ControlOperation::Predict(tool), actions);
            }
        }
        for id in ids.iter().filter(|id| !selected.contains(id)) {
            if self.chance_is(id, &ChanceStage::Pending) {
                self.change_authority_state(id, AuthorityState::Fetching)?;
            }
        }
        Ok(())
    }

    fn complete_predict(
        &mut self,
        tool: ToolPending,
        result: Result<ControlResult, String>,
        actions: &mut ControlActions,
    ) -> io::Result<()> {
        let chance_id = tool.chance_id.as_str();
        if !self.chance_is(chance_id, &ChanceStage::Running) {
            return Ok(());
        }
        match result {
            // An empty prediction set is not a failed speculation: this
            // boundary has no hypothesis to test. The authority value is
            // fetched as an ordinary action so the Agent's call is answered.
            Ok(ControlResult::Predictions(predictions)) if predictions.is_empty() => {
                self.change_authority_state(chance_id, AuthorityState::Fetching)?;
            }
            Ok(ControlResult::Predictions(predictions)) => {
                for prediction in predictions {
                    let command = self.start_candidate(chance_id, prediction)?;
                    let child = match &command {
                        AgentCommand::StartCandidate { branch_id, .. } => branch_id.clone(),
                        _ => unreachable!(),
                    };
                    actions.records.push(json!({
                        "message": "candidate_created",
                        "branch_id": child,
                        "parent": tool.branch_id,
                        "chance_id": chance_id,
                        "fork_at": self.branches[&child].execution_position.transcript_index
                    }));
                    self.schedule(ControlOperation::Establish(command), actions);
                }
            }
            Err(error) => {
                actions.records.push(json!({"message":"policy_error", "chance_id":chance_id, "reason":error}));
                self.change_chance_stage(
                    chance_id,
                    ChanceStage::Discarded(DiscardedReason::Failed),
                    "predict failed",
                )?;
                self.change_authority_state(chance_id, AuthorityState::Fetching)?;
            }
            _ => unreachable!(),
        }
        Ok(())
    }

    /// `launch=true` starts fetching the authority value; `false` only defers
    /// it, leaving the Chance's authority state at `NotRequested`.
    fn complete_launch(
        &mut self,
        tool: ToolPending,
        result: Result<ControlResult, String>,
        actions: &mut ControlActions,
    ) -> io::Result<()> {
        let chance_id = tool.chance_id.as_str();
        let still_waiting = self.chance_is(chance_id, &ChanceStage::Running)
            && self.chance(chance_id)?.authority == AuthorityState::NotRequested;
        match result {
            Ok(ControlResult::Launch(true)) if still_waiting => {
                self.change_authority_state(chance_id, AuthorityState::Fetching)?;
            }
            Ok(ControlResult::Launch(_)) => {}
            Err(error) => actions.records.push(
                json!({"message":"policy_error", "chance_id":chance_id, "reason":error}),
            ),
            _ => unreachable!(),
        }
        Ok(())
    }

    /// `rebalance` pauses, resumes or preempts started candidate work. Pause
    /// and resume are commands; the Branch becomes `Blocked(Rebalanced)` only
    /// when the Agent Adapter reports that it actually holds the Branch.
    fn complete_rebalance(
        &mut self,
        result: Result<ControlResult, String>,
        actions: &mut ControlActions,
    ) -> io::Result<()> {
        let rebalance = match result {
            Ok(ControlResult::Rebalance(rebalance)) => rebalance,
            Err(error) => {
                actions
                    .records
                    .push(json!({"message":"policy_error", "decision":"rebalance", "reason":error}));
                return Ok(());
            }
            _ => unreachable!(),
        };
        if self.stopping {
            return Ok(());
        }
        for (branch_id, action) in rebalance {
            let Some(status) = self.branches.get(&branch_id).map(|branch| branch.status) else {
                continue;
            };
            match (action.as_str(), status) {
                ("pause", BranchStatus::Running) => {
                    actions.commands.push(AgentCommand::Pause { branch_id })
                }
                ("resume", BranchStatus::Blocked(BlockedReason::Rebalanced)) => {
                    actions.commands.push(AgentCommand::Resume { branch_id })
                }
                ("preempt", status) if status.is_live_candidate() => {
                    actions.commands.extend(self.discard_candidate(
                        &branch_id,
                        DiscardedReason::BudgetExhausted,
                        "Rebalance preempted this candidate",
                    )?)
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn complete_establish(
        &mut self,
        command: AgentCommand,
        result: Result<ControlResult, String>,
        actions: &mut ControlActions,
        branch: &str,
    ) -> io::Result<()> {
        match result {
            Ok(ControlResult::Completed) => {
                self.record_runtime_state(branch, RuntimeExecutionState::Ready)?;
                if !self.stopping && self.branches[branch].status == BranchStatus::Created {
                    actions.commands.push(command);
                }
            }
            Err(reason) => {
                self.record_runtime_state(branch, RuntimeExecutionState::Failed)?;
                actions.records.push(json!({"message":"runtime_operation_failed","branch_id":branch,"reason":reason}));
                actions.commands.extend(self.discard_candidate(
                    branch,
                    DiscardedReason::Failed,
                    &format!("Runtime environment establishment failed: {reason}"),
                )?);
            }
            _ => unreachable!(),
        }
        Ok(())
    }

    fn complete_invoke(
        &mut self,
        tool: ToolPending,
        result: Result<ControlResult, String>,
        actions: &mut ControlActions,
    ) -> io::Result<()> {
        let ToolPending {
            chance_id,
            branch_id,
            operation_id,
            ..
        } = tool;
        let live = self
            .chances
            .get(&chance_id)
            .is_some_and(|chance| self.chance_waiting_live(chance));
        let (outcome, evidence) = match result {
            Ok(ControlResult::Outcome(outcome, evidence)) => (outcome, evidence),
            Err(error) => {
                self.change_authority_state(&chance_id, AuthorityState::Unavailable)?;
                self.change_chance_stage(
                    &chance_id,
                    ChanceStage::Discarded(DiscardedReason::Failed),
                    "Authority value unavailable",
                )?;
                return Err(io::Error::other(error));
            }
            _ => unreachable!(),
        };
        self.record_authority_outcome(&chance_id, outcome.clone(), Some(evidence.clone()))?;
        self.record_public_nle_map_action(&branch_id, &operation_id, &evidence)?;
        if self.chance(&chance_id)?.branch_ids.is_empty() {
            // No Proposal was tested; nothing was speculated on this Chance.
            self.change_chance_stage(
                &chance_id,
                ChanceStage::Discarded(DiscardedReason::NotSelected),
                "Authority value arrived without a candidate",
            )?;
        } else {
            self.change_chance_stage(
                &chance_id,
                ChanceStage::Resolving,
                "Authority value arrived",
            )?;
        }
        self.complete_operation(&branch_id, Some(&operation_id), OperationStatus::Succeeded)?;
        if live {
            // The extent hold ends once the authority value is handed back.
            if self.branch(&branch_id)?.status == BranchStatus::Blocked(BlockedReason::ExtentReached)
            {
                self.change_status(&branch_id, BranchStatus::Running, "Authority value delivered")?;
            }
            actions
                .commands
                .push(self.resolve_pending(&branch_id, &operation_id, outcome)?);
        }
        Ok(())
    }

    fn complete_validate(
        &mut self,
        validation_id: u64,
        candidate: String,
        result: Result<ControlResult, String>,
        actions: &mut ControlActions,
    ) -> io::Result<()> {
        if self.stopping || !self.candidate_due(&candidate) {
            return Ok(());
        }
        let (compatible, verdict, notification, validated_at_ms, error) = match result {
            Ok(ControlResult::Disposition(compatible, verdict, notification, validated_at_ms)) => {
                (compatible, verdict, notification, validated_at_ms, None)
            }
            Err(error) => (false, "error".into(), None, None, Some(error)),
            _ => unreachable!(),
        };
        let proposal = self
            .branch_proposal(&candidate)
            .ok_or_else(|| io::Error::other("candidate Proposal is missing"))?
            .clone();
        let chance_id = self.branch(&candidate)?.chance_id.clone()
            .ok_or_else(|| io::Error::other("candidate Chance is missing"))?;
        if error.is_none() {
            self.record_validation(&chance_id, &proposal.proposal_id, validation_id, validated_at_ms);
        }
        let record = json!({"branch_id":candidate, "hypothesis":proposal.payload, "verdict":verdict, "policy_error":error});
        if self.can_promote(&candidate, compatible) {
            self.chance_mut(&chance_id)?.validation_id = Some(validation_id);
            self.record_runtime_state(&candidate, RuntimeExecutionState::PromotionRequested)?;
            self.change_status(
                &candidate,
                BranchStatus::Promoting,
                "Validation accepted; Runtime promotion requested",
            )?;
            let mut record = record;
            if let Some(notification) = notification {
                record["notification"] = json!(notification);
            }
            self.schedule(ControlOperation::Promote(candidate, record), actions);
        } else if compatible && self.ancestor_assumptions_unresolved(self.branch(&candidate)?) {
            // The nested Proposal matches, but the Branch still depends on an
            // unconfirmed ancestor Proposal. Keep it alive without commit
            // eligibility; `progress` will validate it again after the
            // ancestor resolves.
            actions.records.push(json!({
                "branch_id": candidate,
                "hypothesis": self.branch_proposal(&candidate).map(|proposal| &proposal.payload),
                "verdict": verdict,
                "disposition": "Wait",
                "reason": "ancestor Proposal is not confirmed"
            }));
        } else {
            let why = if error.is_some() {
                DiscardedReason::Failed
            } else {
                DiscardedReason::MatchRejected
            };
            actions.commands.extend(self.discard_candidate(
                &candidate,
                why,
                &format!(
                    "Validation disposition: {verdict}; {}",
                    error.as_deref().unwrap_or("no policy error")
                ),
            )?);
            let mut record = record;
            record["disposition"] = json!("Discard");
            actions.records.push(record);
        }
        Ok(())
    }

    fn complete_promote(
        &mut self,
        candidate: String,
        mut record: Value,
        result: Result<ControlResult, String>,
        actions: &mut ControlActions,
    ) -> io::Result<()> {
        let succeeded = result.is_ok();
        self.record_runtime_state(
            &candidate,
            if succeeded {
                RuntimeExecutionState::Promoted
            } else {
                RuntimeExecutionState::Failed
            },
        )?;
        if !self.stopping && self.branches[&candidate].status == BranchStatus::Promoting {
            let notification = record
                .get("notification")
                .and_then(Value::as_str)
                .map(str::to_owned);
            actions
                .commands
                .extend(self.finish_runtime_promotion(&candidate, succeeded, notification)?);
            record["disposition"] = json!(if succeeded { "Promote" } else { "Discard" });
            record["runtime_promote_error"] = json!(result.err());
            actions.records.push(record);
        }
        Ok(())
    }

    fn complete_terminate_or_cleanup(
        &mut self,
        work: ControlOperation,
        result: Result<ControlResult, String>,
        actions: &mut ControlActions,
        branch: &str,
    ) -> io::Result<()> {
        if let Err(error) = result {
            self.record_runtime_state(branch, RuntimeExecutionState::Failed)?;
            return Err(io::Error::other(error));
        }
        let state = if matches!(work, ControlOperation::Terminate(_)) {
            RuntimeExecutionState::Terminated
        } else {
            RuntimeExecutionState::Cleaned
        };
        self.record_runtime_state(branch, state)?;
        if state == RuntimeExecutionState::Terminated {
            self.schedule(ControlOperation::Cleanup(branch.to_string()), actions);
        }
        Ok(())
    }

    /// Drive each Chance forward: `select` over the Pending ones, `launch`
    /// for those waiting to fetch the authority value, and the authority
    /// fetch itself once its candidate environments exist.
    fn progress_chances(
        &mut self,
        capabilities: &RuntimeCapabilities,
        actions: &mut ControlActions,
    ) -> io::Result<()> {
        let selectable = self.selectable_chances();
        if !selectable.is_empty()
            && !self
                .pending_work
                .values()
                .any(|work| matches!(work, ControlOperation::Select(_)))
        {
            self.schedule(ControlOperation::Select(selectable), actions);
        }
        let ids: Vec<String> = self.chances.keys().cloned().collect();
        for id in ids {
            let chance = self.chance(&id)?;
            if !self.chance_waiting_live(chance)
                || self
                    .pending_work
                    .values()
                    .any(|work| work.chance_id() == Some(id.as_str()))
            {
                continue;
            }
            if chance.authority == AuthorityState::NotRequested && chance.stage.is_terminal() {
                // Every candidate is gone, but the Agent's call still needs
                // its real result.
                self.change_authority_state(&id, AuthorityState::Fetching)?;
            }
            let chance = self.chance(&id)?;
            match chance.authority {
                AuthorityState::NotRequested
                    if chance.stage == ChanceStage::Running
                        && chance.launch_epoch != Some(self.epoch) =>
                {
                    let tool = self.tool_pending(&id)?;
                    self.chance_mut(&id)?.launch_epoch = Some(self.epoch);
                    self.schedule(ControlOperation::Launch(tool), actions);
                }
                AuthorityState::Fetching => {
                    let establishing = self.pending_work.values().any(|work| {
                        matches!(work, ControlOperation::Establish(AgentCommand::StartCandidate { branch_id, .. })
                            if chance.branch_ids.contains(branch_id))
                    });
                    if !establishing
                        && capabilities
                            .tools
                            .iter()
                            .any(|declared| declared.name == chance.tool_name)
                    {
                        let tool = self.tool_pending(&id)?;
                        self.schedule(ControlOperation::Invoke(tool), actions);
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub fn pending_work(&self) -> bool {
        !self.pending_work.is_empty()
    }

    pub fn progress(&mut self, capabilities: &RuntimeCapabilities) -> io::Result<ControlActions> {
        let mut actions = ControlActions::default();
        if self.recovering {
            return Ok(actions);
        }
        self.check_command_deadlines()?;
        self.sweep_chances()?;
        if !self.stopping {
            self.progress_chances(capabilities, &mut actions)?;
            for candidate in self.pending_candidates() {
                if self.candidate_due(&candidate)
                    && !self
                        .pending_work
                        .values()
                        .any(|work| work.branch() == Some(candidate.as_str()))
                {
                    if self.candidate_outcome(&candidate).is_some() {
                        self.schedule(ControlOperation::Validate(candidate), &mut actions);
                    } else {
                        actions.commands.extend(self.discard_candidate(
                            &candidate,
                            DiscardedReason::Failed,
                            "Candidate validation deadline expired",
                        )?);
                        actions.records.push(json!({"branch_id":candidate,"disposition":"Discard","verdict":"timeout"}));
                    }
                }
            }
            if self.rebalance_epoch != Some(self.epoch)
                && !self
                    .pending_work
                    .values()
                    .any(|work| matches!(work, ControlOperation::Rebalance))
                && self.branches.values().any(|branch| {
                    branch.role == BranchRole::Candidate
                        && matches!(
                            branch.status,
                            BranchStatus::Running | BranchStatus::Blocked(BlockedReason::Rebalanced)
                        )
                })
            {
                self.rebalance_epoch = Some(self.epoch);
                self.schedule(ControlOperation::Rebalance, &mut actions);
            }
            if let Some(command) = self.next_prompt()? {
                actions.commands.push(command);
            }
        }
        if capabilities.branch_environment && !capabilities.shared_environment {
            let clean: Vec<_> = self
                .branches
                .values()
                .filter(|branch| {
                    branch.status == BranchStatus::Cleaned
                        && !matches!(
                            branch.execution_state.runtime,
                            RuntimeExecutionState::Cleaned
                                | RuntimeExecutionState::TerminationRequested
                        )
                        && !self
                            .pending_work
                            .values()
                            .any(|work| work.branch() == Some(branch.branch_id.as_str()))
                })
                .map(|branch| branch.branch_id.clone())
                .collect();
            for branch in clean {
                self.record_runtime_state(&branch, RuntimeExecutionState::TerminationRequested)?;
                self.schedule(ControlOperation::Terminate(branch), &mut actions);
            }
        }
        Ok(actions)
    }
}
