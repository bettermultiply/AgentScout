//! Requests to independent backend executables. Tool messages can carry framed bytes.

use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, BufRead, BufReader, Read, Write},
    os::{
        fd::OwnedFd,
        unix::{fs::PermissionsExt, net::UnixStream, process::CommandExt},
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Mutex, MutexGuard, TryLockError},
    thread,
    time::{Duration, Instant},
};

use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};

use super::{
    backend::{Backend, BackendContext, BackendError, BackendExecution, CancelHandle},
    backend_registry::{BackendImplementation, BackendInterface},
    execution_wire::{self as wire, Message},
};
use crate::protocol::contract::{CommandOutput, EnvironmentState, RuntimeCapabilities};

const VERSION: u32 = 1;
const MAX_LINE: usize = 1024 * 1024;
const IO_INTERVAL: Duration = Duration::from_millis(50);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const EXIT_TIMEOUT: Duration = Duration::from_secs(2);
const METHODS: &[&str] = &[
    "establish",
    "operate",
    "branch",
    "promote",
    "suspend",
    "resume",
    "terminate",
    "cleanup",
    "shutdown",
];

#[derive(Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct Capabilities {
    #[serde(default)]
    branch_environment: bool,
}

#[derive(Deserialize)]
struct Declaration {
    executable: PathBuf,
    #[serde(default)]
    args: Vec<String>,
    interfaces: Vec<BackendInterface>,
    capabilities: Capabilities,
    config: Value,
}

pub fn resolve_executable(
    path: &Path,
    entry: &Value,
) -> Result<BackendImplementation, Box<dyn std::error::Error>> {
    let mut declaration: Declaration = serde_json::from_value(entry.clone())?;
    declaration.executable = path
        .parent()
        .unwrap_or(Path::new("."))
        .join(&declaration.executable)
        .canonicalize()?;
    let metadata = declaration.executable.metadata()?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
        return Err(format!(
            "backend program is not an executable file: {}",
            declaration.executable.display()
        )
        .into());
    }
    if declaration.interfaces != [BackendInterface::Environment] {
        return Err("executable backend v1 requires exactly the environment interface".into());
    }
    let capabilities = RuntimeCapabilities {
        branch_environment: declaration.capabilities.branch_environment,
        ..Default::default()
    };
    Ok(BackendImplementation {
        interfaces: declaration.interfaces.clone(),
        capabilities,
        create: Box::new(move || Ok(std::sync::Arc::new(ExecutableBackend::start(declaration)?))),
    })
}

struct ExecutableBackend {
    instance: Mutex<Instance>,
    capabilities: RuntimeCapabilities,
}

struct Instance {
    child: Child,
    channel: BufReader<UnixStream>,
    closed: bool,
    environments: BTreeMap<String, EnvironmentState>,
    methods: BTreeSet<String>,
}

#[derive(Deserialize)]
struct Reply {
    ok: bool,
    #[serde(default)]
    result: Value,
    error: Option<String>,
}

#[derive(Deserialize)]
struct Hello {
    version: u32,
    methods: BTreeSet<String>,
    capabilities: Capabilities,
}

fn backend_error(error: impl std::fmt::Display) -> BackendError {
    BackendError::new(format!("executable backend: {error}"))
}

fn check_wait(cancel: &CancelHandle, deadline: Instant) -> Result<(), BackendError> {
    if cancel.is_cancelled() {
        return Err(BackendError::interrupted());
    }
    if Instant::now() >= deadline {
        return Err(BackendError::operation_error(
            "executable backend request timed out",
            "timed_out",
        ));
    }
    Ok(())
}

fn retry(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

// A tool exchange owns the channel until its terminal. Socket timeouts give
// us a chance to send control messages without unwinding the frame decoder.
struct ToolIo<'a> {
    channel: &'a mut BufReader<UnixStream>,
    cancel: &'a CancelHandle,
    cancel_request: Option<Value>,
    deadline: Instant,
    upload_started: bool,
}

impl ToolIo<'_> {
    fn check_deadline(&self) -> io::Result<()> {
        if Instant::now() >= self.deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "tool request timed out",
            ));
        }
        Ok(())
    }
}

impl Read for ToolIo<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let count = self.fill_buf()?.read(buffer)?;
        self.consume(count);
        Ok(count)
    }
}

impl BufRead for ToolIo<'_> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        loop {
            self.check_deadline()?;
            if self.cancel.is_cancelled()
                && let Some(request) = self.cancel_request.take()
            {
                // Only the read phase may send cancellation. A partially sent
                // request must reach its final frame before the next header.
                wire::write_header(self, &request)?;
            }
            if self.channel.buffer().is_empty() {
                match self.channel.fill_buf() {
                    Ok([]) => return Ok(&[]),
                    Ok(_) => (),
                    Err(error) if retry(&error) => continue,
                    Err(error) => return Err(error),
                }
            }
            return Ok(self.channel.buffer());
        }
    }

    fn consume(&mut self, count: usize) {
        self.channel.consume(count);
    }
}

impl Write for ToolIo<'_> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        loop {
            self.check_deadline()?;
            if !self.upload_started && self.cancel.is_cancelled() {
                return Err(io::Error::other(BackendError::interrupted()));
            }
            match self.channel.get_mut().write(buffer) {
                Err(error) if retry(&error) => continue,
                Ok(count) => {
                    self.upload_started |= count != 0;
                    return Ok(count);
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        self.channel.get_mut().flush()
    }
}

impl Instance {
    fn exchange(
        &mut self,
        request: &Value,
        cancel: &CancelHandle,
        deadline: Instant,
    ) -> Result<Reply, BackendError> {
        let mut bytes = serde_json::to_vec(request).map_err(backend_error)?;
        if bytes.len() >= MAX_LINE {
            return Err(backend_error("request exceeds 1 MiB"));
        }
        bytes.push(b'\n');
        check_wait(cancel, deadline)?;
        if self.closed {
            return Err(backend_error("process is already closed"));
        }
        let result = self.exchange_line(&bytes, cancel, deadline);
        if result.is_err() {
            self.abort();
        }
        result
    }

    fn tool(
        &mut self,
        method: &str,
        request: Message,
        cancel: &CancelHandle,
        deadline: Instant,
    ) -> Result<Message, BackendError> {
        check_wait(cancel, deadline)?;
        if self.closed
            || self
                .environments
                .get(request.meta["environment"].as_str().unwrap_or_default())
                != Some(&EnvironmentState::Available)
        {
            return Err(backend_error("environment is not available"));
        }
        if !self.methods.contains(method) {
            return Err(backend_error(format!("method {method} is unavailable")));
        }
        let cancel_request = json!({
            "op": "cancel",
            "environment": request.meta["environment"],
            "operation_id": request.meta["operation_id"],
        });
        let mut io = ToolIo {
            channel: &mut self.channel,
            cancel,
            cancel_request: Some(cancel_request),
            deadline,
            upload_started: false,
        };
        let exchange = (|| -> io::Result<Message> {
            request.write(&mut io)?;
            loop {
                let message = wire::read(&mut io, true)?;
                if message.meta["ok"].is_boolean() {
                    return Ok(message);
                }
                if matches!(message.meta["stream"].as_str(), Some("stdout" | "stderr"))
                    && message.body.is_some()
                {
                    cancel.output_message(message).map_err(io::Error::other)?;
                } else {
                    return Err(wire::invalid("invalid executable backend tool reply"));
                }
            }
        })();
        let response = match exchange {
            Ok(message) => message,
            Err(error) => {
                if let Some(interrupted) = error
                    .get_ref()
                    .and_then(|cause| cause.downcast_ref::<BackendError>())
                    .filter(|cause| cause.cancelled)
                {
                    return Err(interrupted.clone());
                }
                // Once we have sent any bytes, only a complete terminal makes
                // this exchange reusable. Transport failures are not cancellation.
                if io.upload_started
                    || !matches!(
                        error.kind(),
                        io::ErrorKind::InvalidInput | io::ErrorKind::TimedOut
                    )
                {
                    self.abort();
                }
                return Err(if error.kind() == io::ErrorKind::TimedOut {
                    BackendError::operation_error(
                        "executable backend tool request timed out",
                        "timed_out",
                    )
                } else {
                    backend_error(error)
                });
            }
        };
        if response.meta["ok"] == false {
            return Err(BackendError::operation_error(
                response.meta["error"]
                    .as_str()
                    .unwrap_or("executable backend rejected request"),
                response.meta["error_category"]
                    .as_str()
                    .unwrap_or("backend_error"),
            ));
        }
        Ok(response)
    }

    fn exchange_line(
        &mut self,
        mut request: &[u8],
        cancel: &CancelHandle,
        deadline: Instant,
    ) -> Result<Reply, BackendError> {
        while !request.is_empty() {
            check_wait(cancel, deadline)?;
            match self.channel.get_mut().write(request) {
                Ok(0) => return Err(backend_error("process closed its input")),
                Ok(count) => request = &request[count..],
                Err(error) if retry(&error) => (),
                Err(error) => return Err(backend_error(error)),
            }
        }
        let mut bytes = Vec::new();
        loop {
            check_wait(cancel, deadline)?;
            let buffer = match self.channel.fill_buf() {
                Ok([]) => return Err(backend_error("process exited or closed its output")),
                Ok(buffer) => buffer,
                Err(error) if retry(&error) => continue,
                Err(error) => return Err(backend_error(error)),
            };
            let newline = buffer.iter().position(|&byte| byte == b'\n');
            let count = newline.map_or(buffer.len(), |index| index + 1);
            if bytes.len() + count > MAX_LINE {
                return Err(backend_error("reply exceeds 1 MiB"));
            }
            bytes.extend_from_slice(&buffer[..count]);
            self.channel.consume(count);
            if newline.is_some() {
                return serde_json::from_slice(&bytes).map_err(backend_error);
            }
        }
    }

    fn call<T: DeserializeOwned>(
        &mut self,
        request: &Value,
        cancel: &CancelHandle,
        deadline: Instant,
    ) -> Result<T, BackendError> {
        let response = self.exchange(request, cancel, deadline)?;
        if !response.ok {
            return Err(backend_error(
                response.error.unwrap_or_else(|| "request rejected".into()),
            ));
        }
        serde_json::from_value(response.result).map_err(|error| {
            self.abort();
            backend_error(error)
        })
    }

    fn abort(&mut self) {
        if !self.closed {
            // The program is the leader of a dedicated process group.
            // SAFETY: the negative child PID addresses its dedicated process group.
            unsafe {
                libc::kill(-(self.child.id() as libc::pid_t), libc::SIGKILL);
            }
            let _ = self.child.kill();
            let _ = self.child.wait();
            self.closed = true;
            for state in self.environments.values_mut() {
                *state = EnvironmentState::PartiallyAvailable;
            }
        }
    }

    fn finish(&mut self) -> Result<(), BackendError> {
        self.channel
            .get_ref()
            .shutdown(std::net::Shutdown::Write)
            .map_err(backend_error)?;
        let deadline = Instant::now() + EXIT_TIMEOUT;
        loop {
            if let Some(status) = self.child.try_wait().map_err(backend_error)? {
                self.closed = true;
                return if status.success() {
                    Ok(())
                } else {
                    Err(backend_error(status))
                };
            }
            if Instant::now() >= deadline {
                self.abort();
                return Err(backend_error("process did not exit after shutdown"));
            }
            thread::sleep(IO_INTERVAL);
        }
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        self.abort();
    }
}

impl ExecutableBackend {
    fn start(declaration: Declaration) -> Result<Self, BackendError> {
        let (host, program) = UnixStream::pair().map_err(backend_error)?;
        host.set_read_timeout(Some(IO_INTERVAL))
            .map_err(backend_error)?;
        host.set_write_timeout(Some(IO_INTERVAL))
            .map_err(backend_error)?;
        let input = OwnedFd::from(program.try_clone().map_err(backend_error)?);
        let output = OwnedFd::from(program);
        let child = Command::new(&declaration.executable)
            .args(&declaration.args)
            .process_group(0)
            .stdin(Stdio::from(input))
            .stdout(Stdio::from(output))
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| {
                backend_error(format!("{}: {error}", declaration.executable.display()))
            })?;
        let mut instance = Instance {
            child,
            channel: BufReader::new(host),
            closed: false,
            environments: BTreeMap::new(),
            methods: BTreeSet::new(),
        };
        let hello: Hello = instance.call(
            &json!({"op":"hello", "version": VERSION, "config": declaration.config}),
            &CancelHandle::default(),
            Instant::now() + REQUEST_TIMEOUT,
        )?;
        if hello.version != VERSION || hello.capabilities != declaration.capabilities {
            return Err(backend_error(
                "hello version or capabilities do not match the declaration",
            ));
        }
        for method in METHODS {
            if !hello.methods.contains(*method) {
                return Err(backend_error(format!(
                    "hello missing required method {method}"
                )));
            }
        }
        if (hello.methods.contains("environment_operation")
            || hello.methods.contains("invoke_tool"))
            && !hello.methods.contains("cancel")
        {
            return Err(backend_error("hello missing required tool method cancel"));
        }
        instance.methods = hello.methods;
        Ok(Self {
            instance: Mutex::new(instance),
            capabilities: RuntimeCapabilities {
                branch_environment: hello.capabilities.branch_environment,
                ..Default::default()
            },
        })
    }

    fn lock(
        &self,
        cancel: &CancelHandle,
        deadline: Instant,
    ) -> Result<MutexGuard<'_, Instance>, BackendError> {
        loop {
            check_wait(cancel, deadline)?;
            match self.instance.try_lock() {
                Ok(instance) => return Ok(instance),
                Err(TryLockError::Poisoned(_)) => {
                    return Err(backend_error("instance lock poisoned"));
                }
                Err(TryLockError::WouldBlock) => thread::sleep(IO_INTERVAL),
            }
        }
    }

    fn transition(
        &self,
        op: &str,
        context: &BackendContext,
        state: Option<EnvironmentState>,
    ) -> Result<(), BackendError> {
        let cancel = CancelHandle::default();
        let deadline = Instant::now() + REQUEST_TIMEOUT;
        let mut instance = self.lock(&cancel, deadline)?;
        if let Err(mut error) = instance.call::<()>(&request(op, context), &cancel, deadline) {
            if instance.closed && matches!(op, "establish" | "branch") {
                instance.environments.insert(
                    context.environment_id.clone(),
                    EnvironmentState::PartiallyAvailable,
                );
                error.created_environment_id = Some(context.environment_id.clone());
            }
            return Err(error);
        }
        if let Some(state) = state {
            instance
                .environments
                .insert(context.environment_id.clone(), state);
        } else if op == "cleanup" {
            instance.environments.remove(&context.environment_id);
        }
        Ok(())
    }
}

impl Drop for ExecutableBackend {
    fn drop(&mut self) {
        let instance = self
            .instance
            .get_mut()
            .unwrap_or_else(|error| error.into_inner());
        if !instance.closed
            && let Err(error) = self.shutdown()
        {
            eprintln!("{error}");
        }
    }
}

fn request(op: &str, context: &BackendContext) -> Value {
    json!({"op": op, "environment": context.environment_id, "parent": context.parent_environment_id,
        "operation_id": context.operation_id, "tool_call_id": context.tool_call_id,
        "task_id": context.task_id, "generation": context.generation})
}

macro_rules! transition {
    ($plain:ident, $context:ident, $state:expr) => {
        fn $plain(&self, id: &str) -> Result<(), BackendError> {
            self.$context(&BackendContext::new(id))
        }
        fn $context(&self, context: &BackendContext) -> Result<(), BackendError> {
            self.transition(stringify!($plain), context, $state)
        }
    };
}

impl Backend for ExecutableBackend {
    fn capabilities(&self) -> RuntimeCapabilities {
        self.capabilities.clone()
    }

    transition!(
        establish,
        establish_with_context,
        Some(EnvironmentState::Available)
    );
    transition!(promote, promote_with_context, None);
    transition!(
        suspend,
        suspend_with_context,
        Some(EnvironmentState::Suspended)
    );
    transition!(
        resume,
        resume_with_context,
        Some(EnvironmentState::Available)
    );
    transition!(
        terminate,
        terminate_with_context,
        Some(EnvironmentState::Terminated)
    );
    transition!(cleanup, cleanup_with_context, None);

    fn branch(&self, parent: &str, child: &str) -> Result<(), BackendError> {
        self.branch_with_context(&BackendContext::new(child), parent, child)
    }
    fn branch_with_context(
        &self,
        context: &BackendContext,
        parent: &str,
        child: &str,
    ) -> Result<(), BackendError> {
        if !self.capabilities.branch_environment {
            return Err(backend_error("branch_environment is unavailable"));
        }
        self.transition(
            "branch",
            &BackendContext {
                environment_id: child.into(),
                parent_environment_id: Some(parent.into()),
                ..context.clone()
            },
            Some(EnvironmentState::Available),
        )
    }

    fn operate(
        &self,
        id: &str,
        command: &[String],
        cancel: &CancelHandle,
    ) -> Result<CommandOutput, BackendError> {
        self.operate_with_context(&BackendContext::new(id), command, cancel)?
            .single_output()
            .ok_or_else(|| backend_error("missing command output"))
    }
    fn operate_with_context(
        &self,
        context: &BackendContext,
        command: &[String],
        cancel: &CancelHandle,
    ) -> Result<BackendExecution, BackendError> {
        let deadline = Instant::now() + REQUEST_TIMEOUT;
        let mut instance = self.lock(cancel, deadline)?;
        if instance.environments.get(&context.environment_id) != Some(&EnvironmentState::Available)
        {
            return Err(backend_error("environment is not available"));
        }
        let mut request = request("operate", context);
        request["command"] = json!(command);
        let output = instance.call(&request, cancel, deadline)?;
        Ok(BackendExecution::single(self.backend_id(), output))
    }

    fn environment_operation(
        &self,
        context: &BackendContext,
        operation: &str,
        arguments: &Message,
        cancel: &CancelHandle,
    ) -> Result<Message, BackendError> {
        let method = "environment_operation";
        let mut meta = request(method, context);
        meta["operation"] = json!(operation);
        meta["arguments"] = arguments.meta.clone();
        let deadline = Instant::now() + REQUEST_TIMEOUT;
        let mut response = self.lock(cancel, deadline)?.tool(
            method,
            Message {
                meta,
                body: arguments.body.clone(),
            },
            cancel,
            deadline,
        )?;
        response.meta.as_object_mut().unwrap().remove("ok");
        if response.meta.get("value").is_none()
            && response.meta.get("exit_code").is_none()
            && response.meta.get("body_kind").is_none()
        {
            return Err(backend_error("missing environment operation result"));
        }
        Ok(response)
    }

    fn invoke_tool_with_context(
        &self,
        context: &BackendContext,
        tool_name: &str,
        arguments: &Value,
        cancel: &CancelHandle,
    ) -> Result<BackendExecution, BackendError> {
        let method = "invoke_tool";
        let mut meta = request(method, context);
        meta["tool"] = json!(tool_name);
        meta["arguments"] = arguments.clone();
        let deadline = Instant::now() + REQUEST_TIMEOUT;
        let response = self
            .lock(cancel, deadline)?
            .tool(method, meta.into(), cancel, deadline)?;
        if response.body.is_some() {
            return Err(backend_error("unexpected invoke_tool result body"));
        }
        let output =
            serde_json::from_value(response.meta["result"].clone()).map_err(backend_error)?;
        Ok(BackendExecution::single(self.backend_id(), output))
    }

    fn invoke_tool(
        &self,
        id: &str,
        tool_name: &str,
        arguments: &Value,
        cancel: &CancelHandle,
    ) -> Result<CommandOutput, BackendError> {
        self.invoke_tool_with_context(&BackendContext::new(id), tool_name, arguments, cancel)?
            .single_output()
            .ok_or_else(|| backend_error("missing tool output"))
    }

    fn cancel_tool_writers(&self, context: &BackendContext) -> Result<(), BackendError> {
        let instance = self.instance.lock().map_err(backend_error)?;
        if instance.closed || !instance.environments.contains_key(&context.environment_id) {
            return Err(backend_error(
                "cannot confirm writer drain for unavailable instance",
            ));
        }
        Ok(())
    }

    fn state(&self, id: &str) -> Option<EnvironmentState> {
        self.instance.lock().ok()?.environments.get(id).copied()
    }

    fn environments(&self) -> Vec<String> {
        self.instance
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .environments
            .keys()
            .cloned()
            .collect()
    }

    fn shutdown(&self) -> Result<(), BackendError> {
        let cancel = CancelHandle::default();
        let deadline = Instant::now() + EXIT_TIMEOUT;
        let mut instance = self.lock(&cancel, deadline)?;
        if instance.closed && instance.environments.is_empty() {
            return Ok(());
        }
        let mut failure = backend_error("shutdown incomplete");
        for (id, state) in instance.environments.clone() {
            let context = BackendContext::new(&id);
            if state != EnvironmentState::Terminated
                && let Err(error) =
                    instance.call::<()>(&request("terminate", &context), &cancel, deadline)
            {
                failure.add_cleanup_failure(&id, format!("terminate: {error}"));
            }
            match instance.call::<()>(&request("cleanup", &context), &cancel, deadline) {
                Ok(()) => {
                    instance.environments.remove(&id);
                }
                Err(error) => failure.add_cleanup_failure(&id, format!("cleanup: {error}")),
            }
        }
        if !instance.closed {
            let stopped = instance
                .call::<()>(&json!({"op":"shutdown"}), &cancel, deadline)
                .and_then(|()| instance.finish());
            if let Err(error) = stopped {
                failure.add_cleanup_failure("process", error.to_string());
                instance.abort();
            }
        }
        if failure.cleanup_failures.is_empty() {
            Ok(())
        } else {
            Err(failure)
        }
    }
}
