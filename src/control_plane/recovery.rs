//! Recovery components, association validation and atomic manifest publication.
use crate::control_plane::agent_control::{BranchStatus, SpeculativeTree};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::Path;
use std::{fs, io};

/// Restore launch configuration before constructing an Agent session. An
/// explicitly selected model is applied only through the normal idle path.
pub fn models(
    run_dir: &Path,
    generation: Option<&Path>,
    model: Option<String>,
    mut models: Option<std::path::PathBuf>,
) -> io::Result<(Option<String>, Option<String>, Option<std::path::PathBuf>)> {
    let mut startup_model = model.clone();
    let mut resume_model = None;
    if let Some(generation) = generation {
        let record: Value = serde_json::from_slice(&fs::read(generation.join("agent.json"))?)?;
        if let Some(config) = record.get("executionConfig") {
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
            fs::write(&path, serde_json::to_vec_pretty(extra)?)?;
            models = Some(path);
        }
    } else if let Some(source) = &models {
        let bytes = fs::read(source)?;
        let _: Value = serde_json::from_slice(&bytes)?;
        let path = run_dir.join("models.json");
        fs::write(&path, bytes)?;
        models = Some(path);
    }
    Ok((startup_model, resume_model, models))
}

pub fn validate_policy_artifact(tree: &SpeculativeTree, bytes: &[u8]) -> io::Result<()> {
    if tree
        .policy_binding
        .as_ref()
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
    if tree.local_policy_digest.as_deref() != Some(policy_digest) {
        return Err(io::Error::other("Recovery Policy changed"));
    }
    if tree
        .runtime_binding
        .as_ref()
        .and_then(|v| v.get("config_path"))
        != Some(&json!(config_path))
    {
        return Err(io::Error::other("Recovery Runtime binding changed"));
    }
    let config_digest = digest(config_path)?;
    if tree.runtime_binding.as_ref().and_then(|v| v.get("sha256")) != Some(&json!(config_digest)) {
        return Err(io::Error::other("Recovery Runtime configuration changed"));
    }
    Ok(config_digest)
}

pub fn validate_capabilities(
    tree: &SpeculativeTree,
    capabilities: &crate::protocol::contract::RuntimeCapabilities,
) -> io::Result<()> {
    if tree.capability_view.as_ref() != Some(&serde_json::to_value(capabilities)?) {
        return Err(io::Error::other("Recovery Runtime capabilities changed"));
    }
    Ok(())
}

pub fn digest(path: &Path) -> io::Result<String> {
    Ok(format!("{:x}", Sha256::digest(fs::read(path)?)))
}

pub fn publish(run_dir: &Path, generation: &Path, tree: &SpeculativeTree) -> io::Result<()> {
    let tree_path = generation.join("tree.json");
    write(&tree_path, &serde_json::to_vec_pretty(tree)?)?;
    let files = ["tree.json", "agent.json", "runtime.json"]
        .into_iter()
        .map(|name| {
            Ok((
                name.to_string(),
                json!({"path":generation.join(name),"sha256":digest(&generation.join(name))?}),
            ))
        })
        .collect::<io::Result<serde_json::Map<String, Value>>>()?;
    write(
        &run_dir.join("recovery.json"),
        &serde_json::to_vec_pretty(&json!({"version":1,"files":files}))?,
    )
}

pub fn load(run_dir: &Path) -> io::Result<(SpeculativeTree, std::path::PathBuf)> {
    let record: Value = serde_json::from_slice(&fs::read(run_dir.join("recovery.json"))?)?;
    if record["version"] != 1 {
        return Err(io::Error::other("Unsupported recovery version"));
    }
    let mut generation = None;
    for name in ["tree.json", "agent.json", "runtime.json"] {
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
    let mut tree: SpeculativeTree =
        serde_json::from_slice(&fs::read(generation.join("tree.json"))?)?;
    validate(&tree, &generation)?;
    if Some(tree.execution_id.as_str()) != run_dir.file_name().and_then(|name| name.to_str()) {
        return Err(io::Error::other("Recovery task identity mismatch"));
    }
    tree.load_recovery();
    Ok((tree, generation))
}

fn validate(tree: &SpeculativeTree, generation: &Path) -> io::Result<()> {
    let fail =
        |message: &str| io::Error::other(format!("Recovery association mismatch: {message}"));
    let agent: Value = serde_json::from_slice(&fs::read(generation.join("agent.json"))?)?;
    if agent["taskId"] != tree.execution_id
        || agent["authority"] != serde_json::to_value(&tree.authority_branch_id)?
    {
        return Err(fail("task/authority"));
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
                    crate::control_plane::agent_control::RuntimeExecutionState::Unknown
                    | crate::control_plane::agent_control::RuntimeExecutionState::EnvironmentRequested)) {
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
        ) {
            continue;
        }
        let Some(env) = branch.environment_id.as_deref() else {
            if branch.status == BranchStatus::Created {
                continue;
            }
            return Err(fail("live Branch environment missing"));
        };
        // A CompositeRuntime may legitimately associate an environment with
        // only the Backend instances selected for that environment. The
        // Runtime has already validated the concrete participant metadata;
        // recovery only requires the environment to exist in at least one
        // saved component, not in every component.
        if sets.is_empty() || !sets.iter().any(|s| s.contains(env)) {
            return Err(fail("live Branch environment missing"));
        }
    }
    Ok(())
}

fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temporary = path.with_extension("tmp");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    if fs::read(&temporary)? != bytes {
        return Err(io::Error::other("Recovery readback failed"));
    }
    fs::rename(temporary, path)?;
    fs::File::open(path.parent().unwrap())?.sync_all()
}
