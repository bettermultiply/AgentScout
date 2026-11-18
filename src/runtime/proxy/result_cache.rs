//! 已完成请求的有界结果窗口。

use std::collections::{HashMap, VecDeque};

use crate::protocol::contract::OperationResult;

pub(crate) const RESULT_CACHE_LIMIT: usize = 256;

/// 已完成请求的有界结果窗口。超出窗口的请求按普通新请求重新执行；窗口
/// 本身只用于重试去重，不保存环境状态。
#[derive(Default)]
pub(crate) struct ResultCache {
    pub(crate) entries: HashMap<String, OperationResult>,
    order: VecDeque<String>,
}

impl ResultCache {
    pub(super) fn get(&self, request_id: &str) -> Option<OperationResult> {
        self.entries.get(request_id).cloned()
    }

    pub(super) fn insert(&mut self, request_id: String, result: OperationResult) -> Vec<String> {
        if self.entries.insert(request_id.clone(), result).is_none() {
            self.order.push_back(request_id);
        }
        let mut evicted = Vec::new();
        while self.order.len() > RESULT_CACHE_LIMIT {
            if let Some(old) = self.order.pop_front() {
                self.entries.remove(&old);
                evicted.push(old);
            }
        }
        evicted
    }
}
