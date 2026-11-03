//! AScout 控制面管理一个 Agent 的最小状态与 Agent 接入协议。
//!
//! 这里的状态属于控制面。Pi 接入侧只能报告事实并执行返回的命令，不能以
//! 自己的对象保存或裁决 Speculative Tree。

use std::collections::{BTreeMap, BTreeSet};
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
    AwaitingValidation,
    Promoting,
    Promoted,
    Discarding,
    Discarded,
    Cleaning,
    Cleaned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandOperation {
    Promote,
    ResolvePending,
    Discard,
    Cleanup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandStatus {
    Applied,
    Rejected,
    Failed,
    Timeout,
}

/// 控制面操作的种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    StartAgent,
    StartCandidate,
    ResolvePending,
    Promote,
    Discard,
    Cleanup,
}

/// 控制面操作的落实状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationStatus {
    Planned,
    Requested,
    Succeeded,
    Failed,
}

/// Agent 接入侧报告的实际状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentExecutionState {
    Unknown,
    StartRequested,
    Running,
    Stopped,
    Released,
    Failed,
}

/// Runtime 报告的实际状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeExecutionState {
    Unknown,
    EnvironmentRequested,
    Ready,
    PromotionRequested,
    Promoted,
    TerminationRequested,
    Terminated,
    Cleaned,
    Failed,
}

impl Default for AgentExecutionState {
    fn default() -> Self {
        Self::Unknown
    }
}

impl Default for RuntimeExecutionState {
    fn default() -> Self {
        Self::Unknown
    }
}

/// 控制面保存的一条 Branch 记录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchRecord {
    pub branch_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_branch_id: Option<String>,
    pub role: BranchRole,
    pub status: BranchStatus,
    #[serde(default)]
    pub generation: u64,
    #[serde(default)]
    pub execution_position: ExecutionPosition,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal: Option<ProposalRecord>,
    #[serde(default)]
    pub pending_operations: Vec<PendingOperation>,
    #[serde(default)]
    pub execution_state: BranchExecutionState,
}

/// Branch 在完整投机执行历史中的位置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionPosition {
    pub sequence: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    #[serde(default)]
    pub transcript_index: usize,
}

impl Default for ExecutionPosition {
    fn default() -> Self {
        Self {
            sequence: 0,
            operation_id: None,
            transcript_index: 0,
        }
    }
}

/// Candidate 携带的预测。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposalRecord {
    pub proposal_id: String,
    pub operation_id: String,
    pub payload: Value,
}

/// 控制面希望发生的操作，不代表外部操作已经完成。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingOperation {
    pub operation_id: String,
    pub branch_id: String,
    pub kind: OperationKind,
    pub status: OperationStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline_at: Option<u64>,
}

/// Branch 对应的 Agent 与 Runtime 实际事实。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BranchExecutionState {
    pub agent: AgentExecutionState,
    pub runtime: RuntimeExecutionState,
}

/// 一个 Agent 的完整投机树视图。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpeculativeTree {
    pub agent_id: String,
    #[serde(default)]
    pub execution_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_root_branch_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authority_branch_id: Option<String>,
    pub branches: BTreeMap<String, BranchRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_binding: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_binding: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_view: Option<Value>,
}

impl SpeculativeTree {
    fn new(agent_id: String) -> Self {
        Self {
            agent_id,
            execution_id: String::new(),
            history_root_branch_id: None,
            authority_branch_id: None,
            branches: BTreeMap::new(),
            policy_binding: None,
            runtime_binding: None,
            capability_view: None,
        }
    }

    /// 绑定这次投机执行使用的 Policy、Runtime Adapter 和能力视图。
    /// 这些值属于 Tree，不属于某个 Branch。
    pub fn bind_execution(&mut self, policy: Value, runtime: Value, capabilities: Value) {
        self.policy_binding = Some(policy);
        self.runtime_binding = Some(runtime);
        self.capability_view = Some(capabilities);
    }

    /// 根据完整历史中的父子关系计算当前执行树。
    /// 当前执行树不是第二套事实，只是从活动根派生的 Branch 集合。
    pub fn active_branch_ids(&self) -> BTreeSet<String> {
        let Some(root) = self.authority_branch_id.as_deref() else {
            return BTreeSet::new();
        };
        let mut ids = BTreeSet::from([root.to_string()]);
        let mut changed = true;
        while changed {
            changed = false;
            for branch in self.branches.values() {
                if branch
                    .parent_branch_id
                    .as_deref()
                    .is_some_and(|parent| ids.contains(parent))
                    && ids.insert(branch.branch_id.clone())
                {
                    changed = true;
                }
            }
        }
        ids
    }

    /// 返回当前执行树中的 Branch，而不是完整历史中的全部 Branch。
    pub fn active_branches(&self) -> Vec<&BranchRecord> {
        let ids = self.active_branch_ids();
        self.branches
            .values()
            .filter(|branch| ids.contains(&branch.branch_id))
            .collect()
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        checkpoint: Option<Value>,
        #[serde(default)]
        command_id: String,
        #[serde(default)]
        generation: u64,
    },
    ResolvePending {
        branch_id: String,
        operation_id: String,
        accepted: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        outcome: Option<Value>,
        #[serde(default)]
        command_id: String,
        #[serde(default)]
        generation: u64,
    },
    StartBranch {
        branch_id: String,
        role: BranchRole,
        workspace: PathBuf,
        prompt: String,
    },
    Promote {
        branch_id: String,
        #[serde(default)]
        command_id: String,
        #[serde(default)]
        generation: u64,
    },
    Discard {
        branch_id: String,
        #[serde(default)]
        command_id: String,
        #[serde(default)]
        generation: u64,
    },
    Cleanup {
        branch_id: String,
        workspace: PathBuf,
        #[serde(default)]
        command_id: String,
        #[serde(default)]
        generation: u64,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        checkpoint: Option<Value>,
    },
    Transcript {
        branch_id: String,
        branch_role: String,
        entry: Value,
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
    Usage {
        branch_id: String,
        model: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thinking_level: Option<String>,
        usage: Value,
    },
    BranchFailed {
        branch_id: String,
        reason: String,
    },
    CommandResult {
        command_id: String,
        branch_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        operation_id: Option<String>,
        operation: CommandOperation,
        status: CommandStatus,
        generation: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
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

impl ControlState {
    /// 一个 run 只有一个工作目录：Branch 是同一执行状态的别名，不各自持有目录。
    pub fn workspace(&self) -> PathBuf {
        self.workspace_root.join("workspace")
    }
}

/// 控制面管理一个 Agent、其 Tree 和本次 run 的工作目录。
pub struct AgentController {
    state: ControlState,
    next_generation: u64,
    next_command_id: u64,
    pending_commands: BTreeMap<String, PendingCommand>,
    /// A model failure arrived while a Candidate was already being promoted.
    /// The promotion result decides whether the failure is a candidate failure
    /// or an authority failure.
    pending_failures: BTreeMap<String, String>,
    authority_failure: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingCommand {
    branch_id: String,
    operation: CommandOperation,
    operation_id: Option<String>,
    generation: u64,
}

impl AgentController {
    /// 建立控制面状态，并请求当前 Runtime 承载使用的工作根。
    pub fn new(
        agent_id: impl Into<String>,
        workspace_root: impl Into<PathBuf>,
    ) -> io::Result<Self> {
        let workspace_root = workspace_root.into();
        fs::create_dir_all(&workspace_root)?;
        let state = ControlState {
            tree: SpeculativeTree::new(agent_id.into()),
            workspace_root,
        };
        fs::create_dir_all(state.workspace())?;
        Ok(Self {
            state,
            next_generation: 1,
            next_command_id: 1,
            pending_commands: BTreeMap::new(),
            pending_failures: BTreeMap::new(),
            authority_failure: None,
        })
    }

    pub fn state(&self) -> &ControlState {
        &self.state
    }

    /// 把 Candidate 的预测和执行位置写入 Tree；候选运行数据随后只从
    /// BranchRecord 读取，不再需要控制循环维护第二份 Proposal 状态。
    pub fn record_candidate_context(
        &mut self,
        branch_id: &str,
        operation_id: impl Into<String>,
        prediction: Value,
        sequence: u64,
        transcript_index: usize,
    ) -> io::Result<()> {
        let operation_id = operation_id.into();
        let branch = self.branch_mut(branch_id)?;
        if branch.role != BranchRole::Candidate {
            return Err(invalid_input("only a candidate can have a proposal"));
        }
        branch.execution_position = ExecutionPosition {
            sequence,
            operation_id: Some(operation_id.clone()),
            transcript_index,
        };
        branch.proposal = Some(ProposalRecord {
            proposal_id: format!("proposal-{branch_id}"),
            operation_id: operation_id.clone(),
            payload: prediction,
        });
        branch.pending_operations.push(PendingOperation {
            operation_id,
            branch_id: branch_id.to_string(),
            kind: OperationKind::StartCandidate,
            status: OperationStatus::Planned,
            outcome: None,
            evidence: None,
            requested_at: None,
            deadline_at: None,
        });
        branch.execution_state.agent = AgentExecutionState::StartRequested;
        Ok(())
    }

    pub fn mark_operation_requested(
        &mut self,
        branch_id: &str,
        operation_id: &str,
        kind: OperationKind,
    ) -> io::Result<()> {
        let branch = self.branch_mut(branch_id)?;
        if let Some(operation) = branch
            .pending_operations
            .iter_mut()
            .find(|operation| operation.operation_id == operation_id)
        {
            operation.status = OperationStatus::Requested;
        } else {
            branch.pending_operations.push(PendingOperation {
                operation_id: operation_id.to_string(),
                branch_id: branch_id.to_string(),
                kind,
                status: OperationStatus::Requested,
                outcome: None,
                evidence: None,
                requested_at: None,
                deadline_at: None,
            });
        }
        Ok(())
    }

    pub fn mark_agent_started(&mut self, branch_id: &str) -> io::Result<()> {
        let branch = self.branch_mut(branch_id)?;
        branch.execution_state.agent = AgentExecutionState::Running;
        Ok(())
    }

    /// Runtime facts are independent of Agent session cleanup. In particular,
    /// releasing a session does not clean a shared execution environment.
    pub fn record_runtime_state(
        &mut self,
        branch_id: &str,
        state: RuntimeExecutionState,
    ) -> io::Result<()> {
        self.branch_mut(branch_id)?.execution_state.runtime = state;
        Ok(())
    }

    /// Orderly process exit also releases the current Authority's session.
    /// Runtime environment cleanup remains a separate confirmed operation.
    pub fn begin_cleanup(&mut self, branch_id: &str) -> io::Result<AgentCommand> {
        self.branch(branch_id)?;
        let command = self.make_cleanup_command(branch_id)?;
        self.branch_mut(branch_id)?.status = BranchStatus::Cleaning;
        Ok(command)
    }

    pub fn complete_operation(
        &mut self,
        branch_id: &str,
        operation_id: Option<&str>,
        status: OperationStatus,
    ) -> io::Result<()> {
        let branch = self.branch_mut(branch_id)?;
        if let Some(operation) = branch
            .pending_operations
            .iter_mut()
            .find(|operation| operation_id.is_none_or(|id| operation.operation_id == id))
        {
            operation.status = status;
        }
        Ok(())
    }

    /// 将外部结果写回拥有该 Proposal 的 Candidate。
    /// 控制循环只提供事件，不在本地保存结果副本。
    pub fn record_candidate_outcome(
        &mut self,
        parent_branch_id: &str,
        operation_id: &str,
        outcome: Value,
        evidence: Option<Value>,
    ) -> io::Result<()> {
        let candidate_ids: Vec<String> = self
            .state
            .tree
            .branches
            .values()
            .filter(|branch| {
                branch.parent_branch_id.as_deref() == Some(parent_branch_id)
                    && branch
                        .proposal
                        .as_ref()
                        .is_some_and(|proposal| proposal.operation_id == operation_id)
            })
            .map(|branch| branch.branch_id.clone())
            .collect();
        for candidate_id in candidate_ids {
            let branch = self.branch_mut(&candidate_id)?;
            if let Some(operation) = branch.pending_operations.iter_mut().find(|operation| {
                operation.operation_id == operation_id
                    && operation.kind == OperationKind::StartCandidate
            }) {
                operation.outcome = Some(outcome.clone());
                operation.evidence = evidence.clone();
            }
        }
        Ok(())
    }

    /// Return terminal Branches that may be handed to the Ledger.
    pub fn evictable_branches(&self) -> Vec<String> {
        self.state
            .tree
            .branches
            .values()
            .filter(|branch| {
                branch.branch_id
                    != self
                        .state
                        .tree
                        .authority_branch_id
                        .as_deref()
                        .unwrap_or_default()
                    && matches!(branch.status, BranchStatus::Cleaned)
                    && !self
                        .pending_commands
                        .values()
                        .any(|command| command.branch_id == branch.branch_id)
                    && !self.state.tree.branches.values().any(|child| {
                        child.parent_branch_id.as_deref() == Some(branch.branch_id.as_str())
                    })
            })
            .map(|branch| branch.branch_id.clone())
            .collect()
    }

    /// 从活动 Tree 中取出终止 Branch，并把它的所有权交给调用方的 Ledger。
    ///
    /// 从这一步开始，核心数据结构已经不再拥有该 Branch；Ledger 决定
    /// 记录对象何时释放。调用方应立即把返回值传给 Ledger。
    pub fn take_archivable_branch(&mut self, branch_id: &str) -> io::Result<BranchRecord> {
        if !self.evictable_branches().iter().any(|id| id == branch_id) {
            return Err(invalid_input("branch is not safe to archive"));
        }
        self.state
            .tree
            .branches
            .remove(branch_id)
            .ok_or_else(|| invalid_input("branch does not exist"))
    }

    /// Set only when the authority Branch itself can no longer continue.
    pub fn authority_failure(&self) -> Option<&str> {
        self.authority_failure.as_deref()
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
        self.state.tree.history_root_branch_id = Some(branch_id.clone());
        Ok(AgentCommand::StartBranch {
            role: BranchRole::Authority,
            workspace: self.state.workspace(),
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
            workspace: self.state.workspace(),
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
                // A start fact already in transit may arrive after cancellation
                // or after a duplicate start report. It must not resurrect a
                // discarded Branch or abort Parent fallback.
                if branch.status == BranchStatus::Created {
                    branch.status = BranchStatus::Running;
                    branch.execution_state.agent = AgentExecutionState::Running;
                }
                Ok(vec![])
            }
            AgentMessage::ParentOutcome { .. } => Err(invalid_input(
                "outcomes are resolved only after the control-plane Policy evaluation",
            )),
            AgentMessage::CandidateCompleted {
                branch_id,
                completed,
                ..
            } => {
                let branch = self.branch_mut(&branch_id)?;
                if completed && branch.status == BranchStatus::Running {
                    branch.status = BranchStatus::AwaitingValidation;
                }
                Ok(vec![])
            }
            AgentMessage::Output { branch_id, .. }
            | AgentMessage::Usage { branch_id, .. }
            | AgentMessage::Transcript { branch_id, .. }
            | AgentMessage::TurnCompleted { branch_id } => {
                self.branch(&branch_id)?;
                Ok(vec![])
            }
            AgentMessage::ToolPending { branch_id, .. } => {
                self.branch(&branch_id)?;
                Ok(vec![])
            }
            AgentMessage::CleanupCompleted { branch_id } => {
                self.branch_mut(&branch_id)?.execution_state.agent = AgentExecutionState::Released;
                Ok(vec![])
            }
            AgentMessage::BranchFailed { branch_id, reason } => {
                let branch = self.branch(&branch_id)?.clone();
                if matches!(
                    branch.status,
                    BranchStatus::Discarded | BranchStatus::Cleaned
                ) {
                    return Ok(vec![]);
                }
                self.branch_mut(&branch_id)?.execution_state.agent = AgentExecutionState::Failed;
                // A failure of the current authority is not a candidate
                // discard: there is no older authority to fall back to.
                if self.state.tree.authority_branch_id.as_deref() == Some(branch_id.as_str()) {
                    self.authority_failure =
                        Some(format!("authority branch {branch_id} failed: {reason}"));
                    return Ok(vec![]);
                }
                // Promote may already be in flight. Do not discard until the
                // Agent has answered whether the authority switch happened.
                if branch.status == BranchStatus::Promoting {
                    self.pending_failures.insert(branch_id, reason);
                    return Ok(vec![]);
                }
                self.branch_mut(&branch_id)?.status = BranchStatus::Discarding;
                Ok(vec![self.make_discard_command(&branch_id)?])
            }
            AgentMessage::CommandResult {
                command_id,
                branch_id,
                operation_id,
                operation,
                status,
                generation,
                ..
            } => {
                let Some(pending) = self.pending_commands.get(&command_id).cloned() else {
                    return Ok(vec![]);
                };
                if pending.branch_id != branch_id
                    || pending.operation != operation
                    || pending.operation_id != operation_id
                    || pending.generation != generation
                {
                    return Ok(vec![]);
                }
                self.pending_commands.remove(&command_id);
                let operation_status = if status == CommandStatus::Applied {
                    OperationStatus::Succeeded
                } else {
                    OperationStatus::Failed
                };
                self.complete_operation(&branch_id, operation_id.as_deref(), operation_status)?;
                let branch = self.branch(&branch_id)?.clone();
                match (pending.operation, status) {
                    (CommandOperation::Promote, CommandStatus::Applied)
                        if branch.status == BranchStatus::Promoting =>
                    {
                        let old_authority = self
                            .state
                            .tree
                            .authority_branch_id
                            .clone()
                            .ok_or_else(|| invalid_input("tree has no authority branch"))?;
                        if old_authority == branch_id {
                            return Ok(vec![]);
                        }
                        self.branch_mut(&branch_id)?.role = BranchRole::Authority;
                        self.branch_mut(&branch_id)?.status = BranchStatus::Promoted;
                        self.state.tree.authority_branch_id = Some(branch_id.clone());
                        // A model failure that arrived while Promote was in
                        // flight is now a failure of the new authority, not
                        // a reason to discard the candidate.
                        if let Some(reason) = self.pending_failures.remove(&branch_id) {
                            self.authority_failure =
                                Some(format!("authority branch {branch_id} failed: {reason}"));
                        }
                        let old = self.branch_mut(&old_authority)?;
                        old.role = BranchRole::Candidate;
                        old.status = BranchStatus::Discarding;
                        Ok(vec![self.make_discard_command(&old_authority)?])
                    }
                    (CommandOperation::Promote, _) if branch.status == BranchStatus::Promoting => {
                        self.pending_failures.remove(&branch_id);
                        self.branch_mut(&branch_id)?.status = BranchStatus::Discarding;
                        Ok(vec![self.make_discard_command(&branch_id)?])
                    }
                    (CommandOperation::Discard, CommandStatus::Applied)
                        if branch.status == BranchStatus::Discarding =>
                    {
                        self.branch_mut(&branch_id)?.execution_state.agent =
                            AgentExecutionState::Stopped;
                        self.branch_mut(&branch_id)?.status = BranchStatus::Cleaning;
                        Ok(vec![self.make_cleanup_command(&branch_id)?])
                    }
                    (CommandOperation::Cleanup, CommandStatus::Applied)
                        if branch.status == BranchStatus::Cleaning =>
                    {
                        self.branch_mut(&branch_id)?.execution_state.agent =
                            AgentExecutionState::Released;
                        self.branch_mut(&branch_id)?.status = BranchStatus::Cleaned;
                        Ok(vec![])
                    }
                    _ => Ok(vec![]),
                }
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
                || candidate.status == BranchStatus::AwaitingValidation
                || (!compatible && candidate.status == BranchStatus::Created))
        {
            return Err(invalid_input("branch is not a running candidate"));
        }

        if !compatible {
            self.branch_mut(branch_id)?.status = BranchStatus::Discarding;
            return Ok(vec![self.make_discard_command(branch_id)?]);
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
        if self
            .state
            .tree
            .branches
            .values()
            .any(|branch| branch.status == BranchStatus::Promoting)
        {
            return Err(invalid_input("a promotion is already in progress"));
        }
        if !matches!(
            self.branch(&old_authority)?.status,
            BranchStatus::Running | BranchStatus::Promoted
        ) {
            return Err(invalid_input("current authority is not live"));
        }
        let promoted = self.branch_mut(branch_id)?;
        promoted.status = BranchStatus::Promoting;
        Ok(vec![self.make_promote_command(branch_id)?])
    }

    /// 清理命令完成后由控制面收束状态；run 的工作目录由整个 run 共享，不随 Branch 删除。
    pub fn cleanup_completed(&mut self, branch_id: &str) -> io::Result<()> {
        if self.branch(branch_id)?.status != BranchStatus::Cleaning {
            return Err(invalid_input("branch is not awaiting cleanup"));
        }
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
        fs::create_dir_all(self.state.workspace())?;
        let generation = self.next_generation;
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .ok_or_else(|| invalid_input("generation counter exhausted"))?;
        self.state.tree.branches.insert(
            branch_id.to_string(),
            BranchRecord {
                branch_id: branch_id.to_string(),
                parent_branch_id,
                role,
                status,
                generation,
                execution_position: Default::default(),
                proposal: None,
                pending_operations: Vec::new(),
                execution_state: Default::default(),
            },
        );
        Ok(())
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

    fn command_id(&mut self, operation: CommandOperation) -> io::Result<String> {
        let id = format!(
            "command-{}-{}",
            self.next_command_id,
            operation_name(operation)
        );
        self.next_command_id = self
            .next_command_id
            .checked_add(1)
            .ok_or_else(|| invalid_input("command counter exhausted"))?;
        Ok(id)
    }

    fn register(
        &mut self,
        branch_id: &str,
        operation: CommandOperation,
        operation_id: Option<String>,
        generation: u64,
    ) -> io::Result<String> {
        let id = self.command_id(operation)?;
        self.pending_commands.insert(
            id.clone(),
            PendingCommand {
                branch_id: branch_id.to_string(),
                operation,
                operation_id,
                generation,
            },
        );
        Ok(id)
    }

    fn make_promote_command(&mut self, branch_id: &str) -> io::Result<AgentCommand> {
        let generation = self.branch(branch_id)?.generation;
        let command_id = self.register(branch_id, CommandOperation::Promote, None, generation)?;
        Ok(AgentCommand::Promote {
            branch_id: branch_id.to_string(),
            command_id,
            generation,
        })
    }

    fn make_discard_command(&mut self, branch_id: &str) -> io::Result<AgentCommand> {
        let generation = self.branch(branch_id)?.generation;
        let command_id = self.register(branch_id, CommandOperation::Discard, None, generation)?;
        Ok(AgentCommand::Discard {
            branch_id: branch_id.to_string(),
            command_id,
            generation,
        })
    }

    fn make_cleanup_command(&mut self, branch_id: &str) -> io::Result<AgentCommand> {
        let generation = self.branch(branch_id)?.generation;
        let command_id = self.register(branch_id, CommandOperation::Cleanup, None, generation)?;
        Ok(AgentCommand::Cleanup {
            branch_id: branch_id.to_string(),
            workspace: self.state.workspace(),
            command_id,
            generation,
        })
    }
}

fn operation_name(operation: CommandOperation) -> &'static str {
    match operation {
        CommandOperation::Promote => "promote",
        CommandOperation::ResolvePending => "resolve",
        CommandOperation::Discard => "discard",
        CommandOperation::Cleanup => "cleanup",
    }
}

fn invalid_input(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn controller() -> AgentController {
        AgentController {
            state: ControlState {
                tree: SpeculativeTree::new("agent-1".into()),
                workspace_root: PathBuf::from("/memory"),
            },
            next_generation: 1,
            next_command_id: 1,
            pending_commands: BTreeMap::new(),
            pending_failures: BTreeMap::new(),
            authority_failure: None,
        }
    }

    #[test]
    fn candidate_shares_run_workspace_and_promotes() {
        let mut controller = controller();
        controller.state.tree.branches.insert(
            "parent".into(),
            BranchRecord {
                branch_id: "parent".into(),
                parent_branch_id: None,
                role: BranchRole::Authority,
                status: BranchStatus::Running,
                generation: 1,
                execution_position: Default::default(),
                proposal: None,
                pending_operations: Vec::new(),
                execution_state: Default::default(),
            },
        );
        controller.state.tree.branches.insert(
            "candidate".into(),
            BranchRecord {
                branch_id: "candidate".into(),
                parent_branch_id: Some("parent".into()),
                role: BranchRole::Candidate,
                status: BranchStatus::Running,
                generation: 2,

                execution_position: Default::default(),
                proposal: None,
                pending_operations: Vec::new(),
                execution_state: Default::default(),
            },
        );
        controller.state.tree.authority_branch_id = Some("parent".into());
        let commands = controller.resolve_candidate("candidate", true).unwrap();
        assert!(commands
            .iter()
            .any(|command| matches!(command, AgentCommand::Promote { .. })));
        assert_eq!(
            controller.state().tree.authority_branch_id.as_deref(),
            Some("parent")
        );
        assert_eq!(
            controller.state().tree.branches["parent"].status,
            BranchStatus::Running
        );
        let promote = commands
            .into_iter()
            .find_map(|command| match command {
                AgentCommand::Promote {
                    command_id,
                    generation,
                    ..
                } => Some((command_id, generation)),
                _ => None,
            })
            .unwrap();
        let commands = controller
            .handle_message(AgentMessage::CommandResult {
                command_id: promote.0,
                branch_id: "candidate".into(),
                operation_id: None,
                operation: CommandOperation::Promote,
                status: CommandStatus::Applied,
                generation: promote.1,
                reason: None,
            })
            .unwrap();
        assert!(commands
            .iter()
            .any(|command| matches!(command, AgentCommand::Discard { .. })));
        assert_eq!(
            controller.state().tree.authority_branch_id.as_deref(),
            Some("candidate")
        );
    }

    #[test]
    fn mismatch_discards_and_cleans_candidate() {
        let mut controller = controller();
        controller.state.tree.branches.insert(
            "parent".into(),
            BranchRecord {
                branch_id: "parent".into(),
                parent_branch_id: None,
                role: BranchRole::Authority,
                status: BranchStatus::Running,
                generation: 1,
                execution_position: Default::default(),
                proposal: None,
                pending_operations: Vec::new(),
                execution_state: Default::default(),
            },
        );
        controller.state.tree.branches.insert(
            "candidate".into(),
            BranchRecord {
                branch_id: "candidate".into(),
                parent_branch_id: Some("parent".into()),
                role: BranchRole::Candidate,
                status: BranchStatus::Running,
                generation: 2,

                execution_position: Default::default(),
                proposal: None,
                pending_operations: Vec::new(),
                execution_state: Default::default(),
            },
        );
        controller.state.tree.authority_branch_id = Some("parent".into());
        let commands = controller.resolve_candidate("candidate", false).unwrap();
        assert!(commands
            .iter()
            .any(|command| matches!(command, AgentCommand::Discard { .. })));
        let discard = commands
            .into_iter()
            .find_map(|command| match command {
                AgentCommand::Discard {
                    command_id,
                    generation,
                    ..
                } => Some((command_id, generation)),
                _ => None,
            })
            .unwrap();
        let commands = controller
            .handle_message(AgentMessage::CommandResult {
                command_id: discard.0,
                branch_id: "candidate".into(),
                operation_id: None,
                operation: CommandOperation::Discard,
                status: CommandStatus::Applied,
                generation: discard.1,
                reason: None,
            })
            .unwrap();
        let cleanup = commands
            .into_iter()
            .find_map(|command| match command {
                AgentCommand::Cleanup {
                    command_id,
                    generation,
                    ..
                } => Some((command_id, generation)),
                _ => None,
            })
            .unwrap();
        controller
            .handle_message(AgentMessage::CommandResult {
                command_id: cleanup.0,
                branch_id: "candidate".into(),
                operation_id: None,
                operation: CommandOperation::Cleanup,
                status: CommandStatus::Applied,
                generation: cleanup.1,
                reason: None,
            })
            .unwrap();
        assert_eq!(
            controller.state().tree.branches["candidate"].status,
            BranchStatus::Cleaned
        );
    }

    #[test]
    fn command_results_require_exact_pending_identity_and_are_consumed() {
        let mut controller = controller();
        controller.state.tree.branches.insert(
            "parent".into(),
            BranchRecord {
                branch_id: "parent".into(),
                parent_branch_id: None,
                role: BranchRole::Authority,
                status: BranchStatus::Running,
                generation: 1,
                execution_position: Default::default(),
                proposal: None,
                pending_operations: Vec::new(),
                execution_state: Default::default(),
            },
        );
        controller.state.tree.branches.insert(
            "candidate".into(),
            BranchRecord {
                branch_id: "candidate".into(),
                parent_branch_id: Some("parent".into()),
                role: BranchRole::Candidate,
                status: BranchStatus::Running,
                generation: 2,

                execution_position: Default::default(),
                proposal: None,
                pending_operations: Vec::new(),
                execution_state: Default::default(),
            },
        );
        controller.state.tree.authority_branch_id = Some("parent".into());
        let command = controller
            .resolve_candidate("candidate", true)
            .unwrap()
            .remove(0);
        let (command_id, generation) = match command {
            AgentCommand::Promote {
                command_id,
                generation,
                ..
            } => (command_id, generation),
            _ => unreachable!(),
        };
        let stale = controller
            .handle_message(AgentMessage::CommandResult {
                command_id: command_id.clone(),
                branch_id: "candidate".into(),
                operation_id: None,
                operation: CommandOperation::Promote,
                status: CommandStatus::Applied,
                generation: generation + 1,
                reason: None,
            })
            .unwrap();
        assert!(stale.is_empty());
        assert_eq!(
            controller.state().tree.authority_branch_id.as_deref(),
            Some("parent")
        );
        controller
            .handle_message(AgentMessage::CommandResult {
                command_id: command_id.clone(),
                branch_id: "candidate".into(),
                operation_id: None,
                operation: CommandOperation::Promote,
                status: CommandStatus::Applied,
                generation,
                reason: None,
            })
            .unwrap();
        let duplicate = controller
            .handle_message(AgentMessage::CommandResult {
                command_id,
                branch_id: "candidate".into(),
                operation_id: None,
                operation: CommandOperation::Promote,
                status: CommandStatus::Applied,
                generation,
                reason: None,
            })
            .unwrap();
        assert!(duplicate.is_empty());
        assert_eq!(
            controller.state().tree.authority_branch_id.as_deref(),
            Some("candidate")
        );
    }

    #[test]
    fn failed_promotion_discards_without_switching_authority() {
        let mut controller = controller();
        controller.state.tree.branches.insert(
            "parent".into(),
            BranchRecord {
                branch_id: "parent".into(),
                parent_branch_id: None,
                role: BranchRole::Authority,
                status: BranchStatus::Running,
                generation: 1,
                execution_position: Default::default(),
                proposal: None,
                pending_operations: Vec::new(),
                execution_state: Default::default(),
            },
        );
        controller.state.tree.branches.insert(
            "candidate".into(),
            BranchRecord {
                branch_id: "candidate".into(),
                parent_branch_id: Some("parent".into()),
                role: BranchRole::Candidate,
                status: BranchStatus::Running,
                generation: 2,

                execution_position: Default::default(),
                proposal: None,
                pending_operations: Vec::new(),
                execution_state: Default::default(),
            },
        );
        controller.state.tree.authority_branch_id = Some("parent".into());
        let command = controller
            .resolve_candidate("candidate", true)
            .unwrap()
            .remove(0);
        let (command_id, generation) = match command {
            AgentCommand::Promote {
                command_id,
                generation,
                ..
            } => (command_id, generation),
            _ => unreachable!(),
        };
        let commands = controller
            .handle_message(AgentMessage::CommandResult {
                command_id,
                branch_id: "candidate".into(),
                operation_id: None,
                operation: CommandOperation::Promote,
                status: CommandStatus::Failed,
                generation,
                reason: Some("failed".into()),
            })
            .unwrap();
        assert!(commands
            .iter()
            .any(|command| matches!(command, AgentCommand::Discard { .. })));
        assert_eq!(
            controller.state().tree.authority_branch_id.as_deref(),
            Some("parent")
        );
        assert_eq!(
            controller.state().tree.branches["candidate"].status,
            BranchStatus::Discarding
        );
    }

    #[test]
    fn branch_failure_during_promotion_waits_for_promote_result() {
        let mut controller = controller();
        controller.state.tree.branches.insert(
            "parent".into(),
            BranchRecord {
                branch_id: "parent".into(),
                parent_branch_id: None,
                role: BranchRole::Authority,
                status: BranchStatus::Running,
                generation: 1,
                execution_position: Default::default(),
                proposal: None,
                pending_operations: Vec::new(),
                execution_state: Default::default(),
            },
        );
        controller.state.tree.branches.insert(
            "candidate".into(),
            BranchRecord {
                branch_id: "candidate".into(),
                parent_branch_id: Some("parent".into()),
                role: BranchRole::Candidate,
                status: BranchStatus::Running,
                generation: 2,

                execution_position: Default::default(),
                proposal: None,
                pending_operations: Vec::new(),
                execution_state: Default::default(),
            },
        );
        controller.state.tree.authority_branch_id = Some("parent".into());
        let command = controller
            .resolve_candidate("candidate", true)
            .unwrap()
            .remove(0);
        let (command_id, generation) = match command {
            AgentCommand::Promote {
                command_id,
                generation,
                ..
            } => (command_id, generation),
            _ => unreachable!(),
        };

        let commands = controller
            .handle_message(AgentMessage::BranchFailed {
                branch_id: "candidate".into(),
                reason: "429".into(),
            })
            .unwrap();

        assert!(commands.is_empty());
        assert_eq!(
            controller.state().tree.branches["candidate"].status,
            BranchStatus::Promoting
        );
        assert_eq!(
            controller.state().tree.authority_branch_id.as_deref(),
            Some("parent")
        );
        assert!(controller.authority_failure().is_none());

        let commands = controller
            .handle_message(AgentMessage::CommandResult {
                command_id,
                branch_id: "candidate".into(),
                operation_id: None,
                operation: CommandOperation::Promote,
                status: CommandStatus::Applied,
                generation,
                reason: None,
            })
            .unwrap();

        assert_eq!(
            controller.state().tree.authority_branch_id.as_deref(),
            Some("candidate")
        );
        assert_eq!(
            controller.state().tree.branches["candidate"].status,
            BranchStatus::Promoted
        );
        assert!(controller.authority_failure().is_some());
        assert!(commands.iter().any(|command| matches!(
            command,
            AgentCommand::Discard { branch_id, .. } if branch_id == "parent"
        )));
    }

    #[test]
    fn late_promote_result_after_failed_promotion_is_ignored() {
        let mut controller = controller();
        controller.state.tree.branches.insert(
            "parent".into(),
            BranchRecord {
                branch_id: "parent".into(),
                parent_branch_id: None,
                role: BranchRole::Authority,
                status: BranchStatus::Running,
                generation: 1,
                execution_position: Default::default(),
                proposal: None,
                pending_operations: Vec::new(),
                execution_state: Default::default(),
            },
        );
        controller.state.tree.branches.insert(
            "candidate".into(),
            BranchRecord {
                branch_id: "candidate".into(),
                parent_branch_id: Some("parent".into()),
                role: BranchRole::Candidate,
                status: BranchStatus::Running,
                generation: 2,

                execution_position: Default::default(),
                proposal: None,
                pending_operations: Vec::new(),
                execution_state: Default::default(),
            },
        );
        controller.state.tree.authority_branch_id = Some("parent".into());
        let command = controller
            .resolve_candidate("candidate", true)
            .unwrap()
            .remove(0);
        let (command_id, generation) = match command {
            AgentCommand::Promote {
                command_id,
                generation,
                ..
            } => (command_id, generation),
            _ => unreachable!(),
        };
        controller
            .handle_message(AgentMessage::BranchFailed {
                branch_id: "candidate".into(),
                reason: "429".into(),
            })
            .unwrap();
        controller
            .handle_message(AgentMessage::CommandResult {
                command_id: command_id.clone(),
                branch_id: "candidate".into(),
                operation_id: None,
                operation: CommandOperation::Promote,
                status: CommandStatus::Failed,
                generation,
                reason: Some("promote rejected".into()),
            })
            .unwrap();
        assert_eq!(
            controller.state().tree.branches["candidate"].status,
            BranchStatus::Discarding
        );
        assert_eq!(
            controller.state().tree.authority_branch_id.as_deref(),
            Some("parent")
        );

        let commands = controller
            .handle_message(AgentMessage::CommandResult {
                command_id,
                branch_id: "candidate".into(),
                operation_id: None,
                operation: CommandOperation::Promote,
                status: CommandStatus::Applied,
                generation,
                reason: None,
            })
            .unwrap();

        assert!(commands.is_empty());
        assert_eq!(
            controller.state().tree.branches["candidate"].status,
            BranchStatus::Discarding
        );
        assert_eq!(
            controller.state().tree.authority_branch_id.as_deref(),
            Some("parent")
        );
        assert!(controller.authority_failure().is_none());
    }

    #[test]
    fn authority_branch_failure_fails_the_run_without_discard() {
        let mut controller = controller();
        controller.state.tree.branches.insert(
            "authority".into(),
            BranchRecord {
                branch_id: "authority".into(),
                parent_branch_id: None,
                role: BranchRole::Authority,
                status: BranchStatus::Running,
                generation: 1,
                execution_position: Default::default(),
                proposal: None,
                pending_operations: Vec::new(),
                execution_state: Default::default(),
            },
        );
        controller.state.tree.authority_branch_id = Some("authority".into());

        let commands = controller
            .handle_message(AgentMessage::BranchFailed {
                branch_id: "authority".into(),
                reason: "429".into(),
            })
            .unwrap();

        assert!(commands.is_empty());
        assert_eq!(
            controller.state().tree.branches["authority"].status,
            BranchStatus::Running
        );
        assert!(controller.authority_failure().is_some());
    }

    #[test]
    fn late_or_duplicate_branch_started_is_ignored() {
        let mut controller = controller();
        for status in [
            BranchStatus::Running,
            BranchStatus::Discarding,
            BranchStatus::Discarded,
            BranchStatus::Cleaning,
            BranchStatus::Cleaned,
        ] {
            controller.state.tree.branches.insert(
                "candidate".into(),
                BranchRecord {
                    branch_id: "candidate".into(),
                    parent_branch_id: None,
                    role: BranchRole::Candidate,
                    status,
                    generation: 1,

                    execution_position: Default::default(),
                    proposal: None,
                    pending_operations: Vec::new(),
                    execution_state: Default::default(),
                },
            );
            let commands = controller
                .handle_message(AgentMessage::BranchStarted {
                    branch_id: "candidate".into(),
                })
                .unwrap();
            assert!(commands.is_empty());
            assert_eq!(controller.state().tree.branches["candidate"].status, status);
        }
    }

    #[test]
    fn candidate_completion_only_marks_readiness_and_cleanup_failure_is_not_cleaned() {
        let mut controller = controller();
        controller.state.tree.branches.insert(
            "candidate".into(),
            BranchRecord {
                branch_id: "candidate".into(),
                parent_branch_id: None,
                role: BranchRole::Candidate,
                status: BranchStatus::Running,
                generation: 1,
                execution_position: Default::default(),
                proposal: None,
                pending_operations: Vec::new(),
                execution_state: Default::default(),
            },
        );
        controller
            .handle_message(AgentMessage::CandidateCompleted {
                branch_id: "candidate".into(),
                completed: true,
                output_characters: 4,
            })
            .unwrap();
        assert_eq!(
            controller.state().tree.branches["candidate"].status,
            BranchStatus::AwaitingValidation
        );
        controller
            .state
            .tree
            .branches
            .get_mut("candidate")
            .unwrap()
            .status = BranchStatus::Cleaning;
        let command = controller.make_cleanup_command("candidate").unwrap();
        let (command_id, generation) = match command {
            AgentCommand::Cleanup {
                command_id,
                generation,
                ..
            } => (command_id, generation),
            _ => unreachable!(),
        };
        controller
            .handle_message(AgentMessage::CommandResult {
                command_id,
                branch_id: "candidate".into(),
                operation_id: None,
                operation: CommandOperation::Cleanup,
                status: CommandStatus::Failed,
                generation,
                reason: Some("busy".into()),
            })
            .unwrap();
        assert_eq!(
            controller.state().tree.branches["candidate"].status,
            BranchStatus::Cleaning
        );
        let command = controller.begin_cleanup("candidate").unwrap();
        let AgentCommand::Cleanup {
            command_id,
            generation,
            ..
        } = command
        else {
            unreachable!()
        };
        controller
            .handle_message(AgentMessage::CommandResult {
                command_id,
                branch_id: "candidate".into(),
                operation_id: None,
                operation: CommandOperation::Cleanup,
                status: CommandStatus::Applied,
                generation,
                reason: None,
            })
            .unwrap();
        assert_eq!(
            controller.state().tree.branches["candidate"]
                .execution_state
                .agent,
            AgentExecutionState::Released
        );
        // A late start fact must not resurrect a released Agent session.
        controller
            .handle_message(AgentMessage::BranchStarted {
                branch_id: "candidate".into(),
            })
            .unwrap();
        assert_eq!(
            controller.state().tree.branches["candidate"]
                .execution_state
                .agent,
            AgentExecutionState::Released
        );
        assert_eq!(
            controller.state().tree.branches["candidate"]
                .execution_state
                .runtime,
            RuntimeExecutionState::Unknown
        );
    }
}
