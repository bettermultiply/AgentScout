//! Loopback-only browser transport. Both facts and actions use the UI interface.
use super::ui::{Interface, Request};
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

pub fn start(address: &str, interface: Interface) -> io::Result<()> {
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
    let address = listener.local_addr()?;
    eprintln!("AScout browser UI: http://{address}");
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                break;
            };
            let interface = interface.clone();
            let host = address.to_string();
            std::thread::spawn(move || {
                let _ = serve(&mut stream, &interface, &host);
            });
        }
    });
    Ok(())
}

fn serve(stream: &mut TcpStream, interface: &Interface, host: &str) -> io::Result<()> {
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
        ["GET", "/", _] => reply(
            stream,
            "200 OK",
            "text/html; charset=utf-8",
            include_bytes!("web/index.html"),
        ),
        ["GET", "/api/observation/snapshot", _] => {
            let body = serde_json::to_vec(&interface.snapshot())?;
            reply(stream, "200 OK", "application/json", &body)
        }
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
            // EventSource sends Last-Event-ID on automatic reconnect; it supersedes the initial query.
            let sequence = get("last-event-id").or(after).unwrap_or("0").parse::<u64>();
            let Ok(mut sequence) = sequence else {
                return reply(stream, "400 Bad Request", "text/plain", b"Invalid sequence");
            };
            if sequence > interface.sequence() {
                return reply(
                    stream,
                    "409 Conflict",
                    "text/plain",
                    b"Sequence ahead of this run; reload snapshot",
                );
            }
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n"
            )?;
            loop {
                let events = interface.events_after(sequence).map_err(io::Error::other)?;
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
                if !interface.connected() && sequence == interface.sequence() {
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
                Ok(request) => match interface.request(request) {
                    Ok(()) => reply(
                        stream,
                        "202 Accepted",
                        "application/json",
                        b"{\"queued\":true}",
                    ),
                    Err(error) => reply(
                        stream,
                        "503 Service Unavailable",
                        "text/plain",
                        error.as_bytes(),
                    ),
                },
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
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nContent-Security-Policy: default-src 'self'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'self'; frame-ancestors 'none'\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)
}
