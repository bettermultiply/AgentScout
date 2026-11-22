use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::{
    fs::PermissionsExt,
    net::{UnixListener, UnixStream},
    process::{CommandExt, ExitStatusExt},
};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, Weak};
use std::thread;
use std::time::{Duration, Instant};

const HEADER_LIMIT: usize = 1024 * 1024;
const FRAME_SIZE: usize = 64 * 1024;
const MAX_LINE_BYTES: u64 = 32 * 1024 * 1024;
const POLL: Duration = Duration::from_millis(10);
const FLUSH: Duration = Duration::from_millis(500);

type Mutations = HashMap<PathBuf, Weak<Mutex<()>>>;

#[derive(Default)]
struct Holder {
    spool_directory: PathBuf,
    writers: Mutex<HashSet<i32>>,
    mutations: Mutex<Mutations>,
}

pub fn listen(socket: &Path) -> io::Result<UnixListener> {
    if unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) } < 0 {
        return Err(io::Error::last_os_error());
    }
    match fs::remove_file(socket) {
        Ok(()) => (),
        Err(e) if e.kind() == io::ErrorKind::NotFound => (),
        Err(e) => return Err(e),
    }
    let listener = UnixListener::bind(socket)?;
    fs::set_permissions(socket, fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

pub fn serve(listener: UnixListener) -> io::Result<()> {
    let address = listener.local_addr()?;
    let spool_directory = address
        .as_pathname()
        .and_then(Path::parent)
        .ok_or_else(|| io::Error::other("missing socket directory"))?
        .to_owned();
    let holder = Arc::new(Holder {
        spool_directory,
        ..Holder::default()
    });
    let reaper = holder.clone();
    thread::spawn(move || {
        loop {
            thread::sleep(Duration::from_millis(100));
            let writers = reaper.writers.lock().unwrap();
            let me = std::process::id() as i32;
            for (pid, (parent, state)) in processes() {
                if parent == me && state == b'Z' && !writers.contains(&pid) {
                    unsafe {
                        libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG);
                    }
                }
            }
        }
    });
    for incoming in listener.incoming() {
        let mut socket = incoming?;
        let holder = holder.clone();
        thread::spawn(move || {
            if let Err(error) = holder.request(&mut socket) {
                let category = match error.kind() {
                    io::ErrorKind::NotFound => "not_found",
                    io::ErrorKind::PermissionDenied => "permission_denied",
                    io::ErrorKind::InvalidInput | io::ErrorKind::InvalidData => "invalid_input",
                    io::ErrorKind::TimedOut => "timed_out",
                    io::ErrorKind::Interrupted => "cancelled",
                    _ => "io_error",
                };
                let _ = header(
                    &mut socket,
                    json!({"error":error.to_string(),"error_category":category}),
                );
            }
        });
    }
    Ok(())
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

fn string<'a>(v: &'a Value, field: &str) -> io::Result<&'a str> {
    v[field]
        .as_str()
        .ok_or_else(|| invalid(format!("invalid {field}")))
}

fn number(v: &Value, field: &str, default: Option<u64>) -> io::Result<u64> {
    v.get(field)
        .map_or(default, Value::as_u64)
        .ok_or_else(|| invalid(format!("invalid {field}")))
}

fn enabled(v: &Value, field: &str) -> bool {
    v[field] == true
}

fn header(socket: &mut UnixStream, value: Value) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(&value)?;
    bytes.push(b'\n');
    if bytes.len() > HEADER_LIMIT {
        return Err(invalid("oversized message header"));
    }
    socket.write_all(&bytes)
}

fn frame(socket: &mut UnixStream, bytes: &[u8]) -> io::Result<()> {
    socket.write_all(&(bytes.len() as u32).to_be_bytes())?;
    socket.write_all(bytes)
}

fn send_body(
    socket: &mut UnixStream,
    mut value: Value,
    mut source: impl Read,
    mut length: u64,
) -> io::Result<()> {
    value["body_length"] = json!(length);
    header(socket, value)?;
    let mut buffer = [0; FRAME_SIZE];
    let sent = (|| {
        while length > 0 {
            let size = source.read(&mut buffer[..length.min(FRAME_SIZE as u64) as usize])?;
            if size == 0 {
                return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
            }
            frame(socket, &buffer[..size])?;
            length -= size as u64;
        }
        frame(socket, &[])
    })();
    if sent.is_err() {
        let _ = socket.shutdown(std::net::Shutdown::Write);
    }
    sent
}

fn exact(reader: &mut impl Read, bytes: &mut [u8]) -> io::Result<()> {
    reader.read_exact(bytes).map_err(|e| {
        if e.kind() == io::ErrorKind::UnexpectedEof {
            invalid("incomplete body frame")
        } else {
            e
        }
    })
}

impl Holder {
    fn request(&self, socket: &mut UnixStream) -> io::Result<()> {
        socket.set_read_timeout(Some(Duration::from_secs(30)))?;
        socket.set_write_timeout(Some(Duration::from_secs(30)))?;
        let mut reader = BufReader::new(&mut *socket);
        let mut bytes = Vec::new();
        (&mut reader)
            .take((HEADER_LIMIT + 1) as u64)
            .read_until(b'\n', &mut bytes)?;
        if bytes.len() > HEADER_LIMIT || !bytes.ends_with(b"\n") {
            return Err(invalid("oversized or incomplete message header"));
        }
        let request: Value = serde_json::from_slice(&bytes).map_err(|e| invalid(e.to_string()))?;
        if !request.is_object()
            || ["framed", "bytes", "input"]
                .iter()
                .any(|key| request.get(key).is_some())
        {
            return Err(invalid("invalid request header"));
        }
        let op = string(&request, "op")?;
        let mut spool = None;
        let length = request
            .get("body_length")
            .map(|_| number(&request, "body_length", None))
            .transpose()?;
        if let Some(mut remaining) = length {
            if !matches!(op, "exec" | "write_file" | "replace_text") {
                return Err(invalid("unexpected operation body"));
            }
            let mut file = tempfile::tempfile_in(&self.spool_directory)?;
            let mut buffer = [0; FRAME_SIZE];
            loop {
                let mut size = [0; 4];
                exact(&mut reader, &mut size)?;
                let size = u32::from_be_bytes(size) as usize;
                if size == 0 {
                    if remaining != 0 {
                        return Err(invalid("incomplete declared body"));
                    }
                    break;
                }
                if size > FRAME_SIZE || size as u64 > remaining {
                    return Err(invalid("invalid body frame length"));
                }
                exact(&mut reader, &mut buffer[..size])?;
                file.write_all(&buffer[..size])?;
                remaining -= size as u64;
            }
            file.rewind()?;
            spool = Some(file);
        }
        match op {
            "exec" => self.exec(socket, &request, spool),
            "drain" => self.drain(socket, enabled(&request, "cancel")),
            "read_file" | "list_directory" | "write_file" | "replace_text" => {
                let path = Path::new(request["bound_cwd"].as_str().unwrap_or("/"))
                    .join(string(&request, "path")?);
                match op {
                    "read_file" => self.read_file(socket, &request, &path),
                    "list_directory" => self.list(socket, &path),
                    _ => self.mutate(
                        socket,
                        op,
                        &request,
                        &path,
                        spool.ok_or_else(|| invalid("operation requires a body"))?,
                        length.unwrap(),
                    ),
                }
            }
            _ => Err(io::Error::other("unknown holder operation")),
        }
    }

    fn list(&self, socket: &mut UnixStream, path: &Path) -> io::Result<()> {
        let mut entries = Vec::new();
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let ty = entry.file_type()?;
            let kind = if ty.is_symlink() {
                "symlink"
            } else if ty.is_dir() {
                "directory"
            } else if ty.is_file() {
                "file"
            } else {
                "other"
            };
            entries.push((entry.file_name(), kind));
        }
        entries.sort_unstable();
        let entries: Vec<_> = entries
            .into_iter()
            .map(|(name, kind)| json!({"name":name.to_string_lossy(),"type":kind}))
            .collect();
        let mut output = tempfile::tempfile_in(&self.spool_directory)?;
        serde_json::to_writer(&mut output, &json!({"entries":entries}))?;
        let size = output.stream_position()?;
        output.rewind()?;
        send_body(
            socket,
            json!({"value":{},"body_kind":"json_value"}),
            output,
            size,
        )
    }

    fn read_file(&self, socket: &mut UnixStream, request: &Value, path: &Path) -> io::Result<()> {
        if request.get("line_range").is_some() && request.get("byte_range").is_some() {
            return Err(invalid("byte_range and line_range are mutually exclusive"));
        }
        let mut source = BufReader::new(File::open(path)?);
        let prefix = source.fill_buf()?;
        if enabled(request, "binary_fallback")
            && (prefix.starts_with(b"\x89PNG")
                || prefix.starts_with(b"\xff\xd8\xff")
                || prefix.starts_with(b"GIF8")
                || prefix.starts_with(b"BM")
                || (prefix.starts_with(b"RIFF") && prefix.get(8..12) == Some(b"WEBP")))
        {
            let size = source.get_ref().metadata()?.len();
            return send_body(
                socket,
                json!({"value":{"representation":"binary"},"body_kind":"file_bytes"}),
                source,
                size,
            );
        }
        if let Some(bounds) = request.get("line_range") {
            let offset = number(bounds, "offset", None)?;
            let limit = number(bounds, "limit", None)?;
            if offset == 0 {
                return Err(invalid("invalid line range"));
            }
            let max_bytes =
                number(request, "max_bytes", Some(MAX_LINE_BYTES))?.min(MAX_LINE_BYTES) as usize;
            let (mut line, mut selected, mut completed) = (1, 0, 0);
            let (mut eof, mut truncated) = (false, false);
            let (mut chunk, mut content) = (Vec::new(), Vec::new());
            while line < offset || selected < limit {
                chunk.clear();
                (&mut source)
                    .take(FRAME_SIZE as u64)
                    .read_until(b'\n', &mut chunk)?;
                if chunk.is_empty() {
                    if line < offset {
                        return Err(invalid("Offset is beyond end of file"));
                    }
                    eof = true;
                    selected += 1;
                    break;
                }
                if line >= offset {
                    if content.len() + chunk.len() > max_bytes {
                        content.truncate(completed);
                        truncated = true;
                        break;
                    }
                    content.extend_from_slice(&chunk);
                }
                if chunk.ends_with(b"\n") {
                    if line >= offset {
                        selected += 1;
                        completed = content.len();
                    }
                    line += 1;
                }
            }
            if !eof && content.ends_with(b"\n") {
                content.pop();
            }
            std::str::from_utf8(&content).map_err(|e| invalid(e.to_string()))?;
            return send_body(
                socket,
                range(offset, selected, !eof, truncated),
                content.as_slice(),
                content.len() as u64,
            );
        }
        let bounds = &request["byte_range"];
        let offset = number(bounds, "offset", Some(0))?;
        let limit = bounds
            .get("limit")
            .filter(|v| !v.is_null())
            .map(|_| number(bounds, "limit", None))
            .transpose()?;
        if offset != 0 {
            source.seek(SeekFrom::Start(offset))?;
        }
        let size = source.get_ref().metadata()?.len();
        if size == 0 && limit.is_some_and(|n| n != 0) {
            let mut output = tempfile::tempfile_in(&self.spool_directory)?;
            let size = io::copy(&mut source.take(limit.unwrap()), &mut output)?;
            output.rewind()?;
            return send_body(socket, range(offset, size, false, false), output, size);
        }
        let available = size.saturating_sub(offset);
        let size = limit.unwrap_or(available).min(available);
        send_body(
            socket,
            range(offset, size, size < available, false),
            source,
            size,
        )
    }

    fn mutation_lock(&self, path: &Path) -> (PathBuf, Arc<Mutex<()>>) {
        let path = canonical(path);
        let mut locks = self.mutations.lock().unwrap();
        let lock = locks.get(&path).and_then(Weak::upgrade).unwrap_or_else(|| {
            let lock = Arc::new(Mutex::new(()));
            locks.insert(path.clone(), Arc::downgrade(&lock));
            lock
        });
        (path, lock)
    }

    fn mutate(
        &self,
        socket: &mut UnixStream,
        op: &str,
        request: &Value,
        path: &Path,
        mut spool: File,
        length: u64,
    ) -> io::Result<()> {
        let (path, lock) = self.mutation_lock(path);
        let result = {
            let _guard = lock.lock().unwrap();
            if op == "write_file" {
                (|| {
                    if enabled(request, "create_parents") {
                        fs::create_dir_all(
                            path.parent().ok_or_else(|| invalid("invalid file path"))?,
                        )?;
                    }
                    atomic_write(&path, &mut spool)?;
                    Ok(json!({"bytes_written":length}))
                })()
            } else {
                replace_text(&path, request, spool, length)
            }
        };
        let mut locks = self.mutations.lock().unwrap();
        drop(lock);
        if locks
            .get(&path)
            .is_some_and(|lock| lock.strong_count() == 0)
        {
            locks.remove(&path);
        }
        drop(locks);
        header(socket, json!({"value":result?}))
    }

    fn drain(&self, socket: &mut UnixStream, cancel: bool) -> io::Result<()> {
        loop {
            let writers = self.writers.lock().unwrap();
            loop {
                let children = owned_writers();
                if children.is_empty() {
                    return header(socket, json!({"exit_code":0}));
                }
                if !cancel {
                    break;
                }
                for pid in children {
                    unsafe {
                        libc::kill(pid, libc::SIGKILL);
                    }
                }
                thread::sleep(POLL);
            }
            drop(writers);
            if client_closed(socket) {
                return Ok(());
            }
            thread::sleep(POLL);
        }
    }

    fn exec(
        &self,
        socket: &mut UnixStream,
        request: &Value,
        stdin: Option<File>,
    ) -> io::Result<()> {
        let mut command = Command::new("/bin/bash");
        command
            .arg("-c")
            .arg(string(request, "command")?)
            .current_dir(
                Path::new(request["bound_cwd"].as_str().unwrap_or("/"))
                    .join(request["cwd"].as_str().unwrap_or(".")),
            )
            .stdin(stdin.map_or_else(Stdio::null, Stdio::from))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(env) = request.get("env").filter(|v| !v.is_null()) {
            for (key, value) in env
                .as_object()
                .ok_or_else(|| invalid("invalid command environment"))?
            {
                command.env(
                    key,
                    value
                        .as_str()
                        .ok_or_else(|| invalid("invalid environment value"))?,
                );
            }
        }
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut writers = self.writers.lock().unwrap();
        let mut child = command.spawn()?;
        let pid = child.id() as i32;
        writers.insert(pid);
        drop(writers);
        let output = child.stdout.take().unwrap();
        let errors = child.stderr.take().unwrap();
        let pipes = [output.as_raw_fd(), errors.as_raw_fd()];
        let timeout = request["timeout"]
            .as_f64()
            .and_then(|seconds| Duration::try_from_secs_f64(seconds).ok())
            .filter(|time| !time.is_zero());
        let started = Instant::now();
        // Bound a blocked peer's send so the process and its pipes are always reaped.
        let streamed = forward(socket, pipes, || child.try_wait(), started, timeout);
        if streamed.is_err() {
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
        }
        let status = child.wait();
        self.writers.lock().unwrap().remove(&pid);
        // Keep pipe readers alive for detached children. They keep running
        // after successful foreground exit and must not receive SIGPIPE.
        if let Ok(open) = streamed.as_ref() {
            if open[0] {
                thread::spawn(move || {
                    let _ = io::copy(&mut { output }, &mut io::sink());
                });
            }
            if open[1] {
                thread::spawn(move || {
                    let _ = io::copy(&mut { errors }, &mut io::sink());
                });
            }
        }
        streamed?;
        let status = status?;
        header(
            socket,
            json!({"exit_code":status.code().unwrap_or_else(|| -status.signal().unwrap_or(1))}),
        )
    }
}

fn range(offset: u64, limit: u64, has_more: bool, truncated: bool) -> Value {
    json!({"value":{"actual_range":{"offset":offset,"limit":limit},"has_more":has_more,"truncated":truncated},"body_kind":"file_bytes"})
}

// Resolve symlinks even when the target or its ancestors will be created by
// this write. Existing prefixes use the kernel's canonical spelling.
fn canonical(path: &Path) -> PathBuf {
    let mut result = PathBuf::from("/");
    let mut components: std::collections::VecDeque<_> = path
        .components()
        .map(|c| c.as_os_str().to_owned())
        .collect();
    let mut links = 0;
    while let Some(component) = components.pop_front() {
        if component == "/" {
            result = PathBuf::from("/");
            continue;
        }
        if component == ".." {
            result.pop();
            continue;
        }
        if component == "." {
            continue;
        }
        result.push(component);
        if links < 40
            && let Ok(target) = fs::read_link(&result)
        {
            result.pop();
            links += 1;
            for part in target.components().rev() {
                components.push_front(part.as_os_str().to_owned());
            }
        }
    }
    result
}

fn atomic_write(path: &Path, content: &mut impl Read) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| invalid("invalid file path"))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    let permissions = match fs::metadata(path) {
        Ok(metadata) => Some(metadata.permissions()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    io::copy(content, &mut temporary)?;
    if let Some(mode) = permissions {
        temporary.as_file().set_permissions(mode)?;
    }
    temporary.persist(path).map_err(|e| e.error)?;
    Ok(())
}

fn replace_text(path: &Path, request: &Value, mut spool: File, length: u64) -> io::Result<Value> {
    let replacements = request["replacements"]
        .as_array()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| invalid("replacements must not be empty"))?;
    let mut sizes = Vec::new();
    let mut total = 0_u64;
    for edit in replacements {
        if !edit.as_object().is_some_and(|v| {
            v.keys()
                .all(|key| ["old_length", "new_length", "match"].contains(&key.as_str()))
        }) {
            return Err(invalid("invalid replacement metadata"));
        }
        let old = number(edit, "old_length", None)?;
        let new = number(edit, "new_length", None)?;
        let mode = if edit.get("match").is_some() {
            string(edit, "match")?
        } else {
            "exactly_one"
        };
        if old == 0 || !matches!(mode, "all" | "exactly_one") {
            return Err(invalid("invalid replacement lengths or match"));
        }
        total = total
            .checked_add(old)
            .and_then(|n| n.checked_add(new))
            .ok_or_else(|| invalid("invalid replacement body lengths"))?;
        sizes.push((old, new, mode));
    }
    if total != length {
        return Err(invalid("invalid replacement body lengths"));
    }
    let original = fs::read_to_string(path)?;
    let mut text = original.clone();
    let mut counts = Vec::new();
    for (old_size, new_size, mode) in sizes {
        let mut old = String::new();
        let mut new = String::new();
        (&mut spool).take(old_size).read_to_string(&mut old)?;
        (&mut spool).take(new_size).read_to_string(&mut new)?;
        let count = text.matches(&old).count();
        if count == 0 {
            return Err(invalid("Could not find exact old_text"));
        }
        if mode == "exactly_one" && count != 1 {
            return Err(invalid(
                "old_text is ambiguous; expected exactly one occurrence",
            ));
        }
        text = text.replace(&old, &new);
        counts.push(count);
    }
    let changed = text != original;
    if changed {
        atomic_write(path, &mut text.as_bytes())?;
    }
    Ok(json!({"counts":counts,"changed":changed,"bytes_written":if changed {text.len()} else {0}}))
}

fn processes() -> HashMap<i32, (i32, u8)> {
    let Ok(entries) = fs::read_dir("/proc") else {
        return HashMap::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let pid = entry.file_name().to_str()?.parse().ok()?;
            let stat = fs::read(entry.path().join("stat")).ok()?;
            let close = stat.iter().rposition(|&b| b == b')')?;
            let mut fields = stat[close + 1..]
                .split(|b| b.is_ascii_whitespace())
                .filter(|v| !v.is_empty());
            let state = *fields.next()?.first()?;
            let ppid = std::str::from_utf8(fields.next()?).ok()?.parse().ok()?;
            Some((pid, (ppid, state)))
        })
        .collect()
}

fn owned_writers() -> Vec<i32> {
    let me = std::process::id() as i32;
    let processes = processes();
    let mut owned = HashSet::from([me]);
    loop {
        let found: Vec<_> = processes
            .iter()
            .filter(|(pid, (ppid, _))| owned.contains(ppid) && !owned.contains(pid))
            .map(|(&pid, _)| pid)
            .collect();
        if found.is_empty() {
            break;
        }
        owned.extend(found);
    }
    owned
        .into_iter()
        .filter(|pid| *pid != me && processes[pid].1 != b'Z')
        .collect()
}

fn client_closed(socket: &UnixStream) -> bool {
    let mut byte = 0_u8;
    let result = unsafe {
        libc::recv(
            socket.as_raw_fd(),
            (&mut byte as *mut u8).cast(),
            1,
            libc::MSG_PEEK | libc::MSG_DONTWAIT,
        )
    };
    result == 0
        || (result < 0
            && !matches!(
                io::Error::last_os_error().kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
            ))
}

fn stream_event(
    socket: &UnixStream,
    stream: &str,
    bytes: &[u8],
    deadline: Instant,
) -> io::Result<()> {
    let mut event = serde_json::to_vec(&json!({"stream":stream,"body_length":bytes.len()}))?;
    event.push(b'\n');
    event.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    event.extend_from_slice(bytes);
    event.extend_from_slice(&[0; 4]);
    let mut sent = 0;
    let result = (|| {
        while sent < event.len() {
            let size = unsafe {
                libc::send(
                    socket.as_raw_fd(),
                    event[sent..].as_ptr().cast(),
                    event.len() - sent,
                    libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL,
                )
            };
            if size > 0 {
                sent += size as usize;
                continue;
            }
            let error = io::Error::last_os_error();
            match error.kind() {
                io::ErrorKind::Interrupted => continue,
                io::ErrorKind::WouldBlock => (),
                _ => return Err(error),
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Tool timed out while streaming output",
                ));
            }
            let mut fd = libc::pollfd {
                fd: socket.as_raw_fd(),
                events: libc::POLLOUT,
                revents: 0,
            };
            if unsafe { libc::poll(&mut fd, 1, remaining.as_millis().max(1) as i32) } < 0 {
                let failure = io::Error::last_os_error();
                if failure.kind() != io::ErrorKind::Interrupted {
                    return Err(failure);
                }
            }
        }
        Ok(())
    })();
    if result.is_err() && sent != 0 {
        // No terminal header can follow an incomplete body.
        let _ = socket.shutdown(std::net::Shutdown::Write);
    }
    result
}

fn forward(
    socket: &mut UnixStream,
    pipes: [RawFd; 2],
    mut status: impl FnMut() -> io::Result<Option<std::process::ExitStatus>>,
    started: Instant,
    timeout: Option<Duration>,
) -> io::Result<[bool; 2]> {
    let mut fds = pipes.map(|fd| libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    });
    let mut foreground_at = None;
    let mut buffer = [0_u8; FRAME_SIZE];
    loop {
        if foreground_at.is_none() && status()?.is_some() {
            foreground_at = Some(Instant::now());
        }
        if client_closed(socket) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "Tool cancelled"));
        }
        if foreground_at.is_none() && timeout.is_some_and(|t| started.elapsed() >= t) {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "Tool timed out"));
        }
        if foreground_at.is_some() && fds.iter().all(|fd| fd.fd < 0) {
            break;
        }
        let ready = unsafe { libc::poll(fds.as_mut_ptr(), 2, POLL.as_millis() as i32) };
        if ready < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        if foreground_at.is_some() && ready == 0 {
            break;
        }
        for (fd, stream) in fds.iter_mut().zip(["stdout", "stderr"]) {
            if fd.revents == 0 {
                continue;
            }
            let size = unsafe { libc::read(fd.fd, buffer.as_mut_ptr().cast(), buffer.len()) };
            if size == 0 {
                fd.fd = -1;
                continue;
            }
            if size < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error);
            }
            let mut deadline = Instant::now() + Duration::from_secs(30);
            if let Some(at) = foreground_at {
                deadline = deadline.min(at + FLUSH);
            } else if let Some(at) = timeout.and_then(|value| started.checked_add(value)) {
                deadline = deadline.min(at);
            }
            stream_event(socket, stream, &buffer[..size as usize], deadline)?;
        }
        if foreground_at.is_some_and(|at| at.elapsed() >= FLUSH) {
            break;
        }
    }
    Ok(fds.map(|fd| fd.fd >= 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_aliases_share_a_mutation_lock() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target");
        let alias = directory.path().join("alias");
        std::os::unix::fs::symlink(&target, &alias).unwrap();
        let holder = Holder::default();
        let (_, first) = holder.mutation_lock(&target);
        let held = first.lock().unwrap();
        let (_, second) = holder.mutation_lock(&alias);
        assert!(second.try_lock().is_err());
        drop(held);
        assert!(second.try_lock().is_ok());
    }
}
