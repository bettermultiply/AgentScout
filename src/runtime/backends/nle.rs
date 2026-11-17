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
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Mutex;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::protocol::contract::{
    CommandOutput, EnvironmentState, RuntimeCapabilities, RuntimeTool,
};
use crate::runtime::backend::{Backend, BackendError, CancelHandle};

#[derive(Debug, Clone, Deserialize)]
pub struct NleConfig {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{
        composite::CompositeRuntime,
        selection::{BackendConnection, BackendPolicy},
    };
    use std::sync::Arc;

    #[test]
    fn inflight_cancel_waits_for_bridge_terminal() {
        let root = tempfile::tempdir().unwrap();
        let bridge = root.path().join("bridge.py");
        std::fs::write(
            &bridge,
            r#"
import json,sys,time
pending=None
for line in sys.stdin:
    r=json.loads(line)
    if r['op']=='invoke':
        pending=r['id']
        print(json.dumps({'id':pending,'ok':True,'started':True}),file=sys.stderr,flush=True)
        open(__file__+'.started','w').close()
    elif r['op']=='cancel':
        assert r['id']==pending
        time.sleep(0.15)
        print(json.dumps({'id':pending,'ok':False,'cancelled':True}),flush=True)
    else: print(json.dumps({'id':r['id'],'ok':True}),flush=True)
"#,
        )
        .unwrap();
        let backend = Arc::new(
            NleBackend::new(NleConfig {
                adapter_instance: "fake".into(),
                python: "python3".into(),
                bridge: bridge.clone(),
                guidebook: root.path().join("guide"),
                env_id: None,
                seed: None,
                action_delay_seconds: None,
            })
            .unwrap(),
        );
        backend.establish("world").unwrap();
        let cancel = Arc::new(CancelHandle::default());
        let task = std::thread::spawn({
            let backend = backend.clone();
            let cancel = cancel.clone();
            move || backend.invoke_tool("world", "nle_action", &json!({"action":1}), &cancel)
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !bridge.with_extension("py.started").exists() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        cancel.cancel();
        std::thread::sleep(std::time::Duration::from_millis(30));
        assert!(
            !task.is_finished(),
            "cancel receipt must not settle the operation"
        );
        assert!(task.join().unwrap().unwrap_err().cancelled);
    }

    // The fake workload deliberately has shared world state. Lifecycle must
    // stay in NleBackend, and single-component recovery must restore bindings.
    #[test]
    fn shared_world_lifecycles_and_single_composite_participants_survive_recovery() {
        let root = tempfile::tempdir().unwrap();
        let bridge = root.path().join("bridge.py");
        std::fs::write(&bridge, r#"
import json,sys
worlds={}
states={}
for line in sys.stdin:
    r=json.loads(line); op=r['op']; name=r.get('environment'); reply={'ok':True,'id':r['id']}
    if op=='establish': worlds[name]=0; states[name]='available'
    elif op=='branch': worlds[r['child']]=worlds[name]; states[r['child']]='available'
    elif op=='suspend': states[name]='suspended'
    elif op=='resume': states[name]='available'
    elif op=='observe':
        if name not in worlds or states.get(name)!='available': reply.update(ok=False,error='world unavailable')
        else: reply['observation']={'visible':True}
    elif op=='terminate': states[name]='terminated'
    elif op=='cleanup': worlds.pop(name,None); states[name]='cleaned'
    elif op=='save':
        with open(r['path'],'w') as f: json.dump({'environments':worlds,'environment_states':states},f)
    elif op=='load':
        with open(r['path']) as f: data=json.load(f); worlds=data['environments']; states=data['environment_states']; reply['states']=states
    print(json.dumps(reply),flush=True)
"#).unwrap();
        let config = NleConfig {
            adapter_instance: "nle".into(),
            python: "python3".into(),
            bridge,
            guidebook: root.path().join("guide"),
            env_id: None,
            seed: None,
            action_delay_seconds: None,
        };
        let make = || {
            let backend = Arc::new(NleBackend::new(config.clone()).unwrap());
            CompositeRuntime::new(
                "runtime",
                vec![BackendConnection::new("nle", backend).unwrap()],
                BackendPolicy::single("nle"),
            )
            .unwrap()
        };
        let runtime = make();
        runtime.establish("parent").unwrap();
        runtime.branch("parent", "child").unwrap();
        runtime.suspend("child").unwrap();
        runtime
            .operate("parent", &["observe".into()], &CancelHandle::default())
            .unwrap();
        let save = root.path().join("runtime.json");
        runtime.save_runtime(&save).unwrap();
        let restored = make();
        restored.load_runtime(&save).unwrap();
        assert_eq!(restored.state("parent"), Some(EnvironmentState::Available));
        assert_eq!(restored.state("child"), Some(EnvironmentState::Suspended));
        restored.resume("child").unwrap();
        restored.terminate("parent").unwrap();
        restored.cleanup("parent").unwrap();
        restored
            .operate("child", &["observe".into()], &CancelHandle::default())
            .unwrap();
        let again = root.path().join("again.json");
        restored.save_runtime(&again).unwrap();
        let final_runtime = make();
        final_runtime.load_runtime(&again).unwrap();
        assert_eq!(
            final_runtime.state("parent"),
            Some(EnvironmentState::Cleaned)
        );
        final_runtime
            .operate("child", &["observe".into()], &CancelHandle::default())
            .unwrap();
    }
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
    output: std::sync::mpsc::Receiver<std::io::Result<String>>,
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
    actions: Mutex<()>,
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
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            config,
            bridge: Mutex::new(Bridge {
                child,
                input,
                output: receiver,
                next_id: 1,
            }),
            states: Mutex::new(HashMap::new()),
            actions: Mutex::new(()),
        })
    }

    fn request(&self, request: Value) -> Result<Value, BackendError> {
        self.request_cancellable(request, &CancelHandle::default())
    }

    fn request_cancellable(
        &self,
        mut request: Value,
        cancel: &CancelHandle,
    ) -> Result<Value, BackendError> {
        let mut bridge = loop {
            if cancel.is_cancelled() {
                return Err(BackendError::interrupted());
            }
            match self.bridge.try_lock() {
                Ok(bridge) => break bridge,
                Err(std::sync::TryLockError::WouldBlock) => {
                    std::thread::sleep(std::time::Duration::from_millis(10))
                }
                Err(_) => return Err(BackendError::new("NLE bridge lock poisoned")),
            }
        };
        let id = bridge.next_id.to_string();
        bridge.next_id += 1;
        request["id"] = Value::String(id.clone());
        serde_json::to_writer(&mut bridge.input, &request).map_err(error)?;
        bridge.input.write_all(b"\n").map_err(error)?;
        bridge.input.flush().map_err(error)?;
        let mut stopping = false;
        let line = loop {
            if cancel.is_cancelled() && !stopping {
                serde_json::to_writer(&mut bridge.input, &json!({"op":"cancel","id":id}))
                    .map_err(error)?;
                bridge.input.write_all(b"\n").map_err(error)?;
                bridge.input.flush().map_err(error)?;
                stopping = true;
            }
            match bridge
                .output
                .recv_timeout(std::time::Duration::from_millis(10))
            {
                Ok(line) => break line.map_err(error)?,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => return Err(BackendError::new("NLE bridge closed its output")),
            }
        };
        if line.is_empty() {
            return Err(BackendError::new("NLE bridge closed its output"));
        }
        let response: Value = serde_json::from_str(&line).map_err(error)?;
        if response.get("id").and_then(Value::as_str) != Some(id.as_str()) {
            return Err(BackendError::new("NLE bridge response identity mismatch"));
        }
        if response.get("cancelled") == Some(&Value::Bool(true)) {
            return Err(BackendError::interrupted());
        }
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
    fn save_runtime(&self, path: &std::path::Path) -> Result<(), BackendError> {
        // Lifecycle belongs to each Environment, not the shared workload world.
        // Include it in the bridge's existing atomic save publication.
        self.request(json!({"op":"save", "path":path,
            "environment_states": &*self.states.lock().unwrap()}))?;
        Ok(())
    }

    fn load_runtime(&self, path: &std::path::Path) -> Result<(), BackendError> {
        let record: Value =
            serde_json::from_slice(&std::fs::read(path).map_err(error)?).map_err(error)?;
        let saved_states: HashMap<String, EnvironmentState> =
            serde_json::from_value(record["environment_states"].clone()).map_err(error)?;
        let response = self.request(json!({"op":"load", "path":path}))?;
        let states = serde_json::from_value(response["states"].clone()).unwrap_or(saved_states);
        *self.states.lock().unwrap() = states;
        Ok(())
    }
    fn capabilities(&self) -> RuntimeCapabilities {
        RuntimeCapabilities {
            backends: Vec::new(),
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
            branch_environment: true,
            filesystem_fast_delete: None,
            filesystem_fork_cost: None,
            filesystem_btrfs_incremental_diff: None,
            freeze_mechanism: None,
            filesystem_passthrough: None,
            state_directory_overlaps_mount: None,
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
        // Serialize actions in the shared world without blocking lifecycle
        // requests behind their simulated latency.
        let _action = if tool_name == "nle_action" {
            Some(loop {
                if cancel.is_cancelled() {
                    return Err(BackendError::interrupted());
                }
                match self.actions.try_lock() {
                    Ok(guard) => break guard,
                    Err(std::sync::TryLockError::WouldBlock) => {
                        std::thread::sleep(std::time::Duration::from_millis(10))
                    }
                    Err(_) => return Err(BackendError::new("NLE action lock poisoned")),
                }
            })
        } else {
            None
        };
        if tool_name == "nle_action" {
            let deadline = std::time::Instant::now()
                + std::time::Duration::from_secs_f64(
                    self.config.action_delay_seconds.unwrap_or(0.0).max(0.0),
                );
            while std::time::Instant::now() < deadline {
                if cancel.is_cancelled() {
                    return Err(BackendError::interrupted());
                }
                std::thread::sleep(
                    deadline
                        .saturating_duration_since(std::time::Instant::now())
                        .min(std::time::Duration::from_millis(10)),
                );
            }
        }
        let response = self.request_cancellable(
            json!({
                "op": "invoke",
                "environment": environment_id,
                "tool": tool_name,
                "arguments": arguments,
            }),
            cancel,
        )?;
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
        if self.state(environment_id) != Some(EnvironmentState::Available) {
            return Err(BackendError::new("NLE environment is not available"));
        }
        self.request(json!({"op":"suspend", "environment":environment_id}))?;
        self.set_state(environment_id, EnvironmentState::Suspended);
        Ok(())
    }

    fn resume(&self, environment_id: &str) -> Result<(), BackendError> {
        if self.state(environment_id) != Some(EnvironmentState::Suspended) {
            return Err(BackendError::new("NLE environment is not suspended"));
        }
        self.request(json!({"op":"resume", "environment":environment_id}))?;
        self.set_state(environment_id, EnvironmentState::Available);
        Ok(())
    }

    fn terminate(&self, environment_id: &str) -> Result<(), BackendError> {
        if matches!(
            self.state(environment_id),
            Some(EnvironmentState::Terminated | EnvironmentState::Cleaned)
        ) {
            return Ok(());
        }
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
