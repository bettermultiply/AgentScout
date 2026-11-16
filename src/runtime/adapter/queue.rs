//! 按环境串行执行的请求队列。

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
                if !evicted.is_empty() {
                    let mut seen = shared.seen.lock().unwrap();
                    for request_id in evicted {
                        seen.remove(&request_id);
                    }
                }
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
