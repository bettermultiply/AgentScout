use super::execution::runtime_call;
use super::{AgentCommand, AgentMessage, Operation, Runtime, SpeculativeTree};
use crate::control_plane::recovery;
use std::{
    io,
    path::{Path, PathBuf},
};

pub struct Recovery {
    run_dir: PathBuf,
    path: PathBuf,
    pub saved: bool,
    suspended: bool,
    save_requested: bool,
    pub loaded: bool,
}

impl Recovery {
    pub fn new(run_dir: &Path) -> Self {
        Self {
            run_dir: run_dir.to_path_buf(),
            path: run_dir.join("agent-recovery.json"),
            saved: false,
            suspended: false,
            save_requested: false,
            loaded: false,
        }
    }

    pub fn pause(&mut self, tree: &mut SpeculativeTree, at: u64) -> io::Result<Vec<AgentCommand>> {
        if tree.recovering() && !self.loaded {
            return Ok(Vec::new());
        }
        self.loaded = false;
        self.saved = false;
        let directory = self.run_dir.join(format!(
            "recovery-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory)?;
        self.path = directory.join("agent.json");
        self.suspended = false;
        self.save_requested = false;
        Ok(vec![tree.pause_recovery(at)])
    }

    pub fn continue_execution(&mut self, tree: &mut SpeculativeTree, at: u64) -> bool {
        if tree.recovering() {
            if !self.loaded {
                return false;
            }
            tree.continue_recovery(at);
        }
        true
    }

    pub fn handle(
        &mut self,
        tree: &mut SpeculativeTree,
        message: &AgentMessage,
    ) -> io::Result<Option<String>> {
        if !tree.recovering()
            && matches!(
                message,
                AgentMessage::RecoverySaved { .. } | AgentMessage::RecoverySuspended
            )
        {
            self.saved = false;
            return Ok(None);
        }
        match message {
            AgentMessage::RecoverySaved { path, error } => {
                if path != &self.path {
                    return Err(io::Error::other("Recovery path mismatch"));
                }
                if let Some(error) = error {
                    tree.load_recovery();
                    self.loaded = true;
                    self.saved = false;
                    self.suspended = false;
                    self.save_requested = false;
                    return Ok(Some(format!("Save failed; still running: {error}")));
                }
                self.saved = true;
            }
            AgentMessage::RecoverySuspended => self.suspended = true,
            AgentMessage::RecoveryLoaded { error: Some(error) } => {
                return Err(io::Error::other(format!("Recovery load failed: {error}")));
            }
            AgentMessage::RecoveryLoaded { error: None } => self.loaded = true,
            _ => {}
        }
        Ok(None)
    }

    pub fn save_command(&mut self, tree: &SpeculativeTree) -> Option<AgentCommand> {
        if tree.recovering()
            && self.suspended
            && !self.save_requested
            && !tree.pending_work()
            && !tree.recovery_pending_commands()
        {
            self.save_requested = true;
            Some(AgentCommand::Save {
                branch_id: tree.authority_branch_id.clone().unwrap_or_default(),
                path: self.path.clone(),
            })
        } else {
            None
        }
    }

    pub fn publish(
        &mut self,
        tree: &mut SpeculativeTree,
        runtime: &dyn Runtime,
        run_dir: &Path,
        draining: bool,
    ) -> Option<(AgentCommand, String)> {
        if !tree.recovering() || !self.saved || draining || tree.pending_work() {
            return None;
        }
        let result = (|| -> io::Result<()> {
            let generation = self.path.parent().unwrap();
            runtime_call(
                runtime,
                tree.branches[tree
                    .authority_branch_id
                    .as_ref()
                    .ok_or_else(|| io::Error::other("Recovery authority missing"))?]
                .environment_id
                .as_deref()
                .ok_or_else(|| io::Error::other("Recovery authority Environment missing"))?,
                Operation::SaveRuntime {
                    path: generation.join("runtime.json"),
                },
            )
            .map_err(|error| io::Error::other(error.to_string()))?;
            recovery::publish(run_dir, generation, tree)?;
            if let Some(carrier) = crate::telemetry::Carrier::current() {
                // Optional correlation only. Recovery remains usable if this write fails.
                if let Ok(bytes) = serde_json::to_vec(&carrier) {
                    let _ = std::fs::write(generation.join("trace-context.json"), bytes);
                }
            }
            Ok(())
        })();
        self.saved = false;
        self.loaded = true;
        let notice = match result {
            Err(error) => {
                tree.load_recovery();
                format!("Save failed; execution retained: {error}")
            }
            Ok(()) => {
                self.suspended = false;
                self.save_requested = false;
                "Paused; /continue resumes the task".into()
            }
        };
        Some((
            AgentCommand::CancelSave {
                branch_id: tree.authority_branch_id.clone().unwrap_or_default(),
            },
            notice,
        ))
    }
}
