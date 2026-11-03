//! Ledger：控制面的异步旁路记录。
//!
//! 只记录已经发生的运行事实，不参与决策，也不从记录里恢复生命周期。
//! 目标语义见 `docs/design/control-plane.md` §2.6。

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

use serde_json::Value;

use crate::agent_control::BranchRecord;

/// 一次运行的事件记录。文件由调用方给出，Ledger 只负责追加。
pub struct Ledger {
    file: File,
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
            unarchived: BTreeMap::new(),
        })
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
