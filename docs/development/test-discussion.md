# 测试筛选与复跑

## 筛选标准

- 原型阶段默认不新增测试。遇到可稳定复现、且可能再次发生的故障时，先让回归测试复现，再修复。
- 端到端测试从实际 CLI、协议或工具入口检查调用方可见的结果。确定性模拟用手工事件、替身或故障注入验证状态转换、取消、重试和恢复。
- 删除重复断言和只检查内部字段或布局的用例。仅在保留关键断言和独立故障覆盖时合并启动流程。发布重试与重启恢复、Pi 通知交付与协议重传、Firecracker 的 SSH 与 vsock 各覆盖不同故障边界。

## 执行约束

普通测试的工作目录由本次运行独占，默认通过或失败均清理临时产物。本机日志无需写入文档。用手工事件或本地伪模型验证，真实模型调用须单独授权。

`tests/` 下的 `pi-history-regressions.mjs`、`pi-notification.mjs`、`pi-recovery-binding.mjs`、`pi-recovery-sources.mjs`、`pi-session-history.mjs`、`pi-standalone-continuation.mjs` 当前在 `/var/tmp` 创建独立目录，将 `PI_CODING_AGENT_DIR` 指向各自的 `agent` 子目录，禁用 Jiti 磁盘缓存，并在 `finally` 清理。Authority 和下表两份恢复脚本目前仍会留下运行目录。

## 故障实例与复跑入口

| 故障与应保留的检查 | 仓库入口 |
| --- | --- |
| 大文件写入曾因命令参数过长报 `E2BIG`。保留大内容读写、隔离视图和有序编辑检查。常驻 holder 允许命令并发，直接 shell 退出后可返回，后台工作归环境管理。 | `node --test tests/pi-filesystem-tools.mjs`，`cargo test --offline --lib namespace_holder_runs_concurrent_commands_in_each_branch_mount_namespace` |
| 物化曾误用工作区 `PATH` 中的 Python。回归检查污染 `PATH` 时仍能发布文件。 | `cargo test --offline --lib materialization_ignores_workspace_python` |
| 首次准入曾因缺失 `has_predictor` 失败。仓库 fixture 支持手工事件和本地 SSE 复跑，也覆盖 Promote 后在途模型等待点曾丢失的故障。 | `node --test tests/predictor-fact.mjs tests/predictor-fact-e2e.mjs` |
| 已结束的 run 曾在继承历史送完前报告 `ready`。回归扣住首条历史消息，检查输入保持关闭，并在首次就绪时核对完整历史。 | `node --test tests/resume-ended.mjs` |

## 待核实与覆盖缺口

- Authority 发布失败后的公开状态曾读 Ledger 报 `missing field timestamp_ms` 并断开连接，Filesystem 恢复和结束后重开也遇到同一错误。保留公开 API 断言。复跑 `node --test --test-concurrency=1 tests/authority-filesystem.mjs tests/recovery-filesystem.mjs tests/resume-ended.mjs` 前，三份夹具均需处理对已缺失的 `assets/policy/policy/decisions.py` 的引用。另见 [恢复](recovery.md)。
- btrfs snapshot-copy 的 overlay 挂载曾报 `Device or resource busy`，单项复跑通过，稳定性仍待确认。见 [文件系统测试与基准复跑](../fs-backend/current-state.md#测试与基准复跑)。
- 真实 NLE 多步验收已在修复 Promote 等待点后补验通过。纯 Filesystem 的真实短任务覆盖隔离、物化和 Discard，仍缺仓库内持久入口及 Promote 后续跑覆盖。实验统计见 [参考负载](reference-workload.md)。

Django #13195 的投机与串行对照数据仅见于本页。两组各一次真实会话，使用同一 release 二进制，宿主两阶段验收均通过。投机组耗时 71.004 秒，权威物化 239 ms。两次 Promote 分别复用 881 ms 和 923 ms 的候选推理。三次 Discard 中有一次因只读 `/tmp` 无法写入重定向日志，测试未启动。串行组关闭投机准入，耗时 115.332 秒，比投机组长 62.43%。
