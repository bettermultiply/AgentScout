//! Gateway 的 Runtime 接入：握手、请求关联、结果及事件送达。
//! 不拥有 Runtime 状态或 Registry 的能力验证责任。

use std::collections::{HashMap, VecDeque};
use std::io;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::AtomicBool;
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

/// Typed Runtime operation delivery used by coordination, independent of transport.
pub trait Runtime: Sync {
    fn call(&self, environment_id: &str, operation: Operation) -> io::Result<OperationResult>;
    fn establish(&self) -> io::Result<OperationResult> {
        self.call("", Operation::EstablishEnvironment)
    }
    fn branch(
        &self,
        parent_environment_id: &str,
        agent: Option<crate::protocol::contract::AgentLaunch>,
    ) -> io::Result<OperationResult> {
        self.call(
            parent_environment_id,
            Operation::BranchEnvironment { agent },
        )
    }
    fn drain_events(&self) -> Vec<Event>;
}

impl Runtime for ControlSession {
    fn call(&self, environment_id: &str, operation: Operation) -> io::Result<OperationResult> {
        ControlSession::call(self, environment_id, operation)
    }

    fn establish(&self) -> io::Result<OperationResult> {
        ControlSession::call(self, "", Operation::EstablishEnvironment)
    }

    fn branch(
        &self,
        parent_environment_id: &str,
        agent: Option<crate::protocol::contract::AgentLaunch>,
    ) -> io::Result<OperationResult> {
        ControlSession::call(
            self,
            parent_environment_id,
            Operation::BranchEnvironment { agent },
        )
    }

    fn drain_events(&self) -> Vec<Event> {
        ControlSession::drain_events(self)
    }
}

impl Drop for ControlSession {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

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

/// 监听 Adapter 连入的 control plane 入口。
pub struct ControlServer {
    listener: TcpListener,
}

impl ControlServer {
    pub fn bind(address: &str) -> io::Result<Self> {
        let address: SocketAddr = address.parse().map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid control address {address:?}: {error}"),
            )
        })?;
        Ok(Self {
            listener: TcpListener::bind(address)?,
        })
    }

    pub fn accept(&self) -> io::Result<ControlSession> {
        let (stream, _) = self.listener.accept()?;
        ControlSession::start(stream)
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
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
    writer: Mutex<TcpStream>,
    next_request: AtomicU64,
    #[allow(clippy::type_complexity)]
    hello: Arc<(Mutex<Option<HelloInfo>>, Condvar)>,
    results: Arc<(Mutex<ResultStore>, Condvar)>,
    events: Arc<(Mutex<VecDeque<Event>>, Condvar)>,
    /// Set when the Adapter socket reaches EOF or its reader fails.  Waiting
    /// callers must not sleep until their full request timeout after the
    /// Runtime has already gone away.
    disconnected: Arc<AtomicBool>,
    event_waker: Arc<Mutex<Option<Box<dyn Fn() + Send + Sync>>>>,
    reader: Mutex<Option<thread::JoinHandle<()>>>,
}

impl ControlSession {
    pub fn set_event_waker(&self, wake: impl Fn() + Send + Sync + 'static) {
        *self.event_waker.lock().unwrap() = Some(Box::new(wake));
    }
    /// Close both socket halves, including the reader thread's cloned handle.
    pub fn shutdown(&self) -> io::Result<()> {
        let result = self
            .writer
            .lock()
            .unwrap()
            .shutdown(std::net::Shutdown::Both);
        let joined = self
            .reader
            .lock()
            .unwrap()
            .take()
            .map(|reader| reader.join());
        if joined.is_some_and(|result| result.is_err()) {
            return Err(io::Error::other("Runtime reader panicked"));
        }
        match result {
            Err(error) if error.kind() == io::ErrorKind::NotConnected => Ok(()),
            result => result,
        }
    }
    fn start(stream: TcpStream) -> io::Result<Self> {
        let mut reader = stream.try_clone()?;
        let session = Self {
            writer: Mutex::new(stream),
            next_request: AtomicU64::new(1),
            hello: Arc::new((Mutex::new(None), Condvar::new())),
            results: Arc::new((Mutex::new(ResultStore::default()), Condvar::new())),
            events: Arc::new((Mutex::new(VecDeque::new()), Condvar::new())),
            disconnected: Arc::new(AtomicBool::new(false)),
            event_waker: Arc::new(Mutex::new(None)),
            reader: Mutex::new(None),
        };

        let hello = session.hello.clone();
        let results = session.results.clone();
        let events = session.events.clone();
        let disconnected = session.disconnected.clone();
        let event_waker = session.event_waker.clone();
        let reader = thread::Builder::new()
            .name("runtime-reader".into())
            .spawn(move || {
                loop {
                    let message = match transport::read_message::<_, AdapterMessage>(&mut reader) {
                        Ok(message) => message,
                        Err(error) => {
                            eprintln!("control plane: failed to read Adapter message: {error}");
                            disconnected.store(true, Ordering::SeqCst);
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
                            if let Some(wake) = event_waker.lock().unwrap().as_ref() {
                                wake();
                            }
                        }
                        None => {
                            disconnected.store(true, Ordering::SeqCst);
                            break;
                        }
                    }
                }
                // Wake every wait class.  The predicates below re-check the
                // disconnect flag and return promptly with the appropriate
                // absence/error instead of waiting for a 30-second request timer.
                hello.1.notify_all();
                results.1.notify_all();
                events.1.notify_all();
            })?;
        *session.reader.lock().unwrap() = Some(reader);

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
            if self.disconnected.load(Ordering::SeqCst) {
                return None;
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
        let message = ControlMessage::Request(OperationRequest {
            request_id: request_id.to_string(),
            generation: 0,
            environment_id: environment_id.to_string(),
            operation,
            task_id: None,
        });
        let mut writer = self.writer.lock().unwrap();
        transport::write_message(&mut *writer, &message)
    }

    /// 用 control plane 分配的请求标识发请求并等待结果。
    pub fn call_with_id(
        &self,
        request_id: &str,
        environment_id: &str,
        operation: Operation,
    ) -> io::Result<OperationResult> {
        self.send(request_id, environment_id, operation)?;
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
                return Some(result);
            }
            if self.disconnected.load(Ordering::SeqCst) {
                return None;
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
            if self.disconnected.load(Ordering::SeqCst) {
                return None;
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
    use std::net::TcpStream;

    fn session_pair() -> (ControlSession, TcpStream) {
        let server = ControlServer::bind("127.0.0.1:0").unwrap();
        let address = server.listener.local_addr().unwrap();
        let client = TcpStream::connect(address).unwrap();
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
    fn results_are_bounded() {
        let (session, mut client) = session_pair();
        for index in 0..(RESULT_WINDOW + 8) {
            let request_id = format!("request-{index}");
            transport::write_message(&mut client, &AdapterMessage::Result(result(&request_id)))
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
                events.back().and_then(|event| event.request_id.as_deref()) == Some(last.as_str())
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
