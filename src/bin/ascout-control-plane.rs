//! Command-line entry point for the foreground control plane.

use ascout::control_plane::coordination::session;
use std::{env, fs};

#[path = "ascout-control-plane/config.rs"]
mod config;
#[path = "ascout-control-plane/execution.rs"]
mod execution;
#[path = "ascout-control-plane/foreground.rs"]
mod foreground;
#[path = "ascout-control-plane/tui.rs"]
mod tui;
#[path = "ascout-control-plane/ui.rs"]
mod ui;
#[path = "ascout-control-plane/view.rs"]
mod view;

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
    execution::install_shutdown_handlers()?;
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
    let _run_lock = foreground::lock_run(&run_id, &run_dir)?;
    let _run_notice = RunNotice::new(&run_id);
    let prepared = session::Prepared::new(
        run_id,
        run_dir,
        resume,
        selected_policy,
        runtime_option
            .map(Into::into)
            .or_else(|| env::var_os("ASCOUT_RUNTIME_CONFIG").map(Into::into)),
        session::AgentSettings {
            model,
            models,
            thinking,
        },
    )?;
    foreground::run(prepared, &pi_root, web_address.as_deref(), &system_log)
}

struct RunNotice(String);

impl RunNotice {
    fn new(run_id: &str) -> Self {
        eprintln!("Run id: {run_id}");
        Self(run_id.into())
    }
}

impl Drop for RunNotice {
    fn drop(&mut self) {
        eprintln!("Run id: {}", self.0);
    }
}
