# Agent 接入侧设计

本文展开 [AScout 架构](../architecture.md) 中的 Agent 接入侧，定义 Agent 或 Agent product 如何进入 AScout 的受控路径，以及接入组件之间的责任边界。本文只记录不依赖具体产品和 transport 的目标语义；命令拼写、HTTP 字段、端口、进程拆分和产品适配方法属于开发与使用文档。

## 1. 目的与边界

Agent 接入侧把 Agent 的普通执行行为或显式协作意图转换为控制面能够处理的请求，同时把控制面确认后的结果和事件交还给对应 Branch。对于由 Policy Engine 注入的、不依赖或不改变全局状态的本地 Decision，它可以在本地执行并记录 decision log，再通过 Agent Control Gateway 把结果和日志交给控制面；这不使接入侧拥有 Policy 或全局控制状态。它负责绑定当前 `AScoutAgent`、Branch 与本地运行上下文，但不拥有 Speculative Tree、Policy、后端执行状态或 Runtime capability。

Agent 接入侧不能绕过控制面直接发起 AScout lifecycle 转换，也不能把一次本地调用成功解释为后端状态已经转换。需要 Runtime 参与的操作仍须经过 `Agent 接入侧 → 控制面 → Runtime Adapter → Runtime` 闭环。

## 2. 接入组件

### 2.1 Integration

Integration 是 Agent 或具体产品进入 AScout 的适配层，包括 launcher、CLI、library、Command + Skill，以及未来可能采用的 Hook。它负责识别产品侧的调用形式、保留终端与环境体验，并把 Agent intent 交给 Branch-local ASC-Daemon。

不同 integration profile 可以改变调用形式和可观察时机，但必须进入同一套 Agent-facing semantic operation，不能各自定义 lifecycle。具体产品的参数、环境变量、TTY 处理和 Hook API 不进入本文。

### 2.2 ASC-Daemon

ASC-Daemon 是 Branch-local 的本地控制入口。它绑定自身所在 Branch 的上下文，使 CLI 或 library 无需携带裸 Branch identity 或 capability；随后把已经解析上下文的 typed request 交给控制面 Agent Control Gateway。Agent Control Gateway 不再从调用进程环境重新猜测身份。

ASC-Daemon 可以承载 lifecycle event、上下文更新、本地 Decision 结果、decision log 和本地存活信息的收发，但不持有 AScout 全局控制状态，也不自行决定 Branch 是否被接受、丢弃或提升。它只执行 Policy Engine 下发且明确属于本地范围的 Decision；涉及全局状态、资源或生命周期的判断仍交给控制面。

### 2.3 Interposer

Interposer 位于 Agent 请求路径上，捕获当前 integration profile 能观察的 Agent step，并按本地 Policy Decision 或控制面返回结果决定继续原调用、等待或向 Agent 返回受控结果。它可以执行不依赖或不改变全局状态的本地 Decision，并将输入事实、Policy 版本、Decision 结果和执行上下文记录到本地 decision log；它不能据此直接改变 Speculative Tree、Branch 生命周期、全局预算或 Runtime 状态。论文调研中出现过 LLM 调用、工具分发、轨迹写入与调度等接缝；它们是接入候选，不是要求每个 profile 同时实现的固定四接口。

Interposer 的请求路径对延迟敏感，而 ASC-Daemon 还承担本地控制与事件旁路；二者责任不同。是否实现为不同进程、模块或 Hook，不在目标语义中规定。

### 2.4 Operation Integration

Operation Integration 识别规范化 operation 及其 `pending-result boundary`。当真实 operation 被执行时，它必须保持 once-only，并在完成后自动发布 outcome；`PublishOutcome` 因此是内部 lifecycle semantic operation，不是日常 Agent command。

Operation Integration 可以由 wrapper、Hook、effect broker 或其他产品适配机制实现。具体捕获方式可以变化，但不能改变 outcome 的归属、重复执行纪律或 Branch lifecycle。

## 3. Agent-facing 控制路径

日常调用遵循以下路径：

```text
Agent / product
    → Integration
    → Branch-local ASC-Daemon（绑定上下文）
    → 本地 Policy Decision（如适用，记录 decision log）
    → Agent Control Gateway
    → 控制面受管操作
    → Agent-facing result / event
```

当前只保留以下 operation slots：查询自身 `context`、查询 direct `children`、`branch`、`commit`、`accept`、`discard` 和等待 event。它们的参数、返回值、阻塞方式与 wire format 尚未定义，不能从这些名称推导实现接口。需要控制面代为裁决时，delegation 通过 task 或 Branch 创建配置表达，不新增日常命令。

Agent-facing control surface 不等于端用户界面。端用户直接操作 AScoutAgent 当前有效的权威执行主线，不选择内部 Branch，也不参与 Branch 的接受、丢弃、重新挂载或清理。Policy 根据运行事实形成处置意图，控制面在固定生命周期约束内编排，并根据 Runtime Adapter 返回的结果更新当前有效主线。

端用户只能看到当前权威路径产生的内容、状态与交互。Candidate 或其他仍携带未决 Proposal 的 Branch 可以在内部产生结果和事件，但不能直接向用户输出内容或发起交互；只有被接受并进入权威路径后，才可通过同一个 AScoutAgent 视图呈现其用户可见内容。用户输入由当前有效主线接收，之后是否作为输入事实传播给仍在运行的 descendants，遵循 Branch 输入传播规则。

## 4. 部署与生命周期

ASC-Daemon、Interposer 与 Operation Integration 逻辑上属于 Agent 接入侧，但可以运行在外部 Runtime 为对应 Instance 提供的 Branch 隔离环境内。它们随该环境启动、停止或失效，不因此成为 Runtime 的内部组件，也不能持有跨 Branch 的可变权威状态。

Task 绑定 Policy 并建立受管上下文时，Policy Engine 可以把已经解析和校验的 Policy 执行表示注入对应的 Agent 接入侧。接入侧在后续本地执行符合条件的 Decision，不需要为每次 Decision 重新获取或解析 Policy；执行结果和 decision log 仍通过 Agent Control Gateway 返回控制面。

Agent 默认可以不感知投机；可选的显式协作入口可以让 Agent 参与候选创建或裁决，但仍受相同身份、single-winner、效果与 lifecycle 约束。端用户只通过当前权威路径提供输入，不进入内部 Branch lifecycle control surface。

## 5. 固定约束与未定实现

固定约束包括：

- 所有 integration profile 进入同一 semantic API；
- Branch context 由本地接入边界绑定，不由 Agent Control Gateway 猜测；
- Policy 由控制面统一加载、解析、校验和版本管理；不依赖或不改变全局状态的 Decision 可以通过已解析的执行表示注入接入侧执行，并记录本地 decision log；
- 控制面不重复执行已满足本地范围条件的 Decision，但仍接收其结果和日志；
- Agent 接入侧不拥有控制面或后端权威状态；
- 真实 operation 保持 once-only，outcome 自动归入对应受管路径；
- Agent-facing 组件不能绕过控制面或直接控制 Runtime。

尚未确定的实现包括 transport、请求与事件 schema、进程/模块拆分、Interposer 的具体拦截面，以及不同 Agent product 的接入方式。这些内容收敛后应写入 `docs/development/` 或 `docs/usage/`。

## 相关文档

- [AScout 架构](../architecture.md)：四侧调用路径与责任边界
- [Branch 语义设计](branch-semantics.md)：Agent-facing operation 所受的 lifecycle 约束
- [控制面设计](control-plane.md)：Agent Control Gateway 后的控制路径
- [Agent 接入状态](../usage/agent-guide.md)：当前实现与可用性
