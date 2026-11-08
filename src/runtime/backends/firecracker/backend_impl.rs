//! FirecrackerBackend 的环境生命周期与 Backend 实现。

use super::*;
use super::guest_ops::*;
use super::network::*;

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

    pub(super) fn environment(&self, id: &str) -> Result<Arc<Mutex<Environment>>, BackendError> {
        self.environments
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| BackendError::new("unknown environment"))
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

    pub(super) fn start_fresh(
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

    pub(super) fn start_from_snapshot(
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

