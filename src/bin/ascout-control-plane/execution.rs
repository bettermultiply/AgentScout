use super::*;
use ascout::control_plane::agent_control::AgentController;
use ascout::control_plane::coordination::{ControlActions, ControlResult};
use std::io::{Read, Write};
use std::sync::atomic::Ordering;

pub enum Event {
    Agent(io::Result<Option<AgentWireMessage>>),
    Request(ui::Request),
    Completed(u64, Result<ControlResult, String>),
    Timer,
}

struct Task<'a> {
    tree: &'a mut SpeculativeTree,
    stream: &'a mut UnixStream,
    interface: &'a ui::Interface,
    capabilities: &'a RuntimeCapabilities,
    ledger: &'a mut Ledger,
    notice: String,
    started: Instant,
    draining: bool,
    recovery_path: std::path::PathBuf,
    saved: bool,
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
            Event::Request(ui::Request::Shutdown) => {
                if self.tree.recovering {return Ok(Some(ControlActions::default()));}
                if self.tree.pending_work() || self.tree.recovery_pending_commands() {
                    self.notice="Save failed; a control operation is still pending; execution retained".into();
                    return Ok(Some(ControlActions::default()));
                }
                let directory=self.recovery_path.parent().unwrap().join(format!("recovery-{}",std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
                std::fs::create_dir(&directory)?;
                self.recovery_path=directory.join("agent.json");
                self.tree.recovering = true;
                ControlActions {
                    commands: vec![AgentCommand::Save {branch_id:self.tree.authority_branch_id.clone().unwrap_or_default(),path:self.recovery_path.clone()}],
                    ..Default::default()
                }
            },
            Event::Request(ui::Request::Interrupt) => ControlActions {
                commands: if self.tree.recovering {vec![]} else {self.tree.interrupt()?},
                ..Default::default()
            },
            Event::Request(ui::Request::Prompt { prompt }) => {
                if self.tree.recovering {return Ok(Some(ControlActions::default()));}
                self.tree.queue_prompt(prompt);
                ControlActions::default()
            }
            Event::Completed(id, result) => self.tree.complete_work(id, result)?,
            Event::Timer => {
                if !self.tree.recovering {self.tree.check_command_deadlines()?;}
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
                let AgentWireMessage::Message(message) = message else {
                    return Err(io::Error::other("Agent sent a control-plane command"));
                };
                if let AgentMessage::RecoverySaved { path, error } = &message {
                    if path != &self.recovery_path {return Err(io::Error::other("Recovery path mismatch"));}
                    if let Some(error) = error {
                        self.tree.recovering = false;
                        self.saved = false;
                        self.notice = format!("Save failed; still running: {error}");
                    } else {self.saved = true;}
                }
                if let AgentMessage::RecoveryLoaded {error:Some(error)} = &message {
                    return Err(io::Error::other(format!("Recovery load failed: {error}")));
                }
                if let AgentMessage::RecoveryLoaded {error:None} = &message {
                    self.tree.recovering=false;
                }
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
            return Err(io::Error::other(reason.to_string()));
        }
        let progress = self.tree.progress(self.capabilities)?;
        actions.commands.extend(progress.commands);
        actions.operations.extend(progress.operations);
        actions.records.extend(progress.records);
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
        self.interface.publish(
            self.ledger,
            self.tree.authority_ready(),
            self.tree.authority_running(),
            &self.notice,
        )
    }
}

pub fn run(
    stream: &mut UnixStream,
    tree: &mut SpeculativeTree,
    policy: &CompiledPolicy,
    runtime: &ControlSession,
    capabilities: &RuntimeCapabilities,
    interface: &ui::Interface,
    ledger: &mut Ledger,
    channel: (mpsc::Sender<Event>, mpsc::Receiver<Event>),
    run_dir: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let (events, receiver) = channel;
    let mut reader = stream.try_clone()?;
    let close_reader = stream.try_clone()?;
    let (mut signal_reader, mut signal_writer) = UnixStream::pair()?;
    signal_writer.set_nonblocking(true)?;
    use std::os::fd::AsRawFd;
    SIGNAL_FD.store(signal_writer.as_raw_fd(), Ordering::Relaxed);
    let mut task = Task {
        tree,
        stream,
        interface,
        capabilities,
        ledger,
        notice: "Starting Agent session…".into(),
        started: Instant::now(),
        draining: false,
        recovery_path: run_dir.join("agent-recovery.json"),
        saved: false,
    };
    let result = thread::scope(|scope| {
        let sender = events.clone();
        scope.spawn(move || {
            loop {
                let message = transport::read_message(&mut reader);
                let done = !matches!(message, Ok(Some(_)));
                if sender.send(Event::Agent(message)).is_err() || done {
                    break;
                }
            }
        });
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
                let event = match if task.tree.recovering {None} else {task.tree.next_deadline()} {
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
                    Err(error) if !task.tree.is_stopping() && !task.tree.recovering => {
                        task.publish()?;
                        failure = Some(error);
                        ControlActions {
                            commands: task.tree.begin_shutdown()?,
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
                    send(task.stream, command)?;
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
                if task.saved && !task.draining && !task.tree.pending_work() {
                    let save_result=(||->io::Result<()> {
                        let generation=task.recovery_path.parent().unwrap();
                        runtime_call(runtime,"authority",Operation::SaveRuntime {path:generation.join("runtime.json")})
                            .map_err(|error|io::Error::other(error.to_string()))?;
                        recovery::publish(run_dir,generation,task.tree)?;
                        Ok(())
                    })();
                    if let Err(error)=save_result {
                        task.notice=format!("Save failed; execution retained: {error}");
                        task.saved=false;
                        send(task.stream,AgentCommand::CancelSave {branch_id:task.tree.authority_branch_id.clone().unwrap_or_default()})?;
                        task.tree.recovering=false;
                    } else {
                        task.stream.shutdown(std::net::Shutdown::Write)?;
                        task.draining=true;
                    }
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
                if task.tree.is_stopping()
                    && !task.draining
                    && !task.tree.cleanup_pending()?
                    && !task.tree.pending_work()
                {
                    task.stream.shutdown(std::net::Shutdown::Write)?;
                    task.draining = true;
                }
                if !task.notice.starts_with("Save failed") {task.notice = if task.tree.is_stopping() {
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
        let _ = close_reader.shutdown(std::net::Shutdown::Read);
        result
    });
    task.flush_observations()?;
    interface.publish(task.ledger, false, false, "Stopped")?;
    let unarchived: Vec<_> = task.ledger.unarchived().collect();
    if !unarchived.is_empty() && !task.saved {
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
