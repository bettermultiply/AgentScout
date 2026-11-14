use super::*;
use ascout::control_plane::agent_control::AgentController;
use ascout::control_plane::coordination::{ControlActions, ControlResult};
use ascout::control_plane::coordination::{recovery::Recovery, task};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::Ordering;

pub enum Event {
    Agent(io::Result<Option<AgentMessage>>),
    Request(ui::Request),
    Completed(u64, Result<ControlResult, String>),
    Timer,
}

struct Task<'a> {
    tree: &'a mut SpeculativeTree,
    connection: &'a mut agent::Connection,
    interface: &'a ui::Interface,
    capabilities: &'a RuntimeCapabilities,
    ledger: &'a mut Ledger,
    notice: String,
    started: Instant,
    draining: bool,
    recovery: Recovery,
    pending_model: Option<String>,
    model_changing: bool,
    input_sequence: u64,
    input_error: Option<String>,
    lifecycle: task::Task,
}

impl Task<'_> {
    fn record(&mut self, mut value: Value) -> io::Result<()> {
        value["t_ms"] = json!(self.started.elapsed().as_millis() as u64);
        self.ledger.record(value.clone())?;
        if value["message"] == "candidate_created" {
            self.interface.fork(
                self.ledger,
                value["parent"].as_str().unwrap(),
                value["branch_id"].as_str().unwrap(),
            )?;
        }
        Ok(())
    }

    fn handle(&mut self, event: Event) -> io::Result<Option<ControlActions>> {
        let mut actions = match event {
            Event::Request(ui::Request::Pause) => {
                ControlActions {
                    commands: self.recovery.pause(self.tree, now_ms())?,
                    ..Default::default()
                }
            },
            Event::Request(ui::Request::Shutdown) | Event::Request(ui::Request::Interrupt) => {
                self.notice = "Task cancellation requested".into();
                ControlActions {
                    commands: self.lifecycle.cancel(self.tree, matches!(event, Event::Request(ui::Request::Shutdown)))?,
                    ..Default::default()
                }
            },
            Event::Request(ui::Request::Model { model }) => {
                if self.tree.recovering() || self.tree.authority_running() || self.tree.pending_work() || self.tree.recovery_pending_commands() || self.model_changing {
                    self.notice="Model change failed: wait for an idle authority".into();
                } else {self.pending_model=Some(model);}
                ControlActions::default()
            }
            Event::Request(ui::Request::Prompt { prompt }) => {
                if prompt.trim() == "/pause" { return self.handle(Event::Request(ui::Request::Pause)); }
                if !self.recovery.continue_execution(self.tree, now_ms()) { return Ok(Some(ControlActions::default())); }
                if self.tree.task_finishing() {
                    self.lifecycle.retry_materialization();
                    self.notice = "Finishing previous task; input deferred".into();
                    self.tree.queue_prompt(prompt);
                } else {
                    self.tree.queue_prompt(prompt);
                }
                self.input_sequence += 1;
                self.input_error = None;
                self.notice = "Agent running".into();
                ControlActions::default()
            }
            Event::Completed(id, result) => self.tree.complete_work(id, result)?,
            Event::Timer => {
                if !self.tree.recovering() {self.tree.check_command_deadlines()?;}
                ControlActions::default()
            }
            Event::Agent(message) => {
                let Some(message) = message? else {
                    return if self.draining {
                        Ok(None)
                    } else {
                        Err(io::Error::other(
                            "Agent disconnected; cleanup is not confirmed",
                        ))
                    };
                };
                match &message {
                    AgentMessage::TurnFailed {branch_id,reason,..} => {
                        if self.tree.authority_branch_id.as_ref() == Some(branch_id) {
                            self.input_error=Some(reason.clone());
                             self.lifecycle.fail(self.tree);
                        }
                        self.notice=format!("Turn failed; session retained: {reason}");
                    }
                    AgentMessage::ModelChanged {model,error,..} => {self.model_changing=false;self.notice=match error {Some(error)=>format!("Model change failed: {error}"),None=>format!("Model changed: {model}")};}
                    _=>{}
                }
                if let Some(notice) = self.recovery.handle(self.tree, &message)? { self.notice = notice; }
                let commands =
                    if matches!(message, AgentMessage::SpeculativeAdmissionRequest { .. }) {
                        Vec::new()
                    } else {
                        self.tree.handle_message(message.clone())?
                    };
                self.flush_observations()?;
                self.interface.message(self.ledger, &message)?;
                if !matches!(message, AgentMessage::Output { .. }) {
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
                let mut actions =
                    if matches!(message, AgentMessage::SpeculativeAdmissionRequest { .. }) {
                        self.tree.speculative_admission(message)?
                    } else {
                        ControlActions::default()
                    };
                actions.commands.extend(commands);
                actions
            }
        };
        if let Some(reason) = self.tree.authority_failure()
            && !self.tree.is_stopping()
        {
            self.input_error=Some(reason.to_string());
            self.lifecycle.fail(self.tree);
        }
        let progress = if !self.tree.recovering() && (self.model_changing || self.pending_model.is_some()) {ControlActions::default()} else {self.tree.progress(self.capabilities)?};
        actions.commands.extend(progress.commands);
        actions.operations.extend(progress.operations);
        actions.records.extend(progress.records);
        actions.commands.extend(self.lifecycle.progress(self.tree)?);
        // Do not change a model beneath in-flight work or a pending Promote.
        // Descendants retain their own model; newly forked sessions inherit
        // the model from their actual parent, never a global replacement.
        let idle=(!self.tree.recovering()||self.recovery.loaded)&&!self.model_changing&&self.tree.authority_ready()&&!self.tree.authority_running()&&!self.tree.pending_work()&&!self.tree.recovery_pending_commands();
        if idle {
            if let Some(model)=self.pending_model.take() {
                self.model_changing=true;
                actions.commands.push(AgentCommand::Model {branch_id:self.tree.authority_branch_id.clone().unwrap(),model});
            }
        }
        Ok(Some(actions))
    }

    fn flush_observations(&mut self) -> io::Result<()> {
        let mut failure = None;
        for content in self.tree.drain_observations() {
            if let Err(error) = self
                .ledger
                .append_observation(&self.tree.execution_id, content)
            {
                failure.get_or_insert(error);
            }
        }
        failure.map_or(Ok(()), Err)
    }

    fn publish(&mut self) -> io::Result<()> {
        self.flush_observations()?;
        self.interface.publish_input_status(self.input_sequence,
            self.tree.input_ready() && !self.model_changing && self.pending_model.is_none(), &self.input_error);
        self.interface.publish(
            self.ledger,
            self.tree.authority_ready(),
            self.tree.authority_running(),
            &self.notice,
        )
    }
}

pub fn run(
    connection: &mut agent::Connection,
    tree: &mut SpeculativeTree,
    policy: &CompiledPolicy,
    runtime: &dyn Runtime,
    capabilities: &RuntimeCapabilities,
    interface: &ui::Interface,
    ledger: &mut Ledger,
    channel: (mpsc::Sender<Event>, mpsc::Receiver<Event>),
    run_dir: &std::path::Path,
    resume_model: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (events, receiver) = channel;
    let sender = events.clone();
    connection.receive(move |message| sender.send(Event::Agent(message)).is_ok())?;
    let (mut signal_reader, mut signal_writer) = UnixStream::pair()?;
    signal_writer.set_nonblocking(true)?;
    use std::os::fd::AsRawFd;
    SIGNAL_FD.store(signal_writer.as_raw_fd(), Ordering::Relaxed);
    let mut task = Task {
        tree,
        connection,
        interface,
        capabilities,
        ledger,
        notice: "Starting Agent session…".into(),
        started: Instant::now(),
        draining: false,
        recovery: Recovery::new(run_dir),
        pending_model:resume_model,
        model_changing:false,
        input_sequence: 0,
        input_error: None,
        lifecycle: task::Task::default(),
    };
    let result = thread::scope(|scope| {
        let sender = events.clone();
        scope.spawn(move || {
            let mut byte = [0];
            if signal_reader.read(&mut byte).is_ok_and(|count| count > 0) {
                let _ = sender.send(Event::Request(ui::Request::Shutdown));
            }
        });
        if SHUTDOWN_REQUESTED.load(Ordering::Relaxed) {
            let _ = events.send(Event::Request(ui::Request::Shutdown));
        }
        let mut failure = None;
        let result = (|| -> io::Result<()> {
            task.publish()?;
            loop {
                let event = match if task.tree.recovering() {None} else {task.tree.next_deadline()} {
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
                let actions = match task.handle(event) {
                    Ok(Some(actions)) => actions,
                    Ok(None) => break,
                    Err(error) if !task.tree.is_stopping() && !task.tree.recovering() => {
                        task.publish()?;
                        task.input_error=Some(error.to_string());
                        failure = Some(error);
                        ControlActions {
                            commands: task.lifecycle.fail_and_exit(task.tree)?,
                            ..Default::default()
                        }
                    }
                    Err(error) => {
                        task.publish()?;
                        return Err(error);
                    }
                };
                for event in runtime.drain_events() {
                    task.record(
                        json!({"source":"runtime","message":"runtime_event","event":event}),
                    )?;
                }
                task.publish()?;
                for command in actions.commands {
                    task.connection.send(command)?;
                }
                for record in actions.records {
                    task.record(record)?;
                }
                for (id, operation) in actions.operations {
                    let tree_state = task.tree.clone();
                    let sender = events.clone();
                    scope.spawn(move || {
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            AgentController.perform(
                                &tree_state,
                                policy,
                                runtime,
                                capabilities,
                                &operation,
                            )
                        }))
                        .unwrap_or_else(|_| Err("Control operation panicked".into()));
                        let _ = sender.send(Event::Completed(id, result));
                    });
                }
                if let Some(command) = task.recovery.save_command(task.tree) {
                    task.connection.send(command)?;
                }
                if let Some((command, notice)) = task.recovery.publish(task.tree, runtime, run_dir, task.draining) {
                    task.connection.send(command)?;
                    task.notice = notice;
                }
                let started = task.started;
                let ledger = &mut task.ledger;
                let terminating = task.lifecycle.terminating;
                if let Some((commands, wake, result)) = task.lifecycle.materialize(
                    task.tree, runtime, capabilities, task.input_error.is_some(), |mut value| {
                        value["t_ms"] = json!(started.elapsed().as_millis() as u64);
                        ledger.record(value)
                    })? {
                    match result {
                        Ok(()) => task.notice = if task.input_error.is_some() || terminating {"Task ended"} else {"Task succeeded"}.into(),
                        Err(error) => {
                            task.input_error = Some(format!("Materialization failed: {error}"));
                            task.notice = format!("Materialization failed; retained for retry: {error}");
                        }
                    }
                    for command in commands { task.connection.send(command)?; }
                    if wake { let _ = events.send(Event::Timer); }
                }
                loop {
                    let ids = task.tree.evictable_branches();
                    if ids.is_empty() {
                        break;
                    }
                    for id in ids {
                        task.ledger
                            .archive_branch(task.tree.take_archivable_branch(&id)?)?;
                    }
                }
                if !task.draining && task.lifecycle.finish_ready(task.tree)? {
                    task.connection.finish()?;
                    task.draining = true;
                }
                if !task.tree.task_finishing() && !task.tree.recovering() && !task.notice.starts_with("Save failed") && !task.notice.starts_with("Turn failed") && !task.notice.starts_with("Model ") {task.notice = if task.tree.is_stopping() {
                    "Stopping"
                } else if task.tree.authority_running() {
                    "Agent running"
                } else {
                    "Ready"
                }
                .into();}
                task.publish()?;
            }
            failure.map_or(Ok(()), Err)
        })();
        SIGNAL_FD.store(-1, Ordering::Relaxed);
        let _ = signal_writer.write(&[0]);
        task.connection.stop_receiving();
        result
    });
    task.flush_observations()?;
    interface.publish(task.ledger, false, false, "Stopped")?;
    let unarchived: Vec<_> = task.ledger.unarchived().collect();
    if !unarchived.is_empty() && !task.recovery.saved {
        return Err(format!(
            "{}; Ledger history incomplete: {}",
            result
                .as_ref()
                .err()
                .map(ToString::to_string)
                .unwrap_or_default(),
            unarchived.join(", ")
        )
        .into());
    }
    result.map_err(Into::into)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
