# 控制面设计

控制面的主要目的是协调整个投机执行过程，并为 Policy 和 Runtime Adapter 的接入与运行提供协调和验证，避免因语义不匹配而无法正确执行。

## 1. 总体设计

控制面是抽象责任模型中 Mechanism 的实现主体，也是 Speculation Contract 的实现方。其主要目的是协调整个投机执行过程，并为 Policy 与 Runtime Adapter 的接入和运行提供协调与验证，避免策略意图与后端能力之间因语义不匹配而无法正确执行。

控制面为每个 task 维护一棵完整的 Speculative Tree 控制面视图。对外这棵树以单一 Agent 的形式呈现给用户，对内我们将这棵树视作整个投机执行生命周期中由多个子树组成的派生嵌套关系。这个单一 Agent 视图和控制域称为 `AScoutAgent`。用户请求首先进入控制面；控制面负责建立 Agent 控制上下文并启动对应的 Agent 与接入侧，不要求用户手工启动具体 Agent 产品或接入组件。

控制面的工作方式是一条贯穿 Policy、控制结构与外部 Runtime 的闭环，Policy 的表达与 Runtime Adapter 的接入都经由控制面获得一致的校验与编排。

控制面先通过 Policy Engine 解释并执行组成 Policy 的多个 Decision，在运行期针对每个阶段作出实时的决策。得到合理决策后，再把决策编排为受管语义操作；需要 Runtime 参与的环境操作经 Runtime Adapter 请求外部 Runtime 执行，外部效果由 Effect Coordinator 在控制面内直接处理并经 Agent Control Gateway 返回结果。控制面收到 Runtime 返回的结果或事件后，解析并校验该操作是否确实按语义完成，再据实更新 Speculative Tree 的控制面视图。Candidate 的 Promote/Discard 验证必须使用 Parent 真实 operation 产生的 outcome 与 Candidate hypothesis 的比较，不得使用 Candidate 自己的生成结果替代 Parent outcome。控制面还记录运行事实，操作结果用于确认受管操作是否按语义完成，结果与事件记录用于状态对齐、问题定位和审计校验。控制面不定义或直接触碰 Runtime 内部状态，只经 Runtime Adapter 交换需要后端参与的 typed operation 与 event。

这些校验与编排都必须落在 Policy 无法绕过的固定语义边界之内。该边界的清单与说明见 [Policy Decisions](policy-decisions.md) §6；控制面负责在运行期使这些语义被一致地落实。

### 1.1 职责

控制面对 Policy 负有两类责任。

第一种是在 Policy 接入时，提供对 Policy Decision 的解析和校验责任。Task 准入时，其绑定的 Policy 与接入的 Runtime 已经确定；此时要校验 Policy 的合法性，检查其取值是否为控制面可解析、且在当前 Contract 下允许的值。校验通过后，把每个 Decision 解析并注入控制面运行期，将其原始表达转换为运行期可直接使用、无需每次重新解释的决策依据。这种校验用于避免用户在不了解 Runtime 能力时制订 Policy 而造成的语义错配，也能发现 Policy 制订过程中的错误。Policy 进入控制面后主要由 Policy Engine 处理（见 §2.2）。

第二种是在 Task 执行过程中，控制面需要在投机执行的每个阶段及时触发流程并参考 Policy，根据其表达的策略意图作出具体的决策，并转化成一系列针对投机分支（Branch）与环境状态的受控操作。这是因为 Policy 通常只表达策略意图、不关心机制细节；而决策需要何时发生、所需数据如何获取、决策之后需要执行哪些操作来实现目标、哪些对象状态在何时发生变化，都由控制面完成，以减轻 Policy 设计的负担。

## 2. 组件与状态

控制面由若干固定职责的组件构成，包括 Agent Control Gateway、Adapter Registry、Policy Engine、Speculation Coordinator、Effect Coordinator 与 Ledger。它们分别承担不同的职责，围绕 Speculative Tree 的控制面视图协作，并维护各自需要的控制状态。

### 2.1 Agent Control Gateway

Agent Control Gateway（以下简称 ACGateway）是控制面面向 Agent 接入侧的受管通信边界，负责统一管理控制面与接入侧之间的通信。它接收某个 Task 的开始请求，并在控制面协调下建立逻辑 Branch Root；当该 Branch 需要独立执行环境时，Runtime 创建对应的 Branch 执行环境，ACGateway 再在该环境内创建并启动对应的 Agent 接入侧，二者共同形成完整的物理执行节点。尚未建立独立环境的 Branch 可以先由受控接入侧在共享执行视图中推进。

Task 运行期间，它维护与 Agent 接入侧之间的通信会话，双向传递控制面与接入侧所需的信息和命令，包括已编译的本地 Decision、本地 Decision 结果、decision log、运行事件和受管控制请求，并进行入口级的上下文、权限和格式检查。除通信会话所需的暂时状态外，它不持有 Speculative Tree、Branch 生命周期、调度、预算、Policy 或 Runtime 执行状态。

### 2.2 Policy Engine

Policy Engine 是可替换 Policy 进入控制面的固定入口，只有两项职责：编译 Policy，以及根据规定输入作出本次 Policy Decision。解析、展开、检查和形成编译结果都属于编译过程，不另列为组件职责。

编译结果可以按单个 Decision 查询，并且必须可序列化和反序列化。不依赖或不改变全局状态的 Decision，可以由控制面查询后经 Agent Control Gateway 交付给 Agent 接入侧求值。Policy Engine 不决定运行位置，也不负责交付。

需要全局控制状态的 Decision 由 Policy Engine 根据控制面组件按输入约定提供的数据求值，不向算法开放任意访问内部状态的入口。Policy Engine 只返回策略选择；相应状态的拥有组件负责复核、落实选择并更新实际状态。完整边界见 [Policy Engine 设计](control-plane/policy-engine.md)。

### 2.3 Adapter Registry

Adapter Registry 是 Runtime Adapter 进入控制面的固定入口，负责 Adapter 的接入登记、契约兼容性检查、能力声明校验，以及建立验证环境、执行验证场景和观察结果。接入验证应尽可能通过真实环境操作完成；运行期持续验证用于发现接入验证遗漏和实际偏差，不代替接入验证。

Adapter Registry 维护 Adapter 的接入状态、契约版本、能力声明快照和校验结果，但不持有 Speculative Tree、Branch 生命周期或 Runtime 的执行状态。它向 Policy Engine 发布能力事实及其验证状态。未验证能力默认视为不可用，只有部署方主动确认后才能在指定部署范围内使用；验证失败不能通过确认绕过。具体的 operation/event 契约与校验标准见 [Runtime Adapter 契约设计](runtime-adapter-contract.md)。

Adapter 配置在运行期不允许变化。若运行期发现 Adapter 的实际能力不符合声明，Registry 将其降级到仍然经过验证的范围，并让全部任务使用降级后的能力视图；依赖失效能力的投机活动停止并清理，不依赖的活动可以继续。重新验证通过后，Adapter 才能恢复新的能力范围。

Adapter Registry 不直接编排 Branch 生命周期，也不替代 Speculation Coordinator 解释运行期事件。Speculation Coordinator 只使用 Registry 提供的接入结果和能力信息协调 Task 运行。

### 2.4 Speculation Coordinator

Speculation Coordinator 是控制面的全局投机控制器，持有全局投机视图并驱动控制循环。它维护每个 Task 的 Speculative Tree 视图，以及 Edge、Branch、Proposal、`AssumptionSet`、待选机会、调度状态和逻辑视图与 Runtime 实际状态之间的对应关系。它接收 Agent 接入侧的事件，以及 Runtime 经 Runtime Adapter 规范化后返回的结果和事件，据此更新全局视图，识别受影响的 Edge/Branch，并判断当前被激活的 Policy Decision。

Speculation Coordinator 通过 Policy Engine 对需要全局视图的 Decision 求值，并接收 Agent 接入侧已经完成的、不依赖或不改变全局状态的本地 Decision 结果；对已经满足本地范围条件的 Decision 不重复求值。它根据 Policy Decision 和当前全局视图协调各专业组件形成操作意图，将这些意图编排为 semantic operation：需要 Runtime 参与的操作经统一的控制面操作出口交给 Runtime Adapter，Effect Coordinator 在控制面内直接处理的效果则通过 Agent Control Gateway 返回结果，并根据结果或事件继续更新全局视图。

它同时负责维护通过 `admit` 但尚未启动的待选机会及其有效性，向 Policy 提供同一 task 内的待选、运行和暂停工作，以及预算、资源压力、等待时间、依赖和成本等事实；接收 `select` 对当前启动机会的选择和 `rebalance` 对已启动工作的暂停、恢复或抢占意图，并在 Contract、资源与正确性硬约束内执行。逻辑 Branch 可以先推进而不立即创建独立 Runtime 环境；当接入侧识别出即将发生需要独立 Branch-local 状态的变化时，Coordinator 经 Adapter 请求 Runtime 建立独立环境，并在收到实际完成事件后把逻辑状态标记为已对齐。独立环境尚未建立完成时，Coordinator 不把预期状态当作 Runtime 已确认的事实。未被选择的有效机会继续保留；pending result 到达、任务结束、祖先依赖被否定或其他必要前提失效时，Coordinator 将其移出集合。预算计数也由 Coordinator 维护，因为选择与执行都需要同步读取预算余量。

Coordinator 不使用固定价值公式替代 Policy 的跨 Edge 权衡，也不在 `select` 前重复 `admit` 精判。启动或调整运行状态前的有效性、后端能力与硬约束复核属于机制校验；条件暂时不满足时可以延后执行 Policy 的意图。`rebalance` 的抢占结果以 `preempted` 原因进入 `dispose`，而不是由 Coordinator 自行决定清理方式。

### 2.5 Effect Coordinator

Effect Coordinator 负责控制面内共享、外部及 Policy 要求纳管的外部效果处理，是外部效果的统一实际执行者。重点纳管由 Agent 造成且可能影响全局的外部效果，例如调用外部 API。Agent 接入侧在效果真正发生前识别受管的命令或 URL，将请求交给 Effect Coordinator；Effect Coordinator 根据已注册的服务效果插件、效果分类与状态域，结合 Policy 和 Contract 形成处理结果、执行或处理计划。插件可以在控制面内直接生成明确标记并记录来源的模拟响应，也可以拒绝、阻塞、暂存、转换、提交、取消、清理或补偿其负责的效果。Effect Coordinator 不负责普通 Branch-local 操作；Runtime 只负责 Agent 执行所需的隔离环境及环境级执行状态，不承担外部效果的实际执行责任。

效果插件在控制面启动或 Policy 加载阶段注册，并在对应 Task 使用的 Policy 生效时确定，不在运行中的 Task 内热替换。插件可以根据 Task、Branch 和控制面效果状态执行去重、审计及跨 Branch 协调；其生成的模拟结果必须明确标记为模拟结果并记录生成来源，按既定的假设与验证语义处理。效果阻塞发生在效果边界：投机执行遇到不能安全暂存或释放的效果时停在门口，保留此前的前置进展，等待裁决。

### 2.6 Ledger

Ledger 以异步旁路记录控制面需要保留的运行事实，包括 Policy 决策、Branch 与 Proposal 状态变化、验证依据、效果与 obligation 处理结果、资源使用、取消与清理结果、Adapter 映射结果与 Runtime 失败。它用于状态对齐、问题定位、shadow/replay 和契约校验；它不从记录中重新决定生命周期，也不代替 Runtime 持有执行状态。

## 3. 失败处理：回到"Branch 从未创建"

对尚未被接受的投机 Branch，控制面在失败时把它视作在 Speculative Tree 中从未被创建：不重放该操作，不保留它的部分结果作为成功。被接受的 Branch 已完成使命，不适用这一语义。

落到机制上，控制面沿固定状态入口撤销对应 Branch 的控制状态，由 Effect Coordinator 处理受管效果的取消、清理与必要补偿，并经 Runtime Adapter 请求 Runtime 停止对应执行和处理环境资源。Salvage 是可选的额外能力：若某段投机工作产生了可移交权威路径的残值，可以在失败默认之上回收；它不是失败处理的必要条件。

## 4. 与 Runtime Adapter 的衔接

控制面只能经 Runtime Adapter 访问 Runtime。Adapter 把控制面的语义操作翻译成后端可执行的组合调用，把后端能力与结果翻译回控制面可解释的能力事实和规范化事件。控制面请求的是抽象的独立 Candidate 执行环境；独立目录、虚拟机或其他承载方式只是 Adapter 的实现。Adapter 的实现不受 AScout 掌控，因此控制面不假定 Adapter 自觉正确；Adapter Registry 通过接入校验确认 Adapter 能表达基本语义，运行期控制路径通过结果和事件确认操作是否真实完成。具体约束见 [Runtime Adapter 契约设计](runtime-adapter-contract.md)。
