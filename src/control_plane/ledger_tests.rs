use super::*;

fn branch() -> BranchRecord {
    serde_json::from_value(
        serde_json::json!({"branch_id":"branch-test", "role":"candidate", "status":"discarded"}),
    )
    .unwrap()
}

#[test]
fn archive_retries_once_and_repairs_partial_line() {
    for partial in [0, 19] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        let mut ledger = Ledger::open(&path).unwrap();
        ledger
            .record(serde_json::json!({"message":"prior"}))
            .unwrap();
        ledger.write_faults.push_back(partial);
        let log =
            super::super::system_log::SystemLog::open(&dir.path().join("system.log")).unwrap();
        ledger
            .archive_branch(branch(), "session-test", &log)
            .unwrap();
        assert!(log.warnings().is_empty());
        let lines = std::fs::read_to_string(path).unwrap();
        let values: Vec<Value> = lines
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(values.len(), 2);
        assert_eq!(values[1]["message"], "branch_archived");
        assert!(ledger.write_faults.is_empty());
        assert_eq!(ledger.reader().sequence(), 0);
    }
}

#[test]
fn final_archive_failure_warns_projection_and_releases_record() {
    let dir = tempfile::tempdir().unwrap();
    let mut ledger = Ledger::open(&dir.path().join("events.jsonl")).unwrap();
    // Also exercise the existing stderr fallback when the system log cannot write.
    let log = super::super::system_log::SystemLog::open(Path::new("/dev/full")).unwrap();
    let projection =
        super::super::ui_projection::UiProjection::new(ledger.reader()).with_log(log.clone());
    ledger.write_faults.extend([17, 11, 0]);
    let error = ledger
        .archive_branch(branch(), "session-test", &log)
        .unwrap_err()
        .to_string();
    assert!(error.contains("retry failed"));
    assert_eq!(
        ledger.write_faults.len(),
        1,
        "exactly two writes, no infinite retry"
    );
    let warnings = projection.warnings();
    assert_eq!(warnings.len(), 1);
    for identity in [
        "session=session-test",
        "branch_id=branch-test",
        "injected append failure",
    ] {
        assert!(warnings[0].contains(identity));
    }
    assert_eq!(projection.sequence(), 0);
    assert!(projection.state_event_set().events.is_empty());
}

#[test]
fn archive_recovers_a_previous_failed_append_before_retry() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("events.jsonl");
    let mut ledger = Ledger::open(&path).unwrap();
    let log = super::super::system_log::SystemLog::open(&dir.path().join("system.log")).unwrap();
    ledger.write_faults.push_back(13);
    assert!(ledger
        .record(serde_json::json!({"message":"ordinary"}))
        .is_err());
    ledger
        .archive_branch(branch(), "session-test", &log)
        .unwrap();
    let saved = std::fs::read_to_string(path).unwrap();
    assert_eq!(saved.lines().count(), 1);
    assert_eq!(
        serde_json::from_str::<Value>(saved.trim()).unwrap()["message"],
        "branch_archived"
    );
    assert!(log.warnings().is_empty());
}

#[test]
fn unsafe_boundary_reports_recovery_and_retry_unavailable() {
    let mut ledger = Ledger::open(Path::new("/dev/full")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let log = super::super::system_log::SystemLog::open(&dir.path().join("system.log")).unwrap();
    let error = ledger
        .archive_branch(branch(), "session-test", &log)
        .unwrap_err()
        .to_string();
    assert!(error.contains("recovery failed"));
    assert!(error.contains("retry could not be completed"));
    assert!(log.warnings()[0].contains("branch_id=branch-test"));
}

#[test]
fn archive_warning_http_frame_and_actual_log_line() {
    use std::io::{Read, Write};
    use super::super::gateway::ui::{self, Assets, Connection, Presentation};
    #[derive(Clone)]
    struct View;
    impl Presentation for View {
        fn input_status(&self) -> Value { serde_json::json!({}) }
        fn conversation(&self, _: &str) -> Option<Value> { None }
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("events.jsonl");
    let log_path = dir.path().join("system.log");
    let mut ledger = Ledger::open(&path).unwrap();
    let log = super::super::system_log::SystemLog::open(&log_path).unwrap();
    let projection = super::super::ui_projection::UiProjection::new(ledger.reader()).with_log(log.clone());
    ledger.write_faults.extend([17, 11]);
    assert!(ledger.archive_branch(branch(), "session-test", &log).is_err());
    let line = std::fs::read_to_string(log_path).unwrap();
    assert_eq!(line.lines().count(), 1);
    let stamp = line.split_whitespace().next().unwrap();
    assert_eq!(stamp.len(), 24);
    assert_eq!(&stamp[19..20], ".");
    assert!(stamp.ends_with('Z'));
    for identity in ["WARN ledger", "session=session-test", "branch_id=branch-test", "injected append failure"] {
        assert!(line.contains(identity));
    }
    eprintln!("ACTUAL_SYSTEM_LOG {}", line.trim_end());
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let assets = Assets {index_html:b"", app_js:b"", app_css:b"", favicon_svg:b""};
    let mut server = ui::start(&address.to_string(), View, Connection::new(projection.clone(), |_| Ok(())), projection.clone(), assets).unwrap();
    let mut socket = std::net::TcpStream::connect(address).unwrap();
    socket.set_read_timeout(Some(std::time::Duration::from_secs(3))).unwrap();
    write!(socket, "GET /api/observation/events HTTP/1.1\r\nHost: {address}\r\n\r\n").unwrap();
    let mut bytes = Vec::new();
    loop {
        let mut byte = [0];
        assert_eq!(socket.read(&mut byte).unwrap(), 1);
        bytes.push(byte[0]);
        if bytes.windows(4).any(|x| x == b"\r\n\r\n") && bytes.ends_with(b"\n\n") { break; }
    }
    let response = String::from_utf8(bytes).unwrap();
    let frame = response.split_once("\r\n\r\n").unwrap().1;
    assert!(frame.starts_with("event: system-warning\n"));
    assert!(!frame.contains("id:"));
    let data = frame.lines().find_map(|x| x.strip_prefix("data: ")).unwrap();
    assert_eq!(serde_json::from_str::<String>(data).unwrap(), line.trim_end());
    assert_eq!(projection.sequence(), 0);
    assert!(projection.state_event_set().events.is_empty());
    let saved = std::fs::read_to_string(path).unwrap();
    assert!(!saved.contains("system-warning"));
    assert!(!saved.contains("history incomplete"));
    eprintln!("ACTUAL_WARNING_FRAME {frame:?}");
    drop(socket);
    server.stop().unwrap();
}
