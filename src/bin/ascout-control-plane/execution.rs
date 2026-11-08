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
    ledger: Ledger,
    details: BTreeMap<String, String>,
    notice: String,
    started: Instant,
    draining: bool,
}

impl Task<'_> {
    fn record(&mut self, mut value: Value) -> io::Result<()> {
        value["t_ms"] = json!(self.started.elapsed().as_millis() as u64);
        self.ledger.record(value.clone())?;
        if value["message"] == "candidate_created" {
            self.interface.fork(
                value["parent"].as_str().unwrap(),
                value["branch_id"].as_str().unwrap(),
            );
        }
        self.interface.record(value);
        Ok(())
    }

    fn handle(&mut self, event: Event) -> io::Result<Option<ControlActions>> {
        let mut actions = match event {
            Event::Request(ui::Request::Shutdown) => ControlActions {
                commands: self.tree.begin_shutdown()?,
                ..Default::default()
            },
            Event::Request(ui::Request::Interrupt) => ControlActions {
                commands: self.tree.interrupt()?,
                ..Default::default()
            },
            Event::Request(ui::Request::Prompt { prompt }) => {
                self.tree.queue_prompt(prompt);
                ControlActions::default()
            }
            Event::Completed(id, result) => self.tree.complete_work(id, result)?,
            Event::Timer => {
                self.tree.check_command_deadlines()?;
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
                let commands =
                    if matches!(message, AgentMessage::SpeculativeAdmissionRequest { .. }) {
                        Vec::new()
                    } else {
                        self.tree.handle_message(message.clone())?
                    };
                self.interface.message(&message);
                if !matches!(message, AgentMessage::Output { .. }) {
                    let mut fact = serde_json::to_value(&message)?;
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

    fn publish(&self) {
        self.interface.publish(
            self.tree,
            &self.details,
            self.tree.authority_ready(),
            self.tree.authority_running(),
            &self.notice,
        );
    }
}

pub fn run(
    stream: &mut UnixStream,
    tree: &mut SpeculativeTree,
    policy: &CompiledPolicy,
    runtime: &ControlSession,
    capabilities: &RuntimeCapabilities,
    interface: &ui::Interface,
    channel: (mpsc::Sender<Event>, mpsc::Receiver<Event>),
) -> Result<(), Box<dyn std::error::Error>> {
    let (events, receiver) = channel;
    let ledger = Ledger::open(&tree.workspace_root.join("events.jsonl"))?;
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
        details: BTreeMap::new(),
        notice: "Starting Agent session…".into(),
        started: Instant::now(),
        draining: false,
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
        task.publish();
        let mut failure = None;
        let result = (|| -> io::Result<()> {
            loop {
                let event = match task.tree.next_deadline() {
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
                    Err(error) if !task.tree.is_stopping() => {
                        failure = Some(error);
                        ControlActions {
                            commands: task.tree.begin_shutdown()?,
                            ..Default::default()
                        }
                    }
                    Err(error) => return Err(error),
                };
                for event in runtime.drain_events() {
                    task.record(
                        json!({"source":"runtime","message":"runtime_event","event":event}),
                    )?;
                }
                for command in actions.commands {
                    send(task.stream, command)?;
                }
                for record in actions.records {
                    task.record(record)?;
                }
                for (id, operation) in actions.operations {
                    let snapshot = task.tree.clone();
                    let sender = events.clone();
                    scope.spawn(move || {
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            AgentController.perform(
                                &snapshot,
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
                task.notice = if task.tree.is_stopping() {
                    "Stopping"
                } else if task.tree.authority_running() {
                    "Agent running"
                } else {
                    "Ready"
                }
                .into();
                task.publish();
            }
            failure.map_or(Ok(()), Err)
        })();
        SIGNAL_FD.store(-1, Ordering::Relaxed);
        let _ = signal_writer.write(&[0]);
        let _ = close_reader.shutdown(std::net::Shutdown::Read);
        result
    });
    interface.publish(task.tree, &task.details, false, false, "Stopped");
    let unarchived: Vec<_> = task.ledger.unarchived().collect();
    if !unarchived.is_empty() {
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
