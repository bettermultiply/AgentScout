
use std::sync::Arc;

use serde::Deserialize;

use super::backend::{Backend, BackendError};
use crate::protocol::contract::RuntimeCapabilities;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendInterface {
    Environment,
    Filesystem,
}

/// An implementation declaration and an instance constructor. Resolving a
/// declaration must not start or capture a running backend instance.
pub struct BackendImplementation {
    pub interfaces: Vec<BackendInterface>,
    pub capabilities: RuntimeCapabilities,
    pub create: Box<dyn FnOnce() -> Result<Arc<dyn Backend>, BackendError>>,
}

pub(super) struct BackendRegistration {
    pub id: String,
    pub implementation: BackendImplementation,
}

#[derive(Default)]
pub struct BackendRegistry {
    pub(super) entries: Vec<BackendRegistration>,
}

impl BackendRegistry {
    pub fn register(
        &mut self,
        id: &str,
        implementation: BackendImplementation,
        required: &[BackendInterface],
    ) -> Result<(), BackendError> {
        if id.trim().is_empty() || self.entries.iter().any(|entry| entry.id == id) {
            return Err(BackendError::new(format!(
                "empty or duplicate Runtime Backend id {id:?}"
            )));
        }
        for interface in required {
            Self::require_interface(id, &implementation, *interface)?;
        }
        self.entries.push(BackendRegistration {
            id: id.to_owned(),
            implementation,
        });
        Ok(())
    }

    pub(super) fn require_interface(
        id: &str,
        implementation: &BackendImplementation,
        interface: BackendInterface,
    ) -> Result<(), BackendError> {
        if implementation.interfaces.contains(&interface) {
            Ok(())
        } else {
            Err(BackendError::new(format!(
                "Runtime Backend {id} is missing the {interface:?} interface"
            )))
        }
    }
}
