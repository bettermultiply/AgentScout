//! guest agent 通信、API 请求与进程清理。

use super::*;
use super::network::*;

pub(super) fn guest_command(
    socket: &Path,
    command: &[String],
    cancel: &CancelHandle,
) -> Result<CommandOutput, BackendError> {
    let mut stream = UnixStream::connect(socket).map_err(error)?;
    stream
        .set_read_timeout(Some(Duration::from_millis(50)))
        .map_err(error)?;
    stream
        .write_all(format!("CONNECT {PORT}\n").as_bytes())
        .map_err(error)?;
    let mut reader = BufReader::new(stream.try_clone().map_err(error)?);
    let mut forwarding = String::new();
    read_line_cancellable(&mut reader, &mut forwarding, cancel)?;
    if !forwarding.starts_with("OK ") {
        return Err(BackendError::new(format!(
            "vsock forwarding failed: {}",
            forwarding.trim()
        )));
    }
    serde_json::to_writer(
        &mut stream,
        &GuestRequest {
            command: command.to_vec(),
        },
    )
    .map_err(error)?;
    stream.write_all(b"\n").map_err(error)?;
    stream.flush().map_err(error)?;

    let mut response = String::new();
    if let Err(failure) = read_line_cancellable(&mut reader, &mut response, cancel) {
        if failure.cancelled {
            // 先通知 guest 取消，再等它结束命令进程组，避免环境在残留清理前冻结。
            let _ = stream.shutdown(Shutdown::Write);
            wait_for_guest_cleanup(&mut reader);
        } else {
            let _ = stream.shutdown(Shutdown::Both);
        }
        return Err(failure);
    }
    let response: GuestResponse = serde_json::from_str(&response).map_err(error)?;
    Ok(CommandOutput {
        exit_code: response.exit_code,
        stdout: response.stdout,
        stderr: response.stderr,
    })
}

pub(super) fn read_line_cancellable(
    reader: &mut BufReader<UnixStream>,
    output: &mut String,
    cancel: &CancelHandle,
) -> Result<(), BackendError> {
    loop {
        if cancel.is_cancelled() {
            return Err(BackendError::interrupted());
        }
        match reader.read_line(output) {
            Ok(0) => return Err(BackendError::new("guest access closed the connection")),
            Ok(_) => return Ok(()),
            Err(failure)
                if matches!(
                    failure.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(failure) => return Err(error(failure)),
        }
    }
}

/// 取消后等待 guest 关闭连接，作为命令进程组已结束的确认。
pub(super) fn wait_for_guest_cleanup(reader: &mut BufReader<UnixStream>) {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut scratch = String::new();
    while Instant::now() < deadline {
        match reader.read_line(&mut scratch) {
            Ok(0) => return,
            Ok(_) => scratch.clear(),
            Err(failure)
                if matches!(
                    failure.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(_) => return,
        }
    }
}

pub(super) fn wait_for_guest(socket: &Path, timeout: Duration) -> Result<(), BackendError> {
    let deadline = Instant::now() + timeout;
    let cancel = CancelHandle::default();
    loop {
        let command = vec!["true".to_string()];
        if guest_command(socket, &command, &cancel).is_ok() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(BackendError::new("guest Agent access did not become ready"));
        }
        thread::sleep(Duration::from_millis(100));
    }
}

/// 让 guest 的网络真正可用：镜像只配置接口地址，默认路由由环境建立时补齐。
pub(super) fn configure_guest_network(socket: &Path) -> Result<(), BackendError> {
    let cancel = CancelHandle::default();
    let command = vec![
        "ip".to_string(),
        "route".to_string(),
        "replace".to_string(),
        "default".to_string(),
        "via".to_string(),
        BRIDGE_GATEWAY.to_string(),
    ];
    guest_command(socket, &command, &cancel).map(|_| ())
}

pub(super) fn check_socket_paths(api_socket: &Path, vsock_socket: &Path) -> Result<(), BackendError> {
    for socket in [api_socket, vsock_socket] {
        if socket.as_os_str().as_encoded_bytes().len() >= 108 {
            return Err(BackendError::new(format!(
                "Unix socket path is too long: {}",
                socket.display()
            )));
        }
    }
    Ok(())
}

pub(super) fn spawn_firecracker(
    binary: &Path,
    id: &str,
    directory: &Path,
    api_socket: &Path,
    network: &EnvironmentNetwork,
) -> Result<Child, BackendError> {
    let console = File::create(directory.join("console.log")).map_err(error)?;
    let mut command = network.firecracker_command(binary);
    command
        .args(["--id", id, "--api-sock"])
        .arg(api_socket)
        .stdout(console.try_clone().map_err(error)?)
        .stderr(console)
        // 独立进程组：Firecracker 经 sudo 链启动，终止时需要回收整组。
        .process_group(0);
    command.spawn().map_err(error)
}

pub(super) fn kill_environment_process(child: &mut Child) {
    let process_group = child.id() as i32;
    unsafe {
        libc::kill(-process_group, libc::SIGKILL);
    }
    let _ = child.wait();
}

pub(super) fn api_put(socket: &Path, path: &str, body: &Value) -> Result<(), BackendError> {
    api_request("PUT", socket, path, body)
}

pub(super) fn api_patch(socket: &Path, path: &str, body: &Value) -> Result<(), BackendError> {
    api_request("PATCH", socket, path, body)
}

pub(super) fn api_request(method: &str, socket: &Path, path: &str, body: &Value) -> Result<(), BackendError> {
    let body = serde_json::to_vec(body).map_err(error)?;
    let mut stream = UnixStream::connect(socket).map_err(error)?;
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .map_err(error)?;
    stream.write_all(&body).map_err(error)?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(error)?;
    let mut reader = BufReader::new(stream);
    let mut response = String::new();
    let mut content_length = 0_usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).map_err(error)? == 0 {
            break;
        }
        let end = line == "\r\n";
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            content_length = value.trim().parse().unwrap_or(0);
        }
        response.push_str(&line);
        if end {
            break;
        }
    }
    if content_length > 0 {
        let mut body = vec![0; content_length];
        reader.read_exact(&mut body).map_err(error)?;
        response.push_str(&String::from_utf8_lossy(&body));
    }
    let status = response.lines().next().unwrap_or_default();
    if !status.contains(" 204 ") && !status.contains(" 200 ") {
        return Err(BackendError::new(format!(
            "Firecracker API {method} {path} failed: {}",
            response.trim()
        )));
    }
    Ok(())
}

pub(super) fn wait_for_path(path: &Path, timeout: Duration) -> Result<(), BackendError> {
    let deadline = Instant::now() + timeout;
    while !path.exists() {
        if Instant::now() >= deadline {
            return Err(BackendError::new(format!(
                "timed out waiting for {}",
                path.display()
            )));
        }
        thread::sleep(Duration::from_millis(20));
    }
    Ok(())
}

pub(super) fn debugfs_write(rootfs: &Path, source: &Path, destination: &str) -> Result<(), BackendError> {
    run(Command::new("debugfs")
        .args(["-w", "-R"])
        .arg(format!("write {} {destination}", source.display()))
        .arg(rootfs))
}

pub(super) fn run(command: &mut Command) -> Result<(), BackendError> {
    let output = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(error)?;
    if !output.status.success() {
        return Err(BackendError::new(format!(
            "command failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

pub(super) fn error(failure: impl std::fmt::Display) -> BackendError {
    BackendError::new(failure.to_string())
}
