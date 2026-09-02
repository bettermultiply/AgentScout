# Agent 投机执行底座：抽象设计札记

> 本文是投机执行底座的完整设计札记（论文调研、推导过程、候选方案与未来构想）。已收敛的最终设计/架构决策以 [`docs/design/speculation-decisions.md`](../design/speculation-decisions.md) 为准。
>
> **范围决策（2026-08，D-013 修订）**：engine plane（推理引擎钩子层）不在范围内。文档中的双层分析、engine plane 表格与 §10.7 的 Engine Boundary 均保留为论文调研背景与未来自托管时的候选。

## 1. 核心模型：一条投机边的生命周期

统一模型：**执行图上存在一条边 `u → v`，`v` 消费 `u` 的输出 `i`。在 `i` 尚未由权威路径产出时，用 `î` 替代 `i` 启动 `v`。**

`u` 和 `v` 是什么完全不重要，这正是 Speculative Actions 把 LLM 调用、工具调用、MCP 请求、**乃至人类回复**统统建模成 "API call" 的动机（它明确指出人类回复的延迟常常比自动化工具更长）。

**被加速的组件是谁也不重要**：Skim 猜的是整条子轨迹的终点（URL + 答案 schema），DSP/ISP 猜的是整个 agent 的一步，Speculative Interaction Agents 猜的是用户还没说完的话 —— 它们全都落在同一条投机边上。

**关键补充：`u` 有两个实现。** `î` 从哪来？来自 Predictor。而 Predictor 本质上就是 `u` 的一个廉价实现 `S_u`。所以一条边上并存 **权威实现 `A_u`** 与 **投机实现 `S_u`**，而 `A_u` **是否启动、何时启动是一条策略轴**（轴 1），不是模型的固有属性。这一点是整个模型能覆盖 Bypass / Elide 的关键。

```
                          ┌── launch policy: eager | lazy | absent   ← 轴 1
                          ▼
                    ┌──────────── Authority Path (权威路径) ────────────┐
                    │  A_u ─────────────── i (真值，延迟 ℓ_auth) ───────▶│
   Edge (投机边)  ──┤                                                    ├── Reconcile ──▶ Disposition
                    │  S_u ── î ──▶ SpecTask(v, î) @ Isolation ─────────▶│
                    └──────────── Speculative Path (投机路径) ───────────┘
                              │                     │
                        (breadth × depth)      staged effects
                          ← 轴 2                    │
                                              Effect Gate ──▶ 外部世界
```

六个必须命名的实体：

| 实体 | 定义 | 论文对应 |
|---|---|---|
| **Edge** | 图上一条被标注为"可投机"的依赖边，携带 admissibility 元数据 | cost-aware 的 per-edge enable bit；PASTE 的 pattern 匹配点 |
| **Proposal** `î` | 一次猜测。`{value, confidence, provenance, source}` | SPORK 的 probe；PASTE 的 prediction record；DSP 的 approximation step |
| **SpecTask** | 基于 `î` 启动的工作单元，运行在隔离域中，携带 AssumptionSet | PASTE 的 speculative job；ISP 的 target_i process |
| **Authority** | 提供真实输入、结果或外部权威判断的可选来源，由 `A_u` 表示；可以为空 | 大模型 / 主生成流 / 真实环境 / **人**；validator 是独立验证器 |
| **Reconciliation** | 权威到达时，判定 `î ≈ i` 并选择处置 | 各家的 match + commit |
| **Disposition** | 处置结果的枚举 | PASTE 的四终态 + Skim 的 warm-start + SPORK 的 partial-accept |

**命名约定**（三层不重叠）：

- **Proposal** —— orchestration 层的投机值。它的语义承诺较弱，在 L0 到 L3 都适用；投机解码社区也已使用 proposer/proposal（SPORK 的组件名为 `SporkProposer`）。
- **Draft** —— engine 层的 token 级草稿。SPORK 的 partial-token accept 会把被拒 probe 变成 draft；该词不用于 orchestration 层的值。
- **Assumption** —— 依赖图上的边（第 3 节），表示某个 Proposal 成立这一未验证前提。

---

## 2. 正交轴与取值域

本节已迁移至 [Policy Decisions](../design/policy-decisions.md)，后续在正式文档中继续整理其决策分类与取值。

## 3. 组合性：Assumption DAG

**问题**：多个投机同时在飞、且互相有依赖时，语义是什么？

论文现状 —— 三家各自发明了一套局部方案，**没有一家给出通用语义**：

- **Speculative Interaction Agents**：tool DAG + cascade-cancel。工具调用可以引用尚未解析的操作数；某个调用被作废时，下游级联取消，用 `<cancel>` 标签通知模型不要等那个 ID。
- **Speculative Actions（depth 模式）**：多步深度投机。看似会指数爆炸，实际不会 —— 分支只在投机或真实调用返回时才延伸，不一致子树立即剪枝，所以系统最多领先 `a/b` 步，活跃分支数**与视界 T 无关**。
- **ISP**：mismatch at step j → kill 所有 approximation process 和所有 `target_k (k > j)`，因为它们全建在坏前缀上。

**底座应该给的通用机制**：每个 SpecTask 携带 provenance —— 它依赖了哪些**尚未确认的假设**。

```
Assumption     = (edge_id, proposal_id)
AssumptionSet  = frozenset[Assumption]     # SpecTask 的依赖闭包
```

规则：

- `AssumptionSet == ∅` ⟺ 该 task 是权威工作。
- 某 assumption 被 **confirm** → 从所有 task 的 set 中移除；set 变空的 task 可以 commit。
- 某 assumption 被 **refute** → 依赖它的整个子图进入 Salvage → Discard。
- 投机 task 之间**可以**互相依赖（推测链），set 做并集。

这一个机制统一了上面三家的局部方案，并且顺带解决了一个**没人提但很致命**的问题：

> **可见性泄漏**。投机产物绝不能进入权威 context。PASTE 用"commit boundary"、Speculate with Memory 用"actor sees identical context"各自表述了这条。但一旦允许推测链，就需要一条**信息流约束**：非空 AssumptionSet 的数据只能流向同样非空的容器，唯一的降级通道是 `resolve()`。

这是一条可静态检查的类型规则。底座能强制它，单个算法做不到。

---

## 4. Effect 层：底座最大的价值洼地

**11 篇里有 9 篇的处理方式是"只投机只读工具"**：

| 论文 | 副作用处理 |
|---|---|
| SPORK | manifest 标记 read-only 才投机；写/非幂等工具走串行 |
| Speculative Tool Calls | 原文：只能投机 *stateless, cheap* tools；有状态的需要 undo/rollback，**"out of the scope of this paper"** |
| Speculate with Memory | 每个 benchmark 定义 read-only action 白名单，从 API schema 派生；预测的 action type 先查白名单再 pre-launch |
| Skim | 明确排除购买/表单/账户修改；state-mutating 任务直接绕过投机 |
| SPAgent | 工具是只读 Wikipedia 搜索 |
| Speculative Interaction Agents | 手工标注 safe/unsafe；unsafe 缓冲到 commit point |
| PASTE | **三层分级**（见下）+ 审计出 602 个潜在副作用动作并阻止提交 |
| Speculative Actions | Assumption 2 把可逆性写成**前置条件**；提到 fork/snapshot/roll-forward repair，**未实现** |
| Cost-aware | admissibility 三分（side-effect-free / idempotent-under-natural-key / stageable-behind-commit-barrier），不满足则打 `non_speculable` 标记，**在部署期关掉 enable bit，独立于决策规则** |

**这是全领域最一致的短板，也是底座最清晰的机会。**

如果底座能提供真正的效果管理层，就能把所有这些算法的适用范围从只读扩展到可写 —— **这才是系统研究者的贡献，而不是再写一个 predictor**。

建议的 EffectClass（综合 PASTE 三层 + cost-aware 三分 + Speculative Actions 的补偿设想）：

```
PURE                              // 无外部效果，随便投机
IDEMPOTENT(natural_key)           // 重放收敛到同一状态（upsert on deterministic id）
STAGEABLE(commit_barrier)         // 效果可缓冲，屏障后释放
TRANSFORMABLE(variant)            // 存在非变异的等价执行：dry-run / staging target  ← PASTE
COMPENSABLE(undo_fn)              // 可补偿（refund/replace）
OVERWRITABLE(ordering)            // last-write-wins 收敛                        ← Speculative Actions OS
IRREVERSIBLE                      // 不可投机，硬闸
```

三个必须做进 Mechanism 的细节：

1. **commit barrier 的触发条件必须显式建模。** Speculative Interaction Agents 给了一个非平凡的例子：unsafe 调用缓冲，直到 (i) final query update 到达 **且** (ii) 模型发出比之前更大的 tool ID（表示编辑结束）或 pause（计划稳定）。他们类比 MIPS R10000 的写缓冲：**"write operations are buffered but not allowed to execute until the instruction is committed"** —— 这个类比是对的，底座应该照抄处理器的 commit/retire 语义。

2. **PASTE 的 TRANSFORMABLE 被低估了。** "非变异的等价执行"（dry-run、打到 staging 资源）把一个不可投机的操作变成可投机的，而且**不需要补偿**。这比 COMPENSABLE 便宜得多，应该是首选路径。

3. **审计必须是 mechanism。** PASTE 报告"跨所有 workload 标记了 602 个潜在副作用投机动作并阻止其提交，没有任何任务的最终结果偏离 baseline"。这种数字只有底座能产出。

---

## 5. 资源与准入：最强的"底座必要性"论据

这是最初的描述里**完全缺失**的一维，但它是负结果最密集的地方 —— 也就是说，**它证明了单个算法在自己的抽象里解决不了这件事**。

### 负结果清单

| 来源 | 负结果 |
|---|---|
| **SPAgent** | 朴素投机（Speculative Actions baseline）在 >2 rps 时**比不投机慢 49.3%**，推理延迟升高最多 26% |
| **PASTE** | Tool-Only 消融（只加速工具、不改 LLM 调度）**比 baseline 更差**：419s vs LLM-Only 的 342s vs 完整 PASTE 的 270s。原因是投机让 session **更频繁、更集中地**返回 GPU |
| **PASTE** | 并发从 1 → 192，LLM 生成时间涨 **17×**，工具时间几乎不变。人为把工具加速 2× 而不改 LLM 调度器，收益在高负载下被完全吸收 |
| **SPAgent** | 并发上升时每步 decode 慢约 20%；prefill 近线性，hybrid batch 远贵于纯 decode batch |
| **Cost-aware §14.2** | elastic capacity 假设在限流/预留机队/高利用率自建端点下失效。投机会挤占在线请求，`(1−P)·C_spec` **定价了浪费的 token，但没有定价这个机会成本** |
| **Speculative Actions** | breadth 模式 latency reduction 理论上限 **50%** |
| **Speculative Tool Calls** | client-side speedup 严格 `< 2×` |

### 结论

**投机的收益是负载相关的，且在高负载下可以为负。** 因此底座必须提供：

1. **准入控制**：PASTE 的四道闸（executable / policy-safe / 置信度与预期收益 / 预算内）
2. **优先级与抢占**：投机 job 低优先级、可抑制、可抢占；命中转正后变 non-preemptible
3. **过期丢弃**：SPAgent 的 `t_w` —— 等太久的投机请求直接丢，因为重叠价值已经过期（**不是抢占，是过期**，这两者语义不同）
4. **跨层可见性**：底座必须看得到推理引擎的压力。PASTE 的 `EnginePressure(B) = DecodeLoad(B) + γ·KVLoad(B)` 维持在 `[P_low, P_high]` 带内；SPAgent 需要引擎暴露 hybrid batch 的 profiled timing 和 running request 计数
5. **反向调度**：SPAgent 的 Inter-Request Schedule 用 SJF 让投机请求**插队** —— 投机输出通常 <10 token，延迟是排队主导的，不插队就赶不上主路径；而因为它们极小，插队几乎不拖慢推理请求

### 规范化资源记账（这也是 mechanism）

论文对"什么算浪费"的口径**互不兼容**，导致无法横向比较：

| 论文 | 记账口径 |
|---|---|
| Speculative Actions | 用 `k̃`（去重后分支数）**无条件计费** |
| Cost-aware | **只在失败时计费** `(1−P)·C_spec`。理由：成功时这笔钱本来就要花，投机的增量成本为零 |
| SPORK | 用 α（**per-turn** 接受率，不是 per-probe）算 break-even：`α·t_overlap ≥ T_oh` |
| DSP | 区分 total cost 与 **unnecessary cost**（降 30% / 降 60%） |
| ISP | token 成本大致翻倍；最坏情况 approximation process i 跑 `(i mod k)+1` 次，target process i 跑 `i` 次 |
| Cost-aware §9.3 | **fractional waste**：流式取消时只计已生成的 token，例子里从 $0.0165 降到 $0.0060（省 64%），但**posterior 仍记为一次完整失败** |

底座应该定义规范语义，三分：

- **(a) 无论如何都要花的**：命中时投机 work 就是权威 work
- **(b) 纯浪费**：未命中且无残值
- **(c) 部分成本**：被抢占/取消时已消耗的部分（fractional）

理由：算法各自记账则**无法比较、无法做全局预算、无法做 kill-switch**。而且记账口径必须包含 token 之外的资源 —— GPU 占用、并发槽、外部 API 配额 —— 否则 cost plugin 拿到的是价目表价格而非真实边际成本。

---

## 6. 接入面：无侵入的可检验定义

把 11 篇的接入需求归纳，底座只需要在**四个接缝**上做 shim：

| # | 接缝 | 需要的能力 | 谁需要 |
|---|---|---|---|
| 1 | **LLM 调用边界** | 流式、按 token 中断、logprobs、prefix cache 共享、并行 dispatch、按 index 取消 | SPORK（需要 logprobs + prefix 共享）、DSP、ISP、Speculative Interaction Agents（vLLM interruptible streaming） |
| 2 | **工具分发边界** | 统一 dispatch 接口、优先级类、预算、抢占、promote 到 non-preemptible、沙箱、dry-run/staging 变体 | PASTE、SPORK、Speculative Tool Calls |
| 3 | **轨迹/上下文写入边界** | truncate-and-rewrite（`S[:j] + new`）、prompt 变更、commit boundary 强制 | ISP、DSP、PASTE |
| 4 | **调度/准入边界** | 引擎压力可见、准入重排、优先级覆盖 FCFS | PASTE、SPAgent |

**实测的侵入量**（支持"无侵入可行"这个命题）：

- **PASTE**：agent 侧 ~5k 行 TS（tool dispatch 中间件 + LLM client 轻量 wrapper），LLM 侧 ~2k 行 Python（**未修改的 vLLM** 上挂 startup hook）。gemini-cli / Qwen Deep Research / Virtual-Lab 三个 agent 只需要一个小的 dispatch wrapper，**工具 API 完全不改**。
- **SPORK**：thin controller over standard completion APIs，无重训练、无辅助模型、无离线 trace。
- **Skim**：对底层 agent **只有一个硬性要求** —— 能从任意 URL warm-start。不需要注入推理历史（因为只有 URL 过界）。三个观察模态完全不同的 backbone（WebVoyager 截图 / AgentOccam 剪枝 DOM / BrowserUse 浏览器原语）都能接。
- **Speculative Tool Calls**：提议一个新端点 `POST /cache-tool-output/{response_id}`，body 是 `{name, params, output, keep_alive}` 数组，按 `name` + canonicalized `params` 匹配后续 tool call。

### 关键发现：底座必须是**双层**的（范围决策：engine plane 不做）

> **范围决策（2026-08，D-013 修订）**：engine plane 整体排除在范围外 —— 本系统以 API 方式使用大模型，下面 4 篇的机制全是自托管专属。本小节与 §10.7 的分析保留为论文调研背景与未来自托管时的候选。

11 篇里有 **4 篇** 的核心机制在纯 API 层够不着：

| 论文 | 需要 engine plane 的原因 |
|---|---|
| **Speculative Tool Calls（engine-side）** | commit 发生在 **KV cache 层**：检测到 tool-start token → 按 (request_id, tool_name) 查缓存 → 把缓存的参数 token 作为 draft 注入 → 标准 speculative sampling 验证（early exit decoding）→ tool-end token 时查全键 → 命中则把 tool 输出 token 追加进 KV cache，**decode 不中断、序列不被逐出** |
| **SPORK D3** | partial-token accept 需要接进引擎的 speculative-draft proposer（`SporkProposer` 包 vLLM ngram proposer） |
| **SPAgent** | 两级调度器需要引擎暴露 hybrid batch profiled timing、running request 计数、优先级准入覆盖 FCFS |
| **PASTE** | co-scheduler 需要 decode batch 压力、队列深度、context 长度分布、KV 压力 |

**所以底座是 orchestration plane + engine plane 两层，且必须显式定义两层之间的接口。** 只做 API 层的底座会把这 4 篇（也是系统味最重的 4 篇）排除在外。

顺带一提 Speculative Tool Calls 说的 "sequence residency"：基线引擎在生成 tool call 后会把序列从 active batch **逐出**，工具返回后再作为新请求重新 prefill 整个历史。这个逐出/重入开销在多租户下相当可观。"保持驻留"就是消除它。**这是纯粹的 engine plane 收益，orchestration 层完全看不见。**

---

## 7. Shadow / Replay：底座的隐藏杀手锏

前面反复出现，这里收拢成一条论证：

**同一套 shadow 基础设施同时解决四个问题：**

1. **Fidelity 审计** —— 投机与串行同跑、只提交串行结果，是验证 lossless 的唯一手段
2. **Predictor 数据管道** —— PASTE 的 pattern mining、DSP 的 `(state, k)` 对、Speculate with Memory 的 miss episodes，三家各自造轮子
3. **Cost 校准** —— 真实的 `p`、`α`、`ρ`（取消前生成比例）只能从影子流量里测
4. **上线安全** —— cost-aware 的五阶段（offline replay → shadow → canary 1%/5%/25%/100% → online calibration → drift kill-switch）本该是底座能力

Speculate with Memory 顺带给了一个有用的方法学细节：他们把 actor 轨迹**采集一次然后 replay**，以消除 actor 方差、把 API 成本隔离到 speculator 上。这个"actor replay"应该是底座的一等能力 —— 它让 predictor 的评测变成确定性的。

---

## 8. 自证与边界

### 8.1 自证：11 篇能否表达为"policy 取值 + 不改 mechanism"

| 论文 | 轴1 权威启动 | 轴2 拓扑 (b×d) | 预测对象（非轴） | 轴3 `S_u` | 轴4 `≈` | 轴5 Authority | 轴6 主要 Disposition | Fidelity（导出） | Effect | engine plane |
|---|---|---|---|---|---|---|---|---|---|---|
| Speculative Actions | Eager | k 动态 (Thm.4) × d≥1 | next call specifier | 小模型/同模型低推理 | **L2** | 真实环境 | Reuse / Discard | Trajectory | PURE | 否 |
| ├ OS 变体 | Eager | 1×1 | 内核参数 | 小模型 | — | 真实环境 | **Overwrite** | **Best-effort** | **OVERWRITABLE (release-early)** | 否 |
| PASTE | Eager | 多候选 ×1 | concrete invocation | **离线 pattern + 晚绑定** | L0 (canonical) | 主生成流 | **Reuse / Promote / Discard / Preempt** | Trajectory | PURE + **TRANSFORMABLE** | **是** |
| Skim | **Lazy** | 1×1（aggregate 模式 ~4） | 整条轨迹终点 | **站点 profiler** | **L3 (validator)** | **validator** | **Salvage (warm-start URL)** | **Outcome (统计)** | PURE（写操作直接绕过） | 否 |
| ISP | Eager | 1×k (k=4) | plan step | 弱 agent | L0 / **L1(软匹配)** | target agent **+ 人** | Re-execute / Discard | Trajectory / **Best-effort(软匹配时)** | PURE | 否 |
| DSP | Eager | 1×k (**在线学 k**) | plan step | 弱 agent | L0 | target agent | Discard | Trajectory | PURE | 否 |
| SPAgent (Verified) | Eager | k=3 ×1 | action | `π_θ^s` 无 thought 采样 | L0 | 主生成流 | Reuse / Discard | Trajectory | PURE | **是** |
| ├ (Aggressive) | **Absent** | k=3 ×1 | action | 同上 | **⊤ (不验证)** | **无** | — | **Bounded-degradation** | PURE | 是 |
| Spec Interaction | Eager**(物理强制)** | 1× tool DAG | **未来用户输入** | 主模型(需 SFT) | — | 用户输入完成 | **cascade-cancel** / Discard | Bounded-degradation | **STAGEABLE(commit point)** | 否* |
| SPORK | Eager | 1×1（重试节奏 `r·s`） | tool call | **同模型 self-fork** | L0 (name+args 全等) | 主生成流 | Reuse / **Salvage (partial-token)** | Trajectory | PURE (manifest 硬闸) | **是 (D3)** |
| Spec Tool Calls | Eager | λ 个采样 ×1 | tool call | xLAM 小模型 | L0 (canon key) | 主生成流 | Reuse / Discard | Trajectory | PURE (stateless only) | **是 (KV splice)** |
| Speculate w/ Memory | Eager**(Type2 物理强制)** | best-of-k × Type3 链式 | **obs / action / 链式** | **在线记忆** | L0 (full exact) | 真实环境 | Reuse / Discard | Trajectory | PURE (白名单) | 否 |
| Cost-aware | Eager | 1× DAG（top-m 可选） | DAG 上游输出 | 多源 | **L0+L1+L2 三层** | 上游算子 | Re-execute / **Fractional cancel** | 未声明 | **三分 admissibility** | 否 |

\* Speculative Interaction Agents 需要 vLLM interruptible streaming，算接缝 1 的能力，不算 engine plane 插件。

**读表结论：**

1. **没有一篇需要改 Mechanism。** 所有差异都落在 policy 列。抽象通过自证。
2. **轴 1 列只有两个非 Eager**，且恰好就是可省性判据预测的那两个（Skim 的 `u` 是 agent 自己的轨迹、SPAgent 的 `u` 是模型自己的 thought）。**判据有预测力，不是事后贴标签。**
3. **轴 1 与轴 4 强相关但不是同义**：Lazy/Absent 强制 L3/⊤（因为没有 `i`），但 Eager 不强制任何具体层次（L0/L1/L2 都出现了）。这证明两条轴确实独立，只是有单向约束。
4. **Disposition 列证实了 Salvage 缺项** —— Skim 和 SPORK 的主要 Disposition 都是 Salvage，原来的七元组表达不了。且二者的轴 1 取值不同（Lazy vs Eager），说明 Salvage 不是 Lazy 的专属。
5. **`≈` 列的分散度最高**（L0/L1/L2/L3/⊤ 全都出现），证明它必须是独立一轴，而不是"匹配"这个模糊词。
6. **Fidelity 列证明 lossless 不是布尔值** —— 同一个词在 11 篇里指四种不同的保证。
7. **engine plane 列有 4 个“是”** —— 对那 4 篇而言双层设计是必需的；**范围决策（D-013 修订）**：本系统不做 engine plane，这些机制排除在范围外。表格保留作为论文事实。

### 8.2 明确划在边界外的

诚实标注比强行容纳更有价值：

1. **模型协议要求**（Speculative Interaction Agents 的 SI-SFT）。底座提供 async I/O 与 commit barrier，但"模型能否遵守流式编辑协议"是外部依赖 —— 需要模型被 SFT 过才能遵守流式协议（`<partial_query_update>` / `<pause>` / `<cancel>` / tool ID 编辑），未训练和普通 SFT 模型都失败（Qwen 无 SI-SFT 时 HotpotQA 只有 20.1），不要试图用底座解决。该论文的自然语音评测也是负结果（Qwen 22.0→13.6、延迟 9.3s→14.3s，归因"重复动作的退化行为"）—— Overlap-on-user-input 这一支还不成熟。
2. **权威启动 = Absent 的方法**（SPAgent Aggressive）。严格说它不是投机 —— 没有权威路径、没有验证对象。它是**降级近似 / 模型路由**。可以表达为 `authority_launch = absent` + `equivalence = ⊤` + `fidelity = bounded-degradation`，但底座应该在 UI/日志里把它标成**不同的契约等级**，避免和真投机混为一谈。把它放进同一条轴的价值不在于"它是投机"，而在于轴 1 × 轴 4 的格让"从无损重叠连续退化到纯路由"这件事**可见**：操作者能看到自己把旋钮拧到了哪一格。
3. **离线剖析/挖掘管道**（Skim 的 site profiler、PASTE 的 pattern mining）。底座提供 trace 与 replay，但"怎么从 trace 里挖 pattern"是 predictor 插件的事。
4. **跨请求的机会成本定价**。cost-aware 自己承认 `(1−P)·C_spec` 定价不了限流/预留机队下的挤占。底座能提供**真实的边际资源记账**（第 5 节末），但把它转成美元是 policy。

---

## 9. 嵌套投机执行 (Nested Speculation)

### 9.1 双视角：边 vs 实例

- **边视角**（第 1 节）：假设**在哪里被引入** —— 这是语义。
- **实例视角**：假设**由谁携带** —— 这是运行时。

实例视角下，投机 4 次 + 权威 = **5 个并发 agent 实例**。而对每个投机实例来说，它正在执行的步骤"就是正经执行"，因此它自己也可以继续投机 —— 这就是嵌套。

两个视角由 AssumptionSet 连接：

```
边   引入一个 Assumption
实例 携带一个 AssumptionSet（其路径上所有边引入的假设之并）
     assumptions == ∅  ⟺  权威实例
```

论文佐证实例视角与实现一致：ISP 字面上 spawn `approximation_i` / `target_i` 进程；DSP 并发跑 A 与 T。

> **关键结论：AssumptionSet 是为组合性设计的（第 3 节），而嵌套就是组合性本身。嵌套不撑破模型 —— 它是那个机制存在的理由。**

### 9.2 现状：有链无树

纠正一个常见误解 —— 嵌套并非完全空白：

| 论文 | 嵌套形态 |
|---|---|
| ISP | A 的第 `i+1` 步建在自己**未验证**的第 `i` 步上；`target_{i+1}` 建在 A 的未验证前缀上。**单链**，`k=4`，最大并发 `k+1` |
| DSP | 同上，`k` 在线学习 |
| Speculative Actions（depth-focused） | 多步连续 spawn，最多领先 `a/b` 步，活跃分支数与 `T` 无关 |
| **无人实现** | **深层还分叉**。Speculative Actions 自己列为未实现扩展："predict s steps ahead into a tree, attach confidences, and expand promising branches beam-search style" |

**所以缺口是 breadth × depth 的乘积区域，不是 depth 本身。** 这个缺口比"完全没人做嵌套"窄，也因此更可信 —— 单链嵌套已被证明可行且有界，剩下的是把它推广到树。

### 9.3 成本：速率界，不是组合界

朴素直觉是 `k^d` 爆炸。**错。**

Speculative Actions 的界：分支只在投机或真实调用返回时才延伸、不一致子树立即剪枝 ⟹ **系统最多领先 `a/b` 步，活跃分支数与视界 `T` 无关**。这是**速率界** —— 真正的约束是"权威路径还剩多少 wall-clock ÷ 投机器延迟"。

**设计推论 1：depth 不该是策略参数，而是 frontier budget 下涌现的属性。**

DSP 花力气在线学 `k`，本质是在学"往前走多远划算"。但给定一个 frontier budget，需要学的东西变成"下一个该扩展哪个节点" —— 那是优先队列问题，而 PASTE 已经在做（按 expected utility per unit resource 贪心排序）。

> **frontier budget + 优先级排序，吸收了 depth-as-parameter。**

具体机制：**best-first frontier expansion** —— 优先队列的 key = `P(节点存活) × 期望延迟节省 / 资源成本`，pop 到预算耗尽。它同时吸收了 breadth-vs-depth 的分配决策；Speculative Actions Thm.4 的贪心（加分支直到 `Δ*·δq(m;p) < c`）是它在 `depth=1` 上的特例。

**设计推论 2：浅层分叉、深层单链。**

每层都分叉 → `k^d`。只在浅层分叉、之后沿最高置信度**单链**下探 → `k·d`。存活率相应从 `p^d` 变成 `p(k)·p^(d−1)`。

而 `p(k) = 1−(1−p)^k` 对 `k` 饱和很快、成本对 `k` 线性 ⟹ **花 breadth 买 depth 是划算的**。

PASTE 是活证据：

| | top-1 命中 | 多候选整体命中 | depth 3 存活率 |
|---|---|---|---|
| PASTE 实测 | 27.8% | **93.8%** | `0.938³ ≈ 0.83` |
| 若只用 top-1 | 27.8% | — | `0.278³ ≈ 0.02` |

> **breadth 是 depth 的使能器。** 这直接回应"成本急剧攀升"的担忧：答案不是限制深度，而是在浅层花 breadth 把每层存活率推高，让深度变得可行。

### 9.4 适用性判据：看单步命中率

| 论文 | 单步命中率 | 嵌套前景 |
|---|---|---|
| **SPORK** | 工具名 **74.6–99.6%** | **最该做深**：`0.9³ ≈ 0.73` |
| PASTE（多候选） | 93.8% | 好 |
| DSP 预测器（成熟后） | 0.70–0.76 | 中 |
| Speculative Actions | chess 54.7%、e-commerce 22–38%、HotpotQA ≤46% | 不配 breadth 的话 depth 2 就到头 |
| Speculate with Memory | 17–40%（但随经验单调增长） | 需 breadth 支撑；记忆累积后转好 |

> **可检验预测**：嵌套的收益应随单步命中率**超线性**增长。这是底座能做、单个算法做不了的实验 —— 因为需要在同一套调度层上横扫多个 predictor。

### 9.5 透明嵌套 + 反射 gating

"对投机实例而言，它执行的步骤就是正经执行"这一观察指向正确的设计：

**透明嵌套** —— 投机实例与权威实例二进制同构，**它不知道自己是投机的**。这正是 AssumptionSet 的意义：实例不必知道，底座追踪 provenance 并把闸门设在效果层。**这是无侵入的前提。**

**但有一个真实陷阱**：算法自己的成本/置信策略**假设了自己在权威路径上**。DSP 学的 `k`、cost-aware 的 EV 门，算的都是"在真实状态下投机划不划算"。一个 depth-2、存活率 0.25 的实例若不知情，会**过度投机 4×**。

正确的切法：

| 层 | 是否知道自己是投机的 |
|---|---|
| Agent 业务逻辑 | **否**（透明） |
| Predictor `S_u` | 否 |
| **Cost / admission 策略** | **是**（反射）—— 底座必须透传 `P(实例存活)` |
| Effect gate | 是（由 AssumptionSet 直接驱动） |

接口改动仅一处：`should_speculate(..., survival: float)`。

> **这是对 gating 的改动，不是对算法行为逻辑的改动。** 正面回应"嵌套会改变投机算法本身行为逻辑"的担忧：不会 —— 只要把存活概率透传给成本层。

### 9.6 错误率：L0/L2 不升，L1/L3 相乘

每层都验证 ⟹ `p^d` 是**命中率**，不是**错误率**。trajectory-equivalence 下错误率**恒为 0**，嵌套只增加**浪费**，不增加错误。

但软匹配和 validator 会**复合**：Skim 的 verifier precision 82%，嵌两层 → 0.67，三层 → 0.55。

> **硬约束（应编码进 loader）**：`depth > 1 ⟹ equivalence.level ∈ {L0, L2}`

### 9.7 实现复杂度：真正新增的只有两项

| 能力 | 是否新增 | precedent |
|---|---|---|
| 上下文 COW fork | **新增** | SPORK 的 prefix-cache fork 证明 KV 层近乎免费：探针预填 1.6s → 0.35s，prefill 干扰 <1% |
| frontier 预算调度器 | **新增** | PASTE 的 utility-per-resource 排序 + SPAgent 的两级调度 |
| 级联取消 | 本就需要 | Spec Interaction 的 cascade-cancel；ISP 的 kill `target_k, k>j` |
| 提升级联 | 本就需要 | PASTE 的 promote；是 `assumptions == ∅ ⟺ 权威` 的自然推论 |

嵌套下"提升"变成**级联**：depth-1 的 proposal 被确认时，建立其上的 depth-2 实例**不会**变成权威，而是**降级为 depth-1 投机**。模型已经处理了这一点 —— 因为"权威"从来就不是静态标签，而是 `assumptions == ∅` 的函数。

### 9.8 与其他轴的交互汇总

| 轴 | 交互 |
|---|---|
| 轴 1 权威启动 | `absent` 下嵌套无意义 —— 没有验证，深度只是更长的降级链 |
| 轴 2 拓扑 | 嵌套就是这条轴的 depth 维度被认真对待；且 depth 应由预算涌现而非参数指定 |
| 轴 4 等价 | `depth > 1 ⟹ L0/L2`（见 9.6） |
| 轴 6 处置 | Salvage 在嵌套下**更重要** —— 被剪掉的子树可能含大量部分进展 |
| Effect | 嵌套下 AssumptionSet 非单元素，effect gate 必须等**全部**假设确认，而非单个 |

---

## 10. 系统架构：组件清单与边界

### 10.1 四侧中的组件与部署视图

一级架构统一为 **Agent 接入侧 -> 控制面 -> Runtime Adapter -> Runtime**，权威定义见 [`../architecture.md`](../architecture.md) 与 [`../design/decisions.md`](../design/decisions.md) D-039。本节只把研究推导出的内部组件和 `控制面 -> N Instance` 数量关系放回四侧坐标中；它不直接规定 crate、进程或服务拓扑。

```
┌──────────────────────────── Agent 接入侧 ────────────────────────────┐
│ Agent / product + CLI / library / launcher                           │
│ ASC-Daemon + Interposer + Operation Integration（部署在各 Instance） │
└───────────────────────────────┬──────────────────────────────────────┘
                                │① / ⑤ / ⑪
                                ▼
┌───────────────────────────── 控制面 ───────────────────────────────┐
│ Entry API ─────▶ AScoutAgent / Speculative Tree                      │
│                     │                                                 │
│ Policy Engine ◀──④──┼──▶ Assumption Manager ──⑥──▶ Scheduler        │
│      │②             │             │⑦                  │⑧             │
│ Policy plugins      │             ▼                   ▼              │
│                     └──────▶ Effect Coordinator ◀──────┘              │
│                                  │⑫                                   │
│                                  ▼                                    │
│                                Ledger                                 │
└──────────────────────────────────┬────────────────────────────────────┘
                                   │③ / ⑨
                                   ▼
┌──────────────────────── Runtime Adapter ─────────────────────────────┐
│ 稳定 operation/event contract + capability / transport mapping       │
└──────────────────────────────────┬────────────────────────────────────┘
                                   │⑩
                                   ▼
┌────────────────────────── Runtime ─────────────────────────┐
│ Runtime + Execution / Workspace / Input / Effect / Outcome│
│                         │ 管理                                       │
│        ┌────────────────┼──────────────────────────┐                  │
│        ▼                ▼                          ▼                  │
│   Instance 0       Instance 1       ...       Instance N             │
│   Authority        Spec d=1                   Spec d=2                │
│   A = ∅            A = {a₁}                  A = {a₁,a₃}             │
│   own-env          own-env                   shared(I-1)              │
└───────────────────────────────────────────────────────────────────────┘
```

Instance 是 Runtime 管理的运行对象，不是第五侧。Agent、ASC-Daemon 与 Interposer 可以部署在 Instance 的隔离环境内，但逻辑职责仍属于 Agent 接入侧；Isolation Env、执行状态与资源归属属于 Runtime。

**实例的可能状态**（对应 Ray 声明 actor | task | worker 的那一层）：

| 维度 | 取值 | 说明 |
|---|---|---|
| **role** | `Authority` \| `Speculative(d)` | **不是静态标签，是 `A == ∅` 的函数。** promote 就是这个谓词翻转 —— 这也是实例结构必须同构的原因（§10.5） |
| **grain** | `own-env` \| `shared-env` | shared-env 仅对 PURE 效果开放，其余强制 own-env。**隔离粒度由 EffectClass 决定** |
| **lifecycle** | `Forked → Running → Resolving → {Reused, Promoted, Discarded, Preempted, Salvaged}` | 即 Disposition 轴（§2 轴 6） |

### 10.2 组件间数据流

| # | 流向 | 内容 |
|---|---|---|
| ① | Entry API → Policy Engine / AScoutAgent | 任务提交、查询与显式 policy 选择输入 |
| ② | Policy Engine → Policy plugins | 实例化插件、绑定轴配置 |
| ③ | Policy Engine / Effect Coordinator ↔ Runtime Adapter | **跨侧契约协商**：控制面报预测置信度、验证条件与失败代价；Runtime Adapter 返回具体 Runtime 的隔离能力、成本与可用纪律。产出 fidelity 上界；不可行组合拒绝加载 |
| ④ | 控制面 mechanism components → Policy Engine | Assumption Manager 的 resolve 调 Equivalence / Salvage；Scheduler 调 Topology / CostGate；Effect Coordinator 调 Compensation。mechanism 调用 policy，不感知其实现 |
| ⑤ | Interposer / ASC-Daemon → Entry API → Assumption Manager | 权威值到达，触发 `resolve()`：调 Equivalence 判 `î ≈ i`，选 Disposition，沿依赖闭包传播；该路径阻塞权威 continuation |
| ⑥ | Assumption Manager → Scheduler | 可扩展结构、各节点存活概率（嵌套下即 §9.5 要透传给 CostGate 的 `survival`） |
| ⑦ | Assumption Manager → Effect Coordinator | confirm/refute 后形成目标 effect transition；Coordinator 再经 Runtime Adapter 请求 Runtime 执行 |
| ⑧ | Scheduler ↔ Effect Coordinator | 抢占前检查控制面效果计划；Runtime 仍须对真实 obligation 作最终校验 |
| ⑨ | Scheduler / Effect Coordinator → Runtime Adapter | 发送 fork、kill、优先级与 effect transition 等稳定语义 operation |
| ⑩ | Runtime Adapter ↔ Runtime | Adapter 映射机制 operation；Runtime 返回结果、事件、能力与资源压力 |
| ⑪ | Agent 接入侧 ↔ 控制面 | 上行：拦截上报、心跳与 Agent intent；下行：Proposal、confirm/refute、受控结果与 lifecycle event |
| ⑫ | 控制面内部组件 → Ledger | **异步旁路**事件流：成本归属、轨迹、Disposition 与契约比对结果；不在关键路径上 |

### 10.3 使用模型

**四类角色，只有一类写配置，端用户不在其中。**

| 角色 | 感知投机吗 | 要做什么 |
|---|---|---|
| **端用户** | **完全不感知** | 什么都不做。提任务、拿结果，体验与不投机时一致，只是更快 |
| **Agent 开发者** | 不感知 | **代码不改一行**。唯一可能的动作是提供工具的 effect manifest |
| **Operator / 部署方** | 感知 | 声明 policy profile、选隔离后端、设预算与延迟-成本旋钮 |
| **Policy 作者（研究者）** | 就是他在写 | 实现 1–6 个插件接口；多数只需实现 Proposer |

**Policy Engine 接收的是 operator 在部署期写的 policy 声明**。实质后果：**policy 与任务的绑定发生在任务准入时，由路由规则或任务特征决定，不由端用户输入决定** —— §11 那条“不同任务选不同 policy”的指标应据此实现（底座自己选）。

**一个必须留的口子**：**ISP 是 11 篇里唯一故意让端用户感知投机的**。其全部贡献就是 UI 协同设计 —— 用户看到 approximation agent 的步骤、看到它与 target agent 步骤之间的时间差（该间隔本身在展示哪一步是瓶颈），并可在间隔内中断、注入自己的值作为 oracle（论文称其"effectively a third agent"）。所以**"端用户不感知"是默认值而非不变量**，底座须支持把人作为 Authority（轴 5）的 opt-in 模式。

```
Agent 开发者：  把 agent 的 LLM base_url 与工具分发指向底座 → 完事
               （可选）提供 effect manifest

Operator：      声明 profile：proposer / 等价层次 / 预算 /
               声明的 fidelity 契约 / 隔离后端 → 底座编译并校验

端用户：        "修复 issue #42"  →  结果
               （不知道背后开了 5 个实例，也不需要知道）
```

**effect manifest 是接入时唯一非零的人工成本，且不可避免** —— 无法从函数签名推断一个 API 是否幂等。处理副作用的每一篇都要求其等价物（SPORK 的 manifest、Spec Interaction 的手工 safe/unsafe 标注、cost-aware 的 per-edge 标签）。可做的是把它从"全手工"降到"自动引导 + 人工确认残余"：从 MCP 的 `readOnlyHint` / `destructiveHint` / `idempotentHint`、或 OpenAPI 的 HTTP 动词语义引导生成初稿。

### 10.4 组件职责（贯穿示例）

**贯穿示例**：一个编码 agent 收到"修复 issue #42"，轨迹为

```
grep("parse_config") → read_file("config.py") → edit_file(...) → run_tests() → git_push()
       PURE                  PURE                 STAGEABLE       STAGEABLE    IRREVERSIBLE
```

下文所有例子都基于这条轨迹。

---

#### Entry API

**职责**：控制面的薄入口。接收 Agent 接入侧形成的任务提交、状态查询和显式 policy 选择输入，返回 task id 或查询结果，并把请求交给 控制面；它不直接调用 Runtime Adapter，也不独立拥有准入或 policy 规则。

**持有状态**：不持有长期控制状态。在飞任务、policy 绑定与预算等状态由控制面内相应 owner 管理。

**例**：服务收到"修复 issue #42"。Entry API 把任务与用户显式指定的 policy（未指定则为空）交给 控制面；控制面应用默认或指定 policy、完成准入，再经 Runtime Adapter 请求初始化 root Authority Instance，最后由 Entry API 返回 `task_id`。端用户只看到自己提交了任务。

#### Policy Engine（六类 policy 插件的宿主 + 编译校验）

**职责**：注册、实例化六类 policy 插件（Proposer / Equivalence / Topology / CostGate / Salvage / Compensation）；部署期把 operator 的 policy 声明编译成轴配置，做**轴间约束求解**与**跨层契约协商**，产出「已验证的 policy profile」；运行期被 mechanism 反向调用，分发到对应插件。

**持有状态**：已注册的插件实例；profile 表 —— 每个 profile 的轴配置、推导出的 fidelity 上界、所依赖的下层能力。

**例（编译通过）**：operator 写

```
{ proposer: paste-pattern, equivalence: L0-canonical,
  topology: {breadth: 3, depth: auto}, fidelity: trajectory-equivalent }
```

Policy Engine 逐条检查：`depth>1` 要求 L0/L2 ✓；`trajectory-equivalent` 要求 `authority_launch=eager` ✓ 且 `effect ∈ {PURE, STAGEABLE, TRANSFORMABLE}` ✓。再通过 Runtime Adapter 查询“当前具体 Runtime 能否提供 filesystem STAGEABLE” → Adapter 映射的能力报告显示 overlayfs 可以 ✓；问 API 能力表“有 logprobs 吗” → **API 后端没有** → 把依赖 logprobs 的置信门降级为常数阈值，**并把降级原因写进 profile**。编译通过。

**例（拒绝加载）**：operator 写 `{equivalence: L1-semantic, fidelity: trajectory-equivalent}` → 拒绝，报“L1 语义匹配撑不起轨迹等价，最高只能声明 outcome-equivalent”。这正是 ISP 论文自己承认过的事（用了模糊匹配后就不再保证与普通规划结果相同），底座把它变成加载期的强制检查。

**六类插件**：

| 插件 | 职责 | 贯穿示例 |
|---|---|---|
| **Proposer** | 给出 `î` | pattern 池匹配到事件签名 `[grep → ?]` 的历史规律，argument mapper 从 grep 输出首条匹配里抽出路径，产出 Proposal `read_file("config.py")`，置信度 0.9 |
| **Equivalence** | 判 `î ≈ i`，被 `resolve()` 调用 | 权威流实际发出 `read_file("./config.py")`。L0 规范化后两者相等 → 命中。若用 name-only 匹配则会把 `read_file(任意路径)` 都算命中 —— SPORK 明确拒绝这么做，因为对副作用工具会静默改变行为 |
| **Topology** | 决定宽度与深度 | 该 site 有 3 个候选；预算允许，选置信度最高的 2 个，并允许在最高置信那支上再下探一层 |
| **CostGate** | 要不要投机 | `存活率 0.9 × 预期省 2.3s × λ − (1−0.9) × 成本 ≥ 阈值` → SPECULATE。**嵌套下"存活率"必须由底座透传**，否则 depth-2 实例会按 depth-0 的账算，存活率 0.25 时过度投机 4× |
| **Salvage** | 未命中时回收残值 | 权威实际读的是 `settings.py`，投机白读了 `config.py` —— 但那次读的结果留在缓存里，若后续真的用到直接命中。Skim 式的版本是把已到达的 URL 交给权威路径继续 |
| **Compensation** | 补偿已发生的效果 | 投机分支在 staging 目录里跑过测试、留下 artifact；refute 后删除该目录 |

**关键**：mechanism 调用 policy，不感知其实现（图中箭头④自下而上）。换一个 Proposer 不需要动 Reconciler 一行。

#### Assumption Manager

> **命名**：Assumption Manager。曾名 `Speculation Graph`，但本组件**调 Equivalence 插件、强制一条信息流规则、坐在关键路径上执行决策过程**，`Graph` 一词未覆盖"主动"语义。选定理由：**(1) 形状同构的强先例** —— 数据库的 Transaction Manager / Lock Manager（拥有状态、强制不变量、在关键路径、被他人调用），且 `resolve()` 本质是一次 commit 决策；**(2) 让架构图去教抽象** —— 全文概念中心是 assumption（`AssumptionSet`、`A == ∅ ⟺ Authority`、依赖闭包传播），盒子叫这名字，读者看图即把抽象与组件对上；**(3) 对"两个索引"诚实** —— 主名是 assumption，instance 树是其另一索引视图，正是 §9.1 对偶的说法。

**职责**：全系统唯一的"谁依赖谁"真相源。假设 DAG 与实例树是**同一份状态的两个索引**（`by_assumption` / `by_instance`）。

**持有状态**：assumption 节点 `(site, proposal, 状态 ∈ {pending, confirmed, refuted})`；instance 节点 `(id, AssumptionSet, role, grain)`；依赖边。

**例（命中，展示 promote 级联）**：

```
Instance 0  Authority     A = ∅
Instance 1  fork 自 0，基于 a₁ = (grep后的下一次调用, read_file("config.py"))
Instance 2  fork 自 1，基于 a₃ = (read_file后的下一次调用, edit_file(...))
            → Instance 2 的 AssumptionSet = {a₁, a₃}
```

权威 Instance 0 真的发出 `read_file("config.py")` → `a₁` 被 confirm →

- Instance 1 的 AssumptionSet 变空 → **它就地成为 Authority**（role 只是 `A == ∅` 的函数）
- Instance 0 那次调用**直接复用** Instance 1 已拿到的结果
- Instance 2 的 AssumptionSet 收缩为 `{a₃}`，从 depth-2 降为 depth-1

**没有任何实例被重建。** 这就是"实例结构必须同构"的收益 —— promote 只是谓词翻转。

**例（未命中）**：权威发出的是 `read_file("settings.py")` → `a₁` refute → Instance 1 与 Instance 2 **整个子图**进入 Salvage → Discard。这一条统一了 Spec Interaction 的 cascade-cancel、Speculative Actions 的子树剪枝、ISP 的 kill `target_k (k>j)` —— 三家各自发明过的局部方案。

**附带作用**：它是可见性泄漏的静态防线 —— `AssumptionSet` 非空的数据只能流向同样非空的容器，唯一降级通道是 `resolve()`。这是一条可静态检查的信息流规则。

**它是自身唯一的变更者，且 resolve 就是它的写路径。** 对外只暴露操作，不暴露内部结构：

```
resolve(site, 权威值)          ← 关键路径；调 Equivalence 判定，选 Disposition
add_node(...) / preempt(i)     ← 旁路；由 Scheduler 调用

内部强制的不变量：
    A == ∅  ⟺  role == Authority
    refute 沿依赖闭包级联
    AssumptionSet 非空的数据不得流向空容器
```

不变量集中在一处强制执行，调用者无法违反 —— 即 etcd / Ray GCS 的模式：状态与不变量归一个组件，决策逻辑归别人。

**`resolve()` 的两种命中必须分开**（前面 promote 级联的例子里，`a₁` confirm 后发生的是哪一种取决于投机任务的进度）：

- **Reuse**：投机的 `read_file` 早已跑完 → 结果直接转正，权威路径这次调用耗时归零
- **Promote**：投机的 `run_tests()` 还在飞（跑了 8s，还剩 4s）→ 该任务**转为权威身份并置为不可抢占**，权威路径只等剩余 4s，而非完整 12s

PASTE 把 Promote 列为独立终态是对的 —— 少了它，在飞的投机成果会被丢掉重跑。

> **resolve 不独立成组件**。按"一个盒子 = 一份独立状态 + 一份独立接口"的规则，独立 Reconciler 给出的"状态"是"待决 site 表（site → 已开出的 proposal 列表）"—— **那就是图本身**，pending 状态的 assumption 节点恰好是 `(site, proposal)` 对。它是**对图的无状态逻辑**，未通过状态检验。
>
> **体系结构对应**：**commit/retire 与 ROB 在一起**，不独立于它 —— ROB 既持有投机状态、又执行 retire 与 squash。正确对应是：
>
> ```
> Assumption Manager + resolve  ≈  ROB（持有投机状态 + retire / squash）
> Scheduler                   ≈  issue / dispatch
> ```
>
> **但 Scheduler 仍不该并入**，理由与上面无关：
>
> - **它有自己的状态** —— 优先队列、预算热计数器、Runtime 压力读数，三样都不在图里
> - **延迟类别不同** —— `resolve` 在关键路径上（权威路径阻塞等待比对结果）；"要不要再开一个投机"晚 50ms 只是少赚一点。共享一个事件循环会产生**队头阻塞**：一次调度决策（遍历候选、调 CostGate 算 EV、查压力）可能排在一次 resolve 之前，把延迟压到权威路径上 —— 而那正是本系统要削减的量。旁证：Speculative Tool Calls 实测**短工具场景下，逐出与重入引擎的簿记开销会盖过延迟隐藏的收益**，关键路径上的簿记吃掉赢面是这类系统的真实失败模式。此理由与 §10.4 中 Interposer / ASC-Daemon 的拆分同构
>
> **Effect Coordinator 也不该并入**：它持有的 Effect Catalog、分类与协调计划不属于 Assumption DAG；真实 staged effect、obligation、overlay 归属和 materialization 状态则由 Runtime 持有。更硬的理由是**失败模式不同 —— 图错了损失性能，Effect 错了真的发出一封邮件**。控制面的协调路径与 Runtime 的强制路径都必须能被单独审计。

#### Scheduler

**职责**：决定投机什么、多宽、多深、何时抢占。**全系统唯一能看到全局资源压力的地方。**

**持有状态**：可扩展节点的优先队列、**预算余额（热计数器）**、各 Runtime 的压力读数。

> 预算计数器归这里而不归 Ledger：它是花钱的那个，且 CostGate 每次决策都要同步读。Ledger 负责的是**成本归属分类**（resolve 之后才能判定），走异步旁路。

**例**：队列里有 5 个可扩展节点，key = `存活概率 × 预期省时 / 资源成本`。此刻 Runtime 报告 CPU 压力高 → 只弹出 2 个；同时发现 Instance 4 是 utility 最低的投机实例。Scheduler 先根据 Effect Coordinator 的协调视图判断能否请求抢占（流⑧），再经 Runtime Adapter 发出请求；Runtime 对真实 obligation 作最终校验后才能执行抢占。

**为什么它必须存在**（这是整个底座最强的必要性论据）：SPAgent 实测朴素投机在 >2 rps 时**比不投机慢 49.3%**；PASTE 的 Tool-Only 消融（只加速工具、不改 LLM 调度）**比 baseline 更差**（419s vs LLM-Only 342s vs 完整 270s），因为投机让 session 更频繁、更集中地涌回 GPU。**没有这一层，投机在高负载下是负收益 —— 而单个算法的作者做不了这件事，因为他们没有调度层。**

#### Effect Coordinator

**职责**：控制面中所有对外效果的统一协调路径。它根据 EffectClass × StateDomain 矩阵和 Edge & Effect Catalog 形成允许、拒绝或 transition 计划，再经 Runtime Adapter 请求 Runtime 执行；它本身不是效果执行出口。

**持有状态**：Catalog（工具 → EffectClass；边 → enable bit）和每个 SpecTask 的效果协调视图。真实 staged effect、obligation、overlay 归属、commit barrier 与 materialization 状态由 Runtime 的 Effect backend 持有。

**例（STAGEABLE）**：Instance 1 投机执行 `run_tests()`。Catalog 查得它是 `STAGEABLE / filesystem`，Effect Coordinator 经 Runtime Adapter 请求 staging；Runtime 的 Effect backend 分配 overlayfs 上层并登记 obligation，测试 artifact 全落在上层。`a₁` confirm 后 Coordinator 请求放行（流⑦），Runtime 校验后 materialize；refute 时由 Runtime 丢弃上层并完成 cleanup。

**例（硬闸）**：Instance 2 想投机 `git_push()`。Catalog 标为 IRREVERSIBLE，控制面拒绝该投机请求；Runtime 同时强制对应 obligation 和 commit barrier，不能因调用者绕过协调计划而执行。哪怕 CostGate 算出收益巨大也不行。cost-aware 给浪费的 token 定了价，**给不了一次错误的不可逆副作用定价**。

**例（TRANSFORMABLE，比补偿更优）**：若某工具存在 dry-run 或 staging 变体，Coordinator 请求 Runtime 走该变体，拿到**真实**但不产生原始变异的结果，且不需要事后补偿。PASTE 用的就是这条，应作为首选路径。

**关于虚拟执行**：直接伪造一个成功返回值是危险的 —— 伪造值会成为该分支后续行为的条件，即使 L0 全等匹配，该分支也已建立在编造的 observation 上，而**破坏点在验证机制看不见的地方**。救法是把伪造值本身建模成一个 Proposal（生产者是 Effect Coordinator 而非 Proposer），让它进入 AssumptionSet 走同一套 confirm/refute —— 风险从隐藏变成**可见、可计价、可被 Fidelity Monitor 拦截**。

#### Ledger（记录 · 回放 · 校验）

> **一个盒子**：Ledger / Fidelity Monitor / Trace & Shadow Store 合并为 Ledger —— 三者本质都是**记录与校验数据**、都在异步旁路上。
>
> **预算计数器归 Scheduler，成本归属分类归 Ledger**：CostGate 每次决策都要同步读预算余量，热计数器挂在 GB 级 trace 存储后面是错的。分类天然发生在 resolve 之后（只有那时才知道算“本来就要花”还是“纯浪费”），由 Reconciler 发出分类事件、Ledger 存档。

**职责**：三件事 —— 记录（append-only 事件流）、回放（确定性重跑）、校验（契约对撞）。

**持有状态**：事件日志；每 profile 的保真度三元组（声明值 / 从轴推导的上界 / shadow 实测值）；成本归属的三分累计。

**画布小但承重。** 类比 k8s 的 etcd 或 Ray 的 GCS —— 一个小盒子，但离了它系统不工作。**不要在论文里把它写成附属设施**，它承三处重量：

**(1) 成本归属的三分记账**

| 事件 | 计入 |
|---|---|
| Instance 1 投机 `read_file` 命中 | **本来就要花** —— 权威也要读，投机没有增加净成本 |
| Instance 2 投机 `edit_file` 未命中 | **纯浪费** |
| Instance 3 生成到一半被抢占（1000 token 里生成了 300） | **部分成本** —— 只计 300 token，但成功率 posterior 仍记一次完整失败 |

为什么口径必须由底座统一：论文之间本来就不兼容（Speculative Actions 无条件按 `k̃` 计费；cost-aware 只在失败时计 `(1−P)·C_spec`；SPORK 用 per-turn 接受率 α 算 break-even）。各自记账则**无法横向比较、无法做全局预算、无法做熔断**。

**(2) 契约实测 —— 让 lossless 从"声明"变成"测得"**

**例**：profile 声明 `trajectory-equivalent`。抽 1% 任务开 shadow：同一任务同时跑投机路径与纯串行路径，**只提交串行结果**，逐步比对两条轨迹。跑满 500 个任务后发现 3 个不一致，定位到某工具参数序列化不稳定导致 L0 规范化失败 → 告警并关掉该 site 的 enable bit。

三条承重理由：

- **它是 §11 "正确性"指标的唯一实现方式。** 要比"原始实现 vs 底座上的最终可见行为"，只能影子对拍。没有它这条指标不可测，只能像 11 篇一样*论证*。
- **组合一旦成为主打赢面，契约校验就从可选变成必需。** 把 PASTE 的 proposer 与 DSP 的 depth 学习器组合起来 —— **没有任何论文验证过该组合是否 lossless**。静态约束求解只给保真度**上界**，给不出"实际达成"的证据。而组合恰恰是最容易出问题的地方：两个各自无损的 policy 组合起来可能破坏契约。
- **没有一篇论文做过这件事。** 这既是能力也是可发表的结果 —— 底座有可能测出某些已发表的 lossless 声明在特定负载下并不成立。

**(3) 回放 —— predictor 数据供给与确定性评测**

| 用途 | 例 |
|---|---|
| **Predictor 数据供给** | PASTE 要历史 trace 挖 pattern、DSP 要 `(state, k)` 对做在线 RL、Speculate with Memory 要完成轨迹 + miss episodes。**三家各造过一次管道** —— 底座录了就都白拿，"接入新 predictor 的成本"从"自建数据管道"降到零，这是可量化的通用性证据 |
| 确定性评测 | 换一个新 Proposer，先在录好的 1000 条轨迹上**离线回放**，直接算出命中率与预期省时，**一行生产流量都不用碰**。Speculate with Memory 正是这么做的 —— actor 轨迹采一次后重放，消除 actor 方差、把 API 成本隔离到 speculator 上 |
| 成本校准 | 真实的命中率 `p`、接受率 `α`、取消前生成比例 `ρ` 只能从影子流量测出来 |
| 灰度 | offline replay → shadow → canary → online → 漂移熔断 |

#### Runtime Adapter

**Runtime Adapter 职责**：向控制面提供稳定的 fork、销毁、隔离、effect transition、能力查询和压力事件等 typed semantic contract；把这些 operation 映射到当前 Runtime 的原语、transport 和事件，并规范化能力与错误。它不拥有控制面 policy 或 Runtime mutable state。

**适配状态**：可以维护连接和具体 Runtime 的能力映射（支持哪种 fork 保真度、哪些 StateDomain 可 stage），但这些不是第二份 Runtime 权威状态。

**例**：Scheduler 经 Runtime Adapter 请求“从 Instance 1 fork 一个”。Adapter 查询当前 Runtime：自研 Runtime 走进程级 COW fork，亚毫秒级；容器 Runtime 没有 fork 原语，只能起新容器并重放轨迹。这个能力差异通过稳定 contract 上报给 Policy Engine，因此高成本 Runtime 下的 depth 会被自动压低，同一个 policy 不需要感知具体 backend。

---

#### Instance 部署承载（结构同构，数量不定）

**Interposer** —— 数据面，在线拦截四接缝：LLM 调用、工具分发、轨迹写入、调度。

> **例**：agent 调 `client.chat.completions.create(...)`。Interposer 拦下，向控制面上报"Instance 1 要发 LLM 调用，前缀哈希 X"。若控制面回"已有投机结果且 Equivalence 判定命中"，Interposer **直接把结果返回给 agent** —— agent 完全不知道这次调用没有真的发生。若 agent 调 `run_tests()`，Effect Coordinator 形成 staging 请求，Runtime 的 Effect backend 分配并管理 overlay 环境，Interposer 再把调用路由到该受管环境。
>
> **它必须与 ASC-Daemon 分开**：它在请求路径上，延迟敏感；ASC-Daemon 是本地控制入口和消息旁路。二者性能要求与失败语义都不同。

**Agent** —— 用户原样的 agent，**不改一行**。

**ASC-Daemon** —— Agent 接入侧的本地守护进程：承载身份与 AssumptionSet 绑定、生命周期消息和向 Proposer 提供的**上下文句柄**。

> **例**：心跳上报"Instance 1 存活、当前在第 3 步"；接收控制面下发的 `confirm(a₁)`（"你现在是 Authority 了"）；Proposer 要预测 Instance 1 的下一步时，通过 Daemon 拿到轨迹摘要的句柄，**而不是把整个上下文搬到控制面**。
>
> **Proposer 本身不在实例内**：否则每个实例复制一份预测器；且 SPORK 那类自投机探针实际跑在推理侧，根本不在实例里。

**Isolation Env / Runtime** —— Runtime 管理的效果隔离与执行承载。`shared-env` 仅对 PURE 效果开放，其余强制 `own-env`。

---

**两条归属说明：**

- **Edge & Effect Catalog 折进 Effect Coordinator**（有状态但无独立对外接口，不单独画盒子）。在控制面内部，效果分类与协调规则属于 policy 不能放宽的固定规则；真正的 obligation、barrier 与 materialization 强制由 Runtime 承担。
- **Policy Engine（含其插件集合）位于控制面内部**，不构成独立一级侧面。控制面 mechanism components 反向调用 policy plugins，从而保持“机制不感知具体算法”。

### 10.5 关键设计决策：不是所有投机都需要独占隔离环境

**这是能省掉最多复杂度的一条。**

回看 11 篇：**PASTE、SPORK、Speculative Tool Calls、SPAgent 都没有 fork agent** —— 它们只投机"边上的值"（提前发一次工具调用），agent 进程始终只有一个。只有 DSP / ISP 真的跑两个 agent 系统，Skim 跑一条独立快路径。

对应边模型：

| SpecTask 形态 | 是否需要实例 | 适用论文 |
|---|---|---|
| **Task-level**：提前执行这条边上的一次调用 | **否** | PASTE、SPORK、Spec Tool Calls、SPAgent、Speculate with Memory |
| **Instance-level**：提前走 agent 的下一步 / 下一段轨迹 | **是** | DSP、ISP、Skim；**以及所有嵌套（§9）** |

> 若把一切都塞进 agent fork，等于为 11 篇里的 4–5 篇付出它们根本不需要的代价。

**fork 保真度是一个谱，应作为 agent 声明的能力，而非底座定死的选择**：

| 方式 | 前提 | 成本 |
|---|---|---|
| 轨迹重放到新实例 | agent 是轨迹的纯函数（多数 ReAct 满足） | 最低，跨语言 |
| 上下文 COW fork | 有进程内状态但可序列化 | 低 —— SPORK 的 prefix-cache fork：探针预填 1.6s→0.35s，prefill 干扰 <1% |
| 进程 / VM 快照 | 有隐藏状态、连接、RNG | 高 |

这同时给了多隔离后端（docker / firecracker / gVisor / QEMU）一个**架构上的必要性理由**：不同 agent 需要不同的 fork 保真度。否则多后端只是"支持得多"，不构成架构论点。

### 10.6 虚拟执行：伪造返回值有正确性问题，但有救法

**对不可逆或非幂等的 API 直接返回成功、构造一个返回结果 —— 这个危险，且 11 篇无一这么做。**

**问题**：伪造的返回值会成为该分支后续行为的条件。即使匹配是 L0 全等，该分支的轨迹也已建立在一个编造的 observation 上 —— 它破坏了轨迹等价，而破坏点**在验证机制看不见的地方**。

论文的三条真实路线：

| 路线 | 语义 | 论文 |
|---|---|---|
| TRANSFORMABLE | dry-run / 打到 staging，拿到**真实**结果，只是执行不变异 | PASTE |
| STAGEABLE | 干脆不执行，缓冲效果并**阻塞**该分支到 commit barrier | Spec Interaction（类比 MIPS R10000 写缓冲）、cost-aware |
| 不投机 | 硬闸 | SPORK（manifest）、Skim、Spec Tool Calls、Speculate with Memory（白名单） |

**救法**：把伪造的返回值本身建模成一个 **Proposal**，只是其生产者是 Effect Coordinator 而非 Proposer。这样它进入 AssumptionSet，走同一套 confirm/refute —— 风险从隐藏变成**可见、可计价、可被 Fidelity Monitor 拦截**。

### 10.7 边界：Agent 与 Isolation 是必需，Engine 在范围外

**Agent Boundary 与 Isolation Boundary 是必需的；Engine Boundary 在范围外（D-013 修订）。**

三条边界不能并列 —— **绝大多数 agent 通过 API 使用大模型，根本控制不了提供方的 KV cache**。证据也站在这一侧：SPORK 自述 "closed-source APIs cannot host the fork thread"；Speculative Tool Calls 干脆去提议一个尚不存在的新端点 `POST /cache-tool-output/{response_id}`，并说“提供商有动机采纳”；PASTE 需要 vLLM startup hook；SPAgent 需要引擎暴露 hybrid batch 计时与准入覆盖。**这 4 篇是自托管专属，本系统不做这一层；下列分析保留为未来自托管时的候选。**

| 边界 | 地位 | 实现者 | 需要的能力 |
|---|---|---|---|
| **Agent Boundary** | 必需 | Interposer | 四接缝：LLM 调用（流式 / 中断）、工具分发、轨迹写入（truncate-and-rewrite）、调度 |
| **Isolation Boundary** | 必需 | Runtime（由 Runtime Adapter 接入） | docker / firecracker / gVisor / QEMU / 自研；按 fork 保真度选型（§10.4） |
| **Engine Boundary** | **范围外** | 推理引擎钩子（不做） | prefix cache 共享、logprobs、draft proposer、准入覆盖 FCFS、KV/队列压力可见 |

**降级不是排除，而是契约协商（流③）的正常输出**：API 后端不提供 Engine 能力时，依赖它的 policy 配置被自动标为不可用或降级，并把降级原因报给用户。这恰是 §11.1(B) 双向协商存在的意义。

**API 化带来的两个后果，必须接住：**

1. **成本问题变严重。** 自托管靠 continuous batching，投机的 LLM 调用在低负载下近乎免费（Speculative Actions 明确指出这一点）。走 API 则每次投机按全价计费。**因此 CostGate 与准入从"锦上添花"变成必需**，§11.2 的 token 记账口径也从边缘指标变成核心指标。
2. **别把 prompt cache 一起扔掉。** 主流 API 都有 prompt caching。你控制不了它的粒度，但可以**塑造请求前缀去命中它** —— 是 *prefix-cache-aware* 而非 *prefix-cache-controlling*。这个区别在 API 场景下是少数还能拿到的引擎级收益。

### 10.8 自研 Runtime 的定位

**它的存在理由不是"又一个后端"，而是"证明 Isolation Boundary 的接口设计对了"** —— 通用后端能接上，专用后端能把接口用满。

**通用沙箱为投机不需要的东西付费：**

| 通用沙箱的开销 | 投机为何不需要 |
|---|---|
| **双向对抗性隔离** | 容器/VM 要防恶意代码，且对称防护。投机实例是**你自己的 agent**，只需防**意外干扰**而非攻击 —— 需求弱得多，机制可以轻得多 |
| 持久化 / 崩溃一致性 | 实例短命且多数被丢弃 |
| 完整 syscall 面 | agent 只碰网络、文件、exec 等窄集合 |
| **merge / 冲突消解** | 投机**单赢者，永不 merge** —— 无需三方 diff、无需 CRDT。这是 git-worktree 类分支系统的主要成本 |
| 长生命周期资源核算 | 实例存活以秒计 |

**而投机需要、通用沙箱不给的：**

- **廉价 COW fork**（docker 不 fork，只能起新容器再重放）
- **批量快速销毁** —— `k` 个里要拆掉 `k−1` 个，**teardown 延迟在关键路径上**。这一点容器场景从来不在乎，却是投机的主要成本项
- **假设感知的效果暂存** —— EffectClass × StateDomain 矩阵，通用沙箱不知道 commit barrier 为何物
- **确认时的增量状态提升** —— 赢家的状态要廉价地变成权威

**嵌套下的补充要求**：「实例间无协作」在**嵌套下不完全成立**。depth-2 建在 depth-1 的状态之上 —— 不是协作，但是**父子状态依赖**。所以自研 runtime 必须支持**链式 fork（fork 的 fork）**，并在 depth-1 假设被确认时把 depth-2 **重新挂载到权威之上**。

---

## 11. 问题陈述与评测指标

### 11.1 两处框架改进

**(A) EffectClass × StateDomain 是二维的，第 4 节只有一维。**

第 4 节的 EffectClass（PURE / IDEMPOTENT / STAGEABLE / TRANSFORMABLE / COMPENSABLE / OVERWRITABLE / IRREVERSIBLE）描述的是**纪律**；而 filesystem / process / memory / GUI vs network / database / email / payment 描述的是**状态域**。二者正交，交叉出的每个格子对应一种具体隔离机制：

| StateDomain | 典型可用纪律 | 隔离机制 |
|---|---|---|
| filesystem | STAGEABLE | overlayfs / union mount |
| process, memory | STAGEABLE | namespace / COW fork / VM 快照 |
| GUI | TRANSFORMABLE | 影子渲染 / 离屏合成 |
| network (read) | PURE | 直通 |
| network (write) | TRANSFORMABLE / STAGEABLE | 录制回放 / 代理拦截 |
| database | STAGEABLE | 未提交事务 / savepoint |
| email, payment | COMPENSABLE / IRREVERSIBLE | outbox 模式延迟投递 / 硬闸 |

**这个矩阵就是 Effect Coordinator（§10.5）的设计空间。**

**(B) "Speculation Contract" 优于 "Fidelity Contract" —— 双向协商，而非单向声明。**

- **单向声明形态**：算法声明保证什么，底座校验并推上界。
- **正确的形态是跨层双向协商**：
  - **上层 → 下层**：预测置信度、验证条件、失败代价
  - **下层 → 上层**：隔离能力、隔离代价、可用纪律集合

后果是实质性的：单向声明下底座只能**拒绝**不可行组合；双向协商下底座能回答两个更有用的问题 —— "给定这个隔离后端，该算法最多能激进到什么程度"、"要支持该算法，隔离层至少要提供什么"。**静态约束求解即协商器的下层形态。**

这也正是现有工作的缺口所在：一类工作只管预测与提前执行，靠"限制为 read-only / idempotent / sandboxed"绕开副作用；另一类工作只管系统层的状态隔离、分支、事务与回滚，不建模上层的置信度、验证条件与失败代价。**两条路径之间没有契约。** 本文档第 4 节的统计（9/11 篇用只读白名单绕开）是这一判断的直接证据 —— 这不是各家的疏忽，是这条技术路线的系统性缺口。

### 11.2 指标口径

**(1) "Token 消耗不提高"达不到，应改口径。**

投机必然多花 token，每篇都报了：ISP 成本大致翻倍（最坏情况 approximation process `i` 跑 `(i mod k)+1` 次、target process `i` 跑 `i` 次）；Speculative Actions 的成本公式显式带 `k̃` 因子；DSP 的贡献是把增量**降低** 30%/60%，不是消除。

**可站住且更强的说法**：*在同一 policy 下，底座相对该 policy 的原生实现，token 增量 ≈ 0*。即把"投机的 token 成本"（归算法）与"底座的 token 成本"（归底座）分离。

严格表述：**底座 token 开销上界 = 协议标注量，与投机宽度/深度无关。** 之所以不是严格为零，是因为底座可能需要注入协议标注（Spec Interaction 的 `<cancel>` 标签、`<current_plan>` 计划提示）。

**(2) 现有效率指标只能证明"开销小"，证明不了"有赢面"。**

"相同 policy、有无底座的端到端时间"基线选得对（隔离出底座开销），但它的最好结果是"几乎不慢"。必须再加**至少一个只有底座能拿到的赢面**。按强度排：

| # | 赢面 | 论文证据 |
|---|---|---|
| 1 | **负载相关性** | SPAgent：朴素投机在 >2 rps 时比不投机**慢 49.3%**；PASTE：Tool-Only 消融（只加速工具不改 LLM 调度）**比 baseline 更差**（419s vs LLM-Only 342s vs 完整 270s）。原生实现没有调度层，做不了准入与背压。**"speedup vs. 负载"图上，原生实现在高负载区掉到 1 以下，底座不会。** |
| 2 | **嵌套** | 原生实现不可能嵌套（无 AssumptionSet 即无组合语义）。§9.4 的可检验预测 —— 收益随单步命中率超线性增长 —— 只有底座能验证 |
| 3 | **按任务切换 policy** | per-task 选 policy 能否打败任何单一固定 policy。底座独有 |

### 11.3 指标：四处补充

**(a) 传播延迟需重新定义。** 投机实例的状态是**故意发散的**，不该"同步一致状态到所有实例"。真正在关键路径上的是 **confirm/refute 沿 Assumption DAG 的传播延迟** —— promote 要等它。且它是嵌套的核心可扩展性指标（树越深传播链越长）。建议改为：**假设确认传播延迟 vs. 树深度 / 宽度**。

**(b) 保真度实测，而非声明。** shadow mode 让 losslessness 可**测量** —— 没有一篇论文做过，它们都只是论证。"实测 11 个 policy 声明的契约是否达成"本身就是一个结果，且可能发现某些 lossless 声明在特定负载下不成立。

**(c) 可观测性要落成可测量的量。** 建议两个：(i) 浪费资源中能归因到具体 `(site, proposal)` 的比例；(ii) 每次投机触发/拒绝原因的可重建率。schema 可直接借 cost-aware 的 `SpeculationDecision` 遥测行（<1 KB，含 `i_hat_source` / `overrode` / `tier1|2|3_match` / `tokens_generated_before_cancel` / `C_spec_actual_usd`）。

**(d) 副作用泄漏做成测试套件，而非单个数字。** 这是最强的差异化点，值得做成可交付 artifact。PASTE 提供了现成的报告范式：跨所有 workload 标记并阻止 **602 个**潜在副作用投机动作，且无任务最终结果偏离 baseline。**补一条无人测过的**：嵌套下多个不同深度的 staged effect 提交时的**顺序正确性**。
