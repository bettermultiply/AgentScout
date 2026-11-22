use ascout::control_plane::coordination::session;
use std::{
    env, fs,
    path::{Path, PathBuf},
};

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

const HELP: &str = "\
Usage: ascout [pi-root] [options]
       ascout runs

Start AScout in the terminal UI (TUI) by default. The Pi root is optional
when it can be discovered from ASCOUT_PI_ROOT, config, or a sibling checkout.

Commands:
  runs                    List existing runs

Options:
  -h, --help              Show this help and exit
  --web [<address>]       Use the browser UI on a loopback address (default: 127.0.0.1:8080, configurable)
                          Also accepts --web=<address>
  --cwd <dir>             Set the workspace (default: current directory)
  --policy <name>         Select a Policy by name
  --backend <name>        Select the backend when no Runtime config is supplied (filesystem by default)
  --runtime <file>        Use an explicit Runtime JSON config
  --resume <run-id>       Reopen an existing run
  --model <provider/model> Select the Pi model
  --models <file>         Load an additional models file
  --thinking <level>      Set the Pi thinking level
";

fn take_option(
    args: &mut Vec<String>,
    name: &str,
    optional_value: bool,
) -> Result<Option<String>, String> {
    if let Some(index) = args
        .iter()
        .position(|arg| arg == name || arg.strip_prefix(name).is_some_and(|v| v.starts_with('=')))
    {
        let token = args.remove(index);
        if let Some(value) = token.strip_prefix(&format!("{name}=")) {
            return Ok(Some(value.to_owned()));
        }
        if optional_value && (index >= args.len() || args[index].starts_with('-')) {
            return Ok(Some(String::new()));
        }
        if index >= args.len() {
            return Err(format!("{name} requires a value"));
        }
        Ok(Some(args.remove(index)))
    } else {
        Ok(None)
    }
}

fn resolve_pi_root(
    config: &config::Config,
    positional: Option<String>,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let candidate = positional
        .map(PathBuf::from)
        .or_else(|| env::var_os("ASCOUT_PI_ROOT").map(PathBuf::from))
        .or_else(|| config.pi_root.clone())
        .or_else(|| {
            let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../pi");
            path.is_dir().then_some(path)
        })
        .ok_or(
            "Pi root was not found. Set ASCOUT_PI_ROOT, configure pi_root, or run make install",
        )?;
    fs::canonicalize(&candidate)
        .map_err(|e| format!("Pi root {} cannot be opened: {e}", candidate.display()).into())
}

fn resolve_models(
    explicit: Option<PathBuf>,
    config: &config::Config,
) -> Result<Option<PathBuf>, Box<dyn std::error::Error>> {
    if let Some(path) = explicit {
        return Ok(Some(path));
    }
    if let Some(path) = &config.models {
        return fs::canonicalize(path)
            .map(Some)
            .map_err(|e| format!("Models file {} cannot be opened: {e}", path.display()).into());
    }
    let candidates = env::var_os("PI_CODING_AGENT_DIR")
        .map(PathBuf::from)
        .map(|directory| directory.join("models.json"))
        .into_iter()
        .chain(env::var_os("HOME").map(|home| {
            PathBuf::from(home).join(".pi/agent/models.json")
        }));
    for candidate in candidates {
        if candidate.is_file() {
            return fs::canonicalize(&candidate).map(Some).map_err(|e| {
                format!("Models file {} cannot be opened: {e}", candidate.display()).into()
            });
        }
    }
    Ok(None)
}

fn make_runtime_config(
    run_dir: &Path,
    workspace: &Path,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let path = run_dir.join("runtime-config.json");
    let value = serde_json::json!({"backend":"filesystem", "control_address":"127.0.0.1:0", "adapter_instance":"ascout-auto", "filesystem":{"state_directory":run_dir.join("fs-state"), "isolated_paths":[workspace]}});
    fs::write(&path, serde_json::to_vec_pretty(&value)?).map_err(|e| {
        format!(
            "Cannot write generated Runtime config {}: {e}",
            path.display()
        )
    })?;
    Ok(path)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _telemetry = ascout::telemetry::init("ascout-control");
    let mut args: Vec<String> = env::args().skip(1).collect();
    if args.iter().any(|arg| matches!(arg.as_str(), "--help" | "-h")) {
        print!("{HELP}");
        return Ok(());
    }
    execution::install_shutdown_handlers()?;
    let model = take_option(&mut args, "--model", false)?;
    let thinking = take_option(&mut args, "--thinking", false)?;
    let web_value = take_option(&mut args, "--web", true)?;
    let explicit_models = take_option(&mut args, "--models", false)?
        .map(|value| {
            fs::canonicalize(&value)
                .map_err(|e| format!("Models file {value} cannot be opened: {e}"))
        })
        .transpose()?;
    let runtime_option = take_option(&mut args, "--runtime", false)?;
    let policy_name = take_option(&mut args, "--policy", false)?;
    let resume_id = take_option(&mut args, "--resume", false)?;
    let backend_option = take_option(&mut args, "--backend", false)?;
    let cwd = take_option(&mut args, "--cwd", false)?;
    let resume = resume_id.is_some();
    let config = config::Config::load()?;
    let models = resolve_models(explicit_models, &config)?;
    let system_log = ascout::control_plane::system_log::SystemLog::open(&config.log_path)?;
    if args == ["runs"] {
        return config.list();
    }
    if args.len() > 1 {
        return Err("Usage: ascout [pi-root] [options]. See ascout --help.".into());
    }
    let pi_root = resolve_pi_root(&config, args.pop())?;
    let workspace = fs::canonicalize(cwd.map(PathBuf::from).unwrap_or(env::current_dir()?))
        .map_err(|e| format!("Workspace cannot be opened: {e}"))?;
    let selected_policy = if !resume || policy_name.is_some() {
        Some(config.select_policy(policy_name)?)
    } else {
        None
    };
    let (run_id, run_dir) = config.run(resume_id.as_deref())?;
    if !resume && run_dir.starts_with(&workspace) {
        return Err(format!("Workspace {} contains the run directory {}. Choose a project subdirectory or use --cwd so AScout's internal fs-state does not overlap the isolated workspace.", workspace.display(), run_dir.display()).into());
    }
    let runtime = if let Some(path) = runtime_option
        .map(PathBuf::from)
        .or_else(|| env::var_os("ASCOUT_RUNTIME_CONFIG").map(PathBuf::from))
        .or_else(|| config.runtime_config.clone())
    {
        let value: serde_json::Value = serde_json::from_slice(
            &fs::read(&path)
                .map_err(|e| format!("Runtime config {} cannot be opened: {e}", path.display()))?,
        )
        .map_err(|e| format!("Runtime config {} is not valid JSON: {e}", path.display()))?;
        if !resume {
            validate_explicit_runtime_workspace(&path, &value, &workspace)?;
        }
        path
    } else if backend_option.as_deref().unwrap_or(&config.default_backend) == "filesystem" {
        make_runtime_config(&run_dir, &workspace)?
    } else {
        return Err("No Runtime selected. Configure runtime_config, use --runtime, or select filesystem backend".into());
    };
    let _run_lock = foreground::lock_run(&run_id, &run_dir)?;
    let _run_notice = RunNotice::new(&run_id);
    let prepared = session::Prepared::new(
        run_id,
        run_dir,
        resume,
        selected_policy,
        Some(runtime),
        session::AgentSettings {
            model: model.or(config.default_model),
            models,
            thinking: thinking.or(config.default_thinking),
        },
        Some(workspace),
    )?;
    let web_address = web_value
        .map(|value| {
            if value.is_empty() {
                config.web_address.clone()
            } else {
                value
            }
        })
        .filter(|value| !value.is_empty());
    foreground::run(
        prepared,
        &pi_root,
        web_address.as_deref(),
        &system_log,
        config.runtime_adapter.as_deref(),
    )
}

fn validate_explicit_runtime_workspace(
    path: &Path,
    config: &serde_json::Value,
    workspace: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    if config.get("filesystem").is_none() {
        return Ok(());
    }
    let entries = config
        .get("filesystem")
        .and_then(|value| value.get("isolated_paths"))
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            format!(
                "Runtime config {} must list workspace {} in filesystem.isolated_paths",
                path.display(),
                workspace.display()
            )
        })?;
    let contains_workspace = entries.iter().any(|entry| {
        let candidate = entry.as_str().map(PathBuf::from).or_else(|| {
            entry
                .get("path")
                .and_then(serde_json::Value::as_str)
                .map(PathBuf::from)
        });
        candidate.is_some_and(|candidate| candidate == workspace)
    });
    if !contains_workspace {
        return Err(format!(
            "Runtime config {} does not isolate workspace {}. Add that path to filesystem.isolated_paths so the Agent workspace is available.",
            path.display(), workspace.display()
        ).into());
    }
    Ok(())
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
