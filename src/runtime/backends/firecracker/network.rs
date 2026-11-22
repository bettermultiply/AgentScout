
use std::collections::{BTreeSet, HashMap};

use super::guest_ops::error;
use super::*;

#[derive(Clone)]
pub(super) struct EnvironmentNetwork {
    pub(super) namespace: String,
    pub(super) veth_host: String,
    pub(super) veth_subnet: String,
    pub(super) upstream: String,
    pub(super) teardown_enabled: bool,
}

pub(super) const BRIDGE_NAME: &str = "br0";
pub(super) const TAP_NAME: &str = "tap0";
pub(super) const NS_VETH_NAME: &str = "veth0";
pub(super) const BRIDGE_IP: &str = "172.16.0.1/30";
pub(super) const BRIDGE_GATEWAY: &str = "172.16.0.1";
pub(super) const TAP_SUBNET: &str = "172.16.0.0/30";
pub(super) const GUEST_MAC: &str = "06:00:ac:10:00:02";

/// Linux 网络接口名上限（IFNAMSIZ - 1）。netns 名同样按这个上限约束。
const MAX_NETWORK_NAME: usize = 15;

/// 进程内索引避免环境 ID 哈希碰撞。释放后可复用该索引。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct NetworkAllocation {
    index: usize,
    pub(super) namespace: String,
    pub(super) veth_host: String,
    pub(super) veth_subnet: String,
}

pub(super) const NETWORK_CAPACITY: usize = 254 * 64;

#[derive(Debug)]
pub(super) struct NetworkRegistry {
    allocations: HashMap<String, NetworkAllocation>,
    free: BTreeSet<usize>,
    capacity: usize,
}

impl Default for NetworkRegistry {
    fn default() -> Self {
        Self::with_capacity(NETWORK_CAPACITY)
    }
}

impl NetworkRegistry {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            allocations: HashMap::new(),
            free: (0..capacity).collect(),
            capacity,
        }
    }

    pub(super) fn allocate(
        &mut self,
        environment_id: &str,
    ) -> Result<NetworkAllocation, BackendError> {
        if self.allocations.contains_key(environment_id) {
            return Err(BackendError::new(format!(
                "network already allocated for environment {environment_id}"
            )));
        }
        let Some(index) = self.free.pop_first() else {
            return Err(BackendError::new(format!(
                "no free environment subnet remains (capacity {})",
                self.capacity
            )));
        };
        let allocation = match allocation_for_index(index) {
            Ok(allocation) => allocation,
            Err(failure) => {
                self.free.insert(index);
                return Err(failure);
            }
        };
        self.allocations
            .insert(environment_id.to_string(), allocation.clone());
        Ok(allocation)
    }

    pub(super) fn release(&mut self, environment_id: &str) -> Option<NetworkAllocation> {
        let allocation = self.allocations.remove(environment_id)?;
        self.free.insert(allocation.index);
        Some(allocation)
    }
}

fn allocation_for_index(index: usize) -> Result<NetworkAllocation, BackendError> {
    if index >= NETWORK_CAPACITY {
        return Err(BackendError::new(
            "environment subnet index is out of range",
        ));
    }
    let third = 1 + (index / 64);
    let last = (index % 64) * 4;
    let namespace = format!("fc-{index:x}");
    let veth_host = format!("vh-{index:x}");
    if namespace.len() > MAX_NETWORK_NAME || veth_host.len() > MAX_NETWORK_NAME {
        return Err(BackendError::new("environment network name is too long"));
    }
    Ok(NetworkAllocation {
        index,
        namespace,
        veth_host,
        veth_subnet: format!("10.213.{third}.{last}/30"),
    })
}

impl EnvironmentNetwork {
    pub(super) fn new(allocation: &NetworkAllocation) -> Result<Self, BackendError> {
        let network = Self {
            namespace: allocation.namespace.clone(),
            veth_host: allocation.veth_host.clone(),
            veth_subnet: allocation.veth_subnet.clone(),
            upstream: upstream_interface()?,
            teardown_enabled: true,
        };
        Ok(network)
    }

    pub(super) fn setup(&self) -> Result<(), BackendError> {
        let namespace = &self.namespace;
        privileged_remove(&["ip", "link", "delete", &self.veth_host])?;
        privileged_remove(&["ip", "netns", "delete", namespace])?;
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

    pub(super) fn teardown(&self) -> Result<(), BackendError> {
        if !self.teardown_enabled {
            return Ok(());
        }
        privileged_remove(&[
            "iptables",
            "-D",
            "FORWARD",
            "-s",
            &self.veth_subnet,
            "-j",
            "ACCEPT",
        ])?;
        privileged_remove(&[
            "iptables",
            "-D",
            "FORWARD",
            "-d",
            &self.veth_subnet,
            "-j",
            "ACCEPT",
        ])?;
        privileged_remove(&[
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
        ])?;
        // 删除 netns 会把其中的 veth 移回宿主，必须先删除 veth 对。
        privileged_remove(&["ip", "link", "delete", &self.veth_host])?;
        privileged_remove(&["ip", "netns", "delete", &self.namespace])?;
        Ok(())
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
        .env("LC_ALL", "C")
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

/// Ignore only the command's explicit missing-resource diagnostic.
fn privileged_remove(args: &[&str]) -> Result<(), BackendError> {
    privileged(args).or_else(|failure| {
        let prefix = format!("privileged command failed: sudo {}: ", args.join(" "));
        let diagnostic = failure.message.strip_prefix(&prefix).unwrap_or("");
        let absent = if args.first() == Some(&"iptables") {
            let missing = "Bad rule (does a matching rule exist in that chain?).";
            diagnostic == format!("iptables: {missing}")
                || (diagnostic.starts_with("iptables v")
                    && !diagnostic.contains('\n')
                    && diagnostic
                        .split_once(": ")
                        .is_some_and(|(_, text)| text == missing))
        } else if args.get(1) == Some(&"link") {
            diagnostic == format!("Cannot find device \"{}\"", args[3])
        } else if args.get(1) == Some(&"netns") {
            ["/run/netns", "/var/run/netns"].iter().any(|directory| {
                diagnostic == format!(
                    "Cannot remove namespace file \"{directory}/{}\": No such file or directory",
                    args[3]
                )
            })
        } else {
            false
        };
        if absent { Ok(()) } else { Err(failure) }
    })
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
