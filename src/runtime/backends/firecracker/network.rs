//! Firecracker 网络命名空间与受限网络命令。

use super::*;
use super::guest_ops::error;

/// 环境网络：独立 netns、网桥、TAP、veth 与双层 NAT。
///
/// 每个环境拥有自己的 network namespace，guest 因此可以使用固定 MAC 与地址
/// 而不与其他环境冲突。创建与清理需要网络管理权限，经免密 sudo 执行。
#[derive(Clone)]
pub(super) struct EnvironmentNetwork {
    pub(super) namespace: String,
    pub(super) veth_host: String,
    pub(super) veth_subnet: String,
    pub(super) upstream: String,
}

pub(super) const BRIDGE_NAME: &str = "br0";
pub(super) const TAP_NAME: &str = "tap0";
pub(super) const NS_VETH_NAME: &str = "veth0";
pub(super) const BRIDGE_IP: &str = "172.16.0.1/30";
pub(super) const BRIDGE_GATEWAY: &str = "172.16.0.1";
pub(super) const TAP_SUBNET: &str = "172.16.0.0/30";
pub(super) const GUEST_MAC: &str = "06:00:ac:10:00:02";

/// 环境网络所在的 network namespace 名称，供诊断和运维定位环境网络。
pub fn environment_network_namespace(environment_id: &str) -> String {
    format!("fc-{}", short_identifier(environment_id))
}

impl EnvironmentNetwork {
    pub(super) fn create(environment_id: &str) -> Result<Self, BackendError> {
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

    pub(super) fn setup(&self) -> Result<(), BackendError> {
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

    pub(super) fn teardown(&self) {
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

    pub(super) fn host_ip(&self) -> String {
        self.veth_address(1)
    }

    pub(super) fn ns_ip(&self) -> String {
        self.veth_address(2)
    }

    pub(super) fn gateway(&self) -> String {
        self.veth_address(1).trim_end_matches("/30").to_string()
    }

    pub(super) fn veth_address(&self, offset: u8) -> String {
        let base = self.veth_subnet.trim_end_matches("/30");
        let (prefix, last) = base.rsplit_once('.').unwrap();
        let last: u8 = last.parse().unwrap();
        format!("{prefix}.{}/30", last + offset)
    }

    /// 在环境 netns 内以降权用户启动 Firecracker，保持进程与文件归 Adapter 用户。
    pub(super) fn firecracker_command(&self, binary: &Path) -> Command {
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

pub(super) fn short_identifier(environment_id: &str) -> String {
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

pub(super) fn fnv1a(bytes: &[u8]) -> u32 {
    let mut hash = 0x811c_9dc5_u32;
    for byte in bytes {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

pub(super) fn current_user() -> String {
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

pub(super) fn current_group() -> String {
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

pub(super) fn privileged(args: &[&str]) -> Result<(), BackendError> {
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

pub(super) fn privileged_netns(namespace: &str, args: &[&str]) -> Result<(), BackendError> {
    let mut command = vec!["ip", "netns", "exec", namespace];
    command.extend_from_slice(args);
    privileged(&command)
}

pub(super) fn upstream_interface() -> Result<String, BackendError> {
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

