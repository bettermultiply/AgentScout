//! YAML compilation and isolated Python Policy evaluation.
//!
//! This module returns decisions; it never applies them to Branch or Runtime state.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use yaml_rust2::scanner::{Scanner, TokenType};

const MAX_BYTES: usize = 1024 * 1024;
const MAX_FILES: usize = 128;
const RUNNER: &str = include_str!("policy/runner.py");

/// `admit` has two separately invoked entries, not a tenth Decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Decision {
    #[serde(rename = "admit.opportunity_pass")]
    OpportunityPass,
    #[serde(rename = "admit.feasibility_pass")]
    FeasibilityPass,
    #[serde(rename = "select")]
    Select,
    #[serde(rename = "rebalance")]
    Rebalance,
    #[serde(rename = "extent")]
    Extent,
    #[serde(rename = "predict")]
    Predict,
    #[serde(rename = "launch")]
    Launch,
    #[serde(rename = "effect")]
    Effect,
    #[serde(rename = "match")]
    Match,
    #[serde(rename = "dispose")]
    Dispose,
}

impl Decision {
    pub const ALL: [Self; 10] = [
        Self::OpportunityPass,
        Self::FeasibilityPass,
        Self::Select,
        Self::Rebalance,
        Self::Extent,
        Self::Predict,
        Self::Launch,
        Self::Effect,
        Self::Match,
        Self::Dispose,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpportunityPass => "admit.opportunity_pass",
            Self::FeasibilityPass => "admit.feasibility_pass",
            Self::Select => "select",
            Self::Rebalance => "rebalance",
            Self::Extent => "extent",
            Self::Predict => "predict",
            Self::Launch => "launch",
            Self::Effect => "effect",
            Self::Match => "match",
            Self::Dispose => "dispose",
        }
    }

    fn fields(self) -> (&'static [&'static str], &'static [&'static str]) {
        match self {
            Self::OpportunityPass => (&["op"], &[]),
            Self::FeasibilityPass => (&["op", "state", "contract_context"], &[]),
            Self::Select => (&["frontier_view", "scheduling_context"], &[]),
            Self::Rebalance => (&["active_view", "paused_view", "scheduling_context"], &[]),
            Self::Extent => (&["process_state"], &[]),
            Self::Predict => (
                &[
                    "edge_context",
                    "process_context",
                    "assumption_set",
                    "capability_context",
                ],
                &["history"],
            ),
            Self::Launch => (
                &["edge_context", "authority_state", "candidate_view"],
                &["scheduling_context"],
            ),
            Self::Effect => (&["effect_descriptor", "context"], &[]),
            Self::Match => (&["proposal", "evidence", "match_context"], &["outcome"]),
            Self::Dispose => (&["branch", "resolution", "context"], &[]),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    file: String,
    function: String,
    #[serde(default)]
    policy_config: BTreeMap<String, Value>,
    #[serde(default)]
    inputs: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Definition {
    version: u32,
    language: String,
    files: Vec<String>,
    fidelity: String,
    #[serde(default)]
    runtime: Option<String>,
    decisions: BTreeMap<String, Value>,
}

/// Fixed source, bindings and Python environment identity. No author-side paths.
/// Deserialize only from an authenticated source; the digest is not authentication.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledPolicy {
    version: u32,
    python: String,
    files: BTreeMap<String, String>,
    #[serde(default)]
    runtime: Option<String>,
    decisions: BTreeMap<Decision, Binding>,
    digest: String,
}

impl CompiledPolicy {
    pub fn digest(&self) -> &str {
        &self.digest
    }

    /// Runtime configuration selected by this Policy, if any.
    pub fn runtime(&self) -> Option<&str> {
        self.runtime.as_deref()
    }

    /// A serializable Python script with all declared source and bindings embedded.
    /// Run through PolicyEngine, not as an unrestricted Python program.
    pub fn script(&self, decision: Decision) -> io::Result<String> {
        self.validate()?;
        let manifest = json!({
            "version": self.version, "python": self.python, "files": self.files,
            "binding": self.decisions.get(&decision).ok_or_else(|| invalid("missing Decision"))?,
            "decision": decision.as_str(), "digest": self.digest,
        });
        Ok(format!(
            "import os,json\nos.dup2(1,3)\nos.dup2(2,1)\nMANIFEST = json.loads({})\n{}",
            serde_json::to_string(&manifest.to_string())?,
            RUNNER
        ))
    }

    fn content_digest(&self) -> io::Result<String> {
        let bytes = serde_json::to_vec(&(
            self.version,
            &self.python,
            &self.files,
            &self.runtime,
            &self.decisions,
            RUNNER,
        ))?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }

    fn validate(&self) -> io::Result<()> {
        require(self.version == 1, "unsupported compiled Policy version")?;
        require(
            self.decisions.keys().copied().collect::<BTreeSet<_>>() == Decision::ALL.into(),
            "incomplete Decision bindings",
        )?;
        require(
            !self.files.is_empty() && self.files.len() <= MAX_FILES,
            "invalid file count",
        )?;
        require(
            self.files.values().map(String::len).sum::<usize>() <= MAX_BYTES,
            "Policy source exceeds size limit",
        )?;
        for path in self.files.keys() {
            source_path(path)?;
        }
        for (&decision, binding) in &self.decisions {
            validate_binding(decision, binding, &self.files)?;
        }
        require(
            self.digest == self.content_digest()?,
            "compiled Policy digest mismatch",
        )
    }
}

/// Linux prototype: bubblewrap namespaces, read-only source, seccomp, rlimits,
/// bounded result pipe and a supervised wall-clock deadline. No external abilities.
pub struct PolicyEngine {
    python: PathBuf,
    bubblewrap: PathBuf,
    timeout: Duration,
}

impl Default for PolicyEngine {
    fn default() -> Self {
        Self {
            python: "/usr/bin/python3".into(),
            bubblewrap: "/usr/bin/bwrap".into(),
            timeout: Duration::from_secs(5),
        }
    }
}

impl PolicyEngine {
    /// Paths are trusted deployment settings, never read from the Policy YAML.
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
        })
    }

    pub fn compile(&self, path: &Path) -> io::Result<CompiledPolicy> {
        let root = File::open(
            path.parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
        )?;
        let filename = path
            .file_name()
            .and_then(|v| v.to_str())
            .ok_or_else(|| invalid("invalid YAML filename"))?;
        let text = read_limited(open_source(&root, filename)?, MAX_BYTES)?;
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
            let source = read_limited(open_source(&root, &name)?, MAX_BYTES - total)?;
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
            let binding: Binding = serde_json::from_value(value).map_err(invalid)?;
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
            version: 1,
            python: String::new(),
            files,
            runtime: definition.runtime,
            decisions,
            digest: String::new(),
        };
        // Probe the trusted interpreter inside the same isolation as the algorithms.
        let probe = self.execute("import sys,json,os\nb=json.dumps(sys.version).encode();os.write(1,len(b).to_bytes(4,'big')+b)", &json!({}))?;
        policy.python = probe
            .as_str()
            .ok_or_else(|| invalid("invalid Python identity"))?
            .to_owned();
        policy.digest = policy.content_digest()?;
        for decision in Decision::ALL {
            self.execute(&policy.script(decision)?, &json!({"check": true}))
                .map_err(|e| {
                    io::Error::new(e.kind(), format!("{} binding: {e}", decision.as_str()))
                })?;
        }
        Ok(policy)
    }

    pub fn evaluate(
        &self,
        policy: &CompiledPolicy,
        decision: Decision,
        data: &Value,
    ) -> io::Result<Value> {
        policy.validate()?;
        validate_data(decision, &policy.decisions[&decision], data)?;
        let result = self
            .execute(&policy.script(decision)?, &json!({"data": data}))
            .map_err(|e| {
                io::Error::new(e.kind(), format!("{} evaluation: {e}", decision.as_str()))
            })?;
        validate_result(decision, data, &result)?;
        Ok(result)
    }

    fn execute(&self, script: &str, request: &Value) -> io::Result<Value> {
        // No author data in a shared directory: tempfile creates a private 0700 root.
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
                    // Drain bytes already queued before the process exited.
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

fn invalid(message: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}
fn require(condition: bool, message: &str) -> io::Result<()> {
    if condition {
        Ok(())
    } else {
        Err(invalid(message))
    }
}

fn read_limited(file: File, limit: usize) -> io::Result<Vec<u8>> {
    require(file.metadata()?.is_file(), "source must be a regular file")?;
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    require(bytes.len() <= limit, "file exceeds Policy size limit")?;
    Ok(bytes)
}

fn source_path(name: &str) -> io::Result<()> {
    require(
        !name.is_empty()
            && name
                .split('/')
                .all(|s| !s.is_empty() && s != "." && s != ".."),
        "source path must be a normalized relative path",
    )?;
    require(
        Path::new(name)
            .components()
            .all(|c| matches!(c, Component::Normal(_))),
        "source path escapes Policy directory",
    )?;
    require(
        !name.contains(['\\', '\0']) && name.is_ascii(),
        "source path must be ASCII without backslash or NUL",
    )?;
    if name.ends_with(".py") {
        for part in name.trim_end_matches(".py").split('/') {
            require(identifier(part), "Python module paths must be identifiers")?;
            require(
                ![
                    "sys",
                    "os",
                    "json",
                    "inspect",
                    "resource",
                    "ctypes",
                    "importlib",
                    "site",
                    "builtins",
                    "encodings",
                ]
                .contains(&part),
                "source shadows an execution module",
            )?;
        }
    }
    Ok(())
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .enumerate()
            .all(|(i, c)| c == b'_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()))
}

fn open_source(root: &File, name: &str) -> io::Result<File> {
    let mut directory = root.try_clone()?;
    let parts: Vec<_> = name.split('/').collect();
    for (index, part) in parts.iter().enumerate() {
        let part = std::ffi::CString::new(*part).map_err(invalid)?;
        let mut flags = libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK;
        if index + 1 != parts.len() {
            flags |= libc::O_DIRECTORY;
        }
        let fd = unsafe { libc::openat(directory.as_raw_fd(), part.as_ptr(), flags) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        directory = unsafe { File::from_raw_fd(fd) };
    }
    Ok(directory)
}

fn check_yaml(text: &str) -> io::Result<()> {
    let mut scanner = Scanner::new(text.chars());
    let mut depth = 0_usize;
    for token in scanner.by_ref() {
        match token.1 {
            TokenType::Alias(_)
            | TokenType::Anchor(_)
            | TokenType::Tag(_, _)
            | TokenType::TagDirective(_, _)
            | TokenType::VersionDirective(_, _) => {
                return Err(invalid(
                    "YAML anchors, aliases, tags and directives are not supported",
                ));
            }
            TokenType::BlockSequenceStart
            | TokenType::BlockMappingStart
            | TokenType::FlowSequenceStart
            | TokenType::FlowMappingStart => {
                depth += 1;
                require(depth <= 32, "YAML nesting exceeds 32")?;
            }
            TokenType::BlockEnd | TokenType::FlowSequenceEnd | TokenType::FlowMappingEnd => {
                depth = depth.saturating_sub(1)
            }
            _ => {}
        }
    }
    if let Some(error) = scanner.get_error() {
        return Err(invalid(error));
    }
    // serde_yaml::Value rejects duplicate keys (including inside configuration).
    let value: serde_yaml::Value = serde_yaml::from_str(text).map_err(invalid)?;
    fn walk(value: &serde_yaml::Value) -> io::Result<()> {
        match value {
            serde_yaml::Value::Mapping(map) => {
                for (key, value) in map {
                    require(
                        key.as_str().is_some_and(|k| k != "<<"),
                        "YAML keys must be strings; merge keys are forbidden",
                    )?;
                    walk(value)?;
                }
            }
            serde_yaml::Value::Sequence(items) => {
                for item in items {
                    walk(item)?;
                }
            }
            serde_yaml::Value::Number(n) => require(
                n.as_i64().is_some()
                    || (n.as_u64().is_none() && n.as_f64().is_some_and(f64::is_finite)),
                "invalid YAML number",
            )?,
            serde_yaml::Value::Tagged(_) => return Err(invalid("YAML tags are forbidden")),
            _ => {}
        }
        Ok(())
    }
    walk(&value)
}

fn check_json(value: &Value, depth: usize) -> io::Result<()> {
    require(depth <= 32, "JSON nesting exceeds 32")?;
    match value {
        Value::Array(items) => {
            for item in items {
                check_json(item, depth + 1)?;
            }
        }
        Value::Object(map) => {
            for item in map.values() {
                check_json(item, depth + 1)?;
            }
        }
        Value::Number(n) => require(
            n.as_i64().is_some()
                || (n.as_u64().is_none() && n.as_f64().is_some_and(f64::is_finite)),
            "numbers must be signed 64-bit integers or finite floats",
        )?,
        _ => {}
    }
    Ok(())
}

fn validate_binding(
    decision: Decision,
    binding: &Binding,
    files: &BTreeMap<String, String>,
) -> io::Result<()> {
    require(
        files.contains_key(&binding.file) && binding.file.ends_with(".py"),
        "entry file must be a declared Python file",
    )?;
    require(
        identifier(&binding.function),
        "function must be a top-level identifier",
    )?;
    let (_, optional) = decision.fields();
    let mut seen = BTreeSet::new();
    for input in &binding.inputs {
        require(
            (optional.contains(&input.as_str())
                || (decision == Decision::Extent
                    && [
                        "process_state.active_time",
                        "process_state.resource_counters",
                        "process_state.result_facts",
                        "process_state.semantic_facts",
                    ]
                    .contains(&input.as_str())))
                && seen.insert(input),
            "unknown or duplicate optional input",
        )?;
    }
    let parents: Vec<_> = binding.file.split('/').collect();
    for index in 1..parents.len() {
        require(
            files.contains_key(&format!("{}/__init__.py", parents[..index].join("/"))),
            "Python packages must declare __init__.py",
        )?;
    }
    check_json(&serde_json::to_value(&binding.policy_config)?, 0)
}

fn validate_data(decision: Decision, binding: &Binding, data: &Value) -> io::Result<()> {
    check_json(data, 0)?;
    let map = data
        .as_object()
        .ok_or_else(|| invalid("Decision data must be an object"))?;
    let (required, _) = decision.fields();
    require(
        required.iter().all(|k| map.contains_key(*k))
            && binding.inputs.iter().all(|k| {
                let mut value = data;
                for part in k.split('.') {
                    if let Some(next) = value.get(part) {
                        value = next;
                    } else {
                        return false;
                    }
                }
                true
            }),
        "missing Decision input",
    )?;
    require(
        map.keys()
            .all(|k| required.contains(&k.as_str()) || binding.inputs.contains(k)),
        "input is outside this Decision's declared data",
    )?;
    if decision == Decision::Extent {
        let state = data["process_state"]
            .as_object()
            .ok_or_else(|| invalid("process_state must be an object"))?;
        require(
            state.keys().all(|k| {
                ["steps_completed", "phase"].contains(&k.as_str())
                    || binding.inputs.contains(&format!("process_state.{k}"))
            }),
            "undeclared process_state metric",
        )?;
        require(
            data["process_state"]["steps_completed"]
                .as_i64()
                .is_some_and(|v| v >= 0)
                && data["process_state"]["phase"].is_string(),
            "invalid process progress",
        )?;
        if let Some(value) = state.get("active_time") {
            require(
                value.as_f64().is_some_and(|n| n >= 0.0),
                "active_time must be nonnegative seconds",
            )?;
        }
        for key in ["resource_counters", "result_facts", "semantic_facts"] {
            if let Some(value) = state.get(key) {
                require(value.is_object(), "optional process facts must be objects")?;
            }
        }
    }
    match decision {
        Decision::OpportunityPass | Decision::FeasibilityPass => {
            let op = data["op"]
                .as_object()
                .ok_or_else(|| invalid("op must be an object"))?;
            require(
                op.len() == 4
                    && op.get("kind").is_some_and(Value::is_string)
                    && ["produces_result", "result_pending", "has_predictor"]
                        .iter()
                        .all(|k| op.get(*k).is_some_and(Value::is_boolean)),
                "op must contain kind and three boolean opportunity facts",
            )?;
        }
        Decision::Select => {
            ids(&data["frontier_view"], "edge_id")?;
        }
        Decision::Rebalance => {
            let active = ids(&data["active_view"], "branch_id")?;
            let paused = ids(&data["paused_view"], "branch_id")?;
            require(
                active.is_disjoint(&paused),
                "Branch cannot be both active and paused",
            )?;
        }
        Decision::Predict => {
            require(
                data["assumption_set"].as_array().is_some_and(|a| {
                    let mut seen = BTreeSet::new();
                    a.iter()
                        .all(|v| v.as_str().is_some_and(|id| seen.insert(id)))
                }),
                "assumption_set must contain unique Proposal IDs",
            )?;
        }
        Decision::Launch => require(
            data["candidate_view"].is_array(),
            "candidate_view must be an array",
        )?,
        Decision::Dispose => require(
            data["resolution"].as_str().is_some_and(|r| {
                [
                    "accepted",
                    "rejected",
                    "indeterminate",
                    "error",
                    "cancelled",
                    "preempted",
                    "extent_reached",
                    "failed",
                ]
                .contains(&r)
            }),
            "unknown resolution",
        )?,
        _ => {}
    }
    for key in required.iter().copied().chain(
        binding
            .inputs
            .iter()
            .map(String::as_str)
            .filter(|k| !k.contains('.')),
    ) {
        if ![
            "frontier_view",
            "active_view",
            "paused_view",
            "candidate_view",
            "assumption_set",
            "resolution",
            "history",
            "outcome",
        ]
        .contains(&key)
        {
            require(
                data[key].is_object() || (key == "evidence" && data[key].is_null()),
                &format!("{key} must be an object"),
            )?;
        }
    }
    Ok(())
}

fn validate_result(decision: Decision, data: &Value, result: &Value) -> io::Result<()> {
    check_json(result, 0)?;
    let valid = match decision {
        Decision::OpportunityPass | Decision::FeasibilityPass | Decision::Launch => {
            result.is_boolean()
        }
        Decision::Extent => result
            .as_str()
            .is_some_and(|v| ["stop", "continue"].contains(&v)),
        Decision::Select => {
            let candidates = ids(&data["frontier_view"], "edge_id")?;
            let selected = result
                .as_array()
                .ok_or_else(|| invalid("select must return an array"))?;
            let mut seen = BTreeSet::new();
            selected.iter().all(|v| {
                v.as_str()
                    .is_some_and(|id| candidates.contains(id) && seen.insert(id))
            })
        }
        Decision::Rebalance => {
            let mut candidates = ids(&data["active_view"], "branch_id")?;
            candidates.extend(ids(&data["paused_view"], "branch_id")?);
            let actions = result
                .as_array()
                .ok_or_else(|| invalid("rebalance must return an array"))?;
            let mut seen = BTreeSet::new();
            actions.iter().all(|v| {
                v.as_object().is_some_and(|m| m.len() == 2)
                    && v["branch_id"]
                        .as_str()
                        .is_some_and(|id| candidates.contains(id) && seen.insert(id))
                    && v["action"]
                        .as_str()
                        .is_some_and(|a| ["pause", "resume", "preempt"].contains(&a))
            })
        }
        Decision::Predict => {
            result.as_object().is_some_and(|m| m.len() == 2)
                && result["generation"] == "batch"
                && result["proposals"].as_array().is_some_and(|items| {
                    items.iter().all(|p| {
                        p.as_object().is_some_and(|m| {
                            m.contains_key("payload")
                                && m.keys()
                                    .all(|k| ["payload", "provenance"].contains(&k.as_str()))
                        })
                    })
                })
        }
        // Only plans without plugin-specific state/effect transfer can be checked yet.
        Decision::Effect => {
            result.as_object().is_some_and(|m| m.len() == 1)
                && result["mode"]
                    .as_str()
                    .is_some_and(|s| ["block", "deny"].contains(&s))
        }
        Decision::Match => {
            result
                .as_object()
                .is_some_and(|m| m.keys().all(|k| ["verdict", "score"].contains(&k.as_str())))
                && result["verdict"].as_str().is_some_and(|v| {
                    ["compatible", "incompatible", "indeterminate", "error"].contains(&v)
                })
                && result.get("score").is_none()
        }
        Decision::Dispose => {
            result.as_object().is_some_and(|m| m.len() == 1)
                && result["actions"].as_array().is_some_and(|actions| {
                    if actions.is_empty() {
                        return data["resolution"].as_str().is_some_and(|r| {
                            ["indeterminate", "error", "extent_reached"].contains(&r)
                        });
                    }
                    actions.len() == 1
                        && actions[0].as_object().is_some_and(|m| m.len() == 1)
                        && (actions[0]["intent"] == "Discard"
                            || (actions[0]["intent"] == "Promote"
                                && data["resolution"] == "accepted"))
                })
        }
    };
    require(
        valid,
        &format!(
            "invalid or currently unsupported {} result",
            decision.as_str()
        ),
    )
}

fn ids(value: &Value, key: &str) -> io::Result<BTreeSet<String>> {
    let items = value
        .as_array()
        .ok_or_else(|| invalid("Decision view must be an array"))?;
    let mut ids = BTreeSet::new();
    for item in items {
        let id = item[key]
            .as_str()
            .ok_or_else(|| invalid(format!("view missing {key}")))?;
        require(ids.insert(id.to_owned()), "duplicate ID in input view")?;
    }
    Ok(ids)
}
