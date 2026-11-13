//! Component saves are private to one generation. Validate all bytes before loading any backend.
use std::{fs, path::{Path, PathBuf}};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use super::backend::{Backend, BackendError};

fn error(e: impl std::fmt::Display) -> BackendError { BackendError::new(e.to_string()) }

pub(super) fn save(path: &Path, backends: &[(&str, &dyn Backend)], metadata: Value) -> Result<(), BackendError> {
    let directory = path.with_extension("components");
    fs::create_dir(&directory).map_err(error)?;
    let mut components = Vec::new();
    for (index, (id, backend)) in backends.iter().enumerate() {
        let file = directory.join(format!("{index}.json"));
        backend.save_runtime(&file)?;
        let bytes = fs::read(&file).map_err(error)?;
        components.push(json!({"id":id,"file":format!("{index}.json"),"sha256":format!("{:x}",Sha256::digest(bytes))}));
    }
    fs::write(path, serde_json::to_vec(&json!({"version":1,"kind":"runtime-components","components":components,"metadata":metadata})).map_err(error)?).map_err(error)
}

pub(super) fn load(path: &Path, backends: &[(&str, &dyn Backend)]) -> Result<Value, BackendError> {
    let record: Value = serde_json::from_slice(&fs::read(path).map_err(error)?).map_err(error)?;
    let parts = record["components"].as_array().ok_or_else(|| error("Missing runtime components"))?;
    if record["version"] != 1 || record["kind"] != "runtime-components" || parts.len() != backends.len() {
        return Err(error("Incompatible runtime component set"));
    }
    let mut files: Vec<PathBuf> = Vec::new();
    for (index, ((id, _), part)) in backends.iter().zip(parts).enumerate() {
        if part["id"] != *id || part["file"] != format!("{index}.json") { return Err(error("Runtime component identity mismatch")); }
        let file = path.with_extension("components").join(format!("{index}.json"));
        let bytes = fs::read(&file).map_err(error)?;
        if part["sha256"] != format!("{:x}", Sha256::digest(bytes)) { return Err(error("Runtime component checksum mismatch")); }
        files.push(file);
    }
    for ((_, backend), file) in backends.iter().zip(files) { backend.load_runtime(&file)?; }
    Ok(record["metadata"].clone())
}
