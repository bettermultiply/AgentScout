# 术语映射表

本表是 `docs/design/decisions.md`（Branch tree 机制视角，D-001..D-038）与 `docs/design/speculation-decisions.md`（投机执行底座视角，D-001..D-028）两套决策记录的桥梁。投机执行底座就是当前系统（ASCout）本身的设计目标；两套记录是同一系统的两个视角，各自独立编号，通过本表对应。每个术语的一句话定义见 [`glossary.md`](glossary.md)。

**权威声明**：术语与语义以投机执行底座视角（`speculation-decisions.md`、`spec-exec-abstraction.md`）为当前设计目标语言；Branch tree 视角文档保留为机制实现表述。两套记录冲突时，以底座视角为准，并把差异同步到对应文档（AGENTS.md 设计原则 3）。

## 系统级

| 术语 | 说明 |
|---|---|
| **ASCout**（Agent Scout） | 系统名，原名 Parallel Branch World。crate/代码标识符/CLI 命令/仓库目录名暂不联动（仅文档改名） |
| **Speculative Tree** | 原 BranchTree。它是底座的数据结构：Branch 之间的派生关系树，从 root 开始 |
| **task** | 端用户提交的准入单元（API Entry 任务表条目）。一个 task 对应一棵 Speculative Tree（一个 ASCoutAgent 控制域），准入时绑定 policy profile 并建立 root Authority 实例 |

## 实体

| 底座术语 | Branch tree 视角 | 状态 |
|---|---|---|
| **Branch** | （结构视角，指称扩展为整个子树，不再区分 `subtree(A)`） | 已确认 |
| **Instance** | （运行时视角：单个执行单元，携带 AssumptionSet / role = Authority \| Speculative(d) / grain = own-env \| shared-env） | 已确认 |
| **Proposal** `î` | 原 hypothesis 的预测值部分（`{value, confidence, provenance, source}`） | 已确认（hypothesis 废弃） |
| **Assumption** / AssumptionSet | 原 hypothesis 的 compatibility 条件 / 未验证 premise（"proposal 成立"这一假设，随 subtree 保留为 unresolved obligation） | 已确认 |
| **Site** | pending-result boundary 上被标注可投机、实际启动投机的依赖边/节点（结构语义）；pending-result boundary 是"结果未到"的执行边界（时机语义），两者并存 | 已确认 |
| **SpecTask** | candidate Branch（基于 Proposal 提前执行后续步骤的工作单元） | 已确认 |
| **Authority**（`A_u`） | parent（producer lane）+ 真实 operation + resolver capability / delegation | 已确认 |
| **Predictor** `S_u` | （新概念，Branch 视角无对应） | 底座独有 |
| **Reconciliation** | Resolution barrier 中"判 `î ≈ i` 并选处置"的部分 | 已确认（层间映射） |
| **OutcomeSlot** | 同名同义（真实 outcome / 权威值投递槽位） | 已确认 |
| **commit barrier** | Resolution barrier / Accept gate 的效果侧 | 已确认（层间映射） |

## 操作

| 底座术语 | Branch tree 视角 | 状态 |
|---|---|---|
| 在 Site 启动 SpecTask | `branch`（创建 candidate） | 已确认 |
| **commit**（operation）/ **commit request**（中立值） | 原 "commit proposal"（提交提议），改名让位给 Proposal | 已确认 |
| **Reuse** / **Promote** | `accept`（winner 转正；Promote = 在飞任务转正为权威身份） | 已确认（层间映射） |
| **Discard** / **Preempt** | `discard`（含 live-child discard） | 已确认（层间映射） |
| **resolve()** | Resolution barrier 的验证部分（判 Proposal 兼容性 → 选处置） | 已确认（层间映射） |
| **Salvage** | （新概念，Branch 视角无对应；warm-start / 部分进展移交） | 底座独有 |
| **Equivalence** `≈`（L0–L3） | Proposal compatibility 条件（exact payload / predicate / virtual-value mapping） | 已确认（层间映射） |

## 组件与边界

| 底座术语 | Branch tree 视角 | 状态 |
|---|---|---|
| **Policy Engine** | （原 Policy Compiler + Policy Set 合并：六类插件宿主 + 编译校验 + 运行期分发） | 已确认 |
| **Assumption Manager** | （新组件：resolve / Assumption DAG / 信息流约束，坐在关键路径上） | 已确认 |
| **Scheduler** | （原 Frontier Scheduler：准入 / 预算 / 优先级 / 抢占 / 过期丢弃） | 已确认 |
| **Effect Coordinator** | effect runner / broker / service adapter（效果出口 + EffectClass × StateDomain） | 已确认 |
| **Ledger** | （新组件：记录 / 回放 / 校验，fidelity 审计与成本归属分类） | 已确认 |
| **Runtime Adapter** | Harness Runtime Boundary | 已确认 |
| **ASC-Daemon** | 原 Local Controller + Local Daemon 合并：Branch 隔离环境内的本地守护进程（Agent-facing 控制入口 + 控制面旁路） | 已确认（最终定名 ASC-Daemon） |
| **Interposer** | Hook / effect runner（数据面，请求路径上拦截四接缝，与 ASC-Daemon 分离） | 观察待确认 |
| **API Entry** | 不画在架构图上、不单独设包：任务提交/查询 → Entry API；policy 绑定 → Harness Core；任务表 → Harness Core；policy 声明 → Policy Engine | 已确认 |
| **Control Plane** | Harness 侧整体（Harness Core + ASCoutAgentRegistry + 各机制组件） | 已确认 |
| **Instance 侧**（Interposer + Agent + ASC-Daemon + Isolation Env） | Branch 隔离环境内（Local Controller + Agent + Runtime 节点状态） | 已确认 |

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
