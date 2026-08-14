# 开发路线图

本路线图只跟踪当前仓库和新实现。实现顺序服从四侧一级架构，不从旧 crate 或部署图反推组织方式。

## 仓库重置

- [x] 删除旧源码、Cargo manifests、tests、examples、C header、构建产物、实验文件和实现专属文档。
- [x] 只保留目标语义、已接受的新实现决策及面向未来的使用/开发文档。
- [x] 确认 `Agent 接入侧 -> 控制面侧 -> Runtime Adapter 侧 -> Runtime Mechanism` 四侧一级架构（D-039、D-024-I）。

## Agent 接入侧

- [ ] 决定 Agent/product integration、CLI、library、launcher、ASC-Daemon、Interposer 与 Operation Integration 的模块、进程和部署边界。
- [ ] 建立 Branch-local ASC-Daemon 控制入口，并接入控制面 Entry API。
- [ ] 实现首个 `ascout codex` launcher 与 Command + Skill integration，不让产品细节进入控制面或 Runtime。
- [ ] 由 Operation Integration 识别 pending-result boundary，保证真实 operation once-only 并自动发布 outcome。
- [ ] 定义第一版本地调用 transport、request/response envelope 和错误映射；该 transport 不进入 ASCout 目标语义。
- [ ] Command + Skill 证明正确性和收益后，再评估 Hook automation。

## 控制面侧

- [ ] 决定 Harness Core、Entry API、ASCoutAgent/Speculative Tree 与 policy 组件的模块和依赖边界。
- [ ] 建立 ASCoutAgent 与 Speculative Tree 控制状态及同 tree identity/capability 校验。
- [ ] 建立 context、children、branch、commit、accept、discard 和 event wait 的控制语义。
- [ ] 明确 Policy Engine、Assumption Manager、Scheduler、Effect Coordinator 与 Ledger 的最小第一版职责；不因研究组件清单自动各建独立 crate。
- [ ] 实现控制闭环：precheck、Runtime operation、已确认 event、控制状态 finalize、Agent-facing result。
- [ ] 保证 policy 只能在固定 lifecycle、single-winner、capability、obligation 与 cleanup 不变量内作选择。

## Runtime Adapter 侧

- [ ] 定义稳定、typed、可版本化的 Harness Runtime Boundary operation/event contract。
- [ ] 分离 Boundary contract 与具体 Runtime Adapter，不让控制面依赖 Adapter 私有类型、transport 或 backend handle。
- [ ] 定义具体 Adapter 的能力报告、contract 映射、错误规范化和 event 上行规则。
- [ ] 后续收敛 Runtime 选择、Adapter discovery、连接建立、失败、重连与关闭语义。
- [ ] 验证替换具体 Runtime Adapter 不要求修改控制面 semantic API。

## Runtime Mechanism

- [ ] 管理完整 Agent process tree 及其 sub-agents，并限制在一棵 Speculative Tree 内。
- [ ] 强制 identity/capability、sibling isolation、资源归属、loser cleanup 和禁止跨 tree 操作。
- [ ] 实现 Branch barrier：完整 execution quiescence、共同 execution base、role-specific return 和无半创建状态。
- [ ] 实现 Resolution barrier：原子 validation、winner transition、loser cleanup 与 winner/fallback resume。
- [ ] 分离 Branch lifecycle 与 execution activity，包括 wait、suspend、resume、resolver delegation 和 parent-loss fail-closed。
- [ ] 允许带 live descendants 的 candidate commit，并保留 accepted winner identity 与 subtree。
- [ ] 实现 Execution、Workspace、Input、Effect 与 Outcome backend participant contract；具体 backend 技术后续选择。

## 状态与 Operation

- [ ] 实现文件/目录的 Branch Workspace View、ancestor visibility、Branch-local delta、Accept 与 discard。
- [ ] 实现 Input fan-out 和独立读取位置，不设置 input-consumption commit gate。
- [ ] 实现 OutcomeSlot、Proposal compatibility、unresolved obligation、conditional commit 和 Accept validation gate。
- [ ] 实现 query、transform、shared operation、session 和 opaque command contracts，并校验 context equivalence。
- [ ] 隔离 opaque commands 且禁止盲目 reuse；只有 contract 允许时才复用 result、Workspace delta、artifact 或 shared outcome。
- [ ] 实现 service adapter observation surface、guarantee horizon、once-only execution 和 winner-only semantic materialization。

## 首要 Workload

- [ ] 建立 deterministic delayed-choice coding outcome producer 和真实仓库任务。
- [ ] 用相同 Codex 配置比较 sequential 与 ASCout speculation。
- [ ] 验证真实 operation once-only、sibling isolation、Proposal gate、loser cleanup、no-match fallback 和最终任务等价性。
- [ ] 测量端到端 latency、ASCout overhead、hit rate、有效提前工作及 loser token/CPU/memory cost。

## Research

- [ ] 在四侧接口定义建立后选择 continuation、Workspace isolation 和 recovery backends。
- [ ] 定义首个 workload 之外的 irreversible-effect support。
- [ ] 评估附加 Runtime security profiles，不把 adversarial containment 设为默认 Runtime 要求。
