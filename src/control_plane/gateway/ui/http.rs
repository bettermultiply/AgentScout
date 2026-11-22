use super::{Assets, Connection, Presentation, Request};
use crate::control_plane::ui_projection::UiProjection;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub struct Server {
    stopped: Arc<AtomicBool>,
    reader: UiProjection,
    worker: Option<JoinHandle<io::Result<()>>>,
}

impl Server {
    pub fn stop(&mut self) -> io::Result<()> {
        self.stopped.store(true, Ordering::Release);
        self.reader.wake_readers();
        match self.worker.take() {
            Some(worker) => worker
                .join()
                .map_err(|_| io::Error::other("UI listener panicked"))?,
            None => Ok(()),
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

pub fn start<P: Presentation + Clone + 'static>(
    address: &str,
    interface: P,
    connection: Connection,
    reader: UiProjection,
    assets: Assets,
) -> io::Result<Server> {
    let address: std::net::SocketAddr = address
        .parse()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    if !address.ip().is_loopback() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Browser UI must bind a loopback address",
        ));
    }
    let listener = TcpListener::bind(address)?;
    listener.set_nonblocking(true)?;
    let address = listener.local_addr()?;
    eprintln!("AScout browser UI: http://{address}");
    let stopped = Arc::new(AtomicBool::new(false));
    let worker_stopped = stopped.clone();
    let wake_reader = reader.clone();
    let worker = thread::Builder::new()
        .name("ui-listener".into())
        .spawn(move || {
            let mut workers: Vec<(TcpStream, JoinHandle<()>)> = Vec::new();
            let result = (|| -> io::Result<()> {
                while !worker_stopped.load(Ordering::Acquire) {
                    let mut index = 0;
                    while index < workers.len() {
                        if workers[index].1.is_finished() {
                            let (_, worker) = workers.swap_remove(index);
                            let _ = worker.join();
                        } else {
                            index += 1;
                        }
                    }
                    let (mut stream, _) = match listener.accept() {
                        Ok(pair) => pair,
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(20));
                            continue;
                        }
                        Err(error) => return Err(error),
                    };
                    let socket = stream.try_clone()?;
                    let interface = interface.clone();
                    let connection = connection.clone();
                    let reader = reader.clone();
                    let stopped = worker_stopped.clone();
                    let host = address.to_string();
                    let worker =
                        thread::Builder::new()
                            .name("ui-connection".into())
                            .spawn(move || {
                                let _ = serve(
                                    &mut stream,
                                    &interface,
                                    &connection,
                                    &reader,
                                    &host,
                                    assets,
                                    &stopped,
                                );
                            })?;
                    workers.push((socket, worker));
                }
                Ok(())
            })();
            drop(listener);
            worker_stopped.store(true, Ordering::Release);
            reader.wake_readers();
            for (socket, _) in &workers {
                // Stop idle reads while allowing streams to drain final events.
                let _ = socket.shutdown(std::net::Shutdown::Read);
            }
            for (_, worker) in workers {
                let _ = worker.join();
            }
            result
        })?;
    Ok(Server {
        stopped,
        reader: wake_reader,
        worker: Some(worker),
    })
}

fn serve<P: Presentation>(
    stream: &mut TcpStream,
    interface: &P,
    connection: &Connection,
    reader: &UiProjection,
    host: &str,
    assets: Assets,
    stopped: &AtomicBool,
) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let mut bytes = Vec::new();
    let header_end = loop {
        let mut buffer = [0; 4096];
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            return Ok(());
        }
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(index) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            break index + 4;
        }
        if bytes.len() > 16384 {
            return reply(
                stream,
                "413 Payload Too Large",
                "text/plain",
                b"Header too large",
            );
        }
    };
    let header = String::from_utf8_lossy(&bytes[..header_end]).into_owned();
    let mut lines = header.lines();
    let first = lines.next().unwrap_or("");
    let fields: Vec<_> = first.split_whitespace().collect();
    let headers: Vec<_> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(k, v)| (k.to_ascii_lowercase(), v.trim()))
        .collect();
    let get = |name: &str| headers.iter().find(|(k, _)| k == name).map(|(_, v)| *v);
    if get("host") != Some(host) {
        return reply(stream, "403 Forbidden", "text/plain", b"Invalid host");
    }
    match fields.as_slice() {
        ["GET", "/", _] | ["GET", "/index.html", _] | ["GET", "/map", _] => reply(
            stream,
            "200 OK",
            "text/html; charset=utf-8",
            assets.index_html,
        ),
        ["GET", "/assets/app.js", _] => reply(
            stream,
            "200 OK",
            "text/javascript; charset=utf-8",
            assets.app_js,
        ),
        ["GET", "/assets/app.css", _] => {
            reply(stream, "200 OK", "text/css; charset=utf-8", assets.app_css)
        }
        ["GET", "/favicon.svg", _] => reply(stream, "200 OK", "image/svg+xml", assets.favicon_svg),
        ["GET", "/api/observation/state-event-set", _] => {
            let body = serde_json::to_vec(&reader.state_event_set())?;
            reply(stream, "200 OK", "application/json", &body)
        }
        ["GET", "/api/input-status", _] => reply(
            stream,
            "200 OK",
            "application/json",
            &serde_json::to_vec(&interface.input_status())?,
        ),
        ["GET", path, _] if path.starts_with("/api/observation/events") => {
            let (path, query) = path.split_once('?').unwrap_or((path, ""));
            if path != "/api/observation/events" {
                return reply(stream, "404 Not Found", "text/plain", b"Not found");
            }
            let after = query.strip_prefix("after=");
            if !query.is_empty() && after.is_none() {
                return reply(
                    stream,
                    "400 Bad Request",
                    "text/plain",
                    b"Expected after sequence",
                );
            }
            // On reconnect, EventSource's Last-Event-ID is newer than the initial query.
            let sequence = get("last-event-id").or(after).unwrap_or("0").parse::<u64>();
            let Ok(mut sequence) = sequence else {
                return reply(stream, "400 Bad Request", "text/plain", b"Invalid sequence");
            };
            if sequence > reader.sequence() {
                return reply(
                    stream,
                    "409 Conflict",
                    "text/plain",
                    b"Sequence ahead of this run; reload state event set",
                );
            }
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n"
            )?;
            let reader = reader.clone();
            let mut warning_count = 0;
            loop {
                for warning in reader.warnings().into_iter().skip(warning_count) {
                    writeln!(
                        stream,
                        "event: system-warning\ndata: {}\n",
                        serde_json::to_string(&warning)?
                    )?;
                    warning_count += 1;
                }
                stream.flush()?;
                let events = reader
                    .events_after_until(sequence, stopped)
                    .map_err(io::Error::other)?;
                if stopped.load(Ordering::Acquire) && reader.connected() {
                    return Ok(());
                }
                for event in events {
                    sequence = event.sequence;
                    writeln!(
                        stream,
                        "id: {sequence}\ndata: {}\n",
                        serde_json::to_string(&event)?
                    )?;
                }
                stream.write_all(b": keep-alive\n\n")?;
                stream.flush()?;
                if !reader.connected() && sequence == reader.sequence() {
                    return Ok(());
                }
            }
        }
        ["GET", path, _]
            if path.starts_with("/api/branches/") && path.ends_with("/conversation") =>
        {
            let branch = &path["/api/branches/".len()..path.len() - "/conversation".len()];
            let Some(branch) = decode_segment(branch) else {
                return reply(
                    stream,
                    "400 Bad Request",
                    "text/plain",
                    b"Invalid Branch id",
                );
            };
            match interface.conversation(&branch) {
                Some(conversation) => reply(
                    stream,
                    "200 OK",
                    "application/json",
                    &serde_json::to_vec(&conversation)?,
                ),
                None => reply(stream, "404 Not Found", "text/plain", b"Unknown Branch"),
            }
        }
        ["POST", "/api/request", _] => {
            let origin = format!("http://{host}");
            if get("origin") != Some(origin.as_str())
                || get("content-type") != Some("application/json")
                || get("transfer-encoding").is_some()
            {
                return reply(
                    stream,
                    "403 Forbidden",
                    "text/plain",
                    b"Same-origin JSON requests only",
                );
            }
            let Some(length) = get("content-length")
                .and_then(|s| s.parse::<usize>().ok())
                .filter(|n| *n <= 65536)
            else {
                return reply(
                    stream,
                    "413 Payload Too Large",
                    "text/plain",
                    b"Invalid body size",
                );
            };
            while bytes.len() < header_end + length {
                let mut buffer = [0; 4096];
                let count = stream.read(&mut buffer)?;
                if count == 0 {
                    return Ok(());
                }
                bytes.extend_from_slice(&buffer[..count]);
            }
            let request =
                serde_json::from_slice::<Request>(&bytes[header_end..header_end + length]);
            match request {
                Ok(Request::Prompt { ref prompt }) if prompt.trim().is_empty() => {
                    reply(stream, "400 Bad Request", "text/plain", b"Prompt is empty")
                }
                Ok(request) => {
                    let interaction = matches!(
                        request,
                        Request::UiPromptAnswer { .. } | Request::GetCommands { .. }
                    );
                    match connection.request(request) {
                        Ok(()) => reply(
                            stream,
                            "202 Accepted",
                            "application/json",
                            b"{\"queued\":true}",
                        ),
                        Err(error) => reply(
                            stream,
                            if interaction {
                                "409 Conflict"
                            } else {
                                "503 Service Unavailable"
                            },
                            "text/plain",
                            error.as_bytes(),
                        ),
                    }
                }
                Err(error) => reply(
                    stream,
                    "400 Bad Request",
                    "text/plain",
                    error.to_string().as_bytes(),
                ),
            }
        }
        _ => reply(stream, "404 Not Found", "text/plain", b"Not found"),
    }
}

fn decode_segment(value: &str) -> Option<String> {
    let mut bytes = value.bytes();
    let mut decoded = Vec::new();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let high = (bytes.next()? as char).to_digit(16)?;
            let low = (bytes.next()? as char).to_digit(16)?;
            decoded.push((high * 16 + low) as u8);
        } else {
            decoded.push(byte);
        }
    }
    String::from_utf8(decoded).ok()
}

fn reply(stream: &mut TcpStream, status: &str, kind: &str, body: &[u8]) -> io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nContent-Security-Policy: default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self'; img-src 'self' data:; base-uri 'none'; object-src 'none'; frame-ancestors 'none'\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)
}
