//! Per-Branch tool launch plans. Pi owns sessions, not their overlay mounts.
use super::*;
use serde_json::json;

impl FilesystemBackend {
    pub(super) fn persist_tool_plan(&self, environment: &Environment) -> Result<(), BackendError> {
        let mut args: Vec<String> = vec![
            "--die-with-parent".into(),
            "--unshare-user".into(),
            "--unshare-pid".into(),
            "--as-pid-1".into(),
            "--ro-bind".into(),
            "/".into(),
            "/".into(),
            "--proc".into(),
            "/proc".into(),
            "--dev".into(),
            "/dev".into(),
        ];
        for path in &self.config.passthrough_rw {
            if path.exists() {
                args.extend([
                    "--bind".into(),
                    path.display().to_string(),
                    path.display().to_string(),
                ]);
            }
        }
        for mount in environment.mounts.values() {
            for lower in self.launch_lowers(mount) {
                args.extend(["--overlay-src".into(), lower.display().to_string()]);
            }
            args.extend([
                "--overlay".into(),
                mount.upper.display().to_string(),
                mount.work.display().to_string(),
                mount.target.display().to_string(),
            ]);
        }
        args.extend([
            "--tmpfs".into(),
            self.config.state_directory.display().to_string(),
            "--remount-ro".into(),
            self.config.state_directory.display().to_string(),
        ]);
        let socket = self
            .config
            .state_directory
            .join("holder-sockets")
            .join(&environment.id)
            .join("cmd.sock");
        // Tools run inside the branch namespace, where an isolated workspace
        // may be mounted at a different path than its host directory. Pi
        // resolves relative tool paths against this sandbox path; the host
        // directory remains only the overlay lower and publication target.
        let (host_workspace, workspace) = match environment.workspace_root.as_deref() {
            Some(workspace) => {
                // The workspace may be a subdirectory of the isolated mount.
                // Pi resolves tool paths against the workspace, so the host
                // side of the mapping is the host path at the same depth, not
                // the mount's bottom directory.
                let host = environment.mount_for_path(workspace).and_then(|mount| {
                    mount.base.as_ref().map(|base| {
                        let relative = workspace
                            .strip_prefix(&mount.target)
                            .unwrap_or_else(|_| Path::new(""));
                        if relative.as_os_str().is_empty() {
                            base.clone()
                        } else {
                            base.join(relative)
                        }
                    })
                });
                (host, Some(workspace.to_path_buf()))
            }
            None => (None, None),
        };
        self.write_json(&self.config.state_directory.join("tool-plans").join(format!("{}.json", environment.id)), &json!({"program":self.config.bubblewrap_path,"args":args,"holder_socket":socket,"workspace":{"host":host_workspace,"target":workspace}}))
    }
}
