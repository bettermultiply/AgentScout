
use super::super::*;
use super::support::*;
use crate::protocol::contract::AgentLaunch;

/// Fixed marker consumed by scripts/priv-check.sh (same format as the
/// runtime::process tests).
fn report_freeze(process: &crate::runtime::process::Processes, test: &str) {
    eprintln!(
        "ASCOUT_FREEZE_MECHANISM={} test={test}",
        process.freeze_mechanism().as_str()
    );
}

#[test]
fn process_freeze_thaw_stops_and_resumes_writer() {
    let root = btrfs_temp();
    let base = root.path().join("base");
    fs::create_dir_all(&base).unwrap();
    let backend = new_overlay_backend(root.path());
    backend.establish("root").unwrap();
    let (process, _, _) = spawn_in_environment(
        &backend,
        "root",
        &base,
        "i=0; while :; do i=$((i+1)); echo $i > counter; sleep 0.02; done",
    );
    let counter = upper_at(&backend, "root", &base.join("counter"));
    wait_until_for(|| counter.exists(), 3000);
    report_freeze(&process, "process_freeze_thaw_stops_and_resumes_writer");
    eprintln!("freeze mechanism: {}", process.freeze_mechanism().as_str());
    process.freeze("root").unwrap();
    let frozen = fs::read_to_string(&counter).unwrap();
    thread::sleep(std::time::Duration::from_millis(150));
    let still = fs::read_to_string(&counter).unwrap();
    assert_eq!(frozen, still, "frozen process kept writing");
    process.thaw("root").unwrap();
    thread::sleep(std::time::Duration::from_millis(150));
    let resumed = fs::read_to_string(&counter).unwrap();
    assert_ne!(still, resumed, "thawed process did not resume");
    process.stop("root").unwrap();
    process.cleanup("root").unwrap();
    backend.terminate("root").unwrap();
    backend.cleanup("root").unwrap();
}

#[test]
fn process_group_fallback_freezes_and_reports_mechanism() {
    let root = btrfs_temp();
    let base = root.path().join("base");
    fs::create_dir_all(&base).unwrap();
    let backend = new_overlay_backend(root.path());
    backend.establish("root").unwrap();
    let launch = AgentLaunch {
        command: "/bin/sh".into(),
        args: vec![
            "-c".into(),
            "i=0; while :; do i=$((i+1)); echo $i > counter; sleep 0.02; done".into(),
        ],
        cwd: base.clone(),
        log: base.join("agent.log"),
        instance_id: None,
        env: BTreeMap::new(),
    };
    let context = BackendContext {
        environment_id: "root".into(),
        ..BackendContext::default()
    };
    let process = crate::runtime::process::Processes::new_process_group(
        "process",
        backend.bubblewrap_path(),
    );
    assert_eq!(
        process.freeze_mechanism(),
        crate::runtime::process::FreezeMechanism::ProcessGroup
    );
    report_freeze(
        &process,
        "process_group_fallback_freezes_and_reports_mechanism",
    );
    let fragment = backend.launch_fragment(&context, &launch).unwrap();
    process.launch(&context, &launch, &fragment).unwrap();
    let counter = upper_at(&backend, "root", &base.join("counter"));
    wait_until_for(|| counter.exists(), 3000);
    process.freeze("root").unwrap();
    let frozen = fs::read_to_string(&counter).unwrap();
    thread::sleep(std::time::Duration::from_millis(150));
    let still = fs::read_to_string(&counter).unwrap();
    assert_eq!(frozen, still);
    process.thaw("root").unwrap();
    thread::sleep(std::time::Duration::from_millis(150));
    assert_ne!(still, fs::read_to_string(&counter).unwrap());
    process.stop("root").unwrap();
    process.cleanup("root").unwrap();
    backend.terminate("root").unwrap();
    backend.cleanup("root").unwrap();
}
