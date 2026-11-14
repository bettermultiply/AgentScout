//! Typed UI request delivery. Presentation and observation storage live outside Gateway.
use serde::Deserialize;
use std::sync::Arc;

mod http;
pub use http::start;

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
    Model { model: String },
    Prompt { prompt: String },
    Interrupt,
    Pause,
    Shutdown,
}

#[derive(Clone)]
pub struct Connection {
    reader: crate::control_plane::ledger::LedgerReader,
    deliver: Arc<dyn Fn(Request) -> Result<(), String> + Send + Sync>,
}

impl Connection {
    pub fn new(
        reader: crate::control_plane::ledger::LedgerReader,
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
