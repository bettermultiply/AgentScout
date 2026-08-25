# AScout 心智模型

> 本文是理解 AScout 的起点：先说明 AScout 要解决的问题，再给出 AScout 赖以工作的核心模型。本文只记录已经确定的目标语义；逐条决策的权威出处见 [设计决策](design/speculation-decisions.md) 与 [状态与生命周期语义](design/overview.md)，系统结构见 [架构](architecture.md)。

## 1. 一句话定位

AScout（Agent Scout）是面向 Agent 投机执行的**通用底座**。

它不是某一种投机算法，也不承诺每个任务都必然加速；它提供的是让"预测、验证、隔离、取消、复用、副作用提交"这些能力可以组合、可以替换、可以跨任务选择的一层机制。任何投机算法都可以作为一条 policy 挂上来，任何状态域的效果都被同一套机制管理，上下层之间通过显式契约协商。

## 2. 问题陈述：现有工作缺什么

### 2.1 策略孤岛：每种方案只服务一种对象

现有 speculative execution 工作通常针对特定任务和执行对象设计：tool call、plan、function future、workspace branch 各是一套。缺少统一框架来管理预测、验证、隔离、取消和提交，导致方案之间无法组合，也无法让不同的任务选择不同的 policy。

### 2.2 隔离碎片化：隔离范围各不一样

不同方案覆盖的隔离范围不同：有的只能隔离结果或 future，有的能隔离 filesystem、process、memory 或 GUI。对于 network、database、email、payment 等外部副作用，通常只能禁止、延迟或依赖特定 sandbox。缺少一个能够跨任务、跨状态域、跨 speculative policy 统一管理执行与副作用的框架。

### 2.3 上下层脱节：算法与隔离机制之间没有契约

已有工作大多沿两条相对割裂的路径展开：

- 一类关注如何在 Agent 执行中预测并提前执行未来动作，通常通过限制可投机操作类型（read-only、idempotent、sandboxed）来规避副作用风险；
- 另一类关注系统层如何提供状态隔离、分支执行、事务提交或回滚，但没有建模上层投机算法的预测置信度、验证条件和失败代价。

换言之，现有研究缺少一种跨层的 **speculation contract**，把上层算法充分映射到下层执行中。

## 3. 目标与设计原则

### 目标

构建一个面向 speculative execution 的通用底座：允许不同任务选择不同的 speculation policy，并统一负责 tentative execution 的**隔离、调度、验证、取消、复用和副作用提交**。

### 设计原则

1. **Mechanism / Policy 划线**：底座提供算法不可修改的机制，算法以插件形式填入（详见 §5）。
2. **同一坐标系**：所有投机算法的差异都能落进一组正交轴上的取值，底座据此统一表达与强制。
3. **跨层契约**：上层报预测置信度、验证条件与失败代价，下层报隔离能力、隔离代价与可用纪律集合。
4. **Fidelity 可声明、可强制、可审计**：lossless 不是口头论证，而是加载期对撞 + 运行期 shadow 实测。

## 4. 核心模型

### 4.1 投机边：最小的统一单位

一条依赖边 `u → v`：`v` 消费 `u` 的输出 `i`。当 `i` 尚未由权威路径产出时，用预测值 `î` 替代 `i`，提前启动 `v`。

`u` 有两个实现：

- **权威实现 `A_u`**：产出真值 `i`（延迟高）；
- **投机实现 `S_u`**：产出预测 `î`（延迟低，即 Predictor）。

`A_u` 是否启动、何时启动（Eager / Lazy / Absent）是策略，不是模型的固有属性。

tool call、plan step、workspace branch、乃至人类回复，全都落在同一条投机边上——它们的差异只是这条边的位置、两侧延迟与谁是权威。这是底座能统一不同方案的原因。

### 4.2 轴体系：同一坐标系

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

### 4.3 组合性：Assumption DAG

多个投机同时在飞、且互相依赖时，需要一套通用语义。

每个投机任务携带 **AssumptionSet**：它依赖的未验证假设的闭包，其中 `Assumption = (site_id, proposal_id)` 表示"proposal 成立"这一前提。

- `AssumptionSet == ∅` ⟺ 权威工作；
- 某假设被 **confirm** → 从所有任务中移除，集合变空的可转正（commit / promote）；
- 某假设被 **refute** → 依赖它的整个子图级联进入 Salvage / Discard；
- 投机任务之间可以互相依赖，集合取并集。

顺带解决一个致命问题：**可见性泄漏**。非空 AssumptionSet 的数据只能流向同样非空的容器，唯一降级通道是 `resolve()`——一条可静态检查的信息流约束。

Assumption DAG 同时统一了级联取消、不一致子树剪枝、坏前缀重建等局部方案，并让**嵌套投机**（投机之上再投机）成为组合性的自然推论。

### 4.4 效果管理：EffectClass × StateDomain

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

### 4.5 资源、准入与记账

投机收益是负载相关的，高负载下可以为负。因此底座必须提供：

1. **准入控制**：executable / policy-safe / 置信度与预期收益 / 预算内四道闸；
2. **优先级与抢占**：投机任务低优先级、可抢占；命中转正后 non-preemptible；
3. **过期丢弃**：等待超时的投机请求直接丢弃（与抢占语义不同）；
4. **跨层可见性**：推理引擎压力降级为 API 可观测信号（延迟 / 限流 / 配额）；
5. **反向调度**：小投机请求插队，避免排队主导延迟。

**规范化记账**三分：(a) 无论如何都要花的 / (b) 纯浪费 / (c) 被抢占取消时已消耗的部分成本。口径必须包含 token 之外的资源，否则无法横向比较、无法做全局预算、无法做熔断。

### 4.6 保真度与验证

lossless 不是布尔值，而是契约等级。底座把 Fidelity 变成可声明、可强制、可审计的属性：

1. **静态**：由轴设置推出保真度上界；
2. **加载期**：声明的契约与推导上界对撞，不一致则拒绝加载；
3. **运行期**：shadow mode（投机与串行同跑、只提交串行结果）实测是否达成。

同一套 shadow / replay 基础设施同时服务：Fidelity 审计、predictor 数据管道、成本校准与上线灰度（offline replay → shadow → canary → online → drift kill-switch）。

### 4.7 Speculation Contract：跨层双向协商

上层与下层之间不是单向声明，而是双向协商：

- **上层 → 下层**：预测置信度、验证条件、失败代价；
- **下层 → 上层**：隔离能力、隔离代价、可用纪律集合。

后果：单向声明下底座只能拒绝不可行组合；双向协商下底座能回答"给定这个隔离后端，该算法最多能激进到什么程度"、"要支持该算法，隔离层至少要提供什么"。静态约束求解是它的下层形态。

## 5. Mechanism 与 Policy 划线

- **Mechanism（底座提供，算法不可改）**：Site 注册与 admissibility、隔离域、Reconciliation 状态机、Assumption DAG 与传播、Effect Gate / commit barrier、资源准入 / 优先级 / 抢占 / 背压、规范化记账、Fidelity 契约的强制与审计。
- **Policy（插件，有默认实现）**：Predictor、Equivalence、Authority launch、Topology、Cost model & gate、Salvage、Compensation。

判据：如果某个论文的机制被迫写进 Mechanism 层，说明抽象漏了一轴。

## 6. 这些模型如何落到系统上

本文是底座的抽象心智。落到具体系统时，它承载为：

- 一棵 **Speculative Tree**（一个 AScoutAgent 控制域），**Branch** 是树上的计算延续，**Instance** 是运行时执行单元（Branch ↔ Instance 1:1）；
- **branch / commit / accept / discard** 是生命周期操作，**Branch barrier / Resolution barrier** 是原子边界；
- **pending-result boundary** 是"结果未到"的时机，**Site** 是它上面被标注可投机的依赖边；
- delayed-choice coding 是第一个端到端验证 workload。

这些是同一系统的机制表达，不是另一套系统；术语对应见 [术语映射表](design/terminology.md)。

## 7. 文档地图

- 核心模型与逐条决策：`design/speculation-decisions.md`
- 机制视角语义（状态与生命周期）：`design/overview.md`、`design/decisions.md`
- 研究推导与论文调研：`development/spec-exec-abstraction.md`
- 系统结构：`architecture.md`
- 术语表：`design/glossary.md`
