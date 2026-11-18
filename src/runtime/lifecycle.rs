//! BaseRuntime operation admission, shutdown and recovery state.

use std::sync::{Arc, Condvar, Mutex};

#[derive(Default)]
pub(super) struct Lifecycle {
    state: Mutex<State>,
    drained: Condvar,
}

#[derive(Default)]
struct State {
    closed: bool,
    loading: bool,
    preserving: bool,
    active: usize,
}

impl Lifecycle {
    pub(super) fn enter(self: &Arc<Self>, loading: bool) -> Result<OperationGuard, &'static str> {
        let mut state = self.state.lock().unwrap();
        if state.closed {
            return Err("Runtime is shutting down");
        }
        // Restore may replace or roll back the entire backend state.
        if state.loading || (loading && state.active != 0) {
            return Err("Runtime restore requires an idle session");
        }
        state.active += 1;
        state.loading = loading;
        Ok(OperationGuard(self.clone()))
    }

    pub(super) fn close(&self) -> bool {
        let mut state = self.state.lock().unwrap();
        !std::mem::replace(&mut state.closed, true)
    }

    pub(super) fn is_closed(&self) -> bool {
        self.state.lock().unwrap().closed
    }

    pub(super) fn preserve(&self) {
        self.state.lock().unwrap().preserving = true;
    }

    pub(super) fn drain(&self) -> bool {
        self.drained
            .wait_while(self.state.lock().unwrap(), |state| state.active != 0)
            .unwrap()
            .preserving
    }
}

pub(super) struct OperationGuard(Arc<Lifecycle>);

impl Drop for OperationGuard {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().unwrap();
        state.active -= 1;
        if state.active == 0 {
            state.loading = false;
            self.0.drained.notify_all();
        }
    }
}
