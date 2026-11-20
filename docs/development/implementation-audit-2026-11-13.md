# 2026-11-13 实现核查

本次核查修正 Discussion 对现有实现范围的描述，不改变目标语义。依据当前工作区源码及已有测试；工作区包含尚未提交的改动，结论不代表 HEAD 独立具备全部能力。

| 清单编号 | 当前实现证据 | 剩余边界 |
|---|---|---|
| 8、30 | `src/control_plane/speculative_tree/tests/` 覆盖接受确认后切权、接受失败、晋升中失败、迟到结果、重复启动、精确命令身份及嵌套失效；Pi 测试覆盖多个等待点 shutdown。 | 跨进程竞态、握手阶段信号、真实独立环境故障与退出仍需端到端验收。 |
| 16 | `src/runtime/selection.rs` 和 `src/runtime/base.rs` 已实现显式选择、组合与实例装配。 | 不等于按隔离需求动态查找与管理 Backend。 |
| 20 | `src/runtime/base.rs` 对 FS fork／mark／accept 冻结、操作、解冻，并报告失败；`process/freeze.rs` 有 cgroup 和进程组两种路径。 | 进程组逃逸限制须限定路径；无冻结机制仍可运行操作，不代表已保证所有写者静止。嵌套挂载与真实环境覆盖仍待验证。 |
| 22 | `src/control_plane/gateway/runtime.rs` 与 `src/runtime/proxy/` 有请求身份、超时、请求去重、有界结果缓存、环境队列和 EOF shutdown。 | 不承诺跨重启幂等、状态恢复或完整 Workspace 并发契约。 |
| 23 | `src/control_plane/speculative_tree/mod.rs` 保存 generation、execution_position、chance_id、assumption_set、pending_operations、execution_state；Chance 保存 authority_outcome 与 authority_evidence。 | 后续需审查能力、约束与 Runtime 对齐信息是否满足目标契约。 |
| 28 | `src/control_plane/speculative_tree/actions.rs` 按 epoch 调度 Rebalance，处理 pause／resume／preempt；`instrumentation/core/index.ts` 实现 hold。 | 不能笼统说 rebalance 未实现；效果复用与保留等扩展需分别检查。 |
| 29 | `webui/src/components/ChatPane.tsx` 已有多行 textarea；`MessageView.tsx` 展示文字、思考、工具调用和结果。 | 尚无 Markdown 渲染，图片等仍以未展示内容呈现。 |
| 33 | 手工事件和 loopback 伪模型覆盖局部 full/part、外层接受及拒绝。后代继续和失效由手工树验证。 | 深层并发及恢复期间的组合路径仍需专项验证。 |
| 36 | `src/runtime/process/` 有进程身份、冻结、终止与清理，control plane 已有 generation。 | 完整资源租约、跨重启恢复及执行延续方式仍不能据此认定完成。 |

2026-11-14 补记：公共 `chance_created` 已有源侧投机开始时间，`proposal_validated` 已提供真实验证时刻，`chance_stage_changed` 的 Accepted 已关联所用验证 id；这两项不再是观察协议缺口。树形几何和 NLE 地图已有实现，完整交付与统计验收仍是单独事项。

### Discussion 状态复核（静态核对，未重跑验证）

早先关于 session 保存、NLE 种子、恢复与物化的笼统缺口描述未随实现更新，应以如下较具体的范围为准：

- 第 18、30 项：`src/control_plane/coordination/task.rs` 已等待写者结束，调用 `MaterializeAuthority`，只在成功后记录 `task_completed`，失败保留重试状态；`coordination/recovery.rs` 已分开暂停保存与输入继续。`tests/authority-filesystem.mjs` 和恢复测试已有物化、SIGINT、候选晋级后文件写入等断言。剩余是扩展写者、深层候选、存储机制与竞态组合，不能再概称物化或独立 FS 退出未实现。已有执行证据见 [恢复](recovery.md)、[FS 现状](../fs-backend/current-state.md)。
- 第 27、31 项：`instrumentation/pi/session-tree.ts` 已保存完整原生条目并校验已知引用；`tests/pi-standalone-continuation.mjs` 已构造独立 Pi 从最终 session 继续文件任务，`tests/pi-session-history.mjs` 已含实际 `restoreSnapshot` 和 Git 对象断言。剩余是任意 custom 外部引用、跨仓库对象与外部运行资源的交接，不是所有脱离 AScout 续跑和 rollback 均未支持。compaction 摘要保留原文已定；实际验证范围见 [接入实现](agent-access.md#4-session-持久化与状态交接已定方向及实现差距)、[Pi 审查](pi-session-review-2026-11-13.md#实际验证)。
- 第 31、34 项：主动保存恢复已覆盖树、原上下文和候选；NLE 恢复已保存初始 RNG／配置、动作序列和校验值。旧 run 的初始环境自动重建、任意外部资源恢复及全部长轨迹仍不保证，见 [恢复](recovery.md)、[回放](replay.md)。
- 第 32 项：回放已有兼容源的两输入整链验证记录，不能继续笼统写“完整 NLE 回放仍失败”；也不能由该源通过推出任意源或机制整链已验收。匹配碰撞、取消时序、最终回答交付与环境前提仍按 [回放](replay.md) 保留。

本次只核对文档、源码及已有测试断言，没有运行测试或模型；其余清单项保留原有设计／验收限定，不将“未发现完成证据”当成“已证明没有实现”。

本轮实际运行：

- `cargo test --lib control_plane::`：22 项通过。
- `cargo test --lib runtime::tests::`：13 项通过。
- `/usr/bin/node --test tests/pi-access.mjs tests/nle-map-history.mjs tests/tree-layout.mjs`：14 项通过。

共 49 项通过。未新增测试、未调用真实模型、未执行特权环境验收；源码核查到的能力不自动视为端到端通过。其他待定选型与发布范围仍需用户决定，不能仅凭存在局部代码关闭。
