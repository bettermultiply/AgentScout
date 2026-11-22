
use super::*;
use crate::protocol::contract::*;
use std::thread;
use std::time::Duration;

impl Shared {
    pub(super) fn start_state_watcher(self: Arc<Self>) {
        let shared = Arc::downgrade(&self);
        thread::spawn(move || {
            loop {
                thread::sleep(Duration::from_millis(100));
                let Some(adapter) = shared.upgrade() else {
                    break;
                };
                if adapter.runtime.is_closing() {
                    break;
                }
                for environment_id in adapter.runtime.environments() {
                    let Some(current) = adapter.runtime.state(&environment_id) else {
                        continue;
                    };
                    let previous = adapter.known.lock().unwrap().get(&environment_id).copied();
                    match previous {
                        None => {
                            adapter
                                .known
                                .lock()
                                .unwrap()
                                .insert(environment_id, current);
                        }
                        Some(previous) if previous == current => {}
                        Some(previous) => {
                            adapter
                                .known
                                .lock()
                                .unwrap()
                                .insert(environment_id.clone(), current);
                            let reason = if current == EnvironmentState::Terminated {
                                "environment stopped unexpectedly"
                            } else {
                                "environment state changed"
                            };
                            let event = Event {
                                environment_id,
                                request_id: None,
                                payload: EventPayload::EnvironmentStateChanged {
                                    previous: Some(previous),
                                    current,
                                    reason: reason.to_string(),
                                },
                            };
                            let _ = adapter.out.send(AdapterMessage::Event(event));
                        }
                    }
                }
            }
        });
    }
}
