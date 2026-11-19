# 第六轮结构重构记录

本轮只搬代码、改模块路径和必要的可见性；未改函数签名语义、逻辑、常量，也未修 bug。

## 已知问题（只记录，未修改）

1. **`BaseRuntime` 依赖已解决（第七轮）**
   `BaseRuntime` 为每个 Environment 记录实际参与的普通 Backend 实例。
2. **个别文件略超 600 行**
   - 当轮的 Tree 模块约 628 行，主要是类型定义和模块声明。当前状态类型在
     `src/control_plane/speculative_tree/mod.rs`，Agent 协议已移入 Gateway。
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
9. **Tree 动作实现**
   `src/control_plane/speculative_tree/actions.rs` 实现 `SpeculativeTree` 的操作登记、
   反馈处理、过期检查和 `progress`。`SpeculationCoordinator` 使用同一组操作与反馈类型，
   负责调用 Policy 和 Runtime。
10. **`EnvironmentNetwork` / Firecracker 辅助函数**
    仍按原函数边界移动，没有改变命令参数、常量或网络语义。

## 本轮未执行的操作

- 未提交 git。
- 未运行 `cargo public-api`（环境未安装），改用 `grep -h '^pub '` 对比
  公开项名称；只出现模块声明和 `use` 行数量差异。
- 未修改 `docs/` 中设计文档的设计内容，只更新旧文件路径引用和新增模块结构章节。

## 第七轮补充

- 任务 A 已消除 `BaseRuntime` 对具体 `FilesystemBackend` 的编译期依赖；Backend
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

## 第七轮新发现的处理状态

- **accept 不是 O(增量)**：第八轮已修复主路径：冲突检测按 delta 路径探测，
  不再读取完整 target upper；rename/metacopy 才构建 source 视图。
- **复制路径逐文件 `sync_all`**：第八轮已移除；由 staging 目录原子 rename +
  目录 sync 表示副本完成。
- **后台删除竞态**：第八轮对 NotFound 统一视为成功，并修正 background 删除
  路径；最终 benchmark 清理未再出现 NotFound 告警。
- **flatten 成本**：仍存在；deployment 文档继续建议独立状态挂载点。
- **accept 回滚仍非原子**：仍存在；journal 可逆序恢复，但读者可能看到中间态。

## 第八轮补充

- 新增 btrfs 快照副本模式：每个挂载 upper 位于一个 btrfs 子卷的 `upper`
  子目录，fork/mark 用只读/可写快照。启用探测包含创建、只读快照、overlay
  lower 挂载和 btrfs-aware 递归删除；`fast_delete` 不再是启用门槛。本机
  `fast_delete=false` 时仍启用，后台删除走递归 fallback。最终基准显示
  btrfs-snapshot fork 比 reflink 快，但仍随 N 增长（100k 约 1.2–1.4s），
  不是严格 O(1)。
- btrfs 快照模式仍保留 staging + 原子 rename；失败/崩溃的 `.tmp-*` 在下次
  启动清理。
- `clone_upper` 的并行克隆评估：未实现。当前 fork 的主要剩余成本是目录项遍历；
  reflink/快照已覆盖数据拷贝，目视没有低风险且可观的收益，故不在本轮引入
  并行目录/xattr 写入顺序风险。
- 新增 `examples/fs_bench.rs` btrfs-snapshot 两个 case；本机因
  快照副本探测失败时输出 SKIP，`fast_delete=false` 本身不再阻止运行。
  accept lower 链最终基准约 5–6ms，不再随
  N 线性增长。

## 第八轮 FS 错误修复

- `filesystem_operation` 在 operation 记录持久化失败时返回
  “applied but operation record was not persisted”错误，并清理内存中的非持久化记录。
- suspend/resume/terminate 改为持久化候选状态后再替换内存状态。
- `remember`、refresh 后的 snapshot 持久化、fork/mark 的 metadata 写盘都移到
  `state` 锁外；snapshot 持久化由 `persist_all_snapshots` 统一完成。
- launch_fragment 的 workspace root / 新 mount 持久化失败改为返回错误。
- opaque 目录删除失败不再被忽略，会触发 replay 错误与回滚。
- SafeRoot::mkdir 使用源节点 mode，而不是固定 0o700。
- btrfs send/receive 并发 drain send stderr；receive spawn 失败会 kill+wait send。
- 损坏的 `operations.json` 改名为 `.corrupt-<pid>` 并告警。
- 后台删除/递归删除把 NotFound 视为成功；最终 benchmark 未再出现该告警。
- mark 的两个持久化错误不再压成 bool，原始错误进入返回消息。

## 第九轮代码异味处理

已处理：

- `LaunchFragment/MountEntry/MountSpec` 移到中立模块 `runtime/launch_plan.rs`；
  `runtime/backend.rs` 与 filesystem 不再依赖 `runtime::process` 的挂载计划类型。
  （第十轮已删除 `process` 的 re-export 垫片，所有引用改到 `runtime::launch_plan`。）
- 删除恒为 true 的 `BTRFS_INCREMENTAL_DIFF_IMPLEMENTED` 及对应注释/不可达分支。
- 环境进程实现的三个构造器提取共享 `empty` 初始化；仅测试使用的
  `with_mechanism/new_process_group/capabilities` 加 `#[cfg(test)]`，删除无用的
  `id` 字段/方法。
- `BaseRuntime` 的 operate/tool_context/invoke_tool 三个单输出包装提取
  `single_output` helper。
- filesystem probe/btrfs/mounts 的 base mount 条目复用
  `launch_plan::base_mount_entries()`。
- process cgroup/launch 与 adapter dispatch 的等待超时提取为命名常量。

未处理（如实记录）：

- `handle_message`、`complete_work`、`execute`、filesystem `fork` 的按职责拆分：
  第九轮未做；第十轮已完成，见下一节。
- firecracker 两个 `start_*` 函数的重复 prologue/rollback：第九轮未做；
  第十轮已提取共享 `prepare_environment_start` / `rollback_environment_start`。
- `ops/*.rs` 的 fork/mark/diff/accept 前置检查提取：各操作的合法性条件不同，
  只在 accept 内保留现有语义，未抽出可能改变错误顺序的共享函数。
- `FilesystemBackend::mechanism/fast_delete/fork_cost` 仍为 pub：examples 和测试
  直接使用，收窄会改变公开 API。
- `docs/development/decisions.md` 中“filesystem 不执行 generic operate”与代码
  不一致。该文档由并行文档会话维护，本轮只记录：代码中 filesystem 保留
  `Backend::operate` 供直接 Backend 调用；BaseRuntime 组合时仍只把 generic
  operate 委托给 inner。需要文档会话或后续轮次二选一修正。
- `ops/*.rs` 的 `pub fn` 是现有公开数据面 API，未收窄。
- `FreezeMechanism::None` 保留：不是仅测试类型，BaseRuntime 的生产
  suspend/resume/filesystem_operation 都会根据它选择跳过 freeze/thaw。

## 第九轮补充发现

- btrfs 快照副本已在本机启用（`fast_delete=false` 不再阻塞），但 100k 文件时
  fork 实测约 1.2–1.4s，仍随 N 增长，不是严格 O(1)。裸
  `btrfs subvolume snapshot -r` 对 100k 文件测到首次约 0.22s、后续接近 0，
  说明剩余成本在完整 fork 的 per-mount snapshot/prepare 流程；本轮未继续优化。
- `target/tmp` 中仍有少量空的只读 btrfs 子卷根，来自手工探测；普通用户
  `btrfs subvolume delete` EPERM。此为当轮诊断，不是所有后续残留的结论；
  后续必须重新检查挂载、权限、子卷和后台删除状态，不能据此排除测试残留。
- 长函数拆分、firecracker 两个 start 的重复代码、ops 前置检查提取、firecracker
  超时常量未处理，已在上一节列出原因。

## 第十轮代码异味处理

已完成：

- `handle_message`：保留消息 `match`，各分支体提取为 `handle_branch_*`、
  `handle_action_*`、`handle_command_result`、`apply_command_result` 等具名方法；
  `apply_command_result` 仍直接展示 Promote/Discard/Cleanup 的状态分支。
- `complete_work`：保留操作 `match`，提取 `complete_predict`、`complete_establish`、
  `complete_invoke`、`complete_validate`、`complete_promote`、
  `complete_terminate_or_cleanup`。
- `execute`：保留 Operation `match`，提取 `execute_establish/operate/branch/...`；
  `BackendContext` 构造移到 `operation_context`。
- filesystem `fork`：保留 mechanism `match` 与每挂载 chain；提取 replay 校验、
  plan、btrfs/overlay 创建、overlay 单挂载 chain、commit/persist。
- `launch_fragment`：拆为加载环境、发现/准备 mount、构建 fragment、追加未管理
  只读 mount 与 state mask。
- firecracker 两个 `start_*` 共享 `prepare_environment_start`、回滚
  `rollback_environment_start`、收尾 `finish_environment_start`。
- 删除 `runtime/process/mod.rs` 的 LaunchFragment/MountEntry/MountSpec re-export
  垫片；`runtime/tests.rs` 与 process 测试改从 `runtime::launch_plan` 引用。

仍未处理（如实记录）：

- `ops/*.rs` 的 fork/mark/diff/accept 前置检查未抽成一个共享函数：各操作的合法
  条件与错误顺序不同，抽出容易改变错误文本/顺序。空 upper 的缺失层语义单独在
  diff/accept 中按空视图处理。
- 后台回收仍按文件数线性：本机普通用户不能 `btrfs subvolume delete`，只能
  `property set ro false` 后递归删除。已用进程级互斥把多个后台删除串行化，
  降低与前台 fork 的 btrfs 事务争用；未改变最终会删净的语义。
- `MountUnit::volume_is_subvolume` 为新增内部元数据（serde 默认 true）。空挂载
  的子卷父目录现在是普通目录，后续 fork 对该挂载只能复制、不能快照；旧状态
  仍按 true 读取。
- `FilesystemBackend::mechanism/fast_delete/fork_cost`、`ops/*.rs` 的 `pub fn`
  仍是公开或 crate 可见的数据面 API，未收窄。
- `docs/development/decisions.md` 与 `filesystem::operate` 的表述不一致仍存在；
  该文档不在本轮可改范围。
