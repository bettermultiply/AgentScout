
use super::super::*;
use super::support::*;
use crate::protocol::contract::AgentLaunch;

#[test]
fn authority_surface_comparison() {
    let root = btrfs_temp();
    let host_base = root.path().join("host-base");
    let sandbox_base = root.path().join("sandbox-base");
    for base in [&host_base, &sandbox_base] {
        fs::create_dir_all(base).unwrap();
        fs::create_dir_all(base.join("lowerdir")).unwrap();
        write(base, "lowerdir/kept.txt", "kept");
        write(base, "cwd-file", "cwd");
    }
    let home = root.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let token = format!("ascout-audit-{}", std::process::id());
    let report_name = "authority-report.txt";
    let watcher = r#"import ctypes, os, time
libc = ctypes.CDLL("libc.so.6", use_errno=True)
fd = libc.inotify_init1(os.O_NONBLOCK)
wd = libc.inotify_add_watch(fd, b"watch.txt", 0x2)
if wd < 0:
    print("watchfail")
    raise SystemExit
with open("watch.txt", "a") as handle:
    handle.write("x")
time.sleep(0.2)
try:
    data = os.read(fd, 4096)
except BlockingIOError:
    data = b""
print("ok" if data else "fail")
"#;
    for base in [&host_base, &sandbox_base] {
        fs::write(base.join("watch.py"), watcher).unwrap();
        fs::write(base.join("watch.txt"), b"watch").unwrap();
    }
    let backend = new_overlay_backend(root.path());
    backend.establish("root").unwrap();
    let candidates = backend.overlayable_mounts();
    for target in ["/tmp", "/var/tmp"] {
        if !candidates
            .iter()
            .any(|candidate| candidate == Path::new(target))
        {
            eprintln!(
                "SKIP authority_surface_comparison: {target} is not an overlayable mount (candidates: {:?})",
                candidates
            );
            return;
        }
    }
    {
        let state = backend.lock().unwrap();
        if state.environments["root"].mount_for_path(&home).is_none() {
            eprintln!(
                "SKIP authority_surface_comparison: HOME {} is not under an overlayable mount",
                home.display()
            );
            return;
        }
    }
    let script = format!(
        "set +e\n             echo cwd_write=$(echo x > cwd-new && echo ok || echo fail)\n             echo tmp_write=$(echo x > /tmp/{token} && echo ok || echo fail)\n             echo vartmp_write=$(echo x > /var/tmp/{token} && echo ok || echo fail)\n             echo home_cache_write=$(mkdir -p \"$HOME/.cache\" 2>/dev/null; echo x > \"$HOME/.cache/{token}\" && echo ok || echo fail)\n             echo mv_lower=$(mv lowerdir lowerdir2 && echo ok || echo fail)\n             echo hardlink=$(ln cwd-file hardlink && echo ok || echo fail)\n             echo symlink=$(ln -s cwd-file symlink && echo ok || echo fail)\n             echo otmpfile=$(python3 -c 'import os; fd=os.open(\".\",os.O_TMPFILE|os.O_WRONLY,0o600); os.write(fd,b\"x\"); print(\"ok\")' 2>/dev/null || echo fail)\n             echo mmap=$(python3 -c 'import mmap,os; f=os.open(\"cwd-file\",os.O_RDONLY); m=mmap.mmap(f,0,prot=mmap.PROT_READ); print(\"ok\" if m[0:] else \"fail\")' 2>/dev/null || echo fail)\n             echo flock=$(flock cwd-file true && echo ok || echo fail)\n             echo xattr=$(python3 -c 'import os; os.setxattr(\"cwd-file\",\"user.ascout\",b\"1\")' 2>/dev/null && echo ok || echo fail)\n             echo chmod=$(chmod 600 cwd-file && echo ok || echo fail)\n             echo utimes=$(touch -d '2000-01-01' cwd-file && echo ok || echo fail)\n             echo fsync=$(python3 -c 'import os; f=os.open(\"cwd-file\",os.O_RDWR); os.fsync(f)' 2>/dev/null && echo ok || echo fail)\n             echo inotify=$(python3 watch.py 2>/dev/null || echo fail)\n             echo stat=$(stat -c '%d:%i:%s' cwd-file 2>/dev/null || echo fail)\n             echo statfs=$(stat -f -c '%T' . 2>/dev/null || echo fail)\n             echo id=$(id -u):$(id -g)\n             echo devshm=$(echo x > /dev/shm/{token} && echo ok || echo fail)\n             echo hostname=$(hostname 2>/dev/null || echo fail)\n             echo ps=$(ps -e 2>/dev/null | wc -l)\n             echo home=$HOME\n             echo pwd=$(pwd)\n",
    );
    let script = format!("(\n{script}\n) > {report_name} 2>&1");
    let host = Command::new("/bin/sh")
        .arg("-c")
        .arg(&script)
        .current_dir(&host_base)
        .env("HOME", &home)
        .output()
        .unwrap();
    assert!(host.status.success());
    let host_report = fs::read_to_string(host_base.join(report_name)).unwrap();

    // Remove the host run's copy so only a sandbox leak can recreate it.
    let host_cache = home.join(".cache").join(&token);
    let _ = fs::remove_file(&host_cache);
    let launch = AgentLaunch {
        command: "/bin/sh".into(),
        args: vec!["-c".into(), script],
        cwd: sandbox_base.clone(),
        log: sandbox_base.join("agent.log"),
        instance_id: None,
        env: BTreeMap::from([("HOME".to_string(), home.display().to_string())]),
    };
    let context = BackendContext {
        environment_id: "root".into(),
        ..BackendContext::default()
    };
    let process =
        crate::runtime::process::Processes::new("process", backend.bubblewrap_path());
    let fragment = backend.launch_fragment(&context, &launch).unwrap();
    process.launch(&context, &launch, &fragment).unwrap();
    process.wait("root").unwrap();
    let sandbox_report =
        fs::read_to_string(upper_at(&backend, "root", &sandbox_base.join(report_name)))
            .unwrap_or_default();
    eprintln!("== host authority report ==\n{host_report}");
    eprintln!("== sandbox authority report ==\n{sandbox_report}");
    let map = |text: &str| -> BTreeMap<String, String> {
        text.lines()
            .filter_map(|line| line.split_once('='))
            .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
            .collect()
    };
    let host_map = map(&host_report);
    let sandbox_map = map(&sandbox_report);
    for key in [
        "cwd_write",
        "tmp_write",
        "vartmp_write",
        "hardlink",
        "symlink",
        "otmpfile",
        "mmap",
        "flock",
        "xattr",
        "chmod",
        "utimes",
        "fsync",
        "inotify",
        "devshm",
    ] {
        assert_eq!(
            host_map.get(key).map(String::as_str),
            Some("ok"),
            "host operation {key} failed"
        );
        assert_eq!(
            sandbox_map.get(key).map(String::as_str),
            Some("ok"),
            "sandbox operation {key} failed"
        );
    }
    assert_eq!(
        sandbox_map.get("home_cache_write").map(String::as_str),
        Some("ok"),
        "~/.cache must be writable inside the sandbox like on the host"
    );
    assert!(
        !host_cache.exists(),
        "sandbox write to ~/.cache reached the host"
    );
    let _ = fs::remove_file(format!("/tmp/{token}"));
    let _ = fs::remove_file(format!("/var/tmp/{token}"));
    let _ = fs::remove_file(format!("/dev/shm/{token}"));
    let _ = fs::remove_file(&host_cache);
    process.cleanup("root").unwrap();
    backend.terminate("root").unwrap();
    backend.cleanup("root").unwrap();
}

#[test]
fn state_directory_is_hidden_inside_sandbox() {
    let root = btrfs_temp();
    let base = root.path().join("base");
    fs::create_dir_all(&base).unwrap();
    let backend = new_overlay_backend(root.path());
    backend.establish("root").unwrap();
    let state = root.path().join("state");
    let script = format!(
        "test -z \"$(ls -A '{}' 2>/dev/null)\" &&              ! test -e '{}/environments' &&              ! test -e '{}/metadata' &&              ! test -e '{}/snapshots' &&              ! (cd '{}' && touch escape) &&              test -z \"$(ls -A '{}/../state' 2>/dev/null)\" &&              test -z \"$(ls -A /proc/self/root{} 2>/dev/null)\"",
        state.display(),
        state.display(),
        state.display(),
        state.display(),
        state.display(),
        state.display(),
        state.display(),
    );
    assert!(
        run_in_environment(&backend, "root", &base, &script),
        "state directory was visible or writable inside the sandbox"
    );
    backend.terminate("root").unwrap();
    backend.cleanup("root").unwrap();
}

#[test]
fn persistence_failures_are_reported_and_locks_are_released() {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    let root = temp();
    let backend = Arc::new(new_overlay_backend(root.path()));
    backend.establish("root").unwrap();

    let base = root.path().join("base");
    fs::create_dir_all(&base).unwrap();
    let launch = AgentLaunch {
        command: "/bin/true".into(),
        args: Vec::new(),
        cwd: base,
        log: root.path().join("agent.log"),
        instance_id: None,
        env: BTreeMap::new(),
    };
    backend.inject_environment_persist_failure(true);
    let launch_error = backend
        .launch_fragment(&BackendContext::new("root"), &launch)
        .unwrap_err();
    assert!(
        launch_error
            .message
            .contains("failed to persist workspace root")
    );
    backend.inject_environment_persist_failure(false);

    backend.inject_environment_persist_failure(true);
    let error = backend.suspend("root").unwrap_err();
    assert!(
        error
            .message
            .contains("injected environment persistence failure")
    );
    assert_eq!(backend.state("root"), Some(EnvironmentState::Available));
    backend.inject_environment_persist_failure(false);

    backend.inject_operation_persist_failure(true);
    let context = BackendContext {
        operation_id: "persist-failure-fork".into(),
        generation: 1,
        environment_id: "root".into(),
        ..BackendContext::default()
    };
    let operation = Operation::FilesystemFork {
        parent_snapshot_id: "root".into(),
        child_snapshot_id: "child".into(),
        path_scope: vec![],
    };
    let error = backend
        .filesystem_operation(&context, &operation, &CancelHandle::default())
        .unwrap_err();
    assert!(
        error
            .message
            .contains("was applied but its operation record")
    );
    assert!(
        error
            .cleanup_failures
            .iter()
            .any(|value| value.contains("operation record"))
    );
    backend.inject_operation_persist_failure(false);
    backend.terminate("child").unwrap();
    backend.discard("child", &[]).unwrap();

    backend.inject_persist_delay(500);
    let worker = backend.clone();
    let thread = std::thread::spawn(move || worker.suspend("root"));
    std::thread::sleep(Duration::from_millis(100));
    let start = Instant::now();
    assert_eq!(backend.state("root"), Some(EnvironmentState::Available));
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_millis(250),
        "state() blocked on persistence lock for {elapsed:?}"
    );
    thread.join().unwrap().unwrap();
    backend.inject_persist_delay(0);
    backend.resume("root").unwrap();
    backend.terminate("root").unwrap();
    backend.cleanup("root").unwrap();
}

#[test]
fn corrupt_operations_metadata_is_isolated() {
    let root = temp();
    let backend = new_overlay_backend(root.path());
    backend.establish("root").unwrap();
    backend.terminate("root").unwrap();
    backend.cleanup("root").unwrap();
    drop(backend);
    let operations = root.path().join("state/metadata/operations.json");
    fs::write(&operations, b"{not-json").unwrap();
    let backend =
        FilesystemBackend::new(overlapping_state_config(root.path().join("state"))).unwrap();
    assert!(
        !operations.exists(),
        "corrupt operations file was not moved"
    );
    let corrupt: Vec<_> = fs::read_dir(root.path().join("state/metadata"))
        .unwrap()
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("operations.json.corrupt-")
        })
        .collect();
    assert_eq!(corrupt.len(), 1, "corrupt operations file was not isolated");
    drop(backend);
}

#[test]
fn corrupt_operations_metadata_fails_closed_when_isolation_fails() {
    let root = temp();
    let backend = new_overlay_backend(root.path());
    backend.establish("root").unwrap();
    backend.terminate("root").unwrap();
    backend.cleanup("root").unwrap();
    drop(backend);

    let operations = root.path().join("state/metadata/operations.json");
    fs::write(&operations, b"{not-json").unwrap();
    let corrupt = root
        .path()
        .join("state/metadata")
        .join(format!("operations.json.corrupt-{}", std::process::id()));
    fs::create_dir_all(&corrupt).unwrap();
    fs::write(corrupt.join("keep"), b"keep").unwrap();

    let error = match FilesystemBackend::new(overlapping_state_config(root.path().join("state"))) {
        Ok(_) => panic!("corrupt operations metadata must fail closed when it cannot be isolated"),
        Err(error) => error,
    };
    assert!(
        error.message.contains("operations.json"),
        "error did not name the corrupt file: {}",
        error.message
    );
    assert!(
        error.message.contains("could not be isolated"),
        "error did not report the isolation failure: {}",
        error.message
    );
    assert!(
        operations.exists(),
        "bad operations file must not be removed when isolation fails"
    );
    assert_eq!(fs::read(&operations).unwrap(), b"{not-json");
    assert_eq!(fs::read(corrupt.join("keep")).unwrap(), b"keep");
}

#[test]
fn overlapping_state_directory_is_rejected_by_default() {
    let root = btrfs_temp();
    let state = root.path().join("state");
    let backend = FilesystemBackend::new(FilesystemConfig::new(&state)).unwrap();
    let overlaps = backend.state_overlap_mounts();
    if overlaps.is_empty() {
        eprintln!(
            "SKIP overlapping_state_directory_is_rejected_by_default: state directory is not inside an isolated mount"
        );
        return;
    }
    let error = backend
        .establish("root")
        .expect_err("overlapping state directory must be rejected by default");
    let state_path = backend.config.state_directory.display().to_string();
    assert!(
        error.message.contains(&state_path),
        "error did not name the state directory {}: {}",
        state_path,
        error.message
    );
    for mount in &overlaps {
        let mount_path = mount.display().to_string();
        assert!(
            error.message.contains(&mount_path),
            "error did not name the conflicting mount {}: {}",
            mount_path,
            error.message
        );
    }
    assert!(
        error.message.contains("outside the isolated mounts"),
        "error did not suggest moving the state directory: {}",
        error.message
    );
    assert!(
        !backend.paths("root").exists(),
        "rejected establish must not create the environment root"
    );
    assert!(
        !backend.environment_metadata_path("root").exists(),
        "rejected establish must not create environment metadata"
    );
    assert!(
        !state.join("environments/root").exists(),
        "rejected establish must not create mount upper directories"
    );
}

#[test]
fn default_config_starts_when_state_directory_is_outside_isolated_mounts() {
    let workspace_root = btrfs_temp();
    let base = workspace_root.path().join("base");
    fs::create_dir_all(&base).unwrap();
    let probe = FilesystemBackend::new(overlapping_state_config(
        workspace_root.path().join("probe-state"),
    ))
    .unwrap();
    let isolated = probe.overlayable_mounts();
    drop(probe);
    let state = match tempdir_outside_mounts(&base, &isolated) {
        Some(value) => value,
        None => {
            eprintln!(
                "SKIP default_config_starts_when_state_directory_is_outside_isolated_mounts: no writable mount outside isolated mounts"
            );
            return;
        }
    };
    let backend =
        FilesystemBackend::new(FilesystemConfig::new(state.path().join("state"))).unwrap();
    assert!(
        backend.state_overlap_mounts().is_empty(),
        "chosen state layout must not overlap: {:?}",
        backend.state_overlap_mounts()
    );
    backend.establish("root").unwrap();
    let workspace_mount = mount_for(&backend, "root", &base);
    let launch = AgentLaunch {
        command: "/bin/true".into(),
        args: Vec::new(),
        cwd: base.clone(),
        log: base.join("agent.log"),
        instance_id: None,
        env: BTreeMap::new(),
    };
    let context = BackendContext {
        environment_id: "root".into(),
        ..BackendContext::default()
    };
    let fragment = backend
        .launch_fragment(&context, &launch)
        .expect("default config must start with a non-overlapping state directory");
    assert!(
        fragment.mounts.iter().any(|entry| matches!(
            entry,
            MountEntry::Overlay { target, .. } if target == &workspace_mount.target
        )),
        "launch fragment must overlay the workspace mount"
    );
    backend.terminate("root").unwrap();
    backend.cleanup("root").unwrap();
}

#[test]
fn home_cache_and_config_writes_stay_in_home_upper() {
    let root = btrfs_temp();
    let base = root.path().join("base");
    fs::create_dir_all(&base).unwrap();
    let backend = new_overlay_backend(root.path());
    backend.establish("root").unwrap();
    let home = root.path().join("home");
    fs::create_dir_all(&home).unwrap();
    {
        let state = backend.lock().unwrap();
        if state.environments["root"].mount_for_path(&home).is_none() {
            eprintln!(
                "SKIP home_cache_and_config_writes_stay_in_home_upper: HOME {} is not under an overlayable mount",
                home.display()
            );
            return;
        }
    }
    let token = format!("ascout-home-{}", std::process::id());
    let cache = home.join(".cache").join(&token);
    let config = home.join(".config").join(&token);
    let _ = fs::remove_file(&cache);
    let _ = fs::remove_file(&config);
    let script = format!(
        "mkdir -p \"$HOME/.cache\" \"$HOME/.config\" &&              echo cache > \"$HOME/.cache/{token}\" &&              echo config > \"$HOME/.config/{token}\""
    );
    assert!(
        run_in_environment_with_env(
            &backend,
            "root",
            &base,
            &script,
            BTreeMap::from([("HOME".to_string(), home.display().to_string())]),
        ),
        "sandbox could not write $HOME/.cache or $HOME/.config"
    );
    assert!(
        !cache.exists(),
        "sandbox write to ~/.cache reached the host"
    );
    assert!(
        !config.exists(),
        "sandbox write to ~/.config reached the host"
    );
    assert!(
        upper_at(&backend, "root", &cache).exists(),
        "~/.cache write did not reach the /home branch upper"
    );
    assert!(
        upper_at(&backend, "root", &config).exists(),
        "~/.config write did not reach the /home branch upper"
    );
    backend.terminate("root").unwrap();
    backend.cleanup("root").unwrap();
}

#[test]
fn writes_outside_cwd_reach_branch_uppers_not_host() {
    let root = btrfs_temp();
    let base = root.path().join("base");
    fs::create_dir_all(&base).unwrap();
    let backend = new_overlay_backend(root.path());
    backend.establish("root").unwrap();
    let candidates = backend.overlayable_mounts();
    for target in ["/tmp", "/var/tmp", "/srv"] {
        if !candidates
            .iter()
            .any(|candidate| candidate == Path::new(target))
        {
            eprintln!(
                "SKIP writes_outside_cwd_reach_branch_uppers_not_host: {target} is not an overlayable mount (candidates: {:?})",
                candidates
            );
            return;
        }
    }
    let token = format!("ascout-outside-{}", std::process::id());
    let tmp = format!("/tmp/{token}");
    let vartmp = format!("/var/tmp/{token}");
    let srv = format!("/srv/{token}");
    let script = format!("echo tmp > {tmp}; echo vartmp > {vartmp}; echo srv > {srv}");
    assert!(run_in_environment(&backend, "root", &base, &script));
    assert!(!Path::new(&tmp).exists(), "host /tmp was written");
    assert!(!Path::new(&vartmp).exists(), "host /var/tmp was written");
    assert!(!Path::new(&srv).exists(), "host /srv was written");
    let state = backend.lock().unwrap();
    let upper_for = |target: &str| {
        state.environments["root"]
            .mount_by_target(Path::new(target))
            .unwrap_or_else(|| panic!("missing mount unit {target}"))
            .upper
            .clone()
    };
    assert!(upper_for("/tmp").join(&token).exists());
    assert!(upper_for("/var/tmp").join(&token).exists());
    assert!(upper_for("/srv").join(&token).exists());
    drop(state);
    backend.terminate("root").unwrap();
    backend.cleanup("root").unwrap();
}
