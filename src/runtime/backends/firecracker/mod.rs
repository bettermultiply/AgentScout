
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, Weak};
use std::thread;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{Value, json};

pub mod guest_agent;

use crate::protocol::contract::{CommandOutput, EnvironmentState, RuntimeCapabilities};
use crate::runtime::backend::{Backend, BackendContext, BackendError, CancelHandle};
use crate::runtime::backends::firecracker::guest_agent::{GuestRequest, GuestResponse, PORT};

#[derive(Debug, Deserialize)]
pub struct FirecrackerConfig {
    #[serde(default)]
    pub control_socket: PathBuf,
    #[serde(default)]
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
    networks: Mutex<NetworkRegistry>,
    api: Arc<dyn FirecrackerApi>,
    operations: Mutex<HashMap<String, Weak<Mutex<()>>>>,
}

mod backend_impl;
struct Environment {
    state: EnvironmentState,
    child: Option<Child>,
    directory: PathBuf,
    api_socket: PathBuf,
    vsock_socket: PathBuf,
    network: EnvironmentNetwork,
}

mod guest_ops;
mod network;
use guest_ops::FirecrackerApi;
use network::{EnvironmentNetwork, NetworkRegistry};
