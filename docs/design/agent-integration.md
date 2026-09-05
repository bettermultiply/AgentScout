# Agent 接入侧设计

本文展开 [AScout 架构](../architecture.md) 中的 Agent 接入侧，定义 Agent 或 Agent product 如何进入 AScout 的受控路径，以及接入组件之间的责任边界。

## 1. 目的与边界

Agent 接入侧是对应 Agent 的端侧管理节点。它围绕一组 Agent 进程及其执行活动提供本地管理，观察 Agent 行为，并把普通执行行为、显式协作意图和本地执行结果转换为控制面能够处理的请求或事件。对于由 Policy Engine 注入的、不依赖或不改变全局状态的本地 Decision，它可以在本地独立判断和执行，并记录 decision log，再通过 Agent Control Gateway 把 Decision 内容、结果和日志交给控制面。涉及全局控制状态或生命周期权威的决策，必须交由控制面完成。端侧管理节点以 Branch 为执行活动的逻辑归属，为每个 Branch 承载独立的本地管理上下文，并依据 Branch identity、operation 归属和 capability 路由事件与控制命令。

Agent 主进程及其 sub-agent 进程共同构成一个 Agent 执行实体，由同一个端侧管理范围统一观察和处理。sub-agent 的推理请求、工具调用或命令调用，只要处于当前 integration profile 能观察的范围内，就由 Interposer 按所属 Branch 上下文处理；这些活动产生的 operation、Decision 和 outcome 归属于该 Branch。多个 Branch 在尚未需要独立 Branch-local 状态时，可以由同一个端侧管理节点在共享执行环境中承载，各 Branch 仍保持独立的管理上下文。

当某个 Branch 需要独立的 Branch-local 状态时，控制面通过 Runtime Adapter 建立相应的执行环境，该 Branch 的管理上下文随其执行活动进入能够提供相应隔离保证的环境。端侧管理节点可以复用、迁移或重新建立其实际承载方式，只要持续保持 Branch 管理上下文、事件和控制命令的正确归属。

Agent 接入侧不能绕过控制面直接发起 AScout lifecycle 转换，也不能把一次本地调用成功解释为后端状态已经转换。需要 Runtime 参与的操作仍须经过 `Agent 接入侧 → 控制面 → Runtime Adapter → Runtime` 闭环。

## 2. 接入组件

### 2.1 Integration

Integration 是 Agent 或具体产品进入 AScout 的适配层，包括产品相关的 launcher、CLI、library、Command + Skill、拦截器和 Hook。它负责识别产品侧的调用形式、保留终端与环境体验，并把 Agent intent 交给 Branch-local ASC-Daemon。当前支持具备足够拦截点或 Hook 的特定 Agent product；接入适配可以依赖产品的内部执行结构，以观察和控制 AScout 所需的执行阶段。

不同 integration profile 可以改变调用形式和可观察时机，但必须进入同一套 Agent-facing semantic operation，不能各自定义 lifecycle。AScout 不要求 Agent 修改自身投机算法或业务逻辑，但产品必须提供足够的接缝，使接入侧能够观察和控制所需的推理、工具及 operation 生命周期。具体产品的参数、环境变量、TTY 处理和 Hook API 不进入本文。

Agent product 的接入能力包括：观察关键执行活动，在真实 operation 开始前允许 Interposer 暂停或阻止执行，并在获得处理结果后让当前调用沿产品原有的完成路径结束。接入点还需要让 Interposer 识别 operation 的开始、`pending-result boundary` 和完成，并获取其 outcome；需要管理的 Agent 主进程及其 sub-agent 执行活动也应处于该接入 profile 的可观察范围内。

暂停或阻止真实执行后，Interposer 可以等待控制面结果或执行本地 Decision，再选择让真实 operation 继续执行，或使用已经获得语义授权的结果完成当前调用。后者仍沿 Agent 原有的调用完成路径返回，不要求 Agent 改变既有执行流程。Agent product 需要提供的是拦截和调用控制接缝；Branch 上下文、Decision 执行、控制面通信、日志记录以及结果是否获得授权，由端侧管理节点和控制面负责。

### 2.2 ASC-Daemon

ASC-Daemon 是端侧管理节点与控制面之间的本地控制入口。它根据当前 Agent 执行活动绑定相应的 Branch 上下文，使 CLI、library 或产品拦截点无需携带裸 Branch identity 或 capability；在共享执行环境中，它可以承载多个彼此独立的 Branch 上下文。随后它把已经解析上下文的 typed request 交给控制面 Agent Control Gateway。Agent Control Gateway 不再从调用进程环境重新猜测身份。

ASC-Daemon 可以承载 lifecycle event、上下文更新、本地 Decision 结果、decision log 和本地存活信息的收发，但不持有 AScout 全局控制状态，也不自行决定 Branch 是否被接受、丢弃或提升。它只执行 Policy Engine 下发且明确属于本地范围的 Decision；涉及全局状态、资源或生命周期的判断仍交给控制面。

### 2.3 Interposer

Interposer 是端侧管理节点中统一的拦截与操作集成责任实体，位于 Agent 请求路径上，按拦截事件与控制面命令逐次处理 Agent 执行。它捕获当前 integration profile 能观察的 Agent step，识别规范化 operation 及其 `pending-result boundary`，并按本地 Policy Decision 或控制面返回结果决定暂停、阻止真实执行、继续原调用，或让调用沿原有完成路径使用已经获得语义授权的结果完成。当真实 operation 被执行时，Interposer 必须保证其 once-only，并在完成后自动发布 outcome；`PublishOutcome` 因此是内部 lifecycle semantic operation，不是日常 Agent command。

Interposer 可以执行不依赖或不改变全局状态的本地 Decision，并将输入事实、Policy 版本、Decision 结果和执行上下文记录到本地 decision log；它不能据此直接改变 Speculative Tree、Branch 生命周期、全局预算或 Runtime 状态。单次处理需要等待控制面结果时，Interposer 通过 ASC-Daemon 与控制面进行异步协作，不因此形成独立的持续控制循环。它可以由拦截器、Hook、wrapper、effect broker 或其他产品适配机制实现。论文调研中出现过 LLM 调用、工具分发、轨迹写入与调度等接缝；具体接缝由 integration profile 决定，不要求每个 profile 实现固定接口。

Interposer 的请求路径对延迟敏感，而 ASC-Daemon 还承担本地控制与事件旁路；二者责任不同。是否实现为不同进程、模块或 Hook，不在目标语义中规定。

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

ASC-Daemon 与 Interposer 逻辑上属于 Agent 接入侧。它们围绕端侧管理节点承载的 Agent 执行实体工作；在 Branch 尚未建立独立环境时，可以在共享执行环境中按各自 Branch 上下文处理事件与命令，Branch 需要独立的 Branch-local 状态时，再随该 Branch 的执行活动进入 Runtime 建立的环境中。具体承载方式由实现决定，但必须保持各 Branch 管理上下文的独立性。

Task 绑定 Policy 并建立受管上下文时，Policy Engine 可以把已经解析和校验的 Policy 执行表示注入对应的 Agent 接入侧。接入侧在后续本地执行符合条件的 Decision，不需要为每次 Decision 重新获取或解析 Policy；执行结果和 decision log 仍通过 Agent Control Gateway 返回控制面。

Agent 默认可以不感知投机；可选的显式协作入口可以让 Agent 参与候选创建或裁决，但仍受相同身份、single-winner、效果与 lifecycle 约束。端用户只通过当前权威路径提供输入，不进入内部 Branch lifecycle control surface。

## 5. 固定约束与未定实现

固定约束包括：

- 所有 integration profile 进入同一 semantic API；
- Branch context 由本地接入边界绑定，不由 Agent Control Gateway 猜测；
- Policy 由控制面统一加载、解析、校验和版本管理；不依赖或不改变全局状态的 Decision 可以通过已解析的执行表示注入接入侧执行，并记录本地 decision log；
- 控制面不重复执行已满足本地范围条件的 Decision，但仍接收其结果和日志；
- Agent 接入侧不拥有控制面或后端权威状态；
- Agent 接入侧负责对应 Agent 的本地执行管理和授权范围内的本地 Decision；涉及全局控制状态或生命周期权威的决策必须交由控制面；
- 当前支持具备足够拦截点或 Hook 的特定 Agent product，不承诺完全无修改且无需了解产品内部实现的通用接入；
- Agent 接入侧必须在可观察的 Branch-local 变化发生前识别该变化并请求 lazy 创建独立环境；无法确认操作是否会产生需要隔离的变化时，按需要建立独立环境处理；
- Agent 接入侧在识别到共享、外部或 Policy 要求纳管的效果时，必须在效果发生前将请求交给控制面 Effect Coordinator；
- 真实 operation 保持 once-only，outcome 自动归入对应的 Agent 执行路径；
- Agent-facing 组件不能绕过控制面或直接控制 Runtime。

尚未确定的实现包括 transport、请求与事件 schema、进程/模块拆分、Interposer 的具体拦截面，以及不同 Agent product 的接入方式。这些内容收敛后应写入 `docs/development/` 或 `docs/usage/`。

## 相关文档

- [AScout 架构](../architecture.md)：四侧调用路径与责任边界
- [Branch 语义设计](branch-semantics.md)：Agent-facing operation 所受的 lifecycle 约束
- [控制面设计](control-plane.md)：Agent Control Gateway 后的控制路径
- [Agent 接入状态](../usage/agent-guide.md)：当前实现与可用性
