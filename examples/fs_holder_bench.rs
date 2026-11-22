//! Time data preparation, cold holder launch, and the first framed read.
use ascout::protocol::contract::AgentLaunch;
use ascout::runtime::backend::{Backend, BackendContext, CancelHandle};
use ascout::runtime::backends::filesystem::{FilesystemBackend, FilesystemConfig};
use ascout::runtime::execution_wire::Message;
use serde_json::json;
use std::{collections::BTreeMap, fs, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir_in("/var/tmp")?;
    let workspace = root.path().join("workspace");
    fs::create_dir(&workspace)?;
    fs::write(workspace.join("first.txt"), "ready")?;
    let mut config = FilesystemConfig::new(root.path().join("state"));
    config.force_overlay = true;
    config.isolated_paths = Some(vec![workspace.clone()]);
    let backend = FilesystemBackend::new(config)?;
    let launch = AgentLaunch {
        command: "/bin/true".into(),
        args: Vec::new(),
        cwd: workspace,
        log: root.path().join("holder.log"),
        instance_id: None,
        env: BTreeMap::new(),
    };
    let iters: u32 = std::env::var("ASCOUT_BENCH_ITERS")
        .ok()
        .map(|v| v.parse())
        .transpose()?
        .unwrap_or(10);
    println!("operation,phase,p50_ms,p95_ms");
    for operation in ["establish", "fork"] {
        let mut samples = [Vec::new(), Vec::new(), Vec::new()];
        for iteration in 0..iters.max(1) {
            let id = format!("{operation}-{iteration}");
            let mut context = BackendContext::new(&id);
            if operation == "fork" {
                context.parent_environment_id = Some("establish-0".into());
            }
            let start = Instant::now();
            if operation == "establish" {
                backend.establish(&id)?;
            } else {
                backend.branch("establish-0", &id)?;
            }
            let data = start.elapsed();
            backend.start_environment_execution(&context, &launch)?;
            let returned = start.elapsed();
            let response = backend.environment_operation(
                &context,
                "read_file",
                &Message::from(json!({"path":"first.txt"})),
                &CancelHandle::default(),
            )?;
            assert_eq!(
                response.meta["value"]["actual_range"]["limit"], 5,
                "{}",
                response.meta
            );
            let complete = start.elapsed();
            for (phase, duration) in [data, returned, complete].into_iter().enumerate() {
                samples[phase].push(duration.as_secs_f64() * 1e3);
            }
            if id != "establish-0" {
                backend.terminate(&id)?;
                backend.cleanup(&id)?;
            }
        }
        for (phase, mut values) in ["data", "returned", "first_read_ready"].into_iter().zip(samples) {
            values.sort_by(f64::total_cmp);
            let percentile = |p: usize| values[(p * values.len()).div_ceil(100) - 1];
            println!(
                "{operation},{phase},{:.3},{:.3}",
                percentile(50),
                percentile(95)
            );
        }
    }
    backend.terminate("establish-0")?;
    backend.cleanup("establish-0")?;
    Ok(())
}
