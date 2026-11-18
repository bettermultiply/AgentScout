//! Runtime proxy 接入 control plane，按环境排队并上报结果和事件。

mod commands;
mod dispatch;
mod events;
mod queue;
mod result;
mod result_cache;
mod seen_requests;
mod session;

#[cfg(test)]
mod tests;

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
    /// 已经受理过的请求标识：重发同一标识按重试处理，不重复执行。
    pub(super) seen: Mutex<seen_requests::SeenRequests>,
    /// 已经完成的请求结果，按条数有界。重发同一标识时返回同一逻辑结果；
    /// 仍在执行的请求会等待原始结果。
    pub(super) results: Mutex<ResultCache>,
    pub(super) result_signal: Condvar,
    /// Runtime proxy 记录的每个环境最近一次报告的状态，用于发现意外变化。
    pub(super) known: Mutex<HashMap<String, EnvironmentState>>,
}
