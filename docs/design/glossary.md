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
| Branch Node | Branch 的根节点 | 同一结构实体 |
| Instance | Runtime 中承载 Branch Node 的执行单元 | 1:1 对应 |
| OutcomeSlot | 同名同义 | 共享语义 |
| Predictor | 无直接对应 | 底座独有 |
| Salvage | 无直接对应 | 底座独有 |


## 系统级

- **AScout（Agent Scout）**：面向 Agent 投机执行的通用底座，位于投机策略与执行机制之间，分离策略决策与具体运行时实现，使 Agent 能无修改地接入并获得投机执行能力。（`../mental-model.md`；`../architecture.md`；`../design-model.md`）
- **Speculative Tree**：Authority Instance 在投机执行不断发生和结束的过程中形成的树形结构；从 root Branch Node 开始记录 Branch 之间的派生关系。（`../mental-model.md` §4.2）
- **task**：session 中一次独立的 AScout 管理单元。AScout 为每个 task 建立一个 AScoutAgent 控制域，并在其中维护一棵 Speculative Tree。（`../mental-model.md` §4.2）
- **AScoutAgent**：一个 task 对外呈现的单一 Agent 视图，也是该 task 在控制面中的控制域；拥有 Speculative Tree 控制状态和 Branch 关系。（`../mental-model.md` §4.2）
- **Agent 接入侧**：四侧调用路径的入口侧，包含 Agent/product integration，以及 CLI、library、launcher、ASC-Daemon、Interposer 与 Operation Integration 等接入职责。（`../architecture.md` §1；`agent-integration.md`）
- **控制面**：四侧一级架构的控制侧；包含 Entry API、Policy Engine、Effect Coordinator、Assumption Manager、Scheduler 与 Ledger，拥有 AScoutAgent、Speculative Tree 控制状态、policy、请求校验、操作编排和 Runtime event 解释；是 AScout 自身实现的 Mechanism 主体，组件与实现见 `control-plane.md`。（`../architecture.md` §1；`control-plane.md`）

- **Runtime Adapter**：四侧调用路径的适配侧；按控制面的接入约定声明并呈现能力、operation result 与 event，并自行负责目标 Runtime 的能力映射、transport 和内部实现。（`../architecture.md` §1；`../development/runtime-adapter.md`）
- **Runtime**：四侧调用路径中的外部运行侧；提供实际执行、资源、隔离与效果能力，其内部架构不由 AScout 定义。（`../architecture.md` §1）

## 实体

- **Branch**：Speculative Tree 中的一个子树，是树中的计算延续和投机世界的结构实体。（`../mental-model.md` §4.2）
- **Branch Node**：Branch 对应的根节点，表示该 Branch 在 Speculative Tree 中的结构位置。（`../mental-model.md` §4.2）
- **Instance**：Runtime 中承载一个 Branch Node 的实际执行单元，代表实际运行的单一 Agent；Branch Node 与 Instance 1:1。（`../mental-model.md` §4.2）
- **Proposal** `î`：具有全局唯一 `ProposalId` 的投机候选输入；候选 Branch 基于其 payload 提前执行后续步骤。（`../design-model.md` §3.1；`policy-decisions.md` §2.3）
- **AssumptionSet**：Instance/SpecTask 携带的未决 Proposal ID 集合（`frozenset[ProposalId]`）。`AssumptionSet == ∅ ⟺` 该工作为权威工作。（`../design-model.md` §3.1；`policy-decisions.md` §4.2）
- **Edge**：一个投机机会，是投机执行的抽象模型与 Policy 的作用对象；同一条 Edge 上可以有多个 Proposal。可投机性由 `effect`、`admit` 及相应能力校验共同决定，不另设 admissibility 元数据。（`../design-model.md` §3.1；`policy-decisions.md`）
- **pending-result boundary**：真实 operation 结果未到的执行边界（时机语义）；runtime 在此从共同 execution base 实例化多个 Branch。（`branch-semantics.md`；`decisions.md` D-024；术语收敛）
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

- **branch**：从当前 Branch 创建一个或多个 direct child；投机场景中这些 child 才按角色称为 candidate。完整语义可携带 candidate 数量、Proposal、outcome slot 和可选 resolver override。（`branch-semantics.md` §Agent 最小感知模型；`decisions.md` D-037）
- **commit / commit request**：candidate 显式提交自己的子树。可 conditional commit：未验证前提随 subtree 保留为 obligation，验证前不能 Accept。（`decisions.md` D-025、D-037）
- **accept**：接受一个 winner subtree（裁决 = Reuse / Promote）；候选被接受后保留自身 Branch identity 并替换 parent 在树中的位置。（`branch-semantics.md`；`decisions.md` D-009/D-023）
- **discard**：丢弃整个 child subtree（= Disposition 的 Discard / Preempt）；不要求 candidate ready，不自动把 partial state 反馈给 parent。（`branch-semantics.md`；`decisions.md` D-023）
- **resolve()**：权威值到达时判 `î ≈ i`、选 Disposition 并沿依赖闭包传播的关键路径操作，由 Assumption Manager 执行。（`spec-exec-abstraction.md` §10.4 Assumption Manager）
- **Salvage**：失败后回收部分进展（`progress() / handoff()` 一等抽象）；warm-start、partial-token accept、promote 的共性。（`spec-exec-abstraction.md` §2 轴 6）
- **PublishOutcome**：真实 operation 完成后把 outcome 发布到 OutcomeSlot 的机制操作；由 Operation wrapper / Hook / effect broker 自动执行，不作为独立 Agent-facing 命令。（`branch-semantics.md`；`decisions.md` D-037）

## 组件

- **Policy Engine**：可替换 Policy 进入控制面的固定入口，解释 Policy Decision 并检查其取值在当前 Contract 与 Adapter 声明的后端能力下是否合法；加载期约束校验和运行期分发。（`control-plane.md` §3.1）
- **Assumption Manager**：控制面内 Proposal 依赖与确认/否定状态的 owner；维护"谁依赖谁"、跟踪 `AssumptionSet`，强制信息流约束（非空 AssumptionSet 的数据不得流向空容器）。（`control-plane.md` §3.2；`speculation-decisions.md` D-019）
- **Scheduler**：控制面内负责准入、预算、优先级、取消与资源协调的组件；是全系统唯一看到全局资源压力的控制面组件。（`control-plane.md` §3.3；`speculation-decisions.md` D-011/D-020/D-021）
- **Effect Coordinator**：控制面内效果处理的协调者；结合 Policy、Contract 与 Runtime 能力形成允许、暂存、转换、延迟、阻塞或补偿计划，经 Runtime Adapter 请求 Runtime 执行。它不持有真实 staged effect、obligation 或效果提交生效状态。（`control-plane.md` §3.4；`effect-management.md`）
- **Ledger**：异步旁路记录控制面运行事实（Policy 决策、状态变化、验证依据、效果与清理结果等），用于状态对齐、问题定位、shadow/replay 与契约校验；不从记录中决定生命周期。（`control-plane.md` §3.5；`speculation-contract.md` §3）
- **ASC-Daemon**：Agent 接入侧部署在 Branch 隔离环境内的本地控制入口，绑定 Branch context，转发 Agent-facing request 并承载 lifecycle event；不拥有全局控制状态。（`agent-integration.md` §2.2）
- **Interposer**：Agent 接入侧位于请求路径上的捕获与结果路由组件；具体可观察接缝由 integration profile 决定，不固定为四个接口。（`agent-integration.md` §2.3）
- **Entry API**：控制面的稳定入口；接收 Agent 接入侧形成的请求并交给控制面中的相应组件，不构成独立一级侧面，也不直接接触 Runtime。（`decisions.md` D-038；`../architecture.md` §1）

## 状态与生命周期

- **role**：`Authority | Speculative(d)`。这里的 Authority 是实例角色，不等同于可选的 Authority 输入来源；它不是静态标签，而是 `AssumptionSet == ∅` 的函数；promote 就是这个谓词翻转。（`spec-exec-abstraction.md` §10.1）
- **grain**：`own-env | shared-env`；shared-env 仅对 PURE 效果开放，隔离粒度由 EffectClass 决定。（`spec-exec-abstraction.md` §10.1）
- **lifecycle**：`Forked → Running → Resolving → {Reused, Promoted, Discarded, Preempted, Salvaged}`。（`spec-exec-abstraction.md` §10.1）
- **obligation**：未验证前提随 subtree 保留的义务；在验证前不能 Accept。Proposal 仍在 `AssumptionSet` 中即表示相应依赖尚未确认。（`branch-semantics.md`；`decisions.md` D-025）
- **capability**：resolver / validator 等授权能力；direct children 的 resolver capability 默认由 parent 持有，可显式委托给控制面。（`branch-semantics.md`；`decisions.md` D-022）

## 机制与概念

- **Policy Decision**：Policy 根据任务、上下文、资源和 Runtime 能力作出的可替换策略决策；当前七类 decision 及其阶段映射见 [`policy-decisions.md`](policy-decisions.md)。（`../design-model.md` §3.1）
- **commit barrier**：效果释放屏障；照抄处理器 commit/retire 语义（"write operations are buffered but not allowed to execute until the instruction is committed"）。（`spec-exec-abstraction.md` §4）
- **伪装执行（`simulate`）**：对已接入专门 Adapter 的外部服务拦截请求、不转发给真实服务、在 Branch 内生成 synthetic 模拟响应的处理方式，用于避免不可逆外部影响；模拟响应必须带 synthetic provenance、作为未验证假设参与 AssumptionSet 确认/否定，仍需 Authority、validator 或其他规则验证后才能被 winner 依赖。（`effect-management.md`；`policy-decisions.md` §2.5）
- **规范化资源记账**：成本三分 —— (a) 无论如何都要花的（命中时投机 work 即权威 work）/ (b) 纯浪费 / (c) 部分成本（被抢占/取消时已消耗部分）。口径必须包含 token 之外的资源。（`spec-exec-abstraction.md` §5；`speculation-decisions.md` D-012）
- **Shadow / Replay**：投机与串行同跑、只提交串行结果；同一套基础设施服务 fidelity 审计、predictor 数据管道、cost 校准与上线灰度（offline replay → shadow → canary → online → drift kill-switch）。（`spec-exec-abstraction.md` §7；`speculation-decisions.md` D-014）
- **透明嵌套**：投机实例与权威实例二进制同构，实例不知道自己是投机的；底座追踪 provenance、闸门设在效果层。（`spec-exec-abstraction.md` §9.5）
- **反射 gating**：Cost / admission 策略必须知道自身存活概率：`should_speculate(..., survival)`；底座透传 `P(实例存活)`，防止 depth-2 实例按 depth-0 的账过度投机。（`spec-exec-abstraction.md` §9.5）
- **Speculation Contract**：AScout 约束 Policy 表达和 Runtime Adapter 控制面接入行为、保证语义正确与对齐的契约。它规定 Policy 如何表达与校验决策取值、Adapter 如何声明并通过校验兑现后端能力，以及失败时如何收敛；控制面是它的实现方，Runtime 内部实现不在直接约束范围内。（`speculation-contract.md`）

## 范围决策

- **engine plane**：不做（D-013 修订）。KV cache 层 commit、partial-token accept、引擎压力调度、sequence residency 全部排除；跨层可见性降级为 API 可观测信号（延迟 / 限流 / 配额）。（`speculation-decisions.md` D-013/D-011/D-024/D-028）
- **Agent Boundary / Isolation Boundary**：必需的两条边界；Engine Boundary 在范围外。（`speculation-decisions.md` D-024）
