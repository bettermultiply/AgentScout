# 术语表

本表给出 ASCout 每个术语的一句话定义与权威出处。完整语义以各设计文档为准：目标语义见 `docs/design/overview.md` 与 `docs/design/speculation-decisions.md`；两套视角（Branch tree 机制表述 ↔ 投机执行底座抽象）的对应关系见 [`terminology.md`](terminology.md)。术语以投机执行底座视角为当前设计目标语言。

## 系统级

- **ASCout（Agent Scout）**：系统名，原名 Parallel Branch World。面向 Agent 投机执行的 Outer Harness。（`speculation-decisions.md` 术语收敛）
- **Speculative Tree**：原 BranchTree。ASCout 的核心数据结构：Branch 之间的派生关系树，从一个 root Branch 开始，描述计算世界的分支结构。（`overview.md` §Branch 与 Branch tree；术语收敛）
- **task**：端用户提交的准入单元（任务表条目）。一个 task 对应一棵 Speculative Tree（一个 ASCoutAgent 控制域）；准入时绑定 policy profile 并建立 root Authority 实例。（`spec-exec-abstraction.md` §10.4 API Entry）
- **ASCoutAgent**：原 PbwAgent。一棵 Speculative Tree 的控制域：Branch identity、capability、commit/discard 与 resolution 都只在这棵 tree 内有意义。task 的系统视角名字。（`overview.md` §概述）
- **Control Plane**：Harness 侧整体：Policy Engine、Assumption Manager、Scheduler、Effect Coordinator、Ledger、Runtime Adapter。（`spec-exec-abstraction.md` §10.1）
- **Instance 侧**：Branch 隔离环境内：Interposer、Agent、ASC-Daemon、Isolation Env；结构同构、数量不定、各自携带状态。（`spec-exec-abstraction.md` §10.1）

## 实体

- **Branch**：Speculative Tree 中的结构实体（节点）。指称扩展为以该节点为根的**整个子树**，不再区分 `subtree(A)`；提交、丢弃、接受都以整个子树为单位。每个 Branch 节点在运行时由一个 Instance 承载（1:1）。（`overview.md` §Branch 与 Branch tree；术语收敛）
- **Instance**：运行时执行单元，携带 AssumptionSet、role、grain 与 lifecycle；与 Branch 1:1。投机实例与权威实例二进制同构（透明嵌套的前提）。（`spec-exec-abstraction.md` §10.1、§9.5）
- **Proposal** `î`：一次猜测：`{value, confidence, provenance, source}`。候选 Branch 基于它提前执行后续步骤。原 hypothesis 的预测值部分。（`spec-exec-abstraction.md` §1；术语收敛）
- **Assumption**：`(site_id, proposal_id)`，读作"proposal X 成立这一假设"。原 hypothesis 的 compatibility 条件（exact payload / predicate / virtual-value mapping）。（`spec-exec-abstraction.md` §3；`decisions.md` D-025）
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
- **commit / commit request**：candidate 显式提交自己的子树（原 "commit proposal"，改名让位给 Proposal）。可 conditional commit：未验证前提随 subtree 保留为 obligation，验证前不能 Accept。（`decisions.md` D-025、D-037；术语收敛）
- **accept**：接受一个 winner subtree（裁决 = Reuse / Promote）；候选被接受后保留自身 Branch identity 并替换 parent 在树中的位置。（`overview.md`；`decisions.md` D-009/D-023）
- **discard**：丢弃整个 child subtree（= Disposition 的 Discard / Preempt）；不要求 candidate ready，不自动把 partial state 反馈给 parent。（`overview.md`；`decisions.md` D-023）
- **resolve()**：权威值到达时判 `î ≈ i`、选 Disposition 并沿依赖闭包传播的关键路径操作，由 Assumption Manager 执行。（`spec-exec-abstraction.md` §10.4 Assumption Manager）
- **Salvage**：失败后回收部分进展（`progress() / handoff()` 一等抽象）；warm-start、partial-token accept、promote 的共性。（`spec-exec-abstraction.md` §2 轴 6）
- **PublishOutcome**：真实 operation 完成后把 outcome 发布到 OutcomeSlot 的机制操作；由 Operation wrapper / Hook / effect broker 自动执行，不作为独立 Agent-facing 命令。（`overview.md`；`decisions.md` D-037）

## 组件

- **Policy Engine**：六类 policy 插件的宿主（Proposer / Equivalence / Topology / CostGate / Salvage / Compensation）+ 加载期编译校验（轴间约束求解、跨层契约协商）+ 运行期被 mechanism 反向调用分发。原 Policy Compiler + Policy Set。（`spec-exec-abstraction.md` §10.4；术语收敛）
- **Assumption Manager**：全系统唯一"谁依赖谁"真相源；假设 DAG 与实例树是同一状态的两个索引（by_assumption / by_instance）。执行 resolve()，强制信息流约束（非空 AssumptionSet 的数据不得流向空容器）。（`spec-exec-abstraction.md` §10.4；`speculation-decisions.md` D-019）
- **Scheduler**：准入（四道闸）、预算（热计数器）、优先级与抢占、过期丢弃、反向调度；全系统唯一看到全局资源压力的地方。原 Frontier Scheduler。（`spec-exec-abstraction.md` §10.4；`speculation-decisions.md` D-011/D-020/D-021；术语收敛）
- **Effect Coordinator**：所有对外效果的唯一出口；持有 EffectClass × StateDomain 矩阵与 Site & Effect Catalog；staging / commit barrier / 补偿。（`spec-exec-abstraction.md` §10.4）
- **Ledger**：记录（append-only 事件流）/ 回放（确定性重跑）/ 校验（契约对撞：声明值 vs 推导上界 vs shadow 实测）；成本归属三分分类。异步旁路，不在关键路径上。（`spec-exec-abstraction.md` §10.4；`speculation-decisions.md` D-021）
- **Runtime Adapter**：统一 fork / 销毁 / 隔离 / 压力上报接口，屏蔽后端差异；能力差异通过契约协商上报（同 policy 在不同后端自动得到不同激进度）。= Harness Runtime Boundary。（`spec-exec-abstraction.md` §10.4；术语收敛）
- **ASC-Daemon**：Branch 隔离环境内的本地守护进程：Agent-facing 控制入口（原 Local Controller）+ 控制面旁路（原 Local Daemon：身份、AssumptionSet 同步、生命周期、Proposer 上下文句柄）。（`spec-exec-abstraction.md` §10.4；术语收敛）
- **Interposer**：数据面，请求路径上拦截四接缝（LLM 调用、工具分发、轨迹写入、调度）；= Hook / effect runner。与 ASC-Daemon 分离（请求路径延迟敏感 vs 旁路）。（`spec-exec-abstraction.md` §10.4；术语收敛）
- **API Entry**：对外入口职责（任务提交/查询、policy 绑定、在飞任务表）。不画在架构图上、不单独设包：职责分散到 Entry API / Harness Core / Policy Engine。（`speculation-decisions.md` 组件收敛）

## 状态与生命周期

- **role**：`Authority | Speculative(d)`。不是静态标签，是 `AssumptionSet == ∅` 的函数；promote 就是这个谓词翻转。（`spec-exec-abstraction.md` §10.1）
- **grain**：`own-env | shared-env`；shared-env 仅对 PURE 效果开放，隔离粒度由 EffectClass 决定。（`spec-exec-abstraction.md` §10.1）
- **lifecycle**：`Forked → Running → Resolving → {Reused, Promoted, Discarded, Preempted, Salvaged}`。（`spec-exec-abstraction.md` §10.1）
- **obligation**：未验证前提随 subtree 保留的义务；在验证前不能 Accept。Assumption 未确认即 obligation。（`overview.md`；`decisions.md` D-025）
- **capability**：resolver / validator 等授权能力；direct children 的 resolver capability 默认由 parent 持有，可显式委托给 Harness。（`overview.md`；`decisions.md` D-022）

## 机制与概念

- **轴体系**：七轴正交模型 —— 轴 1 权威启动（Eager / Lazy / Absent）、轴 2 拓扑（breadth × depth）、轴 3 预测器、轴 4 等价、轴 5 权威、轴 6 处置、Fidelity 为导出不变量（不是轴）。（`spec-exec-abstraction.md` §2；`speculation-decisions.md` D-001..D-008）
- **commit barrier**：效果释放屏障；照抄处理器 commit/retire 语义（"write operations are buffered but not allowed to execute until the instruction is committed"）。（`spec-exec-abstraction.md` §4）
- **规范化资源记账**：成本三分 —— (a) 无论如何都要花的（命中时投机 work 即权威 work）/ (b) 纯浪费 / (c) 部分成本（被抢占/取消时已消耗部分）。口径必须包含 token 之外的资源。（`spec-exec-abstraction.md` §5；`speculation-decisions.md` D-012）
- **Shadow / Replay**：投机与串行同跑、只提交串行结果；同一套基础设施服务 fidelity 审计、predictor 数据管道、cost 校准与上线灰度（offline replay → shadow → canary → online → drift kill-switch）。（`spec-exec-abstraction.md` §7；`speculation-decisions.md` D-014）
- **透明嵌套**：投机实例与权威实例二进制同构，实例不知道自己是投机的；底座追踪 provenance、闸门设在效果层。（`spec-exec-abstraction.md` §9.5）
- **反射 gating**：Cost / admission 策略必须知道自身存活概率：`should_speculate(..., survival)`；底座透传 `P(实例存活)`，防止 depth-2 实例按 depth-0 的账过度投机。（`spec-exec-abstraction.md` §9.5）
- **Speculation Contract**：跨层双向协商 —— 上层报预测置信度、验证条件、失败代价；下层报隔离能力、隔离代价、可用纪律集合。静态约束求解是它的下层形态。（`spec-exec-abstraction.md` §11.1(B)；`speculation-decisions.md` D-027）

## 范围决策

- **engine plane**：不做（D-013 修订）。KV cache 层 commit、partial-token accept、引擎压力调度、sequence residency 全部排除；跨层可见性降级为 API 可观测信号（延迟 / 限流 / 配额）。（`speculation-decisions.md` D-013/D-011/D-024/D-028）
- **Agent Boundary / Isolation Boundary**：必需的两条边界；Engine Boundary 在范围外。（`speculation-decisions.md` D-024）
