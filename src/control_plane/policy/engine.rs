//! 隔离的 Python Policy 评估进程与有界结果读取。

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

/// Linux prototype: bubblewrap namespaces, read-only source, seccomp, rlimits,
/// bounded result pipe and a supervised wall-clock deadline. No external abilities.
pub struct PolicyEngine {
    python: PathBuf,
    bubblewrap: PathBuf,
    timeout: Duration,
}

/// Executable resolution order: explicit environment variable, PATH, then the
/// historical absolute path. The last step keeps existing deployments working;
/// execution later reports a clear error if none of them exist.
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
            python: resolve_executable("ASCOUT_POLICY_PYTHON", "python3", "/usr/bin/python3"),
            bubblewrap: resolve_executable("ASCOUT_POLICY_BWRAP", "bwrap", "/usr/bin/bwrap"),
            timeout: Duration::from_secs(5),
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
            source: source_path_absolute,
            definition: text.to_owned(),
            // Preserve existing semantics: relative Runtime paths are relative
            // to the launch cwd, not the Policy's directory.
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
        require_executable(&self.python, "ASCOUT_POLICY_PYTHON", "python3")?;
        require_executable(&self.bubblewrap, "ASCOUT_POLICY_BWRAP", "bwrap")?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executable_lookup_uses_path_entries() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("policy-tool");
        fs::write(&executable, b"#!/bin/sh\n").unwrap();
        let path = directory.path().as_os_str();
        assert_eq!(find_in_path_value(path, "policy-tool"), Some(executable));
        assert_eq!(find_in_path_value(path, "missing-tool"), None);
    }

    #[test]
    fn missing_executable_reports_the_override_variable() {
        let error = require_executable(
            Path::new("/definitely/missing/bwrap"),
            "ASCOUT_POLICY_BWRAP",
            "bwrap",
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert!(error.to_string().contains("ASCOUT_POLICY_BWRAP"), "{error}");
    }
}
