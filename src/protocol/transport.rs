//! 会话上的消息编解码：4 字节大端长度前缀 + JSON 负载。

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn read_message_rejects_an_oversized_length_before_allocating() {
        let mut input = Cursor::new(vec![0xff, 0xff, 0xff, 0xff]);
        let error = read_message::<_, serde_json::Value>(&mut input).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("exceeds"), "{error}");
    }

    #[test]
    fn read_message_round_trips_a_normal_message() {
        let mut buffer = Vec::new();
        write_message(&mut buffer, &serde_json::json!({"hello": "world"})).unwrap();
        let value: serde_json::Value = read_message(&mut Cursor::new(buffer)).unwrap().unwrap();
        assert_eq!(value["hello"], "world");
    }

    #[test]
    fn write_message_rejects_payloads_over_the_limit() {
        // A JSON string of this size exceeds the limit without needing a
        // custom serializer.
        let payload = "x".repeat(MAX_MESSAGE_BYTES + 1);
        let mut buffer = Vec::new();
        let error = write_message(&mut buffer, &payload).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("exceeds"), "{error}");
    }
}
