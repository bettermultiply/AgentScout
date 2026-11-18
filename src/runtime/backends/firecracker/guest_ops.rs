//! guest agent 通信、API 请求与进程清理。

use super::network::*;
use super::*;

/// Firecracker HTTP API 的抽象，便于对生命周期错误做可注入的单元测试。
pub(super) trait FirecrackerApi: Send + Sync {
    fn request(
        &self,
        method: &str,
        socket: &Path,
        path: &str,
        body: &Value,
    ) -> Result<(), BackendError>;

    fn put(&self, socket: &Path, path: &str, body: &Value) -> Result<(), BackendError> {
        self.request("PUT", socket, path, body)
    }

    fn patch(&self, socket: &Path, path: &str, body: &Value) -> Result<(), BackendError> {
        self.request("PATCH", socket, path, body)
    }
}

pub(super) struct RealFirecrackerApi;

impl FirecrackerApi for RealFirecrackerApi {
    fn request(
        &self,
        method: &str,
        socket: &Path,
        path: &str,
        body: &Value,
    ) -> Result<(), BackendError> {
        api_request(method, socket, path, body)
    }
}

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
            operation: None,
            arguments: Value::Null,
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

pub(super) fn guest_operation(
    socket: &Path,
    operation: &str,
    arguments: &crate::runtime::execution_wire::Message,
    cancel: &CancelHandle,
) -> Result<crate::runtime::execution_wire::Message, BackendError> {
    let mut stream = UnixStream::connect(socket).map_err(error)?;
    stream
        .set_read_timeout(Some(Duration::from_millis(50)))
        .map_err(error)?;
    stream
        .write_all(format!("CONNECT {PORT}\n").as_bytes())
        .map_err(error)?;
    let mut reader = BufReader::new(stream.try_clone().map_err(error)?);
    let mut line = String::new();
    read_line_cancellable(&mut reader, &mut line, cancel)?;
    if !line.starts_with("OK ") {
        return Err(BackendError::new("vsock forwarding failed"));
    }
    stream
        .set_write_timeout(Some(Duration::from_secs(30)))
        .map_err(error)?;
    crate::runtime::execution_wire::Message {
        meta: json!({"operation":operation,"arguments":arguments.meta}),
        body: arguments.body.clone(),
    }
    .write(&mut stream)
    .map_err(error)?;
    crate::runtime::execution_wire::operation_result(&stream, reader, cancel)
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

pub(super) fn check_socket_paths(
    api_socket: &Path,
    vsock_socket: &Path,
) -> Result<(), BackendError> {
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

/// 终止 Firecracker 进程组并回收子进程。
///
/// 仅有界轮询；未确认退出时由调用方保留 Child 以便重试。
pub(super) fn kill_environment_process(child: &mut Child) -> Result<(), BackendError> {
    #[cfg(test)]
    if STOP_FAILURE.with(|failure| failure.get()) {
        return Err(BackendError::new("injected process stop failure"));
    }
    let process_group = child.id() as i32;
    if child.try_wait().map_err(error)?.is_some() {
        return Ok(());
    }
    if unsafe { libc::kill(-process_group, libc::SIGKILL) } != 0 {
        let failure = io::Error::last_os_error();
        if failure.raw_os_error() != Some(libc::ESRCH) {
            return Err(BackendError::new(format!(
                "failed to kill Firecracker process group {process_group}: {failure}"
            )));
        }
        // A child may exist without its expected process group (e.g. wrapper exit).
        if child.try_wait().map_err(error)?.is_some() {
            return Ok(());
        }
        child.kill().map_err(|failure| {
            BackendError::new(format!(
                "failed to kill Firecracker child {process_group}: {failure}"
            ))
        })?;
    }
    wait_for_environment_exit(child, Duration::from_secs(5))
}

fn wait_for_environment_exit(child: &mut Child, timeout: Duration) -> Result<(), BackendError> {
    let process_group = child.id();
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return Ok(()),
            Ok(None) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => {
                return Err(BackendError::new(format!(
                    "timed out waiting for Firecracker process {process_group} to exit after SIGKILL"
                )));
            }
            Err(failure) => {
                return Err(BackendError::new(format!(
                    "failed to poll Firecracker process {process_group}: {failure}"
                )));
            }
        }
    }
}

#[cfg(test)]
thread_local! {
    pub(super) static STOP_FAILURE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Firecracker API 响应体上限；API 响应很小，超限说明对端异常。
const MAX_API_RESPONSE_BYTES: usize = 1024 * 1024;

pub(super) fn api_request(
    method: &str,
    socket: &Path,
    path: &str,
    body: &Value,
) -> Result<(), BackendError> {
    let body = serde_json::to_vec(body).map_err(error)?;
    let mut stream = UnixStream::connect(socket).map_err(error)?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(error)?;
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .map_err(error)?;
    stream.write_all(&body).map_err(error)?;
    stream.flush().map_err(error)?;
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
            content_length = value.trim().parse().map_err(|_| {
                BackendError::new(format!(
                    "Firecracker API {method} {path} returned invalid Content-Length: {}",
                    value.trim()
                ))
            })?;
        }
        response.push_str(&line);
        if response.len() > MAX_API_RESPONSE_BYTES {
            return Err(BackendError::new(format!(
                "Firecracker API {method} {path} response headers exceed {MAX_API_RESPONSE_BYTES} bytes"
            )));
        }
        if end {
            break;
        }
    }
    if content_length > MAX_API_RESPONSE_BYTES {
        return Err(BackendError::new(format!(
            "Firecracker API {method} {path} response body exceeds {MAX_API_RESPONSE_BYTES} bytes"
        )));
    }
    if content_length > 0 {
        let mut body = vec![0; content_length];
        reader.read_exact(&mut body).map_err(error)?;
        response.push_str(&String::from_utf8_lossy(&body));
    }
    let status_line = response.lines().next().unwrap_or_default();
    let status_code = status_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| {
            BackendError::new(format!(
                "Firecracker API {method} {path} returned malformed status line: {status_line:?}"
            ))
        })?
        .parse::<u16>()
        .map_err(|_| {
            BackendError::new(format!(
                "Firecracker API {method} {path} returned non-numeric status: {status_line:?}"
            ))
        })?;
    if !(200..300).contains(&status_code) {
        return Err(BackendError::new(format!(
            "Firecracker API {method} {path} failed with status {status_code}: {}",
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

/// debugfs 的 `-R` 参数会按自己的命令语法再解析一次，因此路径不能包含
/// 空白、引号、反斜杠或控制字符。这里拒绝这些字符，而不是交给它做有歧义
/// 的分词。Firecracker 状态目录和 rootfs 都是受控路径，正常使用时不会触发。
fn debugfs_token_is_safe(value: &str) -> bool {
    !value.is_empty()
        && value.chars().all(|character| {
            character.is_ascii_graphic() && !matches!(character, '"' | '\\' | '{' | '}')
        })
}

pub(super) fn debugfs_write(
    rootfs: &Path,
    source: &Path,
    destination: &str,
) -> Result<(), BackendError> {
    let source = source.display().to_string();
    if !debugfs_token_is_safe(&source) {
        return Err(BackendError::new(format!(
            "debugfs source path contains characters that cannot be passed safely: {source:?}"
        )));
    }
    if !debugfs_token_is_safe(destination) {
        return Err(BackendError::new(format!(
            "debugfs destination path contains characters that cannot be passed safely: {destination:?}"
        )));
    }
    run(Command::new("debugfs")
        .args(["-w", "-R"])
        .arg(format!("write {source} {destination}"))
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::net::UnixListener;

    fn serve_once(response: &'static [u8]) -> (PathBuf, thread::JoinHandle<()>) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("api.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            // Drain the whole request before closing; otherwise the client can
            // see EPIPE while it is still writing the body.
            let mut request = Vec::new();
            let mut buffer = [0u8; 4096];
            loop {
                match stream.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(count) => {
                        request.extend_from_slice(&buffer[..count]);
                        let Some(header_end) = request.windows(4).position(|w| w == b"\r\n\r\n")
                        else {
                            continue;
                        };
                        let headers = String::from_utf8_lossy(&request[..header_end]);
                        let content_length = headers
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|value| value.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if request.len() >= header_end + 4 + content_length {
                            break;
                        }
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) =>
                    {
                        break;
                    }
                    Err(_) => break,
                }
            }
            let _ = stream.write_all(response);
        });
        // Keep the directory alive for the test process by leaking it; the
        // listener is already bound and open.
        std::mem::forget(directory);
        (path, handle)
    }

    #[test]
    fn api_request_accepts_success_status() {
        let (path, server) = serve_once(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n");
        api_request("PUT", &path, "/test", &json!({})).unwrap();
        server.join().unwrap();
    }

    #[test]
    fn api_request_rejects_non_success_status() {
        let (path, server) =
            serve_once(b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n");
        let error = api_request("PUT", &path, "/test", &json!({})).unwrap_err();
        assert!(error.message.contains("status 500"), "{error}");
        server.join().unwrap();
    }

    #[test]
    fn api_request_rejects_malformed_content_length() {
        let (path, server) = serve_once(b"HTTP/1.1 204 No Content\r\nContent-Length: nope\r\n\r\n");
        let error = api_request("PUT", &path, "/test", &json!({})).unwrap_err();
        assert!(error.message.contains("invalid Content-Length"), "{error}");
        server.join().unwrap();
    }

    #[test]
    fn api_request_rejects_malformed_status_line() {
        let (path, server) = serve_once(b"nonsense\r\n\r\n");
        let error = api_request("PUT", &path, "/test", &json!({})).unwrap_err();
        assert!(error.message.contains("malformed status line"), "{error}");
        server.join().unwrap();
    }

    #[test]
    fn debugfs_rejects_paths_that_break_its_command_syntax() {
        assert!(debugfs_token_is_safe("/tmp/a-b_c.ext4"));
        assert!(!debugfs_token_is_safe("/tmp/with space"));
        assert!(!debugfs_token_is_safe("/tmp/with\"quote"));
        assert!(!debugfs_token_is_safe("/tmp/with\\backslash"));
        assert!(!debugfs_token_is_safe(""));

        let rootfs = Path::new("/tmp/rootfs.ext4");
        let error =
            debugfs_write(rootfs, Path::new("/tmp/source with space"), "/dest").unwrap_err();
        assert!(error.message.contains("cannot be passed safely"), "{error}");
    }

    #[test]
    fn process_exit_timeout_is_bounded_and_retains_a_retryable_child() {
        let mut child = Command::new("sleep")
            .arg("30")
            .process_group(0)
            .spawn()
            .unwrap();
        let start = Instant::now();
        let result = wait_for_environment_exit(&mut child, Duration::from_millis(20));
        let still_running = child.try_wait().unwrap().is_none();
        kill_environment_process(&mut child).unwrap();
        assert!(result.unwrap_err().message.contains("timed out waiting"));
        assert!(start.elapsed() < Duration::from_secs(1));
        assert!(still_running);
        kill_environment_process(&mut child).unwrap();
    }

    #[test]
    fn missing_process_group_falls_back_to_child_kill() {
        let mut child = Command::new("sleep").arg("30").spawn().unwrap();
        kill_environment_process(&mut child).unwrap();
        assert!(child.try_wait().unwrap().is_some());
    }
}
