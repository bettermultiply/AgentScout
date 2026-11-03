//! Adapter 进程：接入控制面会话，按环境串行执行操作，上报结果和事件。
//!
//! 首版只做一条连接、同步完成、一个状态变化事件类型；能力声明、验证和
//! 多 Adapter 管理不在这一版。

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::OpenOptions;
use std::io;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use crate::backend::{Backend, CancelHandle};
use crate::contract::{
    AdapterMessage, AgentLaunch, Content, ControlMessage, EnvironmentState, Event, EventPayload,
    FollowUp, Operation, OperationRequest, OperationResult, PROTOCOL_VERSION, ResourceNote,
    ResultStatus,
};
use crate::transport;

pub struct Adapter {
    pub adapter_instance: String,
    pub backend: Arc<dyn Backend>,
}

impl Adapter {
    pub fn run(self, socket_path: &Path) -> io::Result<()> {
        let stream = std::os::unix::net::UnixStream::connect(socket_path)?;
        let mut reader = stream.try_clone()?;
        let (out, out_rx) = mpsc::channel::<AdapterMessage>();

        let mut writer = stream;
        thread::spawn(move || {
            for message in out_rx {
                if transport::write_message(&mut writer, &message).is_err() {
                    break;
                }
            }
        });

        let _ = out.send(AdapterMessage::Hello {
            protocol_version: PROTOCOL_VERSION,
            adapter_instance: self.adapter_instance,
            capabilities: self.backend.capabilities(),
        });

        let shared = Arc::new(Shared {
            backend: self.backend,
            out,
            queues: Mutex::new(HashMap::new()),
            seen: Mutex::new(HashSet::new()),
            known: Mutex::new(HashMap::new()),
            agent: Mutex::new(None),
        });
        shared.clone().start_state_watcher();

        loop {
            match transport::read_message::<_, ControlMessage>(&mut reader) {
                Ok(Some(ControlMessage::Request(request))) => shared.dispatch(request),
                Ok(None) => {
                    eprintln!("adapter: control plane disconnected");
                    break;
                }
                Err(error) => {
                    eprintln!("adapter: read failed: {error}");
                    break;
                }
            }
        }
        shared.stop_agent();
        Ok(())
    }
}

struct Shared {
    backend: Arc<dyn Backend>,
    out: Sender<AdapterMessage>,
    queues: Mutex<HashMap<String, Arc<Queue>>>,
    /// 已经受理过的请求标识：重发同一标识按重试处理，不重复执行。
    seen: Mutex<HashSet<String>>,
    /// Adapter 记录的每个环境最近一次报告的状态，用于发现意外变化。
    known: Mutex<HashMap<String, EnvironmentState>>,
    /// 由 Runtime 启动的 Agent 接入侧进程。一个 run 只有一个，服务所有环境。
    agent: Mutex<Option<Child>>,
}

impl Shared {
    fn dispatch(self: &Arc<Self>, request: OperationRequest) {
        {
            let mut seen = self.seen.lock().unwrap();
            if !seen.insert(request.request_id.clone()) {
                return;
            }
        }
        let queue = self.queue_for(&request.environment_id);
        let preempt = matches!(
            &request.operation,
            Operation::SuspendEnvironment | Operation::TerminateEnvironment
        );
        queue.enqueue(request, preempt);
    }

    fn queue_for(self: &Arc<Self>, environment_id: &str) -> Arc<Queue> {
        let mut queues = self.queues.lock().unwrap();
        queues
            .entry(environment_id.to_string())
            .or_insert_with(|| Queue::start(self.clone(), environment_id.to_string()))
            .clone()
    }

    fn set_known(&self, environment_id: &str, state: EnvironmentState) {
        self.known
            .lock()
            .unwrap()
            .insert(environment_id.to_string(), state);
    }

    fn execute(
        &self,
        environment_id: &str,
        request: &OperationRequest,
        cancel: &CancelHandle,
    ) -> OperationResult {
        let state = self.backend.state(environment_id);
        match &request.operation {
            Operation::EstablishEnvironment => match state {
                Some(_) => {
                    let mut result = result(request, ResultStatus::Failed, state, false);
                    result.reason = Some("environment already exists".to_string());
                    result
                }
                None => match self.backend.establish(environment_id) {
                    Ok(()) => {
                        let mut result = result(
                            request,
                            ResultStatus::Completed,
                            Some(EnvironmentState::Available),
                            true,
                        );
                        result.resources = vec![
                            ResourceNote {
                                content: Content::Process,
                                note: "environment process started".to_string(),
                            },
                            ResourceNote {
                                content: Content::Workspace,
                                note: "environment workspace created".to_string(),
                            },
                        ];
                        result
                    }
                    Err(error) => {
                        let mut result = result(request, ResultStatus::Failed, None, false);
                        result.reason = Some(error.message);
                        result
                    }
                },
            },
            Operation::OperateEnvironment { command, agent } => match state {
                Some(EnvironmentState::Available) => {
                    if let Some(agent) = agent {
                        if let Err(error) = self.launch_agent(agent) {
                            let mut result = result(request, ResultStatus::Failed, state, false);
                            result.reason = Some(format!("agent launch failed: {error}"));
                            return result;
                        }
                        return result(
                            request,
                            ResultStatus::Completed,
                            Some(EnvironmentState::Available),
                            true,
                        );
                    }
                    match self.backend.operate(environment_id, command, cancel) {
                        Ok(output) => {
                            let mut result = result(
                                request,
                                ResultStatus::Completed,
                                Some(EnvironmentState::Available),
                                true,
                            );
                            result.output = Some(output);
                            result
                        }
                        Err(error) if error.cancelled => {
                            let mut result =
                                result(request, ResultStatus::PartiallyCompleted, state, false);
                            result.reason = Some(error.message);
                            result
                        }
                        Err(error) => {
                            let mut result = result(request, ResultStatus::Failed, state, true);
                            result.reason = Some(error.message);
                            result
                        }
                    }
                }
                Some(other) => {
                    let mut result =
                        result(request, ResultStatus::NotApplicable, Some(other), false);
                    result.reason = Some("environment cannot accept operations".to_string());
                    result
                }
                None => {
                    let mut result = result(request, ResultStatus::Failed, None, false);
                    result.reason = Some("unknown environment".to_string());
                    result
                }
            },
            Operation::BranchEnvironment {
                child_environment_id,
            } => match state {
                Some(EnvironmentState::Available) | Some(EnvironmentState::Suspended) => {
                    if self.backend.state(child_environment_id).is_some() {
                        let mut result = result(request, ResultStatus::Failed, state, true);
                        result.reason = Some("child environment already exists".to_string());
                        result
                    } else {
                        match self.backend.branch(environment_id, child_environment_id) {
                            Ok(()) => {
                                let mut result =
                                    result(request, ResultStatus::Completed, state, true);
                                result.resources = vec![ResourceNote {
                                    content: Content::Process,
                                    note: format!("environment {child_environment_id} created"),
                                }];
                                result
                            }
                            Err(error) => {
                                let mut result = result(request, ResultStatus::Failed, state, true);
                                result.reason = Some(error.message);
                                result
                            }
                        }
                    }
                }
                Some(other) => {
                    let mut result =
                        result(request, ResultStatus::NotApplicable, Some(other), false);
                    result.reason = Some("environment cannot be branched".to_string());
                    result
                }
                None => {
                    let mut result = result(request, ResultStatus::Failed, None, false);
                    result.reason = Some("unknown environment".to_string());
                    result
                }
            },
            Operation::PromoteEnvironment => match state {
                Some(EnvironmentState::Available) | Some(EnvironmentState::Suspended) => {
                    match self.backend.promote(environment_id) {
                        Ok(()) => result(request, ResultStatus::Completed, state, true),
                        Err(error) => {
                            let mut result = result(request, ResultStatus::Failed, state, true);
                            result.reason = Some(error.message);
                            result
                        }
                    }
                }
                Some(other) => {
                    let mut result =
                        result(request, ResultStatus::NotApplicable, Some(other), false);
                    result.reason = Some("environment cannot be promoted".to_string());
                    result
                }
                None => {
                    let mut result = result(request, ResultStatus::Failed, None, false);
                    result.reason = Some("unknown environment".to_string());
                    result
                }
            },
            Operation::SuspendEnvironment => match state {
                Some(EnvironmentState::Available) | Some(EnvironmentState::Suspended) => {
                    match self.backend.suspend(environment_id) {
                        Ok(()) => {
                            let mut result = result(
                                request,
                                ResultStatus::Completed,
                                Some(EnvironmentState::Suspended),
                                true,
                            );
                            result.follow_up = vec![FollowUp::Resume];
                            result
                        }
                        Err(error) => {
                            let mut result = result(request, ResultStatus::Failed, state, true);
                            result.reason = Some(error.message);
                            result
                        }
                    }
                }
                Some(other) => {
                    let mut result =
                        result(request, ResultStatus::NotApplicable, Some(other), false);
                    result.reason = Some("environment cannot be suspended".to_string());
                    result
                }
                None => {
                    let mut result = result(request, ResultStatus::Failed, None, false);
                    result.reason = Some("unknown environment".to_string());
                    result
                }
            },
            Operation::ResumeEnvironment => match state {
                Some(EnvironmentState::Suspended) | Some(EnvironmentState::Available) => {
                    match self.backend.resume(environment_id) {
                        Ok(()) => result(
                            request,
                            ResultStatus::Completed,
                            Some(EnvironmentState::Available),
                            true,
                        ),
                        Err(error) => {
                            let mut result = result(request, ResultStatus::Failed, state, true);
                            result.reason = Some(error.message);
                            result
                        }
                    }
                }
                Some(other) => {
                    let mut result =
                        result(request, ResultStatus::NotApplicable, Some(other), false);
                    result.reason = Some("environment cannot be resumed".to_string());
                    result
                }
                None => {
                    let mut result = result(request, ResultStatus::Failed, None, false);
                    result.reason = Some("unknown environment".to_string());
                    result
                }
            },
            Operation::TerminateEnvironment => match state {
                Some(EnvironmentState::Terminated) => {
                    let mut result = result(
                        request,
                        ResultStatus::Completed,
                        Some(EnvironmentState::Terminated),
                        false,
                    );
                    result.follow_up = vec![FollowUp::Cleanup];
                    result
                }
                Some(_) => match self.backend.terminate(environment_id) {
                    Ok(()) => {
                        let mut result = result(
                            request,
                            ResultStatus::Completed,
                            Some(EnvironmentState::Terminated),
                            false,
                        );
                        result.follow_up = vec![FollowUp::Cleanup];
                        result
                    }
                    Err(error) => {
                        let mut result = result(request, ResultStatus::Failed, state, false);
                        result.reason = Some(error.message);
                        result
                    }
                },
                None => {
                    let mut result = result(request, ResultStatus::Failed, None, false);
                    result.reason = Some("unknown environment".to_string());
                    result
                }
            },
            Operation::CleanupEnvironment => match state {
                Some(_) => match self.backend.cleanup(environment_id) {
                    Ok(()) => result(
                        request,
                        ResultStatus::Completed,
                        Some(EnvironmentState::Cleaned),
                        false,
                    ),
                    Err(error) => {
                        let mut result =
                            result(request, ResultStatus::PartiallyCompleted, state, false);
                        result.reason = Some(error.message);
                        result
                    }
                },
                None => {
                    let mut result = result(request, ResultStatus::NotApplicable, None, false);
                    result.reason = Some("unknown environment".to_string());
                    result
                }
            },
            Operation::ToolContext { tool_name } => {
                if !self
                    .backend
                    .capabilities()
                    .tools
                    .iter()
                    .any(|tool| tool.name == *tool_name)
                {
                    let mut result = result(request, ResultStatus::Unsupported, state, false);
                    result.reason = Some(format!("runtime does not declare tool {tool_name}"));
                    return result;
                }
                match self.backend.tool_context(environment_id, tool_name, cancel) {
                    Ok(output) => {
                        let mut result = result(
                            request,
                            ResultStatus::Completed,
                            Some(EnvironmentState::Available),
                            true,
                        );
                        result.output = Some(output);
                        result
                    }
                    Err(error) => {
                        let mut result = result(request, ResultStatus::Failed, state, true);
                        result.reason = Some(error.message);
                        result
                    }
                }
            }
            Operation::InvokeTool {
                tool_name,
                arguments,
            } => {
                if !self
                    .backend
                    .capabilities()
                    .tools
                    .iter()
                    .any(|tool| tool.name == *tool_name)
                {
                    let mut result = result(request, ResultStatus::Unsupported, state, false);
                    result.reason = Some(format!("runtime does not declare tool {tool_name}"));
                    return result;
                }
                match self
                    .backend
                    .invoke_tool(environment_id, tool_name, arguments, cancel)
                {
                    Ok(output) => {
                        let mut result = result(
                            request,
                            ResultStatus::Completed,
                            Some(EnvironmentState::Available),
                            true,
                        );
                        result.output = Some(output);
                        result
                    }
                    Err(error) => {
                        let mut result = result(request, ResultStatus::Failed, state, true);
                        result.reason = Some(error.message);
                        result
                    }
                }
            }
        }
    }

    /// 启动 Agent 接入侧。控制面决定用哪个产品，Runtime 负责启动。
    /// 一个 run 只有一个接入侧进程，它服务所有环境。
    fn launch_agent(&self, agent: &AgentLaunch) -> io::Result<()> {
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&agent.log)?;
        let mut command = Command::new(&agent.command);
        command
            .args(&agent.args)
            .current_dir(&agent.cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::from(log.try_clone()?))
            .stderr(Stdio::from(log))
            .envs(&agent.env);
        let mut slot = self.agent.lock().unwrap();
        if slot.is_some() {
            return Ok(());
        }
        *slot = Some(command.spawn()?);
        Ok(())
    }

    /// 停止 Agent 接入侧。它属于整个 run，不属于单个环境。
    fn stop_agent(&self) {
        if let Some(mut child) = self.agent.lock().unwrap().take() {
            eprintln!("adapter: stopping agent pid {}", child.id());
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn start_state_watcher(self: Arc<Self>) {
        thread::spawn(move || {
            loop {
                thread::sleep(Duration::from_millis(100));
                for environment_id in self.backend.environments() {
                    let Some(current) = self.backend.state(&environment_id) else {
                        continue;
                    };
                    let previous = self.known.lock().unwrap().get(&environment_id).copied();
                    match previous {
                        None => {
                            self.known.lock().unwrap().insert(environment_id, current);
                        }
                        Some(previous) if previous == current => {}
                        Some(previous) => {
                            self.known
                                .lock()
                                .unwrap()
                                .insert(environment_id.clone(), current);
                            let reason = if current == EnvironmentState::Terminated {
                                "environment stopped unexpectedly"
                            } else {
                                "environment state changed"
                            };
                            let event = Event {
                                environment_id,
                                request_id: None,
                                payload: EventPayload::EnvironmentStateChanged {
                                    previous: Some(previous),
                                    current,
                                    reason: reason.to_string(),
                                },
                            };
                            let _ = self.out.send(AdapterMessage::Event(event));
                        }
                    }
                }
            }
        });
    }
}

fn result(
    request: &OperationRequest,
    status: ResultStatus,
    environment_state: Option<EnvironmentState>,
    can_accept_operations: bool,
) -> OperationResult {
    OperationResult {
        request_id: request.request_id.clone(),
        environment_id: request.environment_id.clone(),
        status,
        environment_state,
        can_accept_operations,
        follow_up: Vec::new(),
        resources: Vec::new(),
        remaining: Vec::new(),
        reason: None,
        output: None,
    }
}

#[derive(Default)]
struct Pending {
    preempt: VecDeque<OperationRequest>,
    normal: VecDeque<OperationRequest>,
}

struct Queue {
    pending: Mutex<Pending>,
    signal: Condvar,
    cancel: CancelHandle,
}

impl Queue {
    fn start(shared: Arc<Shared>, environment_id: String) -> Arc<Self> {
        let queue = Arc::new(Self {
            pending: Mutex::new(Pending::default()),
            signal: Condvar::new(),
            cancel: CancelHandle::default(),
        });
        let worker = queue.clone();
        thread::spawn(move || worker.run(shared, environment_id));
        queue
    }

    /// suspend/terminate 插到队首并打断在飞操作；其他操作按到达顺序排队。
    fn enqueue(&self, request: OperationRequest, preempt: bool) {
        {
            let mut pending = self.pending.lock().unwrap();
            if preempt {
                pending.preempt.push_back(request);
            } else {
                pending.normal.push_back(request);
            }
        }
        if preempt {
            self.cancel.cancel();
        }
        self.signal.notify_one();
    }

    fn run(self: Arc<Self>, shared: Arc<Shared>, environment_id: String) {
        loop {
            let request = {
                let mut pending = self.pending.lock().unwrap();
                loop {
                    if let Some(request) = pending.preempt.pop_front() {
                        break request;
                    }
                    if let Some(request) = pending.normal.pop_front() {
                        break request;
                    }
                    pending = self.signal.wait(pending).unwrap();
                }
            };
            self.cancel.reset();
            let result = shared.execute(&environment_id, &request, &self.cancel);
            if let Some(state) = result.environment_state {
                shared.set_known(&request.environment_id, state);
                if let Operation::BranchEnvironment {
                    child_environment_id,
                } = &request.operation
                    && result.status == ResultStatus::Completed
                {
                    shared.set_known(child_environment_id, EnvironmentState::Available);
                }
            }
            let _ = shared.out.send(AdapterMessage::Result(result));
        }
    }
}
