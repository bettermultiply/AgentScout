//! Ledger：控制面的异步旁路记录。
//!
//! 只记录已经发生的运行事实，不参与决策，也不从记录里恢复生命周期。
//! 目标语义见 `docs/design/control-plane.md` §2.6。

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

use serde_json::Value;

use crate::agent_control::BranchRecord;

/// 一次运行的事件记录。文件由调用方给出，Ledger 只负责追加。
pub struct Ledger {
    file: File,
}

impl Ledger {
    pub fn open(path: &Path) -> io::Result<Self> {
        Ok(Self {
            file: OpenOptions::new().create(true).append(true).open(path)?,
        })
    }

    /// 追加一条记录。记录内容由调用方决定，Ledger 不解释它。
    pub fn record(&mut self, record: Value) -> io::Result<()> {
        writeln!(self.file, "{}", serde_json::to_string(&record)?)
    }

    /// 将已经退出活动控制范围的 Branch 交给 Ledger 保存。
    ///
    /// `branch` 按值传入。Ledger 负责决定记录完成后何时释放它；当前
    /// 实现是在追加记录后立即释放，且不强制同步文件缓冲区。
    pub fn archive_branch(&mut self, branch: BranchRecord) -> io::Result<()> {
        self.record(serde_json::json!({
            "message": "branch_archived",
            "branch_id": branch.branch_id,
            "branch": branch,
        }))
    }
}
