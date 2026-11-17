//! One owned, serialized sandbox. Replacing or dropping it kills and reaps it.
use super::validation::invalid;
use serde_json::Value;
use std::{
    io::{self, Read, Write},
    os::unix::net::UnixStream,
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

pub(super) struct Worker {
    child: Child,
    pipe: UnixStream,
    _directory: tempfile::TempDir,
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_survives_creator_thread_exit() {
        let mut worker = std::thread::spawn(|| {
            let mut worker = Worker::start(
                "import sys,json\nfor line in sys.stdin.buffer:\n payload=b'true'\n sys.stdout.buffer.write(len(payload).to_bytes(4,'big')+payload)\n sys.stdout.buffer.flush()\n",
                Path::new("/usr/bin/python3"),
                Path::new("/usr/bin/bwrap"),
            ).unwrap();
            worker.evaluate(&serde_json::json!({}), Duration::from_secs(5)).unwrap();
            worker
        }).join().unwrap();
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(worker.evaluate(&serde_json::json!({}), Duration::from_secs(5)).unwrap(), serde_json::json!(true));
    }

    #[test]
    fn repeated_requests_share_interpreter_state() {
        let script = "import sys,json\ncount=0\nfor line in sys.stdin.buffer:\n count+=1\n payload=json.dumps(count).encode()\n sys.stdout.buffer.write(len(payload).to_bytes(4,'big')+payload)\n sys.stdout.buffer.flush()\n";
        let mut worker = Worker::start(
            script,
            Path::new("/usr/bin/python3"),
            Path::new("/usr/bin/bwrap"),
        )
        .unwrap();
        assert_eq!(
            worker
                .evaluate(&serde_json::json!({}), Duration::from_secs(5))
                .unwrap(),
            serde_json::json!(1)
        );
        assert_eq!(
            worker
                .evaluate(&serde_json::json!({}), Duration::from_secs(5))
                .unwrap(),
            serde_json::json!(2)
        );
        drop(worker);
        let mut rebuilt = Worker::start(
            script,
            Path::new("/usr/bin/python3"),
            Path::new("/usr/bin/bwrap"),
        )
        .unwrap();
        assert_eq!(
            rebuilt
                .evaluate(&serde_json::json!({}), Duration::from_secs(5))
                .unwrap(),
            serde_json::json!(1)
        );
    }
}

impl Worker {
    pub fn start(script: &str, python: &Path, bwrap: &Path) -> io::Result<Self> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("runner.py");
        std::fs::write(&path, script)?;
        let (pipe, remote) = UnixStream::pair()?;
        let input = remote.try_clone()?;
        pipe.set_nonblocking(true)?;
        let child = Command::new(bwrap)
            .args([
                // Workers outlive the evaluation thread that creates them.
                // Ownership and Drop supervise their lifetime; PDEATHSIG would
                // kill a cached worker when that short-lived thread exits.
                "--unshare-all",
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
            .arg(path)
            .arg("/runner.py")
            .arg("--ro-bind")
            .arg(std::fs::canonicalize(python)?)
            .arg(python)
            .args(["--remount-ro", "/dev", "--remount-ro", "/", "--chdir", "/tmp", "--"])
            .arg(python)
            .args(["-I", "-S", "-B", "/runner.py"])
            .stdin(Stdio::from(std::os::fd::OwnedFd::from(input)))
            .stdout(Stdio::from(std::os::fd::OwnedFd::from(remote)))
            .stderr(Stdio::null())
            .spawn()?;
        Ok(Self {
            child,
            pipe,
            _directory: directory,
        })
    }
    pub fn evaluate(&mut self, request: &Value, timeout: Duration) -> io::Result<Value> {
        self.evaluate_with_capabilities(request, timeout, &mut |_| {
            Err(invalid("Policy capability denied"))
        })
    }
    pub fn evaluate_with_capabilities(
        &mut self,
        request: &Value,
        timeout: Duration,
        complete: &mut dyn FnMut(&Value) -> io::Result<Value>,
    ) -> io::Result<Value> {
        self.evaluate_cancellable(request, timeout, complete, None)
    }
    pub fn evaluate_cancellable(
        &mut self, request: &Value, timeout: Duration,
        complete: &mut dyn FnMut(&Value) -> io::Result<Value>,
        cancelled: Option<&std::sync::atomic::AtomicBool>,
    ) -> io::Result<Value> {
        let mut input = serde_json::to_vec(request)?;
        input.push(b'\n');
        if input.len() > super::MAX_BYTES {
            return Err(invalid("input exceeds size limit"));
        }
        let deadline = Instant::now() + timeout;
        let mut sent = 0;
        let mut output = Vec::new();
        loop {
            if cancelled.is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Acquire)) {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "Policy evaluation cancelled"));
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Policy deadline exceeded",
                ));
            }
            if sent < input.len() {
                match self.pipe.write(&input[sent..]) {
                    Ok(0) => return Err(invalid("Policy input closed")),
                    Ok(n) => sent += n,
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                    Err(e) => return Err(e),
                }
            }
            let mut buffer = [0; 8192];
            match self.pipe.read(&mut buffer) {
                Ok(0) => return Err(invalid("Policy worker closed")),
                Ok(n) => output.extend_from_slice(&buffer[..n]),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(e),
            }
            if output.len() > super::MAX_BYTES + 4 {
                return Err(invalid("Policy result exceeds size limit"));
            }
            if output.len() >= 4 {
                let size = u32::from_be_bytes(output[..4].try_into().unwrap()) as usize;
                if size > super::MAX_BYTES || output.len() > size + 4 {
                    return Err(invalid("invalid Policy response length"));
                }
                if output.len() == size + 4 {
                    let value: Value = serde_json::from_slice(&output[4..]).map_err(invalid)?;
                    super::validation::check_json(&value, 0)?;
                    if let Some(id) = request.get("request_id") {
                        if value.get("request_id") != Some(id) {
                            return Err(invalid("Policy request ID mismatch"));
                        }
                        match value.get("type").and_then(Value::as_str) {
                            Some("result") => return Ok(value["result"].clone()),
                            Some("capability") if value["capability"] == "model_complete" => {
                                let response = complete(&value["request"])?;
                                if Instant::now() >= deadline {
                                    return Err(invalid("Policy deadline exceeded"));
                                }
                                input = serde_json::to_vec(
                                    &serde_json::json!({"type":"capability_result","request_id":id,"response":response}),
                                )?;
                                input.push(b'\n');
                                if input.len() > super::MAX_BYTES {
                                    return Err(invalid(
                                        "Policy capability response exceeds limit",
                                    ));
                                }
                                sent = 0;
                                output.clear();
                                continue;
                            }
                            _ => return Err(invalid("Policy capability denied")),
                        }
                    }
                    return Ok(value);
                }
            }
            if let Some(status) = self.child.try_wait()? {
                return Err(invalid(format!("Policy worker exited: {status}")));
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}
