use ascout::{
    control_plane::gateway::runtime::ControlServer,
    protocol::contract::{AgentLaunch, Operation},
    runtime::{
        BaseRuntime,
        adapter::Adapter,
        backend::{Backend, BackendContext},
        backends::filesystem::{FilesystemBackend, FilesystemConfig},
        execution_wire::{self as wire, Message},
    },
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{BufReader, Read, Write},
    os::unix::net::UnixStream,
    sync::Arc,
    time::Duration,
};

#[test]
fn unified_filesystem_endpoint_binds_relative_operations_to_target_workspace() {
    let root = tempfile::Builder::new()
        .prefix("relative-endpoint-")
        .tempdir_in("/tmp/opencode")
        .unwrap();
    let workspace = root.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let mut config = FilesystemConfig::new(root.path().join("state"));
    config.force_overlay = true;
    config.allow_overlapping_state_directory = true;
    config.isolated_paths = Some(vec![workspace.clone()]);
    config.isolated_targets = vec![(workspace.clone(), "/app".into())];
    let plane = Arc::new(FilesystemBackend::new(config).unwrap());
    let runtime = Arc::new(
        BaseRuntime::new("runtime", plane.clone()).with_bubblewrap(plane.bubblewrap_path()),
    );
    let context = BackendContext::new("relative");
    runtime.establish_with_context(&context).unwrap();
    runtime
        .launch_agent(
            &context,
            &AgentLaunch {
                command: "/bin/sleep".into(),
                args: vec!["30".into()],
                cwd: workspace.clone(),
                log: workspace.join("agent.log"),
                instance_id: Some("host".into()),
                env: BTreeMap::new(),
            },
        )
        .unwrap();
    let server = ControlServer::bind("127.0.0.1:0").unwrap();
    let address = server.local_addr().unwrap().to_string();
    let worker = std::thread::spawn(move || {
        Adapter {
            adapter_instance: "relative".into(),
            runtime,
        }
        .run(&address)
        .unwrap()
    });
    let session = server.accept().unwrap();
    session.wait_for_hello(Duration::from_secs(5)).unwrap();
    let bound = session
        .call(
            "relative",
            Operation::BindExecution {
                branch_id: "relative".into(),
            },
        )
        .unwrap();
    let access = bound.execution_access.unwrap();
    let request = |id: u64, operation: &str, arguments: Value| json!({"command":"execute","request_id":format!("req-{id}"),"operation_id":format!("op-{id}"),"branch_id":"relative","environment_id":"relative","operation":operation,"arguments":arguments});
    let terminal = |socket: UnixStream| {
        let mut reader = BufReader::new(socket);
        loop {
            let event = wire::read(&mut reader, false).unwrap();
            if matches!(
                event.meta["event"].as_str(),
                Some("terminal" | "rejected" | "confirmed")
            ) {
                return event;
            }
            assert_eq!(event.meta["event"], "accepted");
        }
    };
    let call = |request: &Value, bytes: Option<&[u8]>| {
        let mut socket = UnixStream::connect(&access.address).unwrap();
        match bytes {
            Some(bytes) => {
                wire::write_body(&mut socket, request, bytes.len() as u64, bytes).unwrap()
            }
            None => wire::write_header(&mut socket, request).unwrap(),
        }
        terminal(socket)
    };
    fn bytes(event: Message) -> Vec<u8> {
        assert_eq!(event.meta["status"], "succeeded", "{}", event.meta);
        let mut bytes = Vec::new();
        event
            .body
            .unwrap()
            .reader()
            .read_to_end(&mut bytes)
            .unwrap();
        bytes
    }
    let path = json!({"path":"relative-file"});
    let write = request(1, "write_file", path.clone());
    let read = |id| request(id, "read_file", path.clone());
    let original = vec![255; 33 * 1024 * 1024 + 17];
    assert_eq!(
        call(&write, Some(&original)).meta["value"]["bytes_written"],
        original.len()
    );
    assert_eq!(bytes(call(&read(2), None)), original);

    // Each case reuses an identity from a malformed request. Only a complete
    // body may be admitted and no partially transmitted write may replace it.
    let mut socket = UnixStream::connect(&access.address).unwrap();
    let mut damaged = request(3, "write_file", path.clone());
    damaged["body_length"] = 5.into();
    wire::write_header(&mut socket, &damaged).unwrap();
    for part in [&[0, 0][..], &[0, 5], b"ab"] {
        socket.write_all(part).unwrap();
    }
    socket.shutdown(std::net::Shutdown::Write).unwrap();
    assert_eq!(terminal(socket).meta["event"], "rejected");
    assert_eq!(bytes(call(&read(4), None)), original);

    // Exercise short reads of frame headers and body, while verifying a
    // retry is recognized by content rather than by frame boundaries.
    let mut socket = UnixStream::connect(&access.address).unwrap();
    damaged["body_length"] = 2.into();
    let mut encoded = Vec::new();
    wire::write_header(&mut encoded, &damaged).unwrap();
    encoded.extend_from_slice(b"\0\0\0\x01a\0\0\0\x01b\0\0\0\0");
    for byte in encoded {
        socket.write_all(&[byte]).unwrap();
    }
    assert_eq!(terminal(socket).meta["value"]["bytes_written"], 2);
    let replaced = call(
        &request(
            5,
            "replace_text",
            json!({"path":"relative-file","replacements":[{"old_length":2,"new_length":2}]}),
        ),
        Some(b"abcd"),
    );
    assert_eq!(replaced.meta["value"]["counts"], json!([1]));
    let replay = request(3, "write_file", path.clone());
    assert_eq!(call(&replay, Some(b"ab")).meta["value"]["bytes_written"], 2);
    let conflict = call(&replay, Some(b"ef"));
    assert_eq!(conflict.meta["event"], "rejected");
    assert!(
        conflict.meta["reason"]
            .as_str()
            .unwrap()
            .contains("different content")
    );
    assert_eq!(bytes(call(&read(6), None)), b"cd");
    let confirmation = json!({"command":"confirm","operation_id":"op-3","branch_id":"relative","environment_id":"relative"});
    assert_eq!(call(&confirmation, None).meta["event"], "confirmed");
    assert_eq!(call(&replay, Some(b"ab")).meta["event"], "rejected");

    let listing = bytes(call(
        &request(7, "list_directory", json!({"path":"."})),
        None,
    ));
    let listing: Value = serde_json::from_slice(&listing).unwrap();
    assert!(
        listing["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["name"] == "relative-file")
    );
    for (id, args) in [
        (
            8,
            json!({"command":"test \"$PWD\" = /app && test \"$(cat relative-file)\" = cd"}),
        ),
        (9, json!({"cwd":"/tmp","command":"test \"$PWD\" = /tmp"})),
    ] {
        assert_eq!(call(&request(id, "exec", args), None).meta["exit_code"], 0);
    }
    let exec = request(
        10,
        "exec",
        json!({"command": "printf '\\377\\000'; sleep 30"}),
    );
    let mut socket = UnixStream::connect(&access.address).unwrap();
    wire::write_header(&mut socket, &exec).unwrap();
    let mut execution = BufReader::new(socket);
    assert_eq!(
        wire::read(&mut execution, false).unwrap().meta["event"],
        "accepted"
    );
    let output = wire::read(&mut execution, false).unwrap();
    assert_eq!(output.meta["event"], "output");
    assert_eq!(output.meta["value"]["stream"], "stdout");
    let mut chunk = Vec::new();
    output
        .body
        .unwrap()
        .reader()
        .read_to_end(&mut chunk)
        .unwrap();
    assert_eq!(chunk, [255, 0]);
    let cancel = json!({"command":"cancel","operation_id":"op-10","branch_id":"relative","environment_id":"relative"});
    assert_eq!(call(&cancel, None).meta["status"], "cancelled");
    assert_eq!(
        wire::read(&mut execution, false).unwrap().meta["status"],
        "cancelled"
    );
    // A reconnect receives the same output before the cached terminal.
    let mut socket = UnixStream::connect(&access.address).unwrap();
    wire::write_header(&mut socket, &exec).unwrap();
    let mut retry = BufReader::new(socket);
    assert_eq!(
        wire::read(&mut retry, false).unwrap().meta["event"],
        "accepted"
    );
    let output = wire::read(&mut retry, false).unwrap();
    let mut replay = Vec::new();
    output
        .body
        .unwrap()
        .reader()
        .read_to_end(&mut replay)
        .unwrap();
    assert_eq!(replay, chunk);
    assert_eq!(
        wire::read(&mut retry, false).unwrap().meta["status"],
        "cancelled"
    );
    let mut confirmation = cancel;
    confirmation["command"] = "confirm".into();
    assert_eq!(call(&confirmation, None).meta["event"], "confirmed");
    assert_eq!(call(&exec, None).meta["event"], "rejected");

    assert!(!workspace.join("relative-file").exists());
    session.shutdown().unwrap();
    worker.join().unwrap();
}
