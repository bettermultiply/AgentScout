use super::super::*;
use super::support::*;

#[test]
fn recovery_overlay_executes_independently_and_rejects_corruption() {
    let root=tempfile::Builder::new().prefix("fs-recovery-").tempdir_in("/tmp/opencode").unwrap();
    let workspace=root.path().join("workspace");fs::create_dir(&workspace).unwrap();
    let backend=new_overlay_backend(root.path());
    // Isolate this temporary workspace only; never modify the repository or host lowers.
    *backend.overlay_targets.lock().unwrap()=Some(vec![workspace.clone()]);
    backend.establish("authority").unwrap();
    let (process,_,_)=spawn_in_environment(&backend,"authority",&workspace,"echo before > value");
    wait_until_for(||upper_at(&backend,"authority",&workspace.join("value")).exists(),3000);
    process.stop_all();
    let saved=root.path().join("saved.json");backend.save_runtime(&saved).unwrap();
    let original=upper_at(&backend,"authority",&workspace.join("value"));
    fs::write(&original,"after").unwrap();
    backend.load_runtime(&saved).unwrap();
    let restored=upper_at(&backend,"authority",&workspace.join("value"));
    assert_ne!(restored,original);assert_eq!(fs::read_to_string(&restored).unwrap(),"before\n");
    let (resumed,_,_)=spawn_in_environment(&backend,"authority",&workspace,"echo continued > value");
    wait_until_for(||fs::read_to_string(&restored).unwrap_or_default()=="continued\n",3000);resumed.stop_all();
    backend.load_runtime(&saved).unwrap();
    assert_eq!(fs::read_to_string(upper_at(&backend,"authority",&workspace.join("value"))).unwrap(),"before\n");
    let saved_again=root.path().join("again.json");backend.save_runtime(&saved_again).unwrap();backend.load_runtime(&saved_again).unwrap();
    let failed=root.path().join("failed.json");
    set_clone_failure_after(Some(1));assert!(backend.save_runtime(&failed).is_err());set_clone_failure_after(None);
    assert!(!failed.exists());backend.load_runtime(&saved).unwrap();
    fs::write(saved.with_extension("filesystem").join("environments/authority/mutated"),"corrupt").unwrap();
    assert!(backend.load_runtime(&saved).unwrap_err().to_string().contains("content changed"));
}

#[test]
fn combined_save_failure_thaws_process_and_preserves_previous_generation() {
    let root=tempfile::Builder::new().prefix("fs-recovery-combined-").tempdir_in("/tmp/opencode").unwrap();
    let workspace=root.path().join("workspace");fs::create_dir(&workspace).unwrap();
    let backend=Arc::new(new_overlay_backend(root.path()));
    *backend.overlay_targets.lock().unwrap()=Some(vec![workspace.clone()]);
    backend.establish("authority").unwrap();
    let process=crate::runtime::process::ProcessBackend::new("process",backend.bubblewrap_path());
    let launch=crate::protocol::contract::AgentLaunch {command:"/bin/sh".into(),args:vec!["-c".into(),"i=0; while :; do i=$((i+1)); echo $i > counter; sleep 0.02; done".into()],cwd:workspace.clone(),log:root.path().join("agent.log"),instance_id:None,env:BTreeMap::new()};
    let context=BackendContext::new("authority");
    process.launch(&context,&launch,&backend.launch_fragment(&context,&launch).unwrap()).unwrap();
    wait_until_for(||upper_at(&backend,"authority",&workspace.join("counter")).exists(),3000);
    let previous=root.path().join("previous.json");
    process.with_frozen(||backend.save_runtime(&previous)).unwrap();
    let before=fs::read(&previous).unwrap();
    set_clone_failure_after(Some(1));
    let failure=process.with_frozen(||backend.save_runtime(&root.path().join("failed.json")));
    set_clone_failure_after(None);assert!(failure.is_err());
    assert_eq!(fs::read(&previous).unwrap(),before);
    let upper=upper_at(&backend,"authority",&workspace.join("counter"));
    let at_failure=fs::read_to_string(&upper).unwrap();wait_until_for(||fs::read_to_string(&upper).unwrap_or_default()!=at_failure,3000);
    process.stop_all();backend.load_runtime(&previous).unwrap();
    assert!(fs::read_to_string(upper_at(&backend,"authority",&workspace.join("counter"))).unwrap().trim().parse::<u64>().unwrap()>0);
}
