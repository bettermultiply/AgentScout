# Agent 接入状态

AScout 当前没有任何实现，当前仓库只有设计文档。Agent-facing 命令、Rust API、control protocol 和 Runtime lifecycle 均未实现。

## 当前可用内容

- `docs/design/` 中已经确认的目标语义；
- 已确认的四侧一级架构与 `docs/development/` 中的开发决策；
- 计划首先支持具备足够产品拦截点或 Hook 的特定 Agent product。

当前没有可构建代码。临时命令、HTTP control protocol、稳定 Rust API、C API、Runtime lifecycle 和正式 Agent-facing 命令语法均不存在。

## 计划中的 Agent Surface

Routine Agent-facing control 计划通过当前执行环境中的 ASC-Proxy 进入 Agent Control Gateway。ASC-Proxy 隐藏当前 Agent 执行上下文；其具体 transport 和部署形式未定。计划覆盖以下操作：

- 查询当前 Branch context；
- 查询 direct children；
- 创建 candidates；
- commit、accept 和 discard；
- 等待 events。

Outcome 由 Interposer 自动发布，不作为独立 Agent command。Resolver delegation 属于 launch 或创建配置，不作为 routine command。参数、返回值、阻塞行为、event 表示和具体命令拼写均未定义。

## 计划中的 Launch 与 Integration

当前支持具备足够拦截点或 Hook 的特定 Agent product。产品相关的内部执行结构、拦截点和 Hook 归 Agent 接入侧；routine control 计划先进入 ASC-Proxy，再由 Agent Control Gateway 转交控制面。当前不承诺完全无修改且无需了解产品内部实现的通用接入。控制面只依赖 Runtime Adapter 的稳定 typed contract；Adapter discovery、连接和 Runtime handshake 后续单独定义。

launcher、CLI、library、Command + Skill 和 embedded Agent adapter 暂不属于当前范围，后续根据具体产品的可观察和可控制能力单独评估。

支持的 Agent product 必须提供关键执行活动的观察点，以及在真实 operation 开始前暂停或阻止执行的拦截点。Agent 接入侧还需要能够识别 operation 的开始、`pending-result boundary` 和完成，并获取 outcome；它负责后续 Decision、控制面协作以及让调用沿产品原有完成路径结束。

不依赖具体产品的组件责任与控制路径见 [Agent 接入侧设计](../design/agent-integration.md)。

## 可用性规则

不得从目标语义或 roadmap item 推断当前支持。只有当前实现及其验证证据存在后，使用文档才会加入对应构建、运行和 API 说明。
