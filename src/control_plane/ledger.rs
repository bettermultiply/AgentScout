
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


use serde_json::Value;

use crate::control_plane::speculative_tree::BranchRecord;

#[derive(Clone)]
pub struct Ledger {
    inner: Arc<Mutex<LedgerWriter>>,
}

// All opens of a canonical path share the writer's failure state and observation index.
impl Ledger {
    pub fn open(path: &Path) -> io::Result<Self> {
        use std::sync::{OnceLock, Weak};
        static WRITERS: OnceLock<
            Mutex<std::collections::HashMap<std::path::PathBuf, Weak<Mutex<LedgerWriter>>>>,
        > = OnceLock::new();
        let path = if path.exists() {
            std::fs::canonicalize(path)?
        } else {
            std::fs::canonicalize(path.parent().unwrap_or(Path::new(".")))?.join(
                path.file_name()
                    .ok_or_else(|| io::Error::other("Ledger path missing filename"))?,
            )
        };
        let mut writers = WRITERS.get_or_init(Default::default).lock().unwrap();
        if let Some(inner) = writers.get(&path).and_then(Weak::upgrade) {
            return Ok(Self { inner });
        }
        let inner = Arc::new(Mutex::new(LedgerWriter::open(&path)?));
        writers.retain(|_, writer| writer.strong_count() > 0);
        writers.insert(path, Arc::downgrade(&inner));
        Ok(Self { inner })
    }
    pub fn set_system_log(&mut self, log: super::system_log::SystemLog) {
        self.inner.lock().unwrap().system_log = Some(log);
    }
    pub fn warning(&self, description: &str) {
        let log = self.inner.lock().unwrap().system_log.clone();
        let result = log
            .ok_or_else(|| io::Error::other("System log unavailable"))
            .and_then(|log| log.write("WARN", "recovery", None, description));
        if let Err(error) = result {
            eprintln!("System log write failed: {error}; {description}");
        }
    }
    pub fn reader(&self) -> LedgerReader {
        self.inner.lock().unwrap().reader()
    }
    pub fn disconnect_readers(&self) {
        self.inner.lock().unwrap().disconnect_readers();
    }
    pub fn events_after(&self, sequence: u64) -> Result<Vec<ObservationEvent>, String> {
        self.reader().events_after(sequence)
    }
    pub fn record(&mut self, record: Value) -> io::Result<()> {
        self.inner.lock().unwrap().record(record)
    }
    pub fn record_durable(&mut self, record: Value) -> io::Result<()> {
        self.inner.lock().unwrap().record_durable(record)
    }
    pub fn append_observation(
        &mut self,
        task_id: &str,
        content: ObservationContent,
    ) -> io::Result<()> {
        self.inner
            .lock()
            .unwrap()
            .append_observation(task_id, content)
    }
    pub fn archive_branch(
        &mut self,
        branch: BranchRecord,
        run: &str,
        log: &super::system_log::SystemLog,
    ) -> io::Result<()> {
        self.inner
            .lock()
            .unwrap()
            .archive_branch(branch, run, log)
    }
}

struct LedgerWriter {
    system_log: Option<super::system_log::SystemLog>,
    file: File,
    failed: bool,
    append_boundary: Option<u64>,
    reader: LedgerReader,
}

impl LedgerWriter {
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
            system_log: None,
            file,
            failed: false,
            append_boundary: None,
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

    pub fn record_durable(&mut self, record: Value) -> io::Result<()> {
        if self.failed {
            return Err(io::Error::other("Ledger append previously failed"));
        }
        let result = self.record(record).and_then(|()| self.file.sync_all());
        if result.is_err() {
            self.failed = true;
            self.restore_append_boundary()?;
            if let Err(error) = self.file.sync_all() {
                self.failed = true;
                return Err(error);
            }
        }
        result
    }

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
        let result = writeln!(self.file, "{}", serde_json::to_string(&record)?);
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    /// Retry the archive once after restoring the append boundary. Control state stays advanced.
    pub fn archive_branch(
        &mut self,
        branch: BranchRecord,
        run: &str,
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
                run,
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
