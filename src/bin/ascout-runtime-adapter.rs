use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use ascout::adapter::Adapter;
use ascout::firecracker::{FirecrackerBackend, FirecrackerConfig};
use ascout::nle::{NleBackend, NleConfig};

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
    if value.get("backend").and_then(|value| value.as_str()) == Some("nle") {
        let config: NleConfig = serde_json::from_value(value)?;
        let control_socket = config.control_socket.clone();
        let adapter_instance = config.adapter_instance.clone();
        Adapter {
            adapter_instance,
            backend: Arc::new(NleBackend::new(config)?),
        }
        .run(&control_socket)?;
    } else {
        let config: FirecrackerConfig = serde_json::from_value(value)?;
        let control_socket = config.control_socket.clone();
        let adapter_instance = config.adapter_instance.clone();
        Adapter {
            adapter_instance,
            backend: Arc::new(FirecrackerBackend::new(config)?),
        }
        .run(&control_socket)?;
    }
    Ok(())
}
