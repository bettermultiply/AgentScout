//! Task termination and authority materialization coordination.
use super::{AgentCommand, SpeculativeTree};
use crate::protocol::contract::OperationResult;
use serde_json::{Value, json};
use std::io;

#[derive(Default)]
pub struct Task {
    pub exit_requested: bool,
    pub terminating: bool,
    pub materialization_failed: bool,
    aborted: std::collections::BTreeSet<String>,
    failed: bool,
    materializing: bool,
    materialized: Option<Result<OperationResult, String>>,
}

impl Task {
    pub fn retry_materialization(&mut self) {
        self.materialization_failed = false;
    }

    pub fn progress(&mut self, tree: &mut SpeculativeTree) -> io::Result<Vec<AgentCommand>> {
        // A Promote receipt can select a new writer while cancellation is in
        // transit. Stop that authority before allowing publication.
        if self.terminating
            && !tree.recovery_pending_commands()
            && tree
                .authority_branch_id
                .as_ref()
                .is_some_and(|id| self.aborted.insert(id.clone()))
        {
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
        if self.terminating {
            self.retry_materialization();
            return Ok(Vec::new());
        }
        if !tree.recovering()
            && !tree.task_finishing()
            && !tree.authority_running()
            && !tree.pending_work()
            && tree.task_writers_drained()
        {
            return if exit {
                tree.begin_shutdown()
            } else {
                Ok(Vec::new())
            };
        }
        self.terminating = true;
        self.aborted
            .extend(tree.authority_branch_id.iter().cloned());
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

    pub fn materialized(&mut self, result: Result<OperationResult, String>) {
        self.materialized = Some(result);
    }

    pub fn start_materialization(
        &mut self,
        tree: &SpeculativeTree,
        mut record: impl FnMut(Value) -> io::Result<()>,
    ) -> io::Result<Option<String>> {
        if !tree.task_finishing()
            || tree.recovering()
            || self.materialization_failed
            || self.materializing
            || tree.authority_running()
            || !tree.task_writers_drained()
        {
            return Ok(None);
        }
        let authority = tree.authority_branch_id.as_ref();
        let environment = authority
            .and_then(|id| tree.branches.get(id))
            .and_then(|branch| branch.environment_id.clone());
        let Some(environment) = environment else {
            // Report through the normal visible, retryable materialization failure path.
            self.materializing = true;
            self.materialized = Some(Err("authority Branch or Environment missing".into()));
            return Ok(None);
        };
        record(json!({"message":"materialization_started","branch_id":authority}))?;
        self.materializing = true;
        Ok(Some(environment))
    }

    pub fn materialize(
        &mut self,
        tree: &mut SpeculativeTree,
        failed: bool,
        mut record: impl FnMut(Value) -> io::Result<()>,
    ) -> io::Result<Option<(Vec<AgentCommand>, bool, Result<(), String>)>> {
        if !self.materializing
            || self.materialized.is_none()
            || !tree.task_finishing()
            || tree.recovering()
            || self.materialization_failed
            || tree.authority_running()
            || !tree.task_writers_drained()
        {
            return Ok(None);
        }
        let authority = tree.authority_branch_id.clone();
        self.materializing = false;
        match self.materialized.take().unwrap() {
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
