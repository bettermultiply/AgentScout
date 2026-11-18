//! Session startup and sequential control coordination, independent of presentation and transport.
use super::*;
use super::{execution::runtime_call, recovery::Recovery, task::Task};
use crate::control_plane::gateway::ui::Request;
use crate::control_plane::recovery::Resume;
use crate::protocol::contract::{AgentLaunch, ResultStatus};
use std::path::Path;

/// Restore the saved execution or create its root control context before assembly.
pub fn prepare(
    run_dir: &Path,
    resume: bool,
) -> io::Result<(SpeculativeTree, Option<Resume>)> {
    if resume {
        let (tree, saved) = crate::control_plane::recovery::load(run_dir)?;
        Ok((tree, Some(saved)))
    } else {
        if run_dir.join("recovery.json").exists() {
            return Err(io::Error::other("Saved execution exists; use --resume"));
        }
        Ok((SpeculativeTree::with_workspace("agent", run_dir)?, None))
    }
}

/// Establish/load Runtime state and ask Runtime to host the configured Agent.
/// The returned command is completed by `connected` after Agent connection acceptance.
pub fn start(
    tree: &mut SpeculativeTree,
    runtime: &dyn Runtime,
    launch: AgentLaunch,
    recovery_generation: Option<&Path>,
) -> Result<AgentCommand, Box<dyn std::error::Error>> {
    tree.set_agent_launch(launch.clone());
    let mut command = if let Some(path) = recovery_generation {
        AgentCommand::Load {
            execution_accesses: Default::default(),
            environments: tree
                .branches
                .iter()
                .filter_map(|(id, record)| {
                    record.environment_id.clone().map(|env| (id.clone(), env))
                })
                .collect(),
            branch_id: tree
                .authority_branch_id
                .clone()
                .ok_or("Recovery authority missing")?,
            path: path.join("agent.json"),
        }
    } else {
        let branch = tree.reopened_execution_scope().map_or_else(
            || "authority".to_owned(),
            |scope| format!("authority-{scope}"),
        );
        tree.create_authority(&branch, "")?
    };
    let authority = tree
        .authority_branch_id
        .clone()
        .ok_or("authority missing")?;
    if let Some(path) = recovery_generation {
        runtime_call(
            runtime,
            "",
            Operation::LoadRuntime {
                path: path.join("runtime.json"),
            },
        )?;
    } else {
        let established = runtime.establish()?;
        if !established.environment_id.is_empty() {
            tree.bind_environment(&authority, established.environment_id.clone())?;
        }
        if established.status != ResultStatus::Completed || established.environment_id.is_empty() {
            return Err("Runtime failed to establish the authority Environment".into());
        }
        if let AgentCommand::StartBranch { environment_id, execution_access, .. } = &mut command {
            *environment_id = Some(established.environment_id);
            *execution_access = established.execution_access;
        }
    }
    let environment = tree
        .branch(&authority)?
        .environment_id
        .as_deref()
        .ok_or("authority Environment missing")?;
    let started = runtime_call(
        runtime,
        environment,
        Operation::OperateEnvironment {
            command: Vec::new(),
            agent: Some(launch.clone()),
        },
    )?;
    if let AgentCommand::StartBranch {
        execution_access, ..
    } = &mut command
    {
        *execution_access = started.execution_access.clone();
    }
    if let AgentCommand::Load {
        execution_accesses, ..
    } = &mut command
    {
        if let Some(access) = started.execution_access {
            execution_accesses.insert(authority.clone(), access);
        }
    }
    if recovery_generation.is_some() {
        for branch in tree.branches.values().filter(|branch| {
            branch.branch_id != authority
                && (branch.status.is_live_candidate() || branch.status.is_live_authority())
        }) {
            let started = runtime_call(
                runtime,
                branch
                    .environment_id
                    .as_deref()
                    .ok_or("live Branch Environment missing")?,
                Operation::OperateEnvironment {
                    command: Vec::new(),
                    agent: Some(launch.clone()),
                },
            )?;
            if let AgentCommand::Load {
                execution_accesses, ..
            } = &mut command
            {
                if let Some(access) = started.execution_access {
                    execution_accesses.insert(branch.branch_id.clone(), access);
                }
            }
        }
    }
    Ok(command)
}

/// Admit the connected Agent and deliver local Policy before root startup/load.
pub fn connected(
    tree: &mut SpeculativeTree,
    policy: &CompiledPolicy,
    command: AgentCommand,
) -> Result<Vec<AgentCommand>, Box<dyn std::error::Error>> {
    let authority = tree
        .authority_branch_id
        .clone()
        .ok_or("authority missing")?;
    tree.record_runtime_state(&authority, RuntimeExecutionState::Ready)?;
    let decisions = [
        Decision::OpportunityPass,
        Decision::FeasibilityPass,
        Decision::Extent,
        Decision::Effect,
    ]
    .into_iter()
    .map(|decision| {
        Ok((
            decision.as_str().to_string(),
            json!(policy.script(decision)?),
        ))
    })
    .collect::<Result<serde_json::Map<_, _>, Box<dyn std::error::Error>>>()?;
    Ok(vec![
        AgentCommand::LocalPolicy {
            branch_id: authority,
            policy_digest: policy.digest().into(),
            has_predictor: policy.has_decision(Decision::Predict),
            decisions: Value::Object(decisions),
        },
        command,
    ])
}

/// Only coordination bookkeeping lives here; all speculative domain state remains in Tree.
pub struct Session {
    recovery: Recovery,
    lifecycle: Task,
    notice: String,
    input_sequence: u64,
    input_error: Option<String>,
    pending_model: Option<String>,
    model_changing: bool,
}

impl Session {
    pub fn notice(&self) -> &str {
        &self.notice
    }
    pub fn input_sequence(&self) -> u64 {
        self.input_sequence
    }
    pub fn input_error(&self) -> &Option<String> {
        &self.input_error
    }

    pub fn new(run_dir: &Path, model: Option<String>) -> Self {
        Self {
            recovery: Recovery::new(run_dir),
            lifecycle: Task::default(),
            notice: "Starting Agent session…".into(),
            input_sequence: 0,
            input_error: None,
            pending_model: model,
            model_changing: false,
        }
    }

    pub fn interaction(
        &mut self,
        tree: &mut SpeculativeTree,
        request: Request,
    ) -> io::Result<Option<AgentCommand>> {
        match request {
            Request::UiPromptAnswer {
                prompt_id,
                answer,
                cancelled,
            } => tree
                .answer_ui_prompt(prompt_id, answer, cancelled)
                .map(Some),
            Request::GetCommands { request_id } => tree.get_commands(request_id),
            _ => Err(io::Error::other("Invalid interaction request")),
        }
    }

    pub fn request(
        &mut self,
        tree: &mut SpeculativeTree,
        request: Request,
        at: u64,
    ) -> io::Result<ControlActions> {
        let commands = match request {
            Request::UiPromptAnswer { .. } | Request::GetCommands { .. } => Vec::new(),
            Request::Pause => {
                if tree.task_finishing() || tree.is_stopping() {
                    Vec::new()
                } else {
                    self.recovery.pause(tree, at)?
                }
            }
            Request::Continue => {
                if !tree.recovering() || tree.task_finishing() || tree.is_stopping() {
                    Vec::new()
                } else if self.recovery.continue_execution(tree, at) {
                    self.notice = "Continuing task".into();
                    vec![AgentCommand::Continue {
                        branch_id: tree.authority_branch_id.clone().unwrap(),
                    }]
                } else {
                    self.notice = "Pause in progress; wait before /continue".into();
                    Vec::new()
                }
            }
            Request::Shutdown | Request::Interrupt => {
                self.notice = "Task cancellation requested".into();
                self.lifecycle
                    .cancel(tree, matches!(request, Request::Shutdown))?
            }
            Request::Model { model } => {
                if tree.recovering()
                    || tree.authority_running()
                    || tree.pending_work()
                    || tree.recovery_pending_commands()
                    || self.model_changing
                {
                    self.notice = "Model change failed: wait for an idle authority".into();
                } else {
                    self.pending_model = Some(model);
                }
                Vec::new()
            }
            Request::Prompt { prompt } => {
                if prompt.trim() == "/pause" {
                    return self.request(tree, Request::Pause, at);
                }
                if prompt.trim() == "/continue" {
                    return self.request(tree, Request::Continue, at);
                }
                let paused = tree.recovering();
                if tree.task_finishing() {
                    self.lifecycle.retry_materialization();
                    self.notice = "Finishing previous task; input deferred".into();
                }
                tree.queue_prompt(prompt);
                self.input_sequence += 1;
                self.input_error = None;
                self.notice = if paused { "Input queued while paused" } else { "Agent running" }.into();
                Vec::new()
            }
        };
        Ok(ControlActions {
            commands,
            ..Default::default()
        })
    }

    pub fn agent(
        &mut self,
        tree: &mut SpeculativeTree,
        message: &AgentMessage,
    ) -> io::Result<ControlActions> {
        match message {
            AgentMessage::TurnFailed {
                branch_id, reason, ..
            } => {
                if tree.authority_branch_id.as_ref() == Some(branch_id) {
                    self.input_error = Some(reason.clone());
                    self.lifecycle.fail(tree);
                }
                self.notice = format!("Turn failed; session retained: {reason}");
            }
            AgentMessage::ModelChanged { model, error, .. } => {
                self.model_changing = false;
                self.notice = match error {
                    Some(error) => format!("Model change failed: {error}"),
                    None => format!("Model changed: {model}"),
                };
            }
            _ => {}
        }
        if let Some(notice) = self.recovery.handle(tree, message)? {
            self.notice = notice;
        }
        if matches!(message, AgentMessage::SpeculativeAdmissionRequest { .. }) {
            tree.speculative_admission(message.clone())
        } else {
            Ok(ControlActions {
                commands: tree.handle_message(message.clone())?,
                ..Default::default()
            })
        }
    }

    pub fn timer(&mut self, tree: &mut SpeculativeTree) -> io::Result<()> {
        if !tree.recovering() {
            tree.check_command_deadlines()?;
        }
        Ok(())
    }

    pub fn progress(
        &mut self,
        tree: &mut SpeculativeTree,
        capabilities: &RuntimeCapabilities,
        mut actions: ControlActions,
    ) -> io::Result<ControlActions> {
        if let Some(reason) = tree.authority_failure()
            && !tree.is_stopping()
        {
            self.input_error = Some(reason.to_string());
            self.lifecycle.fail(tree);
        }
        let progress =
            if !tree.recovering() && (self.model_changing || self.pending_model.is_some()) {
                ControlActions::default()
            } else {
                tree.progress(capabilities)?
            };
        actions.commands.extend(progress.commands);
        let mut ui_commands = tree.drain_ui_commands();
        ui_commands.append(&mut actions.commands);
        actions.commands = ui_commands;
        actions.operations.extend(progress.operations);
        actions.records.extend(progress.records);
        actions.commands.extend(self.lifecycle.progress(tree)?);
        // Never switch beneath in-flight work or a pending Promote. Descendants
        // retain their own model; new sessions inherit their actual parent's.
        let idle = (!tree.recovering() || self.recovery.loaded)
            && !self.model_changing
            && tree.authority_ready()
            && !tree.authority_running()
            && !tree.pending_work()
            && !tree.recovery_pending_commands();
        if idle && let Some(model) = self.pending_model.take() {
            self.model_changing = true;
            actions.commands.push(AgentCommand::Model {
                branch_id: tree.authority_branch_id.clone().unwrap(),
                model,
            });
        }
        Ok(actions)
    }

    pub fn input_ready(&self, tree: &SpeculativeTree) -> bool {
        tree.input_ready() && !self.model_changing && self.pending_model.is_none()
    }

    pub fn saved(&self) -> bool {
        self.recovery.saved
    }

    pub fn fail_and_exit(
        &mut self,
        tree: &mut SpeculativeTree,
        error: &io::Error,
    ) -> io::Result<ControlActions> {
        self.input_error = Some(error.to_string());
        Ok(ControlActions {
            commands: self.lifecycle.fail_and_exit(tree)?,
            ..Default::default()
        })
    }

    /// Coordinate persistence/materialization after asynchronous work is dispatched.
    /// The caller only sends commands, wakes its event loop and closes transport.
    pub fn settle(
        &mut self,
        tree: &mut SpeculativeTree,
        runtime: &dyn Runtime,
        _capabilities: &RuntimeCapabilities,
        run_dir: &Path,
        draining: bool,
        mut record: impl FnMut(Value) -> io::Result<()>,
        mut materialize: impl FnMut(String),
    ) -> io::Result<(Vec<AgentCommand>, bool, bool)> {
        let mut commands = Vec::new();
        let mut wake = false;
        if let Some(command) = self.recovery.save_command(tree) {
            commands.push(command);
        }
        if let Some((command, notice)) = self.recovery.publish(tree, runtime, run_dir, draining) {
            commands.push(command);
            self.notice = notice;
        }
        let terminating = self.lifecycle.terminating;
        if let Some(environment) = self.lifecycle.start_materialization(tree, &mut record)? {
            materialize(environment);
        }
        if let Some((materialization, ready, result)) =
            self.lifecycle
                .materialize(tree, self.input_error.is_some(), record)?
        {
            commands.extend(materialization);
            wake = ready;
            match result {
                Ok(()) => {
                    self.notice = if self.input_error.is_some() || terminating {
                        "Task ended"
                    } else {
                        "Task succeeded"
                    }
                    .into()
                }
                Err(error) => {
                    self.input_error = Some(format!("Materialization failed: {error}"));
                    self.notice = format!("Materialization failed; retained for retry: {error}");
                }
            }
        }
        let finish = !draining && self.lifecycle.finish_ready(tree)?;
        if !tree.task_finishing()
            && !tree.recovering()
            && !self.notice.starts_with("Save failed")
            && !self.notice.starts_with("Turn failed")
            && !self.notice.starts_with("Model ")
        {
            self.notice = if tree.is_stopping() {
                "Stopping"
            } else if tree.authority_running() {
                "Agent running"
            } else {
                "Ready"
            }
            .into();
        }
        Ok((commands, wake, finish))
    }

    pub fn materialized(
        &mut self,
        result: Result<crate::protocol::contract::OperationResult, String>,
    ) {
        self.lifecycle.materialized(result);
    }
}
