//! 按环境串行执行的请求队列。

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

use crate::protocol::contract::{AdapterMessage, EnvironmentState, Event, EventPayload, Operation, OperationRequest, ResultStatus};
use crate::runtime::backend::CancelHandle;

use super::*;


#[derive(Default)]
struct Pending {
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

    /// suspend/terminate 插到队首并打断在飞操作；其他操作按到达顺序排队。
    pub(super) fn enqueue(&self, request: OperationRequest, preempt: bool) {
        {
            let mut pending = self.pending.lock().unwrap();
            if preempt {
                pending.preempt.push_back(request);
            } else {
                pending.normal.push_back(request);
            }
        }
        if preempt {
            self.cancel.cancel();
        }
        self.signal.notify_one();
    }

    pub(super) fn run(self: Arc<Self>, shared: Arc<Shared>, environment_id: String) {
        loop {
            let request = {
                let mut pending = self.pending.lock().unwrap();
                loop {
                    if let Some(request) = pending.preempt.pop_front() {
                        break request;
                    }
                    if let Some(request) = pending.normal.pop_front() {
                        break request;
                    }
                    pending = self.signal.wait(pending).unwrap();
                }
            };
            self.cancel.reset();
            let _ = shared.out.send(AdapterMessage::Event(Event {
                environment_id: request.environment_id.clone(),
                request_id: Some(request.request_id.clone()),
                payload: EventPayload::OperationStarted {
                    operation: request.operation.clone(),
                },
            }));
            let result = shared.execute(&environment_id, &request, &self.cancel);
            {
                let evicted = shared
                    .results
                    .lock()
                    .unwrap()
                    .insert(request.request_id.clone(), result.clone());
                if !evicted.is_empty() {
                    let mut seen = shared.seen.lock().unwrap();
                    for request_id in evicted {
                        seen.remove(&request_id);
                    }
                }
            }
            shared.result_signal.notify_all();
            if let Some(state) = result.environment_state {
                shared.set_known(&request.environment_id, state);
                if let Operation::BranchEnvironment {
                    child_environment_id,
                } = &request.operation
                    && result.status == ResultStatus::Completed
                {
                    shared.set_known(child_environment_id, EnvironmentState::Available);
                }
            }
            let _ = shared.out.send(AdapterMessage::Event(Event {
                environment_id: request.environment_id.clone(),
                request_id: Some(request.request_id.clone()),
                payload: EventPayload::OperationFinished {
                    result: result.clone(),
                },
            }));
            let _ = shared.out.send(AdapterMessage::Result(result));
        }
    }
}
