# 术语映射表

本表是 `docs/design/decisions.md`（Speculative Tree 机制视角，D-001..D-039）与 `docs/design/speculation-decisions.md`（投机执行底座视角，D-001..D-028）两套决策记录的桥梁。二者描述同一个 AScout 系统，各自在声明的范围内具有权威性，并通过本表使用同一组现行术语。每个术语的一句话定义见 [`glossary.md`](glossary.md)。

**权威声明**：一级架构以 `decisions.md` D-039 和 [`../architecture.md`](../architecture.md) 为准；Branch 生命周期与状态语义以 `decisions.md` 和 [`overview.md`](overview.md) 为准；投机执行抽象以 `speculation-decisions.md` 为准。不同文档若发生冲突，必须显式收敛并同步修改，不能让隐式优先级维持矛盾。

## 系统级

| 术语 | 说明 |
|---|---|
| **AScout**（Agent Scout） | 面向 Agent 投机执行的通用底座，将投机执行所需的能力抽象为统一机制，使开发者能够组合与替换这些能力、构建不同的执行策略 |
| **Speculative Tree** | AScout 的核心控制结构：从 root 开始记录 Branch 之间的派生关系 |
| **task** | 端用户提交的准入单元。一个 task 对应一棵 Speculative Tree（一个 AScoutAgent 控制域），准入时绑定 policy profile 并建立 root Authority 实例 |

## 实体

| 底座术语 | Speculative Tree 视角 | 状态 |
|---|---|---|
| **Branch** | （结构视角，指称扩展为整个子树，不再区分 `subtree(A)`） | 已确认 |
| **Instance** | （运行时视角：单个执行单元，携带 AssumptionSet / role = Authority \| Speculative(d) / grain = own-env \| shared-env） | 已确认 |
| **Proposal** `î` | 投机预测值（`{value, confidence, provenance, source}`） | 已确认 |
| **Assumption** / AssumptionSet | Proposal 成立这一未验证前提及其依赖闭包，随 subtree 保留为 unresolved obligation | 已确认 |
| **Site** | pending-result boundary 上被标注可投机、实际启动投机的依赖边/节点（结构语义）；pending-result boundary 是"结果未到"的执行边界（时机语义），两者并存 | 已确认 |
| **SpecTask** | candidate Branch（基于 Proposal 提前执行后续步骤的工作单元） | 已确认 |
| **Authority**（`A_u`） | parent（producer lane）+ 真实 operation + resolver capability / delegation | 已确认 |
| **Predictor** `S_u` | （新概念，Branch 视角无对应） | 底座独有 |
| **Reconciliation** | Resolution barrier 中"判 `î ≈ i` 并选处置"的部分 | 已确认（层间映射） |
| **OutcomeSlot** | 同名同义（真实 outcome / 权威值投递槽位） | 已确认 |
| **commit barrier** | Resolution barrier / Accept gate 的效果侧 | 已确认（层间映射） |

## 操作

| 底座术语 | Speculative Tree 视角 | 状态 |
|---|---|---|
| 在 Site 启动 SpecTask | `branch`（创建 candidate） | 已确认 |
| **commit**（operation）/ **commit request**（中立值） | Candidate 显式提交自己的 subtree | 已确认 |
| **Reuse** / **Promote** | `accept`（winner 转正；Promote = 在飞任务转正为权威身份） | 已确认（层间映射） |
| **Discard** / **Preempt** | `discard`（含 live-child discard） | 已确认（层间映射） |
| **resolve()** | Resolution barrier 的验证部分（判 Proposal 兼容性 → 选处置） | 已确认（层间映射） |
| **Salvage** | （新概念，Branch 视角无对应；warm-start / 部分进展移交） | 底座独有 |
| **Equivalence** `≈`（L0–L3） | Proposal compatibility 条件（exact payload / predicate / virtual-value mapping） | 已确认（层间映射） |

## 一级架构

| 术语 | 定义 | 状态 |
|---|---|---|
| **Agent 接入侧** | Agent/product，以及 CLI、library、launcher、Agent integration、ASC-Daemon 与 Interposer 等接入职责；把 intent 送入控制面 | 已确认 |
| **控制面侧** | Harness Core 及其内部组件；拥有 AScoutAgent、Speculative Tree 控制状态、policy、校验、编排与事件解释 | 已确认 |
| **Runtime Adapter 侧** | 位于控制面与 Runtime Mechanism 之间，内部同时包含稳定 Harness Runtime Boundary 与面向具体 Runtime 的实际 Adapter | 已确认 |
| **Runtime Mechanism** | 拥有节点执行状态、资源、隔离、capability/obligation validation、barrier、原子转换与 cleanup | 已确认 |
| **Harness Runtime Boundary** | Runtime Adapter 侧面向控制面的不变部分；定义稳定、typed、可版本化的 operation/event contract | 已确认 |
| **Runtime Adapter** | Runtime Adapter 侧面向某个具体 Runtime 的可变部分；把稳定 contract 映射为该 Runtime 的能力、原语、transport 与事件 | 已确认 |

Harness Runtime Boundary 与 Runtime Adapter 同属 Runtime Adapter 侧，但不是同义词。

## 内部组件与运行对象

| 术语 | 所属 | 状态 |
|---|---|---|
| **Policy Engine** | 控制面内部（六类插件宿主、编译校验与运行期分发） | 已确认 |
| **Assumption Manager** | 控制面内部（resolve、Assumption DAG 与信息流约束） | 已确认 |
| **Scheduler** | 控制面内部（准入、预算、优先级、抢占与过期丢弃） | 已确认 |
| **Effect Coordinator** | 控制面内部（效果分类、计划与 transition 协调；不持有 Runtime staged effect 状态） | 已确认 |
| **Ledger** | 控制面内部（记录、回放、fidelity 审计与成本归属） | 已确认 |
| **ASC-Daemon** | Agent 接入侧；驻 Branch 隔离环境的本地守护进程 | 已确认 |
| **Interposer** | Agent 接入侧；请求路径的数据面接入 | 观察待确认 |
| **Entry API** | 控制面入口，不构成独立一级侧面 | 已确认 |
| **Instance** | Runtime Mechanism 管理的运行对象，不构成独立一级侧面 | 已确认 |

## 底座独有概念（Branch 视角无对应，直接吸收）

- 轴体系（轴 1 权威启动 / 轴 2 拓扑 / 轴 3 预测器 / 轴 4 等价 / 轴 5 权威 / 轴 6 处置 / Fidelity 导出不变量）
- EffectClass（PURE / IDEMPOTENT / STAGEABLE / TRANSFORMABLE / COMPENSABLE / OVERWRITABLE / IRREVERSIBLE）× StateDomain
- Fidelity 契约（Trajectory-equivalent / Outcome-equivalent / Bounded-degradation / Best-effort；可声明、可强制、可审计）
- Salvage（残值回收：`progress() / handoff()` 一等抽象）
- 规范化资源记账三分（本来就要花 / 纯浪费 / 部分成本）
- Shadow / Replay（fidelity 审计 + predictor 数据管道 + cost 校准 + 灰度）
- 透明嵌套 + 反射 gating（`should_speculate(..., survival)`）
- Speculation Contract（跨层双向协商）

## 范围决策

- **engine plane 不做**（D-013 修订）：KV cache 层 commit、partial-token accept、引擎压力调度、sequence residency 全部排除；跨层可见性降级为 API 可观测信号；接缝 1 去掉 prefix cache 共享。札记中的双层分析保留为论文背景与未来自托管候选。
- **Engine Boundary 在范围外**（D-024 修订）；Agent Boundary 与 Isolation Boundary 必需。
