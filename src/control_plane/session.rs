//! 控制面侧的最小实现：接入会话（Registry 的位置）和请求客户端。
//!
//! 首版只有会话、登记和请求收发；能力声明、验证和目录留到后面的里程碑。
//! 发操作、收结果和事件由控制面的协调侧使用，不属于 Registry 的职责。

use std::collections::{HashMap, VecDeque};
use std::io;
use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::protocol::contract::{
    AdapterMessage, ControlMessage, Event, Operation, OperationRequest, OperationResult,
    RuntimeCapabilities,
};
use crate::protocol::transport;

/// 已完成结果的有界窗口；超出后淘汰最旧的结果。
const RESULT_WINDOW: usize = 256;
/// 未消费事件的有界窗口；事件是通知而不是历史记录。
const EVENT_WINDOW: usize = 4096;

#[derive(Default)]
struct ResultStore {
    entries: HashMap<String, OperationResult>,
    order: VecDeque<String>,
}

impl ResultStore {
    fn get(&self, request_id: &str) -> Option<OperationResult> {
        self.entries.get(request_id).cloned()
    }

    fn insert(&mut self, request_id: String, result: OperationResult) {
        if self.entries.insert(request_id.clone(), result).is_none() {
            self.order.push_back(request_id);
        }
        while self.order.len() > RESULT_WINDOW {
            if let Some(old) = self.order.pop_front() {
                self.entries.remove(&old);
            }
        }
    }
}

/// 监听 Adapter 连入的控制面入口。
pub struct ControlServer {
    listener: UnixListener,
}

impl ControlServer {
    pub fn bind(path: &Path) -> io::Result<Self> {
        match std::fs::symlink_metadata(path) {
            Ok(metadata) => {
                if !metadata.file_type().is_socket() {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        format!(
                            "refusing to remove non-socket path {}",
                            path.display()
                        ),
                    ));
                }
                std::fs::remove_file(path)?;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        Ok(Self {
            listener: UnixListener::bind(path)?,
        })
    }

    pub fn accept(&self) -> io::Result<ControlSession> {
        let (stream, _) = self.listener.accept()?;
        ControlSession::start(stream)
    }

    /// 允许启动阶段轮询 Adapter 是否已经退出，而不是无限等待连接。
    pub fn set_nonblocking(&self, nonblocking: bool) -> io::Result<()> {
        self.listener.set_nonblocking(nonblocking)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelloInfo {
    pub protocol_version: u32,
    pub adapter_instance: String,
    pub capabilities: RuntimeCapabilities,
    pub backend_ids: Vec<String>,
}

/// 一条 Adapter 会话。
pub struct ControlSession {
    writer: Mutex<UnixStream>,
    next_request: AtomicU64,
    #[allow(clippy::type_complexity)]
    hello: Arc<(Mutex<Option<HelloInfo>>, Condvar)>,
    results: Arc<(Mutex<ResultStore>, Condvar)>,
    events: Arc<(Mutex<VecDeque<Event>>, Condvar)>,
}

impl ControlSession {
    /// Close both socket halves, including the reader thread's cloned handle.
    pub fn shutdown(&self) -> io::Result<()> {
        self.writer
            .lock()
            .unwrap()
            .shutdown(std::net::Shutdown::Both)
    }
    fn start(stream: UnixStream) -> io::Result<Self> {
        let mut reader = stream.try_clone()?;
        let session = Self {
            writer: Mutex::new(stream),
            next_request: AtomicU64::new(1),
            hello: Arc::new((Mutex::new(None), Condvar::new())),
            results: Arc::new((Mutex::new(ResultStore::default()), Condvar::new())),
            events: Arc::new((Mutex::new(VecDeque::new()), Condvar::new())),
        };

        let hello = session.hello.clone();
        let results = session.results.clone();
        let events = session.events.clone();
        thread::spawn(move || {
            loop {
                let message = match transport::read_message::<_, AdapterMessage>(&mut reader) {
                    Ok(message) => message,
                    Err(error) => {
                        eprintln!(
                            "control plane: failed to read Adapter message: {error}"
                        );
                        break;
                    }
                };
                match message {
                    Some(AdapterMessage::Hello {
                        protocol_version,
                        adapter_instance,
                        capabilities,
                        backend_ids,
                    }) => {
                        *hello.0.lock().unwrap() = Some(HelloInfo {
                            protocol_version,
                            adapter_instance,
                            capabilities,
                            backend_ids,
                        });
                        hello.1.notify_all();
                    }
                    Some(AdapterMessage::Result(result)) => {
                        results
                            .0
                            .lock()
                            .unwrap()
                            .insert(result.request_id.clone(), result);
                        results.1.notify_all();
                    }
                    Some(AdapterMessage::Event(event)) => {
                        let mut queue = events.0.lock().unwrap();
                        queue.push_back(event);
                        while queue.len() > EVENT_WINDOW {
                            queue.pop_front();
                        }
                        drop(queue);
                        events.1.notify_all();
                    }
                    None => break,
                }
            }
        });

        Ok(session)
    }

    /// 等待 Adapter 的接入登记。
    pub fn wait_for_hello(&self, timeout: Duration) -> Option<HelloInfo> {
        let deadline = Instant::now() + timeout;
        let mut hello = self.hello.0.lock().unwrap();
        loop {
            if let Some(value) = hello.clone() {
                return Some(value);
            }
            let now = Instant::now();
            if now >= deadline {
                return None;
            }
            let (guard, _) = self.hello.1.wait_timeout(hello, deadline - now).unwrap();
            hello = guard;
        }
    }

    /// 发送一次请求，不等待结果（用于重试等场景）。
    pub fn send(
        &self,
        request_id: &str,
        environment_id: &str,
        operation: Operation,
    ) -> io::Result<()> {
        self.send_with_context(
            request_id,
            environment_id,
            None,
            Some(environment_id),
            operation,
        )
    }

    /// 发送带有任务和逻辑 Branch 上下文的请求。旧调用方继续使用
    /// [`send`]，因此不会改变既有控制协议的最小用法。
    pub fn send_with_context(
        &self,
        request_id: &str,
        environment_id: &str,
        task_id: Option<&str>,
        branch_id: Option<&str>,
        operation: Operation,
    ) -> io::Result<()> {
        let message = ControlMessage::Request(OperationRequest {
            request_id: request_id.to_string(),
            generation: 0,
            environment_id: environment_id.to_string(),
            operation,
            task_id: task_id.map(str::to_string),
            branch_id: branch_id.map(str::to_string),
        });
        let mut writer = self.writer.lock().unwrap();
        transport::write_message(&mut *writer, &message)
    }

    /// 用控制面分配的请求标识发请求并等待结果。
    pub fn call_with_id(
        &self,
        request_id: &str,
        environment_id: &str,
        operation: Operation,
    ) -> io::Result<OperationResult> {
        self.call_with_context(
            request_id,
            environment_id,
            None,
            Some(environment_id),
            operation,
        )
    }

    pub fn call_with_context(
        &self,
        request_id: &str,
        environment_id: &str,
        task_id: Option<&str>,
        branch_id: Option<&str>,
        operation: Operation,
    ) -> io::Result<OperationResult> {
        self.send_with_context(request_id, environment_id, task_id, branch_id, operation)?;
        self.wait_result(request_id, Duration::from_secs(30))
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("no result for request {request_id}"),
                )
            })
    }

    /// 发一次请求并等待结果，请求标识由会话自动分配。
    pub fn call(&self, environment_id: &str, operation: Operation) -> io::Result<OperationResult> {
        let request_id = format!("req-{}", self.next_request.fetch_add(1, Ordering::SeqCst));
        self.call_with_id(&request_id, environment_id, operation)
    }

    pub fn wait_result(&self, request_id: &str, timeout: Duration) -> Option<OperationResult> {
        let deadline = Instant::now() + timeout;
        let mut results = self.results.0.lock().unwrap();
        loop {
            if let Some(result) = results.get(request_id) {
                return Some(result.clone());
            }
            let now = Instant::now();
            if now >= deadline {
                return None;
            }
            let (guard, _) = self
                .results
                .1
                .wait_timeout(results, deadline - now)
                .unwrap();
            results = guard;
        }
    }

    /// 取走已经到达的事件。
    pub fn drain_events(&self) -> Vec<Event> {
        let mut events = self.events.0.lock().unwrap();
        events.drain(..).collect()
    }

    /// 等待下一条事件。
    pub fn wait_event(&self, timeout: Duration) -> Option<Event> {
        let deadline = Instant::now() + timeout;
        let mut events = self.events.0.lock().unwrap();
        loop {
            if let Some(event) = events.pop_front() {
                return Some(event);
            }
            let now = Instant::now();
            if now >= deadline {
                return None;
            }
            let (guard, _) = self.events.1.wait_timeout(events, deadline - now).unwrap();
            events = guard;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::contract::{
        AdapterMessage, EnvironmentState, EventPayload, Operation, OperationResult, ResultStatus,
    };
    use std::os::unix::net::UnixStream;

    fn session_pair() -> (ControlSession, UnixStream) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("control.sock");
        let server = ControlServer::bind(&path).unwrap();
        let client = UnixStream::connect(&path).unwrap();
        let session = server.accept().unwrap();
        (session, client)
    }

    fn result(request_id: &str) -> OperationResult {
        OperationResult {
            request_id: request_id.to_string(),
            environment_id: "env".to_string(),
            status: ResultStatus::Completed,
            environment_state: Some(EnvironmentState::Available),
            can_accept_operations: true,
            follow_up: Vec::new(),
            resources: Vec::new(),
            remaining: Vec::new(),
            backend_states: Vec::new(),
            outputs: Vec::new(),
            reason: None,
            output: None,
            filesystem: None,
        }
    }

    #[test]
    fn bind_refuses_to_remove_a_regular_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("control.sock");
        std::fs::write(&path, b"do not remove").unwrap();
        let error = match ControlServer::bind(&path) {
            Ok(_) => panic!("bind accepted a regular file"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&path).unwrap(), b"do not remove");
    }

    #[test]
    fn bind_replaces_a_stale_socket() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("control.sock");
        drop(UnixListener::bind(&path).unwrap());
        assert!(path.exists());
        ControlServer::bind(&path).unwrap();
    }

    #[test]
    fn results_are_bounded() {
        let (session, mut client) = session_pair();
        for index in 0..(RESULT_WINDOW + 8) {
            let request_id = format!("request-{index}");
            transport::write_message(
                &mut client,
                &AdapterMessage::Result(result(&request_id)),
            )
            .unwrap();
        }
        let last = format!("request-{}", RESULT_WINDOW + 7);
        assert!(session.wait_result(&last, Duration::from_secs(5)).is_some());
        let store = session.results.0.lock().unwrap();
        assert_eq!(store.entries.len(), RESULT_WINDOW);
        assert!(!store.entries.contains_key("request-0"));
        assert!(store.entries.contains_key(&last));
    }

    #[test]
    fn events_are_bounded() {
        let (session, mut client) = session_pair();
        let total = EVENT_WINDOW + 8;
        for index in 0..total {
            let event = Event {
                environment_id: "env".to_string(),
                request_id: Some(format!("event-{index}")),
                payload: EventPayload::OperationStarted {
                    operation: Operation::PromoteEnvironment,
                },
            };
            transport::write_message(&mut client, &AdapterMessage::Event(event)).unwrap();
        }
        let last = format!("event-{}", total - 1);
        for _ in 0..500 {
            let ready = {
                let events = session.events.0.lock().unwrap();
                events
                    .back()
                    .and_then(|event| event.request_id.as_deref())
                    == Some(last.as_str())
            };
            if ready {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let events = session.events.0.lock().unwrap();
        assert_eq!(events.len(), EVENT_WINDOW);
        assert_eq!(
            events.front().and_then(|event| event.request_id.as_deref()),
            Some("event-8")
        );
    }
}
