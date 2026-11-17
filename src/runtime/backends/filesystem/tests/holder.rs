use super::super::*;
use crate::protocol::contract::AgentLaunch;
use crate::runtime::process::ProcessBackend;
use serde_json::json;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn request(socket: &Path, value: serde_json::Value) -> serde_json::Value {
    let mut stream = UnixStream::connect(socket).unwrap();
    stream
        .write_all((value.to_string() + "\n").as_bytes())
        .unwrap();
    let mut reader = BufReader::new(stream);
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap() == 0 {
            panic!("holder closed without a result");
        }
        let event: serde_json::Value = serde_json::from_str(&line).unwrap();
        if event.get("exit_code").is_some() || event.get("error").is_some() {
            return event;
        }
    }
}

fn success(socket: &Path, command: &str) {
    let result = request(socket, json!({"op":"exec","command":command,"cwd":"/app"}));
    assert_eq!(result["exit_code"], 0, "{command}: {result}");
}

fn launch(backend: &FilesystemBackend, environment: &str, workspace: &Path) -> ProcessBackend {
    let launch = AgentLaunch {
        command: "/bin/true".into(),
        args: Vec::new(),
        cwd: workspace.to_path_buf(),
        log: workspace.join("holder-test.log"),
        instance_id: None,
        env: BTreeMap::new(),
    };
    let context = BackendContext {
        environment_id: environment.to_string(),
        ..BackendContext::default()
    };
    let process = ProcessBackend::new("process", backend.bubblewrap_path());
    let fragment = backend.launch_fragment(&context, &launch).unwrap();
    process.launch_holder(&context, &fragment).unwrap();
    process
}

#[test]
fn namespace_holder_runs_concurrent_commands_in_each_branch_mount_namespace() {
    let root = tempfile::Builder::new()
        .prefix("namespace-holder-")
        .tempdir_in("/tmp/opencode")
        .unwrap();
    let workspace = root.path().join("workspace");
    let probe = root.path().join("probe");
    fs::create_dir(&workspace).unwrap();
    fs::create_dir(&probe).unwrap();
    fs::write(workspace.join("base.txt"), "base").unwrap();
    let mut config = FilesystemConfig::new(root.path().join("state"));
    config.force_overlay = true;
    config.allow_overlapping_state_directory = true;
    config.isolated_paths = Some(vec![workspace.clone()]);
    config.isolated_targets = vec![(workspace.clone(), PathBuf::from("/app"))];
    config.passthrough_rw = vec![probe.clone()];
    let backend = Arc::new(FilesystemBackend::new(config).unwrap());
    assert!(backend.usable, "bubblewrap overlay probe failed");
    backend.establish("authority").unwrap();
    let authority = launch(&backend, "authority", &workspace);
    let socket = |environment: &str| {
        root.path()
            .join("state/holder-sockets")
            .join(environment)
            .join("cmd.sock")
    };

    success(&socket("authority"), "printf parent > /app/from-parent");
    assert!(!workspace.join("from-parent").exists());
    backend
        .fork("authority", "candidate", &[PathScope::Workspace])
        .unwrap();
    let candidate = launch(&backend, "candidate", &workspace);

    success(
        &socket("candidate"),
        "test \"$(cat /app/from-parent)\" = parent",
    );
    success(&socket("authority"), "printf later > /app/later");
    success(&socket("candidate"), "test ! -e /app/later");

    let input = request(
        &socket("candidate"),
        json!({
            "op":"exec",
            "command":"cat > /app/input.txt",
            "cwd":"/app",
            "input":"aGVsbG8="
        }),
    );
    assert_eq!(input["exit_code"], 0);
    success(
        &socket("candidate"),
        "test \"$(cat /app/input.txt)\" = hello",
    );

    let start = Instant::now();
    let slow = std::thread::spawn({
        let socket = socket("candidate");
        move || {
            let result = request(
                &socket,
                json!({"op":"exec","command":"sleep 0.35; echo slow","cwd":"/app"}),
            );
            assert_eq!(result["exit_code"], 0);
            Instant::now() - start
        }
    });
    success(&socket("candidate"), "echo fast");
    let fast_at = start.elapsed();
    let slow_at = slow.join().unwrap();
    assert!(
        fast_at < slow_at && slow_at - fast_at > Duration::from_millis(200),
        "independent connections must not serialize: fast={fast_at:?} slow={slow_at:?}"
    );

    let pid_file = probe.join("background.pid");
    success(
        &socket("candidate"),
        &format!("sleep 10 & echo $! > '{}'", pid_file.display()),
    );
    let pid: i32 = fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    success(&socket("candidate"), &format!("kill -0 {pid}"));

    let drained=request(&socket("candidate"),json!({"op":"drain","cancel":true}));
    assert_eq!(drained["exit_code"],0,"{drained}");
    let alive=request(&socket("candidate"),json!({"op":"exec","command":format!("test -r /proc/{pid}/stat && test \"$(awk '{{print $3}}' /proc/{pid}/stat)\" != Z"),"cwd":"/app"}));
    assert_ne!(alive["exit_code"],0,"completed command's background writer must stop before drain confirms");
    success(&socket("candidate"),"echo holder remains available");

    candidate.terminate("candidate").unwrap();
    candidate.stop("candidate").unwrap();
    candidate.cleanup("candidate").unwrap();
    authority.terminate("authority").unwrap();
    authority.stop("authority").unwrap();
    authority.cleanup("authority").unwrap();
    assert!(
        unsafe { libc::kill(pid, 0) } != 0,
        "environment cleanup must end background work with the holder"
    );
    backend.terminate("candidate").unwrap();
    backend.cleanup("candidate").unwrap();
    backend.terminate("authority").unwrap();
    backend.cleanup("authority").unwrap();
    assert!(!workspace.join("from-parent").exists());
}
