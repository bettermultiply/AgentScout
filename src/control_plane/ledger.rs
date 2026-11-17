//! Ledger：control plane 的异步旁路记录。
//!
//! 只记录已经发生的运行事实，不参与决策，也不从记录里恢复生命周期。
//! 目标语义见 `docs/design/control-plane.md` §2.6。

use super::observation::{ObservationContent, ObservationEvent};
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

struct ObservationIndex {
    path: std::path::PathBuf,
    sequence: u64,
    connected: bool,
}

#[derive(Clone)]
pub struct LedgerReader {
    index: Arc<(Mutex<ObservationIndex>, Condvar)>,
}

impl LedgerReader {
    pub fn sequence(&self) -> u64 {
        self.index.0.lock().unwrap().sequence
    }
    pub fn connected(&self) -> bool {
        self.index.0.lock().unwrap().connected
    }
    pub fn read_saved_after(&self, after: u64) -> Result<Vec<ObservationEvent>, String> {
        use std::io::BufRead;
        let (path, through) = {
            let index = self.index.0.lock().unwrap();
            (index.path.clone(), index.sequence)
        };
        if through <= after {
            return Ok(Vec::new());
        }
        let file = File::open(path).map_err(|e| e.to_string())?;
        let mut events = Vec::new();
        for line in io::BufReader::new(file).split(b'\n') {
            let line = line.map_err(|e| e.to_string())?;
            let Ok(value) = serde_json::from_slice::<serde_json::Value>(&line) else {
                continue;
            };
            if let Some(value) = value.get("observation") {
                let event: ObservationEvent =
                    serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
                if event.sequence > after && event.sequence <= through {
                    events.push(event);
                }
            }
        }
        Ok(events)
    }
    pub fn events_after(&self, sequence: u64) -> Result<Vec<ObservationEvent>, String> {
        self.events_after_until(sequence, &std::sync::atomic::AtomicBool::new(false))
    }

    pub(crate) fn wake_readers(&self) {
        let _index = self.index.0.lock().unwrap();
        self.index.1.notify_all();
    }

    pub(crate) fn events_after_until(
        &self,
        sequence: u64,
        stopped: &std::sync::atomic::AtomicBool,
    ) -> Result<Vec<ObservationEvent>, String> {
        {
            let mut index = self.index.0.lock().unwrap();
            if sequence > index.sequence {
                return Err("Sequence ahead of this run; reload state event set".into());
            }
            if sequence == index.sequence && index.connected {
                index = self
                    .index
                    .1
                    .wait_timeout_while(index, Duration::from_secs(15), |index| {
                        index.sequence == sequence
                            && index.connected
                            && !stopped.load(std::sync::atomic::Ordering::Acquire)
                    })
                    .unwrap()
                    .0;
                drop(index);
            }
        }
        self.read_saved_after(sequence)
    }
}

#[cfg(test)]
#[path = "ledger_tests.rs"]
mod tests;

use serde_json::Value;

use crate::control_plane::agent_control::BranchRecord;

/// 一次运行的事件记录。文件由调用方给出，Ledger 只负责追加。
pub struct Ledger {
    file: File,
    failed: bool,
    append_boundary: Option<u64>,
    #[cfg(test)]
    write_faults: std::collections::VecDeque<usize>,
    reader: LedgerReader,
}

impl Ledger {
    pub fn open(path: &Path) -> io::Result<Self> {
        use std::io::BufRead;
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(path)?;
        let mut sequence = 0;
        if file.metadata()?.is_file() {
            for line in io::BufReader::new(File::open(path)?).split(b'\n') {
                let line = line?;
                if let Ok(value) = serde_json::from_slice::<Value>(&line) {
                    if let Some(saved) = value["observation"]["sequence"].as_u64() {
                        sequence = sequence.max(saved);
                    }
                }
            }
        }
        Ok(Self {
            file,
            failed: false,
            append_boundary: None,
            #[cfg(test)]
            write_faults: Default::default(),
            reader: LedgerReader {
                index: Arc::new((
                    Mutex::new(ObservationIndex {
                        path: path.to_owned(),
                        sequence,
                        connected: true,
                    }),
                    Condvar::new(),
                )),
            },
        })
    }

    pub fn disconnect_readers(&self) {
        self.reader.index.0.lock().unwrap().connected = false;
        self.reader.index.1.notify_all();
    }
    pub fn reader(&self) -> LedgerReader {
        self.reader.clone()
    }
    pub fn events_after(&self, sequence: u64) -> Result<Vec<ObservationEvent>, String> {
        self.reader.events_after(sequence)
    }
    /// Publish only successfully saved observations.
    pub fn append_observation(
        &mut self,
        task_id: &str,
        content: ObservationContent,
    ) -> io::Result<()> {
        let sequence = self
            .reader
            .sequence()
            .checked_add(1)
            .ok_or_else(|| io::Error::other("observation sequence exhausted"))?;
        let connected = match &content {
            ObservationContent::RunStatus { connected, .. } => Some(*connected),
            _ => None,
        };
        let event = ObservationEvent {
            sequence,
            timestamp_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            task_id: task_id.into(),
            content,
        };
        self.record(serde_json::json!({"message": "observation", "observation": event}))?;
        let mut index = self.reader.index.0.lock().unwrap();
        index.sequence = sequence;
        if let Some(connected) = connected {
            index.connected = connected;
        }
        self.reader.index.1.notify_all();
        Ok(())
    }

    /// 追加一条记录。记录内容由调用方决定，Ledger 不解释它。
    pub fn record(&mut self, record: Value) -> io::Result<()> {
        if self.failed {
            return Err(io::Error::other("Ledger append previously failed"));
        }
        self.append_boundary = self
            .file
            .metadata()
            .ok()
            .filter(|m| m.is_file())
            .map(|m| m.len());
        #[cfg(test)]
        if let Some(bytes) = self.write_faults.pop_front() {
            let encoded = serde_json::to_vec(&record)?;
            self.file.write_all(&encoded[..bytes.min(encoded.len())])?;
            self.failed = true;
            return Err(io::Error::other("injected append failure"));
        }
        let result = writeln!(self.file, "{}", serde_json::to_string(&record)?);
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    /// Archive a departed Branch, retry once after restoring the append boundary.
    /// Final failure is reported before releasing the record; control state is never restored.
    pub fn archive_branch(
        &mut self,
        branch: BranchRecord,
        session: &str,
        system_log: &super::system_log::SystemLog,
    ) -> io::Result<()> {
        let result = match self.write_archive(&branch) {
            Ok(()) => Ok(()),
            Err(first) => match self.restore_append_boundary() {
                Ok(()) => self.write_archive(&branch).map_err(|retry| {
                    io::Error::other(format!("first append failed: {first}; retry failed: {retry}"))
                }),
                Err(recovery) => Err(io::Error::other(format!(
                    "first append failed: {first}; append boundary recovery failed: {recovery}; retry could not be completed"
                ))),
            },
        };
        if let Err(error) = &result {
            system_log.history_failure(
                session,
                &format!(
                    "branch_id={} branch archive failed: {error}",
                    branch.branch_id
                ),
            );
        }
        result
    }

    fn restore_append_boundary(&mut self) -> io::Result<()> {
        use std::os::unix::fs::FileExt;
        let offset = self
            .append_boundary
            .ok_or_else(|| io::Error::other("no safe append boundary"))?;
        let metadata = self.file.metadata()?;
        if !metadata.is_file() || metadata.len() < offset {
            return Err(io::Error::other("append boundary unavailable"));
        }
        if offset > 0 {
            let mut byte = [0];
            if self.file.read_at(&mut byte, offset - 1)? != 1 || byte[0] != b'\n' {
                return Err(io::Error::other("append boundary is not a complete line"));
            }
        }
        self.file.set_len(offset)?;
        self.failed = false;
        Ok(())
    }

    fn write_archive(&mut self, branch: &BranchRecord) -> io::Result<()> {
        self.record(serde_json::json!({
            "message": "branch_archived",
            "branch_id": branch.branch_id,
            "branch": branch,
        }))
    }
}
