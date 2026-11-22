
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const PORT: u32 = 52;

pub(super) const MAX_REQUEST_BYTES: usize = 1024 * 1024;

const MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
const MAX_CONNECTIONS: usize = 64;
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Serialize, Deserialize)]
pub struct GuestRequest {
    #[serde(default)]
    pub command: Vec<String>,
    #[serde(default)]
    pub operation: Option<String>,
    #[serde(default)]
    pub arguments: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GuestResponse {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub fn serve() -> io::Result<()> {
    if unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let listener = vsock_listener(PORT)?;
    let active = Arc::new(AtomicUsize::new(0));
    loop {
        let descriptor =
            unsafe { libc::accept(listener, std::ptr::null_mut(), std::ptr::null_mut()) };
        if descriptor < 0 {
            return Err(io::Error::last_os_error());
        }
        let active = active.clone();
        thread::spawn(move || {
            let stream = unsafe { UnixStream::from_raw_fd(descriptor) };
            if active.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
                active.fetch_sub(1, Ordering::SeqCst);
                let _ = reject_connection(stream);
                return;
            }
            let _ = serve_connection(stream);
            active.fetch_sub(1, Ordering::SeqCst);
        });
    }
}

fn reject_connection(mut stream: UnixStream) -> io::Result<()> {
    stream.set_read_timeout(Some(CONNECTION_TIMEOUT))?;
    stream.set_write_timeout(Some(CONNECTION_TIMEOUT))?;
    let _ = read_request_line(&mut BufReader::new(stream.try_clone()?));
    let response = GuestResponse {
        exit_code: -1,
        stdout: String::new(),
        stderr: format!(
            "guest access rejected connection: maximum of {MAX_CONNECTIONS} concurrent connections reached"
        ),
    };
    serde_json::to_writer(&mut stream, &response)?;
    stream.write_all(b"\n")?;
    stream.flush()
}

fn read_request_line(reader: &mut impl BufRead) -> io::Result<String> {
    let mut line = Vec::new();
    reader
        .by_ref()
        .take((MAX_REQUEST_BYTES + 1) as u64)
        .read_until(b'\n', &mut line)?;
    if line.len() > MAX_REQUEST_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("guest request exceeds {MAX_REQUEST_BYTES} bytes"),
        ));
    }
    String::from_utf8(line).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn serve_connection(mut stream: UnixStream) -> io::Result<()> {
    stream.set_read_timeout(Some(CONNECTION_TIMEOUT))?;
    stream.set_write_timeout(Some(CONNECTION_TIMEOUT))?;
    use crate::runtime::execution_wire::{self as wire, Message};
    let inbound = match wire::read(&mut BufReader::new(stream.try_clone()?), false) {
        Ok(message) => message,
        Err(error) => {
            return wire::write_header(
                &mut stream,
                &json!({"error":error.to_string(),"error_category":"invalid_input"}),
            );
        }
    };
    let request: GuestRequest = serde_json::from_value(inbound.meta)?;
    if let Some(operation) = request.operation.as_deref() {
        if let Err(error) = crate::runtime::environment::validate_body(
            operation,
            &request.arguments,
            inbound.body.as_ref().map(wire::Body::len),
        ) {
            return wire::write_header(
                &mut stream,
                &json!({"error":error.to_string(),"error_category":"invalid_input"}),
            );
        }
        if operation == "drain" {
            loop {
                let writers = descendants(std::process::id())?;
                if writers.is_empty() {
                    break;
                }
                for pid in writers {
                    unsafe {
                        libc::kill(pid as i32, libc::SIGKILL);
                    }
                }
                thread::sleep(Duration::from_millis(10));
            }
            while unsafe { libc::waitpid(-1, std::ptr::null_mut(), libc::WNOHANG) } > 0 {}
            serde_json::to_writer(&mut stream, &json!({"value":null}))?;
            stream.write_all(b"\n")?;
            return stream.flush();
        }
        if operation == "exec" {
            return execute_environment_command(stream, &request.arguments, inbound.body);
        }
        let response = crate::runtime::environment::file_operation(operation, &request.arguments, inbound.body.as_ref())
            .unwrap_or_else(|error| Message::from(json!({"error":error.to_string(),"error_category":crate::runtime::environment::error_category(&error)})));
        return response.write(&mut stream);
    }
    if inbound.body.is_some() {
        return wire::write_header(&mut stream, &json!({"error":"unexpected command body"}));
    }
    let argv = request.command;
    if argv.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "empty command"));
    }

    let mut command = Command::new(&argv[0]);
    command
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn()?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let stdout_exceeded = Arc::new(AtomicBool::new(false));
    let stderr_exceeded = Arc::new(AtomicBool::new(false));
    let stdout_flag = stdout_exceeded.clone();
    let stderr_flag = stderr_exceeded.clone();
    let stdout_reader = thread::spawn(move || read_all(stdout, stdout_flag));
    let stderr_reader = thread::spawn(move || read_all(stderr, stderr_flag));

    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if stdout_exceeded.load(Ordering::SeqCst) || stderr_exceeded.load(Ordering::SeqCst) {
            kill_process_group(child.id());
        }
        if peer_disconnected(stream.as_raw_fd())? {
            kill_process_group(child.id());
            let _ = child.wait();
            return Ok(());
        }
        thread::sleep(Duration::from_millis(25));
    };
    let stdout_exceeded = stdout_exceeded.load(Ordering::SeqCst);
    let stderr_exceeded = stderr_exceeded.load(Ordering::SeqCst);
    let stdout = stdout_reader.join().unwrap_or_default();
    let stderr = stderr_reader.join().unwrap_or_default();
    if stdout_exceeded || stderr_exceeded {
        let message = format!(
            "command output exceeded {MAX_OUTPUT_BYTES} bytes on {}; output truncated",
            if stdout_exceeded && stderr_exceeded {
                "stdout and stderr"
            } else if stdout_exceeded {
                "stdout"
            } else {
                "stderr"
            }
        );
        return write_response(
            &mut stream,
            GuestResponse {
                exit_code: 1,
                stdout: String::from_utf8_lossy(&stdout).into_owned(),
                stderr: message,
            },
        );
    }
    let response = GuestResponse {
        exit_code: status.code().unwrap_or(128),
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
    };
    write_response(&mut stream, response)
}

fn descendants(root: u32) -> io::Result<Vec<u32>> {
    let mut processes = Vec::new();
    for entry in std::fs::read_dir("/proc")? {
        let entry = entry?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        if let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) {
            if let Some((_, fields)) = stat.rsplit_once(')') {
                let fields: Vec<_> = fields.split_whitespace().collect();
                if let Some(parent) = fields.get(1).and_then(|s| s.parse::<u32>().ok()) {
                    processes.push((pid, parent, fields[0] == "Z"));
                }
            }
        }
    }
    let mut owned = vec![root];
    loop {
        let added: Vec<_> = processes
            .iter()
            .filter(|(pid, parent, _)| !owned.contains(pid) && owned.contains(parent))
            .map(|(pid, _, _)| *pid)
            .collect();
        if added.is_empty() {
            break;
        }
        owned.extend(added);
    }
    Ok(processes
        .iter()
        .filter(|(pid, _, zombie)| *pid != root && owned.contains(pid) && !*zombie)
        .map(|(pid, _, _)| *pid)
        .collect())
}

fn execute_environment_command(
    mut stream: UnixStream,
    args: &Value,
    input: Option<crate::runtime::execution_wire::Body>,
) -> io::Result<()> {
    use crate::runtime::execution_wire::{self as wire, Body};
    use std::io::{Seek, SeekFrom};
    let stdin = match input {
        Some(Body::File { file, offset, .. }) => {
            let mut file = file.try_clone()?;
            file.seek(SeekFrom::Start(offset))?;
            Stdio::from(file)
        }
        None => Stdio::null(),
        Some(Body::Bytes(_)) => return Err(wire::invalid("expected spooled stdin")),
    };
    let mut command = Command::new("/bin/bash");
    command
        .arg("-c")
        .arg(
            args["command"]
                .as_str()
                .ok_or_else(|| io::Error::other("command required"))?,
        )
        .current_dir(
            std::path::Path::new(args["bound_cwd"].as_str().unwrap_or("/"))
                .join(args["cwd"].as_str().unwrap_or(".")),
        )
        .stdin(stdin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(env) = args["env"].as_object() {
        for (key, value) in env {
            if let Some(value) = value.as_str() {
                command.env(key, value);
            }
        }
    }
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    let mut child = command.spawn()?;
    let mut pipes: Vec<Box<dyn ReadFd>> = vec![
        Box::new(child.stdout.take().unwrap()),
        Box::new(child.stderr.take().unwrap()),
    ];
    for pipe in &pipes {
        let fd = pipe.as_raw_fd();
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            kill_process_group(child.id());
            let _ = child.wait();
            return Err(io::Error::last_os_error());
        }
    }
    let started = std::time::Instant::now();
    let timeout = args["timeout"]
        .as_f64()
        .filter(|n| n.is_finite() && *n > 0.0)
        .map(Duration::from_secs_f64);
    let mut failure = None;
    let mut disconnected = false;
    let mut status = None;
    let mut terminal_remaining = None::<Vec<usize>>;
    loop {
        for (index, pipe) in pipes.iter_mut().enumerate() {
            let mut buffer = [0; 8192];
            let capacity = terminal_remaining
                .as_ref()
                .map_or(buffer.len(), |remaining| remaining[index].min(buffer.len()));
            if capacity == 0 {
                continue;
            }
            match pipe.read(&mut buffer[..capacity]) {
                Ok(0) => {}
                Ok(count) => {
                    if let Some(remaining) = terminal_remaining.as_mut() {
                        remaining[index] = remaining[index].saturating_sub(count);
                    }
                    let event = json!({"stream":if index==0 {"stdout"} else {"stderr"}});
                    if !disconnected
                        && wire::write_body(&mut stream, &event, count as u64, &buffer[..count])
                            .is_err()
                    {
                        disconnected = true;
                    }
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                Err(e) => {
                    kill_process_group(child.id());
                    let _ = child.wait();
                    return Err(e);
                }
            }
        }
        if status.is_none() {
            status = child.try_wait()?;
            if status.is_some() {
                // Drain the bytes buffered at exit. Background writes cannot extend this boundary.
                let mut remaining = Vec::new();
                for pipe in &pipes {
                    let mut count: libc::c_int = 0;
                    if unsafe { libc::ioctl(pipe.as_raw_fd(), libc::FIONREAD, &mut count) } < 0 {
                        return Err(io::Error::last_os_error());
                    }
                    remaining.push(count.max(0) as usize);
                }
                terminal_remaining = Some(remaining);
            }
        }
        if terminal_remaining
            .as_ref()
            .is_some_and(|remaining| remaining.iter().all(|count| *count == 0))
        {
            break;
        }
        disconnected |= peer_disconnected(stream.as_raw_fd())?;
        if disconnected || timeout.is_some_and(|limit| started.elapsed() >= limit) {
            kill_process_group(child.id());
            failure = Some(if disconnected {
                "Tool cancelled"
            } else {
                "Tool timed out"
            });
        }
        thread::sleep(Duration::from_millis(5));
    }
    // Keep reading background output after the client leaves to avoid SIGPIPE.
    for mut pipe in pipes {
        thread::spawn(move || {
            let mut buffer = [0; 8192];
            loop {
                match pipe.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(_) => {}
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10))
                    }
                    Err(_) => break,
                }
            }
        });
    }
    if disconnected {
        return Ok(());
    }
    let event = if let Some(failure) = failure {
        json!({"error":failure,"error_category":if failure == "Tool timed out" {"timed_out"} else {"cancelled"}})
    } else {
        json!({"exit_code":status.unwrap().code().unwrap_or(128)})
    };
    wire::write_header(&mut stream, &event)
}

trait ReadFd: Read + AsRawFd + Send {}
impl<T: Read + AsRawFd + Send> ReadFd for T {}

fn write_response(stream: &mut UnixStream, response: GuestResponse) -> io::Result<()> {
    serde_json::to_writer(&mut *stream, &response)?;
    stream.write_all(b"\n")?;
    stream.flush()
}

fn read_all(mut reader: impl Read, exceeded: Arc<AtomicBool>) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                let remaining = MAX_OUTPUT_BYTES.saturating_sub(bytes.len());
                let keep = count.min(remaining);
                bytes.extend_from_slice(&buffer[..keep]);
                if keep < count {
                    exceeded.store(true, Ordering::SeqCst);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    bytes
}

fn kill_process_group(pid: u32) {
    let process_group = pid as i32;
    unsafe {
        libc::kill(-process_group, libc::SIGKILL);
    }
}

fn peer_disconnected(descriptor: RawFd) -> io::Result<bool> {
    let mut byte = 0_u8;
    let result = unsafe {
        libc::recv(
            descriptor,
            (&mut byte as *mut u8).cast(),
            1,
            libc::MSG_PEEK | libc::MSG_DONTWAIT,
        )
    };
    if result == 0 {
        return Ok(true);
    }
    if result < 0 {
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::WouldBlock {
            return Ok(false);
        }
        return Err(error);
    }
    Ok(false)
}

fn vsock_listener(port: u32) -> io::Result<RawFd> {
    let descriptor = unsafe { libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM, 0) };
    if descriptor < 0 {
        return Err(io::Error::last_os_error());
    }
    let address = libc::sockaddr_vm {
        svm_family: libc::AF_VSOCK as libc::sa_family_t,
        svm_reserved1: 0,
        svm_port: port,
        svm_cid: libc::VMADDR_CID_ANY,
        svm_zero: [0; 4],
    };
    let result = unsafe {
        libc::bind(
            descriptor,
            (&address as *const libc::sockaddr_vm).cast(),
            std::mem::size_of::<libc::sockaddr_vm>() as libc::socklen_t,
        )
    };
    if result < 0 || unsafe { libc::listen(descriptor, 16) } < 0 {
        let error = io::Error::last_os_error();
        unsafe { libc::close(descriptor) };
        return Err(error);
    }
    Ok(descriptor)
}
