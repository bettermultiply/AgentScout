# 术语表

本表给出 AScout 每个术语的一句话定义与权威出处，并收录两套设计视角之间的关键映射。完整语义以各设计文档为准：抽象概念见 [`../design-model.md`](../design-model.md)，目标语义见 `docs/design/branch-semantics.md`、`docs/design/effect-management.md`、`docs/design/speculation-contract.md` 与 `docs/design/control-plane.md`，决策速查见 `docs/design/decisions.md` 与 `docs/design/speculation-decisions.md`。

**权威范围**：一级架构以 `decisions.md` D-039 和 [`../architecture.md`](../architecture.md) 为准；Branch 生命周期与状态语义以 [`branch-semantics.md`](branch-semantics.md) 和 `decisions.md` 为准；投机执行抽象以 `speculation-decisions.md` 为准。不同文档若发生冲突，必须显式收敛并同步修改。

## 视角映射

`decisions.md` 从 Speculative Tree 机制视角记录语义，`speculation-decisions.md` 从投机执行底座视角记录语义；二者描述同一个 AScout 系统。

| 底座术语 | Speculative Tree 视角 | 关系 |
|---|---|---|
| 在 Edge 启动 SpecTask | `branch`（创建 candidate） | 同一操作的两种表述 |
| `commit` / `commit request` | candidate 显式提交 subtree | 同一提交语义 |
| Reuse / Promote | `accept`（winner 转正） | 层间映射 |
| Discard / Preempt | `discard`（含 live-child discard） | 层间映射 |
| `resolve()` | Resolution barrier 的验证部分 | 层间映射 |
| Equivalence `≈` | Proposal compatibility 条件 | 层间映射 |
| Branch | Speculative Tree 中的子树 | 同一结构实体 |
| Branch Root | Branch 的根节点 | 同一结构实体 |
| Instance | Runtime 中承载已建立独立环境的 Branch Root 执行活动的执行单元 | 逻辑 Branch 建立独立环境后对应 |
| OutcomeSlot | 同名同义 | 共享语义 |
| Predictor | 无直接对应 | 底座独有 |
| Salvage | 无直接对应 | 底座独有 |


## 系统级

- **AScout（Agent Scout）**：面向 Agent 投机执行的通用底座，位于投机策略与执行机制之间，分离策略决策与具体运行时实现；Agent 可以在不修改自身执行逻辑的情况下接入，具体接入实现应尽可能保持修改面小。（`../mental-model.md`；`../architecture.md`；`../design-model.md`）
- **Speculative Tree**：Authority Instance 在投机执行不断发生和结束的过程中形成的树形结构；从 root Branch Root 开始记录 Branch 之间的派生关系。（`../mental-model.md` §4.2）
- **task**：session 中一次独立的 AScout 管理单元。AScout 为每个 task 建立一个 AScoutAgent 控制域，并在其中维护一棵 Speculative Tree。（`../mental-model.md` §4.2）
- **AScoutAgent**：一个 task 对外呈现的单一 Agent 视图，也是该 task 在控制面中的控制域；主对话操作当前有效的权威执行主线，另可只读观察该 Tree 的 Branch 状态与明确标记为未验证的候选内容，不改变主对话路由或生命周期权限。（`../mental-model.md` §4.2；`agent-integration.md` §3）
- **`/ascout`**：AScout TUI 内打开当前 AScoutAgent 整棵 Speculative Tree 的只读观察操作，可从 Branch 关系与投机状态选择具体 Branch 深入查看；选择仅改变观察对象，不切换主对话、不向候选发送对话，也不接受或丢弃 Branch。它不是 Agent 产品自身的命令，不查询其他 Tree。（用户确认：2026-10-25；`../../Discussion.md`；`../development/decisions.md`）
- **Agent 接入侧**：四侧调用路径的入口侧，负责 Agent 执行实体、执行活动、Branch context、operation 归属、Interposer、ASC-Proxy 和 Product Integration 等接入职责；其中 Interposer 统一承担 Agent 拦截与 operation integration。（`../architecture.md` §1；`agent-integration.md`）
- **控制面**：四侧一级架构的控制侧；包含 Agent Control Gateway、Adapter Registry、Policy Engine、Speculation Coordinator、Effect Coordinator 与 Ledger，拥有 AScoutAgent、Speculative Tree 控制状态、policy、请求校验、操作编排和 Runtime event 解释；是 AScout 自身实现的 Mechanism 主体，组件与实现见 `control-plane.md`。（`../architecture.md` §1；`control-plane.md`）

- **Runtime Adapter**：四侧调用路径的适配侧；按控制面的接入约定声明并呈现能力、operation result 与 event，并自行负责目标 Runtime 的能力映射、transport 和内部实现。（`../architecture.md` §1；`../development/runtime-adapter.md`）
- **Adapter Registry**：控制面内 Runtime Adapter 的固定接入入口；负责登记、契约兼容性检查、能力声明校验和接入时基本语义测试，并发布已校验的能力信息；不拥有 Runtime 或 Speculative Tree 的执行状态。（`control-plane.md` §2.3；`runtime-adapter-contract.md`）
- **Runtime**：四侧调用路径中的外部运行侧；提供 Agent 执行所需的环境、资源和隔离能力，其内部架构不由 AScout 定义；外部效果由控制面内的 Effect Coordinator 负责。（`../architecture.md` §1）

## 实体

- **Branch**：Speculative Tree 中的一个子树，是树中的计算延续和投机世界的结构实体。（`../mental-model.md` §4.2）
- **Branch Root**：Branch 对应的根节点，具有稳定 identity 并直接拥有该根的 local state；表示该 Branch 在 Speculative Tree 中的结构位置。（`../mental-model.md` §4.2）
- **Instance**：在 Runtime 提供的已建立独立环境中，由 Agent 接入侧承载和管理的 Branch Root 执行活动单元；逻辑 Branch 可以在建立独立环境前暂时没有对应 Instance。（`../mental-model.md` §4.2）
- **Proposal** `î`：具有全局唯一 `ProposalId` 的投机候选输入；候选 Branch 基于其 payload 提前执行后续步骤。（`../design-model.md` §3.1；`policy-decisions.md` §2.3）
- **AssumptionSet**：Instance/SpecTask 携带的未决 Proposal ID 集合（`frozenset[ProposalId]`）。`AssumptionSet == ∅ ⟺` 该工作为权威工作。（`../design-model.md` §3.1；`policy-decisions.md` §4.2）
- **Edge**：一个投机机会，是投机执行的抽象模型与 Policy 的作用对象；同一条 Edge 上可以有多个 Proposal。可投机性由 `effect`、`admit` 及相应能力校验共同决定，不另设 admissibility 元数据。（`../design-model.md` §3.1；`policy-decisions.md`）
- **pending-result boundary**：真实 operation 结果未到的执行边界（时机语义）；控制面在此从共同 execution base 登记多个逻辑 Branch，Runtime 按需建立其独立执行环境。（`branch-semantics.md`；`decisions.md` D-024；术语收敛）
- **SpecTask**：基于 Proposal 启动的投机工作单元，运行在隔离域中，携带 AssumptionSet。对应 candidate Branch。（`../design-model.md` §3.1；`policy-decisions.md` §4）
- **Authority** `A_u`：提供真实输入、结果或外部权威判断的可选来源（主生成流、真实环境、用户/人或远程服务）；`Authority` 可以为空。它对应 parent（producer lane）与真实 operation，不等同于 validator。权威值来自谁是 Edge 的固有事实，不是可替换的决策槽；是否取得、何时取得由 `launch` 决定（Eager / Lazy / Absent）。（`../design-model.md` §3.1；`policy-decisions.md` §2.4）
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

- **Prompt**：控制面要求接入侧向指定 Branch 的 AgentSession 发送用户输入。（用户确认：2026-10-25；`../../Discussion.md`）
- **Abort**：控制面要求接入侧中断指定 Branch 的当前执行，不表示外部操作已撤销。（用户确认：2026-10-25；`../../Discussion.md`）
- **Output**：接入侧报告指定 Branch 的实际文本输出或工具活动；显示不赋予候选权威身份。（用户确认：2026-10-25；`../../Discussion.md`）
- **TurnCompleted**：接入侧报告本轮执行结束，不表示预测已确认或候选可以转正。（用户确认：2026-10-25；`../../Discussion.md`）
- **CleanupCompleted**：接入侧确认指定 Branch 已停止并释放会话，不代表环境目录或任意外部资源已清理。（用户确认：2026-10-25；`../../Discussion.md`）
- **ToolPending**：接入侧报告 Parent 已产生真实工具调用但工具尚未执行，并携带操作标识、工具名、参数和可继续上下文。（用户确认：2026-10-25；`../../Discussion.md`）
- **StartCandidate**：控制面要求接入侧根据共同上下文和预测工具结果启动 Candidate 续跑；预测不是 Parent 的真实证据。（用户确认：2026-10-25；`../../Discussion.md`）
- **ResolvePending**：控制面在真实工具结果到达后要求接入侧继续 Parent，或按 Promote 让 Candidate 运行路径接替主路径；它本身不改变 Policy 裁决。（用户确认：2026-10-25；`../../Discussion.md`）

- **branch**：从当前 Branch 创建一个或多个 direct child；投机场景中这些 child 才按角色称为 candidate。完整语义可携带 candidate 数量、Proposal、outcome slot 和可选 resolver override。（`branch-semantics.md` §Agent 最小感知模型；`decisions.md` D-037）
- **commit / commit request**：candidate 显式提交自己的子树。可 conditional commit：未验证前提随 subtree 保留为 obligation，验证前不能 Accept。（`decisions.md` D-025、D-037）
- **accept**：接受一个 winner subtree（裁决 = Reuse / Promote）；候选被接受后保留自身 Branch identity 并替换 parent 在树中的位置。（`branch-semantics.md`；`decisions.md` D-009/D-023）
- **discard**：丢弃整个 child subtree（= Disposition 的 Discard / Preempt）；不要求 candidate ready，不自动把 partial state 反馈给 parent。（`branch-semantics.md`；`decisions.md` D-023）
- **resolve()**：权威值到达时进行匹配、选择处置并沿 Proposal 依赖传播的关键路径操作，由 Speculation Coordinator 协调。（`control-plane.md` §2.3；`policy-decisions.md`）
- **Salvage**：失败后回收部分进展（`progress() / handoff()` 一等抽象）；warm-start、partial-token accept、promote 的共性。（`spec-exec-abstraction.md` §2 轴 6）
- **PublishOutcome**：真实 operation 完成后把 outcome 发布到 OutcomeSlot 的机制操作；由 Operation wrapper / Hook / effect broker 自动执行，不作为独立 Agent-facing 命令。（`branch-semantics.md`；`decisions.md` D-037）

## 组件

- **Policy dialect**：Policy 经 Policy Engine 解析和校验后形成的运行期表示，供控制面及 Agent 接入侧执行 Policy Decision；不依赖或不改变全局状态的 Decision 可随其注入接入侧执行。
- **Policy Engine**：可替换 Policy 进入控制面的固定入口，解释 Policy Decision 并检查其取值在当前 Contract 与 Adapter 声明的后端能力下是否合法；加载期约束校验和运行期分发。（`control-plane.md` §2.2）
- **Speculation Coordinator**：控制面内全局投机视图与控制循环的协调核心；维护 Edge、Branch、Proposal、`AssumptionSet`、待选机会和调度状态之间的关系，激活受影响的 Policy Decision，协调 semantic operation，经 Runtime Adapter 执行并根据结果和事件更新视图。（`control-plane.md` §2.4）
- **Effect Coordinator**：控制面内共享、外部及 Policy 要求纳管外部效果的统一实际执行者；重点处理由 Agent 造成且可能影响全局的效果，例如调用外部 API。它通过控制面内注册的服务效果插件处理命令或 URL，可生成明确标记并记录来源的模拟响应，也可拒绝、阻塞、暂存、转换、提交、取消、清理或补偿效果。它不负责普通 Branch-local 操作；Runtime 不承担外部效果的实际执行责任。（`control-plane.md` §2.5；`effect-management.md`）
- **Ledger**：异步旁路记录控制面运行事实（Policy 决策、状态变化、验证依据、效果与清理结果等），用于状态对齐、问题定位、shadow/replay 与契约校验；不从记录中决定生命周期。（`control-plane.md` §2.6；`speculation-contract.md` §3）
- **ASC-Proxy**：Agent 接入侧与控制面之间的受管通信代理，负责会话、身份确认以及 request、fact、result 和 event 的双向传递；不拥有全局控制状态。（`agent-integration.md` §2.2）
- **Interposer**：Agent 接入侧统一承担 Agent 请求拦截、operation 识别、`pending-result boundary`、本地 Decision 执行和 outcome 路由的责任实体；具体可观察范围由当前接入方式决定。（`agent-integration.md` §2.3）
- **Agent Control Gateway**：控制面面向 Agent 接入侧的受管通信边界；管理 Task 启动、Agent 接入侧会话、Policy dialect、本地 Decision 结果、decision log 与受管控制请求的双向传递。Runtime 创建 Branch 执行环境后，它在该环境内创建并启动 Agent 接入侧；除通信会话所需的暂时状态外，不持有控制面或 Runtime 的权威状态。（`control-plane.md` §2.1）

## 状态与生命周期

- **role**：`Authority | Speculative(d)`。这里的 Authority 是实例角色，不等同于可选的 Authority 输入来源；它不是静态标签，而是 `AssumptionSet == ∅` 的函数；promote 就是这个谓词翻转。（`spec-exec-abstraction.md` §10.1）
- **grain**：`own-env | shared-env`；shared-env 仅对 PURE 效果开放，隔离粒度由 EffectClass 决定。（`spec-exec-abstraction.md` §10.1）
- **lifecycle**：`Forked → Running → Resolving → {Reused, Promoted, Discarded, Preempted, Salvaged}`。（`spec-exec-abstraction.md` §10.1）
- **obligation**：未验证前提随 subtree 保留的义务；在验证前不能 Accept。Proposal 仍在 `AssumptionSet` 中即表示相应依赖尚未确认。（`branch-semantics.md`；`decisions.md` D-025）
- **capability**：resolver / validator 等授权能力；direct children 的 resolver capability 默认由 parent 持有，可显式委托给控制面。（`branch-semantics.md`；`decisions.md` D-022）

## 机制与概念

- **Policy Decision**：Policy 根据任务、上下文、资源和 Runtime 能力作出的可替换策略决策；当前九类 decision 及其阶段映射见 [`policy-decisions.md`](policy-decisions.md)。（`../design-model.md` §3.1）
- **commit barrier**：效果释放屏障；照抄处理器 commit/retire 语义（"write operations are buffered but not allowed to execute until the instruction is committed"）。（`spec-exec-abstraction.md` §4）
- **伪装执行（`simulate`）**：对已注册效果插件的外部服务拦截请求、不转发给真实服务、在 Branch 内生成明确标记并记录来源的模拟响应的处理方式，用于避免不可逆外部影响；模拟响应作为未验证假设参与 AssumptionSet 确认/否定，仍需 Authority、validator 或其他规则验证后才能被 winner 依赖。（`effect-management.md`；`policy-decisions.md` §2.5）
- **规范化资源记账**：成本三分 —— (a) 无论如何都要花的（命中时投机 work 即权威 work）/ (b) 纯浪费 / (c) 部分成本（被抢占/取消时已消耗部分）。口径必须包含 token 之外的资源。（`spec-exec-abstraction.md` §5；`speculation-decisions.md` D-012）
- **Shadow / Replay**：投机与串行同跑、只提交串行结果；同一套基础设施服务 fidelity 审计、predictor 数据管道、cost 校准与上线灰度（offline replay → shadow → canary → online → drift kill-switch）。（`spec-exec-abstraction.md` §7；`speculation-decisions.md` D-014）
- **透明嵌套**：投机实例与权威实例二进制同构，实例不知道自己是投机的；底座追踪 provenance、闸门设在效果层。（`spec-exec-abstraction.md` §9.3）
- **反射 gating**：Policy 可以读取祖先依赖与成本等客观事实，自行估计实例存活概率并用于 `admit` 或 `select`；AScout 不固定 `survival` 字段，也不强制 Policy 使用该模型。（`spec-exec-abstraction.md` §9.3；`policy-decisions.md` §2.8）
- **Speculation Contract**：AScout 约束 Policy 表达和 Runtime Adapter 控制面接入行为、保证语义正确与对齐的契约。它规定 Policy 如何表达与校验决策取值、Adapter 如何声明并通过校验兑现后端能力，以及失败时如何收敛；控制面是它的实现方，Runtime 内部实现不在直接约束范围内。（`speculation-contract.md`）

## 范围决策

- **engine plane**：不做（D-013 修订）。KV cache 层 commit、partial-token accept、引擎压力调度、sequence residency 全部排除；跨层可见性降级为 API 可观测信号（延迟 / 限流 / 配额）。（`speculation-decisions.md` D-013/D-011/D-024/D-028）
- **Agent Boundary / Isolation Boundary**：必需的两条边界；Engine Boundary 在范围外。（`speculation-decisions.md` D-024）
