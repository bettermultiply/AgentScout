//! discard 数据面操作。

use super::*;

impl FilesystemBackend {
    /// Release a branch or mark and schedule its data for removal.
    pub fn discard(
        &self,
        snapshot: &str,
        _scope: &[PathScope],
    ) -> Result<FilesystemResult, BackendError> {
        let (environment, mark, garbage) = {
            let mut state = self.lock()?;
            if let Some(environment) = state.environments.get(snapshot).cloned() {
                if environment.state != EnvironmentState::Terminated {
                    return Err(BackendError::new(
                        "filesystem environment must be terminated before discard",
                    ));
                }
                self.retain_retired_root(&mut state, &environment.root)?;
                state.environments.remove(&environment.id);
                let garbage = self.refresh_references(&mut state)?;
                (Some(environment), None, garbage)
            } else if state.snapshots.contains_key(snapshot) {
                for environment in state.environments.values_mut() {
                    environment.marks.retain(|_, value| value != snapshot);
                }
                let garbage = self.refresh_references(&mut state)?;
                (None, Some(snapshot.to_string()), garbage)
            } else {
                let environment = Self::resolve_environment(&state, snapshot)?;
                if environment.state != EnvironmentState::Terminated {
                    return Err(BackendError::new(
                        "filesystem environment must be terminated before discard",
                    ));
                }
                self.retain_retired_root(&mut state, &environment.root)?;
                state.environments.remove(&environment.id);
                let garbage = self.refresh_references(&mut state)?;
                (Some(environment), None, garbage)
            }
        };
        self.persist_all_snapshots()?;
        if let Some(environment) = &environment {
            self.remove_environment_metadata(&environment.id)?;
        }
        self.schedule_gc(garbage)?;
        let mut result = FilesystemResult::default();
        result.snapshot_id = Some(mark.unwrap_or_else(|| snapshot.to_string()));
        result.confirmation.refs_released = true;
        // True only after every path was moved out of the reachable
        // namespace; physical reclamation continues in the background.
        result.confirmation.gc_eligible = true;
        Ok(result)
    }
}
