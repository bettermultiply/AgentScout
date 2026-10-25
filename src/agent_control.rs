//! AScout 控制面管理一个 Agent 的最小状态与 Agent 接入协议。
//!
//! 这里的状态属于控制面。Pi 接入侧只能报告事实并执行返回的命令，不能以
//! 自己的对象保存或裁决 Speculative Tree。

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 一个 Agent 内 Branch 的角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchRole {
    Authority,
    Candidate,
}

/// 控制面已知的 Branch 生命周期。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchStatus {
    Created,
    Running,
    Promoted,
    Discarded,
    Cleaned,
}

/// 控制面保存的一条 Branch 记录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchRecord {
    pub branch_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_branch_id: Option<String>,
    pub role: BranchRole,
    pub status: BranchStatus,
    pub workspace: PathBuf,
}

/// 一个 Agent 的完整投机树视图。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpeculativeTree {
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authority_branch_id: Option<String>,
    pub branches: BTreeMap<String, BranchRecord>,
}

impl SpeculativeTree {
    fn new(agent_id: String) -> Self {
        Self {
            agent_id,
            authority_branch_id: None,
            branches: BTreeMap::new(),
        }
    }
}

/// 控制面向 Pi 接入侧发送的命令。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum AgentCommand {
    Prompt {
        branch_id: String,
        prompt: String,
    },
    Abort {
        branch_id: String,
    },
    StartCandidate {
        branch_id: String,
        parent_branch_id: String,
        operation_id: String,
        tool_name: String,
        workspace: PathBuf,
        prediction: Value,
        context: Vec<Value>,
    },
    ResolvePending {
        branch_id: String,
        operation_id: String,
        accepted: bool,
    },
    StartBranch {
        branch_id: String,
        role: BranchRole,
        workspace: PathBuf,
        prompt: String,
    },
    Promote {
        branch_id: String,
    },
    Discard {
        branch_id: String,
    },
    Cleanup {
        branch_id: String,
        workspace: PathBuf,
    },
}

/// Pi 接入侧向控制面报告的事实或结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "message", rename_all = "snake_case")]
pub enum AgentMessage {
    Output {
        branch_id: String,
        text: String,
    },
    TurnCompleted {
        branch_id: String,
    },
    ToolPending {
        branch_id: String,
        operation_id: String,
        tool_name: String,
        arguments: Value,
        context: Vec<Value>,
    },
    CleanupCompleted {
        branch_id: String,
    },
    Hello {
        agent_id: String,
    },
    BranchStarted {
        branch_id: String,
    },
    ParentOutcome {
        branch_id: String,
        operation_id: String,
        outcome: Value,
    },
    CandidateCompleted {
        branch_id: String,
        completed: bool,
        output_characters: usize,
    },
    BranchFailed {
        branch_id: String,
        reason: String,
    },
}

/// Agent socket 上的双向消息。长度前缀由传输层负责，消息本身只描述方向。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum AgentWireMessage {
    Command(AgentCommand),
    Message(AgentMessage),
}

/// 控制面当前管理的一个 Agent。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlState {
    pub tree: SpeculativeTree,
    pub workspace_root: PathBuf,
}

/// 控制面管理一个 Agent、其 Tree 和 Branch 工作目录。
pub struct AgentController {
    state: ControlState,
}

impl AgentController {
    /// 建立控制面状态，并请求当前 Runtime 承载使用的工作根。
    pub fn new(
        agent_id: impl Into<String>,
        workspace_root: impl Into<PathBuf>,
    ) -> io::Result<Self> {
        let workspace_root = workspace_root.into();
        fs::create_dir_all(&workspace_root)?;
        Ok(Self {
            state: ControlState {
                tree: SpeculativeTree::new(agent_id.into()),
                workspace_root,
            },
        })
    }

    pub fn state(&self) -> &ControlState {
        &self.state
    }

    /// 由控制面登记初始权威 Branch，并要求 Pi 启动它。
    pub fn create_authority(
        &mut self,
        branch_id: impl Into<String>,
        prompt: impl Into<String>,
    ) -> io::Result<AgentCommand> {
        let branch_id = branch_id.into();
        self.insert_branch(
            &branch_id,
            None,
            BranchRole::Authority,
            BranchStatus::Created,
        )?;
        self.state.tree.authority_branch_id = Some(branch_id.clone());
        Ok(AgentCommand::StartBranch {
            role: BranchRole::Authority,
            workspace: self.workspace_for(&branch_id),
            branch_id,
            prompt: prompt.into(),
        })
    }

    /// 由控制面为 Candidate 请求独立执行环境；当前承载暂以工作目录表示。
    pub fn create_candidate(
        &mut self,
        parent_branch_id: &str,
        branch_id: impl Into<String>,
        prompt: impl Into<String>,
    ) -> io::Result<AgentCommand> {
        let branch_id = branch_id.into();
        let parent = self
            .state
            .tree
            .branches
            .get(parent_branch_id)
            .ok_or_else(|| invalid_input("parent branch does not exist"))?;
        if !matches!(
            parent.status,
            BranchStatus::Created | BranchStatus::Running | BranchStatus::Promoted
        ) {
            return Err(invalid_input("parent branch cannot create a candidate"));
        }
        self.insert_branch(
            &branch_id,
            Some(parent_branch_id.to_string()),
            BranchRole::Candidate,
            BranchStatus::Created,
        )?;
        Ok(AgentCommand::StartBranch {
            role: BranchRole::Candidate,
            workspace: self.workspace_for(&branch_id),
            branch_id,
            prompt: prompt.into(),
        })
    }

    /// 接收 Pi 报告的启动事实。
    pub fn handle_message(&mut self, message: AgentMessage) -> io::Result<Vec<AgentCommand>> {
        match message {
            AgentMessage::Hello { agent_id } if agent_id == self.state.tree.agent_id => Ok(vec![]),
            AgentMessage::Hello { .. } => Err(invalid_input("agent id does not match controller")),
            AgentMessage::BranchStarted { branch_id } => {
                let branch = self.branch_mut(&branch_id)?;
                // A start fact already in transit may arrive after cancellation.
                // It must not resurrect a discarded Branch or abort Parent fallback.
                if matches!(
                    branch.status,
                    BranchStatus::Discarded | BranchStatus::Cleaned
                ) {
                    return Ok(vec![]);
                }
                if branch.status != BranchStatus::Created {
                    return Err(invalid_input("branch was not awaiting start"));
                }
                branch.status = BranchStatus::Running;
                Ok(vec![])
            }
            AgentMessage::ParentOutcome { .. } | AgentMessage::CandidateCompleted { .. } => {
                Err(invalid_input(
                    "outcomes are resolved only after the control-plane Policy evaluation",
                ))
            }
            AgentMessage::Output { branch_id, .. } | AgentMessage::TurnCompleted { branch_id } => {
                self.branch(&branch_id)?;
                Ok(vec![])
            }
            AgentMessage::ToolPending { branch_id, .. } => {
                self.branch(&branch_id)?;
                Ok(vec![])
            }
            AgentMessage::CleanupCompleted { branch_id } => {
                self.cleanup_completed(&branch_id)?;
                Ok(vec![])
            }
            AgentMessage::BranchFailed { branch_id, .. } => {
                let branch = self.branch_mut(&branch_id)?;
                branch.status = BranchStatus::Discarded;
                Ok(vec![AgentCommand::Cleanup {
                    branch_id: branch.branch_id.clone(),
                    workspace: branch.workspace.clone(),
                }])
            }
        }
    }

    /// 只有控制面可以接受 Candidate；接受后旧权威失去权威身份。
    pub fn resolve_candidate(
        &mut self,
        branch_id: &str,
        compatible: bool,
    ) -> io::Result<Vec<AgentCommand>> {
        let candidate = self
            .state
            .tree
            .branches
            .get(branch_id)
            .ok_or_else(|| invalid_input("candidate branch does not exist"))?
            .clone();
        if candidate.role != BranchRole::Candidate
            || !(candidate.status == BranchStatus::Running
                || (!compatible && candidate.status == BranchStatus::Created))
        {
            return Err(invalid_input("branch is not a running candidate"));
        }

        if !compatible {
            self.branch_mut(branch_id)?.status = BranchStatus::Discarded;
            return Ok(vec![
                AgentCommand::Discard {
                    branch_id: branch_id.to_string(),
                },
                AgentCommand::Cleanup {
                    branch_id: branch_id.to_string(),
                    workspace: candidate.workspace,
                },
            ]);
        }

        let old_authority = self
            .state
            .tree
            .authority_branch_id
            .clone()
            .ok_or_else(|| invalid_input("tree has no authority branch"))?;
        if candidate.parent_branch_id.as_deref() != Some(&old_authority) {
            return Err(invalid_input(
                "candidate is not a child of current authority",
            ));
        }
        self.branch_mut(&old_authority)?.status = BranchStatus::Discarded;
        let promoted = self.branch_mut(branch_id)?;
        promoted.role = BranchRole::Authority;
        promoted.status = BranchStatus::Promoted;
        self.state.tree.authority_branch_id = Some(branch_id.to_string());
        Ok(vec![
            AgentCommand::Promote {
                branch_id: branch_id.to_string(),
            },
            AgentCommand::Cleanup {
                branch_id: old_authority.clone(),
                workspace: self.workspace_for(&old_authority),
            },
        ])
    }

    /// 清理命令完成后由控制面删除目录并收束状态。
    pub fn cleanup_completed(&mut self, branch_id: &str) -> io::Result<()> {
        if self.branch(branch_id)?.status != BranchStatus::Discarded {
            return Err(invalid_input("branch is not awaiting cleanup"));
        }
        let workspace = self.branch(branch_id)?.workspace.clone();
        if workspace.exists() {
            fs::remove_dir_all(&workspace)?;
        }
        self.branch_mut(branch_id)?.status = BranchStatus::Cleaned;
        Ok(())
    }

    fn insert_branch(
        &mut self,
        branch_id: &str,
        parent_branch_id: Option<String>,
        role: BranchRole,
        status: BranchStatus,
    ) -> io::Result<()> {
        if self.state.tree.branches.contains_key(branch_id) {
            return Err(invalid_input("branch id already exists"));
        }
        let workspace = self.workspace_for(branch_id);
        fs::create_dir_all(&workspace)?;
        self.state.tree.branches.insert(
            branch_id.to_string(),
            BranchRecord {
                branch_id: branch_id.to_string(),
                parent_branch_id,
                role,
                status,
                workspace,
            },
        );
        Ok(())
    }

    fn workspace_for(&self, branch_id: &str) -> PathBuf {
        self.state.workspace_root.join(branch_id)
    }

    fn branch(&self, branch_id: &str) -> io::Result<&BranchRecord> {
        self.state
            .tree
            .branches
            .get(branch_id)
            .ok_or_else(|| invalid_input("branch does not exist"))
    }

    fn branch_mut(&mut self, branch_id: &str) -> io::Result<&mut BranchRecord> {
        self.state
            .tree
            .branches
            .get_mut(branch_id)
            .ok_or_else(|| invalid_input("branch does not exist"))
    }
}

fn invalid_input(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_plane_owns_candidate_workspace_and_promotes_it() {
        let root = tempfile::tempdir().unwrap();
        let mut controller = AgentController::new("agent-1", root.path()).unwrap();
        let start = controller.create_authority("parent", "test").unwrap();
        assert!(matches!(start, AgentCommand::StartBranch { .. }));
        controller
            .handle_message(AgentMessage::BranchStarted {
                branch_id: "parent".into(),
            })
            .unwrap();
        let candidate_start = controller
            .create_candidate("parent", "candidate", "test")
            .unwrap();
        let candidate_workspace = match candidate_start {
            AgentCommand::StartBranch { workspace, .. } => workspace,
            _ => unreachable!(),
        };
        assert!(candidate_workspace.is_dir());
        controller
            .handle_message(AgentMessage::BranchStarted {
                branch_id: "candidate".into(),
            })
            .unwrap();
        let commands = controller.resolve_candidate("candidate", true).unwrap();
        assert!(
            commands
                .iter()
                .any(|command| matches!(command, AgentCommand::Promote { .. }))
        );
        assert_eq!(
            controller.state().tree.authority_branch_id.as_deref(),
            Some("candidate")
        );
        assert_eq!(
            controller.state().tree.branches["parent"].status,
            BranchStatus::Discarded
        );
    }

    #[test]
    fn mismatch_discards_and_cleans_candidate_directory() {
        let root = tempfile::tempdir().unwrap();
        let mut controller = AgentController::new("agent-1", root.path()).unwrap();
        controller.create_authority("parent", "test").unwrap();
        controller
            .handle_message(AgentMessage::BranchStarted {
                branch_id: "parent".into(),
            })
            .unwrap();
        controller
            .create_candidate("parent", "candidate", "test")
            .unwrap();
        controller
            .handle_message(AgentMessage::BranchStarted {
                branch_id: "candidate".into(),
            })
            .unwrap();
        let commands = controller.resolve_candidate("candidate", false).unwrap();
        assert!(
            commands
                .iter()
                .any(|command| matches!(command, AgentCommand::Discard { .. }))
        );
        controller.cleanup_completed("candidate").unwrap();
        assert!(!root.path().join("candidate").exists());
        assert_eq!(
            controller.state().tree.branches["candidate"].status,
            BranchStatus::Cleaned
        );
    }
}
