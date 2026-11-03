//! NLE workload backend.
//!
//! This backend deliberately keeps NetHack-specific details at the external
//! workload boundary. The generic Adapter sees the environment lifecycle plus
//! the declared `nle_action` and `nethack_guide` tools via
//! `tool_context`/`invoke_tool`; game observation formatting and Guidebook
//! lookup stay in the external bridge.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::backend::{Backend, BackendError, CancelHandle};
use crate::contract::{CommandOutput, EnvironmentState, RuntimeCapabilities, RuntimeTool};

#[derive(Debug, Clone, Deserialize)]
pub struct NleConfig {
    pub control_socket: PathBuf,
    pub adapter_instance: String,
    pub python: PathBuf,
    #[serde(default = "default_bridge")]
    pub bridge: PathBuf,
    #[serde(default = "default_guidebook")]
    pub guidebook: PathBuf,
    #[serde(default)]
    pub env_id: Option<String>,
    #[serde(default)]
    pub seed: Option<u64>,
    /// Simulated action latency.  The workload is slow on purpose so that
    /// speculation has a window to hide its reasoning in.
    #[serde(default)]
    pub action_delay_seconds: Option<f64>,
}

fn default_bridge() -> PathBuf {
    PathBuf::from("examples/nle-runtime-bridge.py")
}

fn default_guidebook() -> PathBuf {
    PathBuf::from("examples/nle/Guidebook.txt")
}

struct Bridge {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    next_id: u64,
}

impl Drop for Bridge {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub struct NleBackend {
    config: NleConfig,
    bridge: Mutex<Bridge>,
    states: Mutex<HashMap<String, EnvironmentState>>,
}

impl NleBackend {
    pub fn new(config: NleConfig) -> Result<Self, BackendError> {
        let mut command = Command::new(&config.python);
        command
            .arg(&config.bridge)
            .arg("--env-id")
            .arg(config.env_id.as_deref().unwrap_or("NetHackScore-v0"))
            .arg("--delay")
            .arg(config.action_delay_seconds.unwrap_or(0.0).to_string())
            .arg("--guidebook")
            .arg(&config.guidebook)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        if !config.bridge.is_file() {
            return Err(BackendError::new(format!(
                "NLE bridge not found: {}",
                config.bridge.display()
            )));
        }
        let mut child = command.spawn().map_err(|error| {
            BackendError::new(format!(
                "failed to start NLE Python {}: {error}",
                config.python.display()
            ))
        })?;
        let input = child
            .stdin
            .take()
            .ok_or_else(|| BackendError::new("NLE bridge stdin unavailable"))?;
        let output = child
            .stdout
            .take()
            .ok_or_else(|| BackendError::new("NLE bridge stdout unavailable"))?;
        Ok(Self {
            config,
            bridge: Mutex::new(Bridge {
                child,
                input,
                output: BufReader::new(output),
                next_id: 1,
            }),
            states: Mutex::new(HashMap::new()),
        })
    }

    fn request(&self, mut request: Value) -> Result<Value, BackendError> {
        let mut bridge = self
            .bridge
            .lock()
            .map_err(|_| BackendError::new("NLE bridge lock poisoned"))?;
        let id = bridge.next_id.to_string();
        bridge.next_id += 1;
        request["id"] = Value::String(id);
        serde_json::to_writer(&mut bridge.input, &request).map_err(error)?;
        bridge.input.write_all(b"\n").map_err(error)?;
        bridge.input.flush().map_err(error)?;
        let mut line = String::new();
        bridge.output.read_line(&mut line).map_err(error)?;
        if line.is_empty() {
            return Err(BackendError::new("NLE bridge closed its output"));
        }
        let response: Value = serde_json::from_str(&line).map_err(error)?;
        if response.get("ok") != Some(&Value::Bool(true)) {
            return Err(BackendError::new(
                response["error"]
                    .as_str()
                    .unwrap_or("NLE bridge request failed"),
            ));
        }
        Ok(response)
    }

    fn set_state(&self, id: &str, state: EnvironmentState) {

        self.states.lock().unwrap().insert(id.to_string(), state);
    }

    fn observe(&self, environment_id: &str) -> Result<Value, BackendError> {
        let response = self.request(json!({"op":"observe", "environment":environment_id}))?;
        Ok(response.get("observation").cloned().unwrap_or(Value::Null))
    }

    fn step(&self, environment_id: &str, action: u64) -> Result<Value, BackendError> {
        let response =
            self.request(json!({"op":"operate", "environment":environment_id, "action":action}))?;
        if response.get("terminated") == Some(&Value::Bool(true))
            || response.get("truncated") == Some(&Value::Bool(true))
        {
            self.set_state(environment_id, EnvironmentState::Terminated);
        }
        Ok(response.get("observation").cloned().unwrap_or(Value::Null))
    }

}

fn error(error: impl std::fmt::Display) -> BackendError {
    BackendError::new(error.to_string())
}

impl Backend for NleBackend {
    fn capabilities(&self) -> RuntimeCapabilities {
        RuntimeCapabilities {
            tools: vec![
                RuntimeTool {
                    name: "nle_action".into(),
                    description: "Perform one visible NetHack action through AScout. The action must be an integer in the game's action space. Action 0 only confirms an active --More-- or confirmation prompt; using it in a normal turn returns an unknown-command message.".into(),
                    parameters: json!({
                        "type": "object",
                        "properties": {"action": {"type": "integer"}},
                        "required": ["action"]
                    }),
                },
                RuntimeTool {
                    name: "nethack_guide".into(),
                    description: "Read the official NetHack 3.6 Guidebook. Call with no section for the table of contents, or with a numbered section such as '3.3' for map symbols or '4' for commands.".into(),
                    parameters: json!({
                        "type": "object",
                        "properties": {"section": {"type": "string"}},
                        "required": []
                    }),
                },
            ],
            shared_environment: true,
            branch_environment: false,
        }
    }

    fn establish(&self, environment_id: &str) -> Result<(), BackendError> {
        let mut request = json!({"op":"establish", "environment":environment_id});
        if let Some(seed) = self.config.seed {
            request["seed"] = json!(seed);
        }
        self.request(request)?;
        self.set_state(environment_id, EnvironmentState::Available);
        Ok(())
    }

    fn operate(
        &self,
        environment_id: &str,
        command: &[String],
        cancel: &CancelHandle,
    ) -> Result<CommandOutput, BackendError> {
        if cancel.is_cancelled() {
            return Err(BackendError::interrupted());
        }
        if command.len() == 1 && command[0] == "observe" {
            let observation = self.observe(environment_id)?;
            return Ok(CommandOutput {
                exit_code: 0,
                stdout: serde_json::to_string(&observation).map_err(error)?,
                stderr: String::new(),
            });
        }
        if command.len() != 2 || command[0] != "action" {
            return Err(BackendError::new(
                "NLE operate requires ['action', '<integer>']",
            ));
        }
        let action: u64 = command[1]
            .parse()
            .map_err(|_| BackendError::new("NLE action must be an integer"))?;
        let observation = self.step(environment_id, action)?;
        Ok(CommandOutput {
            exit_code: 0,
            stdout: serde_json::to_string(&observation).map_err(error)?,
            stderr: String::new(),
        })
    }

    fn tool_context(
        &self,
        environment_id: &str,
        tool_name: &str,
        cancel: &CancelHandle,
    ) -> Result<CommandOutput, BackendError> {
        if cancel.is_cancelled() {
            return Err(BackendError::interrupted());
        }
        if tool_name == "nethack_guide" {
            return Ok(CommandOutput {
                exit_code: 0,
                stdout: "{}".into(),
                stderr: String::new(),
            });
        }
        if tool_name != "nle_action" {
            return Err(BackendError::new("unknown NLE runtime tool"));
        }
        let observation = self.observe(environment_id)?;
        Ok(CommandOutput {
            exit_code: 0,
            stdout: serde_json::to_string(&observation).map_err(error)?,
            stderr: String::new(),
        })
    }

    fn invoke_tool(
        &self,
        environment_id: &str,
        tool_name: &str,
        arguments: &Value,
        cancel: &CancelHandle,
    ) -> Result<CommandOutput, BackendError> {
        if cancel.is_cancelled() {
            return Err(BackendError::interrupted());
        }
        let response = self.request(json!({
            "op": "invoke",
            "environment": environment_id,
            "tool": tool_name,
            "arguments": arguments,
        }))?;
        let result = response
            .get("result")
            .cloned()
            .ok_or_else(|| BackendError::new("NLE tool result missing"))?;
        let evidence = response
            .get("evidence")
            .cloned()
            .unwrap_or_else(|| json!({"is_error": false}));
        let payload = json!({"result": result, "evidence": evidence});
        Ok(CommandOutput {
            exit_code: 0,
            stdout: serde_json::to_string(&payload).map_err(error)?,
            stderr: String::new(),
        })
    }

    fn branch(&self, parent: &str, child: &str) -> Result<(), BackendError> {
        self.request(json!({"op":"branch", "environment":parent, "child":child}))?;
        self.set_state(child, EnvironmentState::Available);
        Ok(())
    }

    fn promote(&self, environment_id: &str) -> Result<(), BackendError> {
        // NLE shares one world and has no environment-side promotion; the
        // operation is still sent so the generic Runtime lifecycle stays intact.
        self.request(json!({"op":"promote", "environment":environment_id}))?;
        Ok(())
    }

    fn suspend(&self, environment_id: &str) -> Result<(), BackendError> {
        self.request(json!({"op":"suspend", "environment":environment_id}))?;
        self.set_state(environment_id, EnvironmentState::Suspended);
        Ok(())
    }

    fn resume(&self, environment_id: &str) -> Result<(), BackendError> {
        self.request(json!({"op":"resume", "environment":environment_id}))?;
        self.set_state(environment_id, EnvironmentState::Available);
        Ok(())
    }

    fn terminate(&self, environment_id: &str) -> Result<(), BackendError> {
        self.request(json!({"op":"terminate", "environment":environment_id}))?;
        self.set_state(environment_id, EnvironmentState::Terminated);
        Ok(())
    }

    fn cleanup(&self, environment_id: &str) -> Result<(), BackendError> {
        self.request(json!({"op":"cleanup", "environment":environment_id}))?;
        self.set_state(environment_id, EnvironmentState::Cleaned);
        Ok(())
    }

    fn state(&self, environment_id: &str) -> Option<EnvironmentState> {
        self.states.lock().unwrap().get(environment_id).copied()
    }

    fn environments(&self) -> Vec<String> {
        self.states.lock().unwrap().keys().cloned().collect()
    }
}

