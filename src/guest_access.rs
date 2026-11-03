//! microVM 内的 Agent 接入侧。
//!
//! 接入侧监听固定的 vsock 端口，接收 argv，直接在 guest 内启动进程并返回输出。

use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::fd::{FromRawFd, RawFd};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub const PORT: u32 = 52;

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
    loop {
        let descriptor =
            unsafe { libc::accept(listener, std::ptr::null_mut(), std::ptr::null_mut()) };
        if descriptor < 0 {
            return Err(io::Error::last_os_error());
        }
        thread::spawn(move || {
            let stream = unsafe { File::from_raw_fd(descriptor) };
            let _ = serve_connection(stream);
        });
    }
}

fn serve_connection(mut stream: File) -> io::Result<()> {
    let request = {
        let mut line = String::new();
        BufReader::new(stream.try_clone()?).read_line(&mut line)?;
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
    let stdout_reader = thread::spawn(move || read_all(stdout));
    let stderr_reader = thread::spawn(move || read_all(stderr));

    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if peer_disconnected(stream.as_raw_fd())? {
            kill_process_group(child.id());
            let _ = child.wait();
            return Ok(());
        }
        thread::sleep(Duration::from_millis(25));
    };
    let response = GuestResponse {
        exit_code: status.code().unwrap_or(128),
        stdout: String::from_utf8_lossy(&stdout_reader.join().unwrap_or_default()).into_owned(),
        stderr: String::from_utf8_lossy(&stderr_reader.join().unwrap_or_default()).into_owned(),
    };
    serde_json::to_writer(&mut stream, &response)?;
    stream.write_all(b"\n")?;
    stream.flush()
}

fn read_all(mut reader: impl Read) -> Vec<u8> {
    let mut bytes = Vec::new();
    let _ = reader.read_to_end(&mut bytes);
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

trait AsRawFd {
    fn as_raw_fd(&self) -> RawFd;
}

impl AsRawFd for File {
    fn as_raw_fd(&self) -> RawFd {
        std::os::fd::AsRawFd::as_raw_fd(self)
    }
}
