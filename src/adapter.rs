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

use crate::backend::{Backend, BackendContext, BackendError, BackendExecution, CancelHandle};
use crate::contract::{
    AdapterMessage, AgentLaunch, Content, ControlMessage, EnvironmentState, Event, EventPayload,
    FollowUp, Operation, OperationRequest, OperationResult, PROTOCOL_VERSION, ResourceNote,
    ResultStatus,
};
use crate::transport;

pub struct Adapter {
    pub adapter_instance: String,
    pub runtime: Arc<dyn Backend>,
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
            capabilities: self.runtime.capabilities(),
            backend_ids: self.runtime.backend_ids(),
        });

        let shared = Arc::new(Shared {
            runtime: self.runtime,
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
        shared.stop_agents();
        Ok(())
    }
}

struct Shared {
    runtime: Arc<dyn Backend>,
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
        let state = self.runtime.state(environment_id);
        let context = BackendContext {
            task_id: request.task_id.clone(),
            branch_id: request
                .branch_id
                .clone()
                .or_else(|| Some(request.environment_id.clone())),
            environment_id: environment_id.to_string(),
            parent_environment_id: None,
        };

        match &request.operation {
            Operation::EstablishEnvironment => {
                if state.is_some() {
                    return self.reasoned_failure(request, state, "environment already exists");
                }
                match self.runtime.establish_with_context(&context) {
                    Ok(()) => {
                        let actual = self.runtime.state(environment_id);
                        let mut result = self.success(request, actual, can_accept(actual));
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
                    Err(error) => self.backend_error(request, environment_id, error),
                }
            }
            Operation::OperateEnvironment { command, agent } => {
                match state {
                    Some(EnvironmentState::Available) => {}
                    Some(other) => {
                        return self.not_applicable(
                            request,
                            Some(other),
                            "environment cannot accept operations",
                        );
                    }
                    None => return self.unknown_environment(request),
                }

                if let Some(agent) = agent {
                    let launch = if self.runtime.manages_agent_process() {
                        self.runtime
                            .launch_agent(&context, agent)
                            .map_err(|error| error.to_string())
                    } else {
                        self.launch_agent(agent).map_err(|error| error.to_string())
                    };
                    return match launch {
                        Ok(()) => {
                            let actual = self.runtime.state(environment_id);
                            self.success(request, actual, can_accept(actual))
                        }
                        Err(error) => self.reasoned_failure(
                            request,
                            state,
                            format!("agent launch failed: {error}"),
                        ),
                    };
                }

                match self.runtime.operate_with_context(&context, command, cancel) {
                    Ok(execution) => {
                        self.execution_result(request, environment_id, execution, false)
                    }
                    Err(error) => self.backend_error(request, environment_id, error),
                }
            }
            Operation::BranchEnvironment {
                child_environment_id,
            } => {
                match state {
                    Some(EnvironmentState::Available) | Some(EnvironmentState::Suspended) => {}
                    Some(other) => {
                        return self.not_applicable(
                            request,
                            Some(other),
                            "environment cannot be branched",
                        );
                    }
                    None => return self.unknown_environment(request),
                }
                if self.runtime.state(child_environment_id).is_some() {
                    return self.reasoned_failure(
                        request,
                        state,
                        "child environment already exists".to_string(),
                    );
                }
                match self.runtime.branch_with_context(
                    &BackendContext {
                        environment_id: environment_id.to_string(),
                        parent_environment_id: Some(environment_id.to_string()),
                        ..context.clone()
                    },
                    environment_id,
                    child_environment_id,
                ) {
                    Ok(()) => {
                        let actual = self.runtime.state(environment_id);
                        let mut result = self.success(request, actual, can_accept(actual));
                        result.resources = vec![ResourceNote {
                            content: Content::Process,
                            note: format!("environment {child_environment_id} created"),
                        }];
                        result
                    }
                    Err(error) => self.backend_error(request, environment_id, error),
                }
            }
            Operation::PromoteEnvironment => {
                if !matches!(
                    state,
                    Some(EnvironmentState::Available | EnvironmentState::Suspended)
                ) {
                    return self.not_applicable(request, state, "environment cannot be promoted");
                }
                match self.runtime.promote_with_context(&context) {
                    Ok(()) => {
                        let actual = self.runtime.state(environment_id);
                        self.success(request, actual, can_accept(actual))
                    }
                    Err(error) => self.backend_error(request, environment_id, error),
                }
            }
            Operation::SuspendEnvironment => {
                if !matches!(
                    state,
                    Some(EnvironmentState::Available | EnvironmentState::Suspended)
                ) {
                    return self.not_applicable(request, state, "environment cannot be suspended");
                }
                match self.runtime.suspend_with_context(&context) {
                    Ok(()) => {
                        let mut result =
                            self.success(request, self.runtime.state(environment_id), true);
                        result.follow_up = vec![FollowUp::Resume];
                        result
                    }
                    Err(error) => self.backend_error(request, environment_id, error),
                }
            }
            Operation::ResumeEnvironment => {
                if !matches!(
                    state,
                    Some(EnvironmentState::Suspended | EnvironmentState::Available)
                ) {
                    return self.not_applicable(request, state, "environment cannot be resumed");
                }
                match self.runtime.resume_with_context(&context) {
                    Ok(()) => self.success(request, self.runtime.state(environment_id), true),
                    Err(error) => self.backend_error(request, environment_id, error),
                }
            }
            Operation::TerminateEnvironment => {
                if state == Some(EnvironmentState::Terminated) {
                    let mut result = self.success(request, state, false);
                    result.follow_up = vec![FollowUp::Cleanup];
                    return result;
                }
                if state.is_none() {
                    return self.unknown_environment(request);
                }
                match self.runtime.terminate_with_context(&context) {
                    Ok(()) => {
                        let mut result =
                            self.success(request, self.runtime.state(environment_id), false);
                        result.follow_up = vec![FollowUp::Cleanup];
                        result
                    }
                    Err(error) => self.backend_error(request, environment_id, error),
                }
            }
            Operation::CleanupEnvironment => {
                if state.is_none() {
                    return self.not_applicable(request, None, "unknown environment");
                }
                match self.runtime.cleanup_with_context(&context) {
                    Ok(()) => self.success(request, self.runtime.state(environment_id), false),
                    Err(error) => self.backend_error(request, environment_id, error),
                }
            }
            Operation::ToolContext { tool_name } => {
                if !self.tool_declared(tool_name) {
                    return self.unsupported(
                        request,
                        state,
                        format!("runtime does not declare tool {tool_name}"),
                    );
                }
                match self
                    .runtime
                    .tool_context_with_context(&context, tool_name, cancel)
                {
                    Ok(execution) => {
                        self.execution_result(request, environment_id, execution, true)
                    }
                    Err(error) => self.backend_error(request, environment_id, error),
                }
            }
            Operation::InvokeTool {
                tool_name,
                arguments,
            } => {
                if !self.tool_declared(tool_name) {
                    return self.unsupported(
                        request,
                        state,
                        format!("runtime does not declare tool {tool_name}"),
                    );
                }
                match self
                    .runtime
                    .invoke_tool_with_context(&context, tool_name, arguments, cancel)
                {
                    Ok(execution) => {
                        self.execution_result(request, environment_id, execution, true)
                    }
                    Err(error) => self.backend_error(request, environment_id, error),
                }
            }
        }
    }
    fn tool_declared(&self, tool_name: &str) -> bool {
        self.runtime
            .capabilities()
            .tools
            .iter()
            .any(|tool| tool.name == tool_name)
    }

    fn success(
        &self,
        request: &OperationRequest,
        state: Option<EnvironmentState>,
        can_accept_operations: bool,
    ) -> OperationResult {
        // 环境实际停留在 PartiallyAvailable 时不能报告 Completed：部分
        // Backend 尚未就绪，控制面据此得到 PartiallyCompleted 与原因。
        let status = if state == Some(EnvironmentState::PartiallyAvailable) {
            ResultStatus::PartiallyCompleted
        } else {
            ResultStatus::Completed
        };
        // 收口：只要环境停留在 PartiallyAvailable，就不能接受后续普通
        // 操作，无论调用方传入 true 还是 false。
        let can_accept_operations =
            can_accept_operations && state != Some(EnvironmentState::PartiallyAvailable);
        let mut result = result(request, status, state, can_accept_operations);
        if status == ResultStatus::PartiallyCompleted {
            result.reason = Some(
                "environment is partially available; not all Backends are consistently available"
                    .to_string(),
            );
        }
        result.backend_states = self.runtime.backend_states(&request.environment_id);
        result
    }

    fn execution_result(
        &self,
        request: &OperationRequest,
        environment_id: &str,
        execution: BackendExecution,
        require_single_output: bool,
    ) -> OperationResult {
        let state = self.runtime.state(environment_id);
        let mut result = self.success(request, state, can_accept(state));
        result.output = execution.single_output();
        result.outputs = execution.outputs;
        if require_single_output && result.output.is_none() {
            result.status = ResultStatus::PartiallyCompleted;
            result.reason = Some(
                "multiple or missing Backend outputs; configure an explicit output_backend"
                    .to_string(),
            );
        }
        result
    }

    fn backend_error(
        &self,
        request: &OperationRequest,
        environment_id: &str,
        error: BackendError,
    ) -> OperationResult {
        let state = self.runtime.state(environment_id);
        let status = if error.is_partial() {
            ResultStatus::PartiallyCompleted
        } else {
            ResultStatus::Failed
        };
        let mut result = result(request, status, state, can_accept(state));
        result.reason = Some(error.to_string());
        result.backend_states = self.runtime.backend_states(environment_id);
        result.outputs = error.outputs;
        result
            .resources
            .extend(error.completed.into_iter().map(|backend_id| ResourceNote {
                content: Content::Resource,
                note: format!("Backend {backend_id} completed"),
            }));
        result
            .remaining
            .extend(error.remaining.into_iter().map(|backend_id| ResourceNote {
                content: Content::Resource,
                note: format!("Backend {backend_id} was not completed"),
            }));
        result.remaining.extend(
            error
                .cleanup_failures
                .into_iter()
                .map(|failure| ResourceNote {
                    content: Content::Resource,
                    note: format!("cleanup incomplete: {failure}"),
                }),
        );
        result
    }

    fn reasoned_failure(
        &self,
        request: &OperationRequest,
        state: Option<EnvironmentState>,
        reason: impl Into<String>,
    ) -> OperationResult {
        let mut result = result(request, ResultStatus::Failed, state, can_accept(state));
        result.reason = Some(reason.into());
        result.backend_states = self.runtime.backend_states(&request.environment_id);
        result
    }

    fn not_applicable(
        &self,
        request: &OperationRequest,
        state: Option<EnvironmentState>,
        reason: &str,
    ) -> OperationResult {
        let mut result = result(request, ResultStatus::NotApplicable, state, false);
        result.reason = Some(reason.to_string());
        result.backend_states = self.runtime.backend_states(&request.environment_id);
        result
    }

    fn unsupported(
        &self,
        request: &OperationRequest,
        state: Option<EnvironmentState>,
        reason: String,
    ) -> OperationResult {
        let mut result = result(request, ResultStatus::Unsupported, state, false);
        result.reason = Some(reason);
        result.backend_states = self.runtime.backend_states(&request.environment_id);
        result
    }

    fn unknown_environment(&self, request: &OperationRequest) -> OperationResult {
        let mut result = result(request, ResultStatus::Failed, None, false);
        result.reason = Some("unknown environment".to_string());
        result.backend_states = self.runtime.backend_states(&request.environment_id);
        result
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

    fn stop_agents(&self) {
        if self.runtime.manages_agent_process() {
            if let Err(error) = self.runtime.stop_agents() {
                eprintln!("adapter: failed to stop Runtime Agent: {error}");
            }
        } else {
            self.stop_agent();
        }
    }
    fn start_state_watcher(self: Arc<Self>) {
        thread::spawn(move || {
            loop {
                thread::sleep(Duration::from_millis(100));
                for environment_id in self.runtime.environments() {
                    let Some(current) = self.runtime.state(&environment_id) else {
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
        backend_states: Vec::new(),
        outputs: Vec::new(),
        reason: None,
        output: None,
    }
}

fn can_accept(state: Option<EnvironmentState>) -> bool {
    matches!(
        state,
        Some(EnvironmentState::Available | EnvironmentState::Suspended)
    )
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
            let _ = shared.out.send(AdapterMessage::Event(Event {
                environment_id: request.environment_id.clone(),
                request_id: Some(request.request_id.clone()),
                payload: EventPayload::OperationStarted {
                    operation: request.operation.clone(),
                },
            }));
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
            let _ = shared.out.send(AdapterMessage::Event(Event {
                environment_id: request.environment_id.clone(),
                request_id: Some(request.request_id.clone()),
                payload: EventPayload::OperationFinished {
                    result: result.clone(),
                },
            }));
            let _ = shared.out.send(AdapterMessage::Result(result));
        }
    }
}
