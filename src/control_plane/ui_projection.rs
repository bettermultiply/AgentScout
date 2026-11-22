use super::{
    ledger::LedgerReader,
    observation::{ObservationEvent, StateEventSet},
    system_log::SystemLog,
};
use std::sync::{Arc, Mutex};
pub struct UiProjection {
    reader: LedgerReader,
    cache: Arc<Mutex<Vec<ObservationEvent>>>,
    log: Option<SystemLog>,
    connections: Arc<Mutex<std::collections::BTreeMap<u64, u64>>>,
    next_connection: Arc<std::sync::atomic::AtomicU64>,
    connection: u64,
}
impl UiProjection {
    pub fn new(reader: LedgerReader) -> Self {
        Self {
            reader,
            cache: Default::default(),
            log: None,
            connections: Default::default(),
            next_connection: Arc::new(std::sync::atomic::AtomicU64::new(1)),
            connection: 0,
        }
    }
    pub fn with_log(mut self, log: SystemLog) -> Self {
        self.log = Some(log);
        self
    }
    pub fn discard_cache(&self) {
        self.cache.lock().unwrap().clear();
    }
    pub fn sequence(&self) -> u64 {
        self.reader.sequence()
    }
    pub fn connected(&self) -> bool {
        self.reader.connected()
    }
    pub fn warnings(&self) -> Vec<String> {
        self.log
            .as_ref()
            .map(SystemLog::warnings)
            .unwrap_or_default()
    }
    pub fn state_event_set(&self) -> StateEventSet {
        let mut cache = self.cache.lock().unwrap();
        let after = cache.last().map(|e| e.sequence).unwrap_or(0);
        if self.reader.sequence() > after {
            cache.extend(
                self.reader
                    .read_saved_after(after)
                    .expect("cannot read saved Ledger observations"),
            );
        }
        let saved =
            StateEventSet::new(cache.last().map(|e| e.sequence).unwrap_or(0), cache.clone());
        self.connections
            .lock()
            .unwrap()
            .insert(self.connection, saved.sequence);
        saved
    }
    pub fn events_after(&self, sequence: u64) -> Result<Vec<ObservationEvent>, String> {
        self.reader.events_after(sequence)
    }
    pub(crate) fn wake_readers(&self) {
        self.reader.wake_readers();
    }
    pub(crate) fn events_after_until(
        &self,
        sequence: u64,
        stopped: &std::sync::atomic::AtomicBool,
    ) -> Result<Vec<ObservationEvent>, String> {
        let events = self.reader.events_after_until(sequence, stopped)?;
        self.connections.lock().unwrap().insert(
            self.connection,
            events.last().map(|e| e.sequence).unwrap_or(sequence),
        );
        Ok(events)
    }
}

impl Clone for UiProjection {
    fn clone(&self) -> Self {
        let connection = self
            .next_connection
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.connections.lock().unwrap().insert(connection, 0);
        Self {
            reader: self.reader.clone(),
            cache: self.cache.clone(),
            log: self.log.clone(),
            connections: self.connections.clone(),
            next_connection: self.next_connection.clone(),
            connection,
        }
    }
}
impl Drop for UiProjection {
    fn drop(&mut self) {
        self.connections.lock().unwrap().remove(&self.connection);
    }
}
