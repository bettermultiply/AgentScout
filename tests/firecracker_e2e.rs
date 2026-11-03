//! 直接覆盖真实 Firecracker 的端到端测试。
//!
//! 这些测试不使用 mock 或替代后端。每个测试只操作 `rootfs.ext4` 的独立副本，
//! 并允许在相应功能尚未实现时保持失败。

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ascout::contract::{
    EnvironmentState, EventPayload, Operation, OperationResult, PROTOCOL_VERSION, ResultStatus,
};
use ascout::control::{ControlServer, ControlSession};
use serde_json::{Value, json};

static FIRECRACKER_TEST: Mutex<()> = Mutex::new(());

const ADAPTER_INSTANCE: &str = "firecracker-e2e";

fn serial_test() -> std::sync::MutexGuard<'static, ()> {
    FIRECRACKER_TEST
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct Assets {
    firecracker: PathBuf,
    kernel: PathBuf,
    rootfs: PathBuf,
}

impl Assets {
    fn locate() -> Self {
        let directory = workspace().join("assets/firecracker");
        let assets = Self {
            firecracker: directory.join("firecracker"),
            kernel: directory.join("vmlinux-6.18.48"),
            rootfs: directory.join("rootfs.ext4"),
        };
        assert_file(&assets.firecracker);
        assert_file(&assets.kernel);
        assert_file(&assets.rootfs);
        assert!(Path::new("/dev/kvm").exists(), "/dev/kvm is unavailable");
        assets
    }
}

struct AdapterHarness {
    session: ControlSession,
    adapter: Child,
    directory: PathBuf,
    state_directory: PathBuf,
    firecracker: PathBuf,
}

impl AdapterHarness {
    fn start(test_name: &str) -> Self {
        let assets = Assets::locate();
        let adapter_binary = built_binary("ascout-runtime-adapter");
        let guest_access_binary = built_binary("ascout-agent-access");
        assert_file(&adapter_binary);
        assert_file(&guest_access_binary);

        let directory = unique_directory(test_name);
        fs::create_dir_all(&directory).unwrap();
        let control_socket = directory.join("control.sock");
        let state_nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let state_directory = workspace()
            .join("target/afc")
            .join(format!("{}-{state_nonce}", std::process::id()));
        fs::create_dir_all(&state_directory).unwrap();
        let config_path = directory.join("adapter.json");
        fs::write(
            &config_path,
            serde_json::to_vec_pretty(&json!({
                "control_socket": control_socket,
                "adapter_instance": ADAPTER_INSTANCE,
                "firecracker_path": assets.firecracker,
                "kernel_path": assets.kernel,
                "rootfs_path": assets.rootfs,
                "guest_access_path": guest_access_binary,
                "state_directory": state_directory,
            }))
            .unwrap(),
        )
        .unwrap();

        let server = ControlServer::bind(&control_socket).unwrap();
        let stdout = File::create(directory.join("adapter.stdout.log")).unwrap();
        let stderr = File::create(directory.join("adapter.stderr.log")).unwrap();
        let adapter = Command::new(&adapter_binary)
            .arg("--config")
            .arg(&config_path)
            .stdout(stdout)
            .stderr(stderr)
            .spawn()
            .unwrap();

        let (sender, receiver) = std::sync::mpsc::channel();
        thread::spawn(move || {
            let _ = sender.send(server.accept());
        });
        let session = receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("Runtime Adapter did not connect to the control socket")
            .unwrap();
        let hello = session
            .wait_for_hello(Duration::from_secs(5))
            .expect("Runtime Adapter did not send hello");
        assert_eq!(hello.protocol_version, PROTOCOL_VERSION);
        assert_eq!(hello.adapter_instance, ADAPTER_INSTANCE);

        Self {
            session,
            adapter,
            directory,
            state_directory,
            firecracker: Assets::locate().firecracker,
        }
    }

    fn firecracker_process(&self, environment_id: &str) -> ProcessInfo {
        wait_until(Duration::from_secs(5), || {
            let processes = matching_firecracker_processes(
                &self.firecracker,
                environment_id,
                &self.state_directory,
            );
            (processes.len() == 1).then(|| processes[0].clone())
        })
        .unwrap_or_else(|| panic!("no unique Firecracker process for {environment_id}"))
    }

    fn assert_no_firecracker_process(&self, environment_id: &str) {
        assert!(
            wait_until(Duration::from_secs(5), || {
                matching_firecracker_processes(
                    &self.firecracker,
                    environment_id,
                    &self.state_directory,
                )
                .is_empty()
                .then_some(())
            })
            .is_some(),
            "Firecracker process for {environment_id} still exists"
        );
    }

    fn terminate_and_cleanup(&self, environment_id: &str) {
        let _ = self
            .session
            .call(environment_id, Operation::TerminateEnvironment);
        let _ = self
            .session
            .call(environment_id, Operation::CleanupEnvironment);
    }
}

impl Drop for AdapterHarness {
    fn drop(&mut self) {
        // 测试失败时 cleanup 可能没有执行：先回收残留进程，再清理网络和目录。
        let environments = fs::read_dir(&self.state_directory)
            .map(|entries| {
                entries
                    .flatten()
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let state_marker = self.state_directory.to_string_lossy().into_owned();
        for process in all_processes() {
            let command_line = process.arguments.join(" ");
            if command_line.contains("firecracker") && command_line.contains(&state_marker) {
                kill(process.pid);
            }
        }
        for environment in &environments {
            let namespace = ascout::firecracker::environment_network_namespace(environment);
            let _ = Command::new("sudo")
                .args(["-n", "ip", "netns", "delete", &namespace])
                .status();
        }
        cleanup_stale_host_rules();
        let _ = self.adapter.kill();
        let _ = self.adapter.wait();
        let _ = fs::remove_dir_all(&self.state_directory);
        let _ = fs::remove_dir_all(&self.directory);
    }
}

/// 删除测试网络遗留的宿主 iptables 规则（Adapter 异常退出时可能未回收）。
fn cleanup_stale_host_rules() {
    for (table, chain) in [("filter", "FORWARD"), ("nat", "POSTROUTING")] {
        let output = Command::new("sudo")
            .args(["-n", "iptables", "-t", table, "-S", chain])
            .output();
        let Ok(output) = output else {
            continue;
        };
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            if !line.starts_with("-A ") || !line.contains("10.213.") {
                continue;
            }
            let specification = line[3..].to_string();
            let mut arguments = vec!["-n", "iptables", "-t", table, "-D"];
            arguments.extend(specification.split_whitespace());
            let _ = Command::new("sudo").args(&arguments).status();
        }
    }
}

#[derive(Clone, Debug)]
struct ProcessInfo {
    pid: u32,
    arguments: Vec<String>,
}

#[test]
fn adapter_connects_and_establishes_a_real_microvm() {
    let _serial = serial_test();
    let harness = AdapterHarness::start("connect-establish");

    let result = harness
        .session
        .call("environment-1", Operation::EstablishEnvironment)
        .unwrap();
    assert_completed_in_state(&result, EnvironmentState::Available);
    harness.firecracker_process("environment-1");

    harness.terminate_and_cleanup("environment-1");
}

#[test]
fn real_microvm_completes_the_full_lifecycle_over_vsock() {
    let _serial = serial_test();
    let harness = AdapterHarness::start("lifecycle");
    let environment = "environment-1";

    assert_completed_in_state(
        &harness
            .session
            .call(environment, Operation::EstablishEnvironment)
            .unwrap(),
        EnvironmentState::Available,
    );
    assert_command(
        &harness.session,
        environment,
        &["sh", "-c", "printf inherited > /root/ascout-state"],
        0,
        "",
    );

    let suspended = harness
        .session
        .call(environment, Operation::SuspendEnvironment)
        .unwrap();
    assert_completed_in_state(&suspended, EnvironmentState::Suspended);
    assert_eq!(
        firecracker_vm_state(&harness.firecracker_process(environment)),
        "Paused"
    );

    let resumed = harness
        .session
        .call(environment, Operation::ResumeEnvironment)
        .unwrap();
    assert_completed_in_state(&resumed, EnvironmentState::Available);
    assert_eq!(
        firecracker_vm_state(&harness.firecracker_process(environment)),
        "Running"
    );
    assert_command(
        &harness.session,
        environment,
        &["cat", "/root/ascout-state"],
        0,
        "inherited",
    );

    let terminated = harness
        .session
        .call(environment, Operation::TerminateEnvironment)
        .unwrap();
    assert_completed_in_state(&terminated, EnvironmentState::Terminated);
    harness.assert_no_firecracker_process(environment);

    let cleaned = harness
        .session
        .call(environment, Operation::CleanupEnvironment)
        .unwrap();
    assert_completed_in_state(&cleaned, EnvironmentState::Cleaned);
    assert_state_directory_has_no_environment(&harness.state_directory, environment);
}

#[test]
fn retrying_an_establish_request_does_not_create_a_second_microvm() {
    let _serial = serial_test();
    let harness = AdapterHarness::start("retry");
    let environment = "environment-1";
    let request_id = "establish-retry";

    let result = harness
        .session
        .call_with_id(request_id, environment, Operation::EstablishEnvironment)
        .unwrap();
    assert_completed_in_state(&result, EnvironmentState::Available);
    harness
        .session
        .send(request_id, environment, Operation::EstablishEnvironment)
        .unwrap();
    thread::sleep(Duration::from_millis(300));

    let processes =
        matching_firecracker_processes(&harness.firecracker, environment, &harness.state_directory);
    assert_eq!(
        processes.len(),
        1,
        "retry created another Firecracker process"
    );
    harness.terminate_and_cleanup(environment);
}

#[test]
fn suspend_interrupts_a_real_in_flight_guest_operation() {
    let _serial = serial_test();
    let harness = AdapterHarness::start("interrupt");
    let environment = "environment-1";
    harness
        .session
        .call(environment, Operation::EstablishEnvironment)
        .unwrap();

    let interrupted = thread::scope(|scope| {
        let operation = scope.spawn(|| {
            harness
                .session
                .call(
                    environment,
                    Operation::OperateEnvironment {
                        command: arguments(&["sh", "-c", "sleep 30 & wait"]),
                        agent: None,
                    },
                )
                .unwrap()
        });
        thread::sleep(Duration::from_millis(500));
        let suspended = harness
            .session
            .call(environment, Operation::SuspendEnvironment)
            .unwrap();
        assert_completed_in_state(&suspended, EnvironmentState::Suspended);
        assert_eq!(
            firecracker_vm_state(&harness.firecracker_process(environment)),
            "Paused"
        );
        operation.join().unwrap()
    });

    assert!(
        matches!(
            interrupted.status,
            ResultStatus::Failed | ResultStatus::PartiallyCompleted
        ),
        "interrupted operation was reported as {:?}",
        interrupted.status
    );
    assert!(interrupted.reason.is_some());

    assert_completed_in_state(
        &harness
            .session
            .call(environment, Operation::ResumeEnvironment)
            .unwrap(),
        EnvironmentState::Available,
    );
    assert_command(
        &harness.session,
        environment,
        &["sh", "-c", "! pgrep -x sleep"],
        0,
        "",
    );

    harness.terminate_and_cleanup(environment);
}

#[test]
fn killing_firecracker_emits_an_environment_state_event() {
    let _serial = serial_test();
    let harness = AdapterHarness::start("unexpected-stop");
    let environment = "environment-1";
    harness
        .session
        .call(environment, Operation::EstablishEnvironment)
        .unwrap();

    kill(harness.firecracker_process(environment).pid);
    let event = harness
        .session
        .wait_event(Duration::from_secs(5))
        .expect("no state event after killing Firecracker");
    assert_eq!(event.environment_id, environment);
    match event.payload {
        EventPayload::EnvironmentStateChanged {
            previous, current, ..
        } => {
            assert_eq!(previous, Some(EnvironmentState::Available));
            assert_eq!(current, EnvironmentState::Terminated);
        }
    }

    let cleaned = harness
        .session
        .call(environment, Operation::CleanupEnvironment)
        .unwrap();
    assert_completed_in_state(&cleaned, EnvironmentState::Cleaned);
}

#[test]
fn branch_restores_real_siblings_with_inherited_process_and_disk_state() {
    let _serial = serial_test();
    let harness = AdapterHarness::start("branch");
    let parent = "parent";
    let child_a = "child-a";
    let child_b = "child-b";
    harness
        .session
        .call(parent, Operation::EstablishEnvironment)
        .unwrap();
    assert_command(
        &harness.session,
        parent,
        &[
            "sh",
            "-c",
            "printf inherited > /root/ascout-state; sleep 300 >/dev/null 2>&1 & echo $! > /root/ascout-marker.pid",
        ],
        0,
        "",
    );

    for child in [child_a, child_b] {
        let branch = harness
            .session
            .call(
                parent,
                Operation::BranchEnvironment {
                    child_environment_id: child.to_string(),
                },
            )
            .unwrap();
        assert_eq!(branch.status, ResultStatus::Completed, "{branch:?}");
        harness.firecracker_process(child);
        assert_command(
            &harness.session,
            child,
            &["cat", "/root/ascout-state"],
            0,
            "inherited",
        );
        assert_command(
            &harness.session,
            child,
            &["sh", "-c", "kill -0 $(cat /root/ascout-marker.pid)"],
            0,
            "",
        );
    }

    assert_command(
        &harness.session,
        child_a,
        &["sh", "-c", "printf child-a > /root/child-a-only"],
        0,
        "",
    );
    for environment in [parent, child_b] {
        assert_command(
            &harness.session,
            environment,
            &["sh", "-c", "test ! -e /root/child-a-only"],
            0,
            "",
        );
    }

    for environment in [parent, child_a, child_b] {
        harness.terminate_and_cleanup(environment);
        harness.assert_no_firecracker_process(environment);
    }
}

#[test]
fn ssh_diagnostic_reaches_a_real_microvm() {
    let _serial = serial_test();
    let harness = AdapterHarness::start("ssh-diagnostic");
    let environment = "environment-1";
    assert_completed_in_state(
        &harness
            .session
            .call(environment, Operation::EstablishEnvironment)
            .unwrap(),
        EnvironmentState::Available,
    );

    let private_key = harness.directory.join("ubuntu-24.04.id_rsa");
    fs::copy(
        workspace().join("assets/firecracker/ubuntu-24.04.id_rsa"),
        &private_key,
    )
    .unwrap();
    let mut permissions = fs::metadata(&private_key).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o600);
    fs::set_permissions(&private_key, permissions).unwrap();

    let namespace = ascout::firecracker::environment_network_namespace(environment);
    let output = wait_until(Duration::from_secs(30), || {
        ssh_into_network(&namespace, &private_key, "printf ascout-ssh").ok()
    })
    .expect("SSH did not reach the Firecracker guest");
    assert_eq!(output, "ascout-ssh");

    harness.terminate_and_cleanup(environment);
}

#[test]
fn codex_starts_and_reaches_the_model_service() {
    let _serial = serial_test();
    let harness = AdapterHarness::start("codex");
    let environment = "environment-1";
    assert_completed_in_state(
        &harness
            .session
            .call(environment, Operation::EstablishEnvironment)
            .unwrap(),
        EnvironmentState::Available,
    );

    let request_id = "codex-hi";
    harness
        .session
        .send(
            request_id,
            environment,
            Operation::OperateEnvironment {
                command: arguments(&[
                    "sh",
                    "-c",
                    "cd /root && export HOME=/root && export PATH=$(dirname $(readlink /usr/local/bin/codex)):$PATH && timeout 120 codex exec --skip-git-repo-check \"hi\" </dev/null",
                ]),
                agent: None,
            },
        )
        .unwrap();
    let result = harness
        .session
        .wait_result(request_id, Duration::from_secs(180))
        .expect("codex operation did not finish in time");
    assert_eq!(result.status, ResultStatus::Completed, "{result:?}");
    let output = result.output.expect("codex result has no output");
    assert_eq!(output.exit_code, 0, "stderr: {}", output.stderr);
    assert!(!output.stdout.trim().is_empty(), "codex produced no output");

    harness.terminate_and_cleanup(environment);
}

fn assert_command(
    session: &ControlSession,
    environment_id: &str,
    command: &[&str],
    exit_code: i32,
    stdout: &str,
) {
    let result = session
        .call(
            environment_id,
            Operation::OperateEnvironment {
                command: arguments(command),
                agent: None,
            },
        )
        .unwrap();
    assert_eq!(result.status, ResultStatus::Completed, "{result:?}");
    let output = result.output.expect("operate result has no command output");
    assert_eq!(output.exit_code, exit_code, "stderr: {}", output.stderr);
    assert_eq!(output.stdout, stdout);
}

fn assert_completed_in_state(result: &OperationResult, state: EnvironmentState) {
    assert_eq!(result.status, ResultStatus::Completed, "{result:?}");
    assert_eq!(result.environment_state, Some(state), "{result:?}");
}

fn assert_state_directory_has_no_environment(state_directory: &Path, environment_id: &str) {
    let residue = fs::read_dir(state_directory)
        .unwrap()
        .flatten()
        .any(|entry| entry.file_name().to_string_lossy().contains(environment_id));
    assert!(!residue, "cleanup left resources for {environment_id}");
}

fn firecracker_vm_state(process: &ProcessInfo) -> String {
    let socket = argument_after(&process.arguments, "--api-sock")
        .expect("Firecracker process has no --api-sock argument");
    let output = Command::new("curl")
        .args(["--silent", "--show-error", "--unix-socket", socket])
        .arg("http://localhost/")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "curl failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    response["state"].as_str().unwrap().to_string()
}

fn argument_after<'a>(arguments: &'a [String], name: &str) -> Option<&'a str> {
    arguments
        .windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].as_str())
}

fn matching_firecracker_processes(
    binary: &Path,
    environment_id: &str,
    state_directory: &Path,
) -> Vec<ProcessInfo> {
    let binary = binary
        .canonicalize()
        .unwrap_or_else(|_| binary.to_path_buf());
    let environment_directory = state_directory.join(environment_id);
    let environment_directory = environment_directory.to_string_lossy().into_owned();
    all_processes()
        .into_iter()
        .filter(|process| {
            process
                .arguments
                .first()
                .map(PathBuf::from)
                .and_then(|path| path.canonicalize().ok())
                .is_some_and(|path| path == binary)
                && argument_after(&process.arguments, "--id") == Some(environment_id)
                && process
                    .arguments
                    .iter()
                    .any(|argument| argument.contains(&environment_directory))
        })
        .collect()
}

fn all_processes() -> Vec<ProcessInfo> {
    fs::read_dir("/proc")
        .unwrap()
        .flatten()
        .filter_map(|entry| {
            let pid = entry.file_name().to_string_lossy().parse::<u32>().ok()?;
            let command_line = fs::read(entry.path().join("cmdline")).ok()?;
            let arguments = command_line
                .split(|byte| *byte == 0)
                .filter(|argument| !argument.is_empty())
                .map(|argument| String::from_utf8_lossy(argument).into_owned())
                .collect::<Vec<_>>();
            (!arguments.is_empty()).then_some(ProcessInfo { pid, arguments })
        })
        .collect()
}

fn kill(pid: u32) {
    let _ = Command::new("kill")
        .args(["-KILL", &pid.to_string()])
        .status();
}

fn wait_until<T>(timeout: Duration, mut check: impl FnMut() -> Option<T>) -> Option<T> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(value) = check() {
            return Some(value);
        }
        if Instant::now() >= deadline {
            return None;
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn arguments(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

fn built_binary(name: &str) -> PathBuf {
    if name == "ascout-runtime-adapter"
        && let Some(path) = option_env!("CARGO_BIN_EXE_ascout-runtime-adapter")
    {
        return PathBuf::from(path);
    }
    if name == "ascout-agent-access"
        && let Some(path) = option_env!("CARGO_BIN_EXE_ascout-agent-access")
    {
        return PathBuf::from(path);
    }
    workspace().join("target/debug").join(name)
}

fn workspace() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn assert_file(path: &Path) {
    assert!(
        path.is_file(),
        "required file is missing: {}",
        path.display()
    );
}

const GUEST_IP: &str = "172.16.0.2";

fn ssh_into_network(namespace: &str, private_key: &Path, command: &str) -> Result<String, String> {
    let output = Command::new("sudo")
        .args(["-n", "ip", "netns", "exec", namespace])
        .args([
            "setpriv",
            "--reuid",
            &current_user(),
            "--regid",
            &current_group(),
            "--init-groups",
        ])
        .args([
            "ssh",
            "-F",
            "/dev/null",
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=2",
            "-o",
            "StrictHostKeyChecking=no",
            "-o",
            "UserKnownHostsFile=/dev/null",
            "-i",
        ])
        .arg(private_key)
        .arg(format!("root@{GUEST_IP}"))
        .arg(command)
        .output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

fn current_user() -> String {
    String::from_utf8_lossy(&Command::new("id").arg("-un").output().unwrap().stdout)
        .trim()
        .to_string()
}

fn current_group() -> String {
    String::from_utf8_lossy(&Command::new("id").arg("-gn").output().unwrap().stdout)
        .trim()
        .to_string()
}

fn unique_directory(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    PathBuf::from("/tmp/opencode/afc").join(format!("{name}-{}-{nonce}", std::process::id()))
}
