
use std::collections::BTreeMap;
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::compile::{Binding, CompiledPolicy, Definition};
use super::decision::Decision;
use super::validation::{
    check_json, check_yaml, invalid, open_source, read_limited, require, source_path,
    validate_binding, validate_data, validate_result,
};
use super::{MAX_BYTES, MAX_FILES};

pub struct PolicyEngine {
    python: PathBuf,
    bubblewrap: PathBuf,
    timeout: Duration,
    worker: std::sync::Mutex<BTreeMap<String, super::worker::Worker>>,
}

fn resolve_executable(variable: &str, name: &str, fallback: &str) -> PathBuf {
    if let Some(value) = env::var_os(variable) {
        return PathBuf::from(value);
    }
    if let Some(path) = find_in_path(name) {
        return path;
    }
    PathBuf::from(fallback)
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    find_in_path_value(&path, name)
}

fn find_in_path_value(path: &std::ffi::OsStr, name: &str) -> Option<PathBuf> {
    env::split_paths(path)
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
}

impl Default for PolicyEngine {
    fn default() -> Self {
        Self {
            python: if let Some(path) = env::var_os("ASCOUT_POLICY_PYTHON") {
                PathBuf::from(path)
            } else if Path::new("/usr/bin/python3").is_file() {
                PathBuf::from("/usr/bin/python3")
            } else {
                find_in_path("python3").unwrap_or_else(|| PathBuf::from("python3"))
            },
            bubblewrap: resolve_executable("ASCOUT_POLICY_BWRAP", "bwrap", "/usr/bin/bwrap"),
            timeout: Duration::from_secs(5),
            worker: std::sync::Mutex::new(BTreeMap::new()),
        }
    }
}

fn require_executable(path: &Path, variable: &str, name: &str) -> io::Result<()> {
    if path.is_file() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "Policy executable {name} not found at {}; set {variable} or add it to PATH",
                path.display()
            ),
        ))
    }
}

impl PolicyEngine {
    pub fn new(python: PathBuf, bubblewrap: PathBuf, timeout: Duration) -> io::Result<Self> {
        require(
            python.is_absolute() && bubblewrap.is_absolute(),
            "execution paths must be absolute",
        )?;
        require(
            !timeout.is_zero() && timeout <= Duration::from_secs(300),
            "timeout must be within 0..300 seconds",
        )?;
        Ok(Self {
            python,
            bubblewrap,
            timeout,
            worker: std::sync::Mutex::new(BTreeMap::new()),
        })
    }

    pub fn confirm_backends(
        &self,
        policy: &CompiledPolicy,
        capabilities: &crate::protocol::contract::RuntimeCapabilities,
    ) -> io::Result<()> {
        let Some(path) = policy.runtime_config_path() else { return Ok(()); };
        let config: Value = serde_json::from_slice(&fs::read(path)?).map_err(invalid)?;
        let identities: Vec<&str> = if let Some(backends) = config.get("backends").and_then(Value::as_array) {
            backends.iter().filter_map(|backend| backend.get("id").and_then(Value::as_str)).collect()
        } else {
            config.get("backend").and_then(Value::as_str).into_iter().collect()
        };
        for backend in identities {
            require(
                capabilities.backends.iter().any(|id| id == backend),
                &format!("Policy requires unavailable backend: {backend}"),
            )?;
        }
        Ok(())
    }

    pub fn compile(&self, path: &Path) -> io::Result<CompiledPolicy> {
        let source_cwd = env::current_dir()?;
        let source_path_absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            source_cwd.join(path)
        };
        let root = File::open(
            path.parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
        )?;
        let filename = path
            .file_name()
            .and_then(|v| v.to_str())
            .ok_or_else(|| invalid("invalid YAML filename"))?;
        let text = open_source(&root, filename)
            .and_then(|file| read_limited(file, MAX_BYTES))
            .map_err(|e| {
                io::Error::new(e.kind(), format!("Policy YAML {}: {e}", path.display()))
            })?;
        let text = std::str::from_utf8(&text).map_err(invalid)?;
        check_yaml(text)?;
        let definition: Definition = serde_yaml::from_str(text).map_err(invalid)?;
        require(
            definition.version == 1 && definition.language == "python",
            "only version 1 / python is supported",
        )?;
        require(
            definition.fidelity == "Best-effort",
            "only Best-effort is supported; stronger guarantees are not implemented",
        )?;
        require(
            definition
                .runtime
                .as_deref()
                .is_none_or(|runtime| !runtime.trim().is_empty()),
            "runtime must not be empty",
        )?;
        require(
            !definition.files.is_empty() && definition.files.len() <= MAX_FILES,
            "invalid files list",
        )?;
        let mut files = BTreeMap::new();
        let mut total = 0;
        for name in definition.files {
            source_path(&name)?;
            let source = open_source(&root, &name)
                .and_then(|file| read_limited(file, MAX_BYTES - total))
                .map_err(|e| {
                    io::Error::new(
                        e.kind(),
                        format!("Policy source {name} ({}): {e}", path.display()),
                    )
                })?;
            total += source.len();
            let source = String::from_utf8(source).map_err(invalid)?;
            require(files.insert(name, source).is_none(), "duplicate file")?;
        }
        let mut decisions = BTreeMap::new();
        let mut raw = definition.decisions;
        for decision in Decision::ALL {
            let value = match decision {
                Decision::OpportunityPass | Decision::FeasibilityPass => {
                    let admit = raw
                        .get_mut("admit")
                        .and_then(Value::as_object_mut)
                        .ok_or_else(|| invalid("missing admit"))?;
                    admit
                        .remove(decision.as_str().split('.').nth(1).unwrap())
                        .ok_or_else(|| invalid(format!("missing {}", decision.as_str())))?
                }
                _ => raw
                    .remove(decision.as_str())
                    .ok_or_else(|| invalid(format!("missing {}", decision.as_str())))?,
            };
            let mut binding: Binding = serde_json::from_value(value).map_err(invalid)?;
            if let Some(history) = &binding.history {
                require(decision == Decision::Predict && history.mode == "transcript_tail"
                    && (1..=64).contains(&history.max_entries), "invalid history projection")?;
                binding.inputs.push("history".into());
            }
            if let Some(outcome) = &binding.outcome {
                require(decision == Decision::Match && outcome == "tool_output", "invalid outcome projection")?;
                binding.inputs.push("outcome".into());
            }
            validate_binding(decision, &binding, &files)?;
            decisions.insert(decision, binding);
        }
        require(
            raw.remove("admit")
                .and_then(|v| v.as_object().map(|v| v.is_empty()))
                == Some(true)
                && raw.is_empty(),
            "unknown Decision or admit entry",
        )?;
        let mut policy = CompiledPolicy {
            source: source_path_absolute,
            definition: text.to_owned(),
            runtime_config_path: definition.runtime.as_ref().map(|runtime| {
                let path = PathBuf::from(runtime);
                if path.is_absolute() {
                    path
                } else {
                    source_cwd.join(path)
                }
            }),
            version: 1,
            python: String::new(),
            files,
            runtime: definition.runtime,
            decisions,
            digest: String::new(),
            policy_model_grant: definition.policy_model_grant,
            pure: BTreeMap::new(),
        };
        if let Some(grant) = policy.policy_model_grant() {
            super::validation::validate_model_grant(grant)?;
        }
        let probe = self.execute("import sys,json,os\nb=json.dumps(sys.version).encode();os.write(1,len(b).to_bytes(4,'big')+b)", &json!({}))?;
        policy.python = probe
            .as_str()
            .ok_or_else(|| invalid("invalid Python identity"))?
            .to_owned();
        policy.digest = policy.content_digest()?;
        for decision in Decision::ALL {
            // The one-shot AST probe must not evict cached Decision workers.
            let probe = policy.check_script(decision)?;
            let checked = super::worker::Worker::start(&probe, &self.python, &self.bubblewrap)
                .and_then(|mut worker| worker.evaluate(&json!({"check": true}), self.timeout))
                .map_err(|e| io::Error::new(e.kind(), format!("{} binding: {e}", decision.as_str())))?;
            if let Some(error) = checked.get("error").and_then(Value::as_str) {
                if decision == Decision::OpportunityPass {
                    return Err(io::Error::new(io::ErrorKind::InvalidInput, error));
                }
                return Err(invalid(error));
            }
            let pure = checked.get("pure").filter(|value| !value.is_null());
            if pure.is_none() && decision == Decision::OpportunityPass {
                return Err(super::validation::opportunity_requires_pure());
            }
            if pure.is_none() || checked.get("partial").and_then(Value::as_bool) == Some(true) {
                self.execute(&policy.script(decision)?, &json!({"check": true}))
                    .map_err(|e| io::Error::new(e.kind(), format!("{} binding: {e}", decision.as_str())))?;
            }
            if let Some(program) = pure {
                policy.pure.insert(decision, program.clone());
            }
        }
        Ok(policy)
    }

    pub fn evaluate(
        &self,
        policy: &CompiledPolicy,
        decision: Decision,
        data: &Value,
    ) -> io::Result<Value> {
        let binding = &policy.decisions[&decision];
        validate_data(decision, binding, data)?;
        if let Some(program) = policy.pure_decision(decision) {
            if let Some(result) = super::pure::evaluate(program, data, &binding.policy_config) {
                let result = result.map_err(|error| invalid(format!("{} evaluation: {error}", decision.as_str())))?;
                validate_result(decision, data, &result)?;
                return Ok(result);
            }
        }
        if decision == Decision::OpportunityPass {
            return Err(super::validation::opportunity_requires_pure());
        }
        let result = self
            .execute(&policy.script(decision)?, &json!({"data": data}))
            .map_err(|e| {
                io::Error::new(e.kind(), format!("{} evaluation: {e}", decision.as_str()))
            })?;
        validate_result(decision, data, &result)?;
        Ok(result)
    }

    pub fn evaluate_with_provider(
        &self,
        policy: &CompiledPolicy,
        decision: Decision,
        data: &Value,
        models: &[String],
        requests: usize,
        provider: &super::ModelProviderProcess,
    ) -> io::Result<Value> {
        self.evaluate_with_capabilities(
            policy,
            decision,
            data,
            models,
            requests,
            &mut |request, deadline| provider.complete(request, deadline),
        )
    }

    /// The host grant callback must honor its deadline.
    pub fn evaluate_with_capabilities(
        &self,
        policy: &CompiledPolicy,
        decision: Decision,
        data: &Value,
        models: &[String],
        requests: usize,
        complete: &mut dyn FnMut(&Value, std::time::Instant) -> io::Result<Value>,
    ) -> io::Result<Value> {
        self.evaluate_capability_request(policy, decision, data, models, requests, complete, None, self.timeout)
    }

    /// Cancellation kills and reaps owned Python and provider processes before returning.
    pub fn evaluate_with_provider_cancellable(
        &self,
        policy: &CompiledPolicy,
        decision: Decision,
        data: &Value,
        models: &[String],
        requests: usize,
        provider: &super::ModelProviderProcess,
        cancelled: &std::sync::atomic::AtomicBool,
        timeout: Option<Duration>,
    ) -> io::Result<Value> {
        self.evaluate_capability_request(
            policy,
            decision,
            data,
            models,
            requests,
            &mut |request, deadline| {
                provider.complete_cancellable(request, deadline, Some(cancelled))
            },
            Some(cancelled),
            timeout.unwrap_or(self.timeout),
        )
    }

    fn evaluate_capability_request(
        &self,
        policy: &CompiledPolicy,
        decision: Decision,
        data: &Value,
        models: &[String],
        requests: usize,
        complete: &mut dyn FnMut(&Value, Instant) -> io::Result<Value>,
        cancelled: Option<&std::sync::atomic::AtomicBool>,
        timeout: Duration,
    ) -> io::Result<Value> {
        policy.validate()?;
        validate_data(decision, &policy.decisions[&decision], data)?;
        let deadline = Instant::now() + timeout;
        let mut remaining = requests;
        let request_id = tempfile::NamedTempFile::new()?
            .path()
            .to_string_lossy()
            .into_owned();
        let script = policy.script(decision)?;
        let mut workers = self
            .worker
            .lock()
            .map_err(|_| invalid("Policy worker lock poisoned"))?;
        if cancelled.is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Acquire)) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "Policy evaluation cancelled"));
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "Policy deadline exceeded"));
        }
        if !workers.contains_key(&script) {
            if workers.len() >= 10 {
                workers.clear();
            }
            workers.insert(
                script.clone(),
                super::worker::Worker::start(&script, &self.python, &self.bubblewrap)?,
            );
        }
        let result = workers.get_mut(&script).unwrap().evaluate_cancellable(
            &json!({"data":data,"request_id":request_id}),
            deadline.saturating_duration_since(Instant::now()),
            &mut |request| {
                let provider = request["provider"]
                    .as_str()
                    .ok_or_else(|| invalid("invalid provider"))?;
                let model = request["model"]
                    .as_str()
                    .ok_or_else(|| invalid("invalid model"))?;
                let prompt = request["prompt"]
                    .as_str()
                    .ok_or_else(|| invalid("invalid prompt"))?;
                let tokens = request["max_tokens"].as_u64().unwrap_or(0);
                if remaining == 0
                    || std::time::Instant::now() >= deadline
                    || prompt.len() > 16384
                    || tokens == 0
                    || tokens > 1024
                    || !models.contains(&format!("{provider}/{model}"))
                {
                    return Err(invalid("Policy capability denied"));
                }
                remaining -= 1;
                complete(request, deadline)
            },
            cancelled,
        );
        let result = result.and_then(|result| {
            validate_result(decision, data, &result)?;
            Ok(result)
        });
        if result.is_err() {
            workers.remove(&script);
        }
        result
    }

    fn execute(&self, script: &str, request: &Value) -> io::Result<Value> {
        if !script.contains("MANIFEST =") {
            return self.execute_once(script, request);
        }
        let mut slot = self
            .worker
            .lock()
            .map_err(|_| invalid("Policy worker lock poisoned"))?;
        if !slot.contains_key(script) {
            if slot.len() >= 10 {
                slot.clear();
            }
            slot.insert(
                script.to_owned(),
                super::worker::Worker::start(script, &self.python, &self.bubblewrap)?,
            );
        }
        let result = slot
            .get_mut(script)
            .unwrap()
            .evaluate(request, self.timeout);
        if result.is_err() {
            slot.remove(script);
        }
        result
    }

    fn execute_once(&self, script: &str, request: &Value) -> io::Result<Value> {
        require_executable(&self.python, "ASCOUT_POLICY_PYTHON", "python3")?;
        require_executable(&self.bubblewrap, "ASCOUT_POLICY_BWRAP", "bwrap")?;
        let directory = tempfile::tempdir()?;
        let script_path = directory.path().join("runner.py");
        let request_path = directory.path().join("request.json");
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&script_path)?
            .write_all(script.as_bytes())?;
        let request = serde_json::to_vec(request)?;
        require(request.len() <= MAX_BYTES, "input exceeds size limit")?;
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&request_path)?
            .write_all(&request)?;
        let (mut reader, writer) = UnixStream::pair()?;
        reader.set_nonblocking(true)?;
        let mut command = Command::new(&self.bubblewrap);
        command
            .args([
                "--unshare-all",
                "--die-with-parent",
                "--new-session",
                "--cap-drop",
                "ALL",
                "--clearenv",
                "--ro-bind",
                "/usr/lib",
                "/usr/lib",
                "--ro-bind",
                "/lib",
                "/lib",
                "--ro-bind",
                "/lib64",
                "/lib64",
                "--dev",
                "/dev",
                "--size",
                "16777216",
                "--tmpfs",
                "/dev/shm",
                "--size",
                "16777216",
                "--tmpfs",
                "/tmp",
                "--ro-bind",
            ])
            .arg(&script_path)
            .arg("/runner.py")
            .arg("--ro-bind")
            .arg(fs::canonicalize(&self.python)?)
            .arg(&self.python)
            .arg("--ro-bind")
            .arg(&request_path)
            .arg("/request.json")
            .args(["--remount-ro", "/", "--chdir", "/tmp", "--"])
            .arg(&self.python)
            .args(["-I", "-S", "-B", "/runner.py"])
            .stdin(Stdio::null())
            .stdout(Stdio::from(std::os::fd::OwnedFd::from(writer)))
            .stderr(Stdio::piped());
        let mut child = command.spawn()?;
        let stderr = child.stderr.take().unwrap();
        let diagnostics = std::thread::spawn(move || {
            let mut kept = Vec::new();
            let mut stream = stderr;
            let mut buffer = [0; 4096];
            while let Ok(n) = stream.read(&mut buffer) {
                if n == 0 {
                    break;
                }
                let count = n.min(16384_usize.saturating_sub(kept.len()));
                kept.extend_from_slice(&buffer[..count]);
            }
            kept
        });
        let deadline = Instant::now() + self.timeout;
        let mut bytes = Vec::new();
        let outcome = (|| -> io::Result<()> {
            loop {
                let mut buffer = [0; 8192];
                match reader.read(&mut buffer) {
                    Ok(n) => bytes.extend_from_slice(&buffer[..n]),
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                    Err(e) => break Err(e),
                }
                if bytes.len() > MAX_BYTES + 4 {
                    break Err(invalid("Policy result exceeds size limit"));
                }
                if let Some(status) = child.try_wait()? {
                    loop {
                        match reader.read(&mut buffer) {
                            Ok(0) => break,
                            Ok(n) => {
                                bytes.extend_from_slice(&buffer[..n]);
                                if bytes.len() > MAX_BYTES + 4 {
                                    break;
                                }
                            }
                            Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                            Err(e) => return Err(e),
                        }
                    }
                    break if status.success() {
                        Ok(())
                    } else {
                        Err(invalid(format!("Policy process exited: {status}")))
                    };
                }
                if Instant::now() >= deadline {
                    break Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "Policy deadline exceeded",
                    ));
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        })();
        // Killing bubblewrap tears down the PID namespace, including its children.
        if outcome.is_err() {
            let _ = child.kill();
        }
        child.wait()?;
        let diagnostics = diagnostics
            .join()
            .map_err(|_| invalid("diagnostic reader failed"))?;
        if let Err(error) = outcome {
            return Err(io::Error::new(
                error.kind(),
                format!("{error}: {}", String::from_utf8_lossy(&diagnostics)),
            ));
        }
        require(
            bytes.len() >= 4 && bytes.len() <= MAX_BYTES + 4,
            "truncated or oversized Policy response",
        )?;
        let size = u32::from_be_bytes(bytes[..4].try_into().unwrap()) as usize;
        require(
            size <= MAX_BYTES && bytes.len() == size + 4,
            "invalid Policy response length",
        )?;
        let response: Value = serde_json::from_slice(&bytes[4..]).map_err(invalid)?;
        check_json(&response, 0)?;
        Ok(response)
    }
}
