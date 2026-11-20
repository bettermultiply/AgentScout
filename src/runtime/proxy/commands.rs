//! Instrumentation command connection, separate from control-plane operations.
use super::*;
use crate::protocol::contract::{EnvironmentState, ExecutionAccess};
use crate::runtime::backend::{BackendContext, BackendError, CancelHandle};
use crate::runtime::execution_wire::{self as wire, Body, Message};
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::{BufReader, Seek, SeekFrom, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
enum Request {
    CancelEnvironment {
        operation_id: String,
        environment_id: String,
    },
    Execute {
        #[serde(default)]
        generation: u64,
        request_id: String,
        operation_id: String,
        environment_id: String,
        #[serde(default)]
        tool: String,
        #[serde(default)]
        operation: Option<String>,
        arguments: Value,
        #[serde(default)]
        context: Value,
    },
    Cancel {
        operation_id: String,
        environment_id: String,
    },
    Confirm {
        operation_id: String,
        environment_id: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fake;
    impl Backend for Fake {
        fn environments(&self) -> Vec<String> {
            vec!["environment".into()]
        }
        fn establish(&self, _: &str) -> Result<(), crate::runtime::backend::BackendError> {
            Ok(())
        }
        fn operate(
            &self,
            _: &str,
            _: &[String],
            _: &CancelHandle,
        ) -> Result<crate::protocol::contract::CommandOutput, crate::runtime::backend::BackendError>
        {
            unreachable!()
        }
        fn state(&self, _: &str) -> Option<EnvironmentState> {
            Some(EnvironmentState::Available)
        }
        fn suspend(&self, _: &str) -> Result<(), crate::runtime::backend::BackendError> {
            Ok(())
        }
        fn resume(&self, _: &str) -> Result<(), crate::runtime::backend::BackendError> {
            Ok(())
        }
        fn terminate(&self, _: &str) -> Result<(), crate::runtime::backend::BackendError> {
            Ok(())
        }
        fn cleanup(&self, _: &str) -> Result<(), crate::runtime::backend::BackendError> {
            Ok(())
        }
        fn branch(&self, _: &str, _: &str) -> Result<(), crate::runtime::backend::BackendError> {
            Ok(())
        }
        fn promote(&self, _: &str) -> Result<(), crate::runtime::backend::BackendError> {
            Ok(())
        }
        fn invoke_tool(
            &self,
            _: &str,
            _: &str,
            _: &Value,
            cancel: &CancelHandle,
        ) -> Result<crate::protocol::contract::CommandOutput, crate::runtime::backend::BackendError>
        {
            cancel.output(json!({"content":[{"type":"text","text":"partial"}]}));
            Ok(crate::protocol::contract::CommandOutput {
                exit_code: 0,
                stdout: "{\"result\":\"done\"}".into(),
                stderr: String::new(),
            })
        }
    }
    fn call(commands: &Commands, request: Value) -> Vec<Value> {
        let mut socket = UnixStream::connect(commands.access("environment").address).unwrap();
        send(&mut socket, &request).unwrap();
        let mut events = Vec::new();
        let mut reader = BufReader::new(socket);
        loop {
            let event = wire::read(&mut reader, false).unwrap();
            let done = matches!(
                event.meta["event"].as_str(),
                Some("terminal" | "confirmed" | "rejected")
            );
            events.push(event.meta);
            if done {
                return events;
            }
        }
    }
    #[test]
    fn operation_identity_output_and_retirement_are_enforced() {
        let runtime = Arc::new(BaseRuntime::new("test", Arc::new(Fake)));
        runtime.establish("environment").unwrap();
        let commands = Commands::start(runtime).unwrap();
        let request = |n| json!({"command":"execute","request_id":format!("req-{n}"),"operation_id":format!("op-{n}"),"environment_id":"environment","tool":"fake","arguments":{}});
        let mut wrong = request(1);
        wrong["operation_id"] = json!("other");
        wrong["command"] = json!("cancel");
        assert_eq!(call(&commands, wrong)[0]["event"], "rejected");
        for n in 1..1300 {
            let events = call(&commands, request(n));
            assert_eq!(events[0]["event"], "accepted");
            assert_eq!(events[1]["event"], "output");
            assert_eq!(events[2]["event"], "terminal");
            assert_eq!(
                call(
                    &commands,
                    json!({"command":"confirm","operation_id":format!("op-{n}"),"environment_id":"environment"})
                )[0]["event"],
                "confirmed"
            );
        }
        assert_eq!(call(&commands, request(1))[0]["event"], "rejected");
        assert!(commands.entries.lock().unwrap().entries.len() < 300);
    }
    #[test]
    fn cancellation_fences_admission_until_writers_stop_and_old_generation_stays_dead() {
        struct Draining {
            entered: std::sync::Barrier,
            release: std::sync::Barrier,
            executing: std::sync::Barrier,
            drains: std::sync::atomic::AtomicUsize,
        }
        impl Backend for Draining {
            fn environments(&self) -> Vec<String> {
                Fake.environments()
            }
            fn establish(&self, e: &str) -> Result<(), crate::runtime::backend::BackendError> {
                Fake.establish(e)
            }
            fn operate(
                &self,
                e: &str,
                a: &[String],
                c: &CancelHandle,
            ) -> Result<
                crate::protocol::contract::CommandOutput,
                crate::runtime::backend::BackendError,
            > {
                Fake.operate(e, a, c)
            }
            fn state(&self, e: &str) -> Option<EnvironmentState> {
                Fake.state(e)
            }
            fn suspend(&self, e: &str) -> Result<(), crate::runtime::backend::BackendError> {
                Fake.suspend(e)
            }
            fn resume(&self, e: &str) -> Result<(), crate::runtime::backend::BackendError> {
                Fake.resume(e)
            }
            fn terminate(&self, e: &str) -> Result<(), crate::runtime::backend::BackendError> {
                Fake.terminate(e)
            }
            fn cleanup(&self, e: &str) -> Result<(), crate::runtime::backend::BackendError> {
                Fake.cleanup(e)
            }
            fn branch(
                &self,
                e: &str,
                c: &str,
            ) -> Result<(), crate::runtime::backend::BackendError> {
                Fake.branch(e, c)
            }
            fn promote(&self, e: &str) -> Result<(), crate::runtime::backend::BackendError> {
                Fake.promote(e)
            }
            fn invoke_tool(
                &self,
                e: &str,
                t: &str,
                a: &Value,
                c: &CancelHandle,
            ) -> Result<
                crate::protocol::contract::CommandOutput,
                crate::runtime::backend::BackendError,
            > {
                if a["wait"] == true {
                    self.executing.wait();
                    while !c.is_cancelled() {
                        std::thread::yield_now();
                    }
                    return Err(crate::runtime::backend::BackendError::interrupted());
                }
                Fake.invoke_tool(e, t, a, c)
            }
            fn cancel_tool_writers(
                &self,
                _: &BackendContext,
            ) -> Result<(), crate::runtime::backend::BackendError> {
                if self.drains.fetch_add(1, Ordering::SeqCst) == 0 {
                    self.entered.wait();
                    self.release.wait();
                    return Err(crate::runtime::backend::BackendError::new(
                        "injected writer drain failure",
                    ));
                }
                Ok(())
            }
        }
        let backend = Arc::new(Draining {
            entered: std::sync::Barrier::new(2),
            release: std::sync::Barrier::new(2),
            executing: std::sync::Barrier::new(2),
            drains: std::sync::atomic::AtomicUsize::new(0),
        });
        let runtime = Arc::new(BaseRuntime::new("test", backend.clone()));
        runtime.establish("environment").unwrap();
        let commands = Arc::new(Commands::start(runtime).unwrap());
        let active = std::thread::spawn({
            let commands = commands.clone();
            move || {
                call(
                    &commands,
                    json!({"command":"execute","request_id":"req-4","operation_id":"active","environment_id":"environment","tool":"fake","arguments":{"wait":true}}),
                )
            }
        });
        backend.executing.wait();
        let cancel = std::thread::spawn({
            let commands = commands.clone();
            move || {
                call(
                    &commands,
                    json!({"command":"cancel_environment","operation_id":"cancel","environment_id":"environment"}),
                )
            }
        });
        backend.entered.wait();
        let execute = |n, generation| json!({"command":"execute","request_id":format!("req-{n}"),"operation_id":format!("op-{n}"),"environment_id":"environment","generation":generation,"tool":"fake","arguments":{}});
        let during = call(&commands, execute(1, 0));
        backend.release.wait();
        assert_eq!(
            active.join().unwrap().last().unwrap()["status"],
            "cancelled"
        );
        assert_eq!(cancel.join().unwrap()[0]["event"], "rejected");
        assert_eq!(
            during[0]["event"], "rejected",
            "execution escaped Environment cancellation during writer drain"
        );
        assert_eq!(
            call(&commands, execute(1, 1))[0]["event"],
            "rejected",
            "failed writer cleanup must keep admission closed"
        );
        let retried = call(
            &commands,
            json!({"command":"cancel_environment","operation_id":"cancel-retry","environment_id":"environment"}),
        );
        assert_eq!(
            retried[0]["generation"], 1,
            "cleanup retry must retain the same generation"
        );
        assert_eq!(
            call(&commands, execute(2, 0))[0]["event"],
            "rejected",
            "delayed old work must not revive"
        );
        assert_eq!(
            call(&commands, execute(3, 1))[0]["event"],
            "accepted",
            "surviving authority can start a new task"
        );
    }
}

struct Entry {
    output: Mutex<Option<Replay>>,
    id: String,
    request: [u8; 32],
    cancel: Arc<CancelHandle>,
    terminal: Mutex<Option<Message>>,
    ready: Condvar,
}
// One anonymous file per operation with output. Readers snapshot the committed
// range and send it outside the lock, so a slow client cannot hold up cancel.
struct Replay {
    file: Arc<std::fs::File>,
    length: u64,
}
impl Replay {
    fn append(&mut self, event: &Message) -> std::io::Result<()> {
        let mut writer = &*self.file;
        writer.seek(SeekFrom::Start(self.length))?;
        event.write(&mut writer)?;
        self.length = writer.stream_position()?;
        Ok(())
    }
}

#[derive(Default)]
struct Table {
    generations: HashMap<String, u64>,
    paused: std::collections::HashSet<String>,
    cancelling: std::collections::HashSet<String>,
    draining: std::collections::HashSet<String>,
    entries: HashMap<(String, String), Arc<Entry>>,
    seen: HashMap<String, super::seen_requests::SeenRequests>,
}
type Entries = Mutex<Table>;

pub struct Commands {
    directory: tempfile::TempDir,
    stopped: Arc<AtomicBool>,
    entries: Arc<Entries>,
    listener: Option<std::thread::JoinHandle<()>>,
}

impl Commands {
    pub(super) fn start(runtime: Arc<BaseRuntime>) -> std::io::Result<Self> {
        let directory = tempfile::Builder::new()
            .prefix("ascout-commands-")
            .tempdir()?;
        let listener = UnixListener::bind(directory.path().join("command.sock"))?;
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let entries = Arc::new(Entries::default());
        let active = entries.clone();
        let listener_thread = std::thread::spawn(move || {
            for connection in listener.incoming() {
                if stop.load(Ordering::Acquire) {
                    break;
                }
                let Ok(connection) = connection else { break };
                let runtime = runtime.clone();
                let entries = entries.clone();
                let stopped = stop.clone();
                std::thread::spawn(move || {
                    let _ = serve(connection, runtime, entries, stopped);
                });
            }
        });
        Ok(Self {
            directory,
            stopped,
            entries: active,
            listener: Some(listener_thread),
        })
    }
    pub(super) fn pause_admission<'a>(
        &'a self,
        environments: &[&str],
    ) -> Result<PausedAdmission<'a>, &'static str> {
        let mut table = self.entries.lock().unwrap();
        let unique: std::collections::HashSet<_> = environments.iter().copied().collect();
        if unique.len() != environments.len()
            || unique.iter().any(|id| table.paused.contains(*id) || table.cancelling.contains(*id))
        {
            return Err("environment execution is already gated");
        }
        let environments: Vec<_> = environments.iter().map(|id| (*id).to_owned()).collect();
        table.paused.extend(environments.iter().cloned());
        Ok(PausedAdmission {
            commands: self,
            environments,
        })
    }

    pub(super) fn access(&self, environment: &str) -> ExecutionAccess {
        ExecutionAccess {
            transport: "unix-json-lines".into(),
            address: self
                .directory
                .path()
                .join("command.sock")
                .display()
                .to_string(),
            environment_id: environment.into(),
        }
    }
    pub(super) fn stop(&self) {
        if self.stopped.swap(true, Ordering::AcqRel) {
            return;
        }
        let _ = UnixStream::connect(self.directory.path().join("command.sock"));
        for entry in self.entries.lock().unwrap().entries.values() {
            entry.cancel.cancel();
        }
    }
}
pub(super) struct PausedAdmission<'a> {
    commands: &'a Commands,
    environments: Vec<String>,
}

impl PausedAdmission<'_> {
    pub(super) fn wait(&self, environment: &str) {
        let active: Vec<_> = self
            .commands
            .entries
            .lock()
            .unwrap()
            .entries
            .iter()
            .filter(|((owner, _), _)| owner == environment)
            .map(|(_, entry)| entry.clone())
            .collect();
        for entry in active {
            drop(
                entry
                    .ready
                    .wait_while(entry.terminal.lock().unwrap(), |value| value.is_none())
                    .unwrap(),
            );
        }
    }
}

impl Drop for PausedAdmission<'_> {
    fn drop(&mut self) {
        let mut table = self.commands.entries.lock().unwrap();
        for environment in &self.environments {
            table.paused.remove(environment);
        }
    }
}

impl Drop for Commands {
    fn drop(&mut self) {
        self.stop();
        if let Some(listener) = self.listener.take() {
            let _ = listener.join();
        }
    }
}

fn send(stream: &mut UnixStream, value: &Value) -> std::io::Result<()> {
    wire::write_header(stream, value)
}

fn serve(
    mut stream: UnixStream,
    runtime: Arc<BaseRuntime>,
    entries: Arc<Entries>,
    stopped: Arc<AtomicBool>,
) -> std::io::Result<()> {
    use std::time::Duration;
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;
    let reject = |stream: &mut UnixStream, error: &dyn std::fmt::Display| {
        send(
            stream,
            &json!({"event":"rejected","reason":error.to_string(),"error_code":"invalid_request"}),
        )
    };
    let mut reader = BufReader::new(stream.try_clone()?);
    let (meta, length) = match wire::read_header(&mut reader) {
        Ok(header) => header,
        Err(error) => return reject(&mut stream, &error),
    };
    let request: Request = match serde_json::from_value(meta.clone()) {
        Ok(request) => request,
        Err(error) => return reject(&mut stream, &error),
    };
    let validated = match &request {
        Request::Execute {
            operation: Some(operation),
            arguments,
            ..
        } => crate::runtime::environment::validate_body(operation, arguments, length),
        _ if length.is_some() => Err(wire::invalid("unexpected request body")),
        _ => Ok(()),
    };
    if let Err(error) = validated {
        return reject(&mut stream, &error);
    }
    let (operation, environment) = match &request {
        Request::Execute {
            operation_id,
            environment_id,
            ..
        }
        | Request::CancelEnvironment {
            operation_id,
            environment_id,
        }
        | Request::Cancel {
            operation_id,
            environment_id,
        }
        | Request::Confirm {
            operation_id,
            environment_id,
        } => (operation_id, environment_id),
    };
    if operation.is_empty() || environment.is_empty() {
        return send(
            &mut stream,
            &json!({"event":"rejected","reason":"Operation and Environment are required"}),
        );
    }
    let key = (environment.clone(), operation.clone());
    let body = match length
        .map(|length| wire::spool_body(&mut reader, length))
        .transpose()
    {
        Ok(body) => body,
        Err(error) => return reject(&mut stream, &error),
    };
    let inbound = Message { meta, body };
    let fingerprint = inbound.fingerprint()?;
    let body = inbound.body;
    if stopped.load(Ordering::Acquire) {
        return reject(&mut stream, &"Runtime is shutting down");
    }
    let guard = match runtime.enter_operation() {
        Ok(guard) => guard,
        Err(reason) => return reject(&mut stream, &reason),
    };
    match request {
        Request::CancelEnvironment { .. } => {
            let owner = environment.clone();
            let active: Vec<_> = {
                let mut table = entries.lock().unwrap();
                if table.paused.contains(&owner) || !table.draining.insert(owner.clone()) {
                    return send(
                        &mut stream,
                        &json!({"event":"rejected","reason":"Environment cancellation in progress"}),
                    );
                }
                if table.cancelling.insert(owner.clone()) {
                    *table.generations.entry(owner.clone()).or_default() += 1;
                }
                table
                    .entries
                    .iter()
                    .filter(|((env, _), _)| env == environment)
                    .map(|(_, entry)| entry.clone())
                    .collect()
            };
            for entry in &active {
                entry.cancel.cancel();
            }
            for entry in &active {
                drop(
                    entry
                        .ready
                        .wait_while(entry.terminal.lock().unwrap(), |value| value.is_none())
                        .unwrap(),
                );
            }
            match runtime.cancel_tool_writers(&BackendContext::new(environment)) {
                Ok(()) => {
                    let mut table = entries.lock().unwrap();
                    table.draining.remove(&owner);
                    table.cancelling.remove(&owner);
                    let generation = table.generations[&owner];
                    send(
                        &mut stream,
                        &json!({"event":"terminal","status":"cancelled","generation":generation}),
                    )
                }
                Err(error) => {
                    entries.lock().unwrap().draining.remove(&owner);
                    send(
                        &mut stream,
                        &json!({"event":"rejected","reason":error.to_string()}),
                    )
                }
            }
        }
        Request::Execute {
            generation,
            request_id,
            tool,
            operation: environment_operation,
            arguments,
            context,
            ..
        } => {
            if environment_operation.as_deref().is_some_and(|operation| {
                !matches!(
                    operation,
                    "read_file" | "write_file" | "replace_text" | "list_directory" | "exec"
                )
            }) || (environment_operation.is_some() && !tool.is_empty())
                || (environment_operation.is_none() && tool.is_empty())
            {
                return send(
                    &mut stream,
                    &json!({"event":"rejected","reason":"exactly one supported operation or declared tool is required"}),
                );
            }
            let (entry, fresh) = {
                let mut table = entries.lock().unwrap();
                if stopped.load(Ordering::Acquire) {
                    return reject(&mut stream, &"Runtime is shutting down");
                }
                let owner = key.0.clone();
                if table.paused.contains(&owner)
                    || table.cancelling.contains(&owner)
                    || generation != *table.generations.get(&owner).unwrap_or(&0)
                {
                    return send(
                        &mut stream,
                        &json!({"event":"rejected","reason":"Environment execution generation is cancelled or cancellation is in progress"}),
                    );
                }
                if let Some(entry) = table.entries.get(&key) {
                    if entry.request != fingerprint {
                        return send(
                            &mut stream,
                            &json!({"event":"rejected","reason":"operation identity reused with different content"}),
                        );
                    }
                    (entry.clone(), false)
                } else {
                    let Table { entries, seen, .. } = &mut *table;
                    let seen = seen.entry(key.0.clone()).or_default();
                    if seen.get(&request_id).is_some() {
                        return send(
                            &mut stream,
                            &json!({"event":"rejected","reason":"request identity already used in Environment"}),
                        );
                    }
                    if let Some(reason) = seen.admission_error(&request_id) {
                        return send(&mut stream, &json!({"event":"rejected","reason":reason}));
                    }
                    entries.retain(|(environment, _), entry| {
                        environment != &key.0 || seen.get(&entry.id).is_some()
                    });
                    seen.insert(request_id.clone(), (fingerprint, false));
                    let entry = Arc::new(Entry {
                        output: Mutex::new(None),
                        id: request_id,
                        request: fingerprint,
                        cancel: Arc::new(CancelHandle::default()),
                        terminal: Mutex::new(None),
                        ready: Condvar::new(),
                    });
                    let weak = Arc::downgrade(&entry);
                    entry.cancel.observe_output(move |message| {
                        if let Some(entry) = weak.upgrade() {
                            let _terminal = entry.terminal.lock().unwrap();
                            let mut output = entry.output.lock().unwrap();
                            if output.is_none() {
                                *output = Some(Replay {
                                    file: Arc::new(tempfile::tempfile()?),
                                    length: 0,
                                });
                            }
                            let event = Message {
                                meta: json!({"event":"output","value":message.meta}),
                                body: message.body,
                            };
                            output.as_mut().unwrap().append(&event)?;
                            entry.ready.notify_all();
                        }
                        Ok(())
                    });
                    table.entries.insert(key.clone(), entry.clone());
                    (entry, true)
                }
            };
            if !fresh {
                drop(body);
            } else {
                let entry = entry.clone();
                let environment = key.0.clone();
                let operation = key.1.clone();
                std::thread::spawn(move || {
                    let _guard = guard;
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        if entry.cancel.is_cancelled() {
                            Err(crate::runtime::backend::BackendError::interrupted())
                        } else if runtime.state(&environment) != Some(EnvironmentState::Available) {
                            Err(crate::runtime::backend::BackendError::new(
                                "Environment is not available",
                            ))
                        } else {
                            let context = BackendContext {
                                environment_id: environment,
                                operation_id: operation,
                                tool_call_id: context
                                    .get("tool_call_id")
                                    .and_then(Value::as_str)
                                    .map(str::to_owned),
                                generation,
                                task_id: context
                                    .get("task_id")
                                    .and_then(Value::as_str)
                                    .map(str::to_owned),
                                ..Default::default()
                            };
                            if let Some(operation) = environment_operation {
                                runtime.environment_operation(
                                    &context,
                                    &operation,
                                    &Message { meta: arguments, body },
                                    &entry.cancel,
                                ).map(|mut message| {
                                    message.meta["event"] = "terminal".into();
                                    message.meta["status"] = "succeeded".into();
                                    message
                                })
                            } else {
                                runtime.invoke_tool_with_context(&context, &tool, &arguments, &entry.cancel)
                                    .and_then(|execution| {
                                        let output = execution.single_output().ok_or_else(|| BackendError::new("ambiguous backend output"))?;
                                        let encode = || -> std::io::Result<Message> {
                                            let mut spool = tempfile::tempfile()?;
                                            spool.write_all(output.stdout.as_bytes())?;
                                            spool.write_all(output.stderr.as_bytes())?;
                                            let length = spool.metadata()?.len();
                                            Ok(Message {
                                                meta: json!({"event":"terminal","status":"succeeded","body_kind":"business_output","stdout_length":output.stdout.len(),"exit_code":output.exit_code}),
                                                body: Some(Body::file(spool, 0, length)),
                                            })
                                        };
                                        encode().map_err(|error| BackendError::new(error.to_string()))
                                    })
                            }
                        }
                    }))
                    .unwrap_or_else(|_| Err(BackendError::new("backend panicked during execution")));
                    let terminal = result.unwrap_or_else(|error| {
                        json!({"event":"terminal","status":if error.cancelled { "cancelled" } else if error.error_category == "timed_out" { "timed_out" } else { "failed" },"error_category":error.error_category,"reason":error.to_string()}).into()
                    });
                    *entry.terminal.lock().unwrap() = Some(terminal);
                    entry.ready.notify_all();
                });
            }
            send(&mut stream, &json!({"event":"accepted"}))?;
            let mut cursor = 0;
            loop {
                let (output, terminal) = {
                    let terminal = entry
                        .ready
                        .wait_while(entry.terminal.lock().unwrap(), |terminal| {
                            terminal.is_none()
                                && entry
                                    .output
                                    .lock()
                                    .unwrap()
                                    .as_ref()
                                    .is_none_or(|output| output.length == cursor)
                        })
                        .unwrap();
                    let output = entry.output.lock().unwrap().as_ref().and_then(|output| {
                        (output.length > cursor).then(|| Body::File {
                            file: output.file.clone(),
                            offset: cursor,
                            len: output.length - cursor,
                        })
                    });
                    (output, terminal.clone())
                };
                if let Some(output) = output {
                    if std::io::copy(&mut output.reader(), &mut stream)? != output.len() {
                        return Err(wire::invalid("incomplete cached output"));
                    }
                    cursor += output.len();
                }
                if let Some(terminal) = terminal {
                    return terminal.write(&mut stream);
                }
            }
        }
        Request::Cancel { .. } => {
            let entry = entries.lock().unwrap().entries.get(&key).cloned();
            let Some(entry) = entry else {
                return send(
                    &mut stream,
                    &json!({"event":"rejected","reason":"unknown operation"}),
                );
            };
            entry.cancel.cancel();
            // Receipt is not completion: only the backend's terminal return is.
            let terminal = entry
                .ready
                .wait_while(entry.terminal.lock().unwrap(), |value| value.is_none())
                .unwrap();
            // Cancellation only acknowledges the terminal status. The execute
            // connection carries the result and all preceding output.
            let mut receipt = terminal.as_ref().unwrap().meta.clone();
            for field in ["value", "exit_code", "body_kind", "stdout_length"] {
                receipt.as_object_mut().unwrap().remove(field);
            }
            drop(terminal);
            send(&mut stream, &receipt)
        }
        Request::Confirm { .. } => {
            let mut table = entries.lock().unwrap();
            let Some(entry) = table.entries.get(&key).cloned() else {
                return send(
                    &mut stream,
                    &json!({"event":"rejected","reason":"unknown operation"}),
                );
            };
            if entry.terminal.lock().unwrap().is_none() {
                return send(
                    &mut stream,
                    &json!({"event":"rejected","reason":"operation is not terminal"}),
                );
            }
            *entry.terminal.lock().unwrap() = Some(
                json!({"event":"rejected","reason":"operation already confirmed; result released"})
                    .into(),
            );
            *entry.output.lock().unwrap() = None;
            if let Some((_, complete)) = table
                .seen
                .get_mut(&key.0)
                .and_then(|seen| seen.get_mut(&entry.id))
            {
                *complete = true;
            }
            // Keep the bounded identity tombstone: confirmation must not permit replay.
            send(&mut stream, &json!({"event":"confirmed"}))
        }
    }
}
