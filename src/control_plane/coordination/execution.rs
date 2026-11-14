//! Runtime result interpretation and final environment cleanup.
use super::{Operation, Runtime, RuntimeCapabilities, RuntimeExecutionState, SpeculativeTree};

pub fn runtime_call(
    runtime: &dyn Runtime,
    environment: &str,
    operation: Operation,
) -> Result<crate::protocol::contract::OperationResult, Box<dyn std::error::Error>> {
    let result = runtime.call(environment, operation)?;
    if result.status != crate::protocol::contract::ResultStatus::Completed {
        return Err(format!("Runtime request failed: {:?}", result.reason).into());
    }
    Ok(result)
}

fn runtime_cleanup(
    runtime: &dyn Runtime,
    branch: &str,
    tree: &mut SpeculativeTree,
) -> Result<(), Box<dyn std::error::Error>> {
    tree.record_runtime_state(branch, RuntimeExecutionState::TerminationRequested)?;
    if let Err(error) = runtime_call(runtime, branch, Operation::TerminateEnvironment) {
        tree.record_runtime_state(branch, RuntimeExecutionState::Failed)?;
        return Err(error);
    }
    tree.record_runtime_state(branch, RuntimeExecutionState::Terminated)?;
    if let Err(error) = runtime_call(runtime, branch, Operation::CleanupEnvironment) {
        tree.record_runtime_state(branch, RuntimeExecutionState::Failed)?;
        return Err(error);
    }
    tree.record_runtime_state(branch, RuntimeExecutionState::Cleaned)?;
    Ok(())
}

pub fn cleanup(
    runtime: &dyn Runtime,
    capabilities: &RuntimeCapabilities,
    tree: &mut SpeculativeTree,
) -> Result<(), Box<dyn std::error::Error>> {
    if tree.recovering() || tree.task_finishing() || tree.authority_branch_id.is_none() {
        return Ok(());
    }
    if capabilities.shared_environment {
        let record_shared = |tree: &mut SpeculativeTree, state| -> std::io::Result<()> {
            let ids: Vec<_> = tree.branches.keys().cloned().collect();
            for id in ids {
                tree.record_runtime_state(&id, state)?;
            }
            Ok(())
        };
        record_shared(tree, RuntimeExecutionState::TerminationRequested)?;
        if let Err(error) = runtime_call(runtime, "authority", Operation::TerminateEnvironment) {
            record_shared(tree, RuntimeExecutionState::Failed)?;
            return Err(error);
        }
        record_shared(tree, RuntimeExecutionState::Terminated)?;
        if let Err(error) = runtime_call(runtime, "authority", Operation::CleanupEnvironment) {
            record_shared(tree, RuntimeExecutionState::Failed)?;
            return Err(error);
        }
        record_shared(tree, RuntimeExecutionState::Cleaned)?;
    } else {
        let ids: Vec<_> = tree
            .branches
            .values()
            .filter(|branch| branch.execution_state.runtime != RuntimeExecutionState::Cleaned)
            .map(|branch| branch.branch_id.clone())
            .collect();
        let mut failures = Vec::new();
        for id in ids {
            if let Err(error) = runtime_cleanup(runtime, &id, tree) {
                failures.push(format!("{id}: {error}"));
            }
        }
        if !failures.is_empty() {
            return Err(failures.join("; ").into());
        }
    }
    Ok(())
}
