//! YAML compilation and isolated Python Policy evaluation.
//!
//! This module returns decisions; it never applies them to Branch or Runtime state.

mod compile;
mod decision;
mod engine;
mod provider;
mod validation;
mod worker;
pub use provider::ModelProviderProcess;

pub use compile::CompiledPolicy;
pub use decision::Decision;
pub use engine::PolicyEngine;

const MAX_BYTES: usize = 1024 * 1024;
const MAX_FILES: usize = 128;
const RUNNER: &str = include_str!("runner.py");
