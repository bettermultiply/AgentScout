//! Firecracker Runtime Adapter 后端。

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::backend::{Backend, BackendError, CancelHandle};
use crate::contract::{CommandOutput, EnvironmentState};
use crate::guest_access::{GuestRequest, GuestResponse, PORT};

#[derive(Debug, Deserialize)]
pub struct FirecrackerConfig {
    pub control_socket: PathBuf,
    pub adapter_instance: String,
    pub firecracker_path: PathBuf,
    pub kernel_path: PathBuf,
    pub rootfs_path: PathBuf,
    pub guest_access_path: PathBuf,
    pub state_directory: PathBuf,
}

pub struct FirecrackerBackend {
    config: FirecrackerConfig,
    environments: Mutex<HashMap<String, Arc<Mutex<Environment>>>>,
}

/// 环境网络：独立 netns、网桥、TAP、veth 与双层 NAT。
///
/// 每个环境拥有自己的 network namespace，guest 因此可以使用固定 MAC 与地址
/// 而不与其他环境冲突。创建与清理需要网络管理权限，经免密 sudo 执行。
#[derive(Clone)]
struct EnvironmentNetwork {
    namespace: String,
    veth_host: String,
    veth_subnet: String,
    upstream: String,
}

const BRIDGE_NAME: &str = "br0";
const TAP_NAME: &str = "tap0";
const NS_VETH_NAME: &str = "veth0";
const BRIDGE_IP: &str = "172.16.0.1/30";
const BRIDGE_GATEWAY: &str = "172.16.0.1";
const TAP_SUBNET: &str = "172.16.0.0/30";
const GUEST_MAC: &str = "06:00:ac:10:00:02";

/// 环境网络所在的 network namespace 名称，供诊断和运维定位环境网络。
pub fn environment_network_namespace(environment_id: &str) -> String {
    format!("fc-{}", short_identifier(environment_id))
}

impl EnvironmentNetwork {
    fn create(environment_id: &str) -> Result<Self, BackendError> {
        let hash = fnv1a(environment_id.as_bytes());
        let network = Self {
            namespace: environment_network_namespace(environment_id),
            veth_host: format!("vh-{}", short_identifier(environment_id)),
            veth_subnet: format!(
                "10.213.{}.{}/30",
                (hash >> 16) as u8,
                ((hash >> 8) as u8) & 0xfc
            ),
            upstream: upstream_interface()?,
        };
        if let Err(failure) = network.setup() {
            network.teardown();
            return Err(failure);
        }
        Ok(network)
    }

    fn setup(&self) -> Result<(), BackendError> {
        let namespace = &self.namespace;
        // 清理同名残留，保证重复建立从干净状态开始。
        let _ = privileged(&["ip", "link", "delete", &self.veth_host]);
        let _ = privileged(&["ip", "netns", "delete", namespace]);
        privileged(&["ip", "netns", "add", namespace])?;
        privileged_netns(
            namespace,
            &["ip", "link", "add", BRIDGE_NAME, "type", "bridge"],
        )?;
        privileged_netns(
            namespace,
            &[
                "ip",
                "link",
                "set",
                BRIDGE_NAME,
                "address",
                "02:00:ac:10:00:01",
            ],
        )?;
        privileged_netns(
            namespace,
            &["ip", "addr", "add", BRIDGE_IP, "dev", BRIDGE_NAME],
        )?;
        privileged_netns(namespace, &["ip", "link", "set", BRIDGE_NAME, "up"])?;
        let user = current_user();
        privileged_netns(
            namespace,
            &[
                "ip", "tuntap", "add", "dev", TAP_NAME, "mode", "tap", "user", &user,
            ],
        )?;
        privileged_netns(
            namespace,
            &["ip", "link", "set", TAP_NAME, "master", BRIDGE_NAME],
        )?;
        privileged_netns(namespace, &["ip", "link", "set", TAP_NAME, "up"])?;

        privileged(&[
            "ip",
            "link",
            "add",
            &self.veth_host,
            "type",
            "veth",
            "peer",
            "name",
            NS_VETH_NAME,
        ])?;
        privileged(&["ip", "link", "set", NS_VETH_NAME, "netns", namespace])?;
        privileged(&["ip", "addr", "add", &self.host_ip(), "dev", &self.veth_host])?;
        privileged(&["ip", "link", "set", &self.veth_host, "up"])?;
        privileged_netns(
            namespace,
            &["ip", "addr", "add", &self.ns_ip(), "dev", NS_VETH_NAME],
        )?;
        privileged_netns(namespace, &["ip", "link", "set", NS_VETH_NAME, "up"])?;
        privileged_netns(
            namespace,
            &["ip", "route", "add", "default", "via", &self.gateway()],
        )?;

        privileged_netns(
            namespace,
            &[
                "iptables",
                "-t",
                "nat",
                "-A",
                "POSTROUTING",
                "-s",
                TAP_SUBNET,
                "-o",
                NS_VETH_NAME,
                "-j",
                "MASQUERADE",
            ],
        )?;
        privileged_netns(namespace, &["iptables", "-P", "FORWARD", "ACCEPT"])?;
        privileged(&[
            "iptables",
            "-t",
            "nat",
            "-A",
            "POSTROUTING",
            "-s",
            &self.veth_subnet,
            "-o",
            &self.upstream,
            "-j",
            "MASQUERADE",
        ])?;
        privileged(&[
            "iptables",
            "-I",
            "FORWARD",
            "1",
            "-s",
            &self.veth_subnet,
            "-j",
            "ACCEPT",
        ])?;
        privileged(&[
            "iptables",
            "-I",
            "FORWARD",
            "1",
            "-d",
            &self.veth_subnet,
            "-j",
            "ACCEPT",
        ])?;
        Ok(())
    }

    fn teardown(&self) {
        let _ = privileged(&[
            "iptables",
            "-D",
            "FORWARD",
            "-s",
            &self.veth_subnet,
            "-j",
            "ACCEPT",
        ]);
        let _ = privileged(&[
            "iptables",
            "-D",
            "FORWARD",
            "-d",
            &self.veth_subnet,
            "-j",
            "ACCEPT",
        ]);
        let _ = privileged(&[
            "iptables",
            "-t",
            "nat",
            "-D",
            "POSTROUTING",
            "-s",
            &self.veth_subnet,
            "-o",
            &self.upstream,
            "-j",
            "MASQUERADE",
        ]);
        // 删除 netns 会把其中的 veth 移回宿主，必须先删除 veth 对。
        let _ = privileged(&["ip", "link", "delete", &self.veth_host]);
        let _ = privileged(&["ip", "netns", "delete", &self.namespace]);
    }

    fn host_ip(&self) -> String {
        self.veth_address(1)
    }

    fn ns_ip(&self) -> String {
        self.veth_address(2)
    }

    fn gateway(&self) -> String {
        self.veth_address(1).trim_end_matches("/30").to_string()
    }

    fn veth_address(&self, offset: u8) -> String {
        let base = self.veth_subnet.trim_end_matches("/30");
        let (prefix, last) = base.rsplit_once('.').unwrap();
        let last: u8 = last.parse().unwrap();
        format!("{prefix}.{}/30", last + offset)
    }

    /// 在环境 netns 内以降权用户启动 Firecracker，保持进程与文件归 Adapter 用户。
    fn firecracker_command(&self, binary: &Path) -> Command {
        let mut command = Command::new("sudo");
        command
            .args(["-n", "ip", "netns", "exec", &self.namespace])
            .args([
                "setpriv",
                "--reuid",
                &current_user(),
                "--regid",
                &current_group(),
                "--init-groups",
            ])
            .arg(binary);
        command
    }
}

fn short_identifier(environment_id: &str) -> String {
    let mut body: String = environment_id
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || *character == '-')
        .take(8)
        .collect();
    body.push_str(&format!(
        "{:04x}",
        fnv1a(environment_id.as_bytes()) & 0xffff
    ));
    body
}

fn fnv1a(bytes: &[u8]) -> u32 {
    let mut hash = 0x811c_9dc5_u32;
    for byte in bytes {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

fn current_user() -> String {
    unsafe {
        let entry = libc::getpwuid(libc::getuid());
        if entry.is_null() {
            String::new()
        } else {
            std::ffi::CStr::from_ptr((*entry).pw_name)
                .to_string_lossy()
                .into_owned()
        }
    }
}

fn current_group() -> String {
    unsafe {
        let entry = libc::getgrgid(libc::getgid());
        if entry.is_null() {
            String::new()
        } else {
            std::ffi::CStr::from_ptr((*entry).gr_name)
                .to_string_lossy()
                .into_owned()
        }
    }
}

fn privileged(args: &[&str]) -> Result<(), BackendError> {
    let output = Command::new("sudo")
        .args(["-n"])
        .args(args)
        .output()
        .map_err(error)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(BackendError::new(format!(
            "privileged command failed: sudo {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

fn privileged_netns(namespace: &str, args: &[&str]) -> Result<(), BackendError> {
    let mut command = vec!["ip", "netns", "exec", namespace];
    command.extend_from_slice(args);
    privileged(&command)
}

fn upstream_interface() -> Result<String, BackendError> {
    let output = Command::new("ip")
        .args(["-j", "route", "list", "default"])
        .output()
        .map_err(error)?;
    let routes: Vec<Value> = serde_json::from_slice(&output.stdout).map_err(error)?;
    routes
        .first()
        .and_then(|route| route.get("dev"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| BackendError::new("host has no default route for environment NAT"))
}

struct Environment {
    state: EnvironmentState,
    child: Option<Child>,
    directory: PathBuf,
    api_socket: PathBuf,
    vsock_socket: PathBuf,
    network: EnvironmentNetwork,
}

impl FirecrackerBackend {
    pub fn new(config: FirecrackerConfig) -> Result<Self, BackendError> {
        for path in [
            &config.firecracker_path,
            &config.kernel_path,
            &config.rootfs_path,
            &config.guest_access_path,
        ] {
            if !path.is_file() {
                return Err(BackendError::new(format!(
                    "required file is missing: {}",
                    path.display()
                )));
            }
        }
        fs::create_dir_all(&config.state_directory).map_err(error)?;
        Ok(Self {
            config,
            environments: Mutex::new(HashMap::new()),
        })
    }

    fn environment(&self, id: &str) -> Result<Arc<Mutex<Environment>>, BackendError> {
        self.environments
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| BackendError::new("unknown environment"))
    }

    fn create_rootfs(&self, directory: &Path) -> Result<PathBuf, BackendError> {
        let rootfs = directory.join("rootfs.ext4");
        run(Command::new("cp")
            .args(["--reflink=auto", "--sparse=always"])
            .arg(&self.config.rootfs_path)
            .arg(&rootfs))?;
        let service = directory.join("ascout-agent-access.service");
        fs::write(
            &service,
            "[Unit]\nDescription=AScout Agent access\nAfter=systemd-modules-load.service\n\n[Service]\nExecStart=/usr/local/bin/ascout-agent-access\nRestart=always\n\n[Install]\nWantedBy=multi-user.target\n",
        )
        .map_err(error)?;
        debugfs_write(
            &rootfs,
            &self.config.guest_access_path,
            "/usr/local/bin/ascout-agent-access",
        )?;
        debugfs_write(
            &rootfs,
            &service,
            "/etc/systemd/system/ascout-agent-access.service",
        )?;
        run(Command::new("debugfs")
            .args(["-w", "-R"])
            .arg("set_inode_field /usr/local/bin/ascout-agent-access mode 0100755")
            .arg(&rootfs))?;
        run(
            Command::new("debugfs")
                .args(["-w", "-R"])
                .arg("symlink /etc/systemd/system/multi-user.target.wants/ascout-agent-access.service ../ascout-agent-access.service")
                .arg(&rootfs),
        )?;
        Ok(rootfs)
    }

    fn start_fresh(
        &self,
        id: &str,
        directory: PathBuf,
        rootfs: PathBuf,
    ) -> Result<Environment, BackendError> {
        let network = EnvironmentNetwork::create(id)?;
        let api_socket = directory.join("api.sock");
        let vsock_socket = directory.join("vsock.sock");
        if let Err(failure) = check_socket_paths(&api_socket, &vsock_socket) {
            network.teardown();
            return Err(failure);
        }
        let child = match spawn_firecracker(
            &self.config.firecracker_path,
            id,
            &directory,
            &api_socket,
            &network,
        ) {
            Ok(child) => child,
            Err(failure) => {
                network.teardown();
                return Err(failure);
            }
        };
        let mut environment = Environment {
            state: EnvironmentState::Available,
            child: Some(child),
            directory,
            api_socket,
            vsock_socket,
            network,
        };
        if let Err(failure) = (|| {
            wait_for_path(&environment.api_socket, Duration::from_secs(5))?;
            api_put(
                &environment.api_socket,
                "/machine-config",
                &json!({
                    "vcpu_count": 2,
                    "mem_size_mib": 512
                }),
            )?;
            api_put(
                &environment.api_socket,
                "/boot-source",
                &json!({
                    "kernel_image_path": self.config.kernel_path,
                    "boot_args": "console=ttyS0 reboot=k panic=1 pci=off root=/dev/vda rw"
                }),
            )?;
            api_put(
                &environment.api_socket,
                "/drives/rootfs",
                &json!({
                    "drive_id": "rootfs",
                    "path_on_host": rootfs,
                    "is_root_device": true,
                    "is_read_only": false
                }),
            )?;
            api_put(
                &environment.api_socket,
                "/network-interfaces/eth0",
                &json!({
                    "iface_id": "eth0",
                    "guest_mac": GUEST_MAC,
                    "host_dev_name": TAP_NAME
                }),
            )?;
            api_put(
                &environment.api_socket,
                "/vsock",
                &json!({
                    "guest_cid": 3,
                    "uds_path": environment.vsock_socket
                }),
            )?;
            api_put(
                &environment.api_socket,
                "/actions",
                &json!({
                    "action_type": "InstanceStart"
                }),
            )?;
            wait_for_guest(&environment.vsock_socket, Duration::from_secs(20))?;
            configure_guest_network(&environment.vsock_socket)
        })() {
            if let Some(child) = environment.child.as_mut() {
                kill_environment_process(child);
            }
            environment.network.teardown();
            return Err(failure);
        }
        Ok(environment)
    }

    fn start_from_snapshot(
        &self,
        id: &str,
        directory: PathBuf,
        rootfs: PathBuf,
        snapshot: PathBuf,
        memory: PathBuf,
    ) -> Result<Environment, BackendError> {
        let network = EnvironmentNetwork::create(id)?;
        let api_socket = directory.join("api.sock");
        let vsock_socket = directory.join("vsock.sock");
        if let Err(failure) = check_socket_paths(&api_socket, &vsock_socket) {
            network.teardown();
            return Err(failure);
        }
        let child = match spawn_firecracker(
            &self.config.firecracker_path,
            id,
            &directory,
            &api_socket,
            &network,
        ) {
            Ok(child) => child,
            Err(failure) => {
                network.teardown();
                return Err(failure);
            }
        };
        let mut environment = Environment {
            state: EnvironmentState::Available,
            child: Some(child),
            directory,
            api_socket,
            vsock_socket,
            network,
        };
        if let Err(failure) = (|| {
            wait_for_path(&environment.api_socket, Duration::from_secs(5))?;
            api_put(
                &environment.api_socket,
                "/snapshot/load",
                &json!({
                    "snapshot_path": snapshot,
                    "mem_file_path": memory,
                    "resume_vm": false,
                    "vsock_override": { "uds_path": environment.vsock_socket },
                    "network_overrides": [{
                        "iface_id": "eth0",
                        "host_dev_name": TAP_NAME
                    }]
                }),
            )?;
            api_patch(
                &environment.api_socket,
                "/drives/rootfs",
                &json!({
                    "drive_id": "rootfs",
                    "path_on_host": rootfs
                }),
            )?;
            api_patch(
                &environment.api_socket,
                "/vm",
                &json!({ "state": "Resumed" }),
            )?;
            wait_for_guest(&environment.vsock_socket, Duration::from_secs(20))?;
            configure_guest_network(&environment.vsock_socket)
        })() {
            if let Some(child) = environment.child.as_mut() {
                kill_environment_process(child);
            }
            environment.network.teardown();
            return Err(failure);
        }
        Ok(environment)
    }
}

impl Backend for FirecrackerBackend {
    fn establish(&self, environment_id: &str) -> Result<(), BackendError> {
        let directory = self.config.state_directory.join(environment_id);
        if directory.exists() {
            return Err(BackendError::new("environment directory already exists"));
        }
        fs::create_dir_all(&directory).map_err(error)?;
        let result = (|| {
            let rootfs = self.create_rootfs(&directory)?;
            self.start_fresh(environment_id, directory.clone(), rootfs)
        })();
        match result {
            Ok(environment) => {
                self.environments.lock().unwrap().insert(
                    environment_id.to_string(),
                    Arc::new(Mutex::new(environment)),
                );
                Ok(())
            }
            Err(failure) => {
                let _ = fs::remove_dir_all(directory);
                Err(failure)
            }
        }
    }

    fn operate(
        &self,
        environment_id: &str,
        command: &[String],
        cancel: &CancelHandle,
    ) -> Result<CommandOutput, BackendError> {
        let environment = self.environment(environment_id)?;
        let socket = environment.lock().unwrap().vsock_socket.clone();
        guest_command(&socket, command, cancel)
    }

    fn branch(&self, parent: &str, child: &str) -> Result<(), BackendError> {
        let parent_environment = self.environment(parent)?;
        let child_directory = self.config.state_directory.join(child);
        if child_directory.exists() {
            return Err(BackendError::new(
                "child environment directory already exists",
            ));
        }
        fs::create_dir_all(&child_directory).map_err(error)?;

        let preparation = (|| {
            let child_rootfs = child_directory.join("rootfs.ext4");
            let snapshot = child_directory.join("snapshot.state");
            let memory = child_directory.join("snapshot.memory");
            let parent_environment = parent_environment.lock().unwrap();
            let resume_parent = parent_environment.state == EnvironmentState::Available;
            if resume_parent {
                api_patch(
                    &parent_environment.api_socket,
                    "/vm",
                    &json!({ "state": "Paused" }),
                )?;
            }
            let snapshot_result = (|| {
                api_put(
                    &parent_environment.api_socket,
                    "/snapshot/create",
                    &json!({
                        "snapshot_type": "Full",
                        "snapshot_path": snapshot,
                        "mem_file_path": memory
                    }),
                )?;
                run(Command::new("cp")
                    .args(["--reflink=auto", "--sparse=always"])
                    .arg(parent_environment.directory.join("rootfs.ext4"))
                    .arg(&child_rootfs))?;
                Ok::<_, BackendError>((child_rootfs, snapshot, memory))
            })();
            if resume_parent {
                let resume_result = api_patch(
                    &parent_environment.api_socket,
                    "/vm",
                    &json!({ "state": "Resumed" }),
                );
                if snapshot_result.is_ok() {
                    resume_result?;
                }
            }
            snapshot_result
        })();

        let result = preparation.and_then(|(rootfs, snapshot, memory)| {
            self.start_from_snapshot(child, child_directory.clone(), rootfs, snapshot, memory)
        });
        match result {
            Ok(environment) => {
                self.environments
                    .lock()
                    .unwrap()
                    .insert(child.to_string(), Arc::new(Mutex::new(environment)));
                Ok(())
            }
            Err(failure) => {
                let _ = fs::remove_dir_all(child_directory);
                Err(failure)
            }
        }
    }

    fn suspend(&self, environment_id: &str) -> Result<(), BackendError> {
        let environment = self.environment(environment_id)?;
        let mut environment = environment.lock().unwrap();
        if environment.state != EnvironmentState::Suspended {
            api_patch(
                &environment.api_socket,
                "/vm",
                &json!({ "state": "Paused" }),
            )?;
            environment.state = EnvironmentState::Suspended;
        }
        Ok(())
    }

    fn resume(&self, environment_id: &str) -> Result<(), BackendError> {
        let environment = self.environment(environment_id)?;
        let mut environment = environment.lock().unwrap();
        if environment.state != EnvironmentState::Available {
            api_patch(
                &environment.api_socket,
                "/vm",
                &json!({ "state": "Resumed" }),
            )?;
            environment.state = EnvironmentState::Available;
        }
        Ok(())
    }

    fn terminate(&self, environment_id: &str) -> Result<(), BackendError> {
        let environment = self.environment(environment_id)?;
        let mut environment = environment.lock().unwrap();
        if let Some(mut child) = environment.child.take() {
            kill_environment_process(&mut child);
        }
        environment.state = EnvironmentState::Terminated;
        Ok(())
    }

    fn cleanup(&self, environment_id: &str) -> Result<(), BackendError> {
        let environment = self.environment(environment_id)?;
        let (directory, network) = {
            let mut environment = environment.lock().unwrap();
            if let Some(mut child) = environment.child.take() {
                kill_environment_process(&mut child);
            }
            environment.state = EnvironmentState::Cleaned;
            (environment.directory.clone(), environment.network.clone())
        };
        network.teardown();
        fs::remove_dir_all(&directory).map_err(error)?;
        Ok(())
    }

    fn state(&self, environment_id: &str) -> Option<EnvironmentState> {
        let environment = self
            .environments
            .lock()
            .unwrap()
            .get(environment_id)
            .cloned()?;
        let mut environment = environment.lock().unwrap();
        if matches!(
            environment.state,
            EnvironmentState::Available | EnvironmentState::Suspended
        ) && let Some(child) = environment.child.as_mut()
            && child.try_wait().ok().flatten().is_some()
        {
            environment.child = None;
            environment.state = EnvironmentState::Terminated;
        }
        Some(environment.state)
    }

    fn environments(&self) -> Vec<String> {
        self.environments.lock().unwrap().keys().cloned().collect()
    }
}

fn guest_command(
    socket: &Path,
    command: &[String],
    cancel: &CancelHandle,
) -> Result<CommandOutput, BackendError> {
    let mut stream = UnixStream::connect(socket).map_err(error)?;
    stream
        .set_read_timeout(Some(Duration::from_millis(50)))
        .map_err(error)?;
    stream
        .write_all(format!("CONNECT {PORT}\n").as_bytes())
        .map_err(error)?;
    let mut reader = BufReader::new(stream.try_clone().map_err(error)?);
    let mut forwarding = String::new();
    read_line_cancellable(&mut reader, &mut forwarding, cancel)?;
    if !forwarding.starts_with("OK ") {
        return Err(BackendError::new(format!(
            "vsock forwarding failed: {}",
            forwarding.trim()
        )));
    }
    serde_json::to_writer(
        &mut stream,
        &GuestRequest {
            command: command.to_vec(),
        },
    )
    .map_err(error)?;
    stream.write_all(b"\n").map_err(error)?;
    stream.flush().map_err(error)?;

    let mut response = String::new();
    if let Err(failure) = read_line_cancellable(&mut reader, &mut response, cancel) {
        if failure.cancelled {
            // 先通知 guest 取消，再等它结束命令进程组，避免环境在残留清理前冻结。
            let _ = stream.shutdown(Shutdown::Write);
            wait_for_guest_cleanup(&mut reader);
        } else {
            let _ = stream.shutdown(Shutdown::Both);
        }
        return Err(failure);
    }
    let response: GuestResponse = serde_json::from_str(&response).map_err(error)?;
    Ok(CommandOutput {
        exit_code: response.exit_code,
        stdout: response.stdout,
        stderr: response.stderr,
    })
}

fn read_line_cancellable(
    reader: &mut BufReader<UnixStream>,
    output: &mut String,
    cancel: &CancelHandle,
) -> Result<(), BackendError> {
    loop {
        if cancel.is_cancelled() {
            return Err(BackendError::interrupted());
        }
        match reader.read_line(output) {
            Ok(0) => return Err(BackendError::new("guest access closed the connection")),
            Ok(_) => return Ok(()),
            Err(failure)
                if matches!(
                    failure.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(failure) => return Err(error(failure)),
        }
    }
}

/// 取消后等待 guest 关闭连接，作为命令进程组已结束的确认。
fn wait_for_guest_cleanup(reader: &mut BufReader<UnixStream>) {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut scratch = String::new();
    while Instant::now() < deadline {
        match reader.read_line(&mut scratch) {
            Ok(0) => return,
            Ok(_) => scratch.clear(),
            Err(failure)
                if matches!(
                    failure.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(_) => return,
        }
    }
}

fn wait_for_guest(socket: &Path, timeout: Duration) -> Result<(), BackendError> {
    let deadline = Instant::now() + timeout;
    let cancel = CancelHandle::default();
    loop {
        let command = vec!["true".to_string()];
        if guest_command(socket, &command, &cancel).is_ok() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(BackendError::new("guest Agent access did not become ready"));
        }
        thread::sleep(Duration::from_millis(100));
    }
}

/// 让 guest 的网络真正可用：镜像只配置接口地址，默认路由由环境建立时补齐。
fn configure_guest_network(socket: &Path) -> Result<(), BackendError> {
    let cancel = CancelHandle::default();
    let command = vec![
        "ip".to_string(),
        "route".to_string(),
        "replace".to_string(),
        "default".to_string(),
        "via".to_string(),
        BRIDGE_GATEWAY.to_string(),
    ];
    guest_command(socket, &command, &cancel).map(|_| ())
}

fn check_socket_paths(api_socket: &Path, vsock_socket: &Path) -> Result<(), BackendError> {
    for socket in [api_socket, vsock_socket] {
        if socket.as_os_str().as_encoded_bytes().len() >= 108 {
            return Err(BackendError::new(format!(
                "Unix socket path is too long: {}",
                socket.display()
            )));
        }
    }
    Ok(())
}

fn spawn_firecracker(
    binary: &Path,
    id: &str,
    directory: &Path,
    api_socket: &Path,
    network: &EnvironmentNetwork,
) -> Result<Child, BackendError> {
    let console = File::create(directory.join("console.log")).map_err(error)?;
    let mut command = network.firecracker_command(binary);
    command
        .args(["--id", id, "--api-sock"])
        .arg(api_socket)
        .stdout(console.try_clone().map_err(error)?)
        .stderr(console)
        // 独立进程组：Firecracker 经 sudo 链启动，终止时需要回收整组。
        .process_group(0);
    command.spawn().map_err(error)
}

fn kill_environment_process(child: &mut Child) {
    let process_group = child.id() as i32;
    unsafe {
        libc::kill(-process_group, libc::SIGKILL);
    }
    let _ = child.wait();
}

fn api_put(socket: &Path, path: &str, body: &Value) -> Result<(), BackendError> {
    api_request("PUT", socket, path, body)
}

fn api_patch(socket: &Path, path: &str, body: &Value) -> Result<(), BackendError> {
    api_request("PATCH", socket, path, body)
}

fn api_request(method: &str, socket: &Path, path: &str, body: &Value) -> Result<(), BackendError> {
    let body = serde_json::to_vec(body).map_err(error)?;
    let mut stream = UnixStream::connect(socket).map_err(error)?;
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .map_err(error)?;
    stream.write_all(&body).map_err(error)?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(error)?;
    let mut reader = BufReader::new(stream);
    let mut response = String::new();
    let mut content_length = 0_usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).map_err(error)? == 0 {
            break;
        }
        let end = line == "\r\n";
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            content_length = value.trim().parse().unwrap_or(0);
        }
        response.push_str(&line);
        if end {
            break;
        }
    }
    if content_length > 0 {
        let mut body = vec![0; content_length];
        reader.read_exact(&mut body).map_err(error)?;
        response.push_str(&String::from_utf8_lossy(&body));
    }
    let status = response.lines().next().unwrap_or_default();
    if !status.contains(" 204 ") && !status.contains(" 200 ") {
        return Err(BackendError::new(format!(
            "Firecracker API {method} {path} failed: {}",
            response.trim()
        )));
    }
    Ok(())
}

fn wait_for_path(path: &Path, timeout: Duration) -> Result<(), BackendError> {
    let deadline = Instant::now() + timeout;
    while !path.exists() {
        if Instant::now() >= deadline {
            return Err(BackendError::new(format!(
                "timed out waiting for {}",
                path.display()
            )));
        }
        thread::sleep(Duration::from_millis(20));
    }
    Ok(())
}

fn debugfs_write(rootfs: &Path, source: &Path, destination: &str) -> Result<(), BackendError> {
    run(Command::new("debugfs")
        .args(["-w", "-R"])
        .arg(format!("write {} {destination}", source.display()))
        .arg(rootfs))
}

fn run(command: &mut Command) -> Result<(), BackendError> {
    let output = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(error)?;
    if !output.status.success() {
        return Err(BackendError::new(format!(
            "command failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

fn error(failure: impl std::fmt::Display) -> BackendError {
    BackendError::new(failure.to_string())
}
