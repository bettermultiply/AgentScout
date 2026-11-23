use super::{execution, tui, ui};
use ascout::control_plane::{
    coordination::{execution as coordinator, session::Prepared},
    gateway::{
        agent,
        runtime::{ControlServer, ControlSession},
    },
    ledger::Ledger,
    system_log::SystemLog,
    ui_projection::UiProjection,
};
use ascout::protocol::contract::RuntimeCapabilities;
use serde_json::Value;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use std::{
    env, fs, io,
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
};

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
    let installed = env::var_os("HOME").map(|home| {
        std::path::PathBuf::from(home).join(".local/share/ascout/instrumentation/pi/main.ts")
    });
    let path = env::var_os("ASCOUT_AGENT_ENTRY")
        .map(std::path::PathBuf::from)
        .or_else(|| installed.filter(|path| path.is_file()))
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

fn start_adapter(
    config_path: &std::path::Path,
    run_dir: &std::path::Path,
    adapter_override: Option<&Path>,
) -> Result<(std::process::Child, ControlSession, RuntimeCapabilities), Box<dyn std::error::Error>>
{
    let mut config: Value = serde_json::from_slice(&fs::read(config_path)?)?;
    let address = config["control_address"]
        .as_str()
        .ok_or("runtime config missing control_address")?;
    let server = ControlServer::bind(address)
        .map_err(|e| format!("Cannot bind Runtime control address {address}: {e}"))?;
    server.set_nonblocking(true)?;
    if address.ends_with(":0") {
        let actual = server.local_addr()?.to_string();
        config["control_address"] = Value::String(actual);
        fs::write(config_path, serde_json::to_vec_pretty(&config)?).map_err(|e| {
            format!(
                "Cannot update generated Runtime config {}: {e}",
                config_path.display()
            )
        })?;
    }
    let nearby = env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|dir| dir.join("ascout-runtime-adapter")));
    let profile = env::current_exe().ok().and_then(|path| {
        path.parent()
            .and_then(|directory| directory.file_name())
            .and_then(|name| name.to_str())
            .map(str::to_owned)
    });
    let profile_binary = profile.as_deref().map(|name| {
        PathBuf::from("target")
            .join(name)
            .join("ascout-runtime-adapter")
    });
    let binary = adapter_override
        .map(PathBuf::from)
        .or_else(|| env::var_os("ASCOUT_RUNTIME_ADAPTER").map(PathBuf::from))
        .or(nearby)
        .or(profile_binary)
        .or_else(|| find_in_path("ascout-runtime-adapter"))
        .filter(|path| path.is_file())
        .ok_or(
            "Runtime Adapter executable not found. Set ASCOUT_RUNTIME_ADAPTER or install AScout",
        )?;
    let log_path = run_dir.join("adapter.log");
    let log = fs::File::create(&log_path)?;
    let mut adapter = Command::new(binary)
        // A separate process group lets Ctrl-C use the tree's shutdown protocol.
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
            let _ = session.shutdown();
            let _ = adapter.kill();
            let _ = adapter.wait();
            return Err("Runtime Adapter handshake timed out".into());
        }
    };
    Ok((adapter, session, hello.capabilities))
}

pub fn lock_run(run_id: &str, run_dir: &Path) -> Result<fs::File, Box<dyn std::error::Error>> {
    if run_dir.file_name().and_then(|name| name.to_str()) != Some(&run_id) {
        return Err("Run directory identity mismatch".into());
    }
    let run_lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(run_dir.join("run.lock"))?;
    run_lock
        .try_lock()
        .map_err(|error| format!("Cannot lock run {run_id}: {error}"))?;
    Ok(run_lock)
}

pub fn run(
    mut prepared: Prepared,
    pi_root: &Path,
    pi_agent_dir: Option<&Path>,
    web_address: Option<&str>,
    system_log: &SystemLog,
    adapter_override: Option<&Path>,
) -> Result<(), Box<dyn std::error::Error>> {
    let socket = prepared.run_dir().join("agent.sock");
    let listener = agent::Listener::bind(&socket)?;
    let (mut adapter, session, capabilities) = start_adapter(
        prepared.runtime_config(),
        prepared.run_dir(),
        adapter_override,
    )?;
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        prepared.bind_runtime(&capabilities)?;
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
        let settings = prepared.agent_settings();
        if let Some(model) = &settings.model {
            agent_args.extend(["--model".into(), model.clone()]);
        }
        if let Some(models) = &settings.models {
            agent_args.extend(["--models".into(), models.display().to_string()]);
        }
        if let Some(thinking) = &settings.thinking {
            agent_args.extend(["--thinking".into(), thinking.clone()]);
        }
        let node = resolve_node()?;
        let installed_provider = env::var_os("HOME").map(|home| {
            PathBuf::from(home).join(".local/share/ascout/instrumentation/pi/policy-provider.ts")
        });
        let provider_entry = env::var_os("ASCOUT_POLICY_PROVIDER")
            .map(PathBuf::from)
            .or_else(|| installed_provider.filter(|path| path.is_file()))
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR")).join("instrumentation/pi/policy-provider.ts")
            });
        let mut provider_args = vec![
            agent_args[0].clone(),
            agent_args[1].clone(),
            provider_entry.display().to_string(),
        ];
        if let Some(models) = &settings.models {
            provider_args.extend(["--models".into(), models.display().to_string()]);
        }
        let mut env = std::collections::BTreeMap::from([(
            "ASCOUT_PI_ROOT".into(),
            pi_root.display().to_string(),
        )]);
        if let Some(directory) = pi_agent_dir {
            let directory = directory
                .to_str()
                .ok_or("Pi agent directory must be UTF-8")?;
            env.insert("PI_CODING_AGENT_DIR".into(), directory.into());
        }
        prepared.bind_policy_provider(ascout::control_plane::policy::ModelProviderProcess {
            executable: node.clone(),
            arguments: provider_args,
            environment: env.clone(),
        });
        let launch =
            prepared.agent_launch(&capabilities, node.display().to_string(), agent_args, env)?;
        let (events, receiver) = mpsc::channel();
        let runtime_events = events.clone();
        session.set_event_waker(move || {
            let _ = runtime_events.send(coordinator::Event::Timer);
        });
        let ledger = Ledger::open(&prepared.run_dir().join("events.jsonl"))?;
        let projection = UiProjection::new(ledger.reader()).with_log(system_log.clone());
        let interface = ui::Interface::new(projection.clone());
        let ui_connection = coordinator::ui_connection(projection.clone(), events.clone());
        let mut tui_thread = None;
        let mut web_server = None;
        let signals = events.clone();
        let mut errors = coordinator::execute(
            prepared,
            ledger,
            system_log,
            &interface,
            &session,
            &capabilities,
            launch,
            &listener,
            (events, receiver),
            || {
                if let Some(address) = web_address {
                    web_server = Some(ascout::control_plane::gateway::ui::start(
                        address,
                        interface.clone(),
                        ui_connection,
                        projection,
                        ascout::control_plane::gateway::ui::Assets {
                            index_html: include_bytes!("../../../webui/dist/index.html"),
                            app_js: include_bytes!("../../../webui/dist/assets/app.js"),
                            app_css: include_bytes!("../../../webui/dist/assets/app.css"),
                            favicon_svg: include_bytes!("../../../webui/dist/favicon.svg"),
                        },
                    )?);
                } else {
                    tui_thread = Some(tui::start(interface.clone(), ui_connection, projection)?);
                }
                Ok(execution::forward(signals)?)
            },
        );
        if let Some(thread) = tui_thread {
            let _ = thread.join();
        }
        if let Some(mut server) = web_server {
            if let Err(error) = server.stop() {
                errors.push(format!("UI shutdown: {error}"));
            }
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
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; ").into())
    }
}
