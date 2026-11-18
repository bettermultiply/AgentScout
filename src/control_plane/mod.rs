//! control plane：投机树、分支协调、会话请求、策略与账本。

pub mod agent_control;
pub mod coordination;
pub mod gateway;
pub mod ledger;
pub mod observation;
pub mod policy;
pub mod recovery;

pub mod system_log;
pub mod ui_projection;
