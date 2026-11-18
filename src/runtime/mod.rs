//! Runtime ownership and coordination. The proxy transports control-plane requests,
//! while the agent supervisor owns the trusted Agent host.

mod agent_supervisor;
pub mod backend;
pub mod backend_registry;
pub mod backends;
pub mod base;
pub mod capabilities;
pub mod environment;
mod executable_backend;
pub mod execution_wire;
mod identity;
pub mod launch_plan;
mod lifecycle;
pub(crate) mod process;
pub mod proxy;
mod recovery;
pub mod selection;

#[cfg(test)]
mod tests;

pub use base::BaseRuntime;
pub use selection::{
    BackendConnection, BackendOperation, BackendPolicy, BackendRoute, BackendSelection,
};
