# 2026-11-13 实现核查

本次核查修正 Discussion 对现有实现范围的描述，不改变目标语义。依据当前工作区源码及已有测试；工作区包含尚未提交的改动，结论不代表 HEAD 独立具备全部能力。

| 清单编号 | 当前实现证据 | 剩余边界 |
|---|---|---|
| 8、30 | `src/control_plane/agent_control/tests/` 覆盖接受确认后切权、接受失败、晋升中失败、迟到结果、重复启动、精确命令身份及嵌套失效；Pi 测试覆盖多个等待点 shutdown。 | 跨进程竞态、握手阶段信号、真实独立环境故障与退出仍需端到端验收。 |
| 16 | `src/runtime/selection.rs`、`composite.rs`、`base.rs` 已实现显式选择、组合与进程／数据面装配。 | 不等于按隔离需求动态查找与管理 Backend。 |
| 20 | `src/runtime/base.rs` 对 FS fork／mark／accept 冻结、操作、解冻，并报告失败；`process/freeze.rs` 有 cgroup 和进程组两种路径。 | 进程组逃逸限制须限定路径；无冻结机制仍可运行操作，不代表已保证所有写者静止。嵌套挂载与真实环境覆盖仍待验证。 |
| 22 | `src/control_plane/session.rs` 与 `src/runtime/adapter/` 有请求身份、超时、请求去重、有界结果缓存、环境队列和 EOF shutdown。 | 不承诺跨重启幂等、状态恢复或完整 Workspace 并发契约。 |
| 23 | `src/control_plane/agent_control/mod.rs` 保存 generation、execution_position、chance_id、assumption_set、pending_operations、execution_state；Chance 保存 authority_outcome 与 authority_evidence。 | 后续需审查能力、约束与 Runtime 对齐信息是否满足目标契约。 |
| 28 | `src/control_plane/coordination/actions.rs` 按 epoch 调度 Rebalance，处理 pause／resume／preempt；`agent-access/core/index.ts` 实现 hold。 | 不能笼统说 rebalance 未实现；效果复用与保留等扩展需分别检查。 |
| 29 | `webui/src/components/ChatPane.tsx` 已有多行 textarea；`MessageView.tsx` 展示文字、思考、工具调用和结果。 | 尚无 Markdown 渲染，图片等仍以未展示内容呈现。 |
| 33 | `src/control_plane/agent_control/tests/nested.rs` 覆盖活跃祖先假设阻止提交和祖先丢弃失效；`tools/replay/verify/b3-deep.mjs` 支持 3／4／10 轮。 | 单元回归不等于端到端祖先丢弃回放；本轮未运行深度脚本。 |
| 36 | `src/runtime/process/` 有进程身份、冻结、终止与清理，控制面已有 generation。 | 完整资源租约、跨重启恢复及执行延续方式仍不能据此认定完成。 |

仍确认存在的具体缺口：公共 ChanceCreated 事件没有源侧投机开始时间，尽管内部 ChanceRecord 已保存该时间；公共 Proposal 真实验证时刻仍缺；session 保存主要随 message_end，compaction 覆盖不足；NLE 配置仍只有单个 seed；历史恢复、物化及通用事件级回放界面不能认定已完成。树形几何和 NLE 地图已有实现，完整交付与统计验收仍是单独事项。

本轮实际运行：

- `cargo test --lib control_plane::`：22 项通过。
- `cargo test --lib runtime::tests::`：13 项通过。
- `/usr/bin/node --test tests/pi-access.mjs tests/nle-map-history.mjs tests/tree-layout.mjs`：14 项通过。

共 49 项通过。未新增测试、未调用真实模型、未执行特权环境验收；源码核查到的能力不自动视为端到端通过。其他待定选型与发布范围仍需用户决定，不能仅凭存在局部代码关闭。
