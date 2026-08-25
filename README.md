# AScout

AScout（Agent Scout）是面向 Agent 投机执行的控制与运行机制。AScout 对外抽象为一个 AScoutAgent；每个 AScoutAgent 在控制面拥有一棵 Speculative Tree。在 pending-result boundary，parent 可以继续真实 operation，candidate Branches 则基于不同 Proposal 提前执行。Candidate 提交 commit request，持有 resolver capability 的主体至多接受一个 winner；控制面通过 Runtime Adapter 侧触发 Runtime Mechanism 的节点级转换和 loser cleanup。

## 仓库状态

仓库当前只有设计文档，没有实现。当前没有可用的 AScout lifecycle、Agent-facing 命令、control protocol、真实 Runtime backend 或 security sandbox。

目标语义与一级架构已经形成。AScout 按 `Agent 接入侧 -> 控制面侧 -> Runtime Adapter 侧 -> Runtime Mechanism` 组织。Runtime Adapter 侧内部包含稳定的 Harness Runtime Boundary 和面向具体 Runtime 的实际 Adapter。设计文档是语义权威；开发文档记录已接受的实现选择与剩余任务。

## 已确认的实现边界

- 新实现使用 Rust。具体 crate、模块、进程和部署边界按四侧 ownership 重新决定（D-024-I）。
- 六类扩展点的职责语义已确认：Execution、Workspace、Input、Effect 和 Outcome 属于 Runtime Mechanism；Operation Integration 属于 Agent 接入侧，负责识别 pending-result boundary、保证真实 operation 只执行一次并自动发布 outcome。
- 默认 Runtime 面向可信 Child Agent，优先高效 sibling isolation；对抗恶意 workload 的 containment 由另行选择的 Runtime 提供。
- Runtime 必须强制维护 identity/capability、sibling isolation、资源归属、loser cleanup 和受管 external effect 路径。

## 文档

文档以两个根文档为入口，再延伸到各语义权威与实现细节（详见 [文档导航](docs/index.md)）。

- [心智模型](docs/mental-model.md)
- [一级架构](docs/architecture.md)
- [目标语义](docs/design/overview.md)
- [设计决策](docs/design/decisions.md)
- [投机执行底座设计决策](docs/design/speculation-decisions.md)
- [开发决策](docs/development/decisions.md)
- [开发路线图](docs/development/roadmap.md)
- [验收状态](docs/development/verification.md)
- [首要验证 Workload](docs/development/reference-workload.md)
- [Agent 接入状态](docs/usage/agent-guide.md)
- [FAQ](docs/usage/faq.md)
- [投机执行底座抽象设计札记](docs/development/spec-exec-abstraction.md)
- [讨论记录](Discussion.md)
