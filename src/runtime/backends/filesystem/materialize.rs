use super::*;
use std::os::fd::AsRawFd;

fn sync_filesystem(path: &Path) -> Result<(), BackendError> {
    let file = fs::File::open(path).map_err(error)?;
    if unsafe { libc::syncfs(file.as_raw_fd()) } != 0 {
        return Err(error(std::io::Error::last_os_error()));
    }
    Ok(())
}

impl FilesystemBackend {
    pub(super) fn materialize_authority(&self, id: &str) -> Result<FilesystemResult, BackendError> {
        let environment = Self::resolve_environment(&*self.lock()?, id)?;
        let python = if Path::new("/usr/bin/python3").is_file() {
            "/usr/bin/python3"
        } else {
            "python3"
        };
        for mount in environment.mounts.values() {
            let package = environment.root.join(format!(
                "publication-{}",
                short_hash(&mount.target.display().to_string())
            ));
            if !package.join("ready").exists() {
                if package.exists() {
                    remove_directory_force(&package)?;
                }
                fs::create_dir(&package).map_err(error)?;
                let mut selected = BTreeMap::<String, bool>::new();
                for layer in mount
                    .lower
                    .iter()
                    .map(|layer| mount.layer_upper(layer))
                    .chain(std::iter::once(mount.upper.clone()))
                {
                    for (path, node) in read_layer_if_exists(&layer)? {
                        // Later copy-ups cannot narrow an earlier opaque/redirect subtree.
                        *selected.entry(path).or_default() |=
                            node.opaque || node.redirect.is_some();
                    }
                }
                let paths: Vec<_> = selected
                    .iter()
                    .map(|(path, subtree)| serde_json::json!({"path":path,"subtree":subtree}))
                    .collect();
                fs::write(
                    package.join("selection.json"),
                    serde_json::to_vec(&paths).map_err(error)?,
                )
                .map_err(error)?;
                let mut command = Command::new(&self.config.bubblewrap_path);
                command.args([
                    "--die-with-parent",
                    "--unshare-user",
                    "--ro-bind",
                    "/",
                    "/",
                    "--dev",
                    "/dev",
                ]);
                command.env("PYTHONHASHSEED", "0");
                let view = if mount.target.is_dir() {
                    mount.target.clone()
                } else {
                    let view = environment.root.join(format!(
                        "publication-view-{}",
                        short_hash(&mount.target.display().to_string())
                    ));
                    fs::create_dir_all(&view).map_err(error)?;
                    view
                };
                for lower in self.launch_lowers(mount) {
                    command.arg("--overlay-src").arg(lower);
                }
                command
                    .arg("--overlay-src")
                    .arg(&mount.upper)
                    .arg("--ro-overlay")
                    .arg(&view);
                command.arg("--bind").arg(&package).arg(&package);
                command
                    .args(["--", python, "-c", include_str!("materialize.py"), "export"])
                    .arg(&view)
                    .arg(&package);
                checked(command, "read Authority delta final view")?;
            }
        }
        // Export all affected mounts before changing any host lower.
        // Keep receipts so a partial publication can retry.
        for mount in environment.mounts.values() {
            let package = environment.root.join(format!(
                "publication-{}",
                short_hash(&mount.target.display().to_string())
            ));
            let destination = mount.base.as_deref().unwrap_or(&mount.target);
            let mut command = Command::new(python);
            command
                .env("PYTHONHASHSEED", "0")
                .args(["-c", include_str!("materialize.py"), "publish"])
                .arg(destination)
                .arg(&package);
            checked(command, "publish Authority delta")?;
            sync_filesystem(destination)?;
        }
        // Keep the old uppers and package until both metadata and the new plan are durable.
        let mut next = environment.clone();
        next.root =
            self.environments_root()
                .join(format!("{}-task-{}", environment.id, now_nanos()));
        next.base_snapshot_id = None;
        next.parent_snapshot_id = None;
        next.marks.clear();
        for mount in next.mounts.values_mut() {
            mount.saved_base = None;
            mount.lower.clear();
            let volume = next.root.join(&mount.layer_key);
            if self.uses_btrfs_storage() {
                fs::create_dir_all(volume.parent().unwrap()).map_err(error)?;
                self.btrfs_create(&volume)?;
            }
            mount.upper = volume.join("upper");
            mount.work = volume.join("work");
        }
        Self::prepare_mount_dirs(&next.mounts.values().cloned().collect::<Vec<_>>())?;
        // Retain the old generation before installing the new plan. Its receipts
        // let a retry skip exporting the already-modified host.
        let mut state = self.lock()?;
        self.retain_retired_root(&mut state, &environment.root)?;
        if let Err(failure) = self
            .persist_environment(&next)
            .and_then(|()| sync_filesystem(&self.config.state_directory))
        {
            // A plan write can fail after metadata is installed.
            self.write_json(
                &self.environment_metadata_path(id),
                &EnvironmentMetadata::from_environment(&environment),
            )?;
            self.persist_tool_plan(&environment)?;
            sync_filesystem(&self.config.state_directory)?;
            return Err(failure);
        }
        state.environments.insert(next.id.clone(), next);
        // GC failure after commit must not trigger publication of a new delta.
        let garbage = self.refresh_references(&mut state)?;
        let referenced = Self::root_referenced(&state, &environment.root);
        drop(state);
        if let Err(failure) = self
            .persist_all_snapshots()
            .and_then(|_| self.schedule_gc(garbage))
        {
            eprintln!("filesystem: publication succeeded; deferred snapshot cleanup: {failure}");
        } else if !referenced {
            if let Err(failure) = self.retire_path(&environment.root) {
                eprintln!("filesystem: publication succeeded; retained old root: {failure}");
            }
        }
        let mut result = FilesystemResult::default();
        result.confirmation.published = true;
        Ok(result)
    }
}

fn checked(mut command: Command, operation: &str) -> Result<(), BackendError> {
    let output = command.output().map_err(error)?;
    if !output.status.success() {
        return Err(BackendError::new(format!(
            "{operation}: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    // Re-execute only this test so the poisoned PATH cannot race other tests.
    #[test]
    fn materialization_ignores_workspace_python() {
        const CHILD: &str = "ASCOUT_TEST_MATERIALIZE_POISONED_PATH";
        let directory = tempfile::tempdir().unwrap();
        if std::env::var_os(CHILD).is_none() {
            let python = directory.path().join("python3");
            fs::write(
                &python,
                "#!/bin/sh\necho 'unexpected workspace python' >&2\nexit 91\n",
            )
            .unwrap();
            fs::set_permissions(&python, fs::Permissions::from_mode(0o755)).unwrap();
            let path = std::env::join_paths(
                std::iter::once(directory.path().to_path_buf())
                    .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
            )
            .unwrap();
            let name = module_path!().split_once("::").unwrap().1;
            let output = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    &format!("{name}::materialization_ignores_workspace_python"),
                ])
                .env(CHILD, "1")
                .env("PATH", path)
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                output.status.success() && stdout.contains("1 passed"),
                "{stdout}\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        let workspace = directory.path().join("workspace");
        fs::create_dir(&workspace).unwrap();
        fs::write(workspace.join("sample.txt"), "before").unwrap();
        let mut config = FilesystemConfig::new(directory.path().join("state"));
        config.force_overlay = true;
        config.isolated_paths = Some(vec![workspace.clone()]);
        let backend = FilesystemBackend::new(config).unwrap();
        backend.establish("authority").unwrap();
        let env =
            FilesystemBackend::resolve_environment(&*backend.lock().unwrap(), "authority").unwrap();
        let mount = env.mounts.values().next().unwrap();
        fs::write(mount.upper.join("sample.txt"), "after").unwrap();
        assert!(
            backend
                .materialize_authority("authority")
                .unwrap()
                .confirmation
                .published
        );
        assert_eq!(
            fs::read_to_string(workspace.join("sample.txt")).unwrap(),
            "after"
        );
    }
}
