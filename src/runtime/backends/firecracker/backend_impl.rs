//! FirecrackerBackend 的环境生命周期与 Backend 实现。

use super::*;
use super::guest_ops::*;
use super::network::*;

/// 环境 id 会直接拼进状态目录路径，因此必须是单个普通路径分量。
pub(super) const MAX_ENVIRONMENT_ID_BYTES: usize = 128;

pub(super) fn validate_environment_id(environment_id: &str) -> Result<(), BackendError> {
    if environment_id.is_empty() {
        return Err(BackendError::new("environment id must not be empty"));
    }
    if environment_id == "." || environment_id == ".." {
        return Err(BackendError::new(format!(
            "environment id {environment_id:?} is not a valid path component"
        )));
    }
    if environment_id.len() > MAX_ENVIRONMENT_ID_BYTES {
        return Err(BackendError::new(format!(
            "environment id exceeds {MAX_ENVIRONMENT_ID_BYTES} bytes"
        )));
    }
    if environment_id
        .chars()
        .any(|character| matches!(character, '/' | '\\' | '\0'))
    {
        return Err(BackendError::new(format!(
            "environment id {environment_id:?} contains an invalid path character"
        )));
    }
    Ok(())
}

impl FirecrackerBackend {
    pub fn new(config: FirecrackerConfig) -> Result<Self, BackendError> {
        Self::with_api(config, Arc::new(RealFirecrackerApi))
    }

    pub(super) fn with_api(
        config: FirecrackerConfig,
        api: Arc<dyn FirecrackerApi>,
    ) -> Result<Self, BackendError> {
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
            networks: Mutex::new(NetworkRegistry::default()),
            api,
        })
    }

    pub(super) fn environment(&self, id: &str) -> Result<Arc<Mutex<Environment>>, BackendError> {
        validate_environment_id(id)?;
        self.environments
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| BackendError::new(format!("unknown environment {id}")))
    }

    fn create_network(&self, id: &str) -> Result<EnvironmentNetwork, BackendError> {
        let allocation = self.networks.lock().unwrap().allocate(id)?;
        match EnvironmentNetwork::create(id, &allocation) {
            Ok(network) => Ok(network),
            Err(failure) => {
                self.networks.lock().unwrap().release(id);
                Err(failure)
            }
        }
    }

    fn release_network(&self, id: &str) {
        self.networks.lock().unwrap().release(id);
    }

    fn write_network_metadata(
        directory: &Path,
        network: &EnvironmentNetwork,
    ) -> Result<(), BackendError> {
        // 网络建立后立即记录，进程异常退出时外部清理仍能定位 netns/veth/子网。
        fs::write(
            directory.join("network"),
            format!(
                "{}\n{}\n{}\n",
                network.namespace, network.veth_host, network.veth_subnet
            ),
        )
        .map_err(error)
    }

    fn write_runtime_metadata(
        directory: &Path,
        network: &EnvironmentNetwork,
        pid: u32,
    ) -> Result<(), BackendError> {
        fs::write(directory.join("pid"), pid.to_string()).map_err(error)?;
        Self::write_network_metadata(directory, network)
    }

    /// 状态目录里记录的进程是否仍然存活。pid 可能被系统复用，因此同时检查
    /// 命令行仍是本环境的 Firecracker。
    fn recorded_environment_process(directory: &Path, environment_id: &str) -> Option<u32> {
        let pid = fs::read_to_string(directory.join("pid"))
            .ok()?
            .trim()
            .parse::<u32>()
            .ok()?;
        let command_line = fs::read(format!("/proc/{pid}/cmdline")).ok()?;
        let arguments: Vec<String> = command_line
            .split(|byte| *byte == 0)
            .filter(|argument| !argument.is_empty())
            .map(|argument| String::from_utf8_lossy(argument).into_owned())
            .collect();
        if !arguments.iter().any(|argument| argument.contains("firecracker")) {
            return None;
        }
        let recorded_id = arguments
            .iter()
            .position(|argument| argument == "--id")
            .and_then(|index| arguments.get(index + 1))?;
        let directory = directory.to_string_lossy();
        (recorded_id == environment_id
            && arguments
                .iter()
                .any(|argument| argument.contains(directory.as_ref())))
        .then_some(pid)
    }

    /// establish/branch 前处理残留目录。没有存活进程的残留直接删除；仍被
    /// 存活 Firecracker 占用时返回清晰错误，避免误删。
    pub(super) fn remove_stale_directory(
        &self,
        directory: &Path,
        environment_id: &str,
    ) -> Result<(), BackendError> {
        if !directory.exists() {
            return Ok(());
        }
        if let Some(pid) = Self::recorded_environment_process(directory, environment_id) {
            return Err(BackendError::new(format!(
                "environment directory {} is still used by live Firecracker process {pid}",
                directory.display()
            )));
        }
        match fs::remove_dir_all(directory) {
            Ok(()) => Ok(()),
            Err(failure) if failure.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(failure) => Err(BackendError::new(format!(
                "failed to remove stale environment directory {}: {failure}",
                directory.display()
            ))),
        }
    }

    pub(super) fn create_rootfs(&self, directory: &Path) -> Result<PathBuf, BackendError> {
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

    /// Create the network, check socket paths, write network metadata and
    /// spawn the firecracker process.  Every failure tears down the partial
    /// start, so callers never observe a half-created Environment.
    fn prepare_environment_start(
        &self,
        id: &str,
        directory: PathBuf,
    ) -> Result<Environment, BackendError> {
        let network = self.create_network(id)?;
        let api_socket = directory.join("api.sock");
        let vsock_socket = directory.join("vsock.sock");
        if let Err(failure) = check_socket_paths(&api_socket, &vsock_socket) {
            network.teardown();
            self.release_network(id);
            return Err(failure);
        }
        if let Err(failure) = Self::write_network_metadata(&directory, &network) {
            network.teardown();
            self.release_network(id);
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
                self.release_network(id);
                return Err(failure);
            }
        };
        Ok(Environment {
            state: EnvironmentState::Available,
            child: Some(child),
            directory,
            api_socket,
            vsock_socket,
            network,
        })
    }

    /// Shared rollback for a started-but-not-fully-configured Environment.
    fn rollback_environment_start(&self, id: &str, environment: &mut Environment) {
        if let Some(child) = environment.child.as_mut() {
            let _ = kill_environment_process(child);
        }
        environment.network.teardown();
        self.release_network(id);
    }

    fn finish_environment_start(
        &self,
        environment: &mut Environment,
    ) -> Result<(), BackendError> {
        let pid = environment.child.as_ref().map(Child::id).unwrap_or_default();
        Self::write_runtime_metadata(&environment.directory, &environment.network, pid)
    }

    pub(super) fn start_fresh(
        &self,
        id: &str,
        directory: PathBuf,
        rootfs: PathBuf,
    ) -> Result<Environment, BackendError> {
        let mut environment = self.prepare_environment_start(id, directory)?;
        if let Err(failure) = (|| {
            wait_for_path(&environment.api_socket, Duration::from_secs(5))?;
            self.api.put(
                &environment.api_socket,
                "/machine-config",
                &json!({
                    "vcpu_count": 2,
                    "mem_size_mib": 512
                }),
            )?;
            self.api.put(
                &environment.api_socket,
                "/boot-source",
                &json!({
                    "kernel_image_path": self.config.kernel_path,
                    "boot_args": "console=ttyS0 reboot=k panic=1 pci=off root=/dev/vda rw"
                }),
            )?;
            self.api.put(
                &environment.api_socket,
                "/drives/rootfs",
                &json!({
                    "drive_id": "rootfs",
                    "path_on_host": rootfs,
                    "is_root_device": true,
                    "is_read_only": false
                }),
            )?;
            self.api.put(
                &environment.api_socket,
                "/network-interfaces/eth0",
                &json!({
                    "iface_id": "eth0",
                    "guest_mac": GUEST_MAC,
                    "host_dev_name": TAP_NAME
                }),
            )?;
            self.api.put(
                &environment.api_socket,
                "/vsock",
                &json!({
                    "guest_cid": 3,
                    "uds_path": environment.vsock_socket
                }),
            )?;
            self.api.put(
                &environment.api_socket,
                "/actions",
                &json!({
                    "action_type": "InstanceStart"
                }),
            )?;
            wait_for_guest(&environment.vsock_socket, Duration::from_secs(20))?;
            configure_guest_network(&environment.vsock_socket)
        })() {
            self.rollback_environment_start(id, &mut environment);
            return Err(failure);
        }
        if let Err(failure) = self.finish_environment_start(&mut environment) {
            self.rollback_environment_start(id, &mut environment);
            return Err(failure);
        }
        Ok(environment)
    }

    pub(super) fn start_from_snapshot(
        &self,
        id: &str,
        directory: PathBuf,
        rootfs: PathBuf,
        snapshot: PathBuf,
        memory: PathBuf,
    ) -> Result<Environment, BackendError> {
        let mut environment = self.prepare_environment_start(id, directory)?;
        if let Err(failure) = (|| {
            wait_for_path(&environment.api_socket, Duration::from_secs(5))?;
            self.api.put(
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
            self.api.patch(
                &environment.api_socket,
                "/drives/rootfs",
                &json!({
                    "drive_id": "rootfs",
                    "path_on_host": rootfs
                }),
            )?;
            self.api.patch(
                &environment.api_socket,
                "/vm",
                &json!({ "state": "Resumed" }),
            )?;
            wait_for_guest(&environment.vsock_socket, Duration::from_secs(20))?;
            configure_guest_network(&environment.vsock_socket)
        })() {
            self.rollback_environment_start(id, &mut environment);
            return Err(failure);
        }
        if let Err(failure) = self.finish_environment_start(&mut environment) {
            self.rollback_environment_start(id, &mut environment);
            return Err(failure);
        }
        Ok(environment)
    }
}

impl Backend for FirecrackerBackend {
    fn capabilities(&self) -> RuntimeCapabilities {
        RuntimeCapabilities {
            branch_environment: true,
            shared_environment: false,
            ..RuntimeCapabilities::default()
        }
    }

    fn establish(&self, environment_id: &str) -> Result<(), BackendError> {
        validate_environment_id(environment_id)?;
        let directory = self.config.state_directory.join(environment_id);
        // Socket 路径长度必须在复制 rootfs 和建立网络之前检查。
        check_socket_paths(
            &directory.join("api.sock"),
            &directory.join("vsock.sock"),
        )?;
        self.remove_stale_directory(&directory, environment_id)?;
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
            Err(mut failure) => {
                if let Err(cleanup) = fs::remove_dir_all(&directory) {
                    failure.add_cleanup_failure(
                        "state_directory",
                        format!(
                            "failed to remove {}: {cleanup}",
                            directory.display()
                        ),
                    );
                }
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
        validate_environment_id(parent)?;
        validate_environment_id(child)?;
        let parent_environment = self.environment(parent)?;
        let child_directory = self.config.state_directory.join(child);
        self.remove_stale_directory(&child_directory, child)?;
        fs::create_dir_all(&child_directory).map_err(error)?;

        let preparation = (|| {
            let child_rootfs = child_directory.join("rootfs.ext4");
            let snapshot = child_directory.join("snapshot.state");
            let memory = child_directory.join("snapshot.memory");
            // 锁内只复制字段，API 调用和整盘复制都在锁外执行。
            let (api_socket, parent_directory, parent_state) = {
                let environment = parent_environment.lock().unwrap();
                (
                    environment.api_socket.clone(),
                    environment.directory.clone(),
                    environment.state,
                )
            };
            let resume_parent = parent_state == EnvironmentState::Available;
            if resume_parent {
                self.api.patch(
                    &api_socket,
                    "/vm",
                    &json!({ "state": "Paused" }),
                )?;
            }
            let snapshot_result = (|| {
                self.api.put(
                    &api_socket,
                    "/snapshot/create",
                    &json!({
                        "snapshot_type": "Full",
                        "snapshot_path": snapshot,
                        "mem_file_path": memory
                    }),
                )?;
                run(Command::new("cp")
                    .args(["--reflink=auto", "--sparse=always"])
                    .arg(parent_directory.join("rootfs.ext4"))
                    .arg(&child_rootfs))?;
                Ok::<_, BackendError>((child_rootfs, snapshot, memory))
            })();
            if resume_parent {
                let resume_result = self
                    .api
                    .patch(&api_socket, "/vm", &json!({ "state": "Resumed" }));
                match (snapshot_result, resume_result) {
                    (Ok(value), Ok(())) => {
                        parent_environment.lock().unwrap().state = EnvironmentState::Available;
                        Ok(value)
                    }
                    (Ok(_), Err(resume)) => {
                        // 父 VM 已暂停但未能恢复；不能把它报告为 Available。
                        parent_environment.lock().unwrap().state = EnvironmentState::Suspended;
                        Err(BackendError::new(format!(
                            "parent resume after snapshot failed: {resume}"
                        )))
                    }
                    (Err(snapshot_failure), Ok(())) => Err(snapshot_failure),
                    (Err(snapshot_failure), Err(resume)) => {
                        parent_environment.lock().unwrap().state = EnvironmentState::Suspended;
                        Err(BackendError::new(format!(
                            "snapshot failed: {snapshot_failure}; parent resume also failed: {resume}"
                        )))
                    }
                }
            } else {
                snapshot_result
            }
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
            Err(mut failure) => {
                if let Err(cleanup) = fs::remove_dir_all(&child_directory) {
                    failure.add_cleanup_failure(
                        "state_directory",
                        format!(
                            "failed to remove {}: {cleanup}",
                            child_directory.display()
                        ),
                    );
                }
                Err(failure)
            }
        }
    }

    fn suspend(&self, environment_id: &str) -> Result<(), BackendError> {
        let environment = self.environment(environment_id)?;
        let (api_socket, already_suspended) = {
            let environment = environment.lock().unwrap();
            (
                environment.api_socket.clone(),
                environment.state == EnvironmentState::Suspended,
            )
        };
        if !already_suspended {
            self.api
                .patch(&api_socket, "/vm", &json!({ "state": "Paused" }))?;
            environment.lock().unwrap().state = EnvironmentState::Suspended;
        }
        Ok(())
    }

    fn resume(&self, environment_id: &str) -> Result<(), BackendError> {
        let environment = self.environment(environment_id)?;
        let (api_socket, already_available) = {
            let environment = environment.lock().unwrap();
            (
                environment.api_socket.clone(),
                environment.state == EnvironmentState::Available,
            )
        };
        if !already_available {
            self.api
                .patch(&api_socket, "/vm", &json!({ "state": "Resumed" }))?;
            environment.lock().unwrap().state = EnvironmentState::Available;
        }
        Ok(())
    }

    fn terminate(&self, environment_id: &str) -> Result<(), BackendError> {
        let environment = self.environment(environment_id)?;
        let child = {
            let mut environment = environment.lock().unwrap();
            environment.state = EnvironmentState::Terminated;
            environment.child.take()
        };
        if let Some(mut child) = child {
            kill_environment_process(&mut child)?;
        }
        Ok(())
    }

    fn cleanup(&self, environment_id: &str) -> Result<(), BackendError> {
        let environment = self.environment(environment_id)?;
        let (directory, network, child) = {
            let mut environment = environment.lock().unwrap();
            (
                environment.directory.clone(),
                environment.network.clone(),
                environment.child.take(),
            )
        };
        let mut failures = Vec::new();
        if let Some(mut child) = child
            && let Err(failure) = kill_environment_process(&mut child)
        {
            failures.push(format!("failed to stop Firecracker process: {failure}"));
        }
        network.teardown();
        match fs::remove_dir_all(&directory) {
            Ok(()) => {}
            Err(failure) if failure.kind() == io::ErrorKind::NotFound => {}
            Err(failure) => failures.push(format!(
                "failed to remove {}: {failure}",
                directory.display()
            )),
        }
        if failures.is_empty() {
            environment.lock().unwrap().state = EnvironmentState::Cleaned;
            self.environments.lock().unwrap().remove(environment_id);
            self.release_network(environment_id);
            Ok(())
        } else {
            // 保留 Terminated 状态，允许调用方重试 cleanup。
            environment.lock().unwrap().state = EnvironmentState::Terminated;
            self.release_network(environment_id);
            Err(BackendError::new(failures.join("; ")))
        }
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
        {
            match child.try_wait() {
                Ok(Some(_)) => {
                    environment.child = None;
                    environment.state = EnvironmentState::Terminated;
                }
                Ok(None) => {}
                Err(failure) => {
                    eprintln!(
                        "firecracker: failed to poll environment {environment_id} process: {failure}"
                    );
                }
            }
        }
        Some(environment.state)
    }

    fn environments(&self) -> Vec<String> {
        self.environments.lock().unwrap().keys().cloned().collect()
    }
}

