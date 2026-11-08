# 第六轮结构重构记录

本轮只搬代码、改模块路径和必要的可见性；未改函数签名语义、逻辑、常量，也未修 bug。

## 已知问题（只记录，未修改）

1. **`BaseRuntime` 直接依赖具体文件系统 Backend**
   `src/runtime/base.rs` 引用 `crate::runtime::backends::filesystem::FilesystemBackend`。
   这与“通用组合层不依赖具体 Backend”的目标方向相反。当前按原行为保留；
   后续可通过注入 `Arc<dyn Backend>` 或能力接口消除。
2. **个别文件略超 600 行**
   - `src/control_plane/agent_control/mod.rs` 约 628 行：主要是类型定义和模块声明，
     按职责继续拆会把紧密相关的状态类型拆散，暂保留。
   - `src/runtime/composite.rs` 约 647 行：`CompositeRuntime` 的 Backend 实现较长，
     但每个方法都是同一职责；未按行数硬切。
   - `src/runtime/backends/filesystem/tests/ops.rs` 约 561 行，仍在限制内。
3. **内部类型可见性为 `pub(crate)`**
   `filesystem::state` 的 `Environment`、`MountUnit`、`BackendState` 等被
   `filesystem` 各子模块和测试共享。为避免 `private_interfaces` 警告，当前给
   这些内部类型 `pub(crate)`，并未对外暴露。后续可用更细的 `pub(in ...)`。
4. **测试模块路径变化**
   测试函数名全部保留，模块前缀按新目录变化，例如
   `filesystem::tests::...` → `runtime::backends::filesystem::tests::...`。
   这符合本轮“测试跟随被测代码”的要求，但会影响按旧测试路径过滤的命令。
5. **`policy` 内部子模块当前为私有**
   `compile`、`decision`、`engine`、`validation` 不再作为公开模块路径；
   公开类型仍从 `control_plane::policy::{Decision, CompiledPolicy, PolicyEngine}`
   导出。
6. **`process`、`adapter`、`filesystem` 的内部子模块同样为私有**
   公开 API 仅保留原有类型的顶层路径；新路径只用于实现拆分。
7. **旧的模块转发已删除**
   没有保留 `pub use` 兼容垫片；所有 `src/bin/*` 与内部引用已直接改到新路径。
8. **`BackendConnection` 等选择类型**
   原先在 `runtime.rs` 中，现拆到 `runtime/selection.rs`，并通过
   `runtime/mod.rs` 原路径重新导出公开类型；未改变类型定义。
9. **`coordination` 的 Tree 动作实现**
   仍在 `impl SpeculativeTree` 上，只是移到 `coordination/actions.rs`；
   没有拆成 trait 或自由函数，避免改变调用方式。
10. **`EnvironmentNetwork` / Firecracker 辅助函数**
    仍按原函数边界移动，没有改变命令参数、常量或网络语义。

## 本轮未执行的操作

- 未提交 git。
- 未运行 `cargo public-api`（环境未安装），改用 `grep -h '^pub '` 对比
  公开项名称；只出现模块声明和 `use` 行数量差异。
- 未修改 `docs/` 中设计文档的设计内容，只更新旧文件路径引用和新增模块结构章节。
