//! Host-selected subprocess bridge. No provider code runs on an uncancellable thread.
use super::validation::invalid;
use serde_json::Value;
use std::{
    io::{self, Read, Write},
    os::unix::net::UnixStream,
    process::{Command, Stdio},
    time::Instant,
};

/// Executable and arguments are deployment configuration, never Policy input.
/// The service reads one JSON line and writes one JSON line, then exits.
/// Killing the service revokes its provider connection; the service must not
/// launch detached descendants.
pub struct ModelProviderProcess {
    pub executable: std::path::PathBuf,
    pub arguments: Vec<String>,
    pub environment: std::collections::BTreeMap<String, String>,
}
impl ModelProviderProcess {
    pub fn complete(&self, request: &Value, deadline: Instant) -> io::Result<Value> {
        self.complete_cancellable(request, deadline, None)
    }
    pub fn complete_cancellable(
        &self,
        request: &Value,
        deadline: Instant,
        cancelled: Option<&std::sync::atomic::AtomicBool>,
    ) -> io::Result<Value> {
        let (mut pipe, remote) = UnixStream::pair()?;
        pipe.set_nonblocking(true)?;
        let mut child = Command::new(&self.executable)
            .args(&self.arguments)
            .envs(&self.environment)
            .stdin(Stdio::from(std::os::fd::OwnedFd::from(remote.try_clone()?)))
            .stdout(Stdio::from(std::os::fd::OwnedFd::from(remote)))
            .stderr(Stdio::null())
            .spawn()?;
        let result = (|| {
            let mut input = serde_json::to_vec(request)?;
            input.push(b'\n');
            let mut sent = 0;
            let mut output = Vec::new();
            loop {
                if cancelled.is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Acquire)) {
                    return Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "Policy provider cancelled",
                    ));
                }
                if Instant::now() >= deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "Policy provider deadline exceeded",
                    ));
                }
                if sent < input.len() {
                    match pipe.write(&input[sent..]) {
                        Ok(0) => return Err(invalid("provider input closed")),
                        Ok(n) => sent += n,
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                        Err(e) => return Err(e),
                    }
                }
                let mut buffer = [0; 8192];
                match pipe.read(&mut buffer) {
                    Ok(0) => return Err(invalid("provider closed without result")),
                    Ok(n) => output.extend_from_slice(&buffer[..n]),
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                    Err(e) => return Err(e),
                }
                if output.len() > super::MAX_BYTES {
                    return Err(invalid("provider result exceeds limit"));
                }
                if output.ends_with(b"\n") {
                    return serde_json::from_slice(&output).map_err(invalid);
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        })();
        let _ = child.kill();
        let _ = child.wait();
        result
    }
}
