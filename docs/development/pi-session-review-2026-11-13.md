# Pi session 审查修复（2026-11-13）

依据设计提交 `6b99d9b` 和实现提交 `2e4f76f`。这份记录只描述本轮实际实现和验证，不把 session 复制等同于完整外部状态交接。所有模型请求均发送到本地伪模型或回放服务，未调用付费模型。

## 修复

- `SessionTree.settle()` 从原源 manager 当前父链找到精确 checkpoint，只在其后、下一条 assistant 前收集本次全部工具结果；不再从全索引按重复 `toolCallId` 取最后一条。缓存已定位真实尾记录，源 manager 清理后仍可重复整理。
- 从候选自己的当前父链定位预测配对尾节点的直接后续记录，保存其 ID；嵌套采纳分别处理各自 checkpoint。运行 manager 与原 checkpoint 保持原样，只改变最终文件。
- 分支复制后以原生 custom authority 记录持久化选择位置，再把运行 manager 选择回原 checkpoint。解决 `branch()` 仅修改内存、独立重开落到复制索引最后旁支的问题。
- cleanup 等待候选 setup、abort、该分支 turn，以及异步 `session_shutdown(reason=quit)`；同步最终记录后 dispose，成功汇总才标停止并删源。EOF shutdown 同样处理尚未登记为活动 session 的原生分支文件。
- 保留并验证父引用、原生 `targetId` / `firstKeptEntryId` / `fromId` 和已安装 rollback 快照的 `data.targetId`。任意 custom payload 的外部引用无法通用解释，不声明已全面验证。

## 分支复制与原生 fork 的差异

Pi `SessionManager.createBranchedSession()` 修改 receiver，仅保留选定父链，并重新生成 label 记录及其 ID；直接在活跃 manager 调用会改变运行会话。本实现保留完整原生索引、原 entry ID、label 与旁支引用，创建独立 header（新 ID、时间、cwd、parentSession），通过原生 manager 追加落点记录。它与原生 fork **并非完全语义等价**：保留范围更大，增加读取位置记录；本轮验证的是 checkpoint 上模型上下文、原 ID 和独立重开位置。不能据此宣称任意 Pi 扩展、迁移或外部资源均支持。

## 实际验证

1. `tests/pi-history-regressions.mjs`：先在修复前运行三项，全部失败；修复后全部通过。复现全索引误选 off-path 预测、误改第一条旁支 child、独立重开错误 leaf。另覆盖并行两结果、下一轮重复 call ID、嵌套两次重挂、停止删除后重复整理与原 checkpoint 不变。
2. `node --test tests/pi-access.mjs tests/pi-session-history.mjs tests/pi-history-regressions.mjs tests/pi-standalone-continuation.mjs`：14 项。覆盖完成、工具等待、候选闸门、准入等待和 EOF 退出；await 后追加的 shutdown custom 记录必须仍在最终文件。SDK double 测试与真实 Pi 测试分别标识，不能混称全部为真实 SDK。
3. 独立真实 Pi：`tests/pi-standalone-continuation.mjs` 从停止汇总的最终文件创建新的真实 `AgentSession`，本地伪模型接收真实结果上下文，用 Pi 内建 `write` 写入 `answer.txt`，再完成原任务。实际断言文件内容、工具成功、两次模型请求、最终落盘回答；另验证真实扩展异步 shutdown append。证明范围是同一目录的文件任务，不是 NLE 外部运行进程的独立迁移。
4. 已安装 `pi-code-rollback`：临时 Git 仓库生成快照，汇总、重开、启动 GC 后，通过扩展 `restoreSnapshot` 实际恢复修改的 `task.txt` 为 baseline，并删除快照后新增的 `new.txt`。保留 targetId、repoRoot、ref、tree 与 Git 对象断言。没有对用户仓库恢复；没有验证跨仓库 object/ref 迁移或完整交互式 `/restore` 导航。
5. `node tools/replay/verify/b1-pi.ts`：真实 Pi 解析两个并行调用、结果续跑、HTTP 请求键和计数通过；产物 `/tmp/opencode/ascout-replay-b1-pi-1jV4iH`。
6. `node tools/replay/verify/b3.ts`：真实控制面、Pi SDK、本地伪模型和真实 NLE；首跑与回放各 1 Chance、2 候选、1 Promote、2 Discard，4 次模型请求，完整权威 role/text 与 NLE 观测相同。回放 4/4 命中，缺失／耗尽／改键均 HTTP 400，两次控制面退出 0。产物 `/tmp/ascout-replay/b3/acceptance.json`，日志 `/tmp/opencode/pi-review-b3.log`。清理仅断言本脚本启动的子进程和占用端口，不干扰另一轮用户任务；不能推广为宿主全部进程已退出。

B3 旧断言要求预测值 `moved`，已按设计改为真实 NLE observation。前两次运行分别因过宽宿主进程断言和这个旧历史断言失败，修正后重跑通过。开发事实应保留失败原因，不能把前两次称作通过。

## 需要主 Agent 决策的最小未决项

- **可选真实结果通知**：当前 `dispose` 返回的 action 仅允许一个 `intent` 字段（`src/control_plane/policy/validation.rs`），`AgentCommand::Promote` 仅含 Branch、command ID、generation；本地 Policy 求值仅覆盖 admit/extent/effect，没有能承载通知选择或正文的现成返回位置。`prompt` 是用户输入并会启动 turn，不能伪装成 Policy 通知。可沿既有 dispose/Promote 扩展可选字段而不新增 Decision，但字段形状、传递内容及在飞时注入时机属于尚未确认的接口取舍，本轮不自行确定。默认完全不追加已保留。
- **预测已被 compaction 摘要覆盖**：保留原文时，恢复可包含已离开父链的预测陈述。需决定：这种 session 是否仍可作为整理完成的最终历史，或应报告摘要尚未整理而保留源文件。本轮保留原始摘要，不擅自让模型重写、删摘要或取消在飞推理。
- **外部状态**：FS 真实差异交接未获新 backend 授权；跨仓库 rollback Git 对象与 NLE 独立运行资源迁移仍未验收。不能把上述同仓库恢复和文件任务续跑称作所有外部状态完整交接。

Pi 当前 `AgentSession.prompt()` 默认支持扩展命令分发以及 skill/template 展开。此前“prompt 不能处理扩展命令”的报告不准确；但当前 launcher 对 CLI/TUI 式 command context 的绑定完整性仍未验证，不能由默认分发能力推断全部扩展命令可直接使用。

不受 Pi 等待生命周期管理、在 hook 返回后自行启动后台写入的扩展，无法由 `abort()` 或 `waitForIdle()` 证明已停止；本轮覆盖的是 awaited hook 和 turn，不声称任意 detached append 均安全。
