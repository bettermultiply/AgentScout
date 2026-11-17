//! Ledger：control plane 的异步旁路记录。
//!
//! 只记录已经发生的运行事实，不参与决策，也不从记录里恢复生命周期。
//! 目标语义见 `docs/design/control-plane.md` §2.6。

use super::observation::{ObservationContent, ObservationEvent, StateEventSet};
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// Immutable blocks: readers clone block handles under the lock, then fold/copy
// outside it. Appending never waits for a reader to process the history.
struct ObservationBlock {
    event: ObservationEvent,
    previous: Option<Arc<ObservationBlock>>,
}
impl Drop for ObservationBlock {
    fn drop(&mut self) {
        let mut previous = self.previous.take();
        while let Some(block) = previous {
            match Arc::try_unwrap(block) {
                Ok(mut block) => previous = block.previous.take(),
                Err(_) => break,
            }
        }
    }
}
struct ObservationIndex {
    head: Option<Arc<ObservationBlock>>,
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
    pub fn state_event_set(&self) -> StateEventSet {
        let (sequence, head) = {
            let index = self.index.0.lock().unwrap();
            (index.sequence, index.head.clone())
        };
        StateEventSet::new(sequence, Self::collect(head, 0))
    }
    fn collect(mut head: Option<Arc<ObservationBlock>>, after: u64) -> Vec<ObservationEvent> {
        let mut events = Vec::new();
        while let Some(block) = head {
            if block.event.sequence <= after {
                break;
            }
            events.push(block.event.clone());
            head = block.previous.clone();
        }
        events.reverse();
        events
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
        let head = {
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
            }
            index.head.clone()
        };
        Ok(Self::collect(head, sequence))
    }
}

use serde_json::Value;

use crate::control_plane::agent_control::BranchRecord;

/// 一次运行的事件记录。文件由调用方给出，Ledger 只负责追加。
pub struct Ledger {
    file: File,
    reader: LedgerReader,
    /// 已经由 Ledger 接管、但尚未写入成功的 Branch。
    ///
    /// 写入失败时 Ledger 继续持有这些 Branch；若在错误返回时直接释放，
    /// 历史就会在没有任何组件负责的情况下消失。
    unarchived: BTreeMap<String, BranchRecord>,
}

impl Ledger {
    pub fn open(path: &Path) -> io::Result<Self> {
        Ok(Self {
            file: OpenOptions::new().create(true).append(true).open(path)?,
            reader: LedgerReader {
                index: Arc::new((
                    Mutex::new(ObservationIndex {
                        head: None,
                        sequence: 0,
                        connected: true,
                    }),
                    Condvar::new(),
                )),
            },
            unarchived: BTreeMap::new(),
        })
    }

    pub fn reader(&self) -> LedgerReader {
        self.reader.clone()
    }
    pub fn state_event_set(&self) -> StateEventSet {
        self.reader.state_event_set()
    }
    pub fn events_after(&self, sequence: u64) -> Result<Vec<ObservationEvent>, String> {
        self.reader.events_after(sequence)
    }
    /// Retain the event in the ordered in-process history even if file append
    /// fails. The caller must report that error as incomplete durable history.
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
        let written =
            self.record(serde_json::json!({"message": "observation", "observation": event}));
        let mut index = self.reader.index.0.lock().unwrap();
        let previous = index.head.take();
        index.head = Some(Arc::new(ObservationBlock { event, previous }));
        index.sequence = sequence;
        if let Some(connected) = connected {
            index.connected = connected;
        }
        self.reader.index.1.notify_all();
        written
    }

    /// 追加一条记录。记录内容由调用方决定，Ledger 不解释它。
    pub fn record(&mut self, record: Value) -> io::Result<()> {
        writeln!(self.file, "{}", serde_json::to_string(&record)?)
    }

    /// 将已经退出活动控制范围的 Branch 交给 Ledger 保存。
    ///
    /// `branch` 按值传入，接管后由 Ledger 负责它的去向。写入成功时 Ledger
    /// 按约定释放这份记录，不把内存释放绑定到 `flush`；写入失败时 Ledger
    /// 保留该 Branch，并由调用方报告这次运行的历史不完整。失败可能已经写入
    /// 部分内容，Ledger 不对文件作回滚承诺。
    pub fn archive_branch(&mut self, branch: BranchRecord) -> io::Result<()> {
        if let Err(error) = self.write_archive(&branch) {
            self.unarchived.insert(branch.branch_id.clone(), branch);
            return Err(error);
        }
        self.unarchived.remove(&branch.branch_id);
        Ok(())
    }

    /// 已经接管但未能写入的 Branch ID。非空表示这次运行的历史不完整。
    pub fn unarchived(&self) -> impl Iterator<Item = &str> {
        self.unarchived.keys().map(String::as_str)
    }

    fn write_archive(&mut self, branch: &BranchRecord) -> io::Result<()> {
        self.record(serde_json::json!({
            "message": "branch_archived",
            "branch_id": branch.branch_id,
            "branch": branch,
        }))
    }
}
