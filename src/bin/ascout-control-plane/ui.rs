//! Presentation folding and conversations. Ledger owns all observation events.
use ascout::control_plane::coordination::execution::{PresentationUpdate, PresentationUpdates};
use ascout::control_plane::gateway::agent::protocol::BranchRole;
pub use ascout::control_plane::gateway::ui::Request;
use ascout::control_plane::observation::{ObservationContent, StateEventSet};
use ascout::control_plane::speculative_tree::{BranchStatus, RuntimeExecutionState};
use ascout::control_plane::ui_projection::UiProjection;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
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
    pub prompts: BTreeMap<
        String,
        (
            String,
            ascout::control_plane::gateway::agent::protocol::UiPrompt,
        ),
    >,
    pub commands: Vec<Value>,
    pub tree: ObservedTree,
    pub events: Vec<Value>,
    pub ready: bool,
    pub running: bool,
    pub notice: String,
    pub connected: bool,
}

pub fn run_view(set: StateEventSet) -> AScoutRunView {
    let mut view = AScoutRunView {
        prompts: BTreeMap::new(),
        commands: Vec::new(),
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
            ObservationContent::UiPromptRequested {
                branch_id,
                prompt_id,
                prompt,
            } => {
                view.prompts.insert(prompt_id, (branch_id, prompt));
            }
            ObservationContent::UiPromptResolved { prompt_id, .. } => {
                view.prompts.remove(&prompt_id);
            }
            ObservationContent::CommandsListed { commands, .. } => {
                view.commands = commands.as_array().cloned().unwrap_or_default();
            }
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
                view.prompts.retain(|_, (owner, _)| {
                    Some(&*owner) == view.tree.authority_branch_id.as_ref()
                });
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
                if !connected {
                    view.prompts.clear();
                }
            }
            _ => {}
        }
    }
    view
}

#[derive(Clone)]
pub struct Interface {
    input_status: Arc<Mutex<Value>>,
    conversations: Arc<Mutex<BTreeMap<String, Conversation>>>,
    reader: UiProjection,
}

impl Interface {
    pub fn new(reader: UiProjection) -> Self {
        Self {
            input_status: Arc::new(Mutex::new(
                serde_json::json!({"sequence":0,"ready":false,"error":null}),
            )),
            conversations: Default::default(),
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
}

impl PresentationUpdates for Interface {
    fn update(&self, update: PresentationUpdate<'_>) {
        match update {
            PresentationUpdate::InputStatus {
                sequence,
                ready,
                error,
            } => {
                *self.input_status.lock().unwrap() =
                    serde_json::json!({"sequence":sequence,"ready":ready,"error":error});
            }
            PresentationUpdate::Fork { parent, child } => {
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
            }
            PresentationUpdate::Output { branch_id, text } => {
                self.conversations
                    .lock()
                    .unwrap()
                    .entry(branch_id.into())
                    .or_default()
                    .streaming
                    .push_str(text);
            }
            PresentationUpdate::SessionHistory { branch_id, entries } => {
                self.conversations
                    .lock()
                    .unwrap()
                    .entry(branch_id.into())
                    .or_default()
                    .inherited
                    .extend(entries.iter().cloned());
            }
            PresentationUpdate::Transcript { branch_id, entry } => {
                let mut conversations = self.conversations.lock().unwrap();
                let conversation = conversations.entry(branch_id.into()).or_default();
                if entry["role"] == "assistant" {
                    conversation.streaming.clear();
                }
                conversation.messages.push(entry.clone());
            }
        }
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
