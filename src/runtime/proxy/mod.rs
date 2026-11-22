
mod commands;
mod dispatch;
mod events;
mod queue;
mod result;
mod result_cache;
mod seen_requests;
mod session;


pub use session::RuntimeProxy;

use super::BaseRuntime;
use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex};

use crate::protocol::contract::{AdapterMessage, EnvironmentState};
use crate::runtime::backend::Backend;

use queue::Queue;
use result_cache::ResultCache;

pub(super) struct Shared {
    pub(super) commands: Option<commands::Commands>,
    pub(super) runtime: Arc<BaseRuntime>,
    pub(super) out: Sender<AdapterMessage>,
    pub(super) queues: Mutex<HashMap<String, Arc<Queue>>>,
    pub(super) seen: Mutex<seen_requests::SeenRequests>,
    pub(super) results: Mutex<ResultCache>,
    pub(super) result_signal: Condvar,
    pub(super) known: Mutex<HashMap<String, EnvironmentState>>,
}
