//! Adapter 进程：接入控制面会话，按环境串行执行操作，上报结果和事件。
//!
//! 首版只做一条连接、同步完成、一个状态变化事件类型；能力声明、验证和
//! 多 Adapter 管理不在这一版。

mod agent;
mod dispatch;
mod queue;
mod result;
mod result_cache;
mod session;

#[cfg(test)]
mod tests;

pub use session::Adapter;

use std::collections::{HashMap, HashSet};
use std::process::Child;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex};

use crate::protocol::contract::{AdapterMessage, EnvironmentState};
use crate::runtime::backend::Backend;

use queue::Queue;
use result_cache::ResultCache;

pub(super) struct Shared {
    pub(super) runtime: Arc<dyn Backend>,
    pub(super) out: Sender<AdapterMessage>,
    pub(super) queues: Mutex<HashMap<String, Arc<Queue>>>,
    /// 已经受理过的请求标识：重发同一标识按重试处理，不重复执行。
    pub(super) seen: Mutex<HashSet<String>>,
    /// 已经完成的请求结果，按条数有界。重发同一标识时返回同一逻辑结果；
    /// 仍在执行的请求会等待原始结果。
    pub(super) results: Mutex<ResultCache>,
    pub(super) result_signal: Condvar,
    /// Adapter 记录的每个环境最近一次报告的状态，用于发现意外变化。
    pub(super) known: Mutex<HashMap<String, EnvironmentState>>,
    /// 由 Runtime 启动的 Agent 接入侧进程。一个 run 只有一个，服务所有环境。
    pub(super) agent: Mutex<Option<Child>>,
}
