# ASCout

ASCout（Agent Scout）是面向 Agent 投机执行的 Outer Harness 设计。ASCout 对外抽象为一个 ASCoutAgent；每个 ASCoutAgent 在 Harness 侧拥有一棵 BranchTree。在 pending-result boundary，parent 可以继续真实 operation，candidate Branches 则基于不同 Proposal 提前执行。Candidate 提交 commit request，持有 resolver capability 的主体至多接受一个 winner，Harness 通过 Runtime Boundary 触发 Runtime 的节点级机制转换和 loser cleanup。

## 仓库状态

仓库当前只有设计文档，没有实现。当前没有可用的 ASCout lifecycle、Agent-facing 命令、control protocol、真实 Runtime backend 或 security sandbox。

目标语义与目标组件模型已经形成。设计文档是语义权威；开发文档记录已接受的开发决策与剩余任务。未来实现围绕目标组件模型（Control Plane：Policy Engine、Assumption Manager、Scheduler、Effect Coordinator、Ledger、Runtime Adapter；Instance 侧：Interposer、Agent、ASC-Daemon、Isolation Env）重新组织。

## 已确认的实现边界

- 新实现使用 Rust。具体 crate/模块与依赖组织围绕目标组件模型重新决定（D-023-I）。
- 六类扩展点的职责语义已确认：Execution、Workspace、Input、Effect 和 Outcome 属于 Runtime 侧；Operation Integration 属于 Harness/Integration 侧，负责识别 pending-result boundary、保证真实 operation 只执行一次并自动发布 outcome。
- 默认 Runtime 面向可信 Child Agent，优先高效 sibling isolation；对抗恶意 workload 的 containment 由另行选择的 Runtime 提供。
- Runtime 必须强制维护 identity/capability、sibling isolation、资源归属、loser cleanup 和受管 external effect 路径。

## 文档

- [文档导航](docs/index.md)
- [全局架构图集](docs/architecture.md)
- [目标语义](docs/design/overview.md)
- [设计决策](docs/design/decisions.md)
- [开发决策](docs/development/decisions.md)
- [开发路线图](docs/development/roadmap.md)
- [验收状态](docs/development/verification.md)
- [首要验证 Workload](docs/development/reference-workload.md)
- [Agent 接入状态](docs/usage/agent-guide.md)
- [FAQ](docs/usage/faq.md)
- [投机执行底座抽象设计札记](docs/development/spec-exec-abstraction.md)
- [讨论记录](Discussion.md)
