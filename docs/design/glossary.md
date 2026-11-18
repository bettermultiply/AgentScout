# 术语表

本表给出 AScout 每个术语的一句话定义与权威出处，并收录两套设计视角之间的关键映射。完整语义以各设计文档为准：抽象概念见 [`../design-model.md`](../design-model.md)，目标语义见 `docs/design/branch-semantics.md`、`docs/design/effect-management.md`、`docs/design/speculation-contract.md` 与 `docs/design/control-plane.md`，决策速查见 `docs/design/decisions.md` 与 `docs/design/speculation-decisions.md`。

**权威范围**：一级架构以 `decisions.md` D-039 和 [`../architecture.md`](../architecture.md) 为准；Branch 生命周期与状态语义以 [`branch-semantics.md`](branch-semantics.md) 和 `decisions.md` 为准；投机执行抽象以 [`../design-model.md`](../design-model.md) 与 [`policy-decisions.md`](policy-decisions.md) 为准。不同文档若发生冲突，必须显式收敛并同步修改。

## 视角映射

**notification**：Policy 在既有 dispose／Promote 中可选构造、向成功 winner 交付的非空白上下文通知；缺省零记录，Pi 使用同名原生 custom message，不启动新 turn。见 [Agent 接入设计](agent-integration.md) §3。

`decisions.md` 从 Speculative Tree 机制视角记录语义，`speculation-decisions.md` 从投机执行底座视角记录设计选择与理由；二者描述同一个 AScout 系统。

| 底座术语 | Speculative Tree 视角 | 关系 |
|---|---|---|
| 在 Chance 上启动 SpecTask | `branch`（创建 candidate） | 同一操作的两种表述 |
| `commit` / `commit request` | candidate 显式提交 subtree | 同一提交语义 |
| Reuse / Promote | `accept`（resolution = accepted）的两种落实：Reuse 交结果，Promote 替换主延续 | 层间映射 |
| Discard / Preempt | `discard`（含 live-child discard） | 层间映射 |
| `resolve()` | Resolution barrier 的验证部分 | 层间映射 |
| Equivalence `≈` | Proposal compatibility 条件 | 层间映射 |
| Branch | Speculative Tree 中的子树 | 同一结构实体 |
| Branch Root | Branch 的根节点 | 同一结构实体 |
| Backend instance | Runtime 内部具有独立 `backend_id` 的 Backend 执行实例；由 Runtime 维护与 Environment 的关联，不是 control plane 资源 | control plane 不传 `backend_id` |
| OutcomeSlot | 同名同义 | 共享语义 |
| Predictor | 无直接对应 | 底座独有 |
| Salvage | 无直接对应 | 底座独有 |


## 系统级

- **run id**：AScout 管理的运行身份，也是 `runs_dir` 下运行记录目录的目录名；新建时使用 `run-` 前缀，恢复时保留原 id。它与 Agent 原生 session id、每轮工作的 task id 分别独立；工作区仍指 workspace。

- **AScout（Agent Scout）**：面向 Agent 投机执行的通用底座，位于投机策略与执行机制之间，分离策略决策与具体运行时实现；Agent 可以在不修改自身执行逻辑的情况下接入，具体接入实现应尽可能保持修改面小。（`../mental-model.md`；`../architecture.md`；`../design-model.md`）
- **Speculative Tree**：Authority Instance 在投机执行不断发生和结束的过程中形成的树形结构；从 root Branch Root 开始记录 Branch 之间的派生关系。（`../mental-model.md` §4.2）
- **task**：session 中 Agent 的一轮工作；Agent 完成本轮工作、开始等待用户输入时，task 完成。一个 session 对应一个 AScoutAgent 和一棵 Speculative Tree，多轮 task 共用该控制域。（`../mental-model.md` §4.2）
- **AScoutAgent**：一个 session 对外呈现的单一 Agent 视图，也是该 session 在 control plane 中的控制域，拥有一棵 Speculative Tree；主对话操作当前有效的权威执行主线，另可只读观察该 Tree 的 Branch 状态与候选内容；候选内容与主对话分开，并随 Branch 的角色与状态显示其投机身份，不改变主对话路由或生命周期权限。（`../mental-model.md` §4.2；`agent-integration.md` §3）
- **交互接口**：Agent 侧声明并提供、AScout 只发现并转发的一组输入能力与提示，其中输入能力包括命令清单、编辑器能力与特殊输入模式，提示包括确认、选择、输入和编辑器等对话式提示；AScout 不硬编码 Agent 的命令表，不嵌入也不复制 Agent 的原生界面或终端画面，不解释 Agent 语义，只转发。（`agent-integration.md` §3.1；`../development/agent-access.md` §3.1）
- **`/ascout`**：AScout TUI 内打开当前 AScoutAgent 整棵 Speculative Tree 的只读观察操作，可从 Branch 关系与投机状态选择具体 Branch 深入查看；选择仅改变观察对象，不切换主对话、不向候选发送对话，也不接受或丢弃 Branch。它不是 Agent 产品自身的命令，不查询其他 Tree。（用户确认：2026-10-25；`../../Discussion.md`；`../development/decisions.md`）
- **Instrumentation**：接到已有 Agent 上的一侧；报告动作事实，求值不改写全局控制状态、也不在多个 Chance、Branch、Proposal 之间协调的 Decision，并落实 control plane 返回的处置。AScout 本身不是 Agent。（`../architecture.md` §1；`agent-integration.md`）
- **agent supervisor**：Runtime 中负责启动、监视和回收 run 唯一 Agent 与 Instrumentation 宿主的部分。宿主位于 Branch 环境外。（`runtime.md`）
- **Guest Agent**：Runtime 在 guest 内执行宿主交来的命令的服务；不是 Instrumentation，也不是本系统所说的 Agent。可执行文件名为 `ascout-guest-agent`。（`../development/runtime-adapter.md`）
- **Pi integration**：Pi 的 Product Integration 实现；Product Integration 仍是通用角色名。（`agent-integration.md`；`../development/agent-access.md`）
- **control plane**：一个 session 的投机控制状态、Policy 绑定、校验和编排的所有者。Gateway 属于它；包含 UI Projection、Policy Engine、Speculation Coordinator 与 Ledger。Registry 属于 Runtime。（`../architecture.md` §1；`control-plane.md`）

- **runtime proxy**：Runtime 与 control plane 之间的接入边界；按 control plane 的接入约定声明并呈现能力、operation result 与 event，并负责能力映射、transport 和事件规范化。（`../architecture.md` §1；`../development/runtime-adapter.md`）
- **backend registry**：Runtime 内读取节点配置并登记可用 backend 实现、入口、接口及能力声明的组件，负责核对所需标准接口。实例生命周期由 base runtime 管理。（`runtime.md`）
- **base runtime**：管理 Environment、实际参与的 backend 及操作结果的 Runtime 组件。（`runtime.md`）
- **Runtime**：AScout 支持的完整执行与效果承载系统；承载 Agent 一步动作，提供环境、资源、状态派生、隔离和外部效果边界。其内部架构不由 AScout 预先固定，具体能力可以通过进程、Overlay、容器或虚拟机组合。（`../architecture.md` §1；`runtime.md`）

## 实体

- **权威 session 文件**：最终汇总为单一 session 的、经过验证并用真实结果整理后的执行历史，保留预测输入节点、原主分支和被丢弃旁支。Agent 从它 resume 得到整理后的权威路径，不保证准确复原候选原始预测输入上下文；它不是新的 control plane 实体或第二棵控制 Tree，单一最终文件不等于单一运行 session。（`agent-integration.md` §3；`control-plane.md` §2.6）

- **Branch**：Speculative Tree 中的一个子树，是树中的计算延续和投机世界的结构实体。（`../mental-model.md` §4.2）
- **Branch Root**：Branch 对应的根节点，具有稳定 identity 并直接拥有该根的 local state；表示该 Branch 在 Speculative Tree 中的结构位置。（`../mental-model.md` §4.2）
- **Instance**：由 Runtime 承载 Branch Root 执行活动的执行单元；多个 Branch 可以共享、复制或独立承载所需状态，也可以在恢复或重建后继续。逻辑 Branch 可以在建立 Instance 前先存在并推进。（`../mental-model.md` §4.2）
- **Proposal** `î`：具有全局唯一 `ProposalId` 的投机候选输入；候选 Branch 基于其 payload 提前执行后续步骤。（`../design-model.md` §3.1；`policy-decisions.md` §2.3）
- **AssumptionSet**：Instance/SpecTask 携带的未决 Proposal 引用集合（`frozenset[(chance_id, proposal_id)]`）。`AssumptionSet == ∅ ⟺` 该工作为权威工作。（`../design-model.md` §3.1；`policy-decisions.md` §4.2）
- **Edge**：描述投机如何进行的通用过程形状的抽象模型（`u --i--> v`，以 Proposal 替代 `i` 提前启动 `v^n`），九类 Policy Decision 的语义定义在它之上。Edge 不是运行时对象，没有 id；它的具体实例是 Chance。（`../design-model.md` §3.1；`policy-decisions.md`）
- **Chance**：Edge 模型的一个具体实例，即被 `admit` 接纳的投机机会。从 `admit` 通过起拥有 id，直到被验证后接受或抛弃、整个过程收敛为止；其下可有多个 Proposal 和多个 Branch，但每个 Proposal/Branch 只对应一个 Chance。`select` 的待选集合、AssumptionSet 中的引用以及运行时 Decision 的作用对象都是 Chance。`admit` 之前的机会发现阶段没有 id，不属于 Chance。Branch 仍是具体执行，Proposal 仍是不可变候选输入，Candidate 仍只是 Branch 的角色。（`../design-model.md` §3.1；`policy-decisions.md`）
- **pending-result boundary**：真实 operation 结果未到的执行边界（时机语义）；control plane 在此登记多个逻辑 Branch，并为每个 Branch 绑定独占 Environment。（`branch-semantics.md`；术语收敛）
- **SpecTask**：基于 Proposal 启动的投机工作单元，运行在隔离域中，携带 AssumptionSet。对应 candidate Branch。（`../design-model.md` §3.1；`policy-decisions.md` §4）
- **Authority** `A_u`：提供真实输入、结果或外部权威判断的可选来源（主生成流、真实环境、用户/人或远程服务）；`Authority` 可以为空。它对应 parent（producer lane）与真实 operation，不等同于 validator。权威值来自谁是 Chance 的固有事实，不是可替换的决策槽；是否取得、何时取得由 `launch(input) -> bool` 决定，Eager、Absent 和 Lazy 只是系统提供的快捷定义。（`../design-model.md` §3.1；`policy-decisions.md` §2.4）
- **validator**：独立的验证器；在有 Authority 时可辅助判断 Proposal 是否兼容，在 Authority 为空时可依据结果或约束直接作可接受性判断。validator 不是 Authority 的一种。（`spec-exec-abstraction.md` §2 轴 4、轴 5）
- **Predictor** `S_u`：`u` 的廉价实现，产出 Proposal；由 `predict` decision 表达。（`policy-decisions.md` §2.3）
- **Reconciliation**：权威值到达时判定 `î ≈ i` 并选择处置的过程；对应 Resolution barrier 的验证部分。（`policy-decisions.md` §2.6–§2.7；术语收敛）
- **OutcomeSlot**：真实 outcome / 权威值的投递槽位；两边同名同义。（`branch-semantics.md`；`spec-exec-abstraction.md`）
- **Disposition**：处置结果枚举：Reuse / Promote / Discard / Preempt / Re-execute / Compensate / Overwrite / Salvage。（`spec-exec-abstraction.md` §2 轴 6）
- **Equivalence** `≈`：判定"猜中"的可插拔同余关系，四层：L0 语法相等 / L1 语义等价 / L2 效果同余 / L3 结果可接受（唯一不需要权威值、可支撑 Bypass 的层次）。（`spec-exec-abstraction.md` §2 轴 4）
- **EffectClass**：效果的投机纪律分类：PURE / IDEMPOTENT / STAGEABLE / TRANSFORMABLE / COMPENSABLE / OVERWRITABLE / IRREVERSIBLE。（`effect-management.md` §3）
- **StateDomain**：状态域：filesystem / process / memory / GUI / network / database / email / payment。EffectClass × StateDomain 交叉决定具体隔离机制。（`effect-management.md` §3）
- **Fidelity 契约**：保真度等级：Trajectory-equivalent / Outcome-equivalent / Bounded-degradation / Best-effort。可声明、可强制（加载期对撞）、可审计（shadow 实测）。（`spec-exec-abstraction.md` §2 轴 7）

## 操作

- **Prompt**：control plane 要求 Instrumentation 向指定 Branch 的 AgentSession 发送用户输入。（用户确认：2026-10-25；`../../Discussion.md`）
- **Abort**：control plane 要求 Instrumentation 中断指定 Branch 的当前执行，不表示外部操作已撤销。（用户确认：2026-10-25；`../../Discussion.md`）
- **Output**：Instrumentation 报告指定 Branch 的实际文本输出或工具活动；显示不赋予候选权威身份。（用户确认：2026-10-25；`../../Discussion.md`）
- **TurnCompleted**：Instrumentation 报告本轮执行结束，不表示预测已确认或候选可以 Promote。（用户确认：2026-10-25；`../../Discussion.md`）
- **ActionStarted / ActionFinished**：Agent action 的实际开始及结果事实；action 包括模型推理、工具执行、启动 sub-agent 等。执行前准入等待由 `SpeculativeAdmissionRequest` 登记，不以准入表示真实操作已发生。结果区分成功、失败、取消、超时，并明确 task、Branch 和 action 身份。（用户确认：2026-11-06；`../../Discussion.md`）
- **AgentSessionReleased / BranchReleased**：Instrumentation 分别确认 Agent session 已释放、Branch 执行活动已释放；两者都不能代替 Runtime 环境清理事实。（用户确认：2026-11-06；`../../Discussion.md`）
- **StartCandidate**：control plane 要求 Instrumentation 根据共同上下文和预测工具结果启动 Candidate 续跑；预测不是 Parent 的真实证据。（用户确认：2026-10-25；`../../Discussion.md`）
- **ResolvePending**：control plane 报告真实等待点的裁决，不阻塞 Parent 取得真实结果并继续；接替主路径由 Promote 命令落实，它本身不改变 Policy 裁决。（用户确认：2026-10-25；`../../Discussion.md`）

- **branch**：从当前 Branch 创建一个或多个 direct child；投机场景中这些 child 才按角色称为 candidate。完整语义可携带 candidate 数量、Proposal、outcome slot 和可选 resolver override。（`branch-semantics.md` §Agent 最小感知模型；`decisions.md` D-037）
- **commit / commit request**：candidate 显式提交自己的子树。可 conditional commit：未验证前提随 subtree 保留为 obligation，验证前不能 Accept。（`branch-semantics.md` §4.3；`decisions.md` D-037）
- **accept**：对 candidate 作出的接受裁决（resolution = accepted）；`Promote` 让候选保留自身 Branch identity 并替换 parent 在树中的位置，`Reuse` 只把已验证结果和按声明移交的受管状态交给现有权威延续并清理候选。（`branch-semantics.md`；`decisions.md` D-009/D-023）
- **discard**：丢弃整个 child subtree（= Disposition 的 Discard / Preempt）；不要求 candidate ready，不自动把 partial state 反馈给 parent。（`branch-semantics.md`；`decisions.md` D-023）
- **resolve()**：权威值到达时进行匹配、选择处置并沿 Proposal 依赖传播的关键路径操作，由 Speculation Coordinator 协调。（`control-plane.md` §2.3；`policy-decisions.md`）
- **Salvage**：失败后回收部分进展（`progress() / handoff()` 一等抽象）；warm-start、partial-token accept、promote 的共性。（`spec-exec-abstraction.md` §2 轴 6）
- **PublishOutcome**：真实 operation 完成后把 outcome 发布到 OutcomeSlot 的机制操作；由 Operation wrapper / Hook / effect broker 自动执行，不作为独立 Agent-facing 命令。（`branch-semantics.md`；`decisions.md` D-037）

## 组件

- **PolicyDefinition**：Policy 作者使用 Policy DSL 编写的具体 Policy，由声明部分及其引用的独立算法源码共同组成；展开并编译后，九类 Policy Decision 均具有明确的决策行为。（`control-plane/policy-engine.md` §2）
- **Policy DSL**：PolicyDefinition 的唯一入口，规定其结构、九类 Decision 各自的输入与输出、可读取事实、可使用能力及组合约束。（`control-plane/policy-engine.md` §2）
- **Policy Engine**：Policy 进入 AScout 的固定入口，只有两项职责：编译 Policy，以及根据规定输入对已编译的 Policy Decision 求值、作出本次 Policy Decision。作出 Policy Decision 不等于落实 Policy Decision；它不负责把 Policy Decision 应用到 Branch、Speculative Tree、外部效果或 Runtime，也不负责选择运行位置或向 Instrumentation 交付编译结果。（`control-plane/policy-engine.md`）
- **Speculation Coordinator**：control plane 中推动处理、调用 Policy 并发出操作的协调角色；不拥有 Tree，不保存第二份投机状态，可以服务多棵树。Chance、Branch、Proposal、`AssumptionSet`、待选机会和调度状态由每个 session 的 AScoutAgent 所拥有的 Speculative Tree 保存。（`control-plane.md` §2.4）
- **External Effect Coordinator**：Runtime 的组件，处理本机隔离覆盖不到、需通过网络访问环境之外的 API 操作，与 network backend 协同，不负责本地文件和进程隔离。Policy 的 `effect` 决定处理方式，Instrumentation 本地落实并将模拟操作交给它。真实请求先暂存在本机；可模拟时才返回明确标记来源的临时结果，不能模拟时在效果边界阻塞当前 Branch，保留已完成进展且不提供模拟结果，待该 Branch 成为权威后发出真实请求并继续，丢弃或前提失效则取消。它无需为跨 Branch 命令去重掌握整棵树；control plane 只在接受、丢弃或前提失效时通知相应暂存请求释放或取消。（`effect-management.md`）
- **Ledger**：异步旁路记录 control plane 运行事实（Policy 决策、状态变化、验证依据、效果与清理结果等），只追加并保存记录，读取只从已保存记录按序号取出，用于状态对齐、问题定位、shadow/replay 与契约校验；不从记录中决定生命周期，不要求据观察日志恢复整棵树。已退出 Tree 的 Branch 归档首次失败后安全恢复追加边界并重试一次，最终失败先以 run id、Branch id 和原因通过系统运行日志及 UI Projection 报警，再释放记录，不永久保留未归档 Branch。（`control-plane.md` §2.6；`speculation-contract.md` §3）
- **ASC-Proxy**：Instrumentation 进程内的受管 Unix socket 客户端，同时管理经 Gateway 接入 control plane 的控制连接及不依赖具体 backend 的独立 Runtime 执行连接，在控制连接上关联 Agent 和 Branch，在执行连接上使用 Environment 和 operation 身份；不拥有全局控制状态，也不是独立代理进程。（`agent-integration.md` §2.2）
- **Interposer**：Instrumentation 统一承担 Agent 请求拦截、operation 识别、`pending-result boundary`、本地 Decision 求值和 outcome 路由的责任实体；具体可观察范围由当前接入方式决定。（`agent-integration.md` §2.3）
- **Gateway**：control plane 面向 Instrumentation、UI 与 Runtime 的统一受管通信边界；负责连接／会话管理、入口格式／身份／上下文／权限检查、消息转交和交付。不拥有 Tree、Policy 或投机编排；消息对投机执行的变化由相应控制逻辑决定。（`control-plane.md` §2.1）

## 状态与生命周期

- **BranchStatus**：Branch 生命周期与运行状态的合并枚举：Created、Running、Blocked(reason)、Finished、Promoting、Promoted、Discarding、Discarded、Cleaning、Cleaned。Blocked 的原因取 ExtentReached、Rebalanced 或 EffectGate，同一时刻只记最近一次挡住它的 Decision；Finished 表示已跑完、等待裁决。（`branch-semantics.md` §2；`control-plane.md` §5）
- **ChanceStage**：Chance 的阶段与结果枚举：Pending、Running、Resolving、Accepted(proposal_id)、Discarded(reason)。Pending 为已 `admit`、等待 `select`；Running 为已被选中；Resolving 为权威值已到、正在 `match` 与处置；Discarded 的原因取 MatchRejected、NotSelected、AncestorInvalidated、BudgetExhausted 或 Failed。（`branch-semantics.md` §2；`control-plane.md` §5）
- **AuthorityState**：属于 Chance 的权威值取得状态，与 Chance 的 Running 并行，由 `launch` 改变：NotRequested、Fetching、Arrived、Unavailable。`launch=false` 只表示暂不请求权威值。（`branch-semantics.md` §2；`policy-decisions.md` §2.4）
- **role**：`Authority | Speculative(d)`。这里的 Authority 是实例角色，不等同于可选的 Authority 输入来源；它不是静态标签，而是 `AssumptionSet == ∅` 的函数；promote 就是这个谓词翻转。（`spec-exec-abstraction.md` §10.1）
- **lifecycle**：Branch 的生命周期与运行状态由 `BranchStatus` 表达，Chance 的阶段与结果由 `ChanceStage` 表达；`Accepted` 与 `Discarded` 等结果落在 Chance 上，Branch 的 Promote、Discard、Cleanup 等动作决定 BranchStatus。（`spec-exec-abstraction.md` §10.1；`branch-semantics.md` §2）
- **obligation**：未验证前提随 subtree 保留的义务；在验证前不能 Accept。Proposal 仍在 `AssumptionSet` 中即表示相应依赖尚未确认。（`branch-semantics.md` §4.3）
- **capability**：宿主明确授予的能力。resolver / validator 的授权遵循 Branch 规则，direct children 的 resolver capability 默认由 parent 持有，可显式委托给 control plane（`branch-semantics.md` §6）。Policy 的 `model_complete` capability 仅允许经宿主检查后请求指定模型；授权绑定正在进行的 Decision 求值、请求标识、模型名单、额度和期限，不能由 Policy 自行声明，也不把凭证交给 Policy。

## 机制与概念

- **Policy Decision**：Policy 根据任务、上下文、资源和 Runtime 能力作出的可替换策略决策；当前九类 decision 及其阶段映射见 [`policy-decisions.md`](policy-decisions.md)。（`../design-model.md` §3.1）
- **commit barrier**：效果释放屏障；照抄处理器 commit/retire 语义（"write operations are buffered but not allowed to execute until the instruction is committed"）。（`spec-exec-abstraction.md` §4）
- **伪装执行（`simulate`）**：对已暂存在本机、尚未发往真实服务的请求生成明确标记来源的临时响应，让 Branch 继续。模拟建立在暂存之上；Branch 被接受后才处理暂存的真实请求，丢弃或前提失效则取消真实请求，模拟结果不会自动变成真实调用。模拟响应作为未验证假设参与 AssumptionSet 确认/否定，仍需 Authority、validator 或其他规则验证后才能被 winner 依赖。（`effect-management.md`；`policy-decisions.md` §2.5）
- **规范化资源记账**：成本三分 —— (a) 无论如何都要花的（命中时投机 work 即权威 work）/ (b) 纯浪费 / (c) 部分成本（被抢占/取消时已消耗部分）。口径必须包含 token 之外的资源。（`spec-exec-abstraction.md` §5；`speculation-decisions.md` D-012）
- **Shadow**：投机与串行同跑、只提交串行结果，用于 fidelity 审计、predictor 数据管道、cost 校准与上线灰度。（`speculation-decisions.md` D-014）
- **Replay（回放）**：在独立验证运行中使用录制内容作为刺激；模型输出回放只检验系统反应，不评价 Agent、不证明性能收益，也不自动恢复原 task 的失败执行。（`speculation-contract.md` §3.3；实现范围见 `../development/replay.md`）
- **透明嵌套**：投机实例与权威实例二进制同构，实例不知道自己是投机的；底座追踪 provenance、闸门设在效果层。（`spec-exec-abstraction.md` §9.3）
- **反射 gating**：Policy 可以读取祖先依赖与成本等客观事实，自行估计实例存活概率并用于 `admit` 或 `select`；AScout 不固定 `survival` 字段，也不强制 Policy 使用该模型。（`spec-exec-abstraction.md` §9.3；`policy-decisions.md` §2.8）
- **Speculation Contract**：AScout 约束 Policy、control plane 与 Runtime 之间语义对齐的契约。它规定 Policy 如何表达与校验决策取值、Runtime 如何通过接入边界声明并兑现能力，以及失败时如何收敛；不固定 Runtime 内部实现。（`speculation-contract.md`）

## 范围决策

- **engine plane**：不做（D-013 修订）。KV cache 层 commit、partial-token accept、引擎压力调度、sequence residency 全部排除；跨层可见性降级为 API 可观测信号（延迟 / 限流 / 配额）。（`speculation-decisions.md` D-013/D-011/D-024）
- **Agent Boundary / Isolation Boundary**：必需的两条边界；Engine Boundary 在范围外。（`speculation-decisions.md` D-024）

- **UI Projection**：位于 Gateway 与 Ledger 之间，按序号读取已保存记录，持有可丢弃的界面缓存及连接读取位置，向 HTTP 与终端界面提供读取并转发系统运行日志报警；不拥有 Tree、不裁决投机、不写回 Ledger。（`control-plane.md`）
- **系统运行日志**：由 control plane 进程持有的固定格式 UTF-8 逐行警告和错误日志，独立于 Ledger；只可配置 `log_path`，启动时必须能打开，不轮转。（`control-plane.md`）
