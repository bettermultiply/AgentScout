
use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

use crate::protocol::contract::{
    AdapterMessage, Event, EventPayload, OperationRequest, ResultStatus,
};
use crate::runtime::backend::CancelHandle;

use super::result::result as make_result;
use super::*;

#[derive(Default)]
struct Pending {
    closed: bool,
    preempt: VecDeque<OperationRequest>,
    normal: VecDeque<OperationRequest>,
}

pub(crate) struct Queue {
    pending: Mutex<Pending>,
    signal: Condvar,
    cancel: CancelHandle,
}

impl Queue {
    pub(super) fn start(shared: Arc<Shared>, environment_id: String) -> Arc<Self> {
        let queue = Arc::new(Self {
            pending: Mutex::new(Pending::default()),
            signal: Condvar::new(),
            cancel: CancelHandle::default(),
        });
        let worker = queue.clone();
        thread::spawn(move || worker.run(shared, environment_id));
        queue
    }

    pub(super) fn enqueue(&self, request: OperationRequest, preempt: bool) {
        {
            let mut pending = self.pending.lock().unwrap();
            if pending.closed {
                return;
            }
            if preempt {
                pending.preempt.push_back(request);
                self.cancel.cancel();
            } else {
                pending.normal.push_back(request);
            }
        }
        self.signal.notify_one();
    }

    pub(super) fn close(&self) {
        let mut pending = self.pending.lock().unwrap();
        pending.closed = true;
        pending.preempt.clear();
        pending.normal.clear();
        self.cancel.cancel();
        self.signal.notify_one();
    }

    pub(super) fn run(self: Arc<Self>, shared: Arc<Shared>, environment_id: String) {
        loop {
            let request = {
                let mut pending = self.pending.lock().unwrap();
                loop {
                    if pending.closed {
                        return;
                    }
                    self.cancel.reset();
                    if let Some(request) = pending.preempt.pop_front() {
                        break request;
                    }
                    if let Some(request) = pending.normal.pop_front() {
                        break request;
                    }
                    pending = self.signal.wait(pending).unwrap();
                }
            };
            if let Err(error) = shared.out.send(AdapterMessage::Event(Event {
                environment_id: request.environment_id.clone(),
                request_id: Some(request.request_id.clone()),
                payload: EventPayload::OperationStarted {
                    operation: request.operation.clone(),
                },
            })) {
                eprintln!("adapter: failed to send operation-started event: {error}");
            }
            let result = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                shared.execute(&environment_id, &request, &self.cancel)
            })) {
                Ok(result) => result,
                Err(_) => {
                    eprintln!(
                        "adapter: operation {} panicked; the queue continues with a failed result",
                        request.request_id
                    );
                    let mut result = make_result(&request, ResultStatus::Failed, None, false);
                    result.reason = Some("runtime operation panicked".to_string());
                    result
                }
            };
            {
                let evicted = shared
                    .results
                    .lock()
                    .unwrap()
                    .insert(request.request_id.clone(), result.clone());
                // Execution identities survive eviction of the larger result payload.
                let _ = evicted;
            }
            if let Some((_, completed)) = shared.seen.lock().unwrap().get_mut(&request.request_id) {
                *completed = true;
            }
            shared.result_signal.notify_all();
            if let Some(state) = result.environment_state {
                shared.set_known(&result.environment_id, state);
            }
            if let Err(error) = shared.out.send(AdapterMessage::Event(Event {
                environment_id: result.environment_id.clone(),
                request_id: Some(request.request_id.clone()),
                payload: EventPayload::OperationFinished {
                    result: result.clone(),
                },
            })) {
                eprintln!("adapter: failed to send operation-finished event: {error}");
            }
            if let Err(error) = shared.out.send(AdapterMessage::Result(result)) {
                eprintln!("adapter: failed to send operation result: {error}");
            }
        }
    }
}
