//! SpeculativeTree 的控制操作与动作投影。

use super::*;

impl SpeculativeTree {
    pub(super) fn schedule(&mut self, operation: ControlOperation, actions: &mut ControlActions) {
        let id = self.next_work;
        self.next_work += 1;
        self.pending_work.insert(id, operation.clone());
        actions.operations.push((id, operation));
    }

    pub(super) fn waiting_point_live(&self, tool: &ToolPending) -> bool {
        !self.stopping
            && self.can_invoke_tool(&tool.branch_id)
            && self.branches[&tool.branch_id]
                .pending_operations
                .iter()
                .any(|op| {
                    op.operation_id == tool.operation_id && op.status == OperationStatus::Requested
                })
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
        let mut actions = ControlActions::default();
        if self.stopping
            || !self.can_invoke_tool(&branch_id)
            || self.branches[&branch_id]
                .pending_operations
                .iter()
                .any(|op| op.operation_id == operation_id)
        {
            return Ok(actions);
        }
        // Admission was decided by the adapter that sent this explicit request.
        // The control-plane phase starts at selection; it does not re-run or
        // claim ownership of the local admission decision.
        self.branches
            .get_mut(&branch_id)
            .expect("admission branch exists")
            .execution_state
            .phase = BranchPhase::Select;
        self.mark_operation_requested(&branch_id, &operation_id, OperationKind::Tool)?;
        self.schedule(
            ControlOperation::Predict(ToolPending {
                branch_id,
                operation_id,
                tool_name,
                arguments,
                checkpoint,
                launch: true,
                speculation_started_at_ms: Some(timestamp_ms),
            }),
            &mut actions,
        );
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
        let branch = work.branch().to_string();
        match work {
            ControlOperation::Predict(tool) => {
                self.complete_predict(tool, result, &mut actions, &branch)?;
            }
            ControlOperation::Establish(command) => {
                self.complete_establish(command, result, &mut actions, &branch)?;
            }
            ControlOperation::Invoke(tool) => {
                self.complete_invoke(tool, result, &mut actions)?;
            }
            ControlOperation::Validate(candidate) => {
                self.complete_validate(candidate, result, &mut actions)?;
            }
            ControlOperation::Promote(candidate, record) => {
                self.complete_promote(candidate, record, result, &mut actions)?;
            }
            work @ (ControlOperation::Terminate(_) | ControlOperation::Cleanup(_)) => {
                self.complete_terminate_or_cleanup(work, result, &mut actions, &branch)?;
            }
        }
        Ok(actions)
    }

    fn complete_predict(
        &mut self,
        tool: ToolPending,
        result: Result<ControlResult, String>,
        actions: &mut ControlActions,
        branch: &str,
    ) -> io::Result<()> {
        let ToolPending {
            branch_id,
            operation_id,
            tool_name,
            checkpoint,
            speculation_started_at_ms,
            ..
        } = &tool;
        if !self.waiting_point_live(&tool) {
            return Ok(());
        }
        self.branches
            .get_mut(branch_id)
            .expect("live waiting branch")
            .execution_state
            .phase = BranchPhase::Predict;
        let launch_unsupported =
            matches!(&result, Ok(ControlResult::PredictionsWithoutAuthority(_)));
        // An empty prediction set is not a failed speculation: it means this
        // boundary has no hypothesis to test. Execute the tool as an ordinary
        // authority action instead of leaving the adapter's tool call waiting
        // for a result that will never arrive.
        let mut direct_invocation = false;
        match result {
            Ok(ControlResult::Predictions(predictions)) if predictions.is_empty() => {
                direct_invocation = true;
                self.tool_invocations.push(tool.clone());
            }
            Ok(ControlResult::Predictions(predictions)) => {
                self.start_candidates(
                    branch_id,
                    operation_id,
                    tool_name,
                    predictions,
                    checkpoint.clone(),
                    *speculation_started_at_ms,
                    actions,
                    false,
                )?;
            }
            Ok(ControlResult::PredictionsWithoutAuthority(predictions)) => {
                self.start_candidates(
                    branch_id,
                    operation_id,
                    tool_name,
                    predictions,
                    checkpoint.clone(),
                    *speculation_started_at_ms,
                    actions,
                    true,
                )?;
            }
            Err(error) => actions
                .records
                .push(json!({"message":"policy_error", "branch_id":branch, "reason":error})),
            _ => unreachable!(),
        }
        self.finish_predict_waiting_point(tool, direct_invocation, launch_unsupported, actions);
        Ok(())
    }

    /// An empty prediction set includes the truthful launch=false prototype
    /// result: the Proposal is not cancelled, but validator/evidence-only
    /// execution is unsupported. Do not retry the same point forever.
    fn finish_predict_waiting_point(
        &mut self,
        tool: ToolPending,
        direct_invocation: bool,
        launch_unsupported: bool,
        actions: &mut ControlActions,
    ) {
        let ToolPending {
            branch_id,
            operation_id,
            ..
        } = &tool;
        if !direct_invocation
            && !launch_unsupported
            && !self.branches.values().any(|candidate| {
                candidate.parent_branch_id.as_deref() == Some(branch_id)
                    && candidate.proposal_id.is_some()
            })
        {
            actions.records.push(json!({
                "message": "launch_not_started",
                "branch_id": branch_id,
                "operation_id": operation_id,
                "status": "unsupported",
                "reason": "validator/evidence-only path is unsupported; Proposal retained outside Branch execution"
            }));
        } else if !direct_invocation && !launch_unsupported {
            // Retain the waiting point until all child environments exist.
            self.tool_invocations.push(tool);
        }
    }

    fn start_candidates(
        &mut self,
        branch_id: &str,
        operation_id: &str,
        tool_name: &str,
        predictions: Vec<Value>,
        checkpoint: Option<Value>,
        speculation_started_at_ms: Option<u64>,
        actions: &mut ControlActions,
        unsupported: bool,
    ) -> io::Result<()> {
        for prediction in predictions {
            let command = self.start_candidate(
                branch_id,
                operation_id,
                tool_name,
                prediction,
                checkpoint.clone(),
                speculation_started_at_ms,
            )?;
            let child = match &command {
                AgentCommand::StartCandidate { branch_id, .. } => branch_id,
                _ => unreachable!(),
            };
            if unsupported {
                actions.records.push(json!({"message":"candidate_created", "branch_id":child, "parent":branch_id, "launch":"unsupported"}));
            } else {
                actions.records.push(json!({"message":"candidate_created", "branch_id":child, "parent":branch_id, "fork_at":self.branches[child].execution_position.transcript_index}));
            }
            self.schedule(ControlOperation::Establish(command), actions);
        }
        if unsupported {
            actions.records.push(json!({"message":"launch_not_started", "branch_id":branch_id, "operation_id":operation_id, "status":"unsupported", "reason":"validator/evidence-only path is unsupported"}));
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
                actions.commands.extend(self.resolve_candidate_with_reason(
                    branch,
                    false,
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
        let live = self.waiting_point_live(&tool);
        let ToolPending {
            branch_id,
            operation_id,
            launch: _,
            ..
        } = tool;
        let (outcome, evidence) =
            result
                .map_err(io::Error::other)
                .and_then(|value| match value {
                    ControlResult::Outcome(outcome, evidence) => Ok((outcome, evidence)),
                    _ => unreachable!(),
                })?;
        self.record_candidate_outcome(&branch_id, &operation_id, outcome.clone(), Some(evidence))?;
        self.branches
            .get_mut(&branch_id)
            .expect("candidate parent exists")
            .execution_state
            .phase = BranchPhase::Effect;
        self.complete_operation(&branch_id, Some(&operation_id), OperationStatus::Succeeded)?;
        if live {
            actions
                .commands
                .push(self.resolve_pending(&branch_id, &operation_id, outcome)?);
        }
        Ok(())
    }

    fn complete_validate(
        &mut self,
        candidate: String,
        result: Result<ControlResult, String>,
        actions: &mut ControlActions,
    ) -> io::Result<()> {
        if self.stopping || !self.candidate_due(&candidate) {
            return Ok(());
        }
        self.branches
            .get_mut(&candidate)
            .expect("candidate exists")
            .execution_state
            .phase = BranchPhase::Dispose;
        let (compatible, verdict, error) = match result {
            Ok(ControlResult::Disposition(compatible, verdict)) => (compatible, verdict, None),
            Err(error) => (false, "error".into(), Some(error)),
            _ => unreachable!(),
        };
        let proposal = self.branches[&candidate]
            .proposal_id
            .as_ref()
            .and_then(|id| self.proposals.get(id))
            .ok_or_else(|| io::Error::other("candidate Proposal is missing"))?;
        let record = json!({"branch_id":candidate, "hypothesis":proposal.payload, "verdict":verdict, "policy_error":error});
        if self.can_promote(&candidate, compatible) {
            self.record_runtime_state(&candidate, RuntimeExecutionState::PromotionRequested)?;
            self.change_status(
                &candidate,
                BranchStatus::Promoting,
                "Validation accepted; Runtime promotion requested",
            )?;
            self.schedule(ControlOperation::Promote(candidate, record), actions);
        } else {
            actions.commands.extend(self.resolve_candidate_with_reason(
                &candidate,
                false,
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
            actions
                .commands
                .extend(self.finish_runtime_promotion(&candidate, succeeded)?);
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

    pub fn pending_work(&self) -> bool {
        !self.pending_work.is_empty()
    }

    pub fn progress(&mut self, capabilities: &RuntimeCapabilities) -> io::Result<ControlActions> {
        let mut actions = ControlActions::default();
        self.check_command_deadlines()?;
        if !self.stopping {
            for tool in std::mem::take(&mut self.tool_invocations) {
                let waiting = self.pending_work.values().any(|work| matches!(work, ControlOperation::Establish(AgentCommand::StartCandidate {parent_branch_id, operation_id, ..}) if parent_branch_id == &tool.branch_id && operation_id == &tool.operation_id));
                if waiting {
                    self.tool_invocations.push(tool);
                } else if self.waiting_point_live(&tool)
                    && capabilities
                        .tools
                        .iter()
                        .any(|declared| declared.name == tool.tool_name)
                {
                    if tool.launch {
                        if let Some(branch) = self.branches.get_mut(&tool.branch_id) {
                            branch.execution_state.phase = BranchPhase::Launch;
                        }
                        self.schedule(ControlOperation::Invoke(tool), &mut actions);
                    } else {
                        actions.records.push(json!({
                            "message": "authority_launch_not_started",
                            "branch_id": tool.branch_id,
                            "operation_id": tool.operation_id,
                            "reason": "policy launch=false; validator/evidence-only path unsupported"
                        }));
                        // launch=false is handled at prediction time and is
                        // never converted into an authority Invoke.
                    }
                }
            }
            for candidate in self.pending_candidates() {
                if self.candidate_due(&candidate)
                    && !self
                        .pending_work
                        .values()
                        .any(|work| work.branch() == candidate)
                {
                    if self.candidate_outcome(&candidate).is_some() {
                        self.branches
                            .get_mut(&candidate)
                            .expect("candidate exists")
                            .execution_state
                            .phase = BranchPhase::Match;
                        self.schedule(ControlOperation::Validate(candidate), &mut actions);
                    } else {
                        actions.commands.extend(self.resolve_candidate_with_reason(
                            &candidate,
                            false,
                            "Candidate validation deadline expired",
                        )?);
                        actions.records.push(json!({"branch_id":candidate,"disposition":"Discard","verdict":"timeout"}));
                    }
                }
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
                            .any(|work| work.branch() == branch.branch_id)
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
