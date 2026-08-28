# AScout 心智模型

> 本文是理解 AScout 的起点：先给出 AScout 的定位（定义、问题、目标、边界），再介绍它的工作模型及其支撑模型。本文只记录已经确定的目标语义；问题与目标的完整论证见 [问题与目标](problems-and-goals.md)，设计原则见 [设计原则](design-principles.md)，逐条决策的权威出处见 [设计决策](design/speculation-decisions.md) 与 [状态与生命周期语义](design/overview.md)，系统结构见 [架构](architecture.md)。

## 1. 定位

AScout（Agent Scout）是面向 Agent 投机执行的通用底座：将投机执行所需的多种能力抽象为统一机制，使开发者能够通过组合与替换这些能力，构建出不同的执行策略。

投机执行想要解决的问题是：Agent 在执行过程中经常需要等待一个慢结果——远程 API、用户决策、CI 测试——它会阻塞后续推理执行，甚至主导整个执行流程。投机执行允许在结果尚未到达时按预测提前推进后续步骤，从而加速整个执行过程；而它所依赖的预测、验证、隔离、取消、复用与副作用提交，正是 AScout 抽象为统一机制的那组能力。

现有投机执行工作大多针对特定执行对象（tool call、plan、workspace branch）或特定状态域设计，能力与任务绑定，方案之间无法组合，也无法让不同任务选择不同策略。这些问题集中表现为三个缺口——策略孤岛、隔离碎片化、上下层脱节，具体论证与 AScout 的目标在 [问题与目标](problems-and-goals.md) 中展开。

AScout 的能力体现在三个层面。
- 策略层：任意算法都可以作为一条 policy 接入，预测器、等价判定、拓扑、成本、补偿等特征可独立替换、组合，并按任务选择。
- 状态域层：文件系统、进程、内存、GUI 以及 network、database、email、payment 等外部状态域，由同一套效果管理机制统一处理，不必为每种状态域另建隔离与副作用方案。
- 契约层：上层算法与下层隔离机制通过 Speculation Contract 双向协商——上层声明预测置信度、验证条件与失败代价，下层声明隔离能力、隔离代价与可用纪律。

落在使用上：
- 端用户提交任务、得到结果，无需感知背后的投机分支；
- Agent 开发者接入后不必重写执行逻辑；
- 部署方（Operator）按部署需要选择 policy 与隔离后端、设定预算；
- Policy 作者只需专注于算法设计（如何预测、如何判定命中、如何选择拓扑与成本），无需关心底层隔离与能力如何实现——这些由底座统一提供。

AScout 不是某个固定算法，也不承诺每个任务必然加速；是否投机、如何投机由控制面根据所绑定的 Policy 决定，投机与否本身就是 Policy 的决策内容。

## 2. AScout 的工作模型

### 2.1 投机执行过程

具体而言，一次投机执行可以描述为以下运行流程。

当 Agent 正在进行某一特定行为时，这一行为（称为**权威行为**）会被捕获并触发投机执行系统。系统首先根据 Agent 当前状态、系统资源条件和投机执行深度等因素，判断当前是否适合进行投机执行。

当判定适合投机时，系统预测权威行为的 N 个可能结果，并将这些结果视作后续行为的输入。据此创建 N 个投机执行过程并运行——这些过程彼此隔离，互不影响，也不影响 Agent 已有行为的执行。

当权威行为完成时，系统将权威结果与各预测结果进行对照，从中选择至多一个（也可以一个都不选）作为匹配结果。不匹配的投机执行过程被终止并回收；匹配的过程则复用其结果。至此，一次投机执行结束。

需要注意：上述流程只描述**单一投机执行过程**的最小工作模型；当嵌套投机等复杂行为共同作用时，系统完整流程由多个这样的最小过程组合而成，并不限于这一线性描述。

投机执行由一组可组合的阶段构成，而不是固定的线性流程。Policy 可以根据任务、执行对象、可用信息和 Runtime 能力选择其中的阶段，并决定各阶段的具体实现与组合方式。某些阶段可以缺失，但被启用的阶段仍需遵守必要的数据依赖和 AScout 固定的正确性、隔离、资源与副作用约束。

一个投机执行过程可以总结为以下阶段：机会发现 -> 准入判断 -> 预测生成 -> 过程执行 -> 验证 -> 处置与提交

具体来说

1. **机会发现**：按照 Policy 规定的方式观察 Agent 或其他执行对象，在检测到特定类型的行为、命令或执行步骤时，识别潜在的投机机会。
2. **准入判断**：由 Policy 判断当前机会是否适合投机，并综合考虑效果类型、资源条件、预测条件、隔离能力和预期收益。
3. **预测生成**：产生一个或多个 Proposal，为后续投机工作提供暂时依据。Proposal 可以来自模型、规则、历史轨迹、外部系统或其他来源。
4. **过程执行**：基于 Proposal 提前执行投机工作。投机工作可以是模型推理、工具调用、命令执行、远程请求或其他计算。
5. **验证**：获取用于验证 Proposal 的权威依据，并按 Policy 规定的条件判断 Proposal 是否兼容或达到可接受标准。权威依据可以来自真实执行、用户输入、环境反馈、远程服务或 validator；没有权威依据时，验证改用 validator 或其他可接受性判断。
6. **处置与提交**：根据验证结果复用、继续、取消、重新执行、清理、残值回收、补偿或以其他方式处理投机工作，并将符合条件的结果、状态变化和外部效果纳入后续执行；没有需要提交的结果或副作用时，可以不设置独立的提交阶段。

这些阶段在职责和实现上相互独立，Policy 可以选择、替换和组合它们，但并不意味着阶段之间没有任何关系。例如，基于 Proposal 的投机工作需要先获得某种暂时依据；没有权威依据时，验证必须改用 validator 或其他可接受性判断；结果和副作用在满足相应条件前不能进入最终执行状态。Policy 决定执行策略，Mechanism 保证策略在既定约束内执行。

### 2.2 投机边：投机执行的最小结构模型

2.1 描述的是投机执行的过程；同一件事在结构上可以压缩为执行图上的一条依赖边。边的起点 `u` 代表一个会产生结果的工作，终点 `v` 代表依赖该结果才能继续的后续工作。当起点的结果 `i` 尚未确定时，系统可以先对结果进行预测，用预测值 `î` 替代 `i`，让终点暂时依据预测继续执行。

2.1 中的元素在这条边上都有对应：权威行为对应边的起点，预测结果对应 `î`，后续行为对应边的终点。N 个预测就是 N 条并行的边；嵌套投机则是边的多层连接。因此投机边描述“哪个结果尚未确定”以及“哪个后续工作可以提前执行”，可以统一表示模型推理、命令执行、工具调用、远程请求和其他计算——它们的差异只是边的两端是什么、谁提供权威依据、投机工作是什么，以及各自需要什么隔离和效果处理能力；投机过程本身不需要为每一种对象重新定义一套基本流程。

在需要区分权威路径和投机路径时，起点 `u` 有两个实现：权威实现 `A_u`（产出真值 `i`）与投机实现 `S_u`（产出预测 `î`）。`A_u` 是否启动、何时启动（Eager / Lazy / Absent）属于轴体系的权威启动轴（见 §3.1 轴 1），由 Policy 选择，不是投机边模型的固有属性。

### 2.3 Policy、Mechanism 与 Speculation Contract

Policy 定义投机执行过程本身：它决定观察什么、如何发现投机机会、哪些机会适合投机、预测什么、如何展开投机工作、如何验证以及如何处置投机工作。不同 Policy 可以选择不同的阶段组合，从而支持不同任务和执行对象的投机策略。

AScout 的 Mechanism 则提供 Policy 无法自行可靠实现的执行、隔离、验证、取消、资源和效果管理能力。Policy 作者面向 AScout 提供的抽象接口设计策略，无需了解具体隔离后端如何创建执行环境、如何管理资源或如何清理失败分支。底座根据 Policy 的需要和 Runtime 的实际能力进行适配，并阻止策略违反固定的正确性、隔离、资源所有权和副作用约束。

Policy 与底层机制通过 **Speculation Contract** 协作。Policy 一侧声明预测置信度、验证条件和失败代价；底层机制一侧声明可提供的隔离能力、隔离代价和可用的效果处理方式。只有双方能力匹配时，某种 Policy 才能在特定任务和 Runtime 上启用。

## 3. 支撑模型

### 3.1 轴体系：同一坐标系

轴体系是底座的统一坐标系（编号沿用决策记录；Fidelity 是导出不变量，不是独立轴）：

| 轴 | 管什么 | 取值 |
|---|---|---|
| 轴 1 权威启动 | `A_u` 跑不跑、何时跑 | Eager / Lazy / Absent |
| 轴 2 拓扑 | 投机树的宽度与深度 | breadth × depth |
| 轴 3 预测器 | 怎么猜 `î` | 小模型 / 自投机 / pattern / 在线记忆 … |
| 轴 4 等价 `≈` | 怎么算"猜中了" | L0 语法 / L1 语义 / L2 效果同余 / L3 结果可接受 |
| 轴 5 权威 | 谁产出真值 `i` | 模型 / 环境 / 人 / validator … |
| 轴 6 处置 | 真值到达后怎么处理投机结果 | Reuse / Promote / Discard / Preempt / Re-execute / Compensate / Overwrite / Salvage |
| Fidelity（导出） | 系统层承诺的保真等级 | Trajectory / Outcome / Bounded-degradation / Best-effort |

每一条轴独立取值，论文之间的差异全部落成轴上的取值，而不是各自发明机制。

### 3.2 投机结果与验证

投机过程中的核心对象包括 Proposal、Authority、Equivalence 和 Disposition。Proposal 是投机路径使用的预测值；Authority 是产生真实结果或权威判断的主体；Equivalence 描述预测与权威结果如何被判定为兼容；Disposition 描述验证后如何处置投机工作。

这些对象共同描述“预测了什么、谁提供依据、怎样判断命中以及命中或失败后怎么办”。它们是投机边生命周期的具体组成部分，而不是独立于执行过程之外的另一套机制。

### 3.3 投机依赖与组合：Assumption DAG

多个投机同时在飞、且互相依赖时，需要一套通用语义。

每个投机任务携带 **AssumptionSet**：它依赖的未验证假设的闭包，其中 `Assumption = (site_id, proposal_id)` 表示"proposal 成立"这一前提。

- `AssumptionSet == ∅` ⟺ 权威工作；
- 某假设被 **confirm** → 从所有任务中移除，集合变空的可转正（commit / promote）；
- 某假设被 **refute** → 依赖它的整个子图级联进入 Salvage / Discard；
- 投机任务之间可以互相依赖，集合取并集。

顺带解决一个致命问题：**可见性泄漏**。非空 AssumptionSet 的数据只能流向同样非空的容器，唯一降级通道是 `resolve()`——一条可静态检查的信息流约束。

Assumption DAG 同时统一了级联取消、不一致子树剪枝、坏前缀重建等局部方案，并让**嵌套投机**（投机之上再投机）成为组合性的自然推论。

### 3.4 状态与副作用管理：EffectClass × StateDomain

投机的最大价值洼地在效果层：多数现有工作只能投机只读操作。底座提供真正的效果管理，把适用范围从只读扩展到可写。

**EffectClass**（效果纪律）与 **StateDomain**（状态域）正交：

| EffectClass | 含义 |
|---|---|
| PURE | 无外部效果，随便投机 |
| IDEMPOTENT | 重放收敛到同一状态 |
| STAGEABLE | 效果可缓冲，commit barrier 后释放 |
| TRANSFORMABLE | 存在非变异的等价执行（dry-run / staging） |
| COMPENSABLE | 可补偿（refund / replace） |
| OVERWRITABLE | last-write-wins 收敛 |
| IRREVERSIBLE | 不可投机，硬闸 |

| StateDomain | 典型可用纪律 | 隔离机制 |
|---|---|---|
| filesystem | STAGEABLE | overlayfs / union mount |
| process, memory | STAGEABLE | namespace / COW fork / VM 快照 |
| GUI | TRANSFORMABLE | 影子渲染 / 离屏合成 |
| network (read) | PURE | 直通 |
| network (write) | TRANSFORMABLE / STAGEABLE | 录制回放 / 代理拦截 |
| database | STAGEABLE | 未提交事务 / savepoint |
| email, payment | COMPENSABLE / IRREVERSIBLE | outbox 延迟投递 / 硬闸 |

二维交叉决定每一种具体隔离机制。效果缓冲到 **commit barrier**（照抄处理器的 commit/retire 语义）之后才释放。外部效果只在 service adapter 声明的 **observation surface** 与 **guarantee horizon** 内获得 sibling non-interference 与 winner materialization 保证。

### 3.5 资源与准入

投机收益是负载相关的，高负载下可以为负。因此底座必须提供：

1. **准入控制**：executable / policy-safe / 置信度与预期收益 / 预算内四道闸；
2. **优先级与抢占**：投机任务低优先级、可抢占；命中转正后 non-preemptible；
3. **过期丢弃**：等待超时的投机请求直接丢弃（与抢占语义不同）；
4. **跨层可见性**：推理引擎压力降级为 API 可观测信号（延迟 / 限流 / 配额）；
5. **反向调度**：小投机请求插队，避免排队主导延迟。

**规范化记账**三分：(a) 无论如何都要花的 / (b) 纯浪费 / (c) 被抢占取消时已消耗的部分成本。口径必须包含 token 之外的资源，否则无法横向比较、无法做全局预算、无法做熔断。

### 3.6 保真度与运行验证

lossless 不是布尔值，而是契约等级。底座把 Fidelity 变成可声明、可强制、可审计的属性：

1. **静态**：由轴设置推出保真度上界；
2. **加载期**：声明的契约与推导上界对撞，不一致则拒绝加载；
3. **运行期**：shadow mode（投机与串行同跑、只提交串行结果）实测是否达成。

同一套 shadow / replay 基础设施同时服务：Fidelity 审计、predictor 数据管道、成本校准与上线灰度（offline replay → shadow → canary → online → drift kill-switch）。

## 4. 这些模型如何落到系统上

本文是底座的抽象心智。落到具体系统时，它承载为：

- 一棵 **Speculative Tree**（一个 AScoutAgent 控制域），**Branch** 是树上的计算延续，**Instance** 是运行时执行单元（Branch ↔ Instance 1:1）；
- **branch / commit / accept / discard** 是生命周期操作，**Branch barrier / Resolution barrier** 是原子边界；
- **pending-result boundary** 是"结果未到"的时机，**Site** 是它上面被标注可投机的依赖边；
- delayed-choice coding 是第一个端到端验证 workload。

这些是同一系统的机制表达，不是另一套系统；术语对应见 [术语映射表](design/terminology.md)。

## 5. 文档地图

- 问题与目标、设计原则：`problems-and-goals.md`、`design-principles.md`
- 工作模型与支撑模型的逐条决策：`design/speculation-decisions.md`
- 机制视角语义（状态与生命周期）：`design/overview.md`、`design/decisions.md`
- 研究推导与论文调研：`development/spec-exec-abstraction.md`
- 系统结构：`architecture.md`
- 术语表：`design/glossary.md`
