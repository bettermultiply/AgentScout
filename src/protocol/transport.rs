
use std::io::{self, Read, Write};

use serde::Serialize;
use serde::de::DeserializeOwned;

/// 单条 control plane 消息的上限。长度头来自对端，不能在验证前直接用来分配内存。
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024 * 1024;

pub fn write_message<W: Write, T: Serialize>(writer: &mut W, message: &T) -> io::Result<()> {
    let payload = serde_json::to_vec(message)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if payload.len() > MAX_MESSAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "message of {} bytes exceeds the {MAX_MESSAGE_BYTES}-byte limit",
                payload.len()
            ),
        ));
    }
    let length = u32::try_from(payload.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "message too large"))?;
    writer.write_all(&length.to_be_bytes())?;
    writer.write_all(&payload)?;
    writer.flush()
}

/// 读取一条消息；连接正常结束（对端关闭）时返回 `None`。
pub fn read_message<R: Read, T: DeserializeOwned>(reader: &mut R) -> io::Result<Option<T>> {
    let mut header = [0u8; 4];
    match reader.read_exact(&mut header) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }
    let length = u32::from_be_bytes(header) as usize;
    if length > MAX_MESSAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("message length {length} exceeds the {MAX_MESSAGE_BYTES}-byte limit"),
        ));
    }
    let mut payload = vec![0u8; length];
    reader.read_exact(&mut payload)?;
    let message = serde_json::from_slice(&payload)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    Ok(Some(message))
}
