//! Instrumentation command connection, separate from control-plane operations.
use super::*;
use crate::protocol::contract::{EnvironmentState, ExecutionAccess};
use crate::runtime::backend::{BackendContext, CancelHandle};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
enum Request {
    CancelBranch {
        branch_id: String,
        operation_id: String,
        environment_id: String,
    },
    Execute {
        #[serde(default)]
        generation: u64,
        request_id: String,
        branch_id: String,
        operation_id: String,
        environment_id: String,
        tool: String,
        arguments: Value,
        #[serde(default)]
        context: Value,
    },
    Cancel {
        branch_id: String,
        operation_id: String,
        environment_id: String,
    },
    Confirm {
        branch_id: String,
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
        BufReader::new(socket)
            .lines()
            .map(|line| serde_json::from_str(&line.unwrap()).unwrap())
            .collect()
    }
    #[test]
    fn authorization_output_and_retirement_are_enforced() {
        let commands = Commands::start(Arc::new(Fake)).unwrap();
        commands.bind("branch", "environment").unwrap();
        let request = |n| json!({"command":"execute","request_id":format!("req-{n}"),"branch_id":"branch","operation_id":format!("op-{n}"),"environment_id":"environment","tool":"fake","arguments":{}});
        let mut wrong = request(1);
        wrong["branch_id"] = json!("other");
        assert_eq!(call(&commands, wrong)[0]["event"], "rejected");
        for n in 1..1300 {
            let events = call(&commands, request(n));
            assert_eq!(events[0]["event"], "accepted");
            assert_eq!(events[1]["event"], "output");
            assert_eq!(events[2]["event"], "terminal");
            assert_eq!(
                call(
                    &commands,
                    json!({"command":"confirm","branch_id":"branch","operation_id":format!("op-{n}"),"environment_id":"environment"})
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
        let commands = Arc::new(Commands::start(backend.clone()).unwrap());
        commands.bind("branch", "environment").unwrap();
        let active = std::thread::spawn({
            let commands = commands.clone();
            move || {
                call(
                    &commands,
                    json!({"command":"execute","request_id":"req-4","operation_id":"active","branch_id":"branch","environment_id":"environment","tool":"fake","arguments":{"wait":true}}),
                )
            }
        });
        backend.executing.wait();
        let cancel = std::thread::spawn({
            let commands = commands.clone();
            move || {
                call(
                    &commands,
                    json!({"command":"cancel_branch","branch_id":"branch","operation_id":"cancel","environment_id":"environment"}),
                )
            }
        });
        backend.entered.wait();
        let execute = |n, generation| json!({"command":"execute","request_id":format!("req-{n}"),"operation_id":format!("op-{n}"),"branch_id":"branch","environment_id":"environment","generation":generation,"tool":"fake","arguments":{}});
        let during = call(&commands, execute(1, 0));
        backend.release.wait();
        assert_eq!(
            active.join().unwrap().last().unwrap()["status"],
            "cancelled"
        );
        assert_eq!(cancel.join().unwrap()[0]["event"], "rejected");
        assert_eq!(
            during[0]["event"], "rejected",
            "execution escaped branch cancellation during writer drain"
        );
        assert_eq!(
            call(&commands, execute(1, 1))[0]["event"],
            "rejected",
            "failed writer cleanup must keep admission closed"
        );
        let retried = call(
            &commands,
            json!({"command":"cancel_branch","branch_id":"branch","operation_id":"cancel-retry","environment_id":"environment"}),
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
    output: Mutex<Vec<Value>>,
    id: String,
    request: [u8; 32],
    cancel: Arc<CancelHandle>,
    terminal: Mutex<Option<Value>>,
    ready: Condvar,
}
#[derive(Default)]
struct Table {
    generations: HashMap<(String, String), u64>,
    cancelling: std::collections::HashSet<(String, String)>,
    draining: std::collections::HashSet<(String, String)>,
    entries: HashMap<(String, String, String), Arc<Entry>>,
    seen: super::seen_requests::SeenRequests,
}
type Entries = Mutex<Table>;

pub struct Commands {
    bindings: Arc<Mutex<HashMap<String, String>>>,
    directory: tempfile::TempDir,
    stopped: Arc<AtomicBool>,
    entries: Arc<Entries>,
    listener: Option<std::thread::JoinHandle<()>>,
}

impl Commands {
    pub(super) fn start(runtime: Arc<dyn Backend>) -> std::io::Result<Self> {
        let directory = tempfile::Builder::new()
            .prefix("ascout-commands-")
            .tempdir()?;
        let listener = UnixListener::bind(directory.path().join("command.sock"))?;
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let entries = Arc::new(Entries::default());
        let active = entries.clone();
        let bindings = Arc::new(Mutex::new(HashMap::new()));
        let authorized = bindings.clone();
        let listener_thread = std::thread::spawn(move || {
            for connection in listener.incoming() {
                if stop.load(Ordering::Acquire) {
                    break;
                }
                let Ok(connection) = connection else { break };
                let runtime = runtime.clone();
                let entries = entries.clone();
                let bindings = authorized.clone();
                std::thread::spawn(move || {
                    let _ = serve(connection, runtime, entries, bindings);
                });
            }
        });
        Ok(Self {
            bindings,
            directory,
            stopped,
            entries: active,
            listener: Some(listener_thread),
        })
    }
    pub(super) fn bind(&self, branch: &str, environment: &str) -> Result<(), &'static str> {
        let mut bindings = self.bindings.lock().unwrap();
        if bindings
            .get(branch)
            .is_some_and(|bound| bound != environment)
            || bindings
                .iter()
                .any(|(owner, bound)| owner != branch && bound == environment)
        {
            return Err("Branch and Environment already have a different owner");
        }
        bindings.insert(branch.into(), environment.into());
        Ok(())
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
}
impl Drop for Commands {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        let _ = UnixStream::connect(self.directory.path().join("command.sock"));
        if let Some(listener) = self.listener.take() {
            let _ = listener.join();
        }
        for entry in self.entries.lock().unwrap().entries.values() {
            entry.cancel.cancel();
        }
    }
}

fn send(stream: &mut UnixStream, value: &Value) -> std::io::Result<()> {
    serde_json::to_writer(&mut *stream, value)?;
    stream.write_all(b"\n")
}

fn serve(
    mut stream: UnixStream,
    runtime: Arc<dyn Backend>,
    entries: Arc<Entries>,
    bindings: Arc<Mutex<HashMap<String, String>>>,
) -> std::io::Result<()> {
    let mut line = String::new();
    BufReader::new(stream.try_clone()?)
        .take(16 * 1024 * 1024)
        .read_line(&mut line)?;
    let encoded: Value = serde_json::from_str(&line)?;
    let fingerprint: [u8; 32] = Sha256::digest(serde_json::to_vec(&encoded)?).into();
    let request: Request = serde_json::from_value(encoded.clone())?;
    let (branch, operation, environment) = match &request {
        Request::Execute {
            branch_id,
            operation_id,
            environment_id,
            ..
        }
        | Request::CancelBranch {
            branch_id,
            operation_id,
            environment_id,
        }
        | Request::Cancel {
            branch_id,
            operation_id,
            environment_id,
        }
        | Request::Confirm {
            branch_id,
            operation_id,
            environment_id,
        } => (branch_id, operation_id, environment_id),
    };
    if branch.is_empty() || operation.is_empty() || environment.is_empty() {
        return send(
            &mut stream,
            &json!({"event":"rejected","reason":"Branch, operation and Environment are required"}),
        );
    }
    let key = (environment.clone(), branch.clone(), operation.clone());
    if bindings.lock().unwrap().get(branch) != Some(environment) {
        return send(
            &mut stream,
            &json!({"event":"rejected","reason":"Branch is not authorized for Environment"}),
        );
    }
    match request {
        Request::CancelBranch { .. } => {
            let owner = (environment.clone(), branch.clone());
            let active: Vec<_> = {
                let mut table = entries.lock().unwrap();
                if !table.draining.insert(owner.clone()) {
                    return send(
                        &mut stream,
                        &json!({"event":"rejected","reason":"Branch cancellation in progress"}),
                    );
                }
                if table.cancelling.insert(owner.clone()) {
                    *table.generations.entry(owner.clone()).or_default() += 1;
                }
                table
                    .entries
                    .iter()
                    .filter(|((env, owner, _), _)| env == environment && owner == branch)
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
            arguments,
            context,
            ..
        } => {
            let (entry, fresh) = {
                let mut table = entries.lock().unwrap();
                let owner = (key.0.clone(), key.1.clone());
                if table.cancelling.contains(&owner)
                    || generation != *table.generations.get(&owner).unwrap_or(&0)
                {
                    return send(
                        &mut stream,
                        &json!({"event":"rejected","reason":"Branch execution generation is cancelled or cancellation is in progress"}),
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
                    if table.seen.get(&request_id).is_some() {
                        return send(
                            &mut stream,
                            &json!({"event":"rejected","reason":"request identity already used"}),
                        );
                    }
                    if let Some(reason) = table.seen.admission_error(&request_id) {
                        return send(&mut stream, &json!({"event":"rejected","reason":reason}));
                    }
                    let Table { entries, seen, .. } = &mut *table;
                    entries.retain(|_, entry| seen.get(&entry.id).is_some());
                    seen.insert(request_id.clone(), (fingerprint, false));
                    let entry = Arc::new(Entry {
                        output: Mutex::new(Vec::new()),
                        id: request_id,
                        request: fingerprint,
                        cancel: Arc::new(CancelHandle::default()),
                        terminal: Mutex::new(None),
                        ready: Condvar::new(),
                    });
                    let weak = Arc::downgrade(&entry);
                    entry.cancel.observe_output(move |value| {
                        if let Some(entry) = weak.upgrade() {
                            let _terminal = entry.terminal.lock().unwrap();
                            entry.output.lock().unwrap().push(value);
                            entry.ready.notify_all();
                        }
                    });
                    table.entries.insert(key.clone(), entry.clone());
                    (entry, true)
                }
            };
            if fresh {
                let entry = entry.clone();
                let environment = key.0.clone();
                let operation = key.2.clone();
                std::thread::spawn(move || {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        if entry.cancel.is_cancelled() {
                            Err(crate::runtime::backend::BackendError::interrupted())
                        } else if runtime.state(&environment) != Some(EnvironmentState::Available) {
                            Err(crate::runtime::backend::BackendError::new(
                                "Environment is not available",
                            ))
                        } else {
                            runtime.invoke_tool_with_context(
                                &BackendContext {
                                    environment_id: environment,
                                    operation_id: operation,
                                    task_id: context
                                        .get("task_id")
                                        .and_then(Value::as_str)
                                        .map(str::to_owned),
                                    ..Default::default()
                                },
                                &tool,
                                &arguments,
                                &entry.cancel,
                            )
                        }
                    }))
                    .unwrap_or_else(|_| {
                        Err(crate::runtime::backend::BackendError::new(
                            "backend panicked during execution",
                        ))
                    });
                    let terminal = match result {
                        Ok(result) => match result.single_output() {
                            Some(output) => {
                                json!({"event":"terminal","status":"succeeded","output":output})
                            }
                            None => {
                                json!({"event":"terminal","status":"failed","reason":"ambiguous backend output"})
                            }
                        },
                        Err(error) => {
                            json!({"event":"terminal","status":if error.cancelled { "cancelled" } else { "failed" },"reason":error.to_string()})
                        }
                    };
                    *entry.terminal.lock().unwrap() = Some(terminal);
                    entry.ready.notify_all();
                });
            }
            send(&mut stream, &json!({"event":"accepted"}))?;
            let mut cursor = 0;
            let mut terminal = entry.terminal.lock().unwrap();
            loop {
                let output = entry.output.lock().unwrap();
                for value in output.iter().skip(cursor) {
                    send(&mut stream, &json!({"event":"output","value":value}))?;
                }
                cursor = output.len();
                drop(output);
                if let Some(value) = terminal.as_ref() {
                    return send(&mut stream, value);
                }
                terminal = entry.ready.wait(terminal).unwrap();
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
            send(&mut stream, terminal.as_ref().unwrap())
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
                json!({"event":"rejected","reason":"operation already confirmed; result released"}),
            );
            entry.output.lock().unwrap().clear();
            if let Some((_, complete)) = table.seen.get_mut(&entry.id) {
                *complete = true;
            }
            // Keep the bounded identity tombstone: confirmation must not permit replay.
            send(&mut stream, &json!({"event":"confirmed"}))
        }
    }
}
