# 控制面设计

控制面的主要目的是协调整个投机执行过程，并为 Policy 和 Runtime 的接入与运行提供协调和验证，避免因语义不匹配而无法正确执行。Runtime Adapter 是 Runtime 与控制面之间的接入边界；Runtime 本身负责动作承载、环境能力和效果边界，控制面不把 Adapter 或 Effect Coordinator 视作另一套 Runtime。

## 1. 总体设计

执行事实分为 Agent action、Agent/Branch 元事件、Agent Adapter 命令回执和 Runtime 事实。控制面只据对应来源更新状态；Runtime 失败不能伪装成 Agent 已报告失败，Agent 清理回执也不代表环境清理。action 关联 task、Branch 和 action identity，结果必须与开始配对；Authority/Candidate 角色由 Tree 判定。完整定义见 [Agent 接入](agent-integration.md)。

控制面是抽象责任模型中 Mechanism 的实现主体，也是 Speculation Contract 的实现方。其主要目的是协调整个投机执行过程，并为 Policy 与 Runtime Adapter 的接入和运行提供协调与验证，避免策略意图与后端能力之间因语义不匹配而无法正确执行。

控制面为每个 session 维护一个 AScoutAgent 和一棵完整的 Speculative Tree 控制面视图。task 是 session 中 Agent 的一轮工作，Agent 完成本轮工作并开始等待用户输入时完成；多轮 task 共用该控制域。对外这棵树以单一 Agent 的形式呈现给用户，对内我们将这棵树视作整个投机执行生命周期中由多个子树组成的派生嵌套关系。这个单一 Agent 视图和控制域称为 `AScoutAgent`。用户请求经 Gateway 进入相应控制逻辑；控制面负责建立 Agent 控制上下文，Runtime 负责实际启动对应的 Agent 与接入进程，不要求用户手工启动具体 Agent 产品或接入组件。

允许修改 Policy。控制面对外宣告任务成功必须等待 Runtime 报告物化成功；Runtime 报告的物化失败等同操作失败。task 完成及物化条件见 [Runtime 设计](runtime.md)。

控制面的工作方式是一条贯穿 Policy、控制结构与 Runtime 的闭环，Policy 的表达与 Runtime 的接入都经由控制面获得一致的校验与编排。

控制面通过 Policy Engine 取得各阶段的决策，再将决策编排为受管操作。环境与效果处理经 Gateway 按 Runtime Adapter 契约交付，由 Runtime 返回实际结果和事件；需要进入 Agent 工具通道的结果经 Gateway 交付。控制面据实更新 Speculative Tree，而不直接修改 Runtime 内部状态。在等待 Parent 真实结果的投机路径中，Candidate 验证必须比较 Parent 真实 operation 的 outcome 与 Candidate hypothesis，不能用 Candidate 自己生成的结果替代。控制面记录决策、操作结果和事件，用于状态对齐与问题定位。

这些校验与编排都必须落在 Policy 无法绕过的固定语义边界之内。该边界的清单与说明见 [Policy Decisions](policy-decisions.md) §6；控制面负责在运行期使这些语义被一致地落实。

### 1.1 职责

控制面对 Policy 负有两类责任。

第一种是在 Policy 接入时，提供对 Policy Decision 的解析和校验责任。Policy 接入时，要结合所接入的 Runtime 校验 Policy 的合法性，检查其取值是否为控制面可解析、且在当前 Contract 下允许的值。校验通过后，把每个 Decision 解析并注入控制面运行期，将其原始表达转换为运行期可直接使用、无需每次重新解释的决策依据。这种校验用于避免用户在不了解 Runtime 能力时制订 Policy 而造成的语义错配，也能发现 Policy 制订过程中的错误。Policy 进入控制面后主要由 Policy Engine 处理（见 §2.2）。

第二种是在 Task 执行过程中，控制面需要在投机执行的每个阶段及时触发流程并参考 Policy，根据其表达的策略意图作出具体的决策，并转化成一系列针对投机分支（Branch）与环境状态的受控操作。这是因为 Policy 通常只表达策略意图、不关心机制细节；而决策需要何时发生、所需数据如何获取、决策之后需要执行哪些操作来实现目标、哪些对象状态在何时发生变化，都由控制面完成，以减轻 Policy 设计的负担。

## 2. 组件与状态

控制面由 Gateway、Policy Engine、Speculation Coordinator 与 Ledger 等组件构成。Gateway 统一承接 Agent 接入侧、UI 与 Runtime 的通信；Registry 属于 Runtime。它们围绕 Speculative Tree 的控制面视图协作；效果处理由 Runtime 内部的 Effect Coordinator 落实，控制面只保存裁决和对齐所需的效果事实。

### 2.1 Gateway

Gateway 是控制面面向 Agent 接入侧、UI 与 Runtime 的统一受管通信边界，负责通信如何接入、消息交给谁以及如何送达。消息对投机执行的变化由相应控制逻辑决定；Runtime 的消息仍遵守 Runtime Adapter 契约，不因通信统一而绕过 Adapter 或 Registry。

Gateway 的通信职责包括：管理连接、会话、握手与断线；进行入口格式、身份、上下文与权限检查；接收请求、执行事实与命令回执并转交相应处理方；交付命令、已编译的本地 Decision、结果及观察事件。Gateway 与 Runtime 采用网络通信；UI 可以与控制面一起部署，也不因此取得 Tree 或生命周期权威。消息覆盖 Agent、UI 和 Runtime 三侧，各侧既有协议和权限边界继续有效。除通信所需的暂时状态外，Gateway 不持有 Speculative Tree、Branch 生命周期、调度、预算、Policy 或 Runtime 执行状态，也不承担投机编排。控制面负责启动协调；Runtime 负责实际启动 Agent 与接入进程，Adapter 长期作为独立部署的接入边界存在。

### 2.2 Policy Engine

Policy Engine 是可替换 Policy 进入控制面的固定入口，只有两项职责：编译 Policy，以及根据规定输入作出本次 Policy Decision。解析、展开、检查和形成编译结果都属于编译过程，不另列为组件职责。

编译结果可以按单个 Decision 查询，并且必须可序列化和反序列化。不依赖或不改变全局状态的 Decision，可以由控制面查询后经 Gateway 交付给 Agent 接入侧求值。Policy Engine 不决定运行位置，也不负责交付。

需要全局控制状态的 Decision 由 Policy Engine 根据控制面组件按输入约定提供的数据求值，不向算法开放任意访问内部状态的入口。Policy Engine 只返回策略选择；相应状态的拥有组件负责复核、落实选择并更新实际状态。完整边界见 [Policy Engine 设计](control-plane/policy-engine.md)。

### 2.3 与 Runtime 内 Adapter Registry 的协作

Adapter Registry 属于 Runtime，负责 Adapter 的接入登记、契约兼容性检查和能力声明解析。首期只检查控制面当前需要的 API 是否存在且可解析，不建立验证环境、不执行验证场景，也不把 API 检查写成环境保证已经验证；主动验证与独立证据保留为后续能力。Runtime 侧职责见 [Runtime 设计](runtime.md#adapter-registry)。

Registry 维护 Adapter 的接入状态、契约版本、能力声明快照和 API 检查结果，但不持有 Speculative Tree、Branch 生命周期或 Runtime 的执行状态。它向 Policy Engine 发布能力事实及其当前检查状态；首期不能据此宣称环境保证已验证。具体的 operation/event 契约与检查标准见 [Runtime Adapter 契约设计](runtime-adapter-contract.md)。

Adapter 配置在运行期不允许变化。若运行期发现 Adapter 的实际能力不符合声明，Registry 将能力视图降级到仍然经过验证的范围；控制面让全部任务使用降级后的能力视图，协调停止并清理依赖失效能力的投机活动，不依赖的活动可以继续。重新验证通过后，Adapter 才能恢复新的能力范围。

Registry 不直接编排 Branch 生命周期，也不替代 Speculation Coordinator 解释运行期事件。Speculation Coordinator 只使用 Registry 提供的接入结果和能力信息协调 Task 运行。首期 Registry 只做声明解析和所需 API 检查；主动验证和独立验证证据属于后续能力。Adapter 不得自行填写验证状态或以自测结果自证。

### 2.4 Speculation Coordinator

Environment 创建由 Runtime 完成并返回身份。协调侧先登记无环境绑定的 Branch，再发出建立请求；派生只传 Parent 的 Environment 身份及启动参数。Runtime 返回后，协调侧把身份写入 BranchRecord 并交付 Agent 启动命令。后续操作及恢复使用该持久化绑定，不生成环境身份。创建失败但已产生环境时仍保存返回绑定，以便清理；未产生环境的 Branch 清理不调用 Runtime。

控制面围绕 Speculative Tree 组织。建立 session 的 AScoutAgent 时建立对应的树，树持有该 session 的 Chance、Branch、Proposal、`AssumptionSet`、待选机会、调度状态和未完成操作，以及逻辑状态与 Runtime 实际事实之间的对应关系。树的方法检查转换是否合法，更新状态并产生后续操作要求；策略选择由 Policy 函数计算。

Speculation Coordinator 协调树、Policy 和外部操作，可以服务多棵树。它不持有某个 session 的第二份投机状态；共享服务、连接与全局配置可以由协调侧持有。Policy 是一组决策函数，按处理点提供事实并求值，返回策略选择；其返回值须经树的当前状态检查后应用。

保存、恢复及任务结束也遵循同一所有权边界：Tree 拥有恢复暂停、继续和任务结束的控制状态及合法转换；协调侧组织 Agent 与 Runtime 的保存、加载、物化和清理顺序，恢复资料由控制面校验后发布和读取。Gateway 只交付相关请求和回执；可执行入口负责参数、装配、呈现及事件驱动，不另行维护或直接改写树的领域状态。

Speculation Coordinator 通过 Policy Engine 对需要全局视图的 Decision 求值，并接收 Agent 接入侧已经完成的、不依赖或不改变全局状态的本地 Decision 结果；对已经满足本地范围条件的 Decision 不重复求值。它根据 Policy Decision 和当前全局视图形成操作意图，经 Gateway 按 Runtime Adapter 契约交给 Runtime，并根据实际结果或事件更新全局视图；需要交付 Agent 的结果经 Gateway 返回。

每棵树维护通过 `admit` 但尚未启动的待选机会及其有效性，向 Policy 提供同一 AScoutAgent 内的待选、运行和暂停工作，以及预算、资源压力、等待时间、依赖和成本等事实；接收 `select` 对当前启动机会的选择和 `rebalance` 对已启动工作的暂停、恢复或抢占意图，并在 Contract、资源与正确性硬约束内执行。每个 Branch 在 Tree 中显式绑定独占的 `environment_id`；Candidate 创建时必须通过 Adapter 请求 Runtime 创建自己的 Environment，不复用 parent Environment。独立环境尚未建立完成时，Coordinator 不把预期状态当作 Runtime 已确认的事实。未被选择的有效机会继续保留；pending result 到达、任务结束、祖先依赖被否定或其他必要前提失效时，Coordinator 将其移出集合。task 的预算计数归对应树，协调侧读取这些事实进行求值和操作协调。

Coordinator 不使用固定价值公式替代 Policy 的跨 Chance 权衡，也不在 `select` 前重复 `admit` 精判。启动或调整运行状态前的有效性、后端能力与硬约束复核属于机制校验；条件暂时不满足时可以延后执行 Policy 的意图。`rebalance` 的抢占结果以 `preempted` 原因进入 `dispose`，而不是由 Coordinator 自行决定清理方式。

消息到达或定时器到期触发处理，同一棵树的控制事件按顺序更新树。耗时的 Runtime 操作和 Policy 求值异步完成，结果再次作为事件交给对应树；在返回期间已失效的结果不得恢复已取消的工作。事件等待和连接管理属于运行设施，不承担策略选择，也不以定期轮询驱动业务。Agent 普通执行仍由接入侧负责。

### 2.5 效果处理的衔接

控制面将 Policy 的效果处理意图和合法裁决交给 Runtime，并依据实际返回的结果更新控制状态。Effect Coordinator 属于 Runtime 的整体效果管理责任；其内部组件和部署不在这里固定。模拟结果仍必须标记来源并遵守假设与验证语义，不能以控制面记录已更新代替实际效果处理完成。具体纪律见 [效果管理](effect-management.md)。

### 2.6 Ledger

Ledger 以异步旁路记录控制面需要保留的运行事实，包括 Policy 决策、Branch 与 Proposal 状态变化、验证依据、效果与 obligation 处理结果、资源使用、取消与清理结果、Adapter 映射结果与 Runtime 失败。它用于状态对齐、问题定位、shadow/replay 和契约校验；它不从记录中重新决定生命周期，也不代替 Runtime 持有执行状态。

Ledger 是观察事件日志的唯一保存者，在追加时分配同一运行内连续递增的序号，并直接提供状态事件集和按序号补发的只读接口。读取完整历史不得阻塞追加。Tree 在创建 Branch、切换 Authority、改变生命周期和环境状态的统一入口产生事件，由协调循环取走并交给 Ledger；事件记录实际转换及其原因，保留同一次处理中连续经过的中间状态。观察日志只承担观察和事实记录，不要求据此在崩溃后重建整棵树，也不为恢复逐次记录完整树状态。

已经退出活动控制范围、移交 Ledger 的 Branch 由 Ledger 承担保存责任。写入失败时 Ledger 继续持有该记录，并报告这次运行的历史不完整；不得在返回错误的同时静默释放它，也不因此把 Branch 重新放回活动 Tree。记录成功写出的判定不包含强制 `flush`，Ledger 不对已失败的文件内容作回滚或去重承诺。

session 记录由接入侧完整持久化并汇总为单一最终权威 session，其真实结果整理规则见 [Agent 接入设计](agent-integration.md) §3。Ledger 的 transcript 事实只保存 Branch id、消息记录 id 与 role，并记录文件位置，不重复保存正文。必须先写文件、后写引用；允许文件存在未被 Ledger 引用的记录，不承诺两者原子提交。运行期多 session 汇总时，接入侧必须保持引用可定位；引用不承诺恢复完整活动 Tree，也不能把整理后的父链当作原始模型请求上下文。产品格式与读取位置提示不构成控制面裁决；内部记录引用不直接扩大只读观察接口公开范围。

## 3. 失败处理：消除失败执行的有效影响

对尚未被接受的投机 Branch，失败后对运行的影响必须等效于没有创建过：不重放该操作，不保留部分结果作为成功，并收敛副作用。但审计记录必须保留；消息旁支不能因此重新执行、获得发布权或对权威生效。被接受的 Branch 已完成使命，不适用这一失败语义。

落到机制上，控制面沿固定状态入口撤销对应 Branch 的控制状态，并通过接入边界请求 Runtime 停止执行、清理资源以及取消、清理或补偿受管效果。Salvage 是可选的额外能力：若某段投机工作产生了可移交权威路径的残值，可以在失败默认之上回收；它不是失败处理的必要条件。

## 4. 与 Runtime Adapter 的衔接

控制面通过 Runtime Adapter 这一接入边界使用 Runtime，而不依赖内部的 Overlay、容器或虚拟机细节。请求表达所需的承载与保证，不默认要求独立环境或完整复制；Runtime 可以在满足要求时共享状态。Registry 校验和运行期结果用于确认能力与实际完成情况，不能仅凭模块名称或请求已发送推断保证成立。与 Policy 的完整协调、协商及运行时检测是后续实现目标，见 [Runtime 开发说明](../development/runtime-adapter.md)。具体接入约束见 [Runtime Adapter 契约设计](runtime-adapter-contract.md)。

## 5. 只读观察

控制面对外提供受限的观察事件，不直接公开 Speculative Tree、控制命令、checkpoint 或 Runtime 内部对象。首次读取取得状态事件集及其对应的最新序号；状态事件集是从第 1 号到该序号的完整有序事件读取，同时给出由这些事件折叠出的 Chance 当前列表（id、父 Branch/父 Chance、operation、tool、阶段、权威值状态、Proposal 与关联 Branch），客户端自行折叠得到展示状态。之后通过 SSE 持续接收增量，断线后携带最后收到的序号补发，不维护另一套独立的树事实。每个事件包含全局递增序号、记录时刻和所属 task；序号在同一控制面运行内覆盖所有观察事件。

树结构事件描述 Branch 创建（identity、Parent、Authority/Candidate、所属 Chance、投机对象类型）、Chance 创建与阶段变化、权威值取得状态以及权威切换（新旧 Authority）。Branch 生命周期事件使用 `BranchStatus` 的 Created、Running、Blocked(reason)、Finished、Promoting、Promoted、Discarding、Discarded、Cleaning、Cleaned，记录新状态与已知原因；`Blocked` 的原因取 ExtentReached、Rebalanced 或 EffectGate，同一时刻只记最近一次挡住它的 Decision，`Blocked` 与 Running 可以随对应 Decision 来回切换。Chance 阶段使用 `ChanceStage` 的 Pending、Running、Resolving、Accepted(proposal_id)、Discarded(reason)，抛弃原因取 MatchRejected、NotSelected、AncestorInvalidated、BudgetExhausted 或 Failed；权威值取得状态使用属于 Chance 的 `AuthorityState` 的 NotRequested、Fetching、Arrived、Unavailable，与 Chance 的 Running 并行。环境事件描述已确认的执行承载、独立 Instance、隔离环境嵌套及关键配置摘要，并区分环境清理结果。未知事实保持未知，命令已发送不表示环境已建立或清理成功。

对话事件仅提示某 Branch 的消息发生变化；内容通过独立的按 Branch 只读接口按需取得，响应同时给出该 Branch 当前的角色与状态。候选内容与主对话分开展示，并随 Branch 的角色与状态显示其投机身份；选择观察对象不改变权威路由或生命周期裁决。

用量事件记录已上报的模型调用结束信息（模型、token、耗时、费用）或工具调用结束事实，不由界面猜测。收益以一次投机的结算表示，包含 Branch、命中与否、时间与金钱两个维度。每个维度分成独立的实测部分与估计部分，各自可以为空；两者永远不相加，也不互相填充，读取方分别累加。命中记为正的节约。未命中只记投机相对整场运行（权威路径）的减益：时间项是投机给权威路径造成的额外延迟的负值，不是候选存活时长，也不是快速模型自身的“白耗”；金钱项是候选真实花掉费用的负值。当前投机数量、累计节约等由事件累积计算，不单独发布统计事件。用量不等于收益，缺少可靠测量或估计依据时不得伪造结算。只读观察界面把每个维度的实测部分与估计部分分别展示、分别累加，不合并成单一数字；没有结算数据时如实显示暂无数据。

一次投机恰好产生一次结算，记为一次投机收尾的命中或未命中，不按每次模型或工具调用重复发布；累计值由读取方从结算事件分别累加实测与估计部分。规则如下：

- **工具投机命中**：时间实测 = 权威侧该工具 `ActionFinished` 的接入侧时刻 − 投机起点；工具投机没有可归属的模型费用差额，金钱的实测与估计部分都为空。
- **模型投机命中且有权威模型校验**：时间实测 = 权威模型推理耗时 − 快速模型推理耗时，金钱实测 = 权威模型费用 − 快速模型费用，二者都来自两次 `Usage` 的接入侧起止时刻与费用；任一数据缺失时该维度的实测部分为空，不退化为估计。
- **模型投机命中但没有权威模型、只靠简单校验**：时间与金钱只能写入估计部分，实测部分为空；估计依据必须在实现文档中写明。
- **未命中**：时间只记投机给权威路径造成的额外延迟的负值；当前没有对该延迟的可靠测量时实测与估计部分都为空，不用候选存活时长或其他耗时替代。金钱实测 = −(该候选分支累计费用)，费用缺失时为空。简单校验失败而必须再跑权威模型的模型投机属于未命中，其时间应记为负的估计值，浪费包含快速模型耗时与校验耗时；缺少校验耗时的测量时不得用部分耗时冒充完整浪费。
- 缺少哪个维度的数据，该维度的对应部分就为空，不用零值或推测值填充；货币只在金额存在时给出。
