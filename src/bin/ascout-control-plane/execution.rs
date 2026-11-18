use super::*;
use ascout::control_plane::agent_control::{AgentCommand, SpeculationCoordinator};
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
    Delivered(AgentCommand, io::Result<()>),
    Timer,
}

struct Task<'a> {
    tree: &'a mut SpeculativeTree,
    connection: &'a mut agent::Connection,
    interface: &'a ui::Interface,
    capabilities: &'a RuntimeCapabilities,
    ledger: &'a mut Ledger,
    system_log: &'a ascout::control_plane::system_log::SystemLog,
    started: Instant,
    draining: bool,
    delivery_failed: bool,
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
        if let Err(error) = self.ledger.record(value.clone()) {
            self.system_log.history_failure(
                self.tree.execution_id(),
                &format!("record append failed: {error}"),
            );
        }
        if value["message"] == "candidate_created" {
            if let Err(error) = self.interface.fork(
                self.ledger,
                value["parent"].as_str().unwrap(),
                value["branch_id"].as_str().unwrap(),
            ) {
                self.system_log.history_failure(
                    self.tree.execution_id(),
                    &format!("observation append failed: {error}"),
                );
            }
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
                self.flush_observations()?;
                if let Err(error) = self.interface.message(self.ledger, &message) {
                    self.system_log.history_failure(
                        self.tree.execution_id(),
                        &format!("observation append failed: {error}"),
                    );
                }
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
                self.system_log.history_failure(
                    self.tree.execution_id(),
                    &format!("observation append failed: {error}"),
                );
                failure.get_or_insert(error);
            }
        }
        let _ = failure;
        Ok(())
    }

    fn flush_records(&mut self) -> io::Result<()> {
        for record in self.tree.drain_records() {
            if record["message"] == "feedback_conflict" {
                if let Err(error) = self.system_log.write(
                    "ERROR",
                    "coordinator",
                    Some(self.tree.execution_id()),
                    &record.to_string(),
                ) {
                    eprintln!("System log write failed: {error}; {record}");
                }
            }
            if let Err(error) = self.ledger.record(record) {
                self.system_log.history_failure(
                    self.tree.execution_id(),
                    &format!("record append failed: {error}"),
                );
            }
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
        let published = self.interface.publish(
            self.ledger,
            self.tree.authority_ready(),
            self.tree.authority_running(),
            self.control.notice(),
        );
        if let Err(error) = published {
            self.system_log.history_failure(
                self.tree.execution_id(),
                &format!("observation append failed: {error}"),
            );
        }
        Ok(())
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
    system_log: &ascout::control_plane::system_log::SystemLog,
    channel: (mpsc::Sender<Event>, mpsc::Receiver<Event>),
    run_dir: &std::path::Path,
    resume_model: Option<String>,
    initial_commands: Vec<AgentCommand>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (events, receiver) = channel;
    let sender = events.clone();
    connection.receive(move |message| sender.send(Event::Agent(message)).is_ok())?;
    let (signal_reader, mut signal_writer) = UnixStream::pair()?;
    signal_reader.set_read_timeout(Some(Duration::from_millis(20)))?;
    signal_writer.set_nonblocking(true)?;
    use std::os::fd::AsRawFd;
    SIGNAL_FD.store(signal_writer.as_raw_fd(), Ordering::Relaxed);
    let mut writer = connection.delivery_writer()?;
    let (deliveries, delivery_queue) =
        mpsc::channel::<(AgentCommand, std::sync::Arc<AtomicBool>)>();
    let mut pending_deliveries: Vec<(String, std::sync::Arc<AtomicBool>)> = Vec::new();
    let mut task = Task {
        tree,
        connection,
        interface,
        capabilities,
        ledger,
        system_log,
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
                    Event::Request(ui::Request::Shutdown | ui::Request::Interrupt)
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
                    task.system_log.write(
                        "ERROR",
                        "runtime",
                        Some(task.tree.execution_id()),
                        &error,
                    )?;
                }
                for event in runtime.drain_events() {
                    task.record(
                        json!({"source":"runtime","message":"runtime_event","event":event}),
                    )?;
                }
                task.publish()?;
                outstanding += actions.commands.len();
                queue_deliveries(&deliveries, &mut pending_deliveries, actions.commands)?;
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
                let run_id = task.tree.execution_id().to_string();
                let (commands, wake, finish) = task.control.settle(
                    task.tree,
                    runtime,
                    capabilities,
                    run_dir,
                    task.draining,
                    |mut value| {
                        value["t_ms"] = json!(started.elapsed().as_millis() as u64);
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
                        let _ = task.ledger.archive_branch(
                            task.tree.take_archivable_branch(&id)?,
                            task.tree.execution_id(),
                            task.system_log,
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
        SIGNAL_FD.store(-1, Ordering::Relaxed);
        // EOF cannot be lost when the nonblocking wake socket is full.
        let _ = signal_writer.shutdown(std::net::Shutdown::Write);
        drop(deliveries);
        for (_, token) in &pending_deliveries {
            token.store(true, Ordering::Release);
        }
        task.connection.stop_receiving();
        result
    });
    task.flush_observations()?;
    if let Err(error) = interface.publish(task.ledger, false, false, "Stopped") {
        system_log.history_failure(
            task.tree.execution_id(),
            &format!("observation append failed: {error}"),
        );
    }
    result.map_err(Into::into)
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
mod signal_tests {
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
