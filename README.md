# AScout

AScout（Agent Scout）是面向 Agent 投机执行的通用底座，位于投机策略与执行机制之间：上层只需设计和组合投机执行策略，不必关心具体实现；下层 Runtime 提供投机执行所需的基本能力，而不与某个特定投机算法耦合。AScout 对外抽象为一个 AScoutAgent；每个 AScoutAgent 在控制面拥有一棵 Speculative Tree。在 pending-result boundary，parent 可以继续真实 operation，candidate Branches 则基于不同 Proposal 提前执行。Candidate 提交 commit request，持有 resolver capability 的主体至多接受一个 winner；控制面通过 Runtime Adapter 触发 Runtime 的节点级转换和 loser cleanup。

## 仓库状态

仓库现在有一个按控制面 ownership 实现的 Agent 管理原型：Rust 控制面拥有 Tree/Branch 状态，通过 Unix socket 管理 Pi Agent 接入，当前 Runtime 承载以不同目录提供最低程度的环境隔离。它还不是正确完成的投机验证闭环、完整 Runtime、通用 Agent 产品或 coding workload。

目标语义与一级架构已经形成。AScout 按 `Agent 接入侧 -> 控制面 -> Runtime Adapter -> Runtime` 组织。Runtime Adapter 对控制面提供稳定 typed contract，并负责目标 Runtime 的能力映射与协议适配。设计文档是语义权威；开发文档记录已接受的实现选择与剩余任务。

## 已确认的实现边界

### 运行 AScout TUI（首步交互实现）

目标系统入口是控制面：用户请求进入控制面，控制面启动 Pi Agent 及其接入组件。Pi 接入只运行 AgentSession、收集 Agent 行为事实和执行控制命令，不是用户入口。

从真实终端启动：

```sh
cargo run --bin ascout-control-plane -- /tmp/opencode ../pi-code
```

第一个参数是本次运行目录的父目录，第二个参数是 Pi 源码路径。AScout 自动启动接入进程，在本次新建目录内运行 Agent，并使用 Pi 自身配置的模型与认证，不写死 provider。AScout 自己呈现流式回答和工具活动，不使用 Pi 主界面。输入 `/ascout` 查看当前 Tree，再输入编号查看 Branch；再次输入 `/ascout` 或按 Esc 返回主对话。Ctrl-C 请求中断，Ctrl-D 退出，主对话中 ↑/↓ 滚动。运行目录及 `agent.log` 保留用于检查。

当前只接通持续对话和只读观察，尚未连接 Policy、真实等待点候选续跑或 Promote；界面明确说明没有发生投机，不再运行无共同等待点的双会话示例。候选创建、真实 Parent outcome 比较、确认式退出清理、完整多行编辑、Markdown 排版与产品特有交互仍待实现。不能据此声称已完成可尝试的投机执行。

Policy YAML 编译与受限 Python 求值已可独立运行：

```sh
cargo run --example policy -- compile assets/policy/policy.yaml /tmp/opencode/policy.json
cargo run --example policy -- evaluate /tmp/opencode/policy.json extent assets/policy/extent-input.json
```

需要 Linux、Python 3、bubblewrap 和 libseccomp。当前接口、支持范围及未完成能力见 [Policy Engine 原型](docs/development/policy-engine.md)；这条路径不等同于完整 Branch 生命周期接入。

- 新实现使用 Rust。具体 crate、模块、进程和部署边界按四侧 ownership 重新决定（D-024-I）。
- 扩展点的职责语义已确认：Execution、Workspace、Input、Effect 和 Outcome 属于 Runtime；Interposer 属于 Agent 接入侧，统一负责识别 pending-result boundary、保证真实 operation 只执行一次并自动发布 outcome。
- 默认 Runtime 面向可信 Child Agent，优先高效 sibling isolation；对抗恶意 workload 的 containment 由另行选择的 Runtime 提供。
- Runtime 必须强制维护隔离环境、环境级资源边界、环境内受控操作、停止和 cleanup；Agent 接入侧负责 Agent 执行实体、Branch context、operation 归属和 capability 路由，Agent 造成且可能影响全局的外部效果（例如调用外部 API）由控制面内的 Effect Coordinator 负责实际处理。

## 文档

文档以心智模型为唯一根文档，再延伸到各语义权威与实现细节（详见 [文档导航](docs/index.md)）。

- [心智模型](docs/mental-model.md)
- [问题与目标](docs/problems-and-goals.md)
- [设计模型](docs/design-model.md)
- [一级架构](docs/architecture.md)
- [Branch 语义](docs/design/branch-semantics.md)
- [设计决策](docs/design/decisions.md)
- [投机执行底座设计决策](docs/design/speculation-decisions.md)
- [开发决策](docs/development/decisions.md)
- [首要验证 Workload](docs/development/reference-workload.md)
- [Agent 接入状态](docs/usage/agent-guide.md)
- [FAQ](docs/usage/faq.md)
- [投机执行底座抽象设计札记](docs/development/spec-exec-abstraction.md)
- [讨论记录](Discussion.md)
