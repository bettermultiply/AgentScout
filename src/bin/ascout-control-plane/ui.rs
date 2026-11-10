//! Read-only observation log; snapshots are complete event prefixes.
use ascout::control_plane::agent_control::{
    ActionKind, ActionStatus, AgentMessage, BranchRole, BranchStatus, RuntimeExecutionState,
    SpeculativeTree,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Clone, Default, Serialize)]
pub struct Conversation {
    pub inherited: Vec<Value>,
    pub messages: Vec<Value>,
    pub streaming: String,
    pub unverified: bool,
}

#[derive(Clone, Serialize)]
pub struct ObservationEvent {
    pub sequence: u64,
    pub timestamp_ms: u64,
    pub task_id: String,
    #[serde(flatten)]
    pub content: ObservationContent,
}

#[derive(Clone, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum ObservationContent {
    BranchCreated {
        branch_id: String,
        parent_branch_id: Option<String>,
        role: BranchRole,
        edge_id: Option<String>,
        speculation_kind: Option<ActionKind>,
    },
    AuthorityChanged {
        branch_id: String,
        previous_authority: Option<String>,
    },
    StatusChanged {
        branch_id: String,
        status: BranchStatus,
        reason: Option<String>,
    },
    EnvironmentChanged {
        branch_id: String,
        state: RuntimeExecutionState,
        independent_instance: Option<bool>,
        parent_environment: Option<String>,
        configuration: BTreeMap<String, Value>,
    },
    EnvironmentCleaned {
        branch_id: String,
        succeeded: bool,
        reason: Option<String>,
    },
    ConversationUpdated {
        branch_id: String,
        unverified: bool,
    },
    ModelUsage {
        branch_id: String,
        action_id: Option<String>,
        model: String,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        cache_read_tokens: Option<u64>,
        cache_write_tokens: Option<u64>,
        total_tokens: Option<u64>,
        cost: Option<f64>,
        duration_ms: Option<u64>,
    },
    ToolFinished {
        branch_id: String,
        action_id: String,
        status: ActionStatus,
    },
    #[allow(dead_code)] // Wire contract only; emitting it requires accounting evidence.
    Settlement {
        branch_id: String,
        hit: bool,
        time_ms: ObservationAmount,
        money: ObservationAmount,
        currency: String,
    },
    RunStatus {
        ready: bool,
        running: bool,
        notice: String,
        connected: bool,
    },
}

#[derive(Clone, Serialize)]
pub struct ObservationAmount {
    pub value: f64,
    pub measured: bool,
}

#[derive(Serialize)]
pub struct ObservationSnapshot {
    pub sequence: u64,
    pub events: Vec<ObservationEvent>,
}

#[derive(Clone)]
pub struct ObservedBranch {
    pub branch_id: String,
    pub parent_branch_id: Option<String>,
    pub role: BranchRole,
    pub status: BranchStatus,
    runtime: Option<RuntimeExecutionState>,
    reason: Option<String>,
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

impl Default for AScoutRunView {
    fn default() -> Self {
        Self {
            tree: Default::default(),
            events: Vec::new(),
            ready: false,
            running: false,
            notice: String::new(),
            connected: true,
        }
    }
}

impl AScoutRunView {
    fn apply(&mut self, event: &ObservationEvent) {
        use ObservationContent::*;
        match &event.content {
            BranchCreated {
                branch_id,
                parent_branch_id,
                role,
                ..
            } => {
                self.tree.branches.insert(
                    branch_id.clone(),
                    ObservedBranch {
                        branch_id: branch_id.clone(),
                        parent_branch_id: parent_branch_id.clone(),
                        role: *role,
                        status: BranchStatus::Created,
                        runtime: None,
                        reason: None,
                    },
                );
            }
            AuthorityChanged {
                branch_id,
                previous_authority,
            } => {
                if let Some(old) = previous_authority
                    .as_ref()
                    .and_then(|id| self.tree.branches.get_mut(id))
                {
                    old.role = BranchRole::Candidate;
                }
                if let Some(branch) = self.tree.branches.get_mut(branch_id) {
                    branch.role = BranchRole::Authority;
                }
                self.tree.authority_branch_id = Some(branch_id.clone());
            }
            StatusChanged {
                branch_id,
                status,
                reason,
            } => {
                if let Some(branch) = self.tree.branches.get_mut(branch_id) {
                    branch.status = *status;
                    branch.reason = reason.clone();
                }
            }
            EnvironmentChanged {
                branch_id, state, ..
            } => {
                if let Some(branch) = self.tree.branches.get_mut(branch_id) {
                    branch.runtime = Some(*state);
                }
            }
            RunStatus {
                ready,
                running,
                notice,
                connected,
            } => {
                self.ready = *ready;
                self.running = *running;
                self.notice = notice.clone();
                self.connected = *connected;
            }
            _ => {}
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Prompt { prompt: String },
    Interrupt,
    Shutdown,
}

struct ObservationLog {
    task_id: String,
    events: Vec<ObservationEvent>,
    conversations: BTreeMap<String, Conversation>,
}

impl ObservationLog {
    fn append(&mut self, content: ObservationContent) {
        self.events.push(ObservationEvent {
            sequence: self.events.len() as u64 + 1,
            timestamp_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            task_id: self.task_id.clone(),
            content,
        });
    }
    fn view(&self) -> AScoutRunView {
        let mut view = AScoutRunView::default();
        for event in &self.events {
            view.apply(event);
        }
        view
    }
}

#[derive(Clone)]
pub struct Interface {
    log: Arc<(Mutex<ObservationLog>, Condvar)>,
    requests: Arc<dyn Fn(Request) -> Result<(), String> + Send + Sync>,
}

impl Interface {
    pub fn new(
        tree: &SpeculativeTree,
        requests: impl Fn(Request) -> Result<(), String> + Send + Sync + 'static,
    ) -> Self {
        let interface = Self {
            log: Arc::new((
                Mutex::new(ObservationLog {
                    task_id: tree.execution_id.clone(),
                    events: Vec::new(),
                    conversations: BTreeMap::new(),
                }),
                Condvar::new(),
            )),
            requests: Arc::new(requests),
        };
        interface.publish(tree, false, false, "Starting Agent session…");
        interface
    }
    pub fn snapshot(&self) -> ObservationSnapshot {
        let log = self.log.0.lock().unwrap();
        ObservationSnapshot {
            sequence: log.events.len() as u64,
            events: log.events.clone(),
        }
    }
    pub fn sequence(&self) -> u64 {
        self.log.0.lock().unwrap().events.len() as u64
    }
    pub fn events_after(&self, sequence: u64) -> Result<Vec<ObservationEvent>, String> {
        let mut log = self.log.0.lock().unwrap();
        if sequence > log.events.len() as u64 {
            return Err("Sequence is ahead of this run; reload snapshot".into());
        }
        if sequence == log.events.len() as u64 && log.view().connected {
            log = self
                .log
                .1
                .wait_timeout(log, Duration::from_secs(15))
                .unwrap()
                .0;
        }
        Ok(log.events[sequence as usize..].to_vec())
    }
    pub fn connected(&self) -> bool {
        self.log.0.lock().unwrap().view().connected
    }
    pub fn run_view(&self) -> AScoutRunView {
        let log = self.log.0.lock().unwrap();
        let mut view = log.view();
        view.events = log
            .events
            .iter()
            .map(|e| serde_json::to_value(e).unwrap())
            .collect();
        view
    }
    pub fn conversation(&self, branch: &str) -> Option<Conversation> {
        let log = self.log.0.lock().unwrap();
        let view = log.view();
        let branch = view.tree.branches.get(branch)?;
        let mut conversation = log
            .conversations
            .get(&branch.branch_id)
            .cloned()
            .unwrap_or_default();
        conversation.unverified = branch.role == BranchRole::Candidate;
        Some(conversation)
    }
    pub fn request(&self, request: Request) -> Result<(), String> {
        if !self.connected() {
            return Err("Control plane disconnected".into());
        }
        (self.requests)(request)
    }
    pub fn publish(&self, tree: &SpeculativeTree, ready: bool, running: bool, notice: &str) {
        let mut log = self.log.0.lock().unwrap();
        let view = log.view();
        let shared = tree
            .capability_view
            .as_ref()
            .and_then(|v| v["shared_environment"].as_bool());
        for branch in tree.branches.values() {
            let old = view.tree.branches.get(&branch.branch_id);
            if old.is_none() {
                let speculation_kind = branch
                    .proposal_id
                    .as_ref()
                    .and_then(|id| tree.proposals.get(id))
                    .and_then(|proposal| {
                        branch
                            .parent_branch_id
                            .as_ref()
                            .and_then(|parent| tree.active_actions.get(parent))
                            .and_then(|a| a.get(&proposal.operation_id))
                    })
                    .map(|action| action.action_kind);
                log.append(ObservationContent::BranchCreated {
                    branch_id: branch.branch_id.clone(),
                    parent_branch_id: branch.parent_branch_id.clone(),
                    role: branch.role,
                    edge_id: None,
                    speculation_kind,
                });
            }
            if old.is_none_or(|old| {
                old.status != branch.status || old.reason != branch.execution_state.failure
            }) {
                log.append(ObservationContent::StatusChanged {
                    branch_id: branch.branch_id.clone(),
                    status: branch.status,
                    reason: branch.execution_state.failure.clone(),
                });
            }
            let runtime = branch.execution_state.runtime;
            if old.is_none_or(|old| old.runtime != Some(runtime)) {
                let independent_instance = match runtime {
                    RuntimeExecutionState::Unknown
                    | RuntimeExecutionState::EnvironmentRequested
                    | RuntimeExecutionState::Failed => None,
                    RuntimeExecutionState::Cleaned => Some(false),
                    _ => shared.map(|shared| !shared),
                };
                let mut configuration = BTreeMap::new();
                if let Some(shared) = shared {
                    configuration.insert("shared_environment".into(), Value::Bool(shared));
                }
                log.append(ObservationContent::EnvironmentChanged {
                    branch_id: branch.branch_id.clone(),
                    state: runtime,
                    independent_instance,
                    parent_environment: None,
                    configuration,
                });
                if runtime == RuntimeExecutionState::Cleaned {
                    log.append(ObservationContent::EnvironmentCleaned {
                        branch_id: branch.branch_id.clone(),
                        succeeded: true,
                        reason: None,
                    });
                }
            }
        }
        if tree.authority_branch_id != view.tree.authority_branch_id
            && let Some(branch_id) = &tree.authority_branch_id
        {
            log.append(ObservationContent::AuthorityChanged {
                branch_id: branch_id.clone(),
                previous_authority: view.tree.authority_branch_id,
            });
        }
        if (ready, running, notice) != (view.ready, view.running, view.notice.as_str()) {
            log.append(ObservationContent::RunStatus {
                ready,
                running,
                notice: notice.into(),
                connected: view.connected,
            });
        }
        self.log.1.notify_all();
    }
    pub fn disconnect(&self) {
        let mut log = self.log.0.lock().unwrap();
        log.append(ObservationContent::RunStatus {
            ready: false,
            running: false,
            notice: "Control plane disconnected".into(),
            connected: false,
        });
        self.log.1.notify_all();
    }
    pub fn fork(&self, parent: &str, child: &str) {
        let mut log = self.log.0.lock().unwrap();
        let parent = log.conversations.get(parent).cloned().unwrap_or_default();
        let mut inherited = parent.inherited;
        inherited.extend(parent.messages);
        log.conversations.insert(
            child.into(),
            Conversation {
                inherited,
                ..Default::default()
            },
        );
        log.append(ObservationContent::ConversationUpdated {
            branch_id: child.into(),
            unverified: true,
        });
        self.log.1.notify_all();
    }
    pub fn message(&self, message: &AgentMessage) {
        let mut log = self.log.0.lock().unwrap();
        match message {
            AgentMessage::Output {
                branch_id, text, ..
            } => {
                log.conversations
                    .entry(branch_id.clone())
                    .or_default()
                    .streaming
                    .push_str(text);
                let unverified = log
                    .view()
                    .tree
                    .branches
                    .get(branch_id)
                    .is_none_or(|b| b.role == BranchRole::Candidate);
                log.append(ObservationContent::ConversationUpdated {
                    branch_id: branch_id.clone(),
                    unverified,
                });
            }
            AgentMessage::Transcript {
                branch_id, entry, ..
            } => {
                let conversation = log.conversations.entry(branch_id.clone()).or_default();
                if entry["role"] == "assistant" {
                    conversation.streaming.clear();
                }
                conversation.messages.push(entry.clone());
                let unverified = log
                    .view()
                    .tree
                    .branches
                    .get(branch_id)
                    .is_none_or(|b| b.role == BranchRole::Candidate);
                log.append(ObservationContent::ConversationUpdated {
                    branch_id: branch_id.clone(),
                    unverified,
                });
            }
            AgentMessage::Usage {
                branch_id,
                action_id,
                model,
                usage,
                ..
            } => {
                log.append(ObservationContent::ModelUsage {
                    branch_id: branch_id.clone(),
                    action_id: action_id.clone(),
                    model: model.clone(),
                    input_tokens: usage["input"].as_u64(),
                    output_tokens: usage["output"].as_u64(),
                    cache_read_tokens: usage["cacheRead"].as_u64(),
                    cache_write_tokens: usage["cacheWrite"].as_u64(),
                    total_tokens: usage["totalTokens"].as_u64(),
                    cost: usage["cost"]["total"].as_f64(),
                    duration_ms: None,
                });
            }
            AgentMessage::ActionFinished {
                branch_id,
                action_id,
                action_kind: ActionKind::ToolExecution,
                status,
                ..
            } => {
                log.append(ObservationContent::ToolFinished {
                    branch_id: branch_id.clone(),
                    action_id: action_id.clone(),
                    status: *status,
                });
            }
            _ => {}
        }
        self.log.1.notify_all();
    }
}
