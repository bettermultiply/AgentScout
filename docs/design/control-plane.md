# 控制面设计

控制面的主要目的是协调整个投机执行过程，并为 Policy 和 Runtime Adapter 的接入与运行提供协调和验证，避免因语义不匹配而无法正确执行。

## 1. 总体设计

控制面是抽象责任模型中 Mechanism 的实现主体，也是 Speculation Contract 的实现方。其主要目的是协调整个投机执行过程，并为 Policy 与 Runtime Adapter 的接入和运行提供协调与验证，避免策略意图与后端能力之间因语义不匹配而无法正确执行。

控制面为每个 task 维护一棵完整的 Speculative Tree 控制面视图。对外这棵树以单一 Agent 的形式呈现给用户，对内我们将这棵树视作整个投机执行生命周期中由多个子树组成的派生嵌套关系。

控制面的工作方式是一条贯穿 Policy、控制结构与外部 Runtime 的闭环，Policy 的表达与 Runtime Adapter 的接入都经由控制面获得一致的校验与编排。

控制面先通过 Policy Engine 解释并执行组成 Policy 的多个 Decision，在运行期针对每个阶段作出实时的决策。得到合理决策后，再把决策编排为受管语义操作；需要 Runtime 参与的操作经 Runtime Adapter 请求外部 Runtime 执行，Effect Coordinator 插件可以在控制面内直接处理受管效果并经 Agent Control Gateway 返回结果。控制面收到 Runtime 返回的结果或事件后，解析并校验该操作是否确实按语义完成，再据实更新 Speculative Tree 的控制面视图。控制面还记录运行事实，操作结果用于确认受管操作是否按语义完成，结果与事件记录用于状态对齐、问题定位和审计校验。控制面不定义或直接触碰 Runtime 内部状态，只经 Runtime Adapter 交换需要后端参与的 typed operation 与 event。

这些校验与编排都必须落在 Policy 无法绕过的固定语义边界之内。该边界的清单与说明见 [Policy Decisions](policy-decisions.md) §6；控制面负责在运行期使这些语义被一致地落实。

### 1.1 职责

控制面对 Policy 负有两类责任。

第一种是在 Policy 接入时，提供对 Policy Decision 的解析和校验责任。Task 准入时，其绑定的 Policy 与接入的 Runtime 已经确定；此时要校验 Policy 的合法性，检查其取值是否为控制面可解析、且在当前 Contract 下允许的值。校验通过后，把每个 Decision 解析并注入控制面运行期，将其原始表达转换为运行期可直接使用、无需每次重新解释的决策依据。这种校验用于避免用户在不了解 Runtime 能力时制订 Policy 而造成的语义错配，也能发现 Policy 制订过程中的错误。Policy 进入控制面后主要由 Policy Engine 处理（见 §2.2）。

第二种是在 Task 执行过程中，控制面需要在投机执行的每个阶段及时触发流程并参考 Policy，根据其表达的策略意图作出具体的决策，并转化成一系列针对投机分支（Branch）与环境状态的受控操作。这是因为 Policy 通常只表达策略意图、不关心机制细节；而决策需要何时发生、所需数据如何获取、决策之后需要执行哪些操作来实现目标、哪些对象状态在何时发生变化，都由控制面完成，以减轻 Policy 设计的负担。

## 2. 组件与状态

控制面由若干固定职责的组件构成，包括 Agent Control Gateway、Policy Engine、Speculation Coordinator、Effect Coordinator 与 Ledger。它们分别承担不同的职责，围绕 Speculative Tree 的控制面视图协作，并维护各自需要的控制状态。

### 2.1 Agent Control Gateway

Agent Control Gateway（以下简称 ACGateway）是控制面面向 Agent 接入侧的受管通信边界，负责统一管理控制面与接入侧之间的通信。它接收某个 Task 的开始请求，并在控制面协调下建立逻辑 Branch Node；当该 Branch 需要独立执行环境时，Runtime 创建对应的 Branch 执行环境，ACGateway 再在该环境内创建并启动对应的 Agent 接入侧，二者共同形成完整的物理执行节点。尚未建立独立环境的 Branch 可以先由受控接入侧在共享执行视图中推进。

Task 运行期间，它维护与 Agent 接入侧之间的通信会话，双向传递控制面与接入侧所需的信息和命令，包括 Policy dialect、本地 Decision 结果、decision log、运行事件和受管控制请求，并进行入口级的上下文、权限和格式检查。除通信会话所需的暂时状态外，它不持有 Speculative Tree、Branch 生命周期、调度、预算、Policy 或 Runtime 执行状态。

### 2.2 Policy Engine

Policy Engine 是可替换 Policy 进入控制面的固定入口。它在 Policy 加载时统一解析 Policy 中的 Decision 及相关逻辑，检查这些决策以及决策与决策之间的组合在当前 Contract 下是否合法，再结合 Runtime Adapter 已报告的能力判断策略能否在当前接入的后端上执行。Policy 声明超出 AScout 固定语义和 Runtime 能力范围时，Policy Engine 拒绝该决策并按既定语义报告降级。

解析结果形成控制面运行期可以直接使用的 Policy dialect；Policy dialect 是 Policy 经解析和校验后形成的、供控制面及 Agent 接入侧执行 Decision 的运行期表示。其中不依赖或不改变全局状态的 Decision，可以随 Policy dialect 在 Agent 接入侧创建时直接注入，并由接入侧在本地执行，以减少高频路径上的通信延迟。接入侧应记录这些本地 Decision 的输入事实、Policy 版本、结果和执行上下文，并将结果与日志返回控制面。

如果某项 Decision 依赖或改变全局状态，则不属于可下沉到接入侧的本地 Decision，其判断和后续操作仍由控制面完成。ACGateway 接收 Agent 接入侧的请求、本地 Decision 结果和日志后，将其交给 Speculation Coordinator；Coordinator 根据事件和全局视图判断受影响的 Edge，向 Policy Engine 请求执行相应的全局 Decision，再由各专业组件形成和处理操作意图。

### 2.3 Speculation Coordinator

Speculation Coordinator 是控制面的全局投机视图与控制循环的协调核心。它维护每个 Task 的 Speculative Tree 视图，以及 Edge、Branch、Proposal、`AssumptionSet`、待选机会、调度状态和逻辑视图与 Runtime 实际状态之间的对应关系。它接收 Agent 接入侧的事件，以及 Runtime 经 Runtime Adapter 规范化后返回的结果和事件，据此更新全局视图，识别受影响的 Edge/Branch，并判断当前被激活的 Policy Decision。

Speculation Coordinator 通过 Policy Engine 执行需要全局视图的 Decision，并接收 Agent 接入侧已经完成的、不依赖或不改变全局状态的本地 Decision 结果；对已经满足本地范围条件的 Decision 不重复执行。它根据 Policy Decision 和当前全局视图协调各专业组件形成操作意图，将这些意图编排为 semantic operation：需要 Runtime 参与的操作经统一的控制面操作出口交给 Runtime Adapter，Effect Coordinator 在控制面内直接处理的效果则通过 Agent Control Gateway 返回结果，并根据结果或事件继续更新全局视图。

它同时负责维护通过 `admit` 但尚未启动的待选机会及其有效性，向 Policy 提供同一 task 内的待选、运行和暂停工作，以及预算、资源压力、等待时间、依赖和成本等事实；接收 `select` 对当前启动机会的选择和 `rebalance` 对已启动工作的暂停、恢复或抢占意图，并在 Contract、资源与正确性硬约束内执行。逻辑 Branch 可以先推进而不立即创建独立 Runtime 环境；当接入侧识别出即将发生需要独立 Branch-local 状态的变化时，Coordinator 协调 Runtime 建立独立环境，并在收到实际完成事件后把逻辑状态标记为已对齐。独立环境尚未建立完成时，Coordinator 不把预期状态当作 Runtime 已确认的事实。未被选择的有效机会继续保留；pending result 到达、任务结束、祖先依赖被否定或其他必要前提失效时，Coordinator 将其移出集合。预算计数也由 Coordinator 维护，因为选择与执行都需要同步读取预算余量。

Coordinator 不使用固定价值公式替代 Policy 的跨 Edge 权衡，也不在 `select` 前重复 `admit` 精判。启动或调整运行状态前的有效性、后端能力与硬约束复核属于机制校验；条件暂时不满足时可以延后执行 Policy 的意图。`rebalance` 的抢占结果以 `preempted` 原因进入 `dispose`，而不是由 Coordinator 自行决定清理方式。

### 2.4 Effect Coordinator

Effect Coordinator 负责控制面内共享、外部及 Policy 要求纳管的效果处理。Agent 接入侧在效果真正发生前识别受管的命令或 URL，将请求交给 Effect Coordinator；Effect Coordinator 根据已注册的服务效果插件、效果分类与状态域，结合 Policy、Contract 与 Adapter 声明的后端能力形成处理结果或计划。插件可以在控制面内直接生成明确标记并记录来源的模拟响应，也可以拒绝或阻塞请求；只有需要 Runtime 提供暂存、代理、转换、提交或清理机制时，才经 Runtime Adapter 请求后端执行。Effect Coordinator 不负责普通 Branch-local 操作，也不持有真实的暂存效果、obligation 或隔离状态——这些状态位于外部 Runtime，但其内部组织不由 AScout 定义。

效果插件在控制面启动或 Policy 加载阶段注册，并在对应 Task 使用的 Policy 生效时确定，不在运行中的 Task 内热替换。插件可以根据 Task、Branch 和控制面效果状态执行去重、审计及跨 Branch 协调；其生成的模拟结果必须明确标记为模拟结果并记录生成来源，按既定的假设与验证语义处理。效果阻塞发生在效果边界：投机执行遇到不能安全暂存或释放的效果时停在门口，保留此前的前置进展，等待裁决。

### 2.5 Ledger

Ledger 以异步旁路记录控制面需要保留的运行事实，包括 Policy 决策、Branch 与 Proposal 状态变化、验证依据、效果与 obligation 处理结果、资源使用、取消与清理结果、Adapter 映射结果与 Runtime 失败。它用于状态对齐、问题定位、shadow/replay 和契约校验；它不从记录中重新决定生命周期，也不代替 Runtime 持有执行状态。

## 3. 失败处理：回到"Branch 从未创建"

对尚未被接受的投机 Branch，控制面在失败时把它视作在 Speculative Tree 中从未被创建：不重放该操作，不保留它的部分结果作为成功。被接受的 Branch 已完成使命，不适用这一语义。

落到机制上，控制面沿固定状态入口撤销对应 Branch 的控制状态，并经 Runtime Adapter 请求 Runtime 停止对应执行、处理资源与受管效果。清理具体如何编排由 Runtime 与 Effect backend 落实。Salvage 是可选的额外能力：若某段投机工作产生了可移交权威路径的残值，可以在失败默认之上回收；它不是失败处理的必要条件。

## 4. 与 Runtime Adapter 的衔接

控制面只能经 Runtime Adapter 访问 Runtime。Adapter 把控制面的语义操作翻译成后端可执行的组合调用，把后端能力与结果翻译回控制面可解释的能力事实和规范化事件。Adapter 的实现不受 AScout 掌控，因此控制面不假定 Adapter 自觉正确；它通过接入校验确认 Adapter 能表达基本语义，并通过运行期结果和事件确认操作是否真实完成。接入与运行期的约束见 [Speculation Contract](speculation-contract.md)。
