//! Recovery components, association validation and atomic manifest publication.
use crate::control_plane::speculative_tree::{BranchStatus, SpeculativeTree};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::{fs, io};

/// Restore launch configuration before constructing an Agent session. An
/// explicitly selected model is applied only through the normal idle path.
pub fn models(
    run_dir: &Path,
    generation: Option<&Path>,
    reopening: bool,
    model: Option<String>,
    mut models: Option<PathBuf>,
    mut thinking: Option<String>,
) -> io::Result<(
    Option<String>,
    Option<String>,
    Option<PathBuf>,
    Option<String>,
)> {
    let mut startup_model = model.clone();
    let mut resume_model = None;
    if let Some(generation) = generation {
        let record: Value = serde_json::from_slice(&fs::read(generation.join("agent.json"))?)?;
        if let Some(config) = record.get("executionConfig") {
            if thinking.is_none() {
                thinking = config["thinkingLevel"].as_str().map(str::to_owned);
            }
            startup_model = config
                .get("modelPattern")
                .and_then(Value::as_str)
                .map(str::to_owned);
            if model != startup_model {
                resume_model = model;
            }
        }
        if let Some(extra) = record
            .get("additionalModels")
            .filter(|value| !value.is_null())
        {
            if let Some(path) = &models {
                let supplied: Value = serde_json::from_slice(&fs::read(path)?)?;
                if &supplied != extra {
                    return Err(io::Error::other(
                        "Recovery additional model configuration changed",
                    ));
                }
            }
            let path = run_dir.join("models.json");
            if !path.is_file() || serde_json::from_slice::<Value>(&fs::read(&path)?)? != *extra {
                fs::write(&path, serde_json::to_vec_pretty(extra)?)?;
            }
            models = Some(path);
        }
    } else if reopening {
        let saved = run_dir.join("models.json");
        let persisted = match fs::read(&saved) {
            Ok(bytes) => Some(serde_json::from_slice::<Value>(&bytes)?),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        if let Some(path) = &models {
            let supplied: Value = serde_json::from_slice(&fs::read(path)?)?;
            if persisted.as_ref() != Some(&supplied) {
                return Err(io::Error::other(
                    "Saved additional model configuration changed",
                ));
            }
        }
        models = persisted.map(|_| saved);
    } else if let Some(source) = &models {
        let bytes = fs::read(source)?;
        let _: Value = serde_json::from_slice(&bytes)?;
        let path = run_dir.join("models.json");
        fs::write(&path, bytes)?;
        models = Some(path);
    }
    Ok((startup_model, resume_model, models, thinking))
}

pub fn validate_policy_artifact(tree: &SpeculativeTree, bytes: &[u8]) -> io::Result<()> {
    if tree
        .policy_binding()
        .and_then(|binding| binding.get("sha256"))
        != Some(&json!(format!("{:x}", Sha256::digest(bytes))))
    {
        return Err(io::Error::other("Recovery Policy artifact changed"));
    }
    Ok(())
}

pub fn validate_bindings(
    tree: &SpeculativeTree,
    policy_digest: &str,
    config_path: &Path,
) -> io::Result<String> {
    if tree.local_policy_digest() != Some(policy_digest) {
        return Err(io::Error::other("Recovery Policy changed"));
    }
    if tree.runtime_binding().and_then(|v| v.get("config_path")) != Some(&json!(config_path)) {
        return Err(io::Error::other("Recovery Runtime binding changed"));
    }
    let config_digest = digest(config_path)?;
    if tree.runtime_binding().and_then(|v| v.get("sha256")) != Some(&json!(config_digest)) {
        return Err(io::Error::other("Recovery Runtime configuration changed"));
    }
    Ok(config_digest)
}

pub fn validate_capabilities(
    tree: &SpeculativeTree,
    capabilities: &crate::protocol::contract::RuntimeCapabilities,
) -> io::Result<()> {
    if tree.capability_view() != Some(&serde_json::to_value(capabilities)?) {
        return Err(io::Error::other("Recovery Runtime capabilities changed"));
    }
    Ok(())
}

pub fn digest(path: &Path) -> io::Result<String> {
    Ok(format!("{:x}", Sha256::digest(fs::read(path)?)))
}

pub fn publish(run_dir: &Path, generation: &Path, tree: &SpeculativeTree) -> io::Result<()> {
    // Component writes alone do not establish a recoverable generation.
    // Apply the same association checks used by load before replacing the
    // last known-good completion marker.
    validate(tree, generation)?;
    for name in ["agent.json", "runtime.json"] {
        fs::File::open(generation.join(name))?.sync_all()?;
    }
    fs::File::open(generation)?.sync_all()?;
    fs::File::open(run_dir)?.sync_all()?;
    let files = ["agent.json", "runtime.json"]
        .into_iter()
        .map(|name| {
            Ok((
                name.to_string(),
                json!({"path":generation.join(name),"sha256":digest(&generation.join(name))?}),
            ))
        })
        .collect::<io::Result<serde_json::Map<String, Value>>>()?;
    let manifest = json!({"version":1,"files":files});
    // Opening reuses the process-owned writer, serializing append, sync and
    // rollback with ordinary events without allocating an observation sequence.
    let mut ledger = super::ledger::Ledger::open(&run_dir.join("events.jsonl"))?;
    let temporary = run_dir.join("recovery.tmp");
    let bytes = serde_json::to_vec_pretty(&manifest)?;
    let mut index = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    index.write_all(&bytes)?;
    index.sync_all()?;
    if let Err(error) = ledger.record_durable(
        json!({"message":"recovery_complete", "task_id":tree.execution_id(),
        "control":tree, "manifest":manifest}),
    ) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    // Compatibility index only. Ledger is the durable completion boundary.
    if let Err(error) = fs::rename(&temporary, run_dir.join("recovery.json")) {
        ledger.warning(&format!(
            "Recovery Ledger saved; compatibility index update failed: {error}"
        ));
    }
    Ok(())
}

#[derive(Debug)]
pub enum Resume {
    Paused(PathBuf),
    Ended(Value),
}

pub fn load(run_dir: &Path) -> io::Result<(SpeculativeTree, Resume)> {
    use std::io::BufRead;
    let mut records = Vec::new();
    for line in io::BufReader::new(fs::File::open(run_dir.join("events.jsonl"))?).lines() {
        if let Ok(record) = serde_json::from_str::<Value>(&line?) {
            records.push(record);
        }
    }
    let mut latest_error = None;
    for (index, record) in records.iter().enumerate().rev() {
        match record["message"].as_str() {
            Some("task_started") => break,
            Some("task_completed") => {
                if latest_error.is_none() {
                    return open_ended(run_dir, &records, index);
                }
                break;
            }
            Some("recovery_complete") => {
                match load_record(run_dir, &record["manifest"], &record["control"]) {
                    Ok((tree, path)) => return Ok((tree, Resume::Paused(path))),
                    Err(error) => {
                        latest_error.get_or_insert(error);
                    }
                }
            }
            _ => {}
        }
    }
    Err(latest_error.unwrap_or_else(|| {
        io::Error::other("No complete Ledger recovery record for the latest task")
    }))
}

fn open_ended(
    run_dir: &Path,
    records: &[Value],
    completed: usize,
) -> io::Result<(SpeculativeTree, Resume)> {
    // Release records after this completion identify the assembled authority.
    let reference = records[completed + 1..]
        .iter()
        .rev()
        .find(|record| {
            record["message"] == "agent_released"
                && record["task_id"] == records[completed]["task_id"]
        })
        .map(|record| record["session"].clone())
        .unwrap_or_else(|| json!({"file": run_dir.join("session.jsonl")}));
    let file = reference["file"]
        .as_str()
        .ok_or_else(|| io::Error::other("Ended run is missing its Pi session reference"))?;
    if !Path::new(file).is_absolute()
        || !fs::metadata(file).is_ok_and(|metadata| metadata.is_file())
    {
        return Err(io::Error::other(format!(
            "Ended run Pi session reference is unavailable: {file}"
        )));
    }

    let run_id = run_dir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::other("Invalid run identity"))?;
    let bindings = records
        .iter()
        .rev()
        .find(|record| record["message"] == "session_file" && record.get("bindings").is_some());
    // Older runs wrote these bindings at shutdown. Never load their execution state.
    let legacy;
    let bindings = if let Some(record) = bindings {
        if record["run_id"] != run_id {
            return Err(io::Error::other("Ended run identity mismatch"));
        }
        &record["bindings"]
    } else {
        legacy = serde_json::from_slice::<Value>(&fs::read(run_dir.join("final-tree.json"))?)?;
        if legacy["execution_id"] != run_id {
            return Err(io::Error::other("Ended run identity mismatch"));
        }
        &legacy
    };
    let tree = SpeculativeTree::reopen_ended(run_dir, run_id, records.len(), bindings)?;
    Ok((tree, Resume::Ended(reference)))
}

fn load_record(
    run_dir: &Path,
    record: &Value,
    control: &Value,
) -> io::Result<(SpeculativeTree, std::path::PathBuf)> {
    if record["version"] != 1 {
        return Err(io::Error::other("Unsupported recovery version"));
    }
    let mut generation = None;
    for name in ["agent.json", "runtime.json"] {
        let file = &record["files"][name];
        let path = std::path::PathBuf::from(
            file["path"]
                .as_str()
                .ok_or_else(|| io::Error::other("Missing recovery component"))?,
        );
        let path = fs::canonicalize(path)?;
        if !path.starts_with(run_dir)
            || path.file_name().and_then(|s| s.to_str()) != Some(name)
            || file["sha256"] != digest(&path)?
        {
            return Err(io::Error::other("Recovery component mismatch"));
        }
        let parent = path.parent().unwrap().to_path_buf();
        if generation
            .as_ref()
            .is_some_and(|previous| previous != &parent)
        {
            return Err(io::Error::other("Mixed recovery generations"));
        }
        generation = Some(parent);
    }
    let generation = generation.unwrap();
    let mut tree: SpeculativeTree = serde_json::from_value(control.clone())?;
    validate(&tree, &generation)?;
    let run_id = run_dir.file_name().and_then(|name| name.to_str());
    tree.validate_recovery_identity(run_id)?;
    tree.load_recovery();
    Ok((tree, generation))
}

fn validate(tree: &SpeculativeTree, generation: &Path) -> io::Result<()> {
    let fail =
        |message: &str| io::Error::other(format!("Recovery association mismatch: {message}"));
    let agent: Value = serde_json::from_slice(&fs::read(generation.join("agent.json"))?)?;
    if agent["taskId"] != tree.execution_id()
        || agent["authority"] != serde_json::to_value(&tree.authority_branch_id)?
    {
        return Err(fail("task/authority"));
    }
    for source in agent["tree"]["sources"]
        .as_array()
        .ok_or_else(|| fail("Pi session references missing"))?
    {
        let path = source["file"]
            .as_str()
            .ok_or_else(|| fail("Pi session reference missing"))?;
        if source["checksum"] != digest(Path::new(path))? {
            return Err(fail("Pi session reference changed"));
        }
    }
    let branches = agent["tree"]["branches"]
        .as_array()
        .ok_or_else(|| fail("native branches missing"))?;
    let native_ids: std::collections::BTreeSet<_> =
        branches.iter().filter_map(|b| b[0].as_str()).collect();
    let mut bound_environments = std::collections::BTreeSet::new();
    for (id, branch) in &tree.branches {
        if let Some(environment_id) = &branch.environment_id {
            if environment_id.is_empty() || !bound_environments.insert(environment_id) {
                return Err(fail("Branch Environment binding is empty or shared"));
            }
        } else if !matches!(branch.status, BranchStatus::Cleaned | BranchStatus::Discarded)
            && !(branch.status == BranchStatus::Created
                && matches!(branch.execution_state.runtime,
                    crate::control_plane::speculative_tree::RuntimeExecutionState::Unknown
                    | crate::control_plane::speculative_tree::RuntimeExecutionState::EnvironmentRequested)) {
            return Err(fail("live Branch Environment binding is missing"));
        }
        if id != &branch.branch_id {
            return Err(fail("branch key"));
        }
        // Released branches can remain in the native tree as checkpoint sources.
        let released = matches!(
            branch.status,
            BranchStatus::Cleaned | BranchStatus::Discarded
        );
        if !released && branch.environment_id.is_some() && !native_ids.contains(id.as_str()) {
            return Err(fail("branch session missing"));
        }
        if let Some(parent) = &branch.parent_branch_id {
            if parent == id {
                return Err(fail("branch parent cycle"));
            }
        }
        for assumption in &branch.assumption_set {
            let chance = tree
                .chances
                .get(&assumption.chance_id)
                .ok_or_else(|| fail("assumption Chance missing"))?;
            if !chance
                .proposals
                .iter()
                .any(|p| p.proposal_id == assumption.proposal_id)
            {
                return Err(fail("assumption Proposal missing"));
            }
        }
    }
    for (id, chance) in &tree.chances {
        if id != &chance.chance_id || !tree.branches.contains_key(&chance.parent_branch_id) {
            return Err(fail("Chance parent"));
        }
        let mut proposals = std::collections::BTreeSet::new();
        if chance
            .proposals
            .iter()
            .any(|p| !proposals.insert(&p.proposal_id))
        {
            return Err(fail("duplicate Proposal"));
        }
        for branch in &chance.branch_ids {
            if let Some(child) = tree.branches.get(branch) {
                if child.chance_id.as_ref() != Some(id) {
                    return Err(fail("Chance child"));
                }
            }
        }
    }
    fn environments(
        path: &Path,
        sets: &mut Vec<std::collections::BTreeSet<String>>,
    ) -> io::Result<()> {
        let record: Value = serde_json::from_slice(&fs::read(path)?)?;
        if record["kind"] == "runtime-components" {
            for (index, _) in record["components"]
                .as_array()
                .ok_or_else(|| io::Error::other("Missing runtime components"))?
                .iter()
                .enumerate()
            {
                environments(
                    &path
                        .with_extension("components")
                        .join(format!("{index}.json")),
                    sets,
                )?;
            }
        } else if record["kind"] == "filesystem-overlay" {
            sets.push(
                record["environments"]
                    .as_array()
                    .ok_or_else(|| io::Error::other("Missing filesystem environments"))?
                    .iter()
                    .filter_map(|e| e["metadata"]["id"].as_str().map(str::to_string))
                    .collect(),
            );
        } else if let Some(envs) = record["environments"].as_object() {
            let worlds = record["worlds"]
                .as_array()
                .ok_or_else(|| io::Error::other("Missing NLE worlds"))?;
            for index in envs.values() {
                if !index.as_u64().is_some_and(|i| (i as usize) < worlds.len()) {
                    return Err(io::Error::other("Recovery NLE environment world missing"));
                }
            }
            sets.push(envs.keys().cloned().collect());
        } else {
            return Err(io::Error::other(
                "Unsupported runtime recovery environment schema",
            ));
        }
        Ok(())
    }
    let mut sets = Vec::new();
    environments(&generation.join("runtime.json"), &mut sets)?;
    for branch in tree.branches.values() {
        if matches!(
            branch.status,
            BranchStatus::Cleaned | BranchStatus::Discarded
        ) && !tree.has_live_environment_dependents(&branch.branch_id)
        {
            continue;
        }
        let Some(env) = branch.environment_id.as_deref() else {
            if branch.status == BranchStatus::Created {
                continue;
            }
            return Err(fail("live Branch environment missing"));
        };
        // An Environment can be associated with multiple Backend instances.
        // The Runtime has already validated the concrete participant metadata;
        // recovery only requires the environment to exist in at least one
        // saved component, not in every component.
        if sets.is_empty() || !sets.iter().any(|s| s.contains(env)) {
            return Err(fail("live Branch environment missing"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_latest_generation_cannot_cross_task_completion() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("events.jsonl"),
            concat!(
                "{\"message\":\"recovery_complete\"}\n",
                "{\"message\":\"task_completed\"}\n",
                "{\"message\":\"recovery_complete\",\"manifest\":{\"version\":1},\"control\":{}}\n"
            ),
        )
        .unwrap();
        assert!(
            load(dir.path())
                .unwrap_err()
                .to_string()
                .contains("Missing recovery component")
        );
    }

    #[test]
    fn inconsistent_components_do_not_replace_completion_marker() {
        let directory = tempfile::tempdir().unwrap();
        let generation = directory.path().join("generation");
        fs::create_dir(&generation).unwrap();
        for name in ["agent.json", "runtime.json"] {
            fs::write(generation.join(name), b"{}").unwrap();
        }
        let marker = directory.path().join("recovery.json");
        fs::write(&marker, b"previous complete generation").unwrap();
        let tree = SpeculativeTree::new("agent-1".into());
        assert!(publish(directory.path(), &generation, &tree).is_err());
        assert_eq!(fs::read(marker).unwrap(), b"previous complete generation");
    }
}
