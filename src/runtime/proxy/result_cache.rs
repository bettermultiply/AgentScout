
use std::collections::{HashMap, VecDeque};

use crate::protocol::contract::OperationResult;

pub(crate) const RESULT_CACHE_LIMIT: usize = 256;

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
