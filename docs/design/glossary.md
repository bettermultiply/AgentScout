# 术语表

本表给出 AScout 每个术语的一句话定义与权威出处。完整语义以各设计文档为准：目标语义见 `docs/design/overview.md`、`docs/design/decisions.md` 与 `docs/design/speculation-decisions.md`；两套视角（Speculative Tree 机制表述与投机执行底座抽象）的对应关系见 [`terminology.md`](terminology.md)。

## 系统级

- **AScout（Agent Scout）**：面向 Agent 投机执行的通用底座，将投机执行所需的多种能力抽象为统一机制，使开发者能够组合与替换这些能力、构建不同的执行策略。（`../mental-model.md`；`../architecture.md`；`overview.md`）
- **Speculative Tree**：AScout 的核心控制结构：从一个 root Branch 开始记录 Branch 之间的派生关系。（`overview.md` §Branch 与 Speculative Tree）
- **task**：端用户提交的准入单元。一个 task 对应一棵 Speculative Tree（一个 AScoutAgent 控制域）；准入时绑定 policy profile 并建立 root Authority 实例。（`spec-exec-abstraction.md` §10.4 Entry API）
- **AScoutAgent**：一棵 Speculative Tree 的控制域；Branch identity、capability、commit/discard 与 resolution 都只在这棵 tree 内有意义。task 是该控制域的用户视角名称。（`overview.md` §概述）
- **Agent 接入侧**：四侧一级架构的入口侧，包含 Agent/product，以及 CLI、library、launcher、Agent integration、ASC-Daemon 与 Interposer 等接入职责。（`../architecture.md` §1）
- **控制面侧**：四侧一级架构的控制侧；Harness Core 及其内部组件拥有 AScoutAgent、Speculative Tree 控制状态、policy、请求校验、操作编排和 Runtime event 解释。（`../architecture.md` §2）
- **Runtime Adapter 侧**：四侧一级架构的适配侧；内部包含稳定 Harness Runtime Boundary 与面向具体 Runtime 的实际 Runtime Adapter。（`../architecture.md` §1）
- **Runtime Mechanism**：四侧一级架构的机制侧；拥有节点执行状态、资源、隔离、capability/obligation validation、barrier、原子转换与 cleanup。（`../architecture.md` §2）

## 实体

- **Branch**：Speculative Tree 中的结构实体（节点）。指称扩展为以该节点为根的**整个子树**，不再区分 `subtree(A)`；提交、丢弃、接受都以整个子树为单位。每个 Branch 节点在运行时由一个 Instance 承载（1:1）。（`overview.md` §Branch 与 Speculative Tree；术语收敛）
- **Instance**：运行时执行单元，携带 AssumptionSet、role、grain 与 lifecycle；与 Branch 1:1。投机实例与权威实例二进制同构（透明嵌套的前提）。（`spec-exec-abstraction.md` §10.1、§9.5）
- **Proposal** `î`：一次投机预测值：`{value, confidence, provenance, source}`。候选 Branch 基于它提前执行后续步骤。（`spec-exec-abstraction.md` §1）
- **Assumption**：`(site_id, proposal_id)`，读作"proposal X 成立这一假设"；compatibility 条件可以是 exact payload、predicate 或 virtual-value mapping。（`spec-exec-abstraction.md` §3；`decisions.md` D-025）
- **AssumptionSet**：Instance/SpecTask 携带的依赖闭包（`frozenset[Assumption]`）。`AssumptionSet == ∅ ⟺` 该工作为权威工作。（`spec-exec-abstraction.md` §3）
- **Site**：一条被标注"可投机"的依赖边 `u → v`，携带 admissibility 元数据；投机启动点（结构语义）。建立于 pending-result boundary 上。（`spec-exec-abstraction.md` §1）
- **pending-result boundary**：真实 operation 结果未到的执行边界（时机语义）；runtime 在此从共同 execution base 实例化多个 Branch。（`overview.md`；`decisions.md` D-024；术语收敛）
- **SpecTask**：基于 Proposal 启动的投机工作单元，运行在隔离域中，携带 AssumptionSet。对应 candidate Branch。（`spec-exec-abstraction.md` §1）
- **Authority** `A_u`：产出真值 `i` 的权威主体（大模型 / 主生成流 / 真实环境 / 人 / validator）。对应 parent（producer lane）与真实 operation。（`spec-exec-abstraction.md` §1、§2 轴 5）
- **Predictor** `S_u`：`u` 的廉价实现，产出 Proposal；轴 3，纯插件。（`spec-exec-abstraction.md` §2 轴 3）
- **Reconciliation**：权威值到达时判定 `î ≈ i` 并选择处置的过程；对应 Resolution barrier 的验证部分。（`spec-exec-abstraction.md` §1；术语收敛）
- **OutcomeSlot**：真实 outcome / 权威值的投递槽位；两边同名同义。（`overview.md`；`spec-exec-abstraction.md`）
- **Disposition**：处置结果枚举：Reuse / Promote / Discard / Preempt / Re-execute / Compensate / Overwrite / Salvage。（`spec-exec-abstraction.md` §2 轴 6）
- **Equivalence** `≈`：判定"猜中"的可插拔同余关系，四层：L0 语法相等 / L1 语义等价 / L2 效果同余 / L3 结果可接受（唯一不需要权威值、可支撑 Bypass 的层次）。（`spec-exec-abstraction.md` §2 轴 4）
- **EffectClass**：效果的投机纪律分类：PURE / IDEMPOTENT / STAGEABLE / TRANSFORMABLE / COMPENSABLE / OVERWRITABLE / IRREVERSIBLE。（`spec-exec-abstraction.md` §4）
- **StateDomain**：状态域：filesystem / process / memory / GUI / network / database / email / payment。EffectClass × StateDomain 交叉决定具体隔离机制。（`spec-exec-abstraction.md` §11.1(A)）
- **Fidelity 契约**：保真度等级：Trajectory-equivalent / Outcome-equivalent / Bounded-degradation / Best-effort。可声明、可强制（加载期对撞）、可审计（shadow 实测）。（`spec-exec-abstraction.md` §2 轴 7）

## 操作

- **branch**：从当前 Branch 创建一个或多个 direct child；投机场景中这些 child 才按角色称为 candidate。完整语义可携带 candidate 数量、Proposal、outcome slot 和可选 resolver override。（`overview.md` §Agent 最小感知模型；`decisions.md` D-037）
- **commit / commit request**：candidate 显式提交自己的子树。可 conditional commit：未验证前提随 subtree 保留为 obligation，验证前不能 Accept。（`decisions.md` D-025、D-037）
- **accept**：接受一个 winner subtree（裁决 = Reuse / Promote）；候选被接受后保留自身 Branch identity 并替换 parent 在树中的位置。（`overview.md`；`decisions.md` D-009/D-023）
- **discard**：丢弃整个 child subtree（= Disposition 的 Discard / Preempt）；不要求 candidate ready，不自动把 partial state 反馈给 parent。（`overview.md`；`decisions.md` D-023）
- **resolve()**：权威值到达时判 `î ≈ i`、选 Disposition 并沿依赖闭包传播的关键路径操作，由 Assumption Manager 执行。（`spec-exec-abstraction.md` §10.4 Assumption Manager）
- **Salvage**：失败后回收部分进展（`progress() / handoff()` 一等抽象）；warm-start、partial-token accept、promote 的共性。（`spec-exec-abstraction.md` §2 轴 6）
- **PublishOutcome**：真实 operation 完成后把 outcome 发布到 OutcomeSlot 的机制操作；由 Operation wrapper / Hook / effect broker 自动执行，不作为独立 Agent-facing 命令。（`overview.md`；`decisions.md` D-037）

## 组件

- **Policy Engine**：六类 policy 插件的宿主（Proposer / Equivalence / Topology / CostGate / Salvage / Compensation），负责加载期约束校验和运行期分发。（`spec-exec-abstraction.md` §10.4）
- **Assumption Manager**：全系统唯一"谁依赖谁"真相源；假设 DAG 与实例树是同一状态的两个索引（by_assumption / by_instance）。执行 resolve()，强制信息流约束（非空 AssumptionSet 的数据不得流向空容器）。（`spec-exec-abstraction.md` §10.4；`speculation-decisions.md` D-019）
- **Scheduler**：负责准入、预算、优先级、抢占、过期丢弃和反向调度；是全系统唯一看到全局资源压力的控制面组件。（`spec-exec-abstraction.md` §10.4；`speculation-decisions.md` D-011/D-020/D-021）
- **Effect Coordinator**：控制面内部的效果协调者；持有 EffectClass × StateDomain 规则、Site & Effect Catalog 和协调计划，经 Harness Runtime Boundary 请求 Runtime Mechanism 执行 effect transition。它不持有真实 staged effect、obligation 或 materialization 状态。（`spec-exec-abstraction.md` §10.4）
- **Ledger**：记录（append-only 事件流）/ 回放（确定性重跑）/ 校验（契约对撞：声明值 vs 推导上界 vs shadow 实测）；成本归属三分分类。异步旁路，不在关键路径上。（`spec-exec-abstraction.md` §10.4；`speculation-decisions.md` D-021）
- **Harness Runtime Boundary**：Runtime Adapter 侧面向控制面的不变部分，定义稳定、typed、可版本化的 Runtime operation/event contract；控制面只依赖此边界。（`../architecture.md` §1）
- **Runtime Adapter**：Runtime Adapter 侧面向某个具体 Runtime 的可变部分，把稳定 contract 映射为该 Runtime 的能力、原语、transport 与事件；它与 Harness Runtime Boundary 不是同一个组件。（`../architecture.md` §1）
- **ASC-Daemon**：Agent 接入侧部署在 Branch 隔离环境内的本地守护进程，承载 Agent-facing 控制入口、上下文绑定、身份/AssumptionSet 同步与生命周期消息。（`spec-exec-abstraction.md` §10.4；术语收敛）
- **Interposer**：数据面，请求路径上拦截四接缝（LLM 调用、工具分发、轨迹写入、调度）；= Hook / effect runner。与 ASC-Daemon 分离（请求路径延迟敏感 vs 旁路）。（`spec-exec-abstraction.md` §10.4；术语收敛）
- **Entry API**：控制面侧的稳定入口；接收 Agent 接入侧形成的请求并交给 Harness Core，不构成独立一级侧面，也不直接接触 Runtime Mechanism。（`decisions.md` D-038；`../architecture.md` §1）

## 状态与生命周期

- **role**：`Authority | Speculative(d)`。不是静态标签，是 `AssumptionSet == ∅` 的函数；promote 就是这个谓词翻转。（`spec-exec-abstraction.md` §10.1）
- **grain**：`own-env | shared-env`；shared-env 仅对 PURE 效果开放，隔离粒度由 EffectClass 决定。（`spec-exec-abstraction.md` §10.1）
- **lifecycle**：`Forked → Running → Resolving → {Reused, Promoted, Discarded, Preempted, Salvaged}`。（`spec-exec-abstraction.md` §10.1）
- **obligation**：未验证前提随 subtree 保留的义务；在验证前不能 Accept。Assumption 未确认即 obligation。（`overview.md`；`decisions.md` D-025）
- **capability**：resolver / validator 等授权能力；direct children 的 resolver capability 默认由 parent 持有，可显式委托给 Harness。（`overview.md`；`decisions.md` D-022）

## 机制与概念

- **Policy Decision**：Policy 根据任务、上下文、资源和 Runtime 能力作出的可替换策略决策；当前六项代表性决策及其阶段映射见 [`policy-decisions.md`](policy-decisions.md)。（`../mental-model.md` §3.1）
- **commit barrier**：效果释放屏障；照抄处理器 commit/retire 语义（"write operations are buffered but not allowed to execute until the instruction is committed"）。（`spec-exec-abstraction.md` §4）
- **规范化资源记账**：成本三分 —— (a) 无论如何都要花的（命中时投机 work 即权威 work）/ (b) 纯浪费 / (c) 部分成本（被抢占/取消时已消耗部分）。口径必须包含 token 之外的资源。（`spec-exec-abstraction.md` §5；`speculation-decisions.md` D-012）
- **Shadow / Replay**：投机与串行同跑、只提交串行结果；同一套基础设施服务 fidelity 审计、predictor 数据管道、cost 校准与上线灰度（offline replay → shadow → canary → online → drift kill-switch）。（`spec-exec-abstraction.md` §7；`speculation-decisions.md` D-014）
- **透明嵌套**：投机实例与权威实例二进制同构，实例不知道自己是投机的；底座追踪 provenance、闸门设在效果层。（`spec-exec-abstraction.md` §9.5）
- **反射 gating**：Cost / admission 策略必须知道自身存活概率：`should_speculate(..., survival)`；底座透传 `P(实例存活)`，防止 depth-2 实例按 depth-0 的账过度投机。（`spec-exec-abstraction.md` §9.5）
- **Speculation Contract**：跨层双向协商 —— 上层报预测置信度、验证条件、失败代价；下层报隔离能力、隔离代价、可用纪律集合。静态约束求解是它的下层形态。（`spec-exec-abstraction.md` §11.1(B)；`speculation-decisions.md` D-027）

## 范围决策

- **engine plane**：不做（D-013 修订）。KV cache 层 commit、partial-token accept、引擎压力调度、sequence residency 全部排除；跨层可见性降级为 API 可观测信号（延迟 / 限流 / 配额）。（`speculation-decisions.md` D-013/D-011/D-024/D-028）
- **Agent Boundary / Isolation Boundary**：必需的两条边界；Engine Boundary 在范围外。（`speculation-decisions.md` D-024）
