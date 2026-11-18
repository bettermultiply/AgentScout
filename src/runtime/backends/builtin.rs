//! Declarations for the implementations compiled into this binary.

use std::{error::Error, path::PathBuf, sync::Arc};

use serde_json::Value;

use super::{
    filesystem::{FilesystemBackend, FilesystemConfig},
    firecracker::{FirecrackerBackend, FirecrackerConfig},
    nle::{NleBackend, NleConfig},
};
use crate::{
    protocol::contract::RuntimeCapabilities,
    runtime::backend_registry::{BackendImplementation, BackendInterface},
};

pub fn resolve(kind: &str, config: &Value) -> Result<BackendImplementation, Box<dyn Error>> {
    use BackendInterface::{Environment, Filesystem};
    let mut interfaces = vec![Environment];
    let (capabilities, create): (_, Box<dyn FnOnce() -> _>) =
        match kind {
            "nle" => {
                let config: NleConfig = serde_json::from_value(config.clone())?;
                (
                    NleBackend::capability_declaration(),
                    Box::new(move || Ok(Arc::new(NleBackend::new(config)?) as _)),
                )
            }
            "firecracker" => {
                let config: FirecrackerConfig = serde_json::from_value(config.clone())?;
                (
                    RuntimeCapabilities {
                        branch_environment: true,
                        ..Default::default()
                    },
                    Box::new(move || Ok(Arc::new(FirecrackerBackend::new(config)?) as _)),
                )
            }
            "filesystem" => {
                interfaces.push(Filesystem);
                let config = filesystem_config(config)?;
                // Usability and performance depend on probes during construction.
                let capabilities = RuntimeCapabilities {
                    filesystem_passthrough: Some(
                        config
                            .passthrough_rw
                            .iter()
                            .map(|p| p.display().to_string())
                            .collect(),
                    ),
                    ..Default::default()
                };
                (
                    capabilities,
                    Box::new(move || Ok(Arc::new(FilesystemBackend::new(config)?) as _)),
                )
            }
            other => return Err(format!(
                "unknown runtime backend type {other}; external backend loading is not configured"
            )
            .into()),
        };
    Ok(BackendImplementation {
        interfaces,
        capabilities,
        create,
    })
}

fn filesystem_config(config: &Value) -> Result<FilesystemConfig, Box<dyn Error>> {
    let state = config
        .get("state_directory")
        .and_then(|v| v.as_str())
        .ok_or("filesystem missing state_directory")?;
    let mut filesystem_config = FilesystemConfig::new(state);
    if let Some(paths) = config.get("passthrough_rw") {
        filesystem_config.passthrough_rw = serde_json::from_value(paths.clone())?;
    }
    if let Some(paths) = config.get("isolated_paths") {
        let entries = paths
            .as_array()
            .ok_or("filesystem isolated_paths must be an array")?;
        let mut isolated = Vec::new();
        let mut targets = Vec::new();
        for entry in entries {
            let (path, target) = match entry {
                serde_json::Value::String(path) => (PathBuf::from(path), None),
                serde_json::Value::Object(object) => {
                    let path = object
                        .get("path")
                        .and_then(|value| value.as_str())
                        .ok_or("filesystem isolated_paths entry missing path")?;
                    let target = object
                        .get("target")
                        .and_then(|value| value.as_str())
                        .map(PathBuf::from);
                    (PathBuf::from(path), target)
                }
                _ => {
                    return Err(
                        "filesystem isolated_paths entries must be strings or objects".into(),
                    );
                }
            };
            if !path.is_absolute() || path == std::path::Path::new("/") || !path.is_dir() {
                return Err(
                    "filesystem isolated_paths must contain existing absolute non-root directories"
                        .into(),
                );
            }
            if let Some(target) = target {
                if !target.is_absolute() || target == std::path::Path::new("/") {
                    return Err(
                        "filesystem isolated_paths target must be an absolute non-root path".into(),
                    );
                }
                targets.push((path.clone(), target));
            }
            isolated.push(path);
        }
        if isolated.is_empty() {
            return Err(
                "filesystem isolated_paths must contain existing absolute non-root directories"
                    .into(),
            );
        }
        filesystem_config.isolated_paths = Some(isolated);
        filesystem_config.isolated_targets = targets;
    }
    if let Some(path) = config.get("bubblewrap_path").and_then(|v| v.as_str()) {
        filesystem_config.bubblewrap_path = PathBuf::from(path);
    }
    Ok(filesystem_config)
}
