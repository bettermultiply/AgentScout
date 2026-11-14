//! Task termination and authority materialization coordination.
use super::execution::runtime_call;
use super::{AgentCommand, Operation, Runtime, RuntimeCapabilities, SpeculativeTree};
use serde_json::{Value, json};
use std::io;

#[derive(Default)]
pub struct Task {
    pub exit_requested: bool,
    pub terminating: bool,
    pub materialization_failed: bool,
    aborted: std::collections::BTreeSet<String>,
    failed: bool,
}

impl Task {
    pub fn retry_materialization(&mut self) {
        self.materialization_failed = false;
    }

    pub fn progress(&mut self, tree: &mut SpeculativeTree) -> io::Result<Vec<AgentCommand>> {
        // A Promote receipt can select a new writer while cancellation is in
        // transit. Stop that authority before allowing publication.
        if self.terminating && tree.authority_running() && !tree.recovery_pending_commands()
            && tree.authority_branch_id.as_ref().is_some_and(|id| self.aborted.insert(id.clone())) {
            tree.interrupt()
        } else {
            Ok(Vec::new())
        }
    }
    pub fn cancel(
        &mut self,
        tree: &mut SpeculativeTree,
        exit: bool,
    ) -> io::Result<Vec<AgentCommand>> {
        self.exit_requested |= exit;
        self.terminating = true;
        self.aborted.extend(tree.authority_branch_id.iter().cloned());
        tree.cancel_task()
    }

    pub fn fail(&mut self, tree: &mut SpeculativeTree) {
        self.failed = true;
        self.terminating = true;
        tree.finish_task();
    }

    pub fn fail_and_exit(&mut self, tree: &mut SpeculativeTree) -> io::Result<Vec<AgentCommand>> {
        self.fail(tree);
        self.exit_requested = true;
        tree.interrupt()
    }

    pub fn finish_ready(&self, tree: &mut SpeculativeTree) -> io::Result<bool> {
        Ok(tree.is_stopping() && !tree.cleanup_pending()? && !tree.pending_work())
    }

    pub fn materialize(
        &mut self,
        tree: &mut SpeculativeTree,
        runtime: &dyn Runtime,
        capabilities: &RuntimeCapabilities,
        failed: bool,
        mut record: impl FnMut(Value) -> io::Result<()>,
    ) -> io::Result<Option<(Vec<AgentCommand>, bool, Result<(), String>)>> {
        if !tree.task_finishing()
            || tree.recovering()
            || self.materialization_failed
            || tree.authority_running()
            || !tree.task_writers_drained()
        {
            return Ok(None);
        }
        let authority = tree.authority_branch_id.clone().unwrap();
        let environment = if capabilities.shared_environment {
            "authority"
        } else {
            &authority
        };
        record(json!({"message":"materialization_started","branch_id":authority}))?;
        match runtime_call(runtime, environment, Operation::MaterializeAuthority) {
            Ok(result) => {
                record(
                    json!({"message":"materialization_completed","branch_id":authority,"filesystem":result.filesystem}),
                )?;
                record(
                    json!({"message":"task_completed","branch_id":authority,"status":if failed || self.failed {"failed"} else if self.terminating {"cancelled"} else {"succeeded"}}),
                )?;
                tree.complete_task();
                self.terminating = false;
                self.aborted.clear();
                self.failed = false;
                let commands = if self.exit_requested {
                    tree.begin_shutdown()?
                } else {
                    Vec::new()
                };
                Ok(Some((commands, !self.exit_requested, Ok(()))))
            }
            Err(error) => {
                record(
                    json!({"message":"materialization_failed","branch_id":authority,"reason":error.to_string()}),
                )?;
                self.materialization_failed = true;
                Ok(Some((Vec::new(), false, Err(error.to_string()))))
            }
        }
    }
}
