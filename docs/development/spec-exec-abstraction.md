# Agent 投机执行底座：抽象设计札记

> 本文是投机执行底座的完整设计札记（论文调研、推导过程、候选方案与未来构想）。已收敛的最终设计/架构决策以 [`docs/design/speculation-decisions.md`](../design/speculation-decisions.md) 为准。
>
> **范围决策（2026-08，D-013 修订）**：engine plane（推理引擎钩子层）不在范围内。文档中的双层分析、engine plane 表格与 §10.6 的 Engine Boundary 均保留为论文调研背景与未来自托管时的候选。

## 1. 核心模型（已下沉）

本节原先的 Edge 生命周期、Proposal/Authority/SpecTask/Reconciliation/Disposition 定义，以及 `A_u` 的启动策略，已分别纳入 [设计模型](../design-model.md) §2.1、§3.1 和 [Policy Decisions](../design/policy-decisions.md) §2。这里不再维护第二份规范表述；本札记后文只保留论文调研、候选方案和实现分析。

---

## 2. 正交轴与取值域

本节已迁移至 [Policy Decisions](../design/policy-decisions.md)，后续在正式文档中继续整理其决策分类与取值。

## 3. 组合性（已下沉）

AssumptionSet、同一 Edge 上的并行候选、嵌套 Edge、single-winner、确认/否定传播和非空假设数据的信息流约束，统一语义见 [Policy Decisions](../design/policy-decisions.md) §4；Edge 与 Proposal/Branch 的抽象关系见 [设计模型](../design-model.md) §3.1。本札记不再重复定义 Assumption DAG。

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

效果分类、状态域、处理模式（包括阻塞、暂存、转换、伪装执行、补偿）及其能力校验的规范语义，统一见 [效果管理设计](../design/effect-management.md) §1–§5；`effect` decision 的接口和模式见 [Policy Decisions](../design/policy-decisions.md) §2.5。本札记不再维护第二套 EffectClass 或 commit-barrier 规则，仅保留上面的论文证据和研究动机。

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

这四类接缝是论文调研得到的候选观察点，不是每个 Agent integration profile 都必须实现的固定接口。现行目标边界已迁移到 [Agent 接入侧设计](../design/agent-integration.md)。

**实测的侵入量**（支持"无侵入可行"这个命题）：

- **PASTE**：agent 侧 ~5k 行 TS（tool dispatch 中间件 + LLM client 轻量 wrapper），LLM 侧 ~2k 行 Python（**未修改的 vLLM** 上挂 startup hook）。gemini-cli / Qwen Deep Research / Virtual-Lab 三个 agent 只需要一个小的 dispatch wrapper，**工具 API 完全不改**。
- **SPORK**：thin controller over standard completion APIs，无重训练、无辅助模型、无离线 trace。
- **Skim**：对底层 agent **只有一个硬性要求** —— 能从任意 URL warm-start。不需要注入推理历史（因为只有 URL 过界）。三个观察模态完全不同的 backbone（WebVoyager 截图 / AgentOccam 剪枝 DOM / BrowserUse 浏览器原语）都能接。
- **Speculative Tool Calls**：提议一个新端点 `POST /cache-tool-output/{response_id}`，body 是 `{name, params, output, keep_alive}` 数组，按 `name` + canonicalized `params` 匹配后续 tool call。

### 关键发现：底座必须是**双层**的（范围决策：engine plane 不做）

> **范围决策（2026-08，D-013 修订）**：engine plane 整体排除在范围外 —— 本系统以 API 方式使用大模型，下面 4 篇的机制全是自托管专属。本小节与 §10.6 的分析保留为论文调研背景与未来自托管时的候选。

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

| 用途 | 论文事实与用途 |
|---|---|
| Predictor 数据供给 | PASTE 需要历史 trace 挖 pattern，DSP 需要 `(state, k)` 对做在线 RL，Speculate with Memory 需要完成轨迹与 miss episodes |
| 确定性评测 | Speculate with Memory 采集一次 actor 轨迹后重放，以消除 actor 方差并把 API 成本隔离到 speculator |
| 成本校准 | 真实命中率 `p`、接受率 `α` 和取消前生成比例 `ρ` 只能从 shadow/replay 数据中测得 |
| 灰度路径 | cost-aware 给出 offline replay → shadow → canary → online calibration → drift kill-switch 的上线过程 |

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

正式的 Edge、Branch 和 `AssumptionSet` 语义见 [设计模型](../design-model.md) §3.1 与 [Policy Decisions](../design/policy-decisions.md) §4。本节只保留研究视角：嵌套把“假设在哪里引入”（Edge）与“假设由谁携带”（Branch 实例）连接起来；ISP、DSP 等论文已经展示了单链嵌套，尚未解决的是带 breadth 的深层树形扩展。

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

“投机实例应与权威实例同构、业务逻辑保持透明”已是正式设计边界。开发调研中仍值得验证的是：嵌套下 Cost/Admission 是否需要读取实例存活概率或其他祖先依赖事实；`survival` 及其接口形态目前只是候选方案，不是固定契约。

### 9.6 错误率：L0/L2 不升，L1/L3 相乘

一种待验证的成本/风险假设是：严格 trajectory-equivalence 下嵌套增加的主要是浪费，而软匹配或 validator 的误接受风险可能随层数复合。`depth > 1` 是否限制某些 Fidelity/match 组合，留作 Contract profile 或实验假设，不在本札记中定义为通用硬约束。

### 9.7 实现复杂度：真正新增的只有两项

| 能力 | 是否新增 | precedent |
|---|---|---|
| 上下文 COW fork | **新增** | SPORK 的 prefix-cache fork 证明 KV 层近乎免费：探针预填 1.6s → 0.35s，prefill 干扰 <1% |
| frontier 预算调度器 | **新增** | PASTE 的 utility-per-resource 排序 + SPAgent 的两级调度 |
| 级联取消 | 本就需要 | Spec Interaction 的 cascade-cancel；ISP 的 kill `target_k, k>j` |
| 提升级联 | 本就需要 | PASTE 的 promote；是 `assumptions == ∅ ⟺ 权威` 的自然推论 |

父 Proposal 确认后的后代重新挂载、身份保持和隔离状态复用已由正式 Branch 语义定义；本节只保留实现成本与后端能力的研究比较，不重复规定状态转换。

### 9.8 与其他轴的交互汇总

嵌套与 `launch`、拓扑、`match`、`dispose` 和 `effect` 的交互仍是实验与 Contract 设计的研究问题。当前正式约束只有：后代保留全部未决依赖；祖先否定会使其失效；后代自身依赖清空前不能进入权威状态。关于 Absent、深层匹配风险、frontier 预算和嵌套 salvage 的更强结论均不在此处固化。

---

## 10. 系统边界与后端研究

### 10.1 四侧调用路径

当前权威结构见 [架构](../architecture.md)：四侧表示完整调用路径中的逻辑位置，不表示四个由 AScout 拥有的子系统，也不直接规定 crate、进程或服务拓扑。

各部分的下一层说明分别见 [Agent 接入侧设计](../design/agent-integration.md)、[控制面设计](../design/control-plane.md) 与 [Runtime Adapter 接入指南](runtime-adapter.md)。Runtime 是外部能力来源，不由 AScout 建立通用内部设计。Instance、Branch、AssumptionSet 与 disposition 的现行语义见对应正式设计文档。

### 10.2 跨侧数据流

当前跨侧闭环统一为：Agent 接入侧形成请求，控制面校验并编排受管操作，Runtime Adapter 映射到外部 Runtime，结果与事件沿反方向返回。具体责任分别见上述三份下沉文档与 [Speculation Contract](../design/speculation-contract.md)。

### 10.3 使用模型

角色与接入责任已经下沉到 [Agent 接入侧设计](../design/agent-integration.md)。保留的研究结论是：端用户与 Agent 默认不必感知投机，但这不是禁止显式协作的不变量；用户或 Agent 可以通过受控入口提供输入或参与裁决。具体 effect 元数据从哪里取得仍是开发问题，不再在组件使用模型中预设为唯一人工接入成本。

### 10.4 投机形态与隔离成本

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

### 10.5 虚拟执行的论文路线

**对不可逆或非幂等的 API 直接返回成功、构造一个返回结果 —— 这个危险，且 11 篇无一这么做。**

**问题**：伪造的返回值会成为该分支后续行为的条件。即使匹配是 L0 全等，该分支的轨迹也已建立在一个编造的 observation 上 —— 它破坏了轨迹等价，而破坏点**在验证机制看不见的地方**。

论文的三条真实路线：

| 路线 | 语义 | 论文 |
|---|---|---|
| TRANSFORMABLE | dry-run / 打到 staging，拿到**真实**结果，只是执行不变异 | PASTE |
| STAGEABLE | 干脆不执行，缓冲效果并**阻塞**该分支到 commit barrier | Spec Interaction（类比 MIPS R10000 写缓冲）、cost-aware |
| 不投机 | 硬闸 | SPORK（manifest）、Skim、Spec Tool Calls、Speculate with Memory（白名单） |

**救法**：把伪造的返回值本身建模成一个 **Proposal**，只是其生产者是 Effect Coordinator 而非 Proposer。这样它进入 AssumptionSet，走同一套 confirm/refute —— 风险从隐藏变成**可见、可计价、可被 Fidelity Monitor 拦截**。

### 10.6 边界：Agent 与 Isolation 是必需，Engine 在范围外

**Agent Boundary 与 Isolation Boundary 是必需的；Engine Boundary 在范围外（D-013 修订）。**

三条边界不能并列 —— **绝大多数 agent 通过 API 使用大模型，根本控制不了提供方的 KV cache**。证据也站在这一侧：SPORK 自述 "closed-source APIs cannot host the fork thread"；Speculative Tool Calls 干脆去提议一个尚不存在的新端点 `POST /cache-tool-output/{response_id}`，并说“提供商有动机采纳”；PASTE 需要 vLLM startup hook；SPAgent 需要引擎暴露 hybrid batch 计时与准入覆盖。**这 4 篇是自托管专属，本系统不做这一层；下列分析保留为未来自托管时的候选。**

| 边界 | 地位 | 实现者 | 需要的能力 |
|---|---|---|---|
| **Agent Boundary** | 必需 | Agent 接入侧 | 由具体 integration profile 选择可观察接缝，统一进入 Agent-facing semantic API |
| **Isolation Boundary** | 必需 | 外部 Runtime（由 Runtime Adapter 接入） | docker / firecracker / gVisor / QEMU / 自研等候选；能力由 Adapter 声明并验证 |
| **Engine Boundary** | **范围外** | 推理引擎钩子（不做） | prefix cache 共享、logprobs、draft proposer、准入覆盖 FCFS、KV/队列压力可见 |

**降级不是排除，而是契约协商（流③）的正常输出**：API 后端不提供 Engine 能力时，依赖它的 policy 配置被自动标为不可用或降级，并把降级原因报给用户。这恰是 §11.1(B) 双向协商存在的意义。

**API 化带来的两个后果，必须接住：**

1. **成本问题变严重。** 自托管靠 continuous batching，投机的 LLM 调用在低负载下近乎免费（Speculative Actions 明确指出这一点）。走 API 则每次投机按全价计费。**因此 CostGate 与准入从"锦上添花"变成必需**，§11.2 的 token 记账口径也从边缘指标变成核心指标。
2. **别把 prompt cache 一起扔掉。** 主流 API 都有 prompt caching。你控制不了它的粒度，但可以**塑造请求前缀去命中它** —— 是 *prefix-cache-aware* 而非 *prefix-cache-controlling*。这个区别在 API 场景下是少数还能拿到的引擎级收益。

### 10.7 自研 Runtime 的定位

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

效果分类与状态域的正式语义见 [效果管理设计](../design/effect-management.md)，Policy 表达与 Runtime Adapter 接入行为的语义对齐契约见 [Speculation Contract](../design/speculation-contract.md)。本节只保留研究结论：现有工作通常以只读白名单回避副作用，预测/调度与隔离/事务之间缺少统一契约；AScout 的机会在于把策略表达、后端能力与效果处理接成一条可校验的路径。

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
