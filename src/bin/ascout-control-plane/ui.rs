//! Presentation folding and conversations. Ledger owns all observation events.
use ascout::control_plane::agent_control::{
    ActionKind, AgentMessage, BranchRole, BranchStatus, RuntimeExecutionState,
};
pub use ascout::control_plane::gateway::ui::Request;
use ascout::control_plane::ledger::{Ledger, LedgerReader};
use ascout::control_plane::observation::{ObservationContent, StateEventSet};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex};

#[derive(Clone, Serialize)]
pub struct Conversation {
    pub inherited: Vec<Value>,
    pub messages: Vec<Value>,
    pub streaming: String,
    pub role: BranchRole,
    pub status: BranchStatus,
}

impl Default for Conversation {
    fn default() -> Self {
        Self {
            inherited: Vec::new(),
            messages: Vec::new(),
            streaming: String::new(),
            role: BranchRole::Authority,
            status: BranchStatus::Created,
        }
    }
}

#[derive(Clone)]
pub struct ObservedBranch {
    pub branch_id: String,
    pub parent_branch_id: Option<String>,
    pub role: BranchRole,
    pub status: BranchStatus,
    pub(super) runtime: Option<RuntimeExecutionState>,
    pub(super) reason: Option<String>,
}

#[derive(Clone, Default)]
pub struct ObservedTree {
    pub authority_branch_id: Option<String>,
    pub branches: BTreeMap<String, ObservedBranch>,
}

pub struct AScoutRunView {
    pub tree: ObservedTree,
    pub events: Vec<Value>,
    pub ready: bool,
    pub running: bool,
    pub notice: String,
    pub connected: bool,
}

pub fn run_view(set: StateEventSet) -> AScoutRunView {
    let mut view = AScoutRunView {
        tree: Default::default(),
        events: Vec::new(),
        ready: false,
        running: false,
        notice: String::new(),
        connected: true,
    };
    for event in set.events {
        view.events.push(serde_json::to_value(&event).unwrap());
        match event.content {
            ObservationContent::BranchCreated {
                branch_id,
                parent_branch_id,
                role,
                ..
            } => {
                view.tree.branches.insert(
                    branch_id.clone(),
                    ObservedBranch {
                        branch_id,
                        parent_branch_id,
                        role,
                        status: BranchStatus::Created,
                        runtime: None,
                        reason: None,
                    },
                );
            }
            ObservationContent::AuthorityChanged {
                branch_id,
                previous_authority,
            } => {
                if let Some(old) = previous_authority.and_then(|id| view.tree.branches.get_mut(&id))
                {
                    old.role = BranchRole::Candidate;
                }
                if let Some(branch) = view.tree.branches.get_mut(&branch_id) {
                    branch.role = BranchRole::Authority;
                }
                view.tree.authority_branch_id = Some(branch_id);
            }
            ObservationContent::StatusChanged {
                branch_id,
                status,
                reason,
            } => {
                if let Some(branch) = view.tree.branches.get_mut(&branch_id) {
                    branch.status = status;
                    branch.reason = reason;
                }
            }
            ObservationContent::EnvironmentChanged {
                branch_id, state, ..
            } => {
                if let Some(branch) = view.tree.branches.get_mut(&branch_id) {
                    branch.runtime = Some(state);
                }
            }
            ObservationContent::RunStatus {
                ready,
                running,
                notice,
                connected,
            } => {
                view.ready = ready;
                view.running = running;
                view.notice = notice;
                view.connected = connected;
            }
            _ => {}
        }
    }
    view
}

#[derive(Clone)]
pub struct Interface {
    input_status: Arc<Mutex<Value>>,
    task_id: String,
    conversations: Arc<Mutex<BTreeMap<String, Conversation>>>,
    /// 每个工具 action 的开始时刻与工具名；工具名来自同一 action 的
    /// `ActionStarted.name`，没有 ActionStarted 时两者都不存在。
    action_starts: Arc<Mutex<BTreeMap<(String, String), (u64, String)>>>,
    reader: LedgerReader,
}

impl Interface {
    pub fn new(task_id: String, reader: LedgerReader) -> Self {
        Self {
            input_status: Arc::new(Mutex::new(
                serde_json::json!({"sequence":0,"ready":false,"error":null}),
            )),
            task_id,
            conversations: Default::default(),
            action_starts: Default::default(),
            reader,
        }
    }
    pub fn conversation(&self, branch: &str) -> Option<Conversation> {
        let view = run_view(self.reader.state_event_set());
        let branch = view.tree.branches.get(branch)?;
        let mut conversation = self
            .conversations
            .lock()
            .unwrap()
            .get(&branch.branch_id)
            .cloned()
            .unwrap_or_default();
        conversation.role = branch.role;
        conversation.status = branch.status;
        Some(conversation)
    }
    pub fn input_status(&self) -> Value {
        self.input_status.lock().unwrap().clone()
    }
    pub fn publish_input_status(&self, sequence: u64, ready: bool, error: &Option<String>) {
        *self.input_status.lock().unwrap() =
            serde_json::json!({"sequence":sequence,"ready":ready,"error":error});
    }
    pub fn publish(
        &self,
        ledger: &mut Ledger,
        ready: bool,
        running: bool,
        notice: &str,
    ) -> io::Result<()> {
        let view = run_view(ledger.state_event_set());
        if (ready, running, notice) != (view.ready, view.running, view.notice.as_str()) {
            ledger.append_observation(
                &self.task_id,
                ObservationContent::RunStatus {
                    ready,
                    running,
                    notice: notice.into(),
                    connected: view.connected,
                },
            )?;
        }
        Ok(())
    }
    pub fn disconnect(&self, ledger: &mut Ledger) -> io::Result<()> {
        ledger.append_observation(
            &self.task_id,
            ObservationContent::RunStatus {
                ready: false,
                running: false,
                notice: "Control plane disconnected".into(),
                connected: false,
            },
        )
    }
    pub fn fork(&self, ledger: &mut Ledger, parent: &str, child: &str) -> io::Result<()> {
        let mut conversations = self.conversations.lock().unwrap();
        let parent = conversations.get(parent).cloned().unwrap_or_default();
        let mut inherited = parent.inherited;
        inherited.extend(parent.messages);
        conversations.insert(
            child.into(),
            Conversation {
                inherited,
                ..Default::default()
            },
        );
        drop(conversations);
        ledger.append_observation(
            &self.task_id,
            ObservationContent::ConversationUpdated {
                branch_id: child.into(),
            },
        )
    }
    pub fn message(&self, ledger: &mut Ledger, message: &AgentMessage) -> io::Result<()> {
        let content = match message {
            AgentMessage::Output {
                branch_id, text, ..
            } => {
                self.conversations
                    .lock()
                    .unwrap()
                    .entry(branch_id.clone())
                    .or_default()
                    .streaming
                    .push_str(text);
                Some(ObservationContent::ConversationUpdated {
                    branch_id: branch_id.clone(),
                })
            }
            AgentMessage::Transcript {
                branch_id, entry, ..
            } => {
                let mut conversations = self.conversations.lock().unwrap();
                let conversation = conversations.entry(branch_id.clone()).or_default();
                if entry["role"] == "assistant" {
                    conversation.streaming.clear();
                }
                conversation.messages.push(entry.clone());
                drop(conversations);
                Some(ObservationContent::ConversationUpdated {
                    branch_id: branch_id.clone(),
                })
            }
            AgentMessage::ActionStarted {
                branch_id,
                action_id,
                name,
                timestamp_ms,
                ..
            } => {
                self.action_starts.lock().unwrap().insert(
                    (branch_id.clone(), action_id.clone()),
                    (*timestamp_ms, name.clone()),
                );
                None
            }
            AgentMessage::Usage {
                branch_id,
                action_id,
                model,
                usage,
                started_at_ms,
                finished_at_ms,
                ..
            } => Some(ObservationContent::ModelUsage {
                branch_id: branch_id.clone(),
                action_id: action_id.clone(),
                model: model.clone(),
                input_tokens: usage["input"].as_u64(),
                output_tokens: usage["output"].as_u64(),
                cache_read_tokens: usage["cacheRead"].as_u64(),
                cache_write_tokens: usage["cacheWrite"].as_u64(),
                total_tokens: usage["totalTokens"].as_u64(),
                cost: usage["cost"]["total"].as_f64(),
                started_at_ms: Some(*started_at_ms),
                finished_at_ms: Some(*finished_at_ms),
                duration_ms: finished_at_ms.checked_sub(*started_at_ms),
            }),
            AgentMessage::ActionFinished {
                branch_id,
                action_id,
                action_kind: ActionKind::ToolExecution,
                status,
                timestamp_ms,
                ..
            } => {
                let start = self
                    .action_starts
                    .lock()
                    .unwrap()
                    .remove(&(branch_id.clone(), action_id.clone()));
                let started_at_ms = start.as_ref().map(|(timestamp, _)| *timestamp);
                let tool_name = start.map(|(_, name)| name);
                Some(ObservationContent::ToolFinished {
                    branch_id: branch_id.clone(),
                    action_id: action_id.clone(),
                    tool_name,
                    status: *status,
                    started_at_ms,
                    finished_at_ms: Some(*timestamp_ms),
                    duration_ms: started_at_ms.and_then(|start| timestamp_ms.checked_sub(start)),
                })
            }
            AgentMessage::ActionFinished {
                branch_id,
                action_id,
                ..
            } => {
                self.action_starts
                    .lock()
                    .unwrap()
                    .remove(&(branch_id.clone(), action_id.clone()));
                None
            }
            _ => None,
        };
        if let Some(content) = content {
            ledger.append_observation(&self.task_id, content)?;
        }
        Ok(())
    }
}

impl ascout::control_plane::gateway::ui::Presentation for Interface {
    fn input_status(&self) -> Value {
        self.input_status()
    }

    fn conversation(&self, branch: &str) -> Option<Value> {
        self.conversation(branch)
            .map(|value| serde_json::to_value(value).unwrap())
    }
}
