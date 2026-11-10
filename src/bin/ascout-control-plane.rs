//! Execution and policy coordination. Presentations consume the UI interface.

use std::env;
use std::fs;
use std::io::{self, IsTerminal};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

#[path = "ascout-control-plane/execution.rs"]
mod execution;
#[path = "ascout-control-plane/tui.rs"]
mod tui;
#[path = "ascout-control-plane/ui.rs"]
mod ui;
#[path = "ascout-control-plane/view.rs"]
mod view;
#[path = "ascout-control-plane/web.rs"]
mod web;

use ascout::control_plane::agent_control::{
    AgentCommand, AgentMessage, AgentWireMessage, SpeculativeTree,
};
use ascout::control_plane::ledger::Ledger;
use ascout::control_plane::policy::{CompiledPolicy, Decision, PolicyEngine};
use ascout::control_plane::session::{ControlServer, ControlSession};
use ascout::protocol::contract::{AgentLaunch, Operation, RuntimeCapabilities};
use ascout::protocol::transport;
use serde_json::Value;
use serde_json::json;

static SIGNAL_FD: AtomicI32 = AtomicI32::new(-1);

static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);

extern "C" fn request_shutdown(_: libc::c_int) {
    // Signal handlers must not allocate, lock, or perform transport I/O.
    SHUTDOWN_REQUESTED.store(true, Ordering::Relaxed);
    let fd = SIGNAL_FD.load(Ordering::Relaxed);
    if fd >= 0 {
        unsafe {
            libc::write(fd, [1u8].as_ptr().cast(), 1);
        }
    }
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

fn find_in_path(name: &str) -> Option<std::path::PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
}

fn resolve_node() -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
    if let Some(value) = env::var_os("ASCOUT_NODE") {
        let path = std::path::PathBuf::from(value);
        if path.is_file() {
            return Ok(path);
        }
        return Err(format!(
            "ASCOUT_NODE points to a missing executable: {}",
            path.display()
        )
        .into());
    }
    find_in_path("node").ok_or_else(|| {
        "node executable not found in PATH; set ASCOUT_NODE to an absolute path".into()
    })
}

fn resolve_agent_entry() -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
    let path = env::var_os("ASCOUT_AGENT_ENTRY")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("agent-access/pi/main.ts")
        });
    if !path.is_file() {
        return Err(format!(
            "Agent entry point not found at {}; set ASCOUT_AGENT_ENTRY",
            path.display()
        )
        .into());
    }
    Ok(path)
}

fn validate_agent_capabilities(
    capabilities: &ascout::control_plane::agent_control::AdapterCapabilities,
) -> Result<(), Box<dyn std::error::Error>> {
    let required = [
        (
            capabilities.pre_execution_interception,
            "preExecutionInterception",
        ),
        (
            capabilities.local_decision_evaluation,
            "localDecisionEvaluation",
        ),
        (
            capabilities.checkpoint_continuation,
            "checkpointContinuation",
        ),
        (
            capabilities.independent_candidate_session,
            "independentCandidateSession",
        ),
        (capabilities.cancel_completion, "cancelCompletion"),
        (capabilities.tool_result_injection, "toolResultInjection"),
    ];
    if let Some((false, name)) = required.into_iter().find(|(supported, _)| !supported) {
        return Err(format!("Agent Adapter lacks required capability {name}").into());
    }
    Ok(())
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
        // Terminal Ctrl-C belongs to the tree. Its children are stopped
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

fn runtime_cleanup(
    session: &ControlSession,
    capabilities: &RuntimeCapabilities,
    branch: &str,
    tree: &mut SpeculativeTree,
) -> Result<(), Box<dyn std::error::Error>> {
    if capabilities.shared_environment {
        return Ok(());
    }
    let environment = runtime_environment(capabilities, branch);
    use ascout::control_plane::agent_control::RuntimeExecutionState;
    tree.record_runtime_state(branch, RuntimeExecutionState::TerminationRequested)?;
    if let Err(error) = runtime_call(session, environment, Operation::TerminateEnvironment) {
        tree.record_runtime_state(branch, RuntimeExecutionState::Failed)?;
        return Err(error);
    }
    tree.record_runtime_state(branch, RuntimeExecutionState::Terminated)?;
    if let Err(error) = runtime_call(session, environment, Operation::CleanupEnvironment) {
        tree.record_runtime_state(branch, RuntimeExecutionState::Failed)?;
        return Err(error);
    }
    tree.record_runtime_state(branch, RuntimeExecutionState::Cleaned)?;
    Ok(())
}

/// 按 Branch 名字向 Runtime 发一次操作；Runtime 自己负责把它对应到环境。
fn runtime_call(
    session: &ControlSession,
    branch: &str,
    operation: Operation,
) -> Result<ascout::protocol::contract::OperationResult, Box<dyn std::error::Error>> {
    let result = session.call(branch, operation)?;
    if result.status != ascout::protocol::contract::ResultStatus::Completed {
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
    // Establish task control state before starting its execution infrastructure.
    let mut tree = SpeculativeTree::with_workspace("agent", &run_dir)?;
    let (mut adapter, session, runtime_capabilities) = start_adapter(&config_path, &run_dir)?;
    tree.execution_id = run_dir.display().to_string();
    tree.bind_execution(
        json!({"source": args.get(2).map(String::as_str).unwrap_or("assets/policy/policy.yaml")}),
        json!({"config_path": config_path}),
        serde_json::to_value(&runtime_capabilities)?,
    );
    tree.bind_local_policy(policy.digest());
    let runtime_tools = serde_json::to_string(&runtime_capabilities.tools)?;
    // 用哪个 Agent 产品、哪个版本由控制面决定；启动它由 Runtime 完成。
    let agent_entry = resolve_agent_entry()?;
    let mut agent_args = vec![
        "--import=".to_string()
            + &pi_root
                .join("packages/coding-agent/src/experimental/source-resolver.ts")
                .display()
                .to_string(),
        "--experimental-strip-types".into(),
        agent_entry.display().to_string(),
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
        command: resolve_node()?.display().to_string(),
        args: agent_args,
        cwd: tree.workspace(),
        log: run_dir.join("agent.log"),
        instance_id: Some("agent".into()),
        env: std::collections::BTreeMap::from([
            ("ASCOUT_RUNTIME_TOOLS".into(), runtime_tools),
            ("ASCOUT_PI_ROOT".into(), pi_root.display().to_string()),
            ("ASCOUT_TASK_ID".into(), tree.execution_id.clone()),
        ]),
    };
    let (events, receiver) = mpsc::channel();
    let requests = events.clone();
    let interface = ui::Interface::new(&tree, move |request| {
        requests
            .send(execution::Event::Request(request))
            .map_err(|_| "Control plane disconnected".into())
    });
    let mut tui_thread = None;
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
            Some(AgentWireMessage::Message(AgentMessage::AgentConnected {
                agent_id,
                capabilities,
            })) if agent_id == "agent" => {
                validate_agent_capabilities(&capabilities)?;
            }
            _ => return Err("Agent handshake failed".into()),
        }
        stream.set_read_timeout(None)?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        let command = tree.create_authority("authority", "")?;
        tree.record_runtime_state(
            "authority",
            ascout::control_plane::agent_control::RuntimeExecutionState::Ready,
        )?;
        let decisions = Decision::ALL
            .into_iter()
            .map(|decision| {
                Ok((
                    decision.as_str().to_string(),
                    json!(policy.script(decision)?),
                ))
            })
            .collect::<Result<serde_json::Map<_, _>, Box<dyn std::error::Error>>>()?;
        send(
            &mut stream,
            AgentCommand::LocalPolicy {
                branch_id: "authority".into(),
                policy_digest: policy.digest().into(),
                decisions: serde_json::Value::Object(decisions),
            },
        )?;
        send(&mut stream, command)?;
        interface.publish(&tree, false, false, "Starting Agent session…");
        tui_thread = if let Some(address) = &web_address {
            web::start(address, interface.clone())?;
            None
        } else {
            Some(tui::start(interface.clone())?)
        };
        let result = execution::run(
            &mut stream,
            &mut tree,
            &policy,
            &session,
            &runtime_capabilities,
            &interface,
            (events, receiver),
        );
        result
    })();
    // 先让 Runtime 停掉环境里的 Agent 进程，再断开 Adapter 让它自行退出。
    let runtime_result: Result<(), Box<dyn std::error::Error>> = (|| {
        // Every retained Branch refers to this same shared environment. A
        // confirmed environment fact therefore applies to all of them, not
        // only the last Authority. Archived records remain historical facts.
        let record_shared =
            |tree: &mut SpeculativeTree, state| -> Result<(), Box<dyn std::error::Error>> {
                let ids: Vec<_> = tree.branches.keys().cloned().collect();
                for id in ids {
                    tree.record_runtime_state(&id, state)?;
                }
                Ok(())
            };
        if tree.authority_branch_id.is_some() {
            if runtime_capabilities.shared_environment {
                record_shared(
                    &mut tree,
                    ascout::control_plane::agent_control::RuntimeExecutionState::TerminationRequested,
                )?;
                if let Err(error) =
                    runtime_call(&session, "authority", Operation::TerminateEnvironment)
                {
                    record_shared(
                        &mut tree,
                        ascout::control_plane::agent_control::RuntimeExecutionState::Failed,
                    )?;
                    return Err(error);
                }
                record_shared(
                    &mut tree,
                    ascout::control_plane::agent_control::RuntimeExecutionState::Terminated,
                )?;
                if let Err(error) =
                    runtime_call(&session, "authority", Operation::CleanupEnvironment)
                {
                    record_shared(
                        &mut tree,
                        ascout::control_plane::agent_control::RuntimeExecutionState::Failed,
                    )?;
                    return Err(error);
                }
                record_shared(
                    &mut tree,
                    ascout::control_plane::agent_control::RuntimeExecutionState::Cleaned,
                )?;
            } else {
                let ids: Vec<_> = tree
                    .branches
                    .values()
                    .filter(|branch| {
                        branch.execution_state.runtime
                            != ascout::control_plane::agent_control::RuntimeExecutionState::Cleaned
                    })
                    .map(|branch| branch.branch_id.clone())
                    .collect();
                let mut failures = Vec::new();
                for id in ids {
                    if let Err(error) =
                        runtime_cleanup(&session, &runtime_capabilities, &id, &mut tree)
                    {
                        failures.push(format!("{id}: {error}"));
                    }
                }
                if !failures.is_empty() {
                    return Err(failures.join("; ").into());
                }
            }
        }
        Ok(())
    })();
    interface.publish(&tree, false, false, "Stopped");
    interface.disconnect();
    if let Some(thread) = tui_thread {
        let _ = thread.join();
    }
    // Cleanup failure must not skip closing the cloned reader socket or
    // reaping the Adapter. Preserve every failure instead of hiding one.
    let mut errors = Vec::new();
    if let Err(error) = result {
        errors.push(format!("Agent execution/shutdown: {error}"));
    }
    if let Err(error) = runtime_result {
        errors.push(format!("Runtime cleanup: {error}"));
    }
    let final_state = serde_json::to_vec_pretty(&tree)
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

fn send(stream: &mut UnixStream, command: AgentCommand) -> io::Result<()> {
    transport::write_message(stream, &AgentWireMessage::Command(command))
}
