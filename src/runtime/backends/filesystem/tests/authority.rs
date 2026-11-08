//! authority 行为、状态遮蔽与元数据迁移测试。

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
        let script = format!(
            "set +e\n             echo cwd_write=$(echo x > cwd-new && echo ok || echo fail)\n             echo tmp_write=$(echo x > /tmp/{token} && echo ok || echo fail)\n             echo vartmp_write=$(echo x > /var/tmp/{token} && echo ok || echo fail)\n             echo home_cache_write=$(mkdir -p \"$HOME/.cache\" 2>/dev/null; echo x > \"$HOME/.cache/{token}\" && echo ok || echo fail)\n             echo mv_lower=$(mv lowerdir lowerdir2 && echo ok || echo fail)\n             echo hardlink=$(ln cwd-file hardlink && echo ok || echo fail)\n             echo symlink=$(ln -s cwd-file symlink && echo ok || echo fail)\n             echo otmpfile=$(python3 -c 'import os; fd=os.open(\".\",os.O_TMPFILE|os.O_WRONLY,0o600); os.write(fd,b\"x\"); print(\"ok\")' 2>/dev/null || echo fail)\n             echo mmap=$(python3 -c 'import mmap,os; f=os.open(\"cwd-file\",os.O_RDONLY); m=mmap.mmap(f,0,prot=mmap.PROT_READ); print(\"ok\" if m[0:] else \"fail\")' 2>/dev/null || echo fail)\n             echo flock=$(flock cwd-file true && echo ok || echo fail)\n             echo xattr=$(python3 -c 'import os; os.setxattr(\"cwd-file\",\"user.ascout\",b\"1\")' 2>/dev/null && echo ok || echo fail)\n             echo chmod=$(chmod 600 cwd-file && echo ok || echo fail)\n             echo utimes=$(touch -d '2000-01-01' cwd-file && echo ok || echo fail)\n             echo fsync=$(python3 -c 'import os; f=os.open(\"cwd-file\",os.O_RDWR); os.fsync(f)' 2>/dev/null && echo ok || echo fail)\n             echo inotify=$(python3 watch.py 2>/dev/null || echo fail)\n             echo stat=$(stat -c '%d:%i:%s' cwd-file 2>/dev/null || echo fail)\n             echo statfs=$(stat -f -c '%T' . 2>/dev/null || echo fail)\n             echo id=$(id -u):$(id -g)\n             echo devshm=$(echo x > /dev/shm/{token} && echo ok || echo fail)\n             echo hostname=$(hostname 2>/dev/null || echo fail)\n             echo ps=$(ps -e 2>/dev/null | wc -l)\n             echo home=$HOME\n             echo pwd=$(pwd)\n",
        );
        let script = format!("(\n{script}\n) > {report_name} 2>&1");
        let host = Command::new("/bin/sh")
            .arg("-c")
            .arg(&script)
            .current_dir(&host_base)
            .env("HOME", "/home/betmul")
            .output()
            .unwrap();
        assert!(host.status.success());
        let host_report = fs::read_to_string(host_base.join(report_name)).unwrap();

        // The host run above legitimately created the same path.  Remove it so
        // the sandbox check proves its write stayed in the branch upper.
        let _ = fs::remove_file(format!("/home/betmul/.cache/{token}"));
        let backend = new_overlay_backend(root.path());
        backend.establish("root").unwrap();
        let launch = AgentLaunch {
            command: "/bin/sh".into(),
            args: vec!["-c".into(), script],
            cwd: sandbox_base.clone(),
            log: sandbox_base.join("agent.log"),
            instance_id: None,
            env: BTreeMap::from([("HOME".to_string(), "/home/betmul".to_string())]),
        };
        let context = BackendContext {
            environment_id: "root".into(),
            ..BackendContext::default()
        };
        let process = crate::runtime::process::ProcessBackend::new("process", backend.bubblewrap_path());
        let fragment = backend.launch_fragment(&context, &launch).unwrap();
        process.launch(&context, &launch, &fragment).unwrap();
        process.wait("root").unwrap();
        let sandbox_report = fs::read_to_string(upper_at(
            &backend,
            "root",
            &sandbox_base.join(report_name),
        ))
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
            !Path::new(&format!("/home/betmul/.cache/{token}")).exists(),
            "sandbox write to ~/.cache reached the host"
        );
        let _ = fs::remove_file(format!("/tmp/{token}"));
        let _ = fs::remove_file(format!("/var/tmp/{token}"));
        let _ = fs::remove_file(format!("/dev/shm/{token}"));
        let _ = fs::remove_file(format!("/home/betmul/.cache/{token}"));
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
    fn home_cache_and_config_writes_stay_in_home_upper() {
        let root = btrfs_temp();
        let base = root.path().join("base");
        fs::create_dir_all(&base).unwrap();
        let backend = new_overlay_backend(root.path());
        backend.establish("root").unwrap();
        let token = format!("ascout-home-{}", std::process::id());
        let cache = PathBuf::from(format!("/home/betmul/.cache/{token}"));
        let config = PathBuf::from(format!("/home/betmul/.config/{token}"));
        let _ = fs::remove_file(&cache);
        let _ = fs::remove_file(&config);
        let script = format!(
            "mkdir -p \"$HOME/.cache\" \"$HOME/.config\" &&              echo cache > \"$HOME/.cache/{token}\" &&              echo config > \"$HOME/.config/{token}\""
        );
        assert!(
            run_in_environment(&backend, "root", &base, &script),
            "sandbox could not write $HOME/.cache or $HOME/.config"
        );
        assert!(!cache.exists(), "sandbox write to ~/.cache reached the host");
        assert!(!config.exists(), "sandbox write to ~/.config reached the host");
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
        let token = format!("ascout-outside-{}", std::process::id());
        let tmp = format!("/tmp/{token}");
        let vartmp = format!("/var/tmp/{token}");
        let srv = format!("/srv/{token}");
        let script = format!(
            "echo tmp > {tmp}; echo vartmp > {vartmp}; echo srv > {srv}"
        );
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

    #[test]
    fn live_replay_blocks_absolute_symlink_escape() {
        let root = btrfs_temp();
        let base = root.path().join("base");
        fs::create_dir_all(&base).unwrap();
        let sensitive = base.join("sensitive");
        fs::create_dir_all(&sensitive).unwrap();
        write(&sensitive, "secret", "host");
        std::os::unix::fs::symlink(&sensitive, base.join("escape")).unwrap();
        let mut config = FilesystemConfig::new(root.path().join("state"));
        config.force_overlay = true;
        config.isolate_unmanaged_mounts = false;
        let backend = FilesystemBackend::new(config).unwrap();
        assert!(backend.usable);
        backend.establish("root").unwrap();
        let (target, target_pid, _) =
            spawn_in_environment(&backend, "root", &base, "sleep 5");
        let workspace = {
            let state = backend.lock().unwrap();
            state.environments["root"].workspace_root.clone().unwrap()
        };
        let safe = SafeRoot::open(target_pid).unwrap();
        let source = root.path().join("source-file");
        fs::write(&source, b"attack").unwrap();
        let relative = workspace.strip_prefix("/").unwrap();
        let target_path = format!(
            "{}/escape/pwn",
            relative.to_string_lossy()
        );
        let result = safe.write_file(&target_path, &source, 0o644);
        // The absolute symlink must resolve inside the sandbox root, not the
        // caller's root.  Writing through it either fails or stays inside the
        // sandbox; it must never reach the host sensitive directory.
        assert!(
            result.is_err() || !sensitive.join("pwn").exists(),
            "write escaped to the host through an absolute symlink"
        );
        assert!(!sensitive.join("pwn").exists());
        assert_eq!(fs::read(sensitive.join("secret")).unwrap(), b"host");
        let _ = target.stop("root");
        backend.terminate("root").unwrap();
        backend.cleanup("root").unwrap();
    }

    #[test]
    fn legacy_scope_metadata_is_migrated_to_mount_units() {
        let root = btrfs_temp();
        let backend = new_overlay_backend(root.path());
        backend.establish("root").unwrap();
        let metadata_path = root
            .path()
            .join("state/metadata/environments/root.json");
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&metadata_path).unwrap()).unwrap();
        value.as_object_mut().unwrap().remove("mounts");
        value["lower"] = serde_json::json!({ "tmp": ["/legacy/layer"] });
        value["workspace_root"] = serde_json::Value::Null;
        fs::write(&metadata_path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
        drop(backend);

        let backend = FilesystemBackend::new(FilesystemConfig::new(root.path().join("state"))).unwrap();
        let state = backend.lock().unwrap();
        let environment = &state.environments["root"];
        let tmp = environment
            .mount_by_target(Path::new("/tmp"))
            .expect("tmp mount migrated");
        assert_eq!(tmp.layer_key, "tmp");
        assert_eq!(tmp.lower, vec![PathBuf::from("/legacy/layer")]);
        drop(state);
        drop(backend);

        // A legacy workspace layer without workspace_root cannot be mapped to
        // an absolute mount point and must fail loudly instead of being lost.
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&metadata_path).unwrap()).unwrap();
        value.as_object_mut().unwrap().remove("mounts");
        value["lower"] = serde_json::json!({ "workspace": ["/legacy/layer"] });
        value["workspace_root"] = serde_json::Value::Null;
        fs::write(&metadata_path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
        let error = match FilesystemBackend::new(FilesystemConfig::new(root.path().join("state"))) {
            Ok(_) => panic!("legacy workspace metadata without workspace_root must fail"),
            Err(error) => error,
        };
        assert!(
            error.message.contains("workspace_root"),
            "unexpected migration error: {}",
            error.message
        );
    }

