//! Execution and policy coordination. Presentations consume the UI interface.

use std::collections::{BTreeMap, VecDeque};
use std::env;
use std::fs;
use std::io::{self, IsTerminal};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

#[path = "ascout-control-plane/tui.rs"]
mod tui;
#[path = "ascout-control-plane/ui.rs"]
mod ui;
#[path = "ascout-control-plane/view.rs"]
mod view;
#[path = "ascout-control-plane/web.rs"]
mod web;

use ascout::agent_control::{
    AgentCommand, AgentController, AgentMessage, AgentWireMessage, OperationKind,
};
use ascout::contract::{AgentLaunch, Operation, RuntimeCapabilities};
use ascout::control::{ControlServer, ControlSession};
use ascout::ledger::Ledger;
use ascout::policy::{CompiledPolicy, Decision, PolicyEngine};
use ascout::transport;
use serde_json::json;
use serde_json::Value;

static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);

extern "C" fn request_shutdown(_: libc::c_int) {
    // Signal handlers must not allocate, lock, or perform transport I/O.
    SHUTDOWN_REQUESTED.store(true, Ordering::Relaxed);
}

fn install_shutdown_handlers() -> io::Result<()> {
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = request_shutdown as *const () as usize;
        action.sa_flags = libc::SA_RESTART;
        libc::sigemptyset(&mut action.sa_mask);
        for signal in [libc::SIGINT, libc::SIGTERM] {
            if libc::sigaction(signal, &action, std::ptr::null_mut()) != 0 {
                return Err(io::Error::last_os_error());
            }
        }
    }
    Ok(())
}

struct Pending {
    operation_id: String,
    candidate: String,
    tool_name: String,
    started: Instant,
}

/// 启动 Runtime Adapter 并完成握手。控制面只持有连接和进程句柄，
/// 不保存任何环境状态。
fn start_adapter(
    config_path: &std::path::Path,
    run_dir: &std::path::Path,
) -> Result<(std::process::Child, ControlSession, RuntimeCapabilities), Box<dyn std::error::Error>>
{
    let config: Value = serde_json::from_slice(&fs::read(config_path)?)?;
    let socket = std::path::PathBuf::from(
        config["control_socket"]
            .as_str()
            .ok_or("runtime config missing control_socket")?,
    );
    let server = ControlServer::bind(&socket)?;
    server.set_nonblocking(true)?;
    let binary = env::var_os("ASCOUT_RUNTIME_ADAPTER")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("target/debug/ascout-runtime-adapter"));
    let log_path = run_dir.join("adapter.log");
    let log = fs::File::create(&log_path)?;
    let mut adapter = Command::new(binary)
        // Terminal Ctrl-C belongs to the controller. Its children are stopped
        // through the shutdown protocol, not killed by the foreground group.
        .process_group(0)
        .arg("--config")
        .arg(config_path)
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log))
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(10);
    let session = loop {
        match server.accept() {
            Ok(session) => break session,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if let Some(status) = adapter.try_wait()? {
                    let detail = fs::read_to_string(&log_path)
                        .unwrap_or_default()
                        .trim()
                        .replace('\n', " | ");
                    return Err(format!(
                        "Runtime Adapter exited before connecting ({status}); see {}: {detail}",
                        log_path.display()
                    )
                    .into());
                }
                if Instant::now() >= deadline {
                    let _ = adapter.kill();
                    let _ = adapter.wait();
                    let detail = fs::read_to_string(&log_path)
                        .unwrap_or_default()
                        .trim()
                        .replace('\n', " | ");
                    return Err(format!(
                        "Runtime Adapter did not connect within 10s; see {}: {detail}",
                        log_path.display()
                    )
                    .into());
                }
                thread::sleep(Duration::from_millis(30));
            }
            Err(error) => return Err(error.into()),
        }
    };
    let hello = session
        .wait_for_hello(Duration::from_secs(10))
        .ok_or("Runtime Adapter handshake timed out")?;
    Ok((adapter, session, hello.capabilities))
}

fn runtime_environment<'a>(capabilities: &RuntimeCapabilities, branch: &'a str) -> &'a str {
    if capabilities.shared_environment {
        "authority"
    } else {
        branch
    }
}

fn runtime_tool<'a>(
    capabilities: &'a RuntimeCapabilities,
    tool_name: &str,
) -> Option<&'a ascout::contract::RuntimeTool> {
    capabilities
        .tools
        .iter()
        .find(|tool| tool.name == tool_name)
}

fn runtime_tool_context(
    session: &ControlSession,
    capabilities: &RuntimeCapabilities,
    branch: &str,
    tool_name: &str,
) -> Result<Value, Box<dyn std::error::Error>> {
    let result = runtime_call(
        session,
        runtime_environment(capabilities, branch),
        Operation::ToolContext {
            tool_name: tool_name.to_string(),
        },
    )?;
    let output = result
        .output
        .ok_or("Runtime tool context returned no output")?;
    Ok(serde_json::from_str(&output.stdout)?)
}

fn runtime_tool_invocation(
    session: &ControlSession,
    capabilities: &RuntimeCapabilities,
    branch: &str,
    tool_name: &str,
    arguments: &Value,
) -> Result<(Value, Value), Box<dyn std::error::Error>> {
    let result = runtime_call(
        session,
        runtime_environment(capabilities, branch),
        Operation::InvokeTool {
            tool_name: tool_name.to_string(),
            arguments: arguments.clone(),
        },
    )?;
    let output = result
        .output
        .ok_or("Runtime tool invocation returned no output")?;
    let value: Value = serde_json::from_str(&output.stdout)?;
    let tool_result = value
        .get("result")
        .cloned()
        .ok_or("Runtime tool result missing")?;
    let evidence = value
        .get("evidence")
        .cloned()
        .ok_or("Runtime tool evidence missing")?;
    Ok((tool_result, evidence))
}

fn runtime_cleanup(
    session: &ControlSession,
    capabilities: &RuntimeCapabilities,
    branch: &str,
    controller: &mut AgentController,
) -> Result<(), Box<dyn std::error::Error>> {
    if capabilities.shared_environment {
        return Ok(());
    }
    let environment = runtime_environment(capabilities, branch);
    use ascout::agent_control::RuntimeExecutionState;
    controller.record_runtime_state(branch, RuntimeExecutionState::TerminationRequested)?;
    if let Err(error) = runtime_call(session, environment, Operation::TerminateEnvironment) {
        controller.record_runtime_state(branch, RuntimeExecutionState::Failed)?;
        return Err(error);
    }
    controller.record_runtime_state(branch, RuntimeExecutionState::Terminated)?;
    if let Err(error) = runtime_call(session, environment, Operation::CleanupEnvironment) {
        controller.record_runtime_state(branch, RuntimeExecutionState::Failed)?;
        return Err(error);
    }
    controller.record_runtime_state(branch, RuntimeExecutionState::Cleaned)?;
    Ok(())
}

/// 按 Branch 名字向 Runtime 发一次操作；Runtime 自己负责把它对应到环境。
fn runtime_call(
    session: &ControlSession,
    branch: &str,
    operation: Operation,
) -> Result<ascout::contract::OperationResult, Box<dyn std::error::Error>> {
    let result = session.call(branch, operation)?;
    if result.status != ascout::contract::ResultStatus::Completed {
        return Err(format!("Runtime request failed: {:?}", result.reason).into());
    }
    Ok(result)
}

fn take_option(args: &mut Vec<String>, name: &str) -> Result<Option<String>, String> {
    let Some(index) = args.iter().position(|arg| arg == name) else {
        return Ok(None);
    };
    if index + 1 >= args.len() {
        return Err(format!("{name} requires a value"));
    }
    let value = args.remove(index + 1);
    args.remove(index);
    Ok(Some(value))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    install_shutdown_handlers()?;
    let mut args: Vec<String> = env::args().skip(1).collect();
    let model = take_option(&mut args, "--model")?;
    let thinking = take_option(&mut args, "--thinking")?;
    let web_address = take_option(&mut args, "--web")?;
    if !(2..=3).contains(&args.len())
        || (web_address.is_none() && (!io::stdin().is_terminal() || !io::stdout().is_terminal()))
    {
        return Err(
            "usage: ascout-control-plane <run-dir> <pi-root> [policy.yaml] [--web 127.0.0.1:8080] [--model <provider/model>] [--thinking <level>]".into(),
        );
    }
    // 运行目录由调用方（实验脚手架）创建，控制面只使用它。
    let run_dir = fs::canonicalize(&args[0])?;
    if !run_dir.is_dir() {
        return Err(format!("{} is not a run directory", run_dir.display()).into());
    }
    let pi_root = fs::canonicalize(&args[1])?;
    let policy = PolicyEngine::default().compile(std::path::Path::new(
        args.get(2)
            .map(String::as_str)
            .unwrap_or("assets/policy/policy.yaml"),
    ))?;

    let socket = run_dir.join("agent.sock");
    let _ = fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket)?;
    listener.set_nonblocking(true)?;

    // Policy 可以选择 Runtime；环境变量保留给部署时覆盖。
    let config_path = env::var_os("ASCOUT_RUNTIME_CONFIG")
        .map(std::path::PathBuf::from)
        .or_else(|| policy.runtime().map(std::path::PathBuf::from))
        .ok_or("no Runtime selected: set runtime in the Policy or ASCOUT_RUNTIME_CONFIG")?;
    let (mut adapter, session, runtime_capabilities) = start_adapter(&config_path, &run_dir)?;
    let mut controller = AgentController::new("agent", &run_dir)?;
    let runtime_tools = serde_json::to_string(&runtime_capabilities.tools)?;
    // 用哪个 Agent 产品、哪个版本由控制面决定；启动它由 Runtime 完成。
    let mut agent_args = vec![
        "--import".into(),
        pi_root
            .join("packages/coding-agent/src/experimental/source-resolver.ts")
            .display()
            .to_string(),
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("agent-access/pi/main.ts")
            .display()
            .to_string(),
        socket.display().to_string(),
        "agent".into(),
    ];
    if let Some(model) = model {
        agent_args.push("--model".into());
        agent_args.push(model);
    }
    if let Some(thinking) = thinking {
        agent_args.push("--thinking".into());
        agent_args.push(thinking);
    }
    let agent_launch = AgentLaunch {
        command: "node".into(),
        args: agent_args,
        cwd: controller.state().workspace(),
        log: run_dir.join("agent.log"),
        env: std::collections::BTreeMap::from([
            ("ASCOUT_RUNTIME_TOOLS".into(), runtime_tools),
            ("ASCOUT_PI_ROOT".into(), pi_root.display().to_string()),
        ]),
    };
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        runtime_call(&session, "authority", Operation::EstablishEnvironment)?;
        // 环境建立后，由 Runtime 在其中建立 Agent 接入侧。
        runtime_call(
            &session,
            "authority",
            Operation::OperateEnvironment {
                command: Vec::new(),
                agent: Some(agent_launch),
            },
        )?;
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return Err(format!(
                            "Agent connection timed out; see {}/agent.log",
                            run_dir.display()
                        )
                        .into());
                    }
                    thread::sleep(Duration::from_millis(30));
                }
                Err(error) => return Err(error.into()),
            }
        };
        stream.set_read_timeout(Some(Duration::from_secs(10)))?;
        match transport::read_message(&mut stream)? {
            Some(AgentWireMessage::Message(AgentMessage::Hello { agent_id }))
                if agent_id == "agent" => {}
            _ => return Err("Agent handshake failed".into()),
        }
        stream.set_read_timeout(None)?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        let command = controller.create_authority("authority", "")?;
        controller.record_runtime_state(
            "authority",
            ascout::agent_control::RuntimeExecutionState::Ready,
        )?;
        send(&mut stream, command)?;
        let (interface, requests) = ui::Interface::new(controller.state().tree.clone());
        let tui_thread = if let Some(address) = &web_address {
            web::start(address, interface.clone())?;
            None
        } else {
            Some(tui::start(interface.clone())?)
        };
        let result = run(
            &mut stream,
            &mut controller,
            &policy,
            &session,
            &runtime_capabilities,
            &run_dir,
            &interface,
            requests,
        );
        interface.disconnect();
        if let Some(thread) = tui_thread {
            let _ = thread.join();
        }
        result
    })();
    // 先让 Runtime 停掉环境里的 Agent 进程，再断开 Adapter 让它自行退出。
    let runtime_result: Result<(), Box<dyn std::error::Error>> = (|| {
        // Every retained Branch refers to this same shared environment. A
        // confirmed environment fact therefore applies to all of them, not
        // only the last Authority. Archived records remain historical facts.
        let record_shared =
            |controller: &mut AgentController, state| -> Result<(), Box<dyn std::error::Error>> {
                let ids: Vec<_> = controller.state().tree.branches.keys().cloned().collect();
                for id in ids {
                    controller.record_runtime_state(&id, state)?;
                }
                Ok(())
            };
        if let Some(authority) = controller.state().tree.authority_branch_id.clone() {
            if runtime_capabilities.shared_environment {
                record_shared(
                    &mut controller,
                    ascout::agent_control::RuntimeExecutionState::TerminationRequested,
                )?;
                if let Err(error) =
                    runtime_call(&session, "authority", Operation::TerminateEnvironment)
                {
                    record_shared(
                        &mut controller,
                        ascout::agent_control::RuntimeExecutionState::Failed,
                    )?;
                    return Err(error);
                }
                record_shared(
                    &mut controller,
                    ascout::agent_control::RuntimeExecutionState::Terminated,
                )?;
                if let Err(error) =
                    runtime_call(&session, "authority", Operation::CleanupEnvironment)
                {
                    record_shared(
                        &mut controller,
                        ascout::agent_control::RuntimeExecutionState::Failed,
                    )?;
                    return Err(error);
                }
                record_shared(
                    &mut controller,
                    ascout::agent_control::RuntimeExecutionState::Cleaned,
                )?;
            } else {
                runtime_cleanup(&session, &runtime_capabilities, &authority, &mut controller)?;
            }
        }
        Ok(())
    })();
    // Cleanup failure must not skip closing the cloned reader socket or
    // reaping the Adapter. Preserve every failure instead of hiding one.
    let mut errors = Vec::new();
    if let Err(error) = result {
        errors.push(format!("Agent execution/shutdown: {error}"));
    }
    if let Err(error) = runtime_result {
        errors.push(format!("Runtime cleanup: {error}"));
    }
    let final_state = serde_json::to_vec_pretty(&controller.state().tree)
        .map_err(std::io::Error::other)
        .and_then(|bytes| std::fs::write(run_dir.join("final-tree.json"), bytes));
    if let Err(error) = final_state {
        errors.push(format!("Final Tree recording: {error}"));
    }
    if let Err(error) = session.shutdown() {
        errors.push(format!("Runtime connection shutdown: {error}"));
    }
    drop(session);
    let deadline = Instant::now() + Duration::from_secs(5);
    while adapter.try_wait()?.is_none() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(50));
    }
    if let Some(status) = adapter.try_wait()? {
        if !status.success() {
            errors.push(format!("Runtime Adapter exited with {status}"));
        }
    } else {
        adapter.kill()?;
        adapter.wait()?;
        errors.push("Runtime Adapter shutdown timed out; forced termination required".into());
    }
    // 运行目录留给调用方检查；不声称异常退出已完成环境清理。
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; ").into())
    }
}

fn run(
    stream: &mut UnixStream,
    controller: &mut AgentController,
    policy: &CompiledPolicy,
    runtime: &ControlSession,
    runtime_capabilities: &RuntimeCapabilities,
    run_dir: &std::path::Path,
    interface: &ui::Interface,
    requests: mpsc::Receiver<ui::Request>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (tx, rx) = mpsc::channel();
    let mut reader = stream.try_clone()?;
    thread::spawn(move || loop {
        let message = transport::read_message::<_, AgentWireMessage>(&mut reader);
        let done = !matches!(&message, Ok(Some(_)));
        if tx.send(message).is_err() || done {
            break;
        }
    });
    let mut details: BTreeMap<String, String> = BTreeMap::new();
    let mut pending: BTreeMap<String, Pending> = BTreeMap::new();
    // Per-branch count of transcript entries written, used to record where a
    // candidate forked from its parent.
    let mut transcripts: BTreeMap<String, usize> = BTreeMap::new();
    let mut finished = std::collections::BTreeSet::new();
    let mut sequence = 0;
    let mut ledger = Ledger::open(&run_dir.join("events.jsonl"))?;
    let mut ready = false;
    let mut running = false;
    let mut queued: VecDeque<String> = VecDeque::new();
    let mut notice = String::from("Starting Agent session…");
    let run_started = Instant::now();
    let execution_result: Result<(), Box<dyn std::error::Error>> = (|| {
        'execution: loop {
            if SHUTDOWN_REQUESTED.load(Ordering::Relaxed) {
                break 'execution;
            }
            interface.publish(&controller.state().tree, &details, ready, running, &notice);
            while let Ok(request) = requests.try_recv() {
                match request {
                    ui::Request::Shutdown => break 'execution,
                    ui::Request::Prompt { prompt } => {
                        if !prompt.trim().is_empty() {
                            queued.push_back(prompt);
                            notice = "Prompt queued for the current authority".into();
                        }
                    }
                    ui::Request::Interrupt => {
                        queued.clear();
                        for (_, work) in std::mem::take(&mut pending) {
                            let parent = candidate_parent(controller, &work.candidate)?;
                            send(
                                stream,
                                AgentCommand::ResolvePending {
                                    branch_id: parent,
                                    operation_id: work.operation_id,
                                    accepted: false,
                                    outcome: None,
                                    command_id: "resolve-interrupt".into(),
                                    generation: 0,
                                },
                            )?;
                            for command in controller.resolve_candidate(&work.candidate, false)? {
                                send(stream, command)?;
                            }
                        }
                        if let Some(id) = &controller.state().tree.authority_branch_id {
                            send(
                                stream,
                                AgentCommand::Abort {
                                    branch_id: id.clone(),
                                },
                            )?;
                            notice =
                                "Interrupt requested; waiting for Agent execution to settle".into();
                        }
                    }
                }
            }
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(Ok(Some(AgentWireMessage::Message(message)))) => {
                    interface.message(&message);
                    // Streaming text is a UI detail: the same text is already in the
                    // assistant transcript entry, so it is not logged twice.
                    let streaming = matches!(message, AgentMessage::Output { .. });
                    if !streaming {
                        if let AgentMessage::Transcript { branch_id, .. } = &message {
                            *transcripts.entry(branch_id.clone()).or_default() += 1;
                        }
                        let mut record = serde_json::to_value(&message)?;
                        if let Some(object) = record.as_object_mut() {
                            object.insert(
                                "t_ms".into(),
                                json!(run_started.elapsed().as_millis() as u64),
                            );
                        }
                        ledger.record(record)?;
                        interface.record(serde_json::to_value(&message)?);
                    }
                    match &message {
                        AgentMessage::CleanupCompleted { branch_id } => {
                            details.entry(branch_id.clone()).or_default().push_str("\n[CleanupCompleted: Agent session stopped/released; managed directory cleanup follows]\n");
                        }
                        AgentMessage::BranchStarted { branch_id } => {
                            details
                                .entry(branch_id.clone())
                                .or_default()
                                .push_str("\n[Agent session actually started]\n");
                            if controller.state().tree.authority_branch_id.as_ref()
                                == Some(branch_id)
                            {
                                ready = true;
                                notice = "Ready | /ascout: Tree | /events: Events | Esc: conversation | ↑/↓: scroll | Ctrl-D: exit".into();
                            }
                        }
                        AgentMessage::TurnCompleted { branch_id } => {
                            finished.insert(branch_id.clone());
                            details
                                .entry(branch_id.clone())
                                .or_default()
                                .push_str("\n[Execution finished; not an acceptance decision]\n");
                            if controller.state().tree.authority_branch_id.as_ref()
                                == Some(branch_id)
                            {
                                running = false;
                                notice =
                                "Ready | /ascout: Tree | /events: Events | Ctrl-C: interrupt | Ctrl-D: exit".into();
                            }
                        }
                        AgentMessage::ToolPending {
                            branch_id,
                            operation_id,
                            tool_name,
                            arguments,
                            checkpoint,
                        } => {
                            let runtime_tool_declared =
                                runtime_tool(runtime_capabilities, &tool_name).is_some();
                            let authority_observation = if runtime_tool_declared {
                                controller
                                    .state()
                                    .tree
                                    .authority_branch_id
                                    .as_ref()
                                    .and_then(|authority| {
                                        runtime_tool_context(
                                            runtime,
                                            runtime_capabilities,
                                            authority,
                                            &tool_name,
                                        )
                                        .ok()
                                    })
                            } else {
                                None
                            };
                            let predictions =
                                if controller.state().tree.authority_branch_id.as_ref()
                                    == Some(branch_id)
                                {
                                    match predict(
                                        policy,
                                        operation_id,
                                        tool_name,
                                        arguments,
                                        authority_observation.as_ref(),
                                    ) {
                                        Ok(value) => value,
                                        Err(error) => {
                                            details.entry(branch_id.clone()).or_default().push_str(
                                                &format!(
                                                    "\n[Policy error; Parent fallback] {error}\n"
                                                ),
                                            );
                                            Vec::new()
                                        }
                                    }
                                } else {
                                    Vec::new()
                                };
                            if predictions.is_empty() {
                                notice = format!(
                                "ToolPending {tool_name}; Policy did not admit speculation; Parent continues"
                            );
                            }
                            for prediction in predictions {
                                sequence += 1;
                                let candidate_id = format!("candidate-{sequence}");
                                ledger.record(json!({
                                    "message": "candidate_created",
                                    "branch_id": candidate_id,
                                    "parent": branch_id,
                                    "fork_at": transcripts.get(branch_id).copied().unwrap_or(0),
                                    "t_ms": run_started.elapsed().as_millis() as u64,
                                }))?;
                                let _ =
                                    controller.create_candidate(&branch_id, &candidate_id, "")?;
                                controller.record_candidate_context(
                                    &candidate_id,
                                    operation_id.clone(),
                                    prediction.clone(),
                                    sequence,
                                    transcripts.get(branch_id).copied().unwrap_or(0),
                                )?;
                                controller.mark_operation_requested(
                                    &candidate_id,
                                    &operation_id,
                                    OperationKind::StartCandidate,
                                )?;
                                interface.fork(branch_id, &candidate_id);
                                if runtime_capabilities.branch_environment
                                    && !runtime_capabilities.shared_environment
                                {
                                    runtime_call(
                                        runtime,
                                        &branch_id,
                                        Operation::BranchEnvironment {
                                            child_environment_id: candidate_id.clone(),
                                        },
                                    )?;
                                }
                                let workspace = controller.state().workspace();
                                controller.record_runtime_state(
                                    &candidate_id,
                                    ascout::agent_control::RuntimeExecutionState::Ready,
                                )?;
                                let pending_id = format!("{operation_id}:{candidate_id}");
                                pending.insert(
                                    pending_id.clone(),
                                    Pending {
                                        operation_id: operation_id.clone(),
                                        candidate: candidate_id.clone(),
                                        tool_name: tool_name.clone(),
                                        started: Instant::now(),
                                    },
                                );
                                send(
                                    stream,
                                    AgentCommand::StartCandidate {
                                        branch_id: candidate_id.clone(),
                                        parent_branch_id: branch_id.clone(),
                                        operation_id: operation_id.clone(),
                                        tool_name: tool_name.clone(),
                                        workspace,
                                        prediction: prediction.clone(),
                                        checkpoint: checkpoint.clone(),
                                        command_id: format!("start-{candidate_id}"),
                                        generation: controller.state().tree.branches[&candidate_id]
                                            .generation,
                                    },
                                )?;
                                details.entry(candidate_id).or_default().push_str(&format!(
                                    "[Candidate requested for {operation_id}; hypothesis={}]\n",
                                    prediction
                                ));
                                notice = format!(
                                "ToolPending {tool_name}; {} Candidates started; Parent continues",
                                pending
                                    .values()
                                    .filter(|work| {
                                        candidate_parent(controller, &work.candidate)
                                            .ok()
                                            .as_deref()
                                            == Some(branch_id)
                                    })
                                    .count()
                            );
                            }
                            if runtime_tool_declared {
                                let authority = controller
                                    .state()
                                    .tree
                                    .authority_branch_id
                                    .clone()
                                    .ok_or("tree has no authority branch")?;
                                let (tool_result, evidence) = runtime_tool_invocation(
                                    runtime,
                                    runtime_capabilities,
                                    &authority,
                                    &tool_name,
                                    arguments,
                                )?;
                                controller.record_candidate_outcome(
                                    branch_id,
                                    operation_id,
                                    evidence.clone(),
                                    Some(evidence.clone()),
                                )?;
                                send(
                                    stream,
                                    AgentCommand::ResolvePending {
                                        branch_id: branch_id.clone(),
                                        operation_id: operation_id.clone(),
                                        accepted: false,
                                        outcome: Some(tool_result),
                                        command_id: format!("resolve-{operation_id}"),
                                        generation: controller.state().tree.branches[branch_id]
                                            .generation,
                                    },
                                )?;
                            }
                        }
                        AgentMessage::ParentOutcome {
                            branch_id,
                            operation_id,
                            outcome,
                        } => {
                            let matching_candidates: Vec<String> = pending
                                .values()
                                .filter(|work| {
                                    candidate_parent(controller, &work.candidate)
                                        .ok()
                                        .as_deref()
                                        == Some(branch_id)
                                        && work.operation_id == *operation_id
                                })
                                .map(|work| work.candidate.clone())
                                .collect();
                            for candidate_id in matching_candidates {
                                // Runtime 工具的权威结果由 Runtime 生成；Pi 回报的文本
                                // 只是该结果在工具通道里的表示，不覆盖可比较的证据。
                                let tool_name = pending
                                    .get(&format!("{operation_id}:{candidate_id}"))
                                    .map(|work| work.tool_name.as_str());
                                if tool_name.is_some_and(|tool| {
                                    runtime_tool(runtime_capabilities, tool).is_none()
                                }) {
                                    controller.record_candidate_outcome(
                                        branch_id,
                                        operation_id,
                                        outcome.clone(),
                                        None,
                                    )?;
                                }
                                details
                                    .entry(candidate_id)
                                    .or_default()
                                    .push_str(&format!("\nParent 真实结果\n{}\n", outcome));
                            }
                            continue;
                        }
                        AgentMessage::BranchFailed { branch_id, reason } => {
                            if controller.state().tree.branches.get(branch_id).is_some_and(
                                |branch| {
                                    matches!(
                                        branch.status,
                                        ascout::agent_control::BranchStatus::Discarded
                                            | ascout::agent_control::BranchStatus::Cleaned
                                    )
                                },
                            ) {
                                continue;
                            }
                            let keys: Vec<_> = pending
                                .iter()
                                .filter(|(_, work)| {
                                    &work.candidate == branch_id
                                        || candidate_parent(controller, &work.candidate)
                                            .ok()
                                            .as_deref()
                                            == Some(branch_id)
                                })
                                .map(|(id, _)| id.clone())
                                .collect();
                            for key in keys {
                                let work = pending.remove(&key).unwrap();
                                let parent = candidate_parent(controller, &work.candidate)?;
                                let outcome = candidate_outcome(
                                    controller,
                                    &work.candidate,
                                    &work.operation_id,
                                );
                                send(
                                    stream,
                                    AgentCommand::ResolvePending {
                                        branch_id: parent.clone(),
                                        operation_id: work.operation_id.clone(),
                                        accepted: false,
                                        outcome,
                                        command_id: format!("resolve-{}", work.operation_id),
                                        generation: controller.state().tree.branches[&parent]
                                            .generation,
                                    },
                                )?;
                                if &work.candidate != branch_id {
                                    for command in
                                        controller.resolve_candidate(&work.candidate, false)?
                                    {
                                        send(stream, command)?;
                                    }
                                }
                            }
                            details
                                .entry(branch_id.clone())
                                .or_default()
                                .push_str(&format!("\n[Failed] {reason}\n"));
                            notice = format!("Branch {branch_id} failed: {reason}");
                            if controller.state().tree.authority_branch_id.as_ref()
                                == Some(branch_id)
                            {
                                ready = false;
                                running = false;
                                // Preserve queued input for a later authority or explicit interrupt.
                            }
                        }
                        AgentMessage::CandidateCompleted { .. } => {
                            return Err(
                            "Unexpected speculative result: pending-result path is not connected"
                                .into(),
                        );
                        }
                        _ => {}
                    }
                    for command in controller.handle_message(message)? {
                        send(stream, command)?;
                    }
                    if let Some(reason) = controller.authority_failure() {
                        return Err(reason.to_string().into());
                    }
                    evict_finished_branches(controller, &mut ledger)?;
                }
                Ok(Ok(Some(AgentWireMessage::Command(_)))) => {
                    return Err("Agent sent a control-plane command".into());
                }
                Ok(Ok(None)) => return Err("Agent disconnected; cleanup is not confirmed".into()),
                Ok(Err(error)) => return Err(error.into()),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => break,
            }
            let keys: Vec<_> = pending
                .iter()
                .filter(|(_, work)| {
                    (candidate_outcome(controller, &work.candidate, &work.operation_id).is_some()
                        && controller.state().tree.branches[&work.candidate].status
                            == ascout::agent_control::BranchStatus::Running)
                        || work.started.elapsed() > Duration::from_secs(120)
                })
                .map(|(id, _)| id.clone())
                .collect();
            for key in keys {
                let Some(work) = pending.remove(&key) else {
                    continue;
                };
                let proposal = candidate_proposal(controller, &work.candidate)?;
                let outcome = candidate_outcome(controller, &work.candidate, &work.operation_id);
                let parent = candidate_parent(controller, &work.candidate)?;
                let (compatible, verdict) = outcome
                    .as_ref()
                    .map(|outcome| matches_outcome(policy, &work.operation_id, &proposal, outcome))
                    .transpose()
                    .unwrap_or_else(|error| {
                        details
                            .entry(work.candidate.clone())
                            .or_default()
                            .push_str(&format!("\n[Policy error; Parent fallback] {error}\n"));
                        None
                    })
                    .unwrap_or((false, "error".into()));
                let mut accepted = compatible
                    && controller.state().tree.authority_branch_id.as_ref() == Some(&parent)
                    && controller.state().tree.branches[&work.candidate].status
                        == ascout::agent_control::BranchStatus::Running;
                // Runtime promotion is part of the same lifecycle event even when
                // the current Runtime (NLE) has no environment-side work to do.
                // Fail the promotion before the Agent switches authority if the
                // Runtime cannot accept it.
                let mut runtime_promote_error = None;
                if accepted {
                    controller.record_runtime_state(
                        &work.candidate,
                        ascout::agent_control::RuntimeExecutionState::PromotionRequested,
                    )?;
                    if let Err(error) = runtime_call(
                        runtime,
                        runtime_environment(runtime_capabilities, &work.candidate),
                        Operation::PromoteEnvironment,
                    ) {
                        accepted = false;
                        runtime_promote_error = Some(error.to_string());
                        controller.record_runtime_state(
                            &work.candidate,
                            ascout::agent_control::RuntimeExecutionState::Failed,
                        )?;
                    } else {
                        controller.record_runtime_state(
                            &work.candidate,
                            ascout::agent_control::RuntimeExecutionState::Promoted,
                        )?;
                    }
                }
                // Parent already received the actual tool result immediately after
                // ToolPending. Disposition only decides Candidate lifecycle.
                // Created sessions that failed to start are cancelled, not accepted.
                let commands = if controller.state().tree.branches[&work.candidate].status
                    == ascout::agent_control::BranchStatus::Running
                {
                    controller.resolve_candidate(&work.candidate, accepted)?
                } else {
                    controller.handle_message(AgentMessage::BranchFailed {
                        branch_id: work.candidate.clone(),
                        reason: "Candidate startup timed out".into(),
                    })?
                };
                for command in commands {
                    send(stream, command)?;
                }
                if let Some(error) = &runtime_promote_error {
                    details
                        .entry(work.candidate.clone())
                        .or_default()
                        .push_str(&format!(
                            "\n[Runtime promote failed; Candidate discarded] {error}\n"
                        ));
                }
                details
                    .entry(work.candidate.clone())
                    .or_default()
                    .push_str(if accepted {
                        "\n[Policy disposition: Promote]\n"
                    } else {
                        "\n[Policy disposition/fallback: Discard]\n"
                    });
                ledger.record(json!({"branch_id": work.candidate, "disposition": if accepted { "Promote" } else { "Discard" }, "hypothesis": proposal, "verdict": verdict, "runtime_promote_error": runtime_promote_error, "t_ms": run_started.elapsed().as_millis() as u64}))?;
                interface.record(json!({"branch_id": work.candidate, "disposition": if accepted { "Promote" } else { "Discard" }, "hypothesis": proposal, "verdict": verdict, "runtime_promote_error": runtime_promote_error}));
                if accepted {
                    // The promoted Branch keeps its own environment; the replaced
                    // authority's environment is the one that gets cleaned up.
                    if runtime_capabilities.branch_environment
                        && !runtime_capabilities.shared_environment
                    {
                        runtime_cleanup(runtime, runtime_capabilities, &parent, controller)?;
                    }
                    // Promote invalidates every other continuation of the old Parent,
                    // including candidates for other concurrently pending operations.
                    let obsolete: Vec<_> = pending
                        .iter()
                        .filter(|(_, sibling)| {
                            candidate_parent(controller, &sibling.candidate)
                                .ok()
                                .as_deref()
                                == Some(parent.as_str())
                        })
                        .map(|(id, _)| id.clone())
                        .collect();
                    for id in obsolete {
                        let sibling = pending.remove(&id).unwrap();
                        if runtime_capabilities.branch_environment
                            && !runtime_capabilities.shared_environment
                        {
                            runtime_cleanup(
                                runtime,
                                runtime_capabilities,
                                &sibling.candidate,
                                controller,
                            )?;
                        }
                        for command in controller.resolve_candidate(&sibling.candidate, false)? {
                            send(stream, command)?;
                        }
                        details
                            .entry(sibling.candidate.clone())
                            .or_default()
                            .push_str("\n[Discard: another Candidate replaced Parent]\n");
                        ledger.record(json!({"branch_id": sibling.candidate, "operation_id": sibling.operation_id, "disposition": "Discard", "reason": "Parent replaced", "t_ms": run_started.elapsed().as_millis() as u64}))?;
                    }
                    running = !finished.contains(&work.candidate);
                    ready = true;
                } else if runtime_capabilities.branch_environment
                    && !runtime_capabilities.shared_environment
                {
                    runtime_cleanup(runtime, runtime_capabilities, &work.candidate, controller)?;
                }
                notice = if accepted {
                    "Candidate promoted; same live session now serves the main conversation".into()
                } else {
                    "Candidate discarded; Parent continues with the actual tool result".into()
                };
                evict_finished_branches(controller, &mut ledger)?;
            }
            if !queued.is_empty() && ready && !running {
                if let Some(id) = controller.state().tree.authority_branch_id.clone() {
                    let line = queued.pop_front().unwrap();
                    finished.remove(&id);
                    send(
                        stream,
                        AgentCommand::Prompt {
                            branch_id: id,
                            prompt: line,
                        },
                    )?;
                    running = true;
                    notice = "Agent running".into();
                }
            }
        }
        Ok(())
    })();
    let branch_ids: Vec<_> = controller
        .state()
        .tree
        .branches
        .values()
        .filter(|branch| {
            branch.execution_state.agent != ascout::agent_control::AgentExecutionState::Released
        })
        .map(|branch| branch.branch_id.clone())
        .collect();
    let mut cleanup = BTreeMap::new();
    for branch_id in branch_ids {
        let command = controller.begin_cleanup(&branch_id)?;
        if let AgentCommand::Cleanup { command_id, .. } = &command {
            cleanup.insert(branch_id, command_id.clone());
        }
        send(stream, command)?;
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while !cleanup.is_empty() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(remaining) {
            Ok(Ok(Some(AgentWireMessage::Message(message)))) => {
                ledger.record(serde_json::to_value(&message)?)?;
                if let AgentMessage::CommandResult {
                    branch_id,
                    command_id,
                    status,
                    ..
                } = &message
                {
                    if cleanup.get(branch_id) == Some(command_id) {
                        if *status != ascout::agent_control::CommandStatus::Applied {
                            return Err(format!("Agent cleanup failed for {branch_id}").into());
                        }
                        cleanup.remove(branch_id);
                    }
                }
                for command in controller.handle_message(message)? {
                    send(stream, command)?;
                }
            }
            _ => return Err("Agent shutdown incomplete: cleanup was not confirmed".into()),
        }
    }
    // Half-close only after cleanup confirmations, keeping the read side alive
    // until the integration has drained its final messages and closed its side.
    stream.shutdown(std::net::Shutdown::Write)?;
    loop {
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(Ok(None)) => break,
            Ok(Ok(Some(AgentWireMessage::Message(message)))) => {
                ledger.record(serde_json::to_value(&message)?)?;
            }
            _ => return Err("Agent shutdown incomplete: connection did not drain".into()),
        }
    }
    interface.publish(&controller.state().tree, &details, false, false, "Stopped");
    execution_result
}

/// Keep only state needed by the active control loop in memory.  The event
/// stream remains the complete history and is written before a Branch is
/// removed from the active view.
fn evict_finished_branches(
    controller: &mut AgentController,
    ledger: &mut Ledger,
) -> Result<(), Box<dyn std::error::Error>> {
    for branch_id in controller.evictable_branches() {
        let branch = controller.take_archivable_branch(&branch_id)?;
        ledger.archive_branch(branch)?;
    }
    Ok(())
}

fn candidate_parent(controller: &AgentController, candidate_id: &str) -> io::Result<String> {
    controller
        .state()
        .tree
        .branches
        .get(candidate_id)
        .and_then(|branch| branch.parent_branch_id.clone())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "candidate parent missing"))
}

fn candidate_proposal(controller: &AgentController, candidate_id: &str) -> io::Result<Value> {
    controller
        .state()
        .tree
        .branches
        .get(candidate_id)
        .and_then(|branch| branch.proposal.as_ref())
        .map(|proposal| proposal.payload.clone())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "candidate proposal missing"))
}

fn candidate_outcome(
    controller: &AgentController,
    candidate_id: &str,
    operation_id: &str,
) -> Option<Value> {
    controller
        .state()
        .tree
        .branches
        .get(candidate_id)
        .and_then(|branch| {
            branch
                .pending_operations
                .iter()
                .find(|operation| operation.operation_id == operation_id)
        })
        .and_then(|operation| operation.outcome.clone())
}

fn predict(
    policy: &CompiledPolicy,
    operation_id: &str,
    tool_name: &str,
    arguments: &serde_json::Value,
    observation: Option<&serde_json::Value>,
) -> Result<Vec<serde_json::Value>, Box<dyn std::error::Error>> {
    let engine = PolicyEngine::default();
    let op = json!({"kind": tool_name, "produces_result": true, "result_pending": true, "has_predictor": true});
    let opportunity = engine.evaluate(policy, Decision::OpportunityPass, &json!({"op": op}))?;
    let feasibility = engine.evaluate(policy, Decision::FeasibilityPass, &json!({"op": op, "state": {}, "contract_context": {"arguments": arguments, "runtime_capabilities": {"candidate_execution": true}}}))?;
    if opportunity != true || feasibility != true {
        return Ok(Vec::new());
    }
    let edge = json!({"edge_id": operation_id, "tool_name": tool_name, "arguments": arguments});
    let selected = engine.evaluate(
        policy,
        Decision::Select,
        &json!({"frontier_view": [edge], "scheduling_context": {"capacity": 1}}),
    )?;
    if !selected
        .as_array()
        .is_some_and(|ids| ids.contains(&json!(operation_id)))
    {
        return Ok(Vec::new());
    }
    let data = json!({
        "edge_context": edge,
        "process_context": {
            "operation_id": operation_id,
            "observation": observation,
            "action": arguments.get("action")
        },
        "assumption_set": [],
        "capability_context": {"tool": tool_name, "runtime_capabilities": {"candidate_execution": true}, "prediction_types": ["string", "boolean", "enum"]}
    });
    let result = engine.evaluate(policy, Decision::Predict, &data)?;
    if engine.evaluate(
        policy,
        Decision::Launch,
        &json!({"edge_context": edge, "authority_state": {}, "candidate_view": []}),
    )? != true
    {
        return Ok(Vec::new());
    }
    Ok(result["proposals"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("payload").cloned())
        .collect())
}

fn matches_outcome(
    policy: &CompiledPolicy,
    operation_id: &str,
    proposal: &serde_json::Value,
    outcome: &serde_json::Value,
) -> Result<(bool, String), Box<dyn std::error::Error>> {
    let result = PolicyEngine::default().evaluate(
        policy,
        Decision::Match,
        &json!({
            "proposal": {"payload": proposal},
            "evidence": outcome,
            "match_context": {"operation_id": operation_id}
        }),
    )?;
    let verdict = result["verdict"].as_str().unwrap_or("error").to_string();
    let compatible = verdict == "compatible";
    let disposition = PolicyEngine::default().evaluate(
        policy,
        Decision::Dispose,
        &json!({
            "branch": {"operation_id": operation_id},
            "resolution": if compatible { "accepted" } else { "rejected" },
            "context": {"match": result}
        }),
    )?;
    Ok((
        compatible && disposition["actions"][0]["intent"] == "Promote",
        verdict,
    ))
}

fn send(stream: &mut UnixStream, command: AgentCommand) -> io::Result<()> {
    transport::write_message(stream, &AgentWireMessage::Command(command))
}
