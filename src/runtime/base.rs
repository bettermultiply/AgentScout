
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

use serde_json::Value;

use super::agent_supervisor::AgentSupervisor;
use super::backend::{Backend, BackendContext, BackendError, BackendExecution, CancelHandle};
use super::backend_registry::{BackendInterface, BackendRegistry};
use super::launch_plan::LaunchFragment;
use super::lifecycle::{Lifecycle, OperationGuard};
use super::selection::{BackendConnection, BackendOperation, BackendPolicy, merge_launch_fragment};
use crate::protocol::contract::{
    AgentLaunch, BackendState, CommandOutput, EnvironmentState, FilesystemResult, Operation,
    RuntimeCapabilities,
};

pub struct BaseRuntime {
    id: String,
    connections: Vec<BackendConnection>,
    policy: BackendPolicy,
    lifecycle: Arc<Lifecycle>,
    agent_supervisor: AgentSupervisor,
    /// 部分创建失败时仅保留仍持有资源的参与者。清理后保留记录以解释最终状态。
    participants: Mutex<HashMap<String, Vec<String>>>,
    control_operations: Mutex<BTreeSet<String>>,
}

pub(super) struct ControlAdmission<'a> {
    active: &'a Mutex<BTreeSet<String>>,
    environments: BTreeSet<String>,
}

impl Drop for ControlAdmission<'_> {
    fn drop(&mut self) {
        let mut active = self.active.lock().unwrap();
        for environment in &self.environments {
            active.remove(environment);
        }
    }
}

impl BaseRuntime {
    pub fn new(id: impl Into<String>, backend: Arc<dyn Backend>) -> Self {
        let backend_id = backend.backend_id().to_owned();
        Self {
            id: id.into(),
            connections: vec![BackendConnection {
                id: backend_id.clone(),
                backend,
            }],
            policy: BackendPolicy::single(backend_id),
            control_operations: Mutex::default(),
            lifecycle: Arc::new(Lifecycle::default()),
            agent_supervisor: AgentSupervisor::default(),
            participants: Mutex::new(HashMap::new()),
        }
    }

    pub fn with_policy(
        id: impl Into<String>,
        connections: Vec<BackendConnection>,
        policy: BackendPolicy,
    ) -> Result<Self, BackendError> {
        Self::validate_policy(&policy, connections.iter().map(|entry| &entry.id))?;
        Ok(Self {
            id: id.into(),
            connections,
            policy,
            control_operations: Mutex::default(),
            lifecycle: Arc::new(Lifecycle::default()),
            agent_supervisor: AgentSupervisor::default(),
            participants: Mutex::new(HashMap::new()),
        })
    }

    pub fn from_registry(
        id: impl Into<String>,
        policy: BackendPolicy,
        registry: BackendRegistry,
    ) -> Result<Self, BackendError> {
        Self::validate_policy(&policy, registry.entries.iter().map(|entry| &entry.id))?;
        for entry in &registry.entries {
            if policy.routes.iter().any(|route| {
                route.operation.is_some_and(BackendOperation::is_filesystem)
                    && route.backends.contains(&entry.id)
            }) {
                BackendRegistry::require_interface(
                    &entry.id,
                    &entry.implementation,
                    BackendInterface::Filesystem,
                )?;
            }
        }
        let mut connections: Vec<BackendConnection> = Vec::new();
        for entry in registry.entries {
            match (entry.implementation.create)()
                .and_then(|backend| BackendConnection::new(entry.id, backend))
            {
                Ok(connection) => connections.push(connection),
                Err(mut error) => {
                    for previous in connections.iter().rev() {
                        if let Err(cleanup) = previous.backend.shutdown() {
                            error.add_cleanup_failure(&previous.id, cleanup.to_string());
                        }
                    }
                    return Err(error);
                }
            }
        }
        Self::with_policy(id, connections, policy)
    }

    pub(super) fn is_closing(&self) -> bool {
        self.lifecycle.is_closed()
    }

    pub(super) fn enter_operation(&self) -> Result<OperationGuard, &'static str> {
        self.lifecycle.enter(false)
    }

    pub(super) fn admit_control(
        &self,
        environments: impl IntoIterator<Item = String>,
    ) -> Result<ControlAdmission<'_>, &'static str> {
        let environments: BTreeSet<_> = environments.into_iter().collect();
        let mut active = self.control_operations.lock().unwrap();
        if !active.is_disjoint(&environments) {
            return Err("an environment already has an active control operation");
        }
        active.extend(environments.iter().cloned());
        Ok(ControlAdmission {
            active: &self.control_operations,
            environments,
        })
    }

    pub(super) fn launch_agent(
        &self,
        context: &BackendContext,
        launch: &AgentLaunch,
    ) -> Result<(), BackendError> {
        let _operation = self.enter_operation().map_err(BackendError::new)?;
        let fragment = self.launch_fragment(context, launch)?;
        self.agent_supervisor.launch(launch, &fragment.env)?;
        // Reserve startup under the admission lock so cleanup waits for it.
        let _startup = self.enter_operation().map_err(BackendError::new)?;
        self.start_environment_execution(context, launch)
    }

    pub(super) fn accept_environment(
        &self,
        target: &BackendContext,
        source: &str,
        full: bool,
        part: Option<&crate::protocol::contract::PartSelection>,
        target_suspended: bool,
        wait: impl Fn(&str),
    ) -> Result<FilesystemResult, BackendError> {
        use crate::protocol::contract::{Acceptance, PartSelection};
        let source_context = BackendContext::new(source);
        if target_suspended {
            if self.aggregate_state(&target.environment_id) != Some(EnvironmentState::Suspended) {
                return Err(BackendError::new("acceptance target is not suspended"));
            }
        } else {
            self.ensure_available(target)?;
        }
        if let Some(PartSelection::Results { results }) = part
            && !PartSelection::valid_results(results)
        {
            return Err(BackendError::new("invalid part result dependencies"));
        }
        self.ensure_available(&source_context)?;
        let participants = self.participants_for(&target.environment_id);
        if participants != self.participants_for(source) {
            return Err(BackendError::new(
                "accept requires matching Backend participants",
            ));
        }
        let participants = participants.ok_or_else(|| BackendError::new("unknown target"))?;
        let isolated: Vec<_> = self
            .connections
            .iter()
            .filter(|connection| {
                participants.contains(&connection.id)
                    && !connection.backend.acceptance_state_is_shared()
            })
            .collect();
        if !target_suspended {
            wait(&target.environment_id);
            self.suspend_with_context(target)?;
        }
        wait(source);
        let results_only = part.filter(|selection| selection.results_only());
        let receipt = |selection: &PartSelection| FilesystemResult {
            acceptance: Some(selection.results_only_receipt()),
            ..Default::default()
        };
        let mut result = (|| {
            for connection in &isolated {
                connection.backend.drain_acceptance(source)?;
            }
            if !full && let Some(selection) = results_only {
                Ok(receipt(selection))
            } else if isolated.len() > 1 {
                results_only.map(receipt).ok_or_else(|| BackendError::new(
                    "state transfer across multiple isolated Backends is unsupported",
                ))
            } else if let Some(connection) = isolated.first() {
                match connection.backend.accept_state(target, source, full, part) {
                    Err(error) if full && results_only.is_some() && !error.is_partial() => {
                        Ok(receipt(results_only.unwrap()))
                    }
                    receipt => receipt,
                }
            } else if full {
                Ok(FilesystemResult {
                    acceptance: Some(Acceptance::Full),
                    ..Default::default()
                })
            } else {
                Err(BackendError::new(
                    "the selected filesystem scopes have no isolated Backend",
                ))
            }
        })();
        if !target_suspended && let Err(resume) = self.resume_with_context(target) {
            let mut failure = result
                .err()
                .unwrap_or_else(|| BackendError::new("could not resume acceptance target"));
            failure.add_cleanup_failure("runtime", resume.to_string());
            result = Err(failure);
        }
        result
    }

    pub(super) fn branch_environment(
        &self,
        context: &BackendContext,
        launch: Option<&AgentLaunch>,
    ) -> Result<String, BackendError> {
        let child = self.create_child_environment(context)?;
        if let Some(launch) = launch {
            let child_context = BackendContext {
                environment_id: child.clone(),
                parent_environment_id: Some(context.environment_id.clone()),
                ..context.clone()
            };
            self.launch_agent(&child_context, launch)
                .map_err(|mut error| {
                    error.created_environment_id = Some(child.clone());
                    error
                })?;
        }
        Ok(child)
    }

    fn validate_policy<'a>(
        policy: &BackendPolicy,
        ids: impl Iterator<Item = &'a String>,
    ) -> Result<(), BackendError> {
        let mut unique = BTreeSet::new();
        for id in ids {
            if !unique.insert(id.clone()) {
                return Err(BackendError::new(format!(
                    "duplicate Runtime Backend id {id}"
                )));
            }
        }
        if unique.is_empty() {
            return Err(BackendError::new("Runtime requires at least one Backend"));
        }
        policy.validate(&unique)?;
        if unique.len() > 1 && policy.default.is_empty() && policy.routes.is_empty() {
            return Err(BackendError::new(
                "multiple Runtime Backends require an explicit Backend Policy",
            ));
        }
        Ok(())
    }

    fn shutdown_connections(&self) -> Result<(), BackendError> {
        let mut failure = BackendError::new("Runtime shutdown partially failed");
        for connection in self.connections.iter().rev() {
            if let Err(error) = connection.backend.shutdown() {
                failure.add_cleanup_failure(&connection.id, error.to_string());
            }
        }
        if failure.cleanup_failures.is_empty() {
            Ok(())
        } else {
            Err(failure)
        }
    }

    pub fn policy(&self) -> &BackendPolicy {
        &self.policy
    }

    fn selected(
        &self,
        context: &BackendContext,
        operation: BackendOperation,
    ) -> Result<(Vec<&BackendConnection>, Option<String>), BackendError> {
        let selection = self.policy.select(context, operation)?;
        let mut result = Vec::with_capacity(selection.backend_ids.len());
        for id in &selection.backend_ids {
            let Some(connection) = self.connections.iter().find(|value| value.id == *id) else {
                return Err(BackendError::new(format!(
                    "Runtime Policy selected unknown Backend {id}"
                )));
            };
            result.push(connection);
        }
        Ok((result, selection.output_backend))
    }

    fn selected_existing(
        &self,
        context: &BackendContext,
        operation: BackendOperation,
    ) -> Result<(Vec<&BackendConnection>, Option<String>), BackendError> {
        let (selected, output_backend) = self.selected(context, operation)?;
        let participants = self
            .participants_for(&context.environment_id)
            .ok_or_else(|| {
                BackendError::new(format!(
                    "environment {} has no recorded Backend participants",
                    context.environment_id
                ))
            })?;
        let selected_ids: BTreeSet<_> = selected
            .iter()
            .map(|connection| connection.id.as_str())
            .collect();
        let participant_ids: BTreeSet<_> = participants.iter().map(String::as_str).collect();
        if !selected_ids.is_subset(&participant_ids) {
            return Err(BackendError::new(format!(
                "Runtime Policy selection does not match participants for environment {}",
                context.environment_id
            )));
        }
        Ok((selected, output_backend))
    }

    fn ensure_available(&self, context: &BackendContext) -> Result<(), BackendError> {
        if self.aggregate_state(&context.environment_id) == Some(EnvironmentState::Available) {
            Ok(())
        } else {
            Err(BackendError::new("Environment is not available"))
        }
    }

    fn execute_outputs<F>(
        &self,
        context: &BackendContext,
        operation: BackendOperation,
        cancel: &CancelHandle,
        mut call: F,
    ) -> Result<BackendExecution, BackendError>
    where
        F: FnMut(
            &BackendConnection,
            &BackendContext,
            &CancelHandle,
        ) -> Result<BackendExecution, BackendError>,
    {
        let (connections, output_backend) = self.selected_existing(context, operation)?;
        let ids: Vec<_> = connections
            .iter()
            .map(|connection| connection.id.clone())
            .collect();
        let mut outputs = Vec::new();
        let mut completed = Vec::new();
        for (index, connection) in connections.iter().enumerate() {
            if cancel.is_cancelled() {
                return Err(BackendError::interrupted()
                    .with_completed(completed)
                    .with_remaining(ids[index..].to_vec())
                    .with_outputs(outputs));
            }
            match call(connection, context, cancel) {
                Ok(execution) => {
                    outputs.extend(normalize_outputs(&connection.id, execution.outputs));
                    completed.push(connection.id.clone());
                }
                Err(mut error) => {
                    outputs.extend(normalize_outputs(
                        &connection.id,
                        error.outputs.drain(..).collect(),
                    ));
                    error.message = format!(
                        "Backend {} failed during {:?}: {}",
                        connection.id, operation, error.message
                    );
                    error.completed = completed;
                    error.remaining = ids[index..].to_vec();
                    error.outputs = outputs;
                    return Err(error);
                }
            }
        }
        Ok(BackendExecution {
            outputs,
            primary_backend: output_backend,
        })
    }

    fn execute_lifecycle_existing(
        &self,
        context: &BackendContext,
        operation: BackendOperation,
    ) -> Result<(), BackendError> {
        let (connections, _) = if operation == BackendOperation::Terminate
            && self.aggregate_state(&context.environment_id)
                == Some(EnvironmentState::PartiallyAvailable)
        {
            let participants = self
                .participants_for(&context.environment_id)
                .ok_or_else(|| BackendError::new("unknown Environment"))?;
            let (mut ordered, _) = self.selected(context, operation)?;
            ordered.retain(|connection| participants.contains(&connection.id));
            for id in participants.iter().rev() {
                if !ordered.iter().any(|connection| &connection.id == id) {
                    ordered.extend(
                        self.connections
                            .iter()
                            .find(|connection| &connection.id == id),
                    );
                }
            }
            (ordered, None)
        } else {
            self.selected_existing(context, operation)?
        };
        let mut completed = Vec::new();
        lifecycle(&connections, context, operation, &mut completed).map_err(|mut error| {
            error.remaining = connections
                .iter()
                .filter(|c| !completed.contains(&c.id))
                .map(|c| c.id.clone())
                .collect();
            error.completed = completed;
            error
        })
    }

    fn create_environment(
        &self,
        context: &BackendContext,
        operation: BackendOperation,
        mut call: impl FnMut(&dyn Backend) -> Result<(), BackendError>,
    ) -> Result<(), BackendError> {
        let (connections, _) = self.selected(context, operation)?;
        if let Some(parent) = &context.parent_environment_id {
            let participants = self
                .participants_for(parent)
                .ok_or_else(|| BackendError::new("unknown parent Environment"))?;
            if connections.iter().any(|c| !participants.contains(&c.id)) {
                return Err(BackendError::new(
                    "child route selects a backend absent from the parent",
                ));
            }
        }
        let ids: Vec<_> = connections.iter().map(|c| c.id.clone()).collect();
        for (index, connection) in connections.iter().enumerate() {
            if let Err(mut error) = call(connection.backend.as_ref()) {
                let mut retained = Vec::new();
                for attempted in connections[..=index].iter().rev() {
                    let state = attempted.backend.state(&context.environment_id);
                    if attempted.id == connection.id && state.is_none() {
                        continue;
                    }
                    if matches!(
                        state,
                        Some(
                            EnvironmentState::Available
                                | EnvironmentState::Suspended
                                | EnvironmentState::PartiallyAvailable
                        )
                    ) && let Err(cleanup) = attempted.backend.terminate_with_context(context)
                    {
                        error.add_cleanup_failure(&attempted.id, format!("terminate: {cleanup}"));
                    }
                    if let Err(cleanup) = attempted.backend.cleanup_with_context(context) {
                        error.add_cleanup_failure(&attempted.id, format!("cleanup: {cleanup}"));
                        retained.push(attempted.id.clone());
                    } else if attempted.backend.state(&context.environment_id).is_some() {
                        retained.push(attempted.id.clone());
                    }
                }
                retained.reverse();
                if !retained.is_empty() {
                    self.record_participants(&context.environment_id, &retained)?;
                    error.created_environment_id = Some(context.environment_id.clone());
                } else {
                    error.created_environment_id = None;
                }
                error.message = format!(
                    "Backend {} failed during {operation:?}: {}",
                    connection.id, error.message
                );
                error.completed = ids[..index].to_vec();
                error.remaining = ids[index..].to_vec();
                return Err(error);
            }
        }
        self.record_participants(&context.environment_id, &ids)
    }

    fn cleanup_all(&self, context: &BackendContext) -> Result<(), BackendError> {
        let participants = self
            .participants_for(&context.environment_id)
            .ok_or_else(|| {
                BackendError::new(format!(
                    "cannot clean environment {} without recorded Backend participants",
                    context.environment_id
                ))
            })?;
        let connections: Vec<_> = participants
            .iter()
            .filter_map(|id| self.connections.iter().find(|c| &c.id == id))
            .collect();
        let mut completed = Vec::new();
        let mut failed = Vec::new();
        let mut messages = Vec::new();
        // 清理必须反向执行，以先释放依赖于其他 Backend 的外层资源。
        for connection in connections.iter().rev() {
            match connection.backend.cleanup_with_context(context) {
                Ok(()) => completed.push(connection.id.clone()),
                Err(error) => {
                    failed.push(connection.id.clone());
                    messages.push(format!("{}: {}", connection.id, error.message));
                }
            }
        }
        let mut participants = self
            .participants
            .lock()
            .map_err(|_| BackendError::new("Runtime participant lock poisoned"))?;
        if let Some(ids) = participants.get_mut(&context.environment_id) {
            ids.retain(|id| {
                !completed.contains(id)
                    || connections
                        .iter()
                        .any(|c| &c.id == id && c.backend.state(&context.environment_id).is_some())
            });
            if ids.is_empty() {
                participants.remove(&context.environment_id);
            }
        }
        if failed.is_empty() {
            Ok(())
        } else {
            Err(BackendError {
                created_environment_id: None,
                message: format!("Runtime cleanup partially failed: {}", messages.join("; ")),
                cancelled: false,
                error_category: "backend_error".into(),
                completed,
                remaining: Vec::new(),
                cleanup_failures: messages,
                outputs: Vec::new(),
            })
        }
    }

    fn record_participants(
        &self,
        environment_id: &str,
        backend_ids: &[String],
    ) -> Result<(), BackendError> {
        let mut participants = self
            .participants
            .lock()
            .map_err(|_| BackendError::new("Runtime participant lock poisoned"))?;
        participants.insert(environment_id.to_string(), backend_ids.to_vec());
        Ok(())
    }

    fn participants_for(&self, environment_id: &str) -> Option<Vec<String>> {
        self.participants
            .lock()
            .ok()?
            .get(environment_id)
            .filter(|participants| !participants.is_empty())
            .cloned()
    }

    fn aggregate_state(&self, environment_id: &str) -> Option<EnvironmentState> {
        // 参与者缺状态也算 PartiallyAvailable，不能将其从汇总中略去。
        if let Some(participants) = self.participants_for(environment_id) {
            let mut states = Vec::with_capacity(participants.len());
            for id in &participants {
                let Some(state) = self
                    .connections
                    .iter()
                    .find(|connection| &connection.id == id)
                    .and_then(|connection| connection.backend.state(environment_id))
                else {
                    return Some(EnvironmentState::PartiallyAvailable);
                };
                states.push(state);
            }
            if states.iter().all(|state| {
                matches!(
                    state,
                    EnvironmentState::Terminated | EnvironmentState::Cleaned
                )
            }) && states.contains(&EnvironmentState::Terminated)
            {
                return Some(EnvironmentState::Terminated);
            }
            let first = states[0];
            return states
                .iter()
                .all(|state| *state == first)
                .then_some(first)
                .or(Some(EnvironmentState::PartiallyAvailable));
        }
        None
    }

    fn merged_capabilities(&self) -> RuntimeCapabilities {
        super::capabilities::merged_capabilities(&self.connections)
    }

    fn materialize_selected(
        &self,
        context: &BackendContext,
    ) -> Result<FilesystemResult, BackendError> {
        fn publish(
            connections: &[&BackendConnection],
            context: &BackendContext,
        ) -> Result<FilesystemResult, BackendError> {
            let Some((current, previous)) = connections.split_last() else {
                return Ok(FilesystemResult::default());
            };
            current
                .backend
                .materialize_after(context, &mut || publish(previous, context))
        }
        let (connections, _) = self.selected_existing(context, BackendOperation::Materialize)?;
        publish(&connections, context)
    }
}

struct RoutedRecovery<'a>(&'a BaseRuntime);

fn normalize_outputs(
    backend_id: &str,
    outputs: Vec<crate::protocol::contract::BackendOutput>,
) -> Vec<crate::protocol::contract::BackendOutput> {
    outputs
        .into_iter()
        .map(|mut output| {
            if output.backend_id == "backend" {
                output.backend_id = backend_id.to_string();
            } else if output.backend_id != backend_id
                && !output.backend_id.starts_with(&format!("{backend_id}/"))
            {
                output.backend_id = format!("{backend_id}/{}", output.backend_id);
            }
            output
        })
        .collect()
}

impl super::recovery::Component for RoutedRecovery<'_> {
    fn save_after(
        &self,
        path: &std::path::Path,
        previous: &mut dyn FnMut() -> Result<(), BackendError>,
    ) -> Result<(), BackendError> {
        previous()?;
        let runtime = self.0;
        let backends: Vec<_> = runtime
            .connections
            .iter()
            .map(|c| c.backend.as_ref())
            .collect();
        let backends: Vec<_> = runtime
            .connections
            .iter()
            .zip(&backends)
            .map(|(connection, backend)| {
                (
                    connection.id.as_str(),
                    backend as &dyn super::recovery::Component,
                )
            })
            .collect();
        super::recovery::save(
            path,
            &backends,
            serde_json::json!({"participants":*runtime.participants.lock().map_err(|_|BackendError::new("Runtime participants poisoned"))?}),
        )
    }

    fn load(&self, path: &std::path::Path) -> Result<(), BackendError> {
        let runtime = self.0;
        let backends: Vec<_> = runtime
            .connections
            .iter()
            .map(|c| c.backend.as_ref())
            .collect();
        let backends: Vec<_> = runtime
            .connections
            .iter()
            .zip(&backends)
            .map(|(connection, backend)| {
                (
                    connection.id.as_str(),
                    backend as &dyn super::recovery::Component,
                )
            })
            .collect();
        let metadata = super::recovery::load(path, &backends)?;
        let validation = (|| {
            let participants: HashMap<String, Vec<String>> =
                if metadata.is_null() && runtime.connections.len() == 1 {
                    let connection = &runtime.connections[0];
                    connection
                        .backend
                        .environments()
                        .into_iter()
                        .map(|environment| (environment, vec![connection.id.clone()]))
                        .collect()
                } else if let Some(saved) = metadata.get("previous_participants") {
                    let mut participants: HashMap<String, Vec<String>> =
                        serde_json::from_value(saved.clone())
                            .map_err(|error| BackendError::new(error.to_string()))?;
                    let outer = runtime.connections.last().unwrap();
                    for environment in outer.backend.environments() {
                        participants
                            .entry(environment)
                            .or_default()
                            .insert(0, outer.id.clone());
                    }
                    participants
                } else {
                    serde_json::from_value(metadata["participants"].clone())
                        .map_err(|error| BackendError::new(error.to_string()))?
                };
            for (environment, ids) in &participants {
                if ids.is_empty()
                    || ids.iter().collect::<BTreeSet<_>>().len() != ids.len()
                    || ids
                        .iter()
                        .any(|id| !runtime.connections.iter().any(|c| &c.id == id))
                {
                    return Err(BackendError::new("Invalid saved Runtime participants"));
                }
                for connection in &runtime.connections {
                    let present = connection
                        .backend
                        .environments()
                        .iter()
                        .any(|id| id == environment);
                    if present != ids.contains(&connection.id) {
                        return Err(BackendError::new(format!(
                            "Runtime participant environment mismatch: {environment}/{}",
                            connection.id
                        )));
                    }
                }
            }
            for connection in &runtime.connections {
                for environment in connection.backend.environments() {
                    if !participants
                        .get(&environment)
                        .is_some_and(|ids| ids.contains(&connection.id))
                    {
                        return Err(BackendError::new(format!(
                            "Untracked runtime participant: {environment}/{}",
                            connection.id
                        )));
                    }
                }
            }
            Ok(participants)
        })();
        let participants = validation.map_err(|mut error| {
            if let Err(cleanup) = runtime.shutdown_connections() {
                error.add_cleanup_failure(&runtime.id, cleanup.to_string());
            }
            error
        })?;
        *runtime
            .participants
            .lock()
            .map_err(|_| BackendError::new("Runtime participants poisoned"))? = participants;
        Ok(())
    }
    fn shutdown(&self) -> Result<(), BackendError> {
        self.0.shutdown_connections()
    }
}

impl Backend for BaseRuntime {
    fn materialize(&self, context: &BackendContext) -> Result<FilesystemResult, BackendError> {
        self.materialize_selected(context)
    }

    fn save_runtime(&self, path: &std::path::Path) -> Result<(), BackendError> {
        let _operation = self.enter_operation().map_err(BackendError::new)?;
        self.lifecycle.preserve();
        super::recovery::Component::save_after(&RoutedRecovery(self), path, &mut || Ok(()))
    }

    fn load_runtime(&self, path: &std::path::Path) -> Result<(), BackendError> {
        let _operation = self.lifecycle.enter(true).map_err(BackendError::new)?;
        self.lifecycle.preserve();
        super::recovery::Component::load(&RoutedRecovery(self), path)
    }

    fn backend_id(&self) -> &str {
        &self.id
    }

    fn backend_ids(&self) -> Vec<String> {
        self.connections.iter().map(|c| c.id.clone()).collect()
    }

    fn capabilities(&self) -> RuntimeCapabilities {
        self.merged_capabilities()
    }

    fn launch_fragment(
        &self,
        context: &BackendContext,
        launch: &AgentLaunch,
    ) -> Result<LaunchFragment, BackendError> {
        let (connections, _) = self.selected_existing(context, BackendOperation::Launch)?;
        let mut fragment = LaunchFragment::default();
        for connection in connections {
            merge_launch_fragment(
                &mut fragment,
                connection.backend.launch_fragment(context, launch)?,
            )?;
        }
        Ok(fragment)
    }

    fn stop_environment_execution(&self) -> Result<(), BackendError> {
        let mut failure = BackendError::new("failed to stop environment execution");
        for connection in &self.connections {
            if let Err(error) = connection.backend.stop_environment_execution() {
                failure.add_cleanup_failure(&connection.id, error.to_string());
            }
        }
        if failure.cleanup_failures.is_empty() {
            Ok(())
        } else {
            Err(failure)
        }
    }

    fn start_environment_execution(
        &self,
        context: &BackendContext,
        launch: &AgentLaunch,
    ) -> Result<(), BackendError> {
        let (connections, _) = self.selected_existing(context, BackendOperation::Launch)?;
        for connection in connections {
            connection
                .backend
                .start_environment_execution(context, launch)?;
        }
        Ok(())
    }

    fn establish(&self, environment_id: &str) -> Result<(), BackendError> {
        self.establish_with_context(&BackendContext::new(environment_id))
    }

    fn establish_with_context(&self, context: &BackendContext) -> Result<(), BackendError> {
        self.create_environment(context, BackendOperation::Establish, |backend| {
            backend.establish_with_context(context)
        })
    }

    fn operate(
        &self,
        environment_id: &str,
        command: &[String],
        cancel: &CancelHandle,
    ) -> Result<CommandOutput, BackendError> {
        let execution =
            self.operate_with_context(&BackendContext::new(environment_id), command, cancel)?;
        execution.single_output().ok_or_else(|| {
            BackendError::new(
                "multiple Backend outputs require an explicit output_backend selection",
            )
            .with_outputs(execution.outputs)
        })
    }

    fn operate_with_context(
        &self,
        context: &BackendContext,
        command: &[String],
        cancel: &CancelHandle,
    ) -> Result<BackendExecution, BackendError> {
        self.execute_outputs(
            context,
            BackendOperation::Operate,
            cancel,
            |connection, context, cancel| {
                connection
                    .backend
                    .operate_with_context(context, command, cancel)
            },
        )
    }

    fn tool_context(
        &self,
        environment_id: &str,
        tool_name: &str,
        cancel: &CancelHandle,
    ) -> Result<CommandOutput, BackendError> {
        let execution = self.tool_context_with_context(
            &BackendContext::new(environment_id),
            tool_name,
            cancel,
        )?;
        execution.single_output().ok_or_else(|| {
            BackendError::new(
                "multiple Backend outputs require an explicit output_backend selection",
            )
            .with_outputs(execution.outputs)
        })
    }

    fn tool_context_with_context(
        &self,
        context: &BackendContext,
        tool_name: &str,
        cancel: &CancelHandle,
    ) -> Result<BackendExecution, BackendError> {
        self.execute_outputs(
            context,
            BackendOperation::ToolContext,
            cancel,
            |connection, context, cancel| {
                connection
                    .backend
                    .tool_context_with_context(context, tool_name, cancel)
            },
        )
    }

    fn invoke_tool(
        &self,
        environment_id: &str,
        tool_name: &str,
        arguments: &Value,
        cancel: &CancelHandle,
    ) -> Result<CommandOutput, BackendError> {
        let execution = self.invoke_tool_with_context(
            &BackendContext::new(environment_id),
            tool_name,
            arguments,
            cancel,
        )?;
        execution.single_output().ok_or_else(|| {
            BackendError::new(
                "multiple Backend outputs require an explicit output_backend selection",
            )
            .with_outputs(execution.outputs)
        })
    }

    fn cancel_tool_writers(&self, context: &BackendContext) -> Result<(), BackendError> {
        let participants = self
            .participants_for(&context.environment_id)
            .ok_or_else(|| BackendError::new("unknown Environment"))?;
        let mut failure = BackendError::new("failed to drain Environment tool writers");
        for connection in self
            .connections
            .iter()
            .rev()
            .filter(|c| participants.contains(&c.id))
        {
            if let Err(error) = connection.backend.cancel_tool_writers(context) {
                failure.add_cleanup_failure(&connection.id, error.to_string());
            }
        }
        if failure.cleanup_failures.is_empty() {
            Ok(())
        } else {
            Err(failure)
        }
    }

    fn environment_operation(
        &self,
        context: &BackendContext,
        operation: &str,
        arguments: &super::execution_wire::Message,
        cancel: &CancelHandle,
    ) -> Result<super::execution_wire::Message, BackendError> {
        self.ensure_available(context)?;
        let kind = BackendOperation::EnvironmentOperation;
        let (connections, preferred) = self.selected_existing(context, kind)?;
        let single = connections.len() == 1;
        let mut result = None;
        let mut completed = Vec::new();
        for (index, connection) in connections.iter().enumerate() {
            let execution = if cancel.is_cancelled() {
                Err(BackendError::interrupted())
            } else {
                connection
                    .backend
                    .environment_operation(context, operation, arguments, cancel)
            };
            match execution {
                Ok(value) => {
                    completed.push(connection.id.clone());
                    if preferred
                        .as_deref()
                        .map_or(single, |id| id == connection.id)
                    {
                        result = Some(value);
                    }
                }
                Err(mut error) => {
                    error.message = format!(
                        "Backend {} failed during {kind:?}: {}",
                        connection.id, error.message
                    );
                    error.completed = completed;
                    error.remaining = connections[index..]
                        .iter()
                        .map(|backend| backend.id.clone())
                        .collect();
                    return Err(error);
                }
            }
        }
        result.ok_or_else(|| BackendError::new("ambiguous backend output"))
    }

    fn invoke_tool_with_context(
        &self,
        context: &BackendContext,
        tool_name: &str,
        arguments: &Value,
        cancel: &CancelHandle,
    ) -> Result<BackendExecution, BackendError> {
        self.ensure_available(context)?;
        self.execute_outputs(
            context,
            BackendOperation::ExecuteTool,
            cancel,
            |connection, context, cancel| {
                connection
                    .backend
                    .invoke_tool_with_context(context, tool_name, arguments, cancel)
            },
        )
    }

    fn filesystem_operation(
        &self,
        context: &BackendContext,
        operation: &Operation,
        cancel: &CancelHandle,
    ) -> Result<FilesystemResult, BackendError> {
        let backend_operation = BackendOperation::for_operation(operation);
        if !backend_operation.is_filesystem() {
            return Err(BackendError::new(
                "filesystem dispatch requires a filesystem operation",
            ));
        }
        if cancel.is_cancelled() {
            return Err(BackendError::interrupted());
        }
        let (connections, _) = self.selected_existing(context, backend_operation)?;
        if connections.len() != 1 {
            return Err(BackendError::new(
                "filesystem operation requires exactly one selected Backend",
            ));
        }
        connections[0]
            .backend
            .filesystem_operation(context, operation, cancel)
    }

    fn branch(&self, parent: &str, child: &str) -> Result<(), BackendError> {
        let context = BackendContext {
            environment_id: child.to_string(),
            parent_environment_id: Some(parent.to_string()),
            ..BackendContext::default()
        };
        self.branch_with_context(&context, parent, child)
    }

    fn branch_with_context(
        &self,
        context: &BackendContext,
        parent: &str,
        child: &str,
    ) -> Result<(), BackendError> {
        let child_context = BackendContext {
            environment_id: child.to_owned(),
            parent_environment_id: Some(parent.to_owned()),
            ..context.clone()
        };
        self.create_environment(&child_context, BackendOperation::Branch, |backend| {
            backend.branch_with_context(&child_context, parent, child)
        })
    }

    fn promote(&self, environment_id: &str) -> Result<(), BackendError> {
        self.promote_with_context(&BackendContext::new(environment_id))
    }

    fn promote_with_context(&self, context: &BackendContext) -> Result<(), BackendError> {
        self.execute_lifecycle_existing(context, BackendOperation::Promote)
    }

    fn suspend(&self, environment_id: &str) -> Result<(), BackendError> {
        self.suspend_with_context(&BackendContext::new(environment_id))
    }

    fn suspend_with_context(&self, context: &BackendContext) -> Result<(), BackendError> {
        self.execute_lifecycle_existing(context, BackendOperation::Suspend)
    }

    fn resume(&self, environment_id: &str) -> Result<(), BackendError> {
        self.resume_with_context(&BackendContext::new(environment_id))
    }

    fn resume_with_context(&self, context: &BackendContext) -> Result<(), BackendError> {
        self.execute_lifecycle_existing(context, BackendOperation::Resume)
    }

    fn terminate(&self, environment_id: &str) -> Result<(), BackendError> {
        self.terminate_with_context(&BackendContext::new(environment_id))
    }

    fn terminate_with_context(&self, context: &BackendContext) -> Result<(), BackendError> {
        self.execute_lifecycle_existing(context, BackendOperation::Terminate)
    }

    fn cleanup(&self, environment_id: &str) -> Result<(), BackendError> {
        self.cleanup_with_context(&BackendContext::new(environment_id))
    }

    fn cleanup_with_context(&self, context: &BackendContext) -> Result<(), BackendError> {
        self.cleanup_all(context)
    }

    fn state(&self, environment_id: &str) -> Option<EnvironmentState> {
        self.aggregate_state(environment_id)
    }

    fn backend_states(&self, environment_id: &str) -> Vec<BackendState> {
        let Some(participants) = self.participants_for(environment_id) else {
            return Vec::new();
        };
        self.connections
            .iter()
            .filter(|c| participants.contains(&c.id))
            .map(|c| BackendState {
                backend_id: c.id.clone(),
                state: c.backend.state(environment_id),
            })
            .collect()
    }

    fn shutdown(&self) -> Result<(), BackendError> {
        if !self.lifecycle.close() {
            return Ok(());
        }
        let host = self.agent_supervisor.stop();
        let environments = if self.lifecycle.drain() {
            self.stop_environment_execution()
        } else {
            self.shutdown_connections()
        };
        match (host, environments) {
            (Ok(()), result) | (result, Ok(())) => result,
            (Err(host), Err(mut environments)) => {
                environments.add_cleanup_failure("Agent host", host.to_string());
                Err(environments)
            }
        }
    }

    fn environments(&self) -> Vec<String> {
        let mut environments = BTreeSet::new();
        for connection in &self.connections {
            environments.extend(connection.backend.environments());
        }
        environments.into_iter().collect()
    }
}

fn lifecycle(
    connections: &[&BackendConnection],
    context: &BackendContext,
    operation: BackendOperation,
    completed: &mut Vec<String>,
) -> Result<(), BackendError> {
    let Some((current, preceding)) = connections.split_last() else {
        return Ok(());
    };
    let count = completed.len();
    let mut called = false;
    let mut previous = || {
        called = true;
        lifecycle(preceding, context, operation, completed)
    };
    let result = match operation {
        BackendOperation::Suspend => current.backend.suspend_after(context, &mut previous),
        BackendOperation::Resume => current.backend.resume_after(context, &mut previous),
        BackendOperation::Terminate => match current.backend.state(&context.environment_id) {
            Some(EnvironmentState::Cleaned | EnvironmentState::Terminated) => previous(),
            _ => current.backend.terminate_after(context, &mut previous),
        },
        BackendOperation::Promote => {
            previous().and_then(|()| current.backend.promote_with_context(context))
        }
        _ => unreachable!("not a lifecycle operation"),
    };
    if result.is_ok() {
        completed.push(current.id.clone());
    }
    result.map_err(|mut error| {
        if !called || completed.len() - count == preceding.len() {
            error.message = format!(
                "Backend {} failed during {operation:?}: {}",
                current.id, error.message
            );
        }
        error
    })
}
