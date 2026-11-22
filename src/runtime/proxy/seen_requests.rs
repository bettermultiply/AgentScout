//! Bounded connection-local request identities. Numeric req-N IDs use a
//! contiguous accepted prefix, so out-of-order first deliveries remain valid.
use std::collections::{BTreeSet, HashMap};
const WINDOW: u64 = 256;
const CAPACITY: usize = 1024;
#[derive(Default)]
pub(crate) struct SeenRequests {
    entries: HashMap<String, ([u8; 32], bool)>,
    ahead: BTreeSet<u64>,
    contiguous: u64,
}
fn sequence(id: &str) -> Option<u64> {
    let raw = id.strip_prefix("req-")?;
    let number: u64 = raw.parse().ok()?;
    (number > 0 && raw == number.to_string()).then_some(number)
}
impl SeenRequests {
    pub fn get(&self, id: &str) -> Option<&([u8; 32], bool)> {
        self.entries.get(id)
    }
    pub fn get_mut(&mut self, id: &str) -> Option<&mut ([u8; 32], bool)> {
        self.entries.get_mut(id)
    }
    pub fn admission_error(&mut self, id: &str) -> Option<&'static str> {
        let floor = self.contiguous.saturating_sub(WINDOW);
        self.entries
            .retain(|key, (_, complete)| !*complete || sequence(key).is_none_or(|n| n > floor));
        if sequence(id).is_some_and(|n| n <= floor) {
            return Some("request identity expired; request will not execute again");
        }
        if self.entries.len() >= CAPACITY && !self.entries.contains_key(id) {
            return Some("request identity window full; request was not accepted");
        }
        None
    }
    pub fn insert(&mut self, id: String, value: ([u8; 32], bool)) {
        if let Some(n) = sequence(&id) {
            self.ahead.insert(n);
            while self
                .contiguous
                .checked_add(1)
                .is_some_and(|next| self.ahead.remove(&next))
            {
                self.contiguous += 1;
            }
        }
        self.entries.insert(id, value);
    }
}
