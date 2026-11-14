//! Per-Branch tool launch plans. Pi owns sessions, not their overlay mounts.
use super::*;
use serde_json::json;

impl FilesystemBackend {
    pub(super) fn persist_tool_plan(&self, environment: &Environment) -> Result<(), BackendError> {
        let mut args: Vec<String> = vec!["--die-with-parent".into(), "--unshare-user".into(), "--unshare-pid".into(), "--as-pid-1".into(), "--ro-bind".into(), "/".into(), "/".into(), "--proc".into(), "/proc".into(), "--dev".into(), "/dev".into()];
        for path in &self.config.passthrough_rw {
            if path.exists() {args.extend(["--bind".into(), path.display().to_string(), path.display().to_string()]);}
        }
        for mount in environment.mounts.values() {
            for lower in self.launch_lowers(mount) {args.extend(["--overlay-src".into(), lower.display().to_string()]);}
            args.extend(["--overlay".into(), mount.upper.display().to_string(), mount.work.display().to_string(), mount.target.display().to_string()]);
        }
        args.extend(["--tmpfs".into(), self.config.state_directory.display().to_string(), "--remount-ro".into(), self.config.state_directory.display().to_string()]);
        self.write_json(&self.config.state_directory.join("tool-plans").join(format!("{}.json", environment.id)), &json!({"program":self.config.bubblewrap_path,"args":args}))
    }
}
