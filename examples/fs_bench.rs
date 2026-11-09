//! Real filesystem backend benchmark.  Not part of the test suite.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use ascout::protocol::contract::AgentLaunch;
use ascout::runtime::backend::{Backend, BackendContext};
use ascout::runtime::backends::filesystem::{
    FilesystemBackend, FilesystemConfig,
};
use ascout::runtime::launch_plan::MountEntry;

const CHANGED_FILES: usize = 10;

struct Case {
    name: &'static str,
    root: PathBuf,
    state: PathBuf,
    workspace: PathBuf,
    cleanup: Vec<PathBuf>,
    /// Per-fork layout threshold override: `0` forces Chain and `usize::MAX`
    /// forces Flatten.
    flatten_limit: usize,
    isolate_unmanaged: bool,
}

#[derive(Debug)]
enum CaseError {
    Unsupported(String),
    Failed(String),
}

impl From<String> for CaseError {
    fn from(error: String) -> Self {
        Self::Failed(error)
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("fs_bench failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let iterations: usize = std::env::var("ASCOUT_BENCH_ITERS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(20);
    if iterations == 0 {
        return Err("ASCOUT_BENCH_ITERS must be greater than zero".into());
    }
    let sizes: Vec<usize> = std::env::var("ASCOUT_BENCH_SIZES")
        .ok()
        .map(|value| value.split(',').filter_map(|v| v.parse().ok()).collect())
        .unwrap_or_else(|| vec![100, 10_000, 100_000]);
    let pid = std::process::id();
    let project = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let cases = vec![
        Case {
            name: "reflink-flatten",
            root: project.join(format!("target/fs-bench-{pid}-reflink-flatten")),
            state: project.join(format!("target/fs-bench-{pid}-reflink-flatten/state")),
            workspace: project.join(format!("target/fs-bench-{pid}-reflink-flatten/ws")),
            cleanup: vec![project.join(format!("target/fs-bench-{pid}-reflink-flatten"))],
            flatten_limit: usize::MAX,
            isolate_unmanaged: true,
        },
        Case {
            name: "reflink-lower",
            root: project.join(format!("target/fs-bench-{pid}-reflink-lower")),
            state: PathBuf::from(format!("/var/tmp/ascout-fs-bench-state-{pid}")),
            workspace: project.join(format!("target/fs-bench-{pid}-reflink-lower/ws")),
            cleanup: vec![
                project.join(format!("target/fs-bench-{pid}-reflink-lower")),
                PathBuf::from(format!("/var/tmp/ascout-fs-bench-state-{pid}")),
            ],
            flatten_limit: 0,
            isolate_unmanaged: true,
        },
        Case {
            // Forced Chain in the same non-overlapping layout as the forced
            // Flatten case below: state on a separate mount, only the
            // workspace mount participates in launch isolation.
            name: "reflink-chain-nonoverlap",
            root: project.join(format!("target/fs-bench-{pid}-chain-nonoverlap")),
            state: PathBuf::from(format!("/var/tmp/ascout-fs-bench-chain-state-{pid}")),
            workspace: project.join(format!("target/fs-bench-{pid}-chain-nonoverlap/ws")),
            cleanup: vec![
                project.join(format!("target/fs-bench-{pid}-chain-nonoverlap")),
                PathBuf::from(format!("/var/tmp/ascout-fs-bench-chain-state-{pid}")),
            ],
            flatten_limit: 0,
            isolate_unmanaged: false,
        },
        Case {
            name: "reflink-flatten-nonoverlap",
            root: project.join(format!("target/fs-bench-{pid}-flat-nonoverlap")),
            state: PathBuf::from(format!("/var/tmp/ascout-fs-bench-flat-state-{pid}")),
            workspace: project.join(format!("target/fs-bench-{pid}-flat-nonoverlap/ws")),
            cleanup: vec![
                project.join(format!("target/fs-bench-{pid}-flat-nonoverlap")),
                PathBuf::from(format!("/var/tmp/ascout-fs-bench-flat-state-{pid}")),
            ],
            flatten_limit: usize::MAX,
            isolate_unmanaged: false,
        },
        Case {
            name: "tmpfs-flatten",
            root: PathBuf::from(format!("/tmp/ascout-fs-bench-{pid}")),
            state: PathBuf::from(format!("/tmp/ascout-fs-bench-{pid}/state")),
            workspace: PathBuf::from(format!("/tmp/ascout-fs-bench-{pid}/ws")),
            cleanup: vec![PathBuf::from(format!("/tmp/ascout-fs-bench-{pid}"))],
            flatten_limit: usize::MAX,
            isolate_unmanaged: true,
        },
        Case {
            name: "btrfs-snapshot-flatten",
            root: project.join(format!("target/fs-bench-{pid}-snapshot-flatten")),
            state: project.join(format!("target/fs-bench-{pid}-snapshot-flatten/state")),
            workspace: project.join(format!("target/fs-bench-{pid}-snapshot-flatten/ws")),
            cleanup: vec![project.join(format!("target/fs-bench-{pid}-snapshot-flatten"))],
            flatten_limit: usize::MAX,
            isolate_unmanaged: true,
        },
        Case {
            name: "btrfs-snapshot-lower",
            root: project.join(format!("target/fs-bench-{pid}-snapshot-lower")),
            state: PathBuf::from(format!("/var/tmp/ascout-fs-bench-snapshot-state-{pid}")),
            workspace: project.join(format!("target/fs-bench-{pid}-snapshot-lower/ws")),
            cleanup: vec![
                project.join(format!("target/fs-bench-{pid}-snapshot-lower")),
                PathBuf::from(format!("/var/tmp/ascout-fs-bench-snapshot-state-{pid}")),
            ],
            flatten_limit: 0,
            isolate_unmanaged: true,
        },
    ];

    let execution = (|| -> Result<(), String> {
    println!("machine:");
    println!("  kernel: {}", uname("-r"));
    println!("  cpu: {}", cpu_model());
    for case in &cases {
        fs::create_dir_all(&case.root).map_err(|e| format!("create root: {e}"))?;
        fs::create_dir_all(&case.state).map_err(|e| format!("create state: {e}"))?;
        fs::create_dir_all(&case.workspace).map_err(|e| format!("create workspace: {e}"))?;
        let fs_type = Command::new("stat")
            .args(["-f", "-c", "%T"])
            .arg(&case.state)
            .output()
            .ok()
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
            .unwrap_or_else(|| "unknown".to_string());
        println!("  case {} state_fs={fs_type}", case.name);
    }

    println!("\ncase,N,operation,p50_ms,p95_ms,fork_cost,mechanism,overlaps");
    let filter = std::env::var("ASCOUT_BENCH_CASES").unwrap_or_default();
    for case in &cases {
        if !filter.is_empty() && !case.name.contains(&filter) {
            continue;
        }
        for &n in &sizes {
            if let Err(error) = run_case(case, n, iterations) {
                match error {
                CaseError::Unsupported(error) => {
                    println!("{},{},{},SKIP: {},,,", case.name, n, "all", error);
                }
                CaseError::Failed(error) => {
                    return Err(format!("{} N={n}: {error}", case.name));
                }
                }
            }
        }
    }
    Ok(())
    })();
    let cleanup_errors = cleanup_paths(cases.iter().flat_map(|case| case.cleanup.iter()));
    match (execution, cleanup_errors.is_empty()) {
        (result, true) => result,
        (result, false) => Err(format!("{}cleanup failed: {}",
            result.err().map(|e| format!("{e}; ")).unwrap_or_default(),
            cleanup_errors.join("; "))),
    }
}

fn cleanup_paths<'a>(paths: impl Iterator<Item = &'a PathBuf>) -> Vec<String> {
    let mut cleanup_errors = Vec::new();
        for path in paths {
            if let Err(error) = fs::remove_dir_all(path) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    cleanup_errors.push(format!("remove {}: {error}", path.display()));
                }
            }
        }
    cleanup_errors
}

fn run_case(case: &Case, n: usize, iterations: usize) -> Result<(), CaseError> {
    let mut config = FilesystemConfig::new(&case.state);
    config.force_overlay = !case.name.starts_with("btrfs-snapshot");
    config.isolate_unmanaged_mounts = case.isolate_unmanaged;
    config.fork_flatten_entry_limit = case.flatten_limit;
    // Benchmark layouts deliberately place the state directory under an
    // isolated mount (including the *-flatten cases); production must not.
    config.allow_overlapping_state_directory = true;
    let backend = FilesystemBackend::new(config).map_err(|e| e.to_string())?;
    let capabilities = backend.capabilities();
    if !capabilities.branch_environment {
        return Err(CaseError::Unsupported("bubblewrap overlay probe failed".into()));
    }
    let overlaps = capabilities
        .state_directory_overlaps_mount
        .clone()
        .unwrap_or_default();
    let fork_cost = capabilities
        .filesystem_fork_cost
        .clone()
        .unwrap_or_else(|| "unknown".into());
    if case.name.starts_with("btrfs-snapshot") && fork_cost != "btrfs_snapshot" {
        return Err(CaseError::Unsupported("btrfs snapshot copies unavailable".into()));
    }
    let mechanism = backend.mechanism().as_str().to_string();
    let cwd = case.workspace.clone();
    let mut total = BTreeMap::<&'static str, Vec<Duration>>::new();
    backend.establish("root").map_err(|e| e.to_string())?;
    let root_upper = workspace_upper(&backend, "root", &cwd)?;
    for index in 0..n {
        write_bench_file(&root_upper, index)?;
    }

    for iteration in 0..iterations {
        let child = format!("child-{iteration}");
        let mark = format!("mark-{iteration}");
        let fork_start = Instant::now();
        backend.fork("root", &child, &[]).map_err(|e| e.to_string())?;
        total.entry("fork").or_default().push(fork_start.elapsed());
        let base = FilesystemBackend::base_snapshot_id_for_child(&child);
        let mark_start = Instant::now();
        backend
            .mark(&child, &mark, &[])
            .map_err(|e| e.to_string())?;
        total.entry("mark").or_default().push(mark_start.elapsed());
        let child_upper = workspace_upper(&backend, &child, &cwd)?;
        for index in 0..CHANGED_FILES.min(n) {
            write_bench_file(&child_upper, index)?;
        }
        let diff_start = Instant::now();
        backend
            .diff(&base, &child, &[])
            .map_err(|e| e.to_string())?;
        total.entry("diff").or_default().push(diff_start.elapsed());
        backend.suspend("root").map_err(|e| e.to_string())?;
        let accept_start = Instant::now();
        let accepted = backend
            .accept(&base, &child, "root", &[], None, None)
            .map_err(|e| e.to_string())?;
        total.entry("accept").or_default().push(accept_start.elapsed());
        if !accepted.conflicts.is_empty() {
            return Err(format!("accept conflicts: {:?}", accepted.conflicts).into());
        }
        backend.resume("root").map_err(|e| e.to_string())?;
        backend.terminate(&child).map_err(|e| e.to_string())?;
        let discard_start = Instant::now();
        backend.discard(&child, &[]).map_err(|e| e.to_string())?;
        total.entry("discard").or_default().push(discard_start.elapsed());
    }
    backend.terminate("root").map_err(|e| e.to_string())?;
    backend.cleanup("root").map_err(|e| e.to_string())?;
    let overlaps_text = if overlaps.is_empty() {
        "-".to_string()
    } else {
        overlaps.join("|")
    };
    for (operation, samples) in total {
        let (p50, p95) = percentile(&samples);
        println!(
            "{},{},{},{:.3},{:.3},{},{},{}",
            case.name,
            n,
            operation,
            p50.as_secs_f64() * 1000.0,
            p95.as_secs_f64() * 1000.0,
            fork_cost,
            mechanism,
            overlaps_text
        );
    }
    Ok(())
}

fn workspace_upper(
    backend: &FilesystemBackend,
    environment: &str,
    cwd: &Path,
) -> Result<PathBuf, String> {
    let context = BackendContext::new(environment);
    let launch = AgentLaunch {
        command: "/bin/true".into(),
        args: Vec::new(),
        cwd: cwd.to_path_buf(),
        log: cwd.join("bench.log"),
        instance_id: None,
        env: BTreeMap::new(),
    };
    let fragment = backend
        .launch_fragment(&context, &launch)
        .map_err(|e| e.to_string())?;
    for entry in fragment.mounts {
        if let MountEntry::Overlay { target, upper, .. } = entry {
            if cwd.starts_with(&target) {
                return Ok(upper);
            }
        }
    }
    Err(format!("no overlay mount for {}", cwd.display()))
}

fn write_bench_file(upper: &Path, index: usize) -> Result<(), String> {
    let path = upper.join(format!("file-{index:06}"));
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut data = vec![b'x'; 4096];
    data[0] = (index % 251) as u8;
    fs::write(path, data).map_err(|e| e.to_string())
}

fn percentile(samples: &[Duration]) -> (Duration, Duration) {
    let mut sorted = samples.to_vec();
    sorted.sort();
    let index = |p: f64| ((sorted.len() - 1) as f64 * p).round() as usize;
    (sorted[index(0.50)], sorted[index(0.95)])
}

fn uname(flag: &str) -> String {
    Command::new("uname")
        .arg(flag)
        .output()
        .ok()
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn cpu_model() -> String {
    fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|text| {
            text.lines()
                .find(|line| line.starts_with("model name"))
                .and_then(|line| line.split_once(':'))
                .map(|(_, value)| value.trim().to_string())
        })
        .unwrap_or_else(|| "unknown".to_string())
}
