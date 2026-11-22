//! Typed UI request delivery. Presentation and observation storage live outside Gateway.
use serde::Deserialize;
use std::sync::Arc;

mod http;
pub use http::{Server, start};

/// Read-only presentation supplied by the UI; Gateway does not fold Tree facts.
pub trait Presentation: Send + Sync {
    fn input_status(&self) -> serde_json::Value;
    fn conversation(&self, branch: &str) -> Option<serde_json::Value>;
}

#[derive(Clone, Copy)]
pub struct Assets {
    pub index_html: &'static [u8],
    pub app_js: &'static [u8],
    pub app_css: &'static [u8],
    pub favicon_svg: &'static [u8],
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    UiPromptAnswer {
        prompt_id: String,
        answer: serde_json::Value,
        #[serde(default)]
        cancelled: bool,
    },
    GetCommands {
        request_id: String,
    },
    Model {
        model: String,
        #[serde(default)]
        thinking: Option<String>,
    },
    Prompt {
        prompt: String,
    },
    Interrupt,
    Pause,
    Continue,
    Shutdown,
}

#[derive(Clone)]
pub struct Connection {
    reader: crate::control_plane::ui_projection::UiProjection,
    deliver: Arc<dyn Fn(Request) -> Result<(), String> + Send + Sync>,
}

impl Connection {
    pub fn new(
        reader: crate::control_plane::ui_projection::UiProjection,
        deliver: impl Fn(Request) -> Result<(), String> + Send + Sync + 'static,
    ) -> Self {
        Self {
            reader,
            deliver: Arc::new(deliver),
        }
    }

    pub fn request(&self, request: Request) -> Result<(), String> {
        if !self.reader.connected() {
            return Err("Control plane disconnected".into());
        }
        (self.deliver)(request)
    }
}
