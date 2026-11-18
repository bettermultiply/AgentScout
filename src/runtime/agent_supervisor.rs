//! The Agent and Instrumentation share one host process for the entire run.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::path::PathBuf;
use std::process::Child;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use super::backend::BackendError;
use crate::protocol::contract::AgentLaunch;

#[derive(Default)]
pub(crate) struct AgentSupervisor {
    host_agent: Mutex<Option<Child>>,
    stopped: AtomicBool,
}

impl AgentSupervisor {
    pub(crate) fn launch(
        &self,
        launch: &AgentLaunch,
        env: &BTreeMap<String, String>,
    ) -> Result<(), BackendError> {
        let mut host = self
            .host_agent
            .lock()
            .map_err(|_| BackendError::new("agent supervisor lock poisoned"))?;
        if self.stopped.load(Ordering::Acquire) {
            return Err(BackendError::new("Agent host is shutting down"));
        }
        if let Some(existing) = host.as_mut() {
            match existing.try_wait().map_err(error)? {
                None => return Ok(()),
                Some(status) => {
                    return Err(BackendError::new(format!(
                        "Agent host process exited before the run completed: {status}"
                    )));
                }
            }
        }
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&launch.log)
            .map_err(error)?;
        // Pi runs on the host, so its own temporary files must stay on the
        // host. Tool commands receive their own namespace through the holder
        // and are not given this Pi-owned temp directory.
        let temporary = launch
            .log
            .parent()
            .map(|parent| parent.join("agent-tmp"))
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        fs::create_dir_all(&temporary).map_err(error)?;
        let mut environment = launch.env.clone();
        environment.extend(env.clone());
        environment
            .entry("TMPDIR".into())
            .or_insert_with(|| temporary.display().to_string());
        let child = std::process::Command::new(&launch.command)
            .args(&launch.args)
            .current_dir(&launch.cwd)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::from(log.try_clone().map_err(error)?))
            .stderr(std::process::Stdio::from(log))
            .envs(&environment)
            .spawn()
            .map_err(error)?;
        *host = Some(child);
        Ok(())
    }

    pub(crate) fn stop(&self) -> Result<(), BackendError> {
        self.stopped.store(true, Ordering::Release);
        let mut host = self
            .host_agent
            .lock()
            .map_err(|_| BackendError::new("agent supervisor lock poisoned"))?;
        if let Some(child) = host.as_mut() {
            if child.try_wait().map_err(error)?.is_none() {
                if let Err(failure) = child.kill()
                    && child.try_wait().map_err(error)?.is_none()
                {
                    return Err(error(failure));
                }
                child.wait().map_err(error)?;
            }
            // Keep the reaped child in the slot. A stopped run cannot relaunch it.
        }
        Ok(())
    }
}

impl Drop for AgentSupervisor {
    fn drop(&mut self) {
        if let Err(failure) = self.stop() {
            eprintln!("agent supervisor: failed to stop Agent host: {failure}");
        }
    }
}

fn error(value: impl std::fmt::Display) -> BackendError {
    BackendError::new(value.to_string())
}
