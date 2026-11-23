use serde::Deserialize;
use std::{
    fs,
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
};

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    log_path: Option<String>,
    policy_dir: Option<String>,
    runs_dir: Option<String>,
    default_policy: Option<String>,
    pi_root: Option<String>,
    runtime_adapter: Option<String>,
    runtime_config: Option<String>,
    default_backend: Option<String>,
    web_address: Option<String>,
    default_model: Option<String>,
    default_thinking: Option<String>,
    models: Option<String>,
}

pub struct Config {
    pub log_path: PathBuf,
    pub policy_dir: PathBuf,
    pub runs_dir: PathBuf,
    pub default_policy: String,
    pub default_policy_configured: bool,
    pub pi_root: Option<PathBuf>,
    pub runtime_adapter: Option<PathBuf>,
    pub runtime_config: Option<PathBuf>,
    pub default_backend: String,
    pub web_address: String,
    pub default_model: Option<String>,
    pub default_thinking: Option<String>,
    pub models: Option<PathBuf>,
}

fn expand(value: &str, home: &Path) -> PathBuf {
    value
        .strip_prefix("~/")
        .map(|p| home.join(p))
        .unwrap_or_else(|| value.into())
}

pub fn name(value: &str) -> Result<(), Box<dyn std::error::Error>> {
    if value.is_empty() || value == "." || value == ".." || value.contains(['/', '\\']) {
        return Err("Policy name and run id must be single directory names".into());
    }
    Ok(())
}

impl Config {
    pub fn load() -> Result<Self, Box<dyn std::error::Error>> {
        let home = PathBuf::from(std::env::var_os("HOME").ok_or("HOME is not set")?);
        let path = home.join(".config/ascout/config.toml");
        let file: FileConfig = match fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text)
                .map_err(|error| format!("Cannot read {}: {error}", path.display()))?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => FileConfig::default(),
            Err(e) => return Err(format!("Cannot read {}: {e}", path.display()).into()),
        };
        let default_policy_configured = file.default_policy.is_some();
        let result = Self {
            log_path: expand(
                file.log_path
                    .as_deref()
                    .unwrap_or("~/.local/share/ascout/ascout.log"),
                &home,
            ),
            policy_dir: std::env::var_os("ASCOUT_POLICY_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    expand(
                        file.policy_dir
                            .as_deref()
                            .unwrap_or("~/.local/share/ascout/policies"),
                        &home,
                    )
                }),
            runs_dir: expand(
                file.runs_dir
                    .as_deref()
                    .unwrap_or("~/.local/share/ascout/runs"),
                &home,
            ),
            default_policy: file.default_policy.unwrap_or_else(|| "coding".into()),
            default_policy_configured,
            pi_root: file.pi_root.map(|value| expand(&value, &home)),
            runtime_adapter: file.runtime_adapter.map(|value| expand(&value, &home)),
            runtime_config: file.runtime_config.map(|value| expand(&value, &home)),
            default_backend: file.default_backend.unwrap_or_else(|| "filesystem".into()),
            web_address: file.web_address.unwrap_or_else(|| "127.0.0.1:8080".into()),
            default_model: file.default_model,
            default_thinking: file.default_thinking,
            models: file.models.map(|value| expand(&value, &home)),
        };
        name(&result.default_policy)?;
        Ok(result)
    }

    pub fn select_policy(
        &self,
        explicit: Option<String>,
    ) -> Result<PathBuf, Box<dyn std::error::Error>> {
        let mut selected = explicit
            .clone()
            .unwrap_or_else(|| self.default_policy.clone());
        name(&selected)?;
        if explicit.is_none() && !self.default_policy_configured && io::stdin().is_terminal() {
            let mut policies = Vec::new();
            if let Ok(entries) = fs::read_dir(&self.policy_dir) {
                for entry in entries {
                    let entry = entry?;
                    if entry.path().join("policy.yaml").is_file()
                        && entry.path().join("decisions.py").is_file()
                    {
                        policies.push(entry.file_name().to_string_lossy().into_owned());
                    }
                }
            }
            policies.sort();
            if !policies.is_empty() {
                eprintln!("Available Policies: {}", policies.join(", "));
                eprint!("Policy [{selected}]: ");
                io::stderr().flush()?;
                let mut input = String::new();
                io::stdin().read_line(&mut input)?;
                if !input.trim().is_empty() {
                    selected = input.trim().into();
                }
                name(&selected)?;
            }
        }
        let path = self.policy_dir.join(selected).join("policy.yaml");
        if !path.is_file() {
            return Err(format!("Policy not found: {}", path.display()).into());
        }
        Ok(path)
    }

    pub fn run(
        &self,
        resume: Option<&str>,
    ) -> Result<(String, PathBuf), Box<dyn std::error::Error>> {
        if let Some(id) = resume {
            name(id)?;
            let path = self.runs_dir.join(id);
            if !path.is_dir() {
                return Err(format!("Unknown run id: {id}").into());
            }
            return Ok((
                id.into(),
                fs::canonicalize(&path)
                    .map_err(|e| format!("Cannot canonicalize run {}: {e}", path.display()))?,
            ));
        }
        fs::create_dir_all(&self.runs_dir).map_err(|e| {
            format!(
                "Cannot create runs directory {}: {e}",
                self.runs_dir.display()
            )
        })?;
        let directory = tempfile::Builder::new()
            .prefix("run-")
            .tempdir_in(&self.runs_dir)?
            .keep();
        let id = directory.file_name().unwrap().to_str().unwrap().to_string();
        Ok((id, fs::canonicalize(directory)?))
    }

    pub fn list(&self) -> Result<(), Box<dyn std::error::Error>> {
        let entries = match fs::read_dir(&self.runs_dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        let mut ids = Vec::new();
        for entry in entries {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                ids.push(entry.file_name());
            }
        }
        ids.sort();
        for id in ids {
            println!("{}", id.to_string_lossy());
        }
        Ok(())
    }
}
