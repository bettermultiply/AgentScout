use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use ascout::runtime::adapter::Adapter;
use ascout::runtime::backend::Backend;
use ascout::runtime::backends::filesystem::{FilesystemBackend, FilesystemConfig};
use ascout::runtime::backends::firecracker::FirecrackerBackend;
use ascout::runtime::backends::nle::NleBackend;
use ascout::runtime::{BackendConnection, BackendPolicy, BaseRuntime, CompositeRuntime};

fn main() {
    if let Err(error) = run() {
        eprintln!("ascout-runtime-adapter: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args_os().skip(1);
    if arguments.next().as_deref() != Some(std::ffi::OsStr::new("--config")) {
        return Err("usage: ascout-runtime-adapter --config <path>".into());
    }
    let path = PathBuf::from(arguments.next().ok_or("missing config path")?);
    if arguments.next().is_some() {
        return Err("unexpected command-line argument".into());
    }
    let bytes = fs::read(path)?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;

    let control_socket = PathBuf::from(
        value
            .get("control_socket")
            .and_then(|value| value.as_str())
            .ok_or("runtime config missing control_socket")?,
    );
    let adapter_instance = value
        .get("adapter_instance")
        .and_then(|value| value.as_str())
        .ok_or("runtime config missing adapter_instance")?
        .to_string();

    // `backends` 数组存在时按多 Backend 配置装配；否则走旧单 Backend 配置，
    // 保持既有行为不变。
    let filesystem = if let Some(config) = value.get("filesystem") {
        let state = config
            .get("state_directory")
            .and_then(|v| v.as_str())
            .ok_or("filesystem missing state_directory")?;
        let mut filesystem_config = FilesystemConfig::new(state);
        if let Some(path) = config.get("bubblewrap_path").and_then(|v| v.as_str()) {
            filesystem_config.bubblewrap_path = PathBuf::from(path);
        }
        Some(Arc::new(FilesystemBackend::new(filesystem_config)?))
    } else {
        None
    };
    let runtime: Arc<dyn Backend> = if value.get("backends").is_some() {
        assemble_runtime(&value, filesystem.clone())?
    } else {
        assemble_single_backend(&value, filesystem.clone())?
    };

    Adapter {
        adapter_instance,
        runtime,
    }
    .run(&control_socket)?;
    Ok(())
}

/// 按配置装配多个 Backend 连接，用 CompositeRuntime 组合，并可选地包一层
/// BaseRuntime 管理单个 Agent 实际执行实例的进程。Agent Session 仍由接入侧管理。
fn assemble_runtime(
    value: &serde_json::Value,
    filesystem: Option<Arc<FilesystemBackend>>,
) -> Result<Arc<dyn Backend>, Box<dyn std::error::Error>> {
    let entries = value
        .get("backends")
        .and_then(|value| value.as_array())
        .ok_or("runtime config backends must be an array")?;
    let mut connections = Vec::new();
    for entry in entries {
        let id = entry
            .get("id")
            .and_then(|value| value.as_str())
            .ok_or("runtime backend entry missing id")?;
        let backend_type = entry
            .get("type")
            .and_then(|value| value.as_str())
            .ok_or("runtime backend entry missing type")?;
        let config = entry
            .get("config")
            .ok_or("runtime backend entry missing config")?;
        let backend: Arc<dyn Backend> = match backend_type {
            "nle" => Arc::new(NleBackend::new(serde_json::from_value(config.clone())?)?),
            "firecracker" => Arc::new(FirecrackerBackend::new(serde_json::from_value(
                config.clone(),
            )?)?),
            other => return Err(format!("unknown runtime backend type {other}").into()),
        };
        connections.push(BackendConnection::new(id, backend)?);
    }
    if connections.is_empty() {
        return Err("runtime config backends must not be empty".into());
    }

    // Policy 是唯一的选择来源。只有单个 Backend 且未显式给出 policy 时，才回退
    // 到“全部路由给它”；多个 Backend 必须显式给出 policy，否则装配失败，绝不
    // 因为“存在多个连接”就默认广播。
    let policy = match value.get("policy") {
        Some(policy) => serde_json::from_value(policy.clone())?,
        None if connections.len() == 1 => BackendPolicy::single(connections[0].id.clone()),
        None => BackendPolicy::default(),
    };
    let composite = CompositeRuntime::new("runtime", connections, policy)?;

    let runtime: Arc<dyn Backend> = match value
        .get("base_runtime")
        .and_then(|value| value.as_str())
        .or_else(|| filesystem.as_ref().map(|_| "runtime"))
    {
        Some(base_id) => {
            let base = BaseRuntime::new(base_id, Arc::new(composite));
            match filesystem {
                Some(fs) => {
                    let bwrap = fs.bubblewrap_path().to_path_buf();
                    Arc::new(base.with_filesystem(fs).with_bubblewrap(bwrap))
                }
                None => Arc::new(base),
            }
        }
        None => Arc::new(composite),
    };
    Ok(runtime)
}

/// 旧单 Backend 配置兼容：没有 `backends` 数组时，按原来的 backend/nle 或
/// Firecracker 字段构造一个 Backend，再包成单连接的 CompositeRuntime，行为不变。
fn assemble_single_backend(
    value: &serde_json::Value,
    filesystem: Option<Arc<FilesystemBackend>>,
) -> Result<Arc<dyn Backend>, Box<dyn std::error::Error>> {
    let is_nle = value.get("backend").and_then(|value| value.as_str()) == Some("nle");
    let (id, backend): (&str, Arc<dyn Backend>) = if is_nle {
        (
            "nle",
            Arc::new(NleBackend::new(serde_json::from_value(value.clone())?)?),
        )
    } else {
        (
            "firecracker",
            Arc::new(FirecrackerBackend::new(serde_json::from_value(
                value.clone(),
            )?)?),
        )
    };
    let composite = CompositeRuntime::new(
        "runtime",
        vec![BackendConnection::new(id, backend)?],
        BackendPolicy::single(id),
    )?;
    if let Some(filesystem) = filesystem {
        let bwrap = filesystem.bubblewrap_path().to_path_buf();
        Ok(Arc::new(
            BaseRuntime::new("runtime", Arc::new(composite))
                .with_filesystem(filesystem)
                .with_bubblewrap(bwrap),
        ))
    } else {
        Ok(Arc::new(composite))
    }
}
