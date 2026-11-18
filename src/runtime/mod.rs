//! 通用 Runtime 组合层。
//!
//! Runtime Adapter 只运行一个进程，但这个进程可以持有多个 Backend
//! 连接。`CompositeRuntime` 负责根据显式 Policy 选择 Backend，并按固定
//! 顺序执行生命周期操作；`BaseRuntime` 是一个特殊的 Backend，它在另
//! 一个 Backend 之上管理一个 Agent 实际执行实例的进程句柄。
//!
//! 这里不认识 NLE、Firecracker 或任何具体 workload。具体 Backend 由
//! `ascout-runtime-adapter` binary 装配进来。Runtime 指一个 Adapter 实例
//! 及其组合的 Backend 的整体，不是一个独立的类型或标记 trait；`CompositeRuntime`
//! 与 `BaseRuntime` 都实现 `Backend`，Adapter 一视同仁地通过 `Arc<dyn Backend>`
//! 调用它们。

pub mod adapter;
pub mod backend;
pub mod backends;
pub mod base;
pub mod capabilities;
pub mod composite;
pub mod environment;
pub mod execution_wire;
mod identity;
pub mod launch_plan;
pub mod process;
mod recovery;
pub mod selection;

#[cfg(test)]
mod tests;

pub use base::BaseRuntime;
pub use composite::CompositeRuntime;
pub use selection::{
    BackendConnection, BackendOperation, BackendPolicy, BackendRoute, BackendSelection,
};
