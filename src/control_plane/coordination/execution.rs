//! Run ordering, Runtime result interpretation, and final environment cleanup.
use super::session::{self, Prepared, Session};
use super::{Operation, Runtime, RuntimeCapabilities, RuntimeExecutionState, SpeculativeTree};
use crate::control_plane::gateway::agent::protocol::{ActionKind, AgentCommand, AgentMessage};
use super::{ControlActions, ControlResult, SpeculationCoordinator};
use crate::control_plane::gateway::{
    agent,
    ui::{self, Request},
};
use crate::control_plane::ledger::Ledger;
use crate::control_plane::observation::ObservationContent;
use crate::control_plane::policy::{CompiledPolicy, PolicyEngine};
use crate::control_plane::recovery::Resume;
use crate::control_plane::system_log::SystemLog;
use crate::control_plane::ui_projection::UiProjection;
use crate::protocol::contract::{AgentLaunch, OperationResult};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io;
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::thread;
use std::time::{Duration, Instant};

pub fn runtime_call(
    runtime: &dyn Runtime,
    environment: &str,
    operation: Operation,
) -> Result<crate::protocol::contract::OperationResult, Box<dyn std::error::Error>> {
    let result = runtime.call(environment, operation)?;
    if result.status != crate::protocol::contract::ResultStatus::Completed {
        return Err(format!("Runtime request failed: {:?}", result.reason).into());
    }
    Ok(result)
}

fn runtime_cleanup(
    runtime: &dyn Runtime,
    environment: &str,
    branch: &str,
    tree: &mut SpeculativeTree,
) -> Result<(), Box<dyn std::error::Error>> {
    tree.record_runtime_state(branch, RuntimeExecutionState::TerminationRequested)?;
    if let Err(error) = runtime_call(runtime, environment, Operation::TerminateEnvironment) {
        tree.record_runtime_state(branch, RuntimeExecutionState::Failed)?;
        return Err(error);
    }
    tree.record_runtime_state(branch, RuntimeExecutionState::Terminated)?;
    if let Err(error) = runtime_call(runtime, environment, Operation::CleanupEnvironment) {
        tree.record_runtime_state(branch, RuntimeExecutionState::Failed)?;
        return Err(error);
    }
    tree.record_runtime_state(branch, RuntimeExecutionState::Cleaned)?;
    Ok(())
}

pub fn cleanup(
    runtime: &dyn Runtime,
    _capabilities: &RuntimeCapabilities,
    tree: &mut SpeculativeTree,
) -> Result<(), Box<dyn std::error::Error>> {
    if tree.recovering() || tree.task_finishing() || tree.authority_branch_id.is_none() {
        return Ok(());
    }
    {
        let ids: Vec<_> = tree
            .branches
            .values()
            .filter(|branch| branch.execution_state.runtime != RuntimeExecutionState::Cleaned)
            .map(|branch| branch.branch_id.clone())
            .collect();
        let mut failures = Vec::new();
        for id in ids {
            let Some(environment) = tree
                .branches
                .get(&id)
                .and_then(|branch| branch.environment_id.clone())
            else {
                continue;
            };
            if let Err(error) = runtime_cleanup(runtime, &environment, &id, tree) {
                failures.push(format!("{id}: {error}"));
            }
        }
        if !failures.is_empty() {
            return Err(failures.join("; ").into());
        }
    }
    Ok(())
}

pub enum Event {
    Interaction(Request, mpsc::Sender<Result<(), String>>),
    Agent(io::Result<Option<AgentMessage>>),
    Request(Request),
    Completed(u64, Result<ControlResult, String>),
    Materialized(Result<OperationResult, String>),
    Delivered(AgentCommand, io::Result<()>),
    Timer,
}

/// Display data forwarded in execution order. The receiver only updates its
/// disposable cache; the Coordinator publishes the corresponding observation.
pub enum PresentationUpdate<'a> {
    Fork {
        parent: &'a str,
        child: &'a str,
    },
    Output {
        branch_id: &'a str,
        text: &'a str,
    },
    SessionHistory {
        branch_id: &'a str,
        entries: &'a [Value],
    },
    Transcript {
        branch_id: &'a str,
        entry: &'a Value,
    },
    InputStatus {
        sequence: u64,
        ready: bool,
        error: Option<&'a str>,
    },
}

pub trait PresentationUpdates {
    fn update(&self, update: PresentationUpdate<'_>);
}

pub fn ui_connection(projection: UiProjection, events: mpsc::Sender<Event>) -> ui::Connection {
    ui::Connection::new(projection, move |request| {
        let disconnected = || "Control plane disconnected".to_string();
        if matches!(
            request,
            Request::UiPromptAnswer { .. } | Request::GetCommands { .. }
        ) {
            let (reply, receiver) = mpsc::channel();
            events
                .send(Event::Interaction(request, reply))
                .map_err(|_| disconnected())?;
            receiver.recv().map_err(|_| disconnected())?
        } else {
            events
                .send(Event::Request(request))
                .map_err(|_| disconnected())
        }
    })
}

/// Start the Agent, run its session and finish even when connection or host setup fails.
/// The host starts its presentation after admission and can scope resources to the event loop.
pub fn execute<Guard>(
    prepared: Prepared,
    ledger: Ledger,
    system_log: &SystemLog,
    presentation: &dyn PresentationUpdates,
    runtime: &dyn Runtime,
    capabilities: &RuntimeCapabilities,
    launch: AgentLaunch,
    listener: &agent::Listener,
    channel: (mpsc::Sender<Event>, mpsc::Receiver<Event>),
    before_run: impl FnOnce() -> Result<Guard, Box<dyn std::error::Error>>,
) -> Vec<String> {
    let Prepared {
        run_dir,
        mut tree,
        policy,
        engine,
        saved,
        resume_model,
        ..
    } = prepared;
    let generation = match &saved {
        Some(Resume::Paused(path)) => Some(path.as_path()),
        _ => None,
    };
    let mut execution = Execution::new(
        ledger,
        &tree,
        &run_dir.join("session.jsonl"),
        system_log,
        presentation,
    );
    let result = (|| {
        let (mut connection, commands) =
            execution.connect(&mut tree, runtime, launch, generation, listener, &policy)?;
        let _guard = before_run()?;
        execution.run(
            &mut connection,
            &mut tree,
            &policy,
            &engine,
            runtime,
            capabilities,
            channel,
            &run_dir,
            resume_model,
            commands,
        )
    })();
    execution.finish(runtime, capabilities, &mut tree, &run_dir, result)
}

pub struct Execution<'a> {
    ledger: Ledger,
    system_log: &'a SystemLog,
    presentation: &'a dyn PresentationUpdates,
    run_id: String,
    last_status: Option<ObservationContent>,
    action_starts: BTreeMap<(String, String), (u64, String)>,
}

impl<'a> Execution<'a> {
    pub fn new(
        mut ledger: Ledger,
        tree: &SpeculativeTree,
        session_file: &Path,
        system_log: &'a SystemLog,
        presentation: &'a dyn PresentationUpdates,
    ) -> Self {
        ledger.set_system_log(system_log.clone());
        let run_id = tree.run_id().to_owned();
        if let Err(error) = ledger.record(json!({
            "source": "control_plane",
            "message": "session_file",
            "task_id": tree.execution_id(),
            "session_file": session_file,
            "run_id": run_id,
            "bindings": {
                "policy_binding": tree.policy_binding(),
                "runtime_binding": tree.runtime_binding(),
                "capability_view": tree.capability_view(),
                "local_policy_digest": tree.local_policy_digest(),
            },
        })) {
            system_log.history_failure(&run_id, &format!("record append failed: {error}"));
        }
        let last_status = ledger.reader().read_saved_after(0).ok().and_then(|events| {
            events
                .into_iter()
                .rev()
                .map(|event| event.content)
                .find(|content| matches!(content, ObservationContent::RunStatus { .. }))
        });
        Self {
            ledger,
            system_log,
            presentation,
            run_id,
            last_status,
            action_starts: BTreeMap::new(),
        }
    }

    pub fn connect(
        &mut self,
        tree: &mut SpeculativeTree,
        runtime: &dyn Runtime,
        launch: AgentLaunch,
        recovery_generation: Option<&Path>,
        listener: &agent::Listener,
        policy: &CompiledPolicy,
    ) -> Result<(agent::Connection, Vec<AgentCommand>), Box<dyn std::error::Error>> {
        let command = session::start(tree, runtime, launch, recovery_generation)?;
        let connection = listener.accept("agent", Duration::from_secs(30))?;
        let commands = session::connected(tree, policy, command)?;
        let _ = self.flush_observations(tree);
        let _ = self.publish(tree.execution_id(), false, false, "Starting Agent session…");
        Ok((connection, commands))
    }

    pub fn finish(
        &mut self,
        runtime: &dyn Runtime,
        capabilities: &RuntimeCapabilities,
        tree: &mut SpeculativeTree,
        run_dir: &Path,
        result: Result<(), Box<dyn std::error::Error>>,
    ) -> Vec<String> {
        let runtime_result = cleanup(runtime, capabilities, tree);
        let mut observation_result = self.flush_observations(tree);
        let task_id = tree.execution_id();
        observation_result = observation_result.and(self.publish(task_id, false, false, "Stopped"));
        observation_result = observation_result.and(self.append_observation(
            task_id,
            ObservationContent::RunStatus {
                ready: false,
                running: false,
                notice: "Control plane disconnected".into(),
                connected: false,
            },
        ));
        self.ledger.disconnect_readers();
        let mut errors = Vec::new();
        if let Err(error) = observation_result {
            errors.push(format!("Observation history incomplete: {error}"));
        }
        if let Err(error) = result {
            errors.push(format!("Agent execution/shutdown: {error}"));
        }
        if let Err(error) = runtime_result {
            errors.push(format!("Runtime cleanup: {error}"));
        }
        let final_state = serde_json::to_vec_pretty(tree)
            .map_err(io::Error::other)
            .and_then(|bytes| std::fs::write(run_dir.join("final-tree.json"), bytes));
        if let Err(error) = final_state {
            errors.push(format!("Final Tree recording: {error}"));
        }
        errors
    }

    fn append_observation(&mut self, task_id: &str, content: ObservationContent) -> io::Result<()> {
        self.ledger
            .append_observation(task_id, content)
            .inspect_err(|error| {
                self.system_log
                    .history_failure(&self.run_id, &format!("observation append failed: {error}"));
            })
    }

    fn observe(&mut self, task_id: &str, content: ObservationContent) {
        let _ = self.append_observation(task_id, content);
    }

    fn publish(
        &mut self,
        task_id: &str,
        ready: bool,
        running: bool,
        notice: &str,
    ) -> io::Result<()> {
        if matches!(&self.last_status, Some(ObservationContent::RunStatus {
            ready: previous_ready, running: previous_running, notice: previous_notice, connected: true,
        }) if (ready, running, notice) == (*previous_ready, *previous_running, previous_notice.as_str()))
        {
            return Ok(());
        }
        let status = ObservationContent::RunStatus {
            ready,
            running,
            notice: notice.into(),
            connected: true,
        };
        self.append_observation(task_id, status.clone())?;
        self.last_status = Some(status);
        Ok(())
    }

    fn flush_observations(&mut self, tree: &mut SpeculativeTree) -> io::Result<()> {
        let mut failure = None;
        for content in tree.drain_observations() {
            if let Err(error) = self.append_observation(tree.execution_id(), content) {
                failure.get_or_insert(error);
            }
        }
        failure.map_or(Ok(()), Err)
    }

    fn agent_observation(&mut self, task_id: &str, message: &AgentMessage) {
        let update = match message {
            AgentMessage::Output {
                branch_id, text, ..
            } => Some((branch_id, PresentationUpdate::Output { branch_id, text })),
            AgentMessage::SessionHistory { branch_id, entries } => Some((
                branch_id,
                PresentationUpdate::SessionHistory { branch_id, entries },
            )),
            AgentMessage::Transcript {
                branch_id, entry, ..
            } => Some((
                branch_id,
                PresentationUpdate::Transcript { branch_id, entry },
            )),
            _ => None,
        };
        if let Some((branch_id, update)) = update {
            self.presentation.update(update);
            self.observe(
                task_id,
                ObservationContent::ConversationUpdated {
                    branch_id: branch_id.clone(),
                },
            );
            return;
        }
        // Only the matching ActionStarted establishes the tool name and start time.
        let content = match message {
            AgentMessage::ActionStarted {
                branch_id,
                action_id,
                name,
                timestamp_ms,
                ..
            } => {
                self.action_starts.insert(
                    (branch_id.clone(), action_id.clone()),
                    (*timestamp_ms, name.clone()),
                );
                None
            }
            AgentMessage::Usage {
                branch_id,
                action_id,
                model,
                usage,
                started_at_ms,
                finished_at_ms,
                ..
            } => Some(ObservationContent::ModelUsage {
                branch_id: branch_id.clone(),
                action_id: action_id.clone(),
                model: model.clone(),
                input_tokens: usage["input"].as_u64(),
                output_tokens: usage["output"].as_u64(),
                cache_read_tokens: usage["cacheRead"].as_u64(),
                cache_write_tokens: usage["cacheWrite"].as_u64(),
                total_tokens: usage["totalTokens"].as_u64(),
                cost: usage["cost"]["total"].as_f64(),
                started_at_ms: Some(*started_at_ms),
                finished_at_ms: Some(*finished_at_ms),
                duration_ms: finished_at_ms.checked_sub(*started_at_ms),
            }),
            AgentMessage::ActionFinished {
                branch_id,
                action_id,
                action_kind: ActionKind::ToolExecution,
                status,
                timestamp_ms,
                ..
            } => {
                let start = self
                    .action_starts
                    .remove(&(branch_id.clone(), action_id.clone()));
                let started_at_ms = start.as_ref().map(|(timestamp, _)| *timestamp);
                let tool_name = start.map(|(_, name)| name);
                Some(ObservationContent::ToolFinished {
                    branch_id: branch_id.clone(),
                    action_id: action_id.clone(),
                    tool_name,
                    status: *status,
                    started_at_ms,
                    finished_at_ms: Some(*timestamp_ms),
                    duration_ms: started_at_ms.and_then(|start| timestamp_ms.checked_sub(start)),
                })
            }
            AgentMessage::ActionFinished {
                branch_id,
                action_id,
                ..
            } => {
                self.action_starts
                    .remove(&(branch_id.clone(), action_id.clone()));
                None
            }
            _ => None,
        };
        if let Some(content) = content {
            self.observe(task_id, content);
        }
    }
}

struct Task<'a, 'b> {
    tree: &'a mut SpeculativeTree,
    connection: &'a mut agent::Connection,
    capabilities: &'a RuntimeCapabilities,
    execution: &'a mut Execution<'b>,
    started: Instant,
    draining: bool,
    delivery_failed: bool,
    control: Session,
}

impl Task<'_, '_> {
    fn record(&mut self, mut value: Value) -> io::Result<()> {
        value["t_ms"] = json!(self.started.elapsed().as_millis() as u64);
        if value.get("task_id").is_none() {
            value["task_id"] = json!(self.tree.execution_id());
        }
        if let Err(error) = self.execution.ledger.record(value.clone()) {
            self.execution.system_log.history_failure(
                self.tree.run_id(),
                &format!("record append failed: {error}"),
            );
        }
        if value["message"] == "candidate_created" {
            let parent = value["parent"].as_str().unwrap();
            let child = value["branch_id"].as_str().unwrap();
            self.execution
                .presentation
                .update(PresentationUpdate::Fork { parent, child });
            self.execution.observe(
                self.tree.execution_id(),
                ObservationContent::ConversationUpdated {
                    branch_id: child.into(),
                },
            );
        }
        Ok(())
    }

    fn handle(&mut self, event: Event) -> io::Result<Option<ControlActions>> {
        let mut actions = match event {
            Event::Interaction(request, reply) => {
                let result = self.control.interaction(self.tree, request);
                match result {
                    Ok(command) => {
                        let _ = reply.send(Ok(()));
                        ControlActions {
                            commands: command.into_iter().collect(),
                            ..Default::default()
                        }
                    }
                    Err(error) => {
                        let _ = reply.send(Err(error.to_string()));
                        ControlActions::default()
                    }
                }
            }
            Event::Request(request) => self.control.request(self.tree, request, now_ms())?,
            Event::Completed(id, result) => self.tree.complete_work(id, result)?,
            Event::Materialized(result) => {
                self.control.materialized(result);
                ControlActions::default()
            }
            Event::Delivered(command, result) => match result {
                Err(error) if error.kind() != io::ErrorKind::Interrupted => {
                    let commands = self.tree.delivery_failed(&command, error.to_string())?;
                    if self.tree.authority_failure().is_some() && !self.delivery_failed {
                        self.delivery_failed = true;
                        self.control.fail_and_exit(self.tree, &error)?
                    } else {
                        ControlActions {
                            commands,
                            ..Default::default()
                        }
                    }
                }
                _ => ControlActions::default(),
            },
            Event::Timer => {
                self.control.timer(self.tree)?;
                ControlActions::default()
            }
            Event::Agent(message) => {
                let message = match message {
                    Ok(Some(message)) => message,
                    result => {
                        if self.draining {
                            return Ok(None);
                        }
                        let error = result.err().unwrap_or_else(|| {
                            io::Error::other("Agent disconnected; cleanup is not confirmed")
                        });
                        self.delivery_failed = true;
                        let actions = self.control.fail_and_exit(self.tree, &error)?;
                        return Ok(Some(self.control.progress(
                            self.tree,
                            self.capabilities,
                            actions,
                        )?));
                    }
                };
                let actions = self.control.agent(self.tree, &message)?;
                let _ = self.execution.flush_observations(self.tree);
                self.execution
                    .agent_observation(self.tree.execution_id(), &message);
                if !matches!(
                    message,
                    AgentMessage::Output { .. } | AgentMessage::SessionHistory { .. }
                ) {
                    let mut fact = serde_json::to_value(&message)?;
                    if let AgentMessage::Transcript { entry, .. } = &message {
                        fact.as_object_mut().unwrap().remove("entry");
                        if let Some(role) = entry.get("role").and_then(Value::as_str) {
                            fact["role"] = json!(role);
                        }
                    }
                    fact["source"] = json!("agent_adapter");
                    self.record(fact)?;
                }
                actions
            }
        };
        actions = self
            .control
            .progress(self.tree, self.capabilities, actions)?;
        Ok(Some(actions))
    }

    fn flush_records(&mut self) -> io::Result<()> {
        for record in self.tree.drain_records() {
            if record["message"] == "feedback_conflict" {
                if let Err(error) = self.execution.system_log.write(
                    "ERROR",
                    "coordinator",
                    Some(self.tree.run_id()),
                    &record.to_string(),
                ) {
                    eprintln!("System log write failed: {error}; {record}");
                }
            }
            if let Err(error) = self.execution.ledger.record(record) {
                self.execution.system_log.history_failure(
                    self.tree.run_id(),
                    &format!("record append failed: {error}"),
                );
            }
        }
        Ok(())
    }

    fn publish(&mut self) -> io::Result<()> {
        let _ = self.execution.flush_observations(self.tree);
        self.flush_records()?;
        self.execution
            .presentation
            .update(PresentationUpdate::InputStatus {
                sequence: self.control.input_sequence(),
                ready: self.control.input_ready(self.tree),
                error: self.control.input_error().as_deref(),
            });
        let _ = self.execution.publish(
            self.tree.execution_id(),
            self.tree.authority_ready(),
            self.tree.authority_running(),
            self.control.notice(),
        );
        Ok(())
    }
}

impl Execution<'_> {
    pub fn run(
        &mut self,
        connection: &mut agent::Connection,
        tree: &mut SpeculativeTree,
        policy: &CompiledPolicy,
        engine: &PolicyEngine,
        runtime: &dyn Runtime,
        capabilities: &RuntimeCapabilities,
        channel: (mpsc::Sender<Event>, mpsc::Receiver<Event>),
        run_dir: &std::path::Path,
        resume_model: Option<String>,
        initial_commands: Vec<AgentCommand>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (events, receiver) = channel;
        let sender = events.clone();
        connection.receive(move |message| sender.send(Event::Agent(message)).is_ok())?;
        let mut writer = connection.delivery_writer()?;
        let (deliveries, delivery_queue) =
            mpsc::channel::<(AgentCommand, std::sync::Arc<AtomicBool>)>();
        let mut pending_deliveries: Vec<(String, std::sync::Arc<AtomicBool>)> = Vec::new();
        let mut task = Task {
            tree,
            connection,
            capabilities,
            execution: self,
            started: Instant::now(),
            draining: false,
            delivery_failed: false,
            control: Session::new(run_dir, resume_model),
        };
        let result = thread::scope(|scope| {
            let completed = events.clone();
            scope.spawn(move || {
                while let Ok((command, cancellation)) = delivery_queue.recv() {
                    let result = writer.send_cancellable(command.clone(), &cancellation);
                    if completed.send(Event::Delivered(command, result)).is_err() {
                        break;
                    }
                }
            });
            let mut failure = None;
            let result = (|| -> io::Result<()> {
                task.publish()?;
                let mut outstanding = initial_commands.len();
                queue_deliveries(&deliveries, &mut pending_deliveries, initial_commands)?;
                loop {
                    let event = match if task.tree.recovering() {
                        None
                    } else {
                        task.tree.next_deadline()
                    } {
                        Some(deadline) => match receiver
                            .recv_timeout(Duration::from_millis(deadline.saturating_sub(now_ms())))
                        {
                            Ok(event) => event,
                            Err(mpsc::RecvTimeoutError::Timeout) => Event::Timer,
                            Err(_) => {
                                return Err(io::Error::other("Control event channel disconnected"));
                            }
                        },
                        None => receiver
                            .recv()
                            .map_err(|_| io::Error::other("Control event channel disconnected"))?,
                    };
                    if matches!(&event, Event::Delivered(..)) {
                        outstanding = outstanding.saturating_sub(1);
                    }
                    if matches!(
                        &event,
                        Event::Request(Request::Shutdown | Request::Interrupt)
                    ) {
                        for (_, token) in &pending_deliveries {
                            token.store(true, Ordering::Release);
                        }
                    }
                    let actions = match task.handle(event) {
                        Ok(Some(actions)) => actions,
                        Ok(None) => break,
                        Err(error) if task.delivery_failed && !task.tree.is_stopping() => {
                            failure.get_or_insert(error);
                            ControlActions {
                                commands: task.tree.begin_shutdown()?,
                                ..Default::default()
                            }
                        }
                        Err(error) if !task.tree.is_stopping() && !task.tree.recovering() => {
                            task.delivery_failed = true;
                            task.publish()?;
                            let actions = task.control.fail_and_exit(task.tree, &error)?;
                            failure = Some(error);
                            actions
                        }
                        Err(error) => {
                            task.publish()?;
                            return Err(error);
                        }
                    };
                    for error in runtime.drain_delivery_errors() {
                        task.execution.system_log.write(
                            "ERROR",
                            "runtime",
                            Some(task.tree.run_id()),
                            &error,
                        )?;
                    }
                    for event in runtime.drain_events() {
                        task.record(
                            json!({"source":"runtime","message":"runtime_event","event":event}),
                        )?;
                    }
                    task.publish()?;
                    for command in &actions.commands {
                        if let AgentCommand::Prompt { branch_id, .. } = command {
                            task.execution.ledger.record_durable(json!({"message":"task_started","task_id":task.tree.execution_id(),"branch_id":branch_id}))?;
                        }
                    }
                    outstanding += actions.commands.len();
                    queue_deliveries(&deliveries, &mut pending_deliveries, actions.commands)?;
                    for record in actions.records {
                        task.record(record)?;
                    }
                    for (id, operation) in actions.operations {
                        let tree_state = task.tree.clone();
                        let sender = events.clone();
                        scope.spawn(move || {
                            let result =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    SpeculationCoordinator.perform(
                                        &tree_state,
                                        policy,
                                        engine,
                                        runtime,
                                        capabilities,
                                        &operation,
                                    )
                                }))
                                .unwrap_or_else(|_| Err("Control operation panicked".into()));
                            let _ = sender.send(Event::Completed(id, result));
                        });
                    }
                    let started = task.started;
                    let ledger = &mut task.execution.ledger;
                    let system_log = task.execution.system_log;
                    let run_id = task.tree.run_id().to_string();
                    let task_id = task.tree.execution_id().to_string();
                    let (commands, wake, finish) = task.control.settle(
                        task.tree,
                        runtime,
                        capabilities,
                        run_dir,
                        task.draining,
                        |mut value| {
                            value["t_ms"] = json!(started.elapsed().as_millis() as u64);
                            value["task_id"] = json!(task_id);
                            if let Err(error) = ledger.record(value) {
                                system_log.history_failure(
                                    &run_id,
                                    &format!("record append failed: {error}"),
                                );
                            }
                            Ok(())
                        },
                        |environment| {
                            let sender = events.clone();
                            scope.spawn(move || {
                                let result =
                                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                        runtime_call(
                                            runtime,
                                            &environment,
                                            Operation::MaterializeAuthority,
                                        )
                                        .map_err(|error| error.to_string())
                                    }))
                                    .unwrap_or_else(|_| Err("Materialization panicked".into()));
                                let _ = sender.send(Event::Materialized(result));
                            });
                        },
                    )?;
                    outstanding += commands.len();
                    queue_deliveries(&deliveries, &mut pending_deliveries, commands)?;
                    if wake {
                        let _ = events.send(Event::Timer);
                    }
                    loop {
                        let ids = task.tree.evictable_branches();
                        if ids.is_empty() {
                            break;
                        }
                        for id in ids {
                            let _ = task.execution.ledger.archive_branch(
                                task.tree.take_archivable_branch(&id)?,
                                task.tree.run_id(),
                                task.execution.system_log,
                            );
                        }
                    }
                    if finish && outstanding == 0 {
                        task.connection.finish()?;
                        task.draining = true;
                    }
                    task.publish()?;
                }
                failure.map_or(Ok(()), Err)
            })();
            drop(deliveries);
            for (_, token) in &pending_deliveries {
                token.store(true, Ordering::Release);
            }
            task.connection.stop_receiving();
            result
        });
        let _ = task.execution.flush_observations(task.tree);
        let _ = task
            .execution
            .publish(task.tree.execution_id(), false, false, "Stopped");
        result.map_err(Into::into)
    }
}

fn queue_deliveries(
    sender: &mpsc::Sender<(AgentCommand, std::sync::Arc<AtomicBool>)>,
    pending: &mut Vec<(String, std::sync::Arc<AtomicBool>)>,
    commands: Vec<AgentCommand>,
) -> io::Result<()> {
    pending.retain(|(_, token)| std::sync::Arc::strong_count(token) > 1);
    for command in commands {
        let branch = serde_json::to_value(&command)?["branch_id"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        if matches!(
            &command,
            AgentCommand::Abort { .. }
                | AgentCommand::Discard { .. }
                | AgentCommand::Cleanup { .. }
        ) {
            for (id, token) in pending.iter() {
                if id == &branch {
                    token.store(true, Ordering::Release);
                }
            }
        }
        let token = std::sync::Arc::new(AtomicBool::new(false));
        if matches!(
            &command,
            AgentCommand::StartCandidate { .. }
                | AgentCommand::StartBranch { .. }
                | AgentCommand::Prompt { .. }
                | AgentCommand::ResolvePending { .. }
                | AgentCommand::Continue { .. }
        ) {
            pending.push((branch, token.clone()));
        }
        sender
            .send((command, token))
            .map_err(|_| io::Error::other("Agent delivery worker stopped"))?;
    }
    Ok(())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waiting_delivery_does_not_block_cancel_or_disable_cleanup() {
        let (sender, queue) = mpsc::channel();
        let mut pending = Vec::new();
        queue_deliveries(
            &sender,
            &mut pending,
            vec![AgentCommand::Prompt {
                branch_id: "b".into(),
                prompt: "go".into(),
            }],
        )
        .unwrap();
        let (_, startup) = queue.recv().unwrap();
        // The delivery worker is still waiting. The control path can enqueue
        // cancellation and cleanup without waiting for any reception receipt.
        queue_deliveries(
            &sender,
            &mut pending,
            vec![
                AgentCommand::Abort {
                    branch_id: "b".into(),
                    command_id: "abort".into(),
                    generation: 0,
                },
                AgentCommand::Cleanup {
                    branch_id: "b".into(),
                    workspace: "/tmp/fake".into(),
                    command_id: "cleanup".into(),
                    generation: 0,
                },
            ],
        )
        .unwrap();
        assert!(startup.load(Ordering::Acquire));
        let (_, abort) = queue.recv().unwrap();
        let (_, cleanup) = queue.recv().unwrap();
        assert!(!abort.load(Ordering::Acquire));
        assert!(!cleanup.load(Ordering::Acquire));
    }
}
