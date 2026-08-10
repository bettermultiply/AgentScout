# ASCout

ASCout（Agent Scout）是面向 Agent 投机执行的 Outer Harness 设计。ASCout 对外抽象为一个 ASCoutAgent；每个 ASCoutAgent 在 Harness 侧拥有一棵 BranchTree。在 pending-result boundary，parent 可以继续真实 operation，candidate Branches 则基于不同 Proposal 提前执行。Candidate 提交 commit request，持有 resolver capability 的主体至多接受一个 winner，Harness 通过 Runtime Boundary 触发 Runtime 的节点级机制转换和 loser cleanup。

## 仓库状态

仓库当前已有第一版 Rust workspace skeleton。过时原型的源码、tests、examples、C header、构建产物和实现专属文档已经删除；Git history 是旧原型的唯一历史来源，不能约束新实现。当前可构建 `pbw` binary 和当前 workspace crates，但还没有可用的 ASCout lifecycle、Agent-facing 命令语法、control protocol、真实 Runtime backend 或 security sandbox。

目标语义已经形成。设计文档是语义权威；开发文档记录已确认的 Rust workspace 架构、当前 skeleton 和剩余任务。

## 已确认的实现边界

- 新实现使用 Rust，并采用 Cargo workspace。
- 初始 skeleton crate 拓扑为 `pbw-contract`、`pbw-runtime-api`、`pbw-runtime-core`、`pbw-runtime-default`、`pbw-harness`、`pbw-integration-command`、`pbw-launch-codex` 和 `pbw-cli`；Branch-local control endpoint 边界确认后，已新增 `pbw-local-control` 独立 crate。
- `pbw-cli` 只负责 CLI integration；routine CLI 在 Branch 隔离环境内通过 Local Controller 发出无身份 intent，不选择、创建或连接 Runtime。Local Controller 第一版可实现为约定端口 HTTP Server，并把自身绑定的 ASCoutAgent/Branch context 交给 Harness Entry API。Harness Core 后续通过 Harness Runtime Boundary 连接独立 Runtime Service；具体 Runtime connection protocol 尚未收敛。
- Runtime 侧扩展点是 Execution、Workspace、Input、Effect 和 Outcome；Operation Integration 属于 Harness 侧。
- 默认 Runtime 面向可信 Child Agent，优先高效 sibling isolation；对抗恶意 workload 的 containment 由另行选择的 Runtime 提供。
- Runtime 必须强制维护 identity/capability、sibling isolation、资源归属、loser cleanup 和受管 external effect 路径。

D-021-I 定义的一次性 skeleton 已创建并通过本地验证。当前额外完成了 Harness-side BranchTree metadata control path；skeleton 仍只证明 ownership graph、CLI/Local Controller/Harness boundary 和 Runtime placeholder 失败路径，不代表 ASCout Runtime lifecycle、Runtime Service connection 或隔离能力已经实现。

## 文档

- [文档导航](docs/index.md)
- [全局架构图集](docs/architecture.md)
- [目标语义](docs/design/overview.md)
- [设计决策](docs/design/decisions.md)
- [开发架构](docs/development/architecture.md)
- [开发决策](docs/development/decisions.md)
- [开发路线图](docs/development/roadmap.md)
- [验收状态](docs/development/verification.md)
- [首要验证 Workload](docs/development/reference-workload.md)
- [Agent 接入状态](docs/usage/agent-guide.md)
- [FAQ](docs/usage/faq.md)
- [讨论记录](Discussion.md)
