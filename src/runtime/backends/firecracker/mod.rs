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

pub mod guest_access;

use crate::runtime::backend::{Backend, BackendError, CancelHandle};
use crate::protocol::contract::{CommandOutput, EnvironmentState};
use crate::runtime::backends::firecracker::guest_access::{GuestRequest, GuestResponse, PORT};

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
use network::EnvironmentNetwork;

pub use network::environment_network_namespace;
