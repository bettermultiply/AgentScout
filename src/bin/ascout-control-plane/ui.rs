//! Presentation-neutral facts and requests. No layout or lifecycle decisions.
use ascout::agent_control::{AgentMessage, SpeculativeTree};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::{mpsc, Arc, Mutex};

#[derive(Clone, Default, Serialize)]
pub struct Conversation {
    pub inherited: Vec<Value>,
    pub messages: Vec<Value>,
    pub streaming: String,
}

#[derive(Clone, Serialize)]
/// The read-only facts that AScout publishes for one run's UI.
pub struct AScoutRunView {
    pub tree: SpeculativeTree,
    pub conversations: BTreeMap<String, Conversation>,
    pub facts: BTreeMap<String, String>,
    pub events: Vec<Value>,
    pub ready: bool,
    pub running: bool,
    pub notice: String,
    pub connected: bool,
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Prompt { prompt: String },
    Interrupt,
    Shutdown,
}

#[derive(Clone)]
pub struct Interface {
    run_view: Arc<Mutex<AScoutRunView>>,
    requests: mpsc::Sender<Request>,
}

impl Interface {
    pub fn new(tree: SpeculativeTree) -> (Self, mpsc::Receiver<Request>) {
        let (requests, receiver) = mpsc::channel();
        (
            Self {
                run_view: Arc::new(Mutex::new(AScoutRunView {
                    tree,
                    conversations: BTreeMap::new(),
                    facts: BTreeMap::new(),
                    events: Vec::new(),
                    ready: false,
                    running: false,
                    notice: "Starting Agent session…".into(),
                    connected: true,
                })),
                requests,
            },
            receiver,
        )
    }

    pub fn run_view(&self) -> AScoutRunView {
        self.run_view.lock().unwrap().clone()
    }
    pub fn request(&self, request: Request) -> Result<(), String> {
        if !self.run_view.lock().unwrap().connected {
            return Err("Control plane disconnected".into());
        }
        self.requests
            .send(request)
            .map_err(|_| "Control plane disconnected".into())
    }
    pub fn publish(
        &self,
        tree: &SpeculativeTree,
        facts: &BTreeMap<String, String>,
        ready: bool,
        running: bool,
        notice: &str,
    ) {
        let mut state = self.run_view.lock().unwrap();
        state.tree = tree.clone();
        state.facts = facts.clone();
        state.ready = ready;
        state.running = running;
        state.notice = notice.into();
    }
    pub fn disconnect(&self) {
        self.run_view.lock().unwrap().connected = false;
    }
    pub fn record(&self, event: Value) {
        self.run_view.lock().unwrap().events.push(event);
    }
    pub fn fork(&self, parent: &str, child: &str) {
        let mut state = self.run_view.lock().unwrap();
        let parent = state.conversations.get(parent).cloned().unwrap_or_default();
        let mut inherited = parent.inherited;
        inherited.extend(parent.messages);
        state.conversations.insert(
            child.into(),
            Conversation {
                inherited,
                ..Default::default()
            },
        );
    }
    pub fn message(&self, message: &AgentMessage) {
        let mut state = self.run_view.lock().unwrap();
        match message {
            AgentMessage::Output { branch_id, text } => {
                state
                    .conversations
                    .entry(branch_id.clone())
                    .or_default()
                    .streaming
                    .push_str(text);
            }
            AgentMessage::Transcript {
                branch_id, entry, ..
            } => {
                let conversation = state.conversations.entry(branch_id.clone()).or_default();
                if entry["role"] == "assistant" {
                    conversation.streaming.clear();
                }
                conversation.messages.push(entry.clone());
            }
            _ => {}
        }
    }
}
