use super::execution_wire::{Body, Message, invalid};
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::BufRead;
use std::{
    collections::HashMap,
    io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, Weak},
};

pub fn error_category(error: &io::Error) -> &'static str {
    match error.kind() {
        io::ErrorKind::InvalidInput | io::ErrorKind::InvalidData => "invalid_input",
        io::ErrorKind::NotFound => "not_found",
        io::ErrorKind::PermissionDenied => "permission_denied",
        io::ErrorKind::TimedOut => "timed_out",
        _ => "io_error",
    }
}

const MAX_LINE_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Replacement {
    old_length: u64,
    new_length: u64,
    #[serde(default, rename = "match")]
    mode: Match,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Match {
    #[default]
    ExactlyOne,
    All,
}

fn replacements(args: &Value, length: u64) -> io::Result<Vec<Replacement>> {
    let edits: Vec<Replacement> = serde_json::from_value(args["replacements"].clone())?;
    let total = edits.iter().try_fold(0_u64, |total, edit| {
        total
            .checked_add(edit.old_length)?
            .checked_add(edit.new_length)
    });
    if edits.is_empty() || edits.iter().any(|edit| edit.old_length == 0) || total != Some(length) {
        return Err(invalid("invalid replacement body lengths"));
    }
    Ok(edits)
}

pub fn validate_body(operation: &str, args: &Value, body_length: Option<u64>) -> io::Result<()> {
    if !args.is_object() || args.get("bytes").is_some() || args.get("input").is_some() {
        return Err(invalid(
            "expected operation metadata and a separate binary body",
        ));
    }
    match (operation, body_length) {
        ("write_file" | "replace_text", None) => Err(invalid("operation requires a body")),
        ("replace_text", Some(length)) => replacements(args, length).map(|_| ()),
        ("write_file" | "exec", _) | (_, None) => Ok(()),
        _ => Err(invalid("unexpected operation body")),
    }
}

fn canonical_target(path: &Path) -> io::Result<PathBuf> {
    match std::fs::canonicalize(path) {
        Ok(path) => Ok(path),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let parent = path.parent().ok_or_else(|| invalid("path has no parent"))?;
            Ok(canonical_target(parent)?
                .join(path.file_name().ok_or_else(|| invalid("invalid path"))?))
        }
        Err(e) => Err(e),
    }
}

fn atomic_write(path: &Path, mut source: impl Read, length: u64) -> io::Result<()> {
    let mut temporary = tempfile::NamedTempFile::new_in(
        path.parent().ok_or_else(|| invalid("path has no parent"))?,
    )?;
    if let Ok(metadata) = std::fs::metadata(path) {
        temporary
            .as_file()
            .set_permissions(metadata.permissions())?;
    }
    if io::copy(&mut source, &mut temporary)? != length {
        return Err(invalid("incomplete write body"));
    }
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|e| e.error)?;
    Ok(())
}

pub fn file_operation(operation: &str, args: &Value, body: Option<&Body>) -> io::Result<Message> {
    validate_body(operation, args, body.map(Body::len))?;
    let path = args["path"]
        .as_str()
        .ok_or_else(|| invalid("path is required"))?;
    let path = Path::new(args["bound_cwd"].as_str().unwrap_or("/")).join(path);
    if operation == "list_directory" {
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            entries.push(json!({"name":entry.file_name().to_string_lossy(),"type":if kind.is_symlink(){"symlink"}else if kind.is_dir(){"directory"}else if kind.is_file(){"file"}else{"other"}}));
        }
        entries.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
        // Directory entries are business data and may exceed the control header.
        let mut spool = tempfile::tempfile()?;
        serde_json::to_writer(&mut spool, &json!({"entries": entries}))?;
        let length = spool.metadata()?.len();
        return Ok(Message {
            meta: json!({"value": {}, "body_kind": "json_value"}),
            body: Some(Body::file(spool, 0, length)),
        });
    }
    if operation == "read_file" {
        return read_file(&path, args);
    }
    if !matches!(operation, "write_file" | "replace_text") {
        return Err(invalid("unsupported environment operation"));
    }
    let path = canonical_target(&path)?;
    static LOCKS: OnceLock<Mutex<HashMap<PathBuf, Weak<Mutex<()>>>>> = OnceLock::new();
    let lock = {
        let mut locks = LOCKS.get_or_init(Mutex::default).lock().unwrap();
        locks.retain(|_, lock| lock.strong_count() > 0);
        let lock = locks
            .get(&path)
            .and_then(Weak::upgrade)
            .unwrap_or_else(|| Arc::new(Mutex::new(())));
        locks.insert(path.clone(), Arc::downgrade(&lock));
        lock
    };
    let _guard = lock.lock().unwrap();
    if operation == "write_file" {
        let body = body.unwrap();
        if args["create_parents"] == true {
            std::fs::create_dir_all(path.parent().unwrap())?;
        }
        atomic_write(&path, body.reader(), body.len())?;
        return Ok(json!({"value":{"bytes_written":body.len()}}).into());
    }
    let original = std::fs::read_to_string(&path)?;
    let mut text = original.clone();
    let body = body.unwrap();
    let mut contents = body.reader();
    let mut counts = Vec::new();
    for replacement in replacements(args, body.len())? {
        let old = read_text(&mut contents, replacement.old_length)?;
        let new = read_text(&mut contents, replacement.new_length)?;
        let count = text.match_indices(&old).count();
        if count == 0 {
            return Err(invalid("Could not find exact old_text"));
        }
        if matches!(replacement.mode, Match::ExactlyOne) && count != 1 {
            return Err(invalid(
                "old_text is ambiguous; expected exactly one occurrence",
            ));
        }
        text = text.replace(&old, &new);
        counts.push(count);
    }
    let changed = text != original;
    if changed {
        atomic_write(&path, text.as_bytes(), text.len() as u64)?;
    }
    Ok(json!({"value":{"counts":counts,"changed":changed,"bytes_written":if changed{text.len()}else{0}}}).into())
}

fn read_text(source: &mut impl Read, length: u64) -> io::Result<String> {
    let mut text = String::new();
    if source.by_ref().take(length).read_to_string(&mut text)? as u64 != length {
        return Err(invalid("incomplete replacement text"));
    }
    Ok(text)
}

fn read_result(value: Value, body: Body) -> Message {
    Message {
        meta: json!({"value": value, "body_kind": "file_bytes"}),
        body: Some(body),
    }
}

fn read_file(path: &Path, args: &Value) -> io::Result<Message> {
    if !args["byte_range"].is_null() && !args["line_range"].is_null() {
        return Err(invalid("byte_range and line_range are mutually exclusive"));
    }
    let mut file = std::fs::File::open(path)?;
    let size = file.metadata()?.len();
    if args["binary_fallback"] == true {
        let mut prefix = [0_u8; 12];
        let n = file.read(&mut prefix)?;
        file.seek(SeekFrom::Start(0))?;
        let p = &prefix[..n];
        if p.starts_with(b"\x89PNG")
            || p.starts_with(b"\xff\xd8\xff")
            || p.starts_with(b"GIF8")
            || p.starts_with(b"BM")
            || (p.starts_with(b"RIFF") && p.get(8..12) == Some(b"WEBP"))
        {
            return Ok(read_result(
                json!({"representation":"binary"}),
                Body::file(file, 0, size),
            ));
        }
    }
    if !args["line_range"].is_null() {
        let max_bytes = args["max_bytes"]
            .as_u64()
            .unwrap_or(MAX_LINE_BYTES)
            .min(MAX_LINE_BYTES) as usize;
        let range = &args["line_range"];
        let start = range["offset"]
            .as_u64()
            .filter(|n| *n > 0)
            .ok_or_else(|| invalid("line offset must be a positive integer"))?;
        let count = range["limit"]
            .as_u64()
            .ok_or_else(|| invalid("line limit must be a nonnegative integer"))?;
        let mut reader = io::BufReader::new(file);
        let mut line = 1_u64;
        let mut bytes = Vec::new();
        let mut selected = 0_u64;
        let mut reached_eof = false;
        let mut completed_bytes = 0;
        let mut truncated = false;
        loop {
            if line >= start && selected >= count {
                break;
            }
            let chunk = reader.fill_buf()?;
            if chunk.is_empty() {
                reached_eof = true;
                if line < start {
                    return Err(invalid("Offset is beyond end of file"));
                }
                if line >= start && selected < count {
                    selected += 1; // Empty files and trailing newlines have a final empty line.
                }
                break;
            }
            let length = chunk
                .iter()
                .position(|b| *b == b'\n')
                .map_or(chunk.len(), |i| i + 1);
            let ended_with_newline = chunk[length - 1] == b'\n';
            if line >= start {
                if bytes.len().saturating_add(length) > max_bytes {
                    bytes.truncate(completed_bytes);
                    truncated = true;
                    break;
                }
                bytes.extend_from_slice(&chunk[..length]);
            }
            reader.consume(length);
            if ended_with_newline {
                if line >= start {
                    selected += 1;
                    completed_bytes = bytes.len();
                }
                line += 1;
            }
        }
        let has_more = !reached_eof;
        if bytes.last() == Some(&b'\n') && (!reached_eof || truncated) {
            bytes.pop();
        }
        std::str::from_utf8(&bytes).map_err(|_| invalid("file is not valid UTF-8"))?;
        return Ok(read_result(
            json!({"actual_range":{"offset":start,"limit":selected},"has_more":has_more,"truncated":truncated}),
            Body::bytes(bytes),
        ));
    }
    let range = &args["byte_range"];
    if !range.is_null() {
        for key in ["offset", "limit"] {
            if !range[key].is_null() && range[key].as_u64().is_none() {
                return Err(invalid("byte range values must be nonnegative integers"));
            }
        }
    }
    let offset = range["offset"].as_u64().unwrap_or(0);
    let available = size.saturating_sub(offset);
    let requested = range["limit"].as_u64();
    // An explicit range can read pseudo-files that report a zero length.
    if size == 0
        && let Some(limit) = requested.filter(|limit| *limit > 0)
    {
        file.seek(SeekFrom::Start(offset))?;
        let mut spool = tempfile::tempfile()?;
        let length = io::copy(&mut file.take(limit), &mut spool)?;
        return Ok(read_result(
            json!({"actual_range":{"offset":offset,"limit":length},"has_more":false,"truncated":false}),
            Body::file(spool, 0, length),
        ));
    }
    let length = requested.unwrap_or(available).min(available);
    Ok(read_result(
        json!({"actual_range":{"offset":offset,"limit":length},"has_more":length<available,"truncated":false}),
        Body::file(file, offset, length),
    ))
}
