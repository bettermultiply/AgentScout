use super::*;
use serde_json::{json, Value};
use std::os::unix::fs::FileTypeExt;

// Paths in the saved metadata always name the private generation, never the live upper.
fn relocate(value: &mut Value, from: &Path, to: &Path) {
    match value {
        Value::String(s) => {
            if let Ok(relative)=Path::new(s).strip_prefix(from) { *s=to.join(relative).display().to_string(); }
        }
        Value::Array(values) => for v in values {relocate(v,from,to)},
        Value::Object(values) => for v in values.values_mut() {relocate(v,from,to)},
        _ => (),
    }
}

impl FilesystemBackend {
    pub(super) fn save_recovery(&self, path: &Path) -> Result<(), BackendError> {
        let state=self.lock()?;
        let directory=path.with_extension("filesystem");
        fs::create_dir(&directory).map_err(error)?;
        let mut environments: Vec<_>=state.environments.values().map(|e|json!({"root":e.root,"metadata":EnvironmentMetadata::from_environment(e)})).collect();
        let snapshots: Vec<_>=state.snapshots.values().map(SnapshotMetadata::from_record).collect();
        // Host mount points are locations, not immutable content identities. Save
        // each bottom layer once; existing saved bases are already owned snapshots.
        let mut bases=BTreeMap::new();
        for saved in &mut environments {
            let mounts=saved["metadata"]["mounts"].as_object_mut().ok_or_else(||BackendError::new("Missing mounts"))?;
            for mount in mounts.values_mut() {
                let target:PathBuf=serde_json::from_value(mount["target"].clone()).map_err(error)?;
                let base:Option<PathBuf>=serde_json::from_value(mount["saved_base"].clone()).map_err(error)?;
                let base=base.unwrap_or(target);
                if base==Path::new("/") || self.config.state_directory.starts_with(&base) || directory.starts_with(&base) {
                    return Err(BackendError::new(format!("Cannot independently save filesystem lower {} containing runtime or recovery storage; use isolated_paths outside that storage",base.display())));
                }
                let destination=directory.join("bases").join(short_hash(&base.display().to_string()));
                if !bases.contains_key(&base) {
                    tree_digest(&base)?;
                    clone_upper(&base,&destination,self.reflink_available)?;
                    bases.insert(base,destination.clone());
                }
                mount["saved_base"]=json!(destination);
            }
        }
        let mut roots=BTreeSet::new();
        for env in state.environments.values() {
            let relative=env.root.strip_prefix(&self.config.state_directory).map_err(error)?;
            roots.insert(relative.components().next().ok_or_else(||BackendError::new("Missing environment root"))?.as_os_str().to_string_lossy().into_owned());
        }
        for snapshot in state.snapshots.values() {
            let relative=snapshot.path.strip_prefix(&self.config.state_directory).map_err(error)?;
            roots.insert(relative.components().next().ok_or_else(||BackendError::new("Missing snapshot root"))?.as_os_str().to_string_lossy().into_owned());
        }
        let mut excluded:BTreeSet<PathBuf>=state.environments.values().flat_map(|e|e.mounts.values().map(|m|m.work.clone())).collect();
        fn exclude_work(path:&Path,excluded:&mut BTreeSet<PathBuf>)->Result<(),BackendError>{
            for entry in fs::read_dir(path).map_err(error)? {
                let entry=entry.map_err(error)?;
                if !entry.file_type().map_err(error)?.is_dir(){continue;}
                let parent=entry.path().parent().map(Path::to_path_buf);
                let mount_volume=parent.as_ref().is_some_and(|p| {
                    let unit=if p.file_name().is_some_and(|n|n=="volume"){p.parent()}else{Some(p.as_path())};
                    unit.and_then(Path::parent).and_then(Path::file_name).is_some_and(|n|n=="mounts")
                });
                if entry.file_name()=="work" && mount_volume && parent.is_some_and(|p|p.join("upper").is_dir()) {excluded.insert(entry.path());continue;}
                exclude_work(&entry.path(),excluded)?;
            }
            Ok(())
        }
        for name in &roots {exclude_work(&self.config.state_directory.join(name),&mut excluded)?;}
        for name in &roots {
            clone_upper_excluding(&self.config.state_directory.join(name), &directory.join(name), self.reflink_available,&excluded)?;
        }
        let mut subvolumes=Vec::new();
        if self.uses_btrfs_storage() {
            fn collect(path:&Path,root:&Path,excluded:&BTreeSet<PathBuf>,output:&mut Vec<PathBuf>)->Result<(),BackendError>{
                if excluded.contains(path) {return Ok(());}
                let metadata=fs::symlink_metadata(path).map_err(error)?;
                if !metadata.is_dir() {return Ok(());}
                if metadata.ino()==256 {output.push(path.strip_prefix(root).map_err(error)?.to_path_buf());}
                for entry in fs::read_dir(path).map_err(error)? {collect(&entry.map_err(error)?.path(),root,excluded,output)?;}
                Ok(())
            }
            for name in &roots {collect(&self.config.state_directory.join(name),&self.config.state_directory,&excluded,&mut subvolumes)?;}
        }
        if !bases.is_empty() {roots.insert("bases".into());}
        let mut record=json!({"version":2,"kind":"filesystem-overlay","mechanism":self.mechanism.as_str(),"snapshot_copies":self.snapshot_copies,"root":directory,"roots":roots,"content_sha256":tree_digest(&directory)?,"environments":environments,"snapshots":snapshots,"operations":state.operations});
        record["subvolumes"]=json!(subvolumes);
        record["system_binding"]=system_binding(&self.config.bubblewrap_path)?;
        relocate(&mut record, &self.config.state_directory, &directory);
        fs::write(path,serde_json::to_vec(&record).map_err(error)?).map_err(error)
    }

    pub(super) fn load_recovery(&self, path: &Path) -> Result<(), BackendError> {
        let mut record:Value=serde_json::from_slice(&fs::read(path).map_err(error)?).map_err(error)?;
        if record["version"]!=2 || record["kind"]!="filesystem-overlay" {return Err(BackendError::new("Unsupported filesystem recovery version; independent lower content required"));}
        if record["mechanism"]!=self.mechanism.as_str() || record["snapshot_copies"]!=self.snapshot_copies {return Err(BackendError::new("Filesystem recovery storage mechanism changed"));}
        let source=path.with_extension("filesystem");
        if record["root"]!=source.display().to_string() {return Err(BackendError::new("Filesystem recovery resource binding mismatch"));}
        if record["content_sha256"]!=tree_digest(&source)? {return Err(BackendError::new("Filesystem recovery content changed"));}
        if record["system_binding"]!=system_binding(&self.config.bubblewrap_path)? {return Err(BackendError::new("Filesystem system dependency version or content changed"));}
        // Never overwrite the previous live tree or the save. All restored uppers are independent.
        let target=self.config.state_directory.join(format!("restored-{}",now_nanos()));
        fs::create_dir(&target).map_err(error)?;
        let result=(|| {
        let roots:Vec<String>=serde_json::from_value(record["roots"].clone()).map_err(error)?;
        for name in roots {validate_component(&name)?;clone_upper(&source.join(&name),&target.join(&name),self.reflink_available)?;}
        let mut subvolumes:Vec<PathBuf>=serde_json::from_value(record["subvolumes"].clone()).map_err(error)?;
        subvolumes.sort_by_key(|p|p.components().count());
        for relative in subvolumes {
            if relative.is_absolute() || relative.components().any(|c|!matches!(c,std::path::Component::Normal(_))) {return Err(BackendError::new("Invalid saved subvolume path"));}
            let volume=target.join(relative);
            let temporary=staging_path(&volume);
            fs::rename(&volume,&temporary).map_err(error)?;
            self.btrfs_create(&volume)?;
            clone_upper(&temporary,&volume,self.reflink_available)?;
            remove_directory_force(&temporary)?;
        }
        relocate(&mut record,&source,&target);
        let envs:Vec<Value>=serde_json::from_value(record["environments"].clone()).map_err(error)?;
        let snaps:Vec<SnapshotMetadata>=serde_json::from_value(record["snapshots"].clone()).map_err(error)?;
        let operations=serde_json::from_value(record["operations"].clone()).map_err(error)?;
        let mut environments=HashMap::new();
        for saved in envs {
            let metadata:EnvironmentMetadata=serde_json::from_value(saved["metadata"].clone()).map_err(error)?;
            validate_component(&metadata.id)?;
            let env_root:PathBuf=serde_json::from_value(saved["root"].clone()).map_err(error)?;
            if !env_root.starts_with(&target) {return Err(BackendError::new("Filesystem environment root outside restored state"));}
            let env=metadata.into_environment(env_root)?;
            for mount in env.mounts.values() {
                if !mount.upper.starts_with(&target) || !mount.work.starts_with(&target) || !mount.upper.is_dir() || mount.saved_base.as_ref().is_none_or(|p|!p.starts_with(&target)||!p.is_dir()) || mount.lower.iter().any(|p|!p.starts_with(&target)||!p.is_dir()) {
                    return Err(BackendError::new("Filesystem recovery mount is missing or outside restored state"));
                }
                ensure_overlay_work(&mount.upper,&mount.work)?;
            }
            if environments.insert(env.id.clone(),env).is_some() {return Err(BackendError::new("Duplicate filesystem environment"));}
        }
        let mut snapshots=HashMap::new();
        for snapshot in snaps {
            if !snapshot.path.starts_with(&target) || !snapshot.path.is_dir() {return Err(BackendError::new("Missing filesystem snapshot"));}
            if snapshots.insert(snapshot.snapshot_id.clone(),snapshot.into_record()).is_some() {return Err(BackendError::new("Duplicate filesystem snapshot"));}
        }
        *self.lock()?=BackendState {environments,snapshots,operations};
        Ok(())
        })();
        if result.is_err() { let _=remove_path(&self.config.btrfs_path,&target,self.uses_btrfs_storage()); }
        result
    }
}

// Bind the installed system version without copying the host root. This does
// not attest arbitrary unmanaged files; explicit mutable data belongs in mounts.
fn system_binding(bwrap:&Path)->Result<Value,BackendError> {
    let (program,args)=if Path::new("/usr/bin/pacman").exists() {("/usr/bin/pacman",vec!["-Q"])}
        else if Path::new("/usr/bin/dpkg-query").exists(){("/usr/bin/dpkg-query",vec!["-W"])}
        else {return Err(BackendError::new("Cannot bind system dependency versions: supported package database missing"));};
    let output=Command::new(program).args(args).output().map_err(error)?;
    if !output.status.success(){return Err(BackendError::new("Cannot read system dependency versions"));}
    let mut files=BTreeMap::new();
    for path in [Path::new("/bin/sh"),Path::new("/usr/bin/env"),bwrap] {
        let path=if path.is_absolute(){path.to_path_buf()}else{PathBuf::from("/usr/bin").join(path)};
        files.insert(path.display().to_string(),format!("{:x}",Sha256::digest(fs::read(&path).map_err(error)?)));
    }
    Ok(json!({"packages":String::from_utf8(output.stdout).map_err(error)?,"executables":files}))
}

fn tree_digest(root: &Path) -> Result<String,BackendError> {
    fn visit(path:&Path, root:&Path, hash:&mut Sha256)->Result<(),BackendError> {
        let metadata=fs::symlink_metadata(path).map_err(error)?;
        if metadata.file_type().is_socket() {return Err(BackendError::new(format!("Cannot save active filesystem socket: {}",path.display())));}
        hash.update(path.strip_prefix(root).map_err(error)?.as_os_str().as_encoded_bytes());
        hash.update(metadata.mode().to_le_bytes());hash.update(metadata.rdev().to_le_bytes());
        for name in ["opaque","whiteout","redirect","metacopy"] {
            hash.update(name.as_bytes());
            if let Some(value)=overlay_xattr(path,name) {hash.update([1]);hash.update(value);}else{hash.update([0]);}
        }
        if metadata.is_file() {hash.update(fs::read(path).map_err(error)?);}
        else if metadata.file_type().is_symlink() {hash.update(fs::read_link(path).map_err(error)?.as_os_str().as_encoded_bytes());}
        else if metadata.is_dir() {
            let mut entries=fs::read_dir(path).map_err(error)?.collect::<Result<Vec<_>,_>>().map_err(error)?;
            entries.sort_by_key(|e|e.file_name());
            for entry in entries {visit(&entry.path(),root,hash)?;}
        }
        Ok(())
    }
    let mut hash=Sha256::new();visit(root,root,&mut hash)?;Ok(format!("{:x}",hash.finalize()))
}
