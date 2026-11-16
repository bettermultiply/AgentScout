//! microVM 内的 Agent 接入侧。
//!
//! 接入侧监听固定的 vsock 端口，接收 argv，直接在 guest 内启动进程并返回输出。

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

pub const PORT: u32 = 52;

/// 首行请求（argv JSON）上限。
const MAX_REQUEST_BYTES: usize = 1024 * 1024;
/// 单条流（stdout 或 stderr）保留给控制面的上限；超出时终止命令并返回错误。
const MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
/// 同时执行的 guest 连接上限。
const MAX_CONNECTIONS: usize = 64;
/// 读取请求首行和写出响应的超时。
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Serialize, Deserialize)]
pub struct GuestRequest {
    pub command: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GuestResponse {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub fn serve() -> io::Result<()> {
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
    let request = {
        let mut reader = BufReader::new(stream.try_clone()?);
        let line = read_request_line(&mut reader).map_err(|error| {
            if matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ) {
                io::Error::new(io::ErrorKind::TimedOut, "timed out reading guest request")
            } else {
                error
            }
        })?;
        serde_json::from_str::<GuestRequest>(&line)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
    };
    if request.command.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "empty command"));
    }

    let mut command = Command::new(&request.command[0]);
    command
        .args(&request.command[1..])
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
