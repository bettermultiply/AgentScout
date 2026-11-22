use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{self, BufRead, Read, Write};
use std::os::unix::{fs::FileExt, net::UnixStream};
use std::sync::Arc;

pub const FRAME: usize = 64 * 1024;
pub const HEADER: usize = 1024 * 1024;

pub fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

#[derive(Clone)]
pub enum Body {
    Bytes(Arc<[u8]>),
    File {
        file: Arc<File>,
        offset: u64,
        len: u64,
    },
}

impl Body {
    pub fn bytes(bytes: Vec<u8>) -> Self {
        Self::Bytes(bytes.into())
    }
    pub fn file(file: File, offset: u64, len: u64) -> Self {
        Self::File {
            file: Arc::new(file),
            offset,
            len,
        }
    }
    pub fn len(&self) -> u64 {
        match self {
            Self::Bytes(bytes) => bytes.len() as u64,
            Self::File { len, .. } => *len,
        }
    }
    pub fn reader(&self) -> impl Read + '_ {
        struct Reader<'a> {
            body: &'a Body,
            position: u64,
        }
        impl Read for Reader<'_> {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                let len = buf.len().min(
                    (self.body.len() - self.position)
                        .try_into()
                        .unwrap_or(usize::MAX),
                );
                if len == 0 {
                    return Ok(0);
                }
                let read = match self.body {
                    Body::Bytes(bytes) => {
                        let start = self.position as usize;
                        buf[..len].copy_from_slice(&bytes[start..start + len]);
                        len
                    }
                    Body::File { file, offset, .. } => {
                        file.read_at(&mut buf[..len], offset + self.position)?
                    }
                };
                self.position += read as u64;
                Ok(read)
            }
        }
        Reader {
            body: self,
            position: 0,
        }
    }
    pub fn fingerprint_into(&self, hash: &mut Sha256) -> io::Result<()> {
        hash.update(self.len().to_be_bytes());
        if io::copy(&mut self.reader(), hash)? != self.len() {
            return Err(invalid("incomplete payload"));
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct Message {
    pub meta: Value,
    pub body: Option<Body>,
}
impl From<Value> for Message {
    fn from(meta: Value) -> Self {
        Self { meta, body: None }
    }
}
impl Message {
    pub fn fingerprint(&self) -> io::Result<[u8; 32]> {
        let mut hash = Sha256::new();
        serde_json::to_writer(&mut hash, &self.meta)?;
        if let Some(body) = &self.body {
            body.fingerprint_into(&mut hash)?;
        }
        Ok(hash.finalize().into())
    }
    pub fn write(&self, writer: &mut impl Write) -> io::Result<()> {
        match &self.body {
            Some(body) => write_body(writer, &self.meta, body.len(), body.reader()),
            None => write_header(writer, &self.meta),
        }
    }
}

pub fn write_header(writer: &mut impl Write, meta: &Value) -> io::Result<()> {
    let mut header = Vec::new();
    struct HeaderWriter<'a>(&'a mut Vec<u8>);
    impl Write for HeaderWriter<'_> {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if self.0.len().saturating_add(buf.len()) >= HEADER {
                return Err(invalid("oversized message header"));
            }
            self.0.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(HeaderWriter(&mut header), meta)?;
    header.push(b'\n');
    writer.write_all(&header)
}

pub fn write_body(
    writer: &mut impl Write,
    meta: &Value,
    length: u64,
    mut body: impl Read,
) -> io::Result<()> {
    let mut header = meta.clone();
    header["body_length"] = length.into();
    write_header(writer, &header)?;
    let mut remaining = length;
    let mut buffer = [0; FRAME];
    while remaining > 0 {
        let n = remaining.min(FRAME as u64) as usize;
        body.read_exact(&mut buffer[..n])?;
        writer.write_all(&(n as u32).to_be_bytes())?;
        writer.write_all(&buffer[..n])?;
        remaining -= n as u64;
    }
    writer.write_all(&0_u32.to_be_bytes())
}

pub(crate) fn read_header(reader: &mut impl BufRead) -> io::Result<(Value, Option<u64>)> {
    let mut header = Vec::new();
    reader
        .by_ref()
        .take(HEADER as u64 + 1)
        .read_until(b'\n', &mut header)?;
    if header.is_empty() {
        return Err(io::ErrorKind::UnexpectedEof.into());
    }
    if header.len() > HEADER || !header.ends_with(b"\n") {
        return Err(invalid("oversized or incomplete message header"));
    }
    let mut meta: Value = serde_json::from_slice(&header)?;
    let fields = meta
        .as_object_mut()
        .ok_or_else(|| invalid("message header must be an object"))?;
    if fields.contains_key("framed") {
        return Err(invalid("unsupported execution framing"));
    }
    let length = fields
        .remove("body_length")
        .map(|length| {
            length
                .as_u64()
                .ok_or_else(|| invalid("invalid body length"))
        })
        .transpose()?;
    Ok((meta, length))
}

pub(crate) fn spool_body(reader: &mut impl Read, length: u64) -> io::Result<Body> {
    let mut spool = tempfile::tempfile()?;
    read_body(reader, length, &mut spool)?;
    Ok(Body::file(spool, 0, length))
}

pub fn read(reader: &mut impl BufRead, output_stream: bool) -> io::Result<Message> {
    let (meta, length) = read_header(reader)?;
    let body = if let Some(length) = length {
        if output_stream && matches!(meta["stream"].as_str(), Some("stdout" | "stderr")) {
            if length > FRAME as u64 {
                return Err(invalid("oversized output event"));
            }
            let mut bytes = Vec::new();
            read_body(reader, length, &mut bytes)?;
            Some(Body::bytes(bytes))
        } else {
            Some(spool_body(reader, length)?)
        }
    } else {
        None
    };
    Ok(Message { meta, body })
}

fn read_body(reader: &mut impl Read, length: u64, destination: &mut impl Write) -> io::Result<()> {
    let mut remaining = length;
    let mut buffer = [0; FRAME];
    loop {
        let mut frame = [0; 4];
        reader.read_exact(&mut frame)?;
        let size = u32::from_be_bytes(frame) as usize;
        if size == 0 {
            return if remaining == 0 {
                Ok(())
            } else {
                Err(invalid("incomplete declared body"))
            };
        }
        if size > FRAME || size as u64 > remaining {
            return Err(invalid("invalid body frame length"));
        }
        reader.read_exact(&mut buffer[..size])?;
        destination.write_all(&buffer[..size])?;
        remaining -= size as u64;
    }
}

/// Keep partial frames intact across read timeouts. A half-close asks the
/// target to stop and we still consume its output before reporting terminal.
pub fn operation_result(
    socket: &UnixStream,
    reader: impl Read,
    cancel: &super::backend::CancelHandle,
) -> Result<Message, super::backend::BackendError> {
    use super::backend::BackendError;
    let mut stopping = false;
    let mut reader = io::BufReader::new(PollRead {
        socket,
        input: reader,
        cancel,
        stopping: &mut stopping,
    });
    loop {
        let result = read(&mut reader, true);
        let stopping = *reader.get_ref().stopping;
        let event = result.map_err(|error| {
            if stopping {
                BackendError::interrupted()
            } else {
                BackendError::new(error.to_string())
            }
        })?;
        if let Some(reason) = event.meta["error"].as_str() {
            return Err(if stopping {
                BackendError::interrupted()
            } else {
                BackendError::operation_error(
                    reason,
                    event.meta["error_category"]
                        .as_str()
                        .unwrap_or("backend_error"),
                )
            });
        }
        if event.meta.get("value").is_some() || event.meta.get("exit_code").is_some() {
            return if stopping {
                Err(BackendError::interrupted())
            } else {
                Ok(event)
            };
        }
        if matches!(event.meta["stream"].as_str(), Some("stdout" | "stderr"))
            && event.body.is_some()
        {
            cancel.output_message(event)?;
        } else {
            return Err(BackendError::new("unexpected environment output"));
        }
    }
}

struct PollRead<'a, R> {
    socket: &'a UnixStream,
    input: R,
    cancel: &'a super::backend::CancelHandle,
    stopping: &'a mut bool,
}
impl<R: Read> Read for PollRead<'_, R> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.cancel.is_cancelled() && !*self.stopping {
                self.socket.shutdown(std::net::Shutdown::Write)?;
                *self.stopping = true;
            }
            match self.input.read(bytes) {
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock
                            | io::ErrorKind::TimedOut
                            | io::ErrorKind::Interrupted
                    ) =>
                {
                    continue;
                }
                result => return result,
            }
        }
    }
}
