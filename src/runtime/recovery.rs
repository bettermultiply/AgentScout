use super::backend::{Backend, BackendError};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn error(e: impl std::fmt::Display) -> BackendError {
    BackendError::new(e.to_string())
}

pub(super) trait Component {
    fn save_after(
        &self,
        path: &Path,
        previous: &mut dyn FnMut() -> Result<(), BackendError>,
    ) -> Result<(), BackendError>;
    fn load(&self, path: &Path) -> Result<(), BackendError>;
    fn shutdown(&self) -> Result<(), BackendError>;
}

impl Component for &dyn Backend {
    fn save_after(
        &self,
        path: &Path,
        previous: &mut dyn FnMut() -> Result<(), BackendError>,
    ) -> Result<(), BackendError> {
        self.save_runtime_after(path, previous)
    }
    fn load(&self, path: &Path) -> Result<(), BackendError> {
        self.load_runtime(path)
    }
    fn shutdown(&self) -> Result<(), BackendError> {
        Backend::shutdown(*self)
    }
}

pub(super) fn save(
    path: &Path,
    backends: &[(&str, &dyn Component)],
    metadata: Value,
) -> Result<(), BackendError> {
    let directory = path.with_extension("components");
    fs::create_dir(&directory).map_err(error)?;
    let mut components = Vec::new();
    save_components(&directory, backends, &mut components)?;
    fs::write(path, serde_json::to_vec(&json!({"version":1,"kind":"runtime-components","components":components,"metadata":metadata})).map_err(error)?).map_err(error)
}

fn save_components(
    directory: &Path,
    backends: &[(&str, &dyn Component)],
    components: &mut Vec<Value>,
) -> Result<(), BackendError> {
    let Some(((id, backend), previous)) = backends.split_last() else {
        return Ok(());
    };
    let name = format!("{}.json", previous.len());
    let file = directory.join(&name);
    backend.save_after(&file, &mut || {
        save_components(directory, previous, components)
    })?;
    let bytes = fs::read(&file).map_err(error)?;
    components.push(json!({"id":id,"file":name,"sha256":format!("{:x}",Sha256::digest(bytes))}));
    Ok(())
}

fn record(path: &Path) -> Result<Value, BackendError> {
    serde_json::from_slice(&fs::read(path).map_err(error)?).map_err(error)
}

fn component_files(
    path: &Path,
    record: &Value,
    ids: &[&str],
) -> Result<Vec<PathBuf>, BackendError> {
    let parts = record["components"]
        .as_array()
        .ok_or_else(|| error("Missing runtime components"))?;
    if record["version"] != 1 || record["kind"] != "runtime-components" || parts.len() != ids.len()
    {
        return Err(error("Incompatible runtime component set"));
    }
    let mut files = Vec::with_capacity(ids.len());
    for (index, (id, part)) in ids.iter().zip(parts).enumerate() {
        let name = format!("{index}.json");
        if part["id"] != *id || part["file"] != name {
            return Err(error("Runtime component identity mismatch"));
        }
        let file = path.with_extension("components").join(name);
        let bytes = fs::read(&file).map_err(error)?;
        if part["sha256"] != format!("{:x}", Sha256::digest(bytes)) {
            return Err(error("Runtime component checksum mismatch"));
        }
        files.push(file);
    }
    Ok(files)
}

pub(super) fn load(
    path: &Path,
    backends: &[(&str, &dyn Component)],
) -> Result<Value, BackendError> {
    let root = record(path)?;
    let ids: Vec<_> = backends.iter().map(|(id, _)| *id).collect();
    let (files, metadata) = if ids.len() == 1 && root["kind"] != "runtime-components" {
        // A single component may have been saved directly at the root path.
        (vec![path.to_owned()], Value::Null)
    } else if ids.len() > 1
        && root["metadata"].is_null()
        && root["components"][0]["id"] == "inner"
        && root["components"][1]["id"] == "data-plane"
    {
        let outer = component_files(path, &root, &["inner", "data-plane"])?;
        let inner = record(&outer[0])?;
        let mut files = component_files(&outer[0], &inner, &ids[..ids.len() - 1])?;
        files.push(outer[1].clone());
        (
            files,
            json!({"previous_participants": inner["metadata"]["participants"]}),
        )
    } else {
        (
            component_files(path, &root, &ids)?,
            root["metadata"].clone(),
        )
    };
    for ((_, backend), file) in backends.iter().zip(files) {
        if let Err(failure) = backend.load(&file) {
            let cleanup: Vec<_> = backends
                .iter()
                .rev()
                .filter_map(|(id, b)| b.shutdown().err().map(|e| format!("{id}: {e}")))
                .collect();
            return Err(error(format!("{failure}; load cleanup: {cleanup:?}")));
        }
    }
    Ok(metadata)
}
