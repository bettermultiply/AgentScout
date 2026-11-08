# 第六轮结构重构记录

本轮只搬代码、改模块路径和必要的可见性；未改函数签名语义、逻辑、常量，也未修 bug。

## 已知问题（只记录，未修改）

1. **`BaseRuntime` 依赖已解决（第七轮）**
   `BaseRuntime` 现在持有 `Option<Arc<dyn Backend>>` 作为 data plane，不再引用
   具体 `FilesystemBackend`。`with_data_plane` 接受任意 Backend trait 对象。
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

## 第七轮补充

- 任务 A 已消除 `BaseRuntime` 对具体 `FilesystemBackend` 的编译期依赖；Backend
  trait 现有接口已足够，未新增 data-plane trait 方法。
- 任务 B 清理了测试中的本机用户名/绝对路径；依赖特定挂载的测试现在先检查
  `overlayable_mounts()`，不满足时打印 `SKIP <原因>`。
- 任务 C 增加 `state_directory_overlaps_mount` 能力字段和启动日志；重叠挂载仍使用
  flatten，非重叠挂载的 lower 链路径有测试覆盖。
- 任务 D 的 accept replay journal 会备份目标 upper 的旧内容，失败时逆序恢复；
  仍非原子，中间态可被并发读者观察。
- 任务 E 基准发现：flatten 挂载的 fork/diff/mark/accept 随 N 线性增长；
  非重叠 lower 链的 diff 保持 O(增量)，但当前 accept 仍会读取完整目标 upper，
  因此 accept 随 N 线性增长（瓶颈位置：`merge_delta_layers` 前读取
  `target_mount.upper` 整层，以及 replay source maps 构建）。

## 第七轮新发现（未修）

- **accept 不是 O(增量)**：`src/runtime/backends/filesystem/ops/accept.rs` 在冲突
  检测前执行 `read_layer(&target_mount.upper)`，并构建完整 source maps；
  benchmark 显示 100k 文件时 accept ≈2.25s（lower 链）且随 N 增长。
- **复制路径逐文件 `sync_all`**：`src/runtime/backends/filesystem/copy.rs`
  `copy_file` 对每个文件调用 `sync_all`，在 accept 回滚备份/恢复的大目录上会
  放大成本。
- **后台删除竞态**：benchmark 清理阶段出现
  `background removal ... failed: No such file or directory`；`discard` 的 trash
  路径与被 benchmark 清理的目录发生竞态，错误只打印，不影响退出码。
- **flatten 成本**：状态目录与 workspace 同挂载时 fork/mark/diff/accept 都是
  O(upper)，deployment 文档已建议把状态目录放到独立挂载点。
- **accept 回滚仍非原子**：journal 支持逆序恢复全部已应用操作，但并发读者可能
  在重放窗口读到中间态；文档已明确。

