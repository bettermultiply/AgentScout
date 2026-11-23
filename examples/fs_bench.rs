use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use ascout::protocol::contract::AgentLaunch;
use ascout::runtime::backend::{Backend, BackendContext};
use ascout::runtime::backends::filesystem::{FilesystemBackend, FilesystemConfig};
use ascout::runtime::launch_plan::MountEntry;

#[path = "../src/runtime/backends/filesystem/scratch_dir.rs"]
mod scratch_dir;
use scratch_dir::{ScratchDir, mount_id};

const CHANGED_FILES: usize = 10;

struct Case {
    name: &'static str,
    base: &'static str,
    separate_state: bool,
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
    let cases = [
        ("reflink-flatten", "/var/tmp", false, usize::MAX, true),
        ("reflink-lower", "/var/tmp", true, 0, true),
        ("reflink-chain-nonoverlap", "/var/tmp", true, 0, false),
        (
            "reflink-flatten-nonoverlap",
            "/var/tmp",
            true,
            usize::MAX,
            false,
        ),
        ("tmpfs-flatten", "/tmp", false, usize::MAX, true),
        (
            "btrfs-snapshot-flatten",
            "/var/tmp",
            false,
            usize::MAX,
            true,
        ),
        ("btrfs-snapshot-lower", "/var/tmp", true, 0, true),
    ]
    .map(
        |(name, base, separate_state, flatten_limit, isolate_unmanaged)| Case {
            name,
            base,
            separate_state,
            flatten_limit,
            isolate_unmanaged,
        },
    );

    let source = fs::canonicalize(env!("CARGO_MANIFEST_DIR"))
        .map_err(|error| format!("resolve source directory: {error}"))?;
    let state_base = std::env::var_os("ASCOUT_PRIV_STATE_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    println!("machine:");
    println!("  kernel: {}", uname("-r"));
    println!("  cpu: {}", cpu_model());
    println!("\ncase,N,operation,p50_ms,p95_ms,fork_cost,mechanism,overlaps");
    let filter = std::env::var("ASCOUT_BENCH_CASES").unwrap_or_default();
    for case in &cases {
        if !filter.is_empty() && !case.name.contains(&filter) {
            continue;
        }
        for &n in &sizes {
            match with_case_dirs(case, &source, state_base.as_deref(), n, iterations) {
                Ok(()) => (),
                Err(CaseError::Unsupported(error)) => {
                    println!("{},{n},all,SKIP: {error},,,,", case.name);
                }
                Err(CaseError::Failed(error)) => {
                    return Err(format!("{} N={n}: {error}", case.name));
                }
            }
        }
    }
    Ok(())
}

fn outside_source(path: &Path, source: &Path) -> Result<PathBuf, CaseError> {
    let resolved =
        fs::canonicalize(path).map_err(|error| format!("resolve {}: {error}", path.display()))?;
    if resolved.starts_with(source) {
        return Err(format!("{} resolves inside the source tree", path.display()).into());
    }
    Ok(resolved)
}

fn fs_type(path: &Path) -> Option<String> {
    let output = Command::new("stat")
        .args(["-f", "-c", "%T"])
        .arg(path)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn separate_state_dir(
    case: &Case,
    workspace_base: &Path,
    source: &Path,
    state_base: Option<&Path>,
) -> Result<ScratchDir, CaseError> {
    let workspace_mount = mount_id(workspace_base).map_err(|error| error.to_string())?;
    let prefix = "ascout-fs-bench-state-";
    if let Some(path) = state_base {
        let base = outside_source(path, source)?;
        if mount_id(&base).map_err(|error| format!("{}: {error}", base.display()))?
            == workspace_mount
        {
            return Err(CaseError::Unsupported(format!(
                "ASCOUT_PRIV_STATE_DIR={} must be on a different mount from {}",
                base.display(),
                workspace_base.display()
            )));
        }
        return ScratchDir::new(&base, prefix)
            .map_err(|error| format!("ASCOUT_PRIV_STATE_DIR={}: {error}", base.display()).into());
    }

    let cache = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from);
    let home_cache = std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache"));
    let needs_btrfs = case.name.starts_with("btrfs-snapshot");
    for candidate in [cache, home_cache]
        .into_iter()
        .flatten()
        .filter(|p| p.is_absolute())
    {
        let Ok(base) = outside_source(&candidate, source) else {
            continue;
        };
        if mount_id(&base).is_ok_and(|id| id != workspace_mount)
            && (!needs_btrfs || fs_type(&base).as_deref() == Some("btrfs"))
        {
            if let Ok(dir) = ScratchDir::new(&base, prefix) {
                return Ok(dir);
            }
        }
    }
    let fs_requirement = if needs_btrfs { " btrfs" } else { "" };
    Err(CaseError::Unsupported(format!(
        "no writable{fs_requirement} cache directory outside the source tree on a different mount from {}; set ASCOUT_PRIV_STATE_DIR",
        workspace_base.display()
    )))
}

fn with_case_dirs(
    case: &Case,
    source: &Path,
    state_base: Option<&Path>,
    n: usize,
    iterations: usize,
) -> Result<(), CaseError> {
    let failed_io = |error: std::io::Error| CaseError::Failed(error.to_string());
    let workspace_base = outside_source(Path::new(case.base), source)?;
    let root = ScratchDir::new(&workspace_base, "ascout-fs-bench-").map_err(failed_io)?;
    let mut state_root = None;
    let result = (|| {
        if case.separate_state {
            state_root = Some(separate_state_dir(
                case,
                &workspace_base,
                source,
                state_base,
            )?);
        }
        let state = state_root.as_ref().unwrap_or(&root).path().join("state");
        let workspace = root.path().join("ws");
        fs::create_dir(&state).map_err(failed_io)?;
        fs::create_dir(&workspace).map_err(failed_io)?;
        if let Some(fs_type) = fs_type(&state) {
            eprintln!("  case {} state_fs={fs_type}", case.name);
        }
        run_case(case, &state, &workspace, n, iterations)
    })();
    let cleanup_errors = state_root
        .into_iter()
        .chain([root])
        .filter_map(|dir| dir.close().err().map(|e| e.to_string()))
        .collect::<Vec<_>>();
    if cleanup_errors.is_empty() {
        result
    } else {
        Err(CaseError::Failed(format!(
            "{}cleanup failed: {}",
            match result {
                Ok(()) => String::new(),
                Err(CaseError::Failed(error) | CaseError::Unsupported(error)) =>
                    format!("{error}; "),
            },
            cleanup_errors.join("; ")
        )))
    }
}

fn run_case(
    case: &Case,
    state: &Path,
    workspace: &Path,
    n: usize,
    iterations: usize,
) -> Result<(), CaseError> {
    let mut config = FilesystemConfig::new(state);
    config.force_overlay = !case.name.starts_with("btrfs-snapshot");
    config.isolate_unmanaged_mounts = case.isolate_unmanaged;
    config.fork_flatten_entry_limit = case.flatten_limit;
    // Benchmark layouts deliberately place the state directory under an
    // isolated mount (including the *-flatten cases); production must not.
    config.allow_overlapping_state_directory = true;
    let backend = FilesystemBackend::new(config).map_err(|e| e.to_string())?;
    let capabilities = backend.capabilities();
    if !capabilities.branch_environment {
        return Err(CaseError::Unsupported(
            "bubblewrap overlay probe failed".into(),
        ));
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
        return Err(CaseError::Unsupported(
            "btrfs snapshot copies unavailable".into(),
        ));
    }
    let mechanism = backend.mechanism().as_str().to_string();
    let cwd = workspace;
    let mut total = BTreeMap::<&'static str, Vec<Duration>>::new();
    backend.establish("root").map_err(|e| e.to_string())?;
    let root_upper = workspace_upper(&backend, "root", cwd)?;
    for index in 0..n {
        write_bench_file(&root_upper, index)?;
    }

    for iteration in 0..iterations {
        let child = format!("child-{iteration}");
        let mark = format!("mark-{iteration}");
        let fork_start = Instant::now();
        backend
            .fork("root", &child, &[])
            .map_err(|e| e.to_string())?;
        total.entry("fork").or_default().push(fork_start.elapsed());
        let base = FilesystemBackend::base_snapshot_id_for_child(&child);
        let mark_start = Instant::now();
        backend
            .mark(&child, &mark, &[])
            .map_err(|e| e.to_string())?;
        total.entry("mark").or_default().push(mark_start.elapsed());
        let child_upper = workspace_upper(&backend, &child, cwd)?;
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
        total
            .entry("accept")
            .or_default()
            .push(accept_start.elapsed());
        if !accepted.conflicts.is_empty() {
            return Err(format!("accept conflicts: {:?}", accepted.conflicts).into());
        }
        backend.resume("root").map_err(|e| e.to_string())?;
        backend.terminate(&child).map_err(|e| e.to_string())?;
        let discard_start = Instant::now();
        backend.discard(&child, &[]).map_err(|e| e.to_string())?;
        total
            .entry("discard")
            .or_default()
            .push(discard_start.elapsed());
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
