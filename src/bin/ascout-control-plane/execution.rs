use super::*;
use ascout::control_plane::agent_control::SpeculationCoordinator;
use ascout::control_plane::coordination::session::Session;
use ascout::control_plane::coordination::{ControlActions, ControlResult};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::Ordering;

pub enum Event {
    Interaction(ui::Request, mpsc::Sender<Result<(), String>>),
    Agent(io::Result<Option<AgentMessage>>),
    Request(ui::Request),
    Completed(u64, Result<ControlResult, String>),
    Materialized(Result<ascout::protocol::contract::OperationResult, String>),
    Timer,
}

struct Task<'a> {
    tree: &'a mut SpeculativeTree,
    connection: &'a mut agent::Connection,
    interface: &'a ui::Interface,
    capabilities: &'a RuntimeCapabilities,
    ledger: &'a mut Ledger,
    started: Instant,
    draining: bool,
    control: Session,
}

fn forward_signals(
    mut reader: UnixStream,
    shutdown: &AtomicBool,
    interrupt: &AtomicBool,
    sender: mpsc::Sender<Event>,
) {
    // The pipe is only a wake hint. Check pending intent periodically too:
    // a full pipe or a late flag must never strand the main loop in recv().
    let mut byte = [0];
    loop {
        if shutdown.swap(false, Ordering::AcqRel) {
            let _ = sender.send(Event::Request(ui::Request::Shutdown));
        }
        if interrupt.swap(false, Ordering::AcqRel) {
            let _ = sender.send(Event::Request(ui::Request::Interrupt));
        }
        match reader.read(&mut byte) {
            Ok(0) => break,
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::Interrupted
                ) => {}
            Err(_) => break,
        }
    }
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
            Event::Timer => {
                self.control.timer(self.tree)?;
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
                let actions = self.control.agent(self.tree, &message)?;
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
                actions
            }
        };
        actions = self
            .control
            .progress(self.tree, self.capabilities, actions)?;
        Ok(Some(actions))
    }

    fn flush_observations(&mut self) -> io::Result<()> {
        let mut failure = None;
        for content in self.tree.drain_observations() {
            if let Err(error) = self
                .ledger
                .append_observation(self.tree.execution_id(), content)
            {
                failure.get_or_insert(error);
            }
        }
        failure.map_or(Ok(()), Err)
    }

    fn flush_records(&mut self) -> io::Result<()> {
        for record in self.tree.drain_records() {
            self.ledger.record(record)?;
        }
        Ok(())
    }

    fn publish(&mut self) -> io::Result<()> {
        self.flush_observations()?;
        self.flush_records()?;
        self.interface.publish_input_status(
            self.control.input_sequence(),
            self.control.input_ready(self.tree),
            self.control.input_error(),
        );
        self.interface.publish(
            self.ledger,
            self.tree.authority_ready(),
            self.tree.authority_running(),
            self.control.notice(),
        )
    }
}

pub fn run(
    connection: &mut agent::Connection,
    tree: &mut SpeculativeTree,
    policy: &CompiledPolicy,
    engine: &PolicyEngine,
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
    let (signal_reader, mut signal_writer) = UnixStream::pair()?;
    signal_reader.set_read_timeout(Some(Duration::from_millis(20)))?;
    signal_writer.set_nonblocking(true)?;
    use std::os::fd::AsRawFd;
    SIGNAL_FD.store(signal_writer.as_raw_fd(), Ordering::Relaxed);
    let mut task = Task {
        tree,
        connection,
        interface,
        capabilities,
        ledger,
        started: Instant::now(),
        draining: false,
        control: Session::new(run_dir, resume_model),
    };
    let result = thread::scope(|scope| {
        let sender = events.clone();
        scope.spawn(move || {
            forward_signals(
                signal_reader,
                &SHUTDOWN_REQUESTED,
                &INTERRUPT_REQUESTED,
                sender,
            );
        });
        if SHUTDOWN_REQUESTED.load(Ordering::Relaxed) || INTERRUPT_REQUESTED.load(Ordering::Relaxed)
        {
            let _ = signal_writer.write(&[1]);
        }
        let mut failure = None;
        let result = (|| -> io::Result<()> {
            task.publish()?;
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
                let actions = match task.handle(event) {
                    Ok(Some(actions)) => actions,
                    Ok(None) => break,
                    Err(error) if !task.tree.is_stopping() && !task.tree.recovering() => {
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
                let ledger = &mut task.ledger;
                let (commands, wake, finish) = task.control.settle(
                    task.tree,
                    runtime,
                    capabilities,
                    run_dir,
                    task.draining,
                    |mut value| {
                        value["t_ms"] = json!(started.elapsed().as_millis() as u64);
                        ledger.record(value)
                    },
                    |environment| {
                        let sender = events.clone();
                        scope.spawn(move || {
                            let result =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    ascout::control_plane::coordination::execution::runtime_call(
                                        runtime,
                                        &environment,
                                        ascout::protocol::contract::Operation::MaterializeAuthority,
                                    )
                                    .map_err(|error| error.to_string())
                                }))
                                .unwrap_or_else(|_| Err("Materialization panicked".into()));
                            let _ = sender.send(Event::Materialized(result));
                        });
                    },
                )?;
                for command in commands {
                    task.connection.send(command)?;
                }
                if wake {
                    let _ = events.send(Event::Timer);
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
                if finish {
                    task.connection.finish()?;
                    task.draining = true;
                }
                task.publish()?;
            }
            failure.map_or(Ok(()), Err)
        })();
        SIGNAL_FD.store(-1, Ordering::Relaxed);
        // EOF cannot be lost when the nonblocking wake socket is full.
        let _ = signal_writer.shutdown(std::net::Shutdown::Write);
        task.connection.stop_receiving();
        result
    });
    task.flush_observations()?;
    interface.publish(task.ledger, false, false, "Stopped")?;
    let unarchived: Vec<_> = task.ledger.unarchived().collect();
    if !unarchived.is_empty() && !task.control.saved() {
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

#[cfg(test)]
mod signal_tests {
    use super::*;

    #[test]
    fn late_and_repeated_signals_do_not_need_another_pipe_byte() {
        let (mut reader, writer) = UnixStream::pair().unwrap();
        reader
            .set_read_timeout(Some(Duration::from_millis(20)))
            .unwrap();
        let shutdown = AtomicBool::new(false);
        let interrupt = AtomicBool::new(false);
        let (sender, receiver) = mpsc::channel();
        // Force the wake byte to be consumed before the pending flag is visible.
        (&writer).write_all(&[1]).unwrap();
        reader.read_exact(&mut [0]).unwrap();
        thread::scope(|scope| {
            scope.spawn(|| forward_signals(reader, &shutdown, &interrupt, sender));
            let result = || {
                for _ in 0..3 {
                    interrupt.store(true, Ordering::Release);
                    assert!(matches!(
                        receiver.recv_timeout(Duration::from_millis(500)),
                        Ok(Event::Request(ui::Request::Interrupt))
                    ));
                }
                shutdown.store(true, Ordering::Release);
                assert!(matches!(
                    receiver.recv_timeout(Duration::from_millis(500)),
                    Ok(Event::Request(ui::Request::Shutdown))
                ));
            };
            // Close even when an assertion fails, so the scoped worker cannot hang.
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(result));
            writer.shutdown(std::net::Shutdown::Write).unwrap();
            if let Err(error) = outcome {
                std::panic::resume_unwind(error);
            }
        });
    }
}
