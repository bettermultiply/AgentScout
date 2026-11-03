//! 控制面侧的最小实现：接入会话（Registry 的位置）和请求客户端。
//!
//! 首版只有会话、登记和请求收发；能力声明、验证和目录留到后面的里程碑。
//! 发操作、收结果和事件由控制面的协调侧使用，不属于 Registry 的职责。

use std::collections::{HashMap, VecDeque};
use std::io;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::contract::{
    AdapterMessage, ControlMessage, Event, Operation, OperationRequest, OperationResult,
    RuntimeCapabilities,
};
use crate::transport;

/// 监听 Adapter 连入的控制面入口。
pub struct ControlServer {
    listener: UnixListener,
}

impl ControlServer {
    pub fn bind(path: &Path) -> io::Result<Self> {
        let _ = std::fs::remove_file(path);
        Ok(Self {
            listener: UnixListener::bind(path)?,
        })
    }

    pub fn accept(&self) -> io::Result<ControlSession> {
        let (stream, _) = self.listener.accept()?;
        Ok(ControlSession::start(stream))
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
}

/// 一条 Adapter 会话。
pub struct ControlSession {
    writer: Mutex<UnixStream>,
    next_request: AtomicU64,
    #[allow(clippy::type_complexity)]
    hello: Arc<(Mutex<Option<HelloInfo>>, Condvar)>,
    results: Arc<(Mutex<HashMap<String, OperationResult>>, Condvar)>,
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
    fn start(stream: UnixStream) -> Self {
        let mut reader = stream.try_clone().expect("clone control stream");
        let session = Self {
            writer: Mutex::new(stream),
            next_request: AtomicU64::new(1),
            hello: Arc::new((Mutex::new(None), Condvar::new())),
            results: Arc::new((Mutex::new(HashMap::new()), Condvar::new())),
            events: Arc::new((Mutex::new(VecDeque::new()), Condvar::new())),
        };

        let hello = session.hello.clone();
        let results = session.results.clone();
        let events = session.events.clone();
        thread::spawn(move || loop {
            let message: Option<AdapterMessage> =
                transport::read_message(&mut reader).unwrap_or_default();
            match message {
                Some(AdapterMessage::Hello {
                    protocol_version,
                    adapter_instance,
                    capabilities,
                }) => {
                    *hello.0.lock().unwrap() = Some(HelloInfo {
                        protocol_version,
                        adapter_instance,
                        capabilities,
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
                    events.0.lock().unwrap().push_back(event);
                    events.1.notify_all();
                }
                None => break,
            }
        });

        session
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
        let message = ControlMessage::Request(OperationRequest {
            request_id: request_id.to_string(),
            environment_id: environment_id.to_string(),
            operation,
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
