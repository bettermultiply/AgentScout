# 开发路线图

本路线图只跟踪当前仓库和新实现。

## 仓库重置

- [x] 删除旧源码、Cargo manifests、tests、examples、C header、构建产物、实验文件和实现专属文档。
- [x] 只保留目标语义、已接受的新实现决策及面向未来的使用/开发文档。

## 核心架构

- [ ] 围绕目标组件模型（Control Plane：Policy Engine、Assumption Manager、Scheduler、Effect Coordinator、Ledger、Runtime Adapter；Instance 侧：Interposer、Agent、ASC-Daemon、Isolation Env）决定 crate/模块与依赖组织（D-023-I）。
- [ ] 建立 Harness policy ownership，以及 context、children、创建、commit、accept、discard 和 event wait 操作。
- [ ] 建立 Command + Skill Operation Integration、Codex launch adapter 和 CLI integration entry。
- [ ] 建立 Branch-local Local Controller（ASC-Daemon 的控制入口部分），并接入 central Harness Entry/Core dispatch。

## Runtime 正确性

- [ ] 管理完整 Agent process tree 及其 sub-agents，并限制在一棵 Branch tree 内。
- [ ] 强制保证 identity/capability 不可伪造、sibling isolation、资源归属、loser cleanup 和禁止跨 tree 操作。
- [ ] 实现 Branch barrier：完整 execution quiescence、共同 execution base、role-specific return 和无半创建状态。
- [ ] 实现 Resolution barrier：原子 validation、winner transition、loser cleanup 与 winner/fallback resume。
- [ ] 分离 Branch lifecycle 与 execution activity，包括 wait、suspend、resume、resolver delegation 和 parent-loss fail-closed。
- [ ] 允许带 live descendants 的 candidate commit，并保留 accepted winner identity 与 subtree。

## 状态与 Operation

- [ ] 实现文件/目录的 Branch Workspace View、ancestor visibility、Branch-local delta、Accept 与 discard。
- [ ] 实现 Input fan-out 和独立读取位置，不设置 input-consumption commit gate。
- [ ] 实现 OutcomeSlot、Proposal compatibility、unresolved obligation、conditional commit 和 Accept validation gate。
- [ ] 实现 query、transform、shared operation、session 和 opaque command contracts，并校验 context equivalence。
- [ ] 隔离 opaque commands 且禁止盲目 reuse；只有 contract 允许时才复用 result、Workspace delta、artifact 或 shared outcome。
- [ ] 实现 service adapter observation surface、guarantee horizon、once-only execution 和 winner-only semantic materialization。

## Launch 与 Integration

- [ ] 定义并实现第一版 Branch-local Local Controller HTTP Server/client：隔离环境内约定端口、request/response envelope、错误到 CLI 退出码映射。
- [ ] 定义 Harness Core 选择并连接独立 Runtime Service 的 client/adapter、service discovery、连接失败和重连边界。
- [ ] 通过 Harness Entry API 与 Harness Core 完成 ASCoutAgent start path；Runtime-side start handshake 仍待实现。
- [ ] 实现首个 `ascout codex` adapter，不让 Codex detail 进入 Runtime 实现。
- [ ] 先实现 Command + Skill，再评估 Hook automation。
- [ ] 由 Operation Integration 自动发布真实 outcome，不增加独立 Agent publish/delegate commands。

## 首要 Workload

- [ ] 建立 deterministic delayed-choice coding outcome producer 和真实仓库任务。
- [ ] 用相同 Codex 配置比较 sequential 与 ASCout speculation。
- [ ] 验证真实 operation once-only、sibling isolation、Proposal gate、loser cleanup、no-match fallback 和最终任务等价性。
- [ ] 测量端到端 latency、ASCout overhead、hit rate、有效提前工作及 loser token/CPU/memory cost。
- [ ] 只有 Command + Skill 证明正确性和收益后才增加 Hook automation。

## Research

- [ ] 在组件模型与接口定义建立后选择 continuation、Workspace isolation 和 recovery backends。
- [ ] 定义首个 workload 之外的 irreversible-effect support。
- [ ] 评估附加 Runtime security profiles，不把 adversarial containment 设为默认 Runtime 要求。
