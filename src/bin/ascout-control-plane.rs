//! Execution and policy coordination. Presentations consume the UI interface.

use std::env;
use std::fs;
use std::io::{self, IsTerminal};
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

use ascout::control_plane::agent_control::{AgentCommand, AgentMessage, SpeculativeTree};
use ascout::control_plane::gateway::{
    agent,
    runtime::{ControlServer, ControlSession, Runtime},
};
use ascout::control_plane::ledger::Ledger;
use ascout::control_plane::policy::{CompiledPolicy, Decision, PolicyEngine};
use ascout::control_plane::{
    coordination::execution::{cleanup, runtime_call},
    recovery,
};
use ascout::protocol::contract::{AgentLaunch, Operation, RuntimeCapabilities};
use serde_json::Value;
use serde_json::json;
use sha2::{Digest, Sha256};

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
    let hello = match session.wait_for_hello(Duration::from_secs(10)) {
        Some(hello) => hello,
        None => {
            // Failed admission must not leave the process started by assembly behind.
            let _ = session.shutdown();
            let _ = adapter.kill();
            let _ = adapter.wait();
            return Err("Runtime Adapter handshake timed out".into());
        }
    };
    Ok((adapter, session, hello.capabilities))
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
    let models = take_option(&mut args, "--models")?
        .map(fs::canonicalize)
        .transpose()?;
    let policy_artifact =
        take_option(&mut args, "--policy-artifact")?.map(std::path::PathBuf::from);
    let runtime_option = take_option(&mut args, "--runtime")?;
    let resume = if let Some(index) = args.iter().position(|value| value == "--resume") {
        args.remove(index);
        true
    } else {
        false
    };
    if !(2..=3).contains(&args.len())
        || (web_address.is_none() && (!io::stdin().is_terminal() || !io::stdout().is_terminal()))
    {
        return Err(
            "usage: ascout-control-plane <run-dir> <pi-root> [policy.yaml] [--policy-artifact <policy.json>] [--resume] [--runtime <runtime.json>] [--web 127.0.0.1:8080] [--model <provider/model>] [--models <additional-models.json>] [--thinking <level>]".into(),
        );
    }
    // 运行目录由调用方（实验脚手架）创建，控制面只使用它。
    let run_dir = fs::canonicalize(&args[0])?;
    if !run_dir.is_dir() {
        return Err(format!("{} is not a run directory", run_dir.display()).into());
    }
    let pi_root = fs::canonicalize(&args[1])?;
    let artifact_path = run_dir.join("policy.json");
    if !resume
        && [
            "policy.json",
            "events.jsonl",
            "session.jsonl",
            "recovery.json",
            "agent.sock",
        ]
        .iter()
        .any(|name| run_dir.join(name).symlink_metadata().is_ok())
    {
        return Err("Saved Policy exists; refusing to overwrite run data".into());
    }
    // Establish task control state before starting its execution infrastructure.
    let recovered = if resume {
        Some(recovery::load(&run_dir)?)
    } else {
        None
    };
    let recovery_generation = recovered.as_ref().map(|(_, path)| path.clone());
    let (startup_model, resume_model, models) =
        recovery::models(&run_dir, recovery_generation.as_deref(), model, models)?;
    let mut tree = if let Some((tree, _)) = recovered {
        tree
    } else {
        if run_dir.join("recovery.json").exists() {
            return Err("Saved execution exists; use --resume".into());
        }
        SpeculativeTree::with_workspace("agent", &run_dir)?
    };
    let engine = PolicyEngine::default();
    let source_artifact = policy_artifact
        .clone()
        .unwrap_or_else(|| artifact_path.clone());
    let artifact_bytes = if !resume && policy_artifact.is_none() {
        None
    } else {
        match fs::read(&source_artifact) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == io::ErrorKind::NotFound && policy_artifact.is_none() => {
                None
            }
            Err(error) => {
                return Err(
                    format!("Policy artifact {}: {error}", source_artifact.display()).into(),
                );
            }
        }
    };
    let policy = if artifact_bytes.is_some() {
        let bytes = artifact_bytes.as_ref().unwrap();
        if resume {
            recovery::validate_policy_artifact(&tree, bytes)?;
        }
        let saved = CompiledPolicy::from_bytes(bytes)?;
        if let Some(path) = args.get(2) {
            let supplied = engine.compile(std::path::Path::new(path))?;
            if supplied.digest() != saved.digest() {
                return Err("Recovery Policy changed".into());
            }
        }
        saved
    } else {
        let path = args
            .get(2)
            .map(String::as_str)
            .or_else(|| {
                if resume {
                    None
                } else {
                    Some("assets/policy/policy/policy.yaml")
                }
            })
            .ok_or("Saved Policy artifact missing; supply policy.yaml explicitly")?;
        engine.compile(std::path::Path::new(path))?
    };
    // Policy references a Runtime configuration, not a saved Runtime. Preserve
    // the original cwd resolution even when the artifact lives inside the run.
    let config_path = runtime_option
        .map(std::ffi::OsString::from)
        .or_else(|| env::var_os("ASCOUT_RUNTIME_CONFIG"))
        .map(std::path::PathBuf::from)
        .or_else(|| {
            if resume {
                tree.runtime_binding
                    .as_ref()?
                    .get("config_path")?
                    .as_str()
                    .map(std::path::PathBuf::from)
            } else {
                None
            }
        })
        .or_else(|| policy.runtime_config_path().map(std::path::PathBuf::from))
        .ok_or("no Runtime selected: set runtime in the Policy or ASCOUT_RUNTIME_CONFIG")?;
    tree.execution_id = run_dir.display().to_string();
    let config_digest = if resume {
        recovery::validate_bindings(&tree, policy.digest(), &config_path)?
    } else {
        recovery::digest(&config_path)?
    };
    let artifact_digest = if let Some(bytes) = artifact_bytes {
        format!("{:x}", Sha256::digest(bytes))
    } else {
        policy.save(&artifact_path)?;
        // Hash the exact deterministic bytes published by save, not a second
        // read of a file another process could have changed in the meantime.
        format!("{:x}", Sha256::digest(serde_json::to_vec_pretty(&policy)?))
    };
    let policy_binding =
        json!({"source": policy.source(), "artifact": "policy.json", "sha256": artifact_digest});
    if !resume && policy_artifact.is_some() {
        policy.save(&artifact_path)?;
    }
    let socket = run_dir.join("agent.sock");
    let listener = agent::Listener::bind(&socket)?;
    let (mut adapter, session, runtime_capabilities) = start_adapter(&config_path, &run_dir)?;
    if resume {
        recovery::validate_capabilities(&tree, &runtime_capabilities)?;
    }
    tree.bind_execution(
        policy_binding,
        json!({"config_path": config_path,"sha256":config_digest}),
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
    if let Some(model) = startup_model {
        agent_args.push("--model".into());
        agent_args.push(model);
    }
    if let Some(models) = models {
        agent_args.push("--models".into());
        agent_args.push(models.display().to_string());
    }
    if let Some(thinking) = thinking {
        agent_args.push("--thinking".into());
        agent_args.push(thinking);
    }
    let session_file = run_dir.join("session.jsonl");
    let agent_env = std::collections::BTreeMap::from([
        ("ASCOUT_RUNTIME_TOOLS".into(), runtime_tools),
        ("ASCOUT_PI_ROOT".into(), pi_root.display().to_string()),
        ("ASCOUT_TASK_ID".into(), tree.execution_id.clone()),
        (
            "ASCOUT_SESSION_FILE".into(),
            session_file.display().to_string(),
        ),
    ]);
    let agent_launch = AgentLaunch {
        command: resolve_node()?.display().to_string(),
        args: agent_args,
        cwd: tree.workspace(),
        log: run_dir.join("agent.log"),
        instance_id: Some("agent".into()),
        env: agent_env,
    };
    tree.set_agent_launch(agent_launch.clone());
    let (events, receiver) = mpsc::channel();
    let requests = events.clone();
    let mut ledger = Ledger::open(&run_dir.join("events.jsonl"))?;
    ledger.record(json!({
        "source": "control_plane",
        "message": "session_file",
        "task_id": tree.execution_id,
        "session_file": session_file,
    }))?;
    let interface = ui::Interface::new(tree.execution_id.clone(), ledger.reader());
    let ui_connection =
        ascout::control_plane::gateway::ui::Connection::new(ledger.reader(), move |request| {
            requests
                .send(execution::Event::Request(request))
                .map_err(|_| "Control plane disconnected".into())
        });
    let mut tui_thread = None;
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        runtime_call(
            &session,
            "authority",
            if resume {
                Operation::LoadRuntime {
                    path: recovery_generation.as_ref().unwrap().join("runtime.json"),
                }
            } else {
                Operation::EstablishEnvironment
            },
        )?;
        // 环境建立后，由 Runtime 在其中建立 Agent 接入侧。
        runtime_call(
            &session,
            "authority",
            Operation::OperateEnvironment {
                command: Vec::new(),
                agent: Some(agent_launch),
            },
        )?;
        let mut connection = listener.accept("agent", Duration::from_secs(30))?;
        let command = if resume {
            AgentCommand::Load {
                branch_id: tree
                    .authority_branch_id
                    .clone()
                    .ok_or("Recovery authority missing")?,
                path: recovery_generation.as_ref().unwrap().join("agent.json"),
            }
        } else {
            tree.create_authority("authority", "")?
        };
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
        connection.send(AgentCommand::LocalPolicy {
            branch_id: "authority".into(),
            policy_digest: policy.digest().into(),
            decisions: serde_json::Value::Object(decisions),
        })?;
        connection.send(command)?;
        let mut observation_error = None;
        for content in tree.drain_observations() {
            if let Err(error) = ledger.append_observation(&tree.execution_id, content) {
                observation_error.get_or_insert(error);
            }
        }
        if let Some(error) = observation_error {
            return Err(error.into());
        }
        interface.publish(&mut ledger, false, false, "Starting Agent session…")?;
        tui_thread = if let Some(address) = &web_address {
            ascout::control_plane::gateway::ui::start(
                address,
                interface.clone(),
                ui_connection.clone(),
                ledger.reader(),
                ascout::control_plane::gateway::ui::Assets {
                    index_html: include_bytes!("../../webui/dist/index.html"),
                    app_js: include_bytes!("../../webui/dist/assets/app.js"),
                    app_css: include_bytes!("../../webui/dist/assets/app.css"),
                    favicon_svg: include_bytes!("../../webui/dist/favicon.svg"),
                },
            )?;
            None
        } else {
            Some(tui::start(
                interface.clone(),
                ui_connection.clone(),
                ledger.reader(),
            )?)
        };
        let result = execution::run(
            &mut connection,
            &mut tree,
            &policy,
            &engine,
            &session,
            &runtime_capabilities,
            &interface,
            &mut ledger,
            (events, receiver),
            &run_dir,
            resume_model,
        );
        result
    })();
    // 先让 Runtime 停掉环境里的 Agent 进程，再断开 Adapter 让它自行退出。
    let runtime_result = cleanup(&session, &runtime_capabilities, &mut tree);
    let observation_result = (|| -> io::Result<()> {
        let mut failure = None;
        for content in tree.drain_observations() {
            if let Err(error) = ledger.append_observation(&tree.execution_id, content) {
                failure.get_or_insert(error);
            }
        }
        if let Err(error) = interface.publish(&mut ledger, false, false, "Stopped") {
            failure.get_or_insert(error);
        }
        if let Err(error) = interface.disconnect(&mut ledger) {
            failure.get_or_insert(error);
        }
        failure.map_or(Ok(()), Err)
    })();
    if let Some(thread) = tui_thread {
        let _ = thread.join();
    }
    // Cleanup failure must not skip closing the cloned reader socket or
    // reaping the Adapter. Preserve every failure instead of hiding one.
    let mut errors = Vec::new();
    if let Err(error) = observation_result {
        errors.push(format!("Observation history incomplete: {error}"));
    }
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
