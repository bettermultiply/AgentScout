# Agent 接入侧设计

本文展开 [AScout 架构](../architecture.md) 中的 Agent 接入侧，定义 Agent 或 Agent product 如何进入 AScout 的受控路径，以及接入组件之间的责任边界。

## 1. 总体设计

Agent 接入侧是围绕 Agent 执行环境提供本地管理的责任边界，不另设“端侧管理节点”这一并列组件。它负责管理由 Agent 主进程及其 sub-agent 组成的执行实体，以及这些进程产生的执行活动。一个执行环境可以建立相应的接入侧承载，环境内的每个 Agent 在启动时接入该侧，并持有独立的 Agent 执行上下文；每个上下文再通过 Branch 上下文区分不同的逻辑执行延续，并依据 Agent、Branch、operation 归属和 capability 处理事件与控制命令。

Agent 接入侧与控制面共同完成投机执行：它观察和拦截 Agent 行为，在本侧处理属于本地范围的 Policy Decision，并将依赖全局控制状态或生命周期的决策交由控制面。多个 Branch 可以暂时共享同一个执行环境和接入侧承载；当 Branch 需要独立的 Branch-local 状态时，其管理上下文随执行活动进入相应的独立环境。

### 1.1 职责

Agent 接入侧在 Agent 执行过程中落实 Policy Decision 对投机执行流程的决策。对于由 Policy Engine 编译并交付的 Decision，接入侧结合当前 Agent 执行上下文和可观察事实，对属于本地范围的 Decision 独立作出判断，并向控制面同步 Decision 内容、输入事实、结果和 log 信息。

对于依赖全局控制状态、资源或 Branch 生命周期的 Decision，Agent 接入侧将相关事实和请求提交控制面，由控制面完成全局判断、生命周期编排和后续处理。Agent 接入侧接收控制面返回的结果和命令，并据此继续处理对应的 Agent 执行；相关事实、Decision、结果和事件通过 Agent 接入侧与控制面之间的双向通道传递。

Agent 接入侧负责观察和拦截 Agent 的执行行为，识别 operation 的开始、`pending-result boundary`、完成和 outcome，并处理 Agent 主进程及其 sub-agent 产生的可观察执行活动。它根据本地 Decision 或控制面返回结果，允许真实 operation 继续、暂停或阻止真实执行、等待后续结果，或者让当前调用沿 Agent 原有的完成路径使用已经获得语义授权的结果完成。

Agent 接入侧负责在 Branch-local 状态或受纳管 Effect 发生前识别相应边界，并保持 Branch 管理上下文、事件和控制命令的正确归属。多个 Branch 在尚未需要独立 Branch-local 状态时，可以在共享执行环境中承载，各 Branch 仍保持独立的管理上下文。当某个 Branch 需要独立的 Branch-local 状态时，Agent 接入侧向控制面请求建立相应的独立执行环境，该 Branch 的管理上下文随其执行活动进入能够提供相应隔离保证的环境；具体承载方式可以复用、迁移或重新建立。识别到共享、外部或 Policy 要求纳管的 Effect 时，Agent 接入侧在 Effect 发生前将相关请求交给控制面处理。真实 operation 保持 once-only，完成后的 outcome 自动归属对应 Branch 并发布到控制路径。

## 2. 接入组件

### 2.1 Product Integration

Product Integration 是面向具体 Agent product 的产品适配组件，负责依据接入约定，将 Agent 提供的执行点连接到 Agent 接入侧。Agent 在启动时通过 Product Integration 接入当前执行环境中的 Agent 接入侧；Product Integration 以尽可能小的产品改造接入 Agent 的内部执行结构，使 Agent 接入侧能够观察目标执行活动，并在真实 operation 开始前暂停或阻止其执行。

当 Agent 即将执行可管理步骤时，Product Integration 将该步骤交给 Agent 接入侧，并等待 Agent 接入侧返回处置结果。返回继续时，Product Integration 允许产品按原有路径执行该步骤；返回跳过时，Product Integration 不执行原步骤，并在需要时让当前调用沿产品原有的完成路径使用 Agent 接入侧提供的结果结束。暂停通过等待处置结果体现，不需要额外的暂停机制。

Product Integration 从被拦截步骤的原调用返回路径自动取得成功、失败或取消结果，并将其交给 Agent 接入侧形成 outcome。它不修改 Agent 的业务逻辑，也不要求 Agent 主动调用 AScout API；具体产品适配可以了解其内部执行结构，但应将改造限制在观察和控制所需的接入范围内。

### 2.2 ASC-Proxy

ASC-Proxy 是 Agent 接入侧与控制面之间的受管通信代理。它可以由控制面随 Agent 一起启动的进程内组件、本地 IPC 客户端、网络客户端或独立进程实现，不要求用户手工启动或直接管理。它与控制面建立通信会话，并在会话建立过程中确认通信双方的身份；在后续通信中，它通过通信凭证证明消息来自对应的 Agent 接入侧，避免信息伪造。

通信会话建立后，Agent 接入侧通过 ASC-Proxy 向控制面提交 Agent 执行事实、控制请求、本地 Decision 结果和 outcome；控制面通过同一会话向 Agent 接入侧返回命令、结果和事件。ASC-Proxy 负责消息的发送、接收和传递，不参与 Agent 步骤的执行、Decision 的判断或 Branch 生命周期的处理。正式用户流程中，控制面负责启动和回收该 Agent 及其接入会话，用户不直接管理 ASC-Proxy。

### 2.3 Interposer

Interposer 是 Agent 接入侧中介入 Agent 执行接缝并落实本地 AScout 语义的责任实体。Agent 即将执行可管理步骤时，Product Integration 将步骤交给 Interposer；Interposer 从拦截点提取步骤事实，识别 operation 的开始、`pending-result boundary` 以及可能产生的 Branch-local 状态和 Effect，并暂停当前步骤，等待本地 Decision 或控制面返回处置结果。普通步骤根据处置结果沿原有路径继续执行，不需要进入投机处理。

属于本地范围的 Decision 由 Interposer 结合当前执行事实直接判断和执行；依赖全局控制状态、资源或 Branch lifecycle 的 Decision，则由 Interposer 通过 ASC-Proxy 将相关事实和请求提交控制面，并等待控制面返回结果或命令。

Interposer 根据处置结果允许真实 operation 继续，或跳过真实 operation。继续时，Product Integration 按产品原有路径执行步骤；跳过时，真实 operation 不执行，并在需要时让当前调用沿产品原有完成路径使用已经获得语义授权的结果结束。Interposer 可以暂停一次 Agent 步骤，并在本地 Decision 或控制面结果返回后继续处理。

Interposer 还负责在 Branch-local 状态或受纳管 Effect 发生前识别相应边界。需要独立 Branch-local 状态时，它通过 ASC-Proxy 向控制面请求建立相应的独立执行环境；识别到共享、外部或 Policy 要求纳管的 Effect 时，它在 Effect 发生前将相关请求提交控制面。真实 operation 保持 once-only；继续执行时，Interposer 从原调用返回路径取得成功、失败或取消结果，将 outcome 归属对应 Branch 并通过 ASC-Proxy 提交控制面。Interposer 同时记录本地 Decision 的输入事实、求值内容、Policy 版本、结果和执行上下文。

## 3. Agent 执行接入路径

用户交互入口与后端 Agent 执行实体分离：用户始终使用同一 AScoutAgent 视图，接入实现依据控制面已确认的权威执行处理输入、输出和中断路由。Promote 不要求用户重新连接、加载另一个 session 或等待候选会话结束，也不为路由变化取消或重启候选正在进行的执行。已展示内容不可撤回，未获接受的候选内容不得作为正式回复提前发布；切换时必须保持输入归属及输出顺序，旧主路径失去发布权后仍按生命周期规则停止和清理。运行能力不满足时应拒绝该接替路径，不能以路由切换伪造接受或效果处理完成。

用户可通过独立的只读观察查看候选状态及内容，内容必须明确标记为未验证并与主对话分开。接入侧按 Branch 报告实际模型、工具和执行活动；控制面提供预测、比较、处置与生命周期事实。界面应区分请求已发送、实际开始、执行结束和清理确认，不能把未知状态显示为已完成。只读观察不使候选获得用户对话或裁决权限，也不把其内容注入其他 Branch 的上下文。

Agent 按照自身原有流程执行步骤。当 Agent 即将执行处于接入范围内的步骤时，Product Integration 将该步骤交给 Interposer。Interposer 根据本地 Decision 或控制面返回的处置结果处理该步骤；需要控制面参与时，Interposer 通过 ASC-Proxy 传递相关事实和请求，并接收控制面返回的结果或命令。Product Integration 根据 Interposer 的处置结果让原步骤继续执行，或跳过真实执行并沿 Agent 原有的完成路径结束当前调用。

真实步骤继续执行时，Product Integration 从原调用返回路径取得结果并交给 Interposer；Interposer 将其作为 outcome 归属对应 Branch，并通过 ASC-Proxy 提交控制面。Agent 主进程及其 sub-agent 产生的可观察步骤都经过同一处理路径，因此 AScout 介入的是步骤执行边界，而不改变 Agent 原有的执行流程。

## 4. 部署与生命周期

Agent 接入侧可以在执行环境中按需要建立其本地承载，并由 ASC-Proxy 与控制面建立会话。环境内的每个 Agent 在启动时通过 Product Integration 接入 Agent 接入侧，并持有独立的 Agent 执行上下文。多个 Agent 及其 Branch 可以暂时共享同一个执行环境和接入侧承载；当某个 Branch 需要独立的 Branch-local 状态时，控制面通过 Runtime Adapter 建立相应的独立执行环境，并使该 Branch 的执行活动和管理上下文进入该环境。具体承载方式由实现决定，但必须保持各 Agent 和 Branch 上下文的独立性。

Task 绑定 Policy 并建立受管上下文时，控制面可以从 Policy Engine 查询已经编译的本地 Decision，并经 Agent Control Gateway 交付给对应的 Agent 接入侧。接入侧反序列化后对这些 Decision 求值，不需要为每次 Decision 重新获取或解析 Policy；结果和 decision log 仍通过 ASC-Proxy 与 Agent Control Gateway 返回控制面。接入侧不能自行加载、编译或修改 Policy。

## 相关文档

- [AScout 架构](../architecture.md)：四侧调用路径与责任边界
- [Branch 语义设计](branch-semantics.md)：Agent-facing operation 所受的 lifecycle 约束
- [控制面设计](control-plane.md)：Agent Control Gateway 后的控制路径
- [Agent 接入状态](../usage/agent-guide.md)：当前实现与可用性
