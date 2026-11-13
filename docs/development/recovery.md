# 主动退出恢复

2026-11-13 开始实施；本文件逐项记录实际能力，未通过端到端验证前不代表完整交付。

Pi 源码依据（相邻 ../pi checkout）：

- `packages/coding-agent/src/core/agent-session.ts` 的 `abort()` 立即调用 `agent.abort()`，取消 retry／compaction／branch summary，再等待 idle。
- 同文件 `_handleAgentEvent` 在 `message_end` 追加原生 assistant、toolResult、custom message；部分 assistant 的终态由 agent loop 提供。
- `packages/agent/src/agent-loop.ts` 将 stream 的最终部分消息交付 `message_end`，工具准备阶段检查 AbortSignal，已开始工具通过同一 signal 协作取消。取消不撤销已发生的外部副作用。
- `packages/agent/src/agent.ts` 的 `continue()` 不允许从 assistant 直接继续；重新打开 session 不自动启动推理。恢复等待下一条用户输入。

目标：直接中断、保存整棵有效树及原上下文、校验后恢复。保存失败不得触发原文件或环境删除，也不得报告退出成功。最终单一 Pi 历史与运行恢复元数据职责不同。

## 已实现的有限路径

控制面的 Shutdown（包括已有退出入口和退出信号）在无未完成控制操作、无活动/阻塞工具等待时，请求 Pi 同时 abort 所有 session，保存原生消息、原 Branch 源、接入状态和 Tree；NLE 保存完整初始 RNG/configuration、动作序列及逐步校验值。保存使用独立目录，最后发布包含三个组件 SHA-256 的 `recovery.json`。恢复校验组件、Policy digest、Runtime 配置内容和能力后加载，等待用户输入，不自动推理。保存失败报告失败并解除保存冻结；不删除原生源。仅单后端 NLE 路径已验证。

入口（run-dir 为原运行目录，其配置、源文件及工作区必须保留）：

```sh
cargo build --bins
target/debug/ascout-control-plane <run-dir> <pi-root> <policy.yaml> --resume --web 127.0.0.1:8080 --model <provider/model>
```

退出仍使用现有 Shutdown；保存后目录包含 `recovery-<timestamp>/{tree,agent,runtime}.json` 和 `recovery.json`。恢复不能只复制最终 `session.jsonl`；`sessions/` 保存各 Branch 的原上下文。NLE 重建只在固定版本和固定配置下校验，不声称公共 observation/RNG 校验是任意隐藏状态等价的数学证明。任意外部工具副作用不能靠此重放。

## 实际验证（2026-11-13）

- `cargo check --bins`、`cargo build --bins`、`cargo test --lib --no-run` 通过（最后一项是编译，不是执行全部 Rust 测试）。
- `node --test tests/pi-access.mjs tests/pi-session-history.mjs tests/pi-notification.mjs tests/pi-notification-adapter.mjs`：18/18。
- `node /tmp/opencode/check-recovery.mjs`：真实 Pi + 本地假 OpenAI SSE 服务 + 真实 NLE；模型流永不自然结束，Shutdown 触发取消并保存退出；跨进程 `--resume` 不请求模型，用户输入后请求第二次并再次保存退出。通过，证据 `/tmp/opencode/control-recovery-wKIQIV`。测试只请求本地假模型。
- 真实 NLE 手工 JSON-lines 驱动：30 动作、3 环境引用、保存时重建校验、进程退出、加载、observation 连续、下一动作、再次保存，通过；证据 `/tmp/opencode/nle-recovery-final-34_tgbvl`。
- 原生 SessionTree snapshot/restore 手工检查通过，证据 `/tmp/opencode/recovery-native-z0ck82`。
- 最后重建后再次执行 `node /tmp/opencode/check-recovery.mjs` 通过，证据 `/tmp/opencode/control-recovery-3xEDZs`；`cargo test --lib runtime::adapter -- --nocapture` 5/5，通过，包括预期注入 panic 的队列恢复。

## 未完成，不能作为整棵有效执行恢复交付

- 活动或阻塞工具批次、等待 sibling result 的候选、未完成控制操作目前直接拒绝保存并保留执行。这不满足完整目标，尤其尚未通过“authority + 两个有效候选含阻塞候选”的跨进程验收。
- FS 执行承载重新挂载、多后端组合保存尚未实现，明确拒绝；不能声称保存了任意子进程内存。
- 活跃命令/工具的等待关系重建、恢复前整棵 Tree/Chance/Proposal/环境关联的全面校验仍缺失。当前采取拒绝而非盲重放。
- 失败后继续运行会改变原生源，旧保存不能保证仍可恢复；需进一步独立复制源并完善保存事务。模型/extension 配置版本绑定尚不完整。
- 通知仅有既有 18 项回归，不代表已验证跨恢复的全部不丢不重场景。
