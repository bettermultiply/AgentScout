//! microVM 内的 Guest Agent。
//!
//! Guest Agent 监听固定的 vsock 端口，接收 argv，直接在 guest 内启动进程并返回输出。

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

/// 首行请求（argv JSON）上限。
pub(super) const MAX_REQUEST_BYTES: usize = 1024 * 1024;

/// 单条流（stdout 或 stderr）保留给control plane的上限；超出时终止命令并返回错误。
const MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
/// 同时执行的 guest 连接上限。
const MAX_CONNECTIONS: usize = 64;
/// 读取请求首行和写出响应的超时。
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
    // 读取请求以便客户端完成写入；内容本身不需要解析。
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
    // 让命令进入独立会话，取消时可以终止整个进程组。
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
                // Snapshot bytes already buffered when the foreground exited.
                // Drain all of them, without an arbitrary time/size cutoff or
                // allowing a continuous background writer to delay terminal.
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
    // Foreground output is drained before terminal; background output continues
    // to be consumed after the client leaves rather than giving writers SIGPIPE.
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

/// 有界读取：超过上限后继续排空管道，但不继续占用内存。
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

/// 取消时终止整个进程组，避免命令的子进程变成孤儿。
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    use crate::runtime::execution_wire as wire;

    fn contents(message: &wire::Message) -> Vec<u8> {
        let mut bytes = Vec::new();
        message
            .body
            .as_ref()
            .unwrap()
            .reader()
            .read_to_end(&mut bytes)
            .unwrap();
        bytes
    }

    #[test]
    fn generic_files_and_command_stream_use_native_guest_operations() {
        let root = tempfile::tempdir_in("/tmp/opencode").unwrap();
        let path = root.path().join("binary");
        let request = |operation: &str, arguments: Value, body: Option<Vec<u8>>| {
            let (mut client, server) = UnixStream::pair().unwrap();
            let worker = thread::spawn(|| serve_connection(server));
            wire::Message {
                meta: json!({"operation":operation,"arguments":arguments}),
                body: body.map(wire::Body::bytes),
            }
            .write(&mut client)
            .unwrap();
            let mut reader = BufReader::new(client);
            let mut events = Vec::new();
            loop {
                let event = wire::read(&mut reader, true).unwrap();
                let terminal = event.meta["stream"].is_null();
                events.push(event);
                if terminal {
                    break;
                }
            }
            worker.join().unwrap().unwrap();
            events
        };
        let bytes = vec![0, 255, 10];
        let written = request("write_file", json!({"path":path}), Some(bytes.clone()));
        assert_eq!(written[0].meta["value"]["bytes_written"], 3);
        let read = request("read_file", json!({"path":path}), None);
        assert_eq!(contents(&read[0]), bytes);
        assert_eq!(
            read[0].meta["value"]["actual_range"],
            json!({"offset":0,"limit":3})
        );

        let (mut client, server) = UnixStream::pair().unwrap();
        let worker = thread::spawn(|| serve_connection(server));
        wire::write_header(
            &mut client,
            &json!({"operation":"write_file","arguments":{"path":path},"body_length":4}),
        )
        .unwrap();
        client.write_all(b"\0\0\0\x03bad\0\0\0\0").unwrap();
        client.shutdown(std::net::Shutdown::Write).unwrap();
        let rejected = wire::read(&mut BufReader::new(client), false).unwrap();
        assert_eq!(rejected.meta["error_category"], "invalid_input");
        worker.join().unwrap().unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);

        let events = request(
            "exec",
            json!({"cwd":root.path(),"command":"printf early; sleep 0.05; printf late >&2; exit 7"}),
            None,
        );
        assert_eq!(events.last().unwrap().meta["exit_code"], 7);
        let output: Vec<u8> = events
            .iter()
            .filter(|event| event.body.is_some())
            .flat_map(contents)
            .collect();
        assert_eq!(output, b"earlylate");
        assert!(events.iter().any(|event| event.meta["stream"] == "stderr"));
        let events = request("exec", json!({"command":"head -c 2097152 /dev/zero"}), None);
        assert_eq!(
            events
                .iter()
                .filter_map(|event| event.body.as_ref())
                .map(wire::Body::len)
                .sum::<u64>(),
            2097152
        );
    }

    #[test]
    fn generic_command_cancellation_stops_the_process_group() {
        let root = tempfile::tempdir_in("/tmp/opencode").unwrap();
        let (mut client, server) = UnixStream::pair().unwrap();
        let worker = thread::spawn(|| serve_connection(server));
        wire::write_header(&mut client,&json!({"operation":"exec","arguments":{"cwd":root.path(),"command":"(sleep 0.4; echo leaked > leak) & sleep 0.05; echo ready; sleep 30"}})).unwrap();
        let mut reader = BufReader::new(client);
        let ready = wire::read(&mut reader, true).unwrap();
        assert_eq!(contents(&ready), b"ready\n");
        reader
            .get_ref()
            .shutdown(std::net::Shutdown::Write)
            .unwrap();
        assert!(
            matches!(wire::read(&mut reader, true), Err(error) if error.kind() == io::ErrorKind::UnexpectedEof)
        );
        worker.join().unwrap().unwrap();
        thread::sleep(Duration::from_millis(450));
        assert!(!root.path().join("leak").exists());
    }

    #[test]
    fn read_all_stops_storing_after_the_limit() {
        let input = vec![b'x'; MAX_OUTPUT_BYTES + 1024];
        let exceeded = Arc::new(AtomicBool::new(false));
        let bytes = read_all(Cursor::new(input), exceeded.clone());
        assert_eq!(bytes.len(), MAX_OUTPUT_BYTES);
        assert!(exceeded.load(Ordering::SeqCst));
    }

    #[test]
    fn read_all_keeps_output_below_the_limit() {
        let exceeded = Arc::new(AtomicBool::new(false));
        let bytes = read_all(Cursor::new(b"short output".to_vec()), exceeded.clone());
        assert_eq!(bytes, b"short output");
        assert!(!exceeded.load(Ordering::SeqCst));
    }

    #[test]
    fn oversized_first_line_is_rejected() {
        let input = vec![b'a'; MAX_REQUEST_BYTES + 16];
        let mut reader = BufReader::new(Cursor::new(input));
        let error = read_request_line(&mut reader).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("exceeds"), "{error}");
    }

    #[test]
    fn normal_first_line_is_accepted() {
        let mut reader = BufReader::new(Cursor::new(b"{\"command\":[\"true\"]}\n".to_vec()));
        let line = read_request_line(&mut reader).unwrap();
        assert_eq!(line.trim(), "{\"command\":[\"true\"]}");
    }
}
