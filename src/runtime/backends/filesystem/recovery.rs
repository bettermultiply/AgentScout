use super::*;
use serde_json::{json, Value};

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
        if self.mechanism != FilesystemMechanism::Overlay || self.snapshot_copies {
            return Err(BackendError::new("Filesystem recovery currently requires portable overlay storage (no btrfs subvolumes)"));
        }
        let state=self.lock()?;
        let directory=path.with_extension("filesystem");
        fs::create_dir(&directory).map_err(error)?;
        let environments: Vec<_>=state.environments.values().map(|e|json!({"root":e.root,"metadata":EnvironmentMetadata::from_environment(e)})).collect();
        let snapshots: Vec<_>=state.snapshots.values().map(SnapshotMetadata::from_record).collect();
        let mut resources=BTreeMap::new();
        for env in state.environments.values() {
            for mount in env.mounts.values() {
                for resource in std::iter::once(&mount.target).chain(mount.lower.iter()) {
                    if !resource.starts_with(&self.config.state_directory) {
                        let info=fs::metadata(resource).map_err(error)?;
                        resources.insert(resource.display().to_string(),json!({"dev":info.dev(),"ino":info.ino()}));
                    }
                }
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
        let excluded=state.environments.values().flat_map(|e|e.mounts.values().map(|m|m.work.clone())).collect();
        for name in &roots {
            clone_upper_excluding(&self.config.state_directory.join(name), &directory.join(name), self.reflink_available,&excluded)?;
        }
        let mut record=json!({"version":1,"kind":"filesystem-overlay","root":directory,"roots":roots,"content_sha256":tree_digest(&directory)?,"environments":environments,"snapshots":snapshots,"operations":state.operations,"resources":resources});
        relocate(&mut record, &self.config.state_directory, &directory);
        fs::write(path,serde_json::to_vec(&record).map_err(error)?).map_err(error)
    }

    pub(super) fn load_recovery(&self, path: &Path) -> Result<(), BackendError> {
        if self.mechanism != FilesystemMechanism::Overlay || self.snapshot_copies {
            return Err(BackendError::new("Saved portable overlay storage is incompatible with current filesystem mechanism"));
        }
        let mut record:Value=serde_json::from_slice(&fs::read(path).map_err(error)?).map_err(error)?;
        if record["version"]!=1 || record["kind"]!="filesystem-overlay" {return Err(BackendError::new("Unsupported filesystem recovery version"));}
        let source=path.with_extension("filesystem");
        if record["root"]!=source.display().to_string() {return Err(BackendError::new("Filesystem recovery resource binding mismatch"));}
        if record["content_sha256"]!=tree_digest(&source)? {return Err(BackendError::new("Filesystem recovery content changed"));}
        for (resource,identity) in record["resources"].as_object().ok_or_else(||BackendError::new("Missing filesystem resources"))? {
            let info=fs::metadata(resource).map_err(error)?;
            if identity["dev"]!=info.dev() || identity["ino"]!=info.ino() {return Err(BackendError::new(format!("Filesystem external resource identity changed: {resource}")));}
        }
        // Never overwrite the previous live tree or the save. All restored uppers are independent.
        let target=self.config.state_directory.join(format!("restored-{}",now_nanos()));
        fs::create_dir(&target).map_err(error)?;
        let roots:Vec<String>=serde_json::from_value(record["roots"].clone()).map_err(error)?;
        for name in roots {validate_component(&name)?;clone_upper(&source.join(&name),&target.join(&name),self.reflink_available)?;}
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
                if !mount.upper.starts_with(&target) || !mount.work.starts_with(&target) || !mount.upper.is_dir() || mount.lower.iter().any(|p|!p.is_dir()) {
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
    }
}

fn tree_digest(root: &Path) -> Result<String,BackendError> {
    fn visit(path:&Path, root:&Path, hash:&mut Sha256)->Result<(),BackendError> {
        let metadata=fs::symlink_metadata(path).map_err(error)?;
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
