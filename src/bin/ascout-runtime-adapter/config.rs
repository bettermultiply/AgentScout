//! Runtime startup configuration and compatibility normalization.

use std::{error::Error, fs, path::Path, sync::Arc};

use ascout::runtime::{
    BackendOperation, BackendPolicy, BackendRoute, BaseRuntime,
    backend_registry::{BackendInterface, BackendRegistry},
    backends::{builtin, resolve_executable},
    proxy::RuntimeProxy,
};
use serde_json::Value;

pub(super) fn assemble(path: &Path) -> Result<(RuntimeProxy, String), Box<dyn Error>> {
    let value: Value = serde_json::from_slice(&fs::read(path)?)?;
    let control_address = required_string(&value, "control_address")?.to_owned();
    let adapter_instance = required_string(&value, "adapter_instance")?.to_owned();
    let mut registry = BackendRegistry::default();
    let mut register = |id: &str,
                        kind: &str,
                        config: &Value,
                        executable_entry: Option<&Value>,
                        required: &[BackendInterface]| {
        let implementation = if kind == "executable" {
            resolve_executable(
                path,
                executable_entry.ok_or("executable backend requires a backends entry")?,
            )?
        } else {
            builtin::resolve(kind, config)?
        };
        registry.register(id, implementation, required)?;
        Ok::<(), Box<dyn Error>>(())
    };
    use BackendInterface::{Environment, Filesystem};
    let legacy_filesystem_only =
        value.get("backends").is_none() && value["backend"].as_str() == Some("filesystem");
    let runtime_id = if value.get("backends").is_some() {
        value["base_runtime"]
            .as_str()
            .unwrap_or("runtime")
            .to_owned()
    } else {
        "runtime".to_owned()
    };
    let mut policy = if let Some(entries) = value.get("backends") {
        let entries = entries
            .as_array()
            .ok_or("runtime config backends must be an array")?;
        if entries.is_empty() {
            return Err("runtime config backends must not be empty".into());
        }
        for entry in entries {
            register(
                required_string(entry, "id")?,
                required_string(entry, "type")?,
                entry
                    .get("config")
                    .ok_or("runtime backend entry missing config")?,
                Some(entry),
                &[Environment],
            )?;
        }
        match value.get("policy") {
            Some(policy) => serde_json::from_value(policy.clone())?,
            None if entries.len() == 1 => {
                BackendPolicy::single(required_string(&entries[0], "id")?)
            }
            None => BackendPolicy::default(),
        }
    } else if legacy_filesystem_only {
        let config = value
            .get("filesystem")
            .ok_or("filesystem backend requires filesystem configuration")?;
        register(
            "filesystem",
            "filesystem",
            config,
            None,
            &[Environment, Filesystem],
        )?;
        BackendPolicy::single("filesystem")
    } else {
        let kind = value["backend"].as_str().unwrap_or("firecracker");
        register(kind, kind, &value, None, &[Environment])?;
        BackendPolicy::single(kind)
    };
    if !legacy_filesystem_only && let Some(config) = value.get("filesystem") {
        register(
            "filesystem",
            "filesystem",
            config,
            None,
            &[Environment, Filesystem],
        )?;
        add_legacy_filesystem_routes(&mut policy);
    }
    let runtime = Arc::new(BaseRuntime::from_registry(runtime_id, policy, registry)?);
    Ok((
        RuntimeProxy {
            adapter_instance,
            runtime,
        },
        control_address,
    ))
}

fn required_string<'a>(value: &'a Value, field: &str) -> Result<&'a str, Box<dyn Error>> {
    value[field]
        .as_str()
        .ok_or_else(|| format!("runtime config missing {field}").into())
}

// Translate the legacy assembly input into ordered routes before any instance starts.
fn add_legacy_filesystem_routes(policy: &mut BackendPolicy) {
    use BackendOperation::*;
    let original = policy.routes.clone();
    for operation in [
        Establish,
        Branch,
        Promote,
        Materialize,
        Suspend,
        Resume,
        Terminate,
        Launch,
        EnvironmentOperation,
        FilesystemFork,
        FilesystemMark,
        FilesystemDiff,
        FilesystemAccept,
        FilesystemDiscard,
    ] {
        policy
            .routes
            .retain(|route| route.operation != Some(operation));
        if matches!(
            operation,
            EnvironmentOperation
                | FilesystemFork
                | FilesystemMark
                | FilesystemDiff
                | FilesystemAccept
                | FilesystemDiscard
        ) {
            policy.routes.push({
                let mut route = BackendRoute::default();
                route.operation = Some(operation);
                route.backends.push("filesystem".into());
                route.with_specificity(4)
            });
            continue;
        }
        let source = if operation == Launch {
            Operate
        } else {
            operation
        };
        let routes = original
            .iter()
            .filter(|route| route.operation.is_none() || route.operation == Some(source));
        let fallback = {
            let mut route = BackendRoute::default();
            route.backends.clone_from(&policy.default);
            route
        };
        for (route, score) in routes
            .map(|route| (route, route.specificity() + 4))
            .chain((!fallback.backends.is_empty()).then_some((&fallback, 1)))
        {
            let mut route = route.clone();
            route.operation = Some(operation);
            let mut route = route.with_specificity(score);
            if matches!(operation, Establish | Branch) {
                route.backends.insert(0, "filesystem".into());
            } else {
                route.backends.push("filesystem".into());
            }
            policy.routes.push(route);
        }
    }
}
