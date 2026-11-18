//! Execution and policy coordination. Presentations consume the UI interface.

use std::env;
use std::fs;
use std::io;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

#[path = "ascout-control-plane/config.rs"]
mod config;
#[path = "ascout-control-plane/execution.rs"]
mod execution;
#[path = "ascout-control-plane/tui.rs"]
mod tui;
#[path = "ascout-control-plane/ui.rs"]
mod ui;
#[path = "ascout-control-plane/view.rs"]
mod view;

use ascout::control_plane::agent_control::{AgentMessage, SpeculativeTree};
use ascout::control_plane::gateway::{
    agent,
    runtime::{ControlServer, ControlSession, Runtime},
};
use ascout::control_plane::ledger::Ledger;
use ascout::control_plane::policy::{CompiledPolicy, PolicyEngine};
use ascout::control_plane::{
    coordination::{execution::cleanup, session as coordination_session},
    recovery,
};
use ascout::protocol::contract::{AgentLaunch, RuntimeCapabilities};
use serde_json::Value;
use serde_json::json;
use sha2::{Digest, Sha256};

static SIGNAL_FD: AtomicI32 = AtomicI32::new(-1);

static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);
static INTERRUPT_REQUESTED: AtomicBool = AtomicBool::new(false);

extern "C" fn request_shutdown(signal: libc::c_int) {
    // Signal handlers must not allocate, lock, or perform transport I/O.
    if signal == libc::SIGINT {
        INTERRUPT_REQUESTED.store(true, Ordering::Release);
    } else {
        SHUTDOWN_REQUESTED.store(true, Ordering::Release);
    }
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
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("instrumentation/pi/main.ts")
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

/// 启动 Runtime Adapter 并完成握手。control plane 只持有连接和进程句柄，
/// 不保存任何环境状态。
fn start_adapter(
    config_path: &std::path::Path,
    run_dir: &std::path::Path,
) -> Result<(std::process::Child, ControlSession, RuntimeCapabilities), Box<dyn std::error::Error>>
{
    let config: Value = serde_json::from_slice(&fs::read(config_path)?)?;
    let address = config["control_address"]
        .as_str()
        .ok_or("runtime config missing control_address")?;
    let server = ControlServer::bind(address)?;
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
    let accepted = (|| -> io::Result<ControlSession> {
        loop {
            match server.accept() {
                Ok(session) => break Ok(session),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    if let Some(status) = adapter.try_wait()? {
                        let detail = fs::read_to_string(&log_path)
                            .unwrap_or_default()
                            .trim()
                            .replace('\n', " | ");
                        return Err(io::Error::other(format!(
                            "Runtime Adapter exited before connecting ({status}); see {}: {detail}",
                            log_path.display()
                        )));
                    }
                    if Instant::now() >= deadline {
                        let _ = adapter.kill();
                        let _ = adapter.wait();
                        let detail = fs::read_to_string(&log_path)
                            .unwrap_or_default()
                            .trim()
                            .replace('\n', " | ");
                        return Err(io::Error::other(format!(
                            "Runtime Adapter did not connect within 10s; see {}: {detail}",
                            log_path.display()
                        )));
                    }
                    thread::sleep(Duration::from_millis(30));
                }
                Err(error) => return Err(error),
            }
        }
    })();
    let session = match accepted {
        Ok(session) => session,
        Err(error) => {
            let _ = adapter.kill();
            let _ = adapter.wait();
            return Err(error.into());
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
    let runtime_option = take_option(&mut args, "--runtime")?;
    let policy_name = take_option(&mut args, "--policy")?;
    let resume_id = take_option(&mut args, "--resume")?;
    let resume = resume_id.is_some();
    let config = config::Config::load()?;
    let system_log = ascout::control_plane::system_log::SystemLog::open(&config.log_path)?;
    if args == ["runs"] {
        return config.list();
    }
    if args.len() != 1 {
        return Err("usage: ascout-control-plane <pi-root> [--policy <name>] [--resume <run-id>] [--runtime <runtime.json>] [--web <address>] [--model <provider/model>] [--models <file>] [--thinking <level>]; ascout-control-plane runs".into());
    }
    let pi_root = fs::canonicalize(&args[0])?;
    let selected_policy = if !resume || policy_name.is_some() {
        Some(config.select_policy(policy_name)?)
    } else {
        None
    };
    let (run_id, run_dir) = config.run(resume_id.as_deref())?;
    if run_dir.file_name().and_then(|name| name.to_str()) != Some(&run_id) {
        return Err("Run directory identity mismatch".into());
    }
    // Hold ownership before reading history, replacing sockets or launching the Agent.
    let run_lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(run_dir.join("run.lock"))?;
    run_lock
        .try_lock()
        .map_err(|error| format!("Cannot lock run {run_id}: {error}"))?;
    eprintln!("Run id: {run_id}");
    struct RunNotice(String);
    impl Drop for RunNotice {
        fn drop(&mut self) {
            eprintln!("Run id: {}", self.0);
        }
    }
    let _run_notice = RunNotice(run_id.clone());
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
    let (mut tree, saved) = coordination_session::prepare(&run_dir, resume)?;
    let recovery_generation = match &saved {
        Some(recovery::Resume::Paused(path)) => Some(path.as_path()),
        _ => None,
    };
    let native_session = match &saved {
        Some(recovery::Resume::Ended(reference)) => Some(reference),
        _ => None,
    };
    let (startup_model, resume_model, models, thinking) = recovery::models(
        &run_dir,
        recovery_generation,
        native_session.is_some(),
        model,
        models,
        thinking,
    )?;
    let engine = PolicyEngine::default();
    let artifact_bytes = if !resume {
        None
    } else {
        match fs::read(&artifact_path) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(format!("Policy artifact {}: {error}", artifact_path.display()).into());
            }
        }
    };
    let policy = if artifact_bytes.is_some() {
        let bytes = artifact_bytes.as_ref().unwrap();
        if resume {
            recovery::validate_policy_artifact(&tree, bytes)?;
        }
        let saved = CompiledPolicy::from_bytes(bytes)?;
        if let Some(path) = selected_policy.as_ref() {
            let supplied = engine.compile(path)?;
            if supplied.digest() != saved.digest() {
                return Err("Recovery Policy changed".into());
            }
        }
        saved
    } else {
        let path = selected_policy
            .as_ref()
            .ok_or("Saved Policy artifact missing; supply --policy <name> explicitly")?;
        engine.compile(path)?
    };
    // Policy references a Runtime configuration, not a saved Runtime. Preserve
    // the original cwd resolution even when the artifact lives inside the run.
    let config_path = runtime_option
        .map(std::ffi::OsString::from)
        .or_else(|| env::var_os("ASCOUT_RUNTIME_CONFIG"))
        .map(std::path::PathBuf::from)
        .or_else(|| {
            if resume {
                tree.runtime_binding()?
                    .get("config_path")?
                    .as_str()
                    .map(std::path::PathBuf::from)
            } else {
                None
            }
        })
        .or_else(|| policy.runtime_config_path().map(std::path::PathBuf::from))
        .ok_or("no Runtime selected: set runtime in the Policy or ASCOUT_RUNTIME_CONFIG")?;
    if !resume {
        tree.set_execution_id(run_id.clone())?;
    }
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
    let socket = run_dir.join("agent.sock");
    let listener = agent::Listener::bind(&socket)?;
    let (mut adapter, session, runtime_capabilities) = start_adapter(&config_path, &run_dir)?;
    // Every fallible assembly step after process launch shares this teardown.
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        engine.confirm_backends(&policy, &runtime_capabilities)?;
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
        // 用哪个 Agent 产品、哪个版本由 control plane 决定；启动它由 Runtime 完成。
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
        let mut agent_env = std::collections::BTreeMap::from([
            ("ASCOUT_RUNTIME_TOOLS".into(), runtime_tools),
            ("ASCOUT_PI_ROOT".into(), pi_root.display().to_string()),
            ("ASCOUT_TASK_ID".into(), tree.execution_id().to_string()),
            (
                "ASCOUT_SESSION_FILE".into(),
                session_file.display().to_string(),
            ),
        ]);
        if let Some(reference) = native_session {
            agent_env.insert("ASCOUT_OPEN_SESSION".into(), serde_json::to_string(reference)?);
        }
        if recovery_generation.is_some() {
            agent_env.insert("ASCOUT_LOAD_SESSION".into(), "1".into());
        }
        let agent_launch = AgentLaunch {
            command: resolve_node()?.display().to_string(),
            args: agent_args,
            cwd: tree.workspace(),
            log: run_dir.join("agent.log"),
            instance_id: Some("agent".into()),
            env: agent_env,
        };
        let (events, receiver) = mpsc::channel();
        let runtime_events = events.clone();
        session.set_event_waker(move || {
            let _ = runtime_events.send(execution::Event::Timer);
        });
        let requests = events.clone();
        let mut ledger = Ledger::open(&run_dir.join("events.jsonl"))?;
        ledger.set_system_log(system_log.clone());
        if let Err(error) = ledger.record(json!({
            "source": "control_plane",
            "message": "session_file",
            "task_id": tree.execution_id(),
            "session_file": session_file,
            "run_id": run_id,
            "bindings": {
                "policy_binding": tree.policy_binding(),
                "runtime_binding": tree.runtime_binding(),
                "capability_view": tree.capability_view(),
                "local_policy_digest": tree.local_policy_digest(),
            },
        })) {
            system_log.history_failure(&run_id, &format!("record append failed: {error}"));
        }
        let projection = ascout::control_plane::ui_projection::UiProjection::new(ledger.reader())
            .with_log(system_log.clone());
        let interface = ui::Interface::new(tree.execution_id().to_string(), projection.clone());
        let ui_connection =
            ascout::control_plane::gateway::ui::Connection::new(projection.clone(), move |request| {
                if matches!(
                    request,
                    ascout::control_plane::gateway::ui::Request::UiPromptAnswer { .. }
                        | ascout::control_plane::gateway::ui::Request::GetCommands { .. }
                ) {
                    let (reply, receiver) = mpsc::channel();
                    requests
                        .send(execution::Event::Interaction(request, reply))
                        .map_err(|_| "Control plane disconnected".to_string())?;
                    return receiver
                        .recv()
                        .map_err(|_| "Control plane disconnected".to_string())?;
                }
                requests
                    .send(execution::Event::Request(request))
                    .map_err(|_| "Control plane disconnected".into())
            },
        );
        let mut tui_thread = None;
        let mut web_server = None;
        let result = (|| -> Result<(), Box<dyn std::error::Error>> {
            let command = coordination_session::start(
                &mut tree,
                &session,
                agent_launch,
                recovery_generation,
            )?;
            let mut connection = listener.accept("agent", Duration::from_secs(30))?;
            let commands = coordination_session::connected(&mut tree, &policy, command)?;
            let mut observation_error = None;
            for content in tree.drain_observations() {
                if let Err(error) = ledger.append_observation(tree.execution_id(), content) {
                    system_log.history_failure(
                        &run_id,
                        &format!("observation append failed: {error}"),
                    );
                    observation_error.get_or_insert(error);
                }
            }
            let _ = observation_error;
            if let Err(error) =
                interface.publish(&mut ledger, false, false, "Starting Agent session…")
            {
                system_log
                    .history_failure(&run_id, &format!("observation append failed: {error}"));
            }
            tui_thread = if let Some(address) = &web_address {
                web_server = Some(ascout::control_plane::gateway::ui::start(
                    address,
                    interface.clone(),
                    ui_connection.clone(),
                    projection.clone(),
                    ascout::control_plane::gateway::ui::Assets {
                        index_html: include_bytes!("../../webui/dist/index.html"),
                        app_js: include_bytes!("../../webui/dist/assets/app.js"),
                        app_css: include_bytes!("../../webui/dist/assets/app.css"),
                        favicon_svg: include_bytes!("../../webui/dist/favicon.svg"),
                    },
                )?);
                None
            } else {
                Some(tui::start(
                    interface.clone(),
                    ui_connection.clone(),
                    projection.clone(),
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
                &system_log,
                (events, receiver),
                &run_dir,
                resume_model,
                commands,
            );
            result
        })();
        // 先让 Runtime 停掉环境里的 Agent 进程，再断开 Adapter 让它自行退出。
        let runtime_result = cleanup(&session, &runtime_capabilities, &mut tree);
        let observation_result = (|| -> io::Result<()> {
            let mut failure = None;
            for content in tree.drain_observations() {
                if let Err(error) = ledger.append_observation(tree.execution_id(), content) {
                    system_log.history_failure(
                        &run_id,
                        &format!("observation append failed: {error}"),
                    );
                    failure.get_or_insert(error);
                }
            }
            if let Err(error) = interface.publish(&mut ledger, false, false, "Stopped") {
                system_log
                    .history_failure(&run_id, &format!("observation append failed: {error}"));
                failure.get_or_insert(error);
            }
            if let Err(error) = interface.disconnect(&mut ledger) {
                system_log
                    .history_failure(&run_id, &format!("observation append failed: {error}"));
                failure.get_or_insert(error);
            }
            failure.map_or(Ok(()), Err)
        })();
        ledger.disconnect_readers();
        if let Some(thread) = tui_thread {
            let _ = thread.join();
        }
        // Cleanup failure must not skip closing the cloned reader socket or
        // reaping the Adapter. Preserve every failure instead of hiding one.
        let mut errors = Vec::new();
        if let Some(mut server) = web_server {
            if let Err(error) = server.stop() {
                errors.push(format!("UI shutdown: {error}"));
            }
        }
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
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; ").into())
        }
    })();
    let mut errors = Vec::new();
    if let Err(error) = result {
        errors.push(error.to_string());
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
