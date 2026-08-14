# 投机执行底座：设计决策记录

本文档从 [`../development/spec-exec-abstraction.md`](../development/spec-exec-abstraction.md) 中提取**最终成立的设计/架构决策**，每条整理为「问题 → 选择 → 原因」的问答条目，按原文档出现顺序编号。格式与 `docs/design/decisions.md` 一致。设计过程本身（方法论、优先级、修订轨迹）不收录。

本文档与 `docs/design/decisions.md` 是**同一系统**的两套设计记录，各自独立编号（本文档 D-001..D-028；后者 D-001..D-039）：后者从 Speculative Tree 机制视角记录已收敛语义，本文档从投机执行底座视角记录已收敛语义。投机执行底座就是当前系统（AScout）本身的设计目标，不是另一套相邻系统。两套记录之间的术语对应与权威范围见 [`terminology.md`](terminology.md)；一级架构统一以 [`../architecture.md`](../architecture.md) 与 `decisions.md` D-039 为准。

---

## D-001：轴体系如何划分？投机形态是独立轴吗？

**问题**：Overlap / Bypass / Elide / Overwrite 是否构成一条独立的"Shape"轴？

**选择**：不构成。Shape 是"权威启动策略 × 效果释放策略"乘积上一块区域的名字，拆为两处：

- **轴 1（Authority Launch）**：`A_u` 是否启动、何时启动 —— `Eager`（与 SpecTask 同时启动）/ `Lazy`（validator 拒绝后补启）/ `Absent`（永不启动）。取值可为状态依赖函数（SPAgent 全部候选低于阈值 β 时从 Absent 切到 Eager）。
- **Effect 轴的 release-early 取值**：Overwrite 归此处 —— OS 变体的权威是 Eager 的，特殊之处是投机效果在验证前就释放到外部世界（last-write-wins）。

**原因**：拆开后 Bypass 与 Elide 自动被边模型覆盖（`A_u` 是否启动变成策略而非模型固有属性），且两条性质变成可推导的：不启动 `A_u` ⟹ 无 `i` 可比 ⟹ 接受判据只能是 L3 一元谓词（解释为什么 Skim 是唯一用 validator 的）；Lazy 拒绝后是串行追加 ⟹ Salvage 是必需品而非可选优化。

---

## D-002：什么时候可以省掉权威路径（Bypass）？

**问题**：`A_u` 可省的条件是什么？

**选择**（判据）：`A_u` 可省 ⟺ (`u` 是**计算**而非**世界**) ∧ (保真度契约**不以** `A_u` 的输出定义正确性)。

**自证**：`u` 是世界（环境 / 人）→ Eager 被物理强制；契约以 `A_u` 定义正确（trajectory-equivalent）→ Eager 被契约强制；两者都不成立 → 恰好是全部 11 篇里仅有的两篇非 Eager（Skim、SPAgent Aggressive）。判据有预测力，不是事后贴标签。

---

## D-003：预测对象是设计轴吗？

**问题**：所谓"猜工具结果 vs 猜推理结果"（以及 plan step / 子轨迹 / 链式 / 未来用户输入等七种预测对象）能否构成一条独立轴？

**选择**：不构成。全部可归约：plan step = action + observation；子轨迹 = (action, observation) 复合；链式 = 显式组合；未来的用户输入 = observation（其 action 是"请求输入"，`<pause>` 字面就是）。连 action/observation 二分也不是关于值的 —— 同一个值生产侧看是 action、消费侧看是 observation，取决于站在边的哪一侧。

**原因**：唯一的操作性残留（生产者是世界还是计算）正是轴 1 可省性判据的第一个合取项 —— 它是**轴 1 的投影**。延迟节省公式（Speculate with Memory 三类型）可由"边的位置 + 两侧延迟"推出，落进成本模型即可。

---

## D-004：Fidelity 在设计中是什么地位？

**问题**：保真度契约（lossless）如何进入设计？它是与 Equivalence、Authority 并列的轴吗？

**选择**：不是轴，是**导出不变量** —— 各轴设置上的一个合取。也不与 Equivalence 合并：二者可独立变化（SPORK 是 L0 + trajectory-equivalent；Speculative Interaction Agents 用精确 ID 匹配却只有 bounded-degradation，掉保真度的通道是"模型在不完整输入上已动过手"而非匹配谓词）。

**原因**：Equivalence 是机制层旋钮，Fidelity 是系统层承诺。保真度可从至少四条通道丢失：放松匹配、提前释放效果、不启动权威、模型协议退化。11 篇说 lossless 说的不是一件事（Trajectory / Outcome / Bounded-degradation / Best-effort 四级）。

**强制机制**（三步）：静态由轴设置推保真度上界 → 加载期声明与上界对撞、不一致拒绝加载 → 运行期 shadow mode 实测。静态只能证上界，证不了达成。

---

## D-005：等价关系定义在什么上？

**问题**：判定"猜中了"用什么层次？底座提供什么？

**选择**：四层 —— L0 语法相等 / L1 语义等价 / L2 效果同余 / L3 结果可接受。底座把 `≈` 定义为可插拔的**同余关系**而非相等判定。

**关键结论**：

1. **L2 是被低估的设计** —— Speculative Actions 的 cache key 是 `(h,q)` call specifier，比对的不是猜的 action 本身而是它蕴含的下一次调用。等价定义在"值对下游的影响"上，天然更宽松且仍 lossless。
2. **L3 是唯一能支撑 Bypass 的等价**（不需要权威值），代价是必然 lossy —— verifier 应调成"倾向拒绝"（Skim 的偏置方向），让不确定性转化为 fallback 而非静默错误，这是底座默认值。
3. **放宽 `≈` 会静默打破 lossless**（ISP 承认、SPORK 拒绝 name-only 匹配）⟹ 底座必须把 `≈` 层次与 Fidelity 契约绑定校验。

---

## D-006：权威值一定来自权威路径吗？

**问题**：`resolve()` 的输入只能来自那条已经在跑的权威路径吗？

**选择**：不是。`resolve()` 必须能接受**外部注入的权威值**（ISP 的 oracle interrupt：用户注入的值被当作 oracle，"effectively a third agent"）；并支持按 index 取消权威任务。

**轴 1 与轴 6 的关系**（约束，不是恒等）：轴 6 管谁产出 `i`，轴 1 管 `A_u` 跑不跑，耦合只有一处 —— 生产者是世界则 Eager 物理强制，轴 1 的可行域是轴 6 的函数。

**设计承认的合法组合**：`Authority=人 × launch=Lazy` —— 先按对用户意图的猜测往下干、只在 validator 拒绝时打扰用户（ISP 是它的反向）。一个好抽象应该能生成没人做过的组合，这条通过。

---

## D-007：处置（Disposition）枚举是什么？

**问题**：投机任务的处置集合是什么？

**选择**：Reuse / Promote / Discard / Preempt / Re-execute / Compensate / Overwrite / **Salvage**。其中 Salvage（残值回收：失败后回收部分进展）是关键设计 —— 三个独立实例在三个层次：Skim 的 warm-start（被拒后只有 URL 过界）、SPORK 的 partial-token accept（被拒 probe 变 draft）、PASTE 的 promote（在飞进度转正）。共性：投机工作往往有"公共前缀"结构。

**设计**：底座把"部分进展的表示与移交"做成一等抽象，而非每个算法自己发明：

```
interface Salvageable<P> {
    progress(): P                      // 提取当前部分进展
    handoff(p: P, to: AuthorityPath)   // 移交给权威路径
}
```

**范围**：Salvage 不是 Lazy 的专属（Skim 是 Lazy、SPORK 是 Eager）。另：Promote 是独立终态（在飞任务命中后转权威身份、变 non-preemptible）；Preempt 与 Discard 语义不同（SPAgent 等待超 `t_w` 是丢弃不是抢占 —— 重叠价值已过期）。

---

## D-008：Mechanism / Policy 的线画在哪？

**问题**：底座提供什么、算法填什么？

**选择**：**Mechanism（底座提供，算法不可改）**：Site 注册与 admissibility；隔离域；权威到达事件 + Reconciliation 状态机；Assumption DAG 与传播；Effect Gate / commit barrier；资源准入、优先级、抢占、背压；规范化资源记账；Fidelity 契约的强制与审计。

**Policy（插件，有默认实现）**：Predictor / `S_u`；Equivalence `≈`；Authority launch；Topology（breadth × depth）；Cost model & gate；Salvage；Compensation。

**判据**：如果某个论文的机制被迫写进 Mechanism 层，说明抽象漏了一轴。

---

## D-009：多个投机同时在飞且互相依赖时，语义是什么？

**问题**：组合性问题 —— 投机 work 能不能依赖另一个未确认的投机 work？（11 篇没有一篇给出通用语义）

**选择**：每个 SpecTask 携带 provenance（AssumptionSet）：`Assumption = (site_id, proposal_id)`；`AssumptionSet = frozenset[Assumption]`。

规则：`AssumptionSet == ∅` ⟺ 权威工作；某 assumption **confirm** → 从所有 task 的 set 移除，变空的可以 commit；**refute** → 依赖它的整个子图进入 Salvage → Discard；投机 task 之间可互相依赖，set 做并集。

**统一了三家局部方案**：Spec Interaction 的 cascade-cancel、Speculative Actions 的不一致子树剪枝、ISP 的 kill 建在坏前缀上的 `target_k (k > j)`。

**顺带解决可见性泄漏**：投机产物绝不能进入权威 context。一旦允许推测链，需要一条可静态检查的信息流约束：非空 AssumptionSet 的数据只能流向同样非空的容器，唯一降级通道是 `resolve()`。底座能强制它，单个算法做不到。

---

## D-010：效果层是只读白名单，还是真正的效果管理？

**问题**：9/11 篇只投机只读工具，底座如何处理副作用？

**选择**：底座提供真正的效果管理层，把算法适用范围从只读扩展到可写。EffectClass：

```
PURE                              // 无外部效果，随便投机
IDEMPOTENT(natural_key)           // 重放收敛到同一状态
STAGEABLE(commit_barrier)         // 效果可缓冲，屏障后释放
TRANSFORMABLE(variant)            // 存在非变异的等价执行：dry-run / staging
COMPENSABLE(undo_fn)              // 可补偿（refund/replace）
OVERWRITABLE(ordering)            // last-write-wins 收敛
IRREVERSIBLE                      // 不可投机，硬闸
```

**三个必须做进 Mechanism 的细节**：

1. **commit barrier 触发条件显式建模** —— 照抄处理器 commit/retire 语义（Spec Interaction 类比 MIPS R10000 写缓冲："write operations are buffered but not allowed to execute until the instruction is committed"）。
2. **TRANSFORMABLE 是首选路径** —— dry-run/staging 把一个不可投机操作变成可投机且不需要补偿，比 COMPENSABLE 便宜得多。
3. **审计必须是 mechanism** —— PASTE 标记并阻止 602 个潜在副作用投机动作、最终结果无偏离，这种数字只有底座能产出。

---

## D-011：资源与准入进底座吗？

**问题**：投机的收益是负载相关的（SPAgent 朴素投机 >2 rps 时比不投机慢 49.3%；PASTE Tool-Only 消融比 baseline 更差），单个算法能自己解决准入吗？

**选择**：不能，底座必须提供五项：

1. **准入控制** —— 四道闸：executable / policy-safe / 置信度与预期收益 / 预算内
2. **优先级与抢占** —— 投机 job 低优先级、可抢占；命中转正后 non-preemptible
3. **过期丢弃** —— `t_w`：等太久的投机请求直接丢（与抢占语义不同，重叠价值已过期）
4. **跨层可见性** —— 推理引擎压力（PASTE 的 `EnginePressure(B) = DecodeLoad(B) + γ·KVLoad(B)` 维持在带内）。**范围决策（D-013 修订）**：engine plane 不做后引擎内部压力不可见，该项降级为 API 可观测信号（延迟、限流、配额余量）；引擎级压力感知留给未来自托管场景
5. **反向调度** —— SJF 让投机请求插队（投机输出 <10 token，延迟排队主导，插队几乎不拖慢推理）

**原因**：负结果证明单个算法在自己的抽象里解决不了 —— 他们没有调度层。

---

## D-012：什么算"浪费"？谁定口径？

**问题**：论文对"什么算浪费"的口径互不兼容，无法横向比较。

**选择**：底座定义规范语义，三分：(a) **无论如何都要花的**（命中时投机 work 就是权威 work）；(b) **纯浪费**（未命中且无残值，`(1−P)·C_spec`）；(c) **部分成本**（被抢占/取消时已消耗部分，fractional —— 流式取消只计已生成 token，但 posterior 仍记一次完整失败）。

**原因**：各自记账则无法比较、无法做全局预算、无法做 kill-switch。口径必须包含 token 之外的资源（GPU 占用、并发槽、外部 API 配额），否则 cost plugin 拿到的是价目表价格而非真实边际成本。

---

## D-013：底座是单层还是双层？

**问题**：纯 API 层的底座够吗？

**选择**：底座是**单层 orchestration plane**；engine plane（推理引擎钩子层）**明确排除在范围外，本系统不做**（初版曾选双层，此为修订）。被排除的机制：Speculative Tool Calls（KV cache 层 commit）、SPORK D3（partial-token accept 接进 draft proposer）、SPAgent（两级调度器）、PASTE（co-scheduler），以及 sequence residency 等纯 engine 侧收益。

**原因**：本系统以 API 方式使用大模型为主（SPORK 自述 "closed-source APIs cannot host the fork thread"；Speculative Tool Calls 提议的是尚不存在的新端点），engine plane 的 4 篇全部是自托管专属，拿不到这些收益也不打算自托管。被排除的机制不影响 orchestration plane 的核心抽象（轴体系、Assumption DAG、效果管理、资源准入），只在接入接缝 1（LLM 调用边界）上收窄能力清单（见 D-028 修订）。若未来自托管推理引擎，可重新评估。

---

## D-014：shadow / replay 是四套基础设施吗？

**问题**：Fidelity 审计、predictor 数据管道、成本校准、上线安全，要分别建吗？

**选择**：一套。同一套 shadow 基础设施同时解决：Fidelity 审计（投机与串行同跑、只提交串行结果，是验证 lossless 的唯一手段）、predictor 数据管道（PASTE 挖 pattern、DSP 学 k、Speculate with Memory 的 miss episodes，三家各造轮子）、cost 校准（`p` / `α` / `ρ` 只能从影子流量测）、上线安全（五阶段灰度：offline replay → shadow → canary → online → drift kill-switch）。

**方法学细节**：actor replay（采集一次轨迹后重放）是一等能力 —— 消除 actor 方差、把 API 成本隔离到 speculator，让 predictor 评测变成确定性。

---

## D-015：嵌套投机的拓扑怎么设计？

**问题**：深树投机的成本上界是什么？depth 要作为策略参数手调吗？树长什么形状？

**选择**：

1. **成本是速率界，不是组合界** —— 分支只在投机或真实调用返回时才延伸、不一致子树立即剪枝 ⟹ 系统最多领先 `a/b` 步，活跃分支数与视界 `T` 无关。真正的约束是"权威路径还剩多少 wall-clock ÷ 投机器延迟"。
2. **depth 由 frontier budget 涌现，不是策略参数** —— **best-first frontier expansion**：优先队列 key = `P(节点存活) × 期望延迟节省 / 资源成本`，pop 到预算耗尽。它吸收了 breadth-vs-depth 分配；Speculative Actions Thm.4 的贪心是它在 `depth=1` 上的特例。
3. **浅层分叉、深层单链** —— 每层分叉 → `k^d`；浅层分叉后沿最高置信度单链下探 → `k·d`，存活率 `p(k)·p^(d−1)`。`p(k) = 1−(1−p)^k` 对 k 饱和快、成本对 k 线性 ⟹ 花 breadth 买 depth 划算。PASTE 是活证据：top-1 命中 27.8% vs 多候选整体 93.8%，depth 3 存活率 `0.938³ ≈ 0.83` vs `0.278³ ≈ 0.02`。**breadth 是 depth 的使能器。**

---

## D-016：投机实例知道自己被投机吗？

**问题**：透明嵌套会不会改变投机算法本身的行为逻辑？

**选择**：Agent 业务逻辑**不知道**（透明）—— 实例与权威二进制同构，底座追踪 provenance、闸门设在效果层，这是**无侵入的前提**。但 **Cost / admission 策略必须知道**（反射）：算法自己的成本/置信策略假设自己站在权威路径上（DSP 学的 k、cost-aware 的 EV 门），depth-2 存活率 0.25 的实例不知情会**过度投机 4×**。

**接口设计**：`should_speculate(..., survival: float)` —— 嵌套下 `P(实例存活)` 必须由底座透传。**这是对 gating 的改动，不是对算法行为逻辑的改动。**

---

## D-017：Control Plane 与 Instance 的部署关系是什么？

**问题**：投机执行底座的控制面如何管理运行中的 Instance？

**选择**：部署视角采用 k8s / Ray 式的 **一个 Control Plane 对应 N 个 Instance**；Instance 结构同构、数量不定、各自携带状态。这只是四侧一级架构之下的运行对象部署视角：Control Plane 属于控制面侧，Instance 是 Runtime Mechanism 管理的运行对象；Harness Runtime Boundary 与实际 Runtime Adapter 位于二者之间。Instance 不构成独立一级架构侧面。

**架构权威**：完整一级架构统一为 `Agent 接入侧 -> 控制面侧 -> Runtime Adapter 侧 -> Runtime Mechanism`（`decisions.md` D-039）。Mechanism/Policy 分离仍是职责论证，Control Plane / N Instance 图只说明控制面与受管运行对象的数量关系，不能省略 Adapter 侧或替代一级架构图。

**实例的状态维度**：**role** = `Authority` \| `Speculative(d)` —— 不是静态标签，是 `A == ∅` 的函数（promote 就是这个谓词翻转，实例结构必须同构的原因）；**grain** = `own-env` \| `shared-env`（shared-env 仅对 PURE 开放，**隔离粒度由 EffectClass 决定**）；**lifecycle** = Forked → Running → Resolving → {Reused, Promoted, Discarded, Preempted, Salvaged}。

---

## D-018：谁写配置？端用户感知投机吗？

**问题**：系统的使用模型是什么？

**选择**：四类角色，只有一类写配置：端用户完全不感知（提任务、拿结果，只是更快）；Agent 开发者代码不改一行（唯一动作：提供工具 effect manifest）；Operator 声明 policy profile、选隔离后端、设预算旋钮；Policy 作者（研究者）实现 1–6 个插件接口，多数只需实现 Proposer。

**关键设计**：policy 与任务的绑定发生在任务准入时，由路由规则或任务特征决定（底座自己选），不由端用户输入决定。**"端用户不感知"是默认值而非不变量** —— ISP 是唯一故意让端用户感知投机的（UI 协同设计、用户可在间隔内中断并注入自己的值作为 oracle），底座须支持把人作为 Authority 的 opt-in 模式。

**effect manifest 是接入时唯一非零的人工成本且不可避免**（无法从函数签名推断幂等性）—— 从 MCP 的 `readOnlyHint` / `destructiveHint` / `idempotentHint` 或 OpenAPI 动词语义引导生成初稿，降到"自动引导 + 人工确认残余"。

---

## D-019：Assumption Manager 是什么？

**问题**：负责假设 DAG 的组件叫什么、语义是什么？

**选择**：**Assumption Manager** —— 全系统唯一的"谁依赖谁"真相源。假设 DAG 与实例树是**同一份状态的两个索引**（`by_assumption` / `by_instance`）。它调 Equivalence 插件、强制信息流规则、坐在关键路径上执行决策过程（`resolve()` 本质是一次 commit 决策），不是被动的数据结构。

**内部强制的不变量**（集中在一处，调用者无法违反）：`A == ∅ ⟺ role == Authority`；refute 沿依赖闭包级联；AssumptionSet 非空的数据不得流向空容器。

**命名依据**：形状同构的强先例 —— 数据库 Transaction Manager / Lock Manager（拥有状态、强制不变量、在关键路径、被他人调用）；让架构图去教抽象（全文概念中心是 assumption）。

---

## D-020：resolve 与调度、效果各归哪个组件？

**问题**：resolve 逻辑、调度决策、效果协调是独立组件还是合并？

**选择**：三处独立，各有一个理由：

- **resolve 不独立成组件，住在 Assumption Manager 里** —— 它的"状态"（待决 site 表）就是图本身（pending 状态的 assumption 节点恰好是 `(site, proposal)` 对），是**对图的无状态逻辑**，未通过"一个盒子 = 一份独立状态 + 一份独立接口"检验。体系结构对应：`Assumption Manager + resolve ≈ ROB`（持有投机状态 + retire/squash）；`Scheduler ≈ issue/dispatch`。
- **Scheduler 独立** —— 有自己的状态（优先队列、预算热计数器、压力读数）；延迟类别不同：`resolve` 在关键路径上（权威路径阻塞等待），调度决策晚 50ms 只是少赚，共享事件循环产生**队头阻塞**，把调度簿记压到权威路径上（Speculative Tool Calls 实测短工具场景下逐出/重入簿记盖过延迟隐藏收益 —— 关键路径簿记吃掉赢面是真实失败模式）。
- **Effect Coordinator 独立** —— 它在控制面持有 Effect Catalog、效果分类与协调计划；真实 staged effect、obligation 和 materialization 状态属于 Runtime Mechanism。两类状态都不属于 Assumption DAG，且失败模式不同：图错了损失性能，Effect 错了真的发出一封邮件。安全闸必须能被单独审计。

---

## D-021：记录类组件是一个还是三个？预算计数器归谁？

**问题**：Ledger / Fidelity Monitor / Trace & Shadow Store 如何组织？

**选择**：**一个 Ledger**（记录 · 回放 · 校验）—— 三者本质都是记录与校验数据、都在异步旁路上。预算计数器**不**挂在 Ledger 上：**预算计数器归 Scheduler**（它是花钱的那个，CostGate 每次决策都要同步读），**成本归属分类归 Ledger**（分类天然发生在 resolve 之后，由 Reconciler 发分类事件、Ledger 存档）。

---

## D-022：每个 SpecTask 都要独占隔离环境吗？

**问题**：Task-level 与 Instance-level 投机需要同样的隔离设施吗？

**选择**：不需要。**Task-level（提前执行边上的一次调用）不需要实例** —— PASTE、SPORK、Spec Tool Calls、SPAgent、Speculate with Memory（agent 进程始终只有一个）；**Instance-level（提前走 agent 的下一步/下一段轨迹）需要** —— DSP、ISP、Skim、以及所有嵌套。把一切都塞进 agent fork，等于为 11 篇里的 4–5 篇付出根本不需要的代价。

**fork 保真度是谱，作为 agent 声明的能力，而非底座定死的选择**：轨迹重放到新实例（agent 是轨迹的纯函数，多数 ReAct 满足，最低、跨语言）→ 上下文 COW fork（低 —— SPORK 的 prefix-cache fork：探针预填 1.6s→0.35s、prefill 干扰 <1%）→ 进程 / VM 快照（高）。这给了多隔离后端（docker / firecracker / gVisor / QEMU）一个**架构上的必要性理由**：不同 agent 需要不同的 fork 保真度，否则多后端只是"支持得多"。

---

## D-023：虚拟执行（伪造返回值）怎么处理？

**问题**：对不可逆或非幂等的 API 直接返回成功、构造一个返回结果，行吗？

**选择**：**不行，11 篇无一这么做**。伪造值会成为分支后续行为的条件，即使 L0 全等匹配，轨迹也已建立在编造的 observation 上 —— 破坏点在验证机制看不见的地方。

**救法**：把伪造的返回值本身建模成一个 **Proposal**（生产者是 Effect Coordinator 而非 Proposer），进入 AssumptionSet 走同一套 confirm/refute —— 风险从隐藏变成可见、可计价、可被 Fidelity Monitor 拦截。

**论文的三条真实路线**：TRANSFORMABLE（dry-run/staging 拿真实结果）、STAGEABLE（缓冲效果并阻塞到 commit barrier）、不投机（硬闸）。

---

## D-024：三条边界并列吗？

**问题**：Agent Boundary / Isolation Boundary / Engine Boundary 是不是同等地位？

**选择**：不是。**Agent Boundary 与 Isolation Boundary 必需；Engine Boundary 不在范围内（D-013 修订）** —— 绝大多数 agent 通过 API 使用大模型，控制不了提供方 KV cache（SPORK 自述 "closed-source APIs cannot host the fork thread"；Speculative Tool Calls 提议的是尚不存在的新端点；PASTE 需 vLLM startup hook；SPAgent 需引擎暴露 hybrid batch 计时 —— 这 4 篇自托管专属，本系统不做这一层）。

**降级不是排除，是契约协商的正常输出**：API 后端不提供 Engine 能力时，依赖它的 policy 配置被自动标为不可用或降级，并把原因报给用户。

**API 化的两个后果**：(1) 成本问题变严重 —— 自托管靠 continuous batching 投机近乎免费，API 每次按全价计费，CostGate 从锦上添花变成必需；(2) 别把 prompt cache 一起扔掉 —— 控制不了它的粒度，但可以**塑造请求前缀去命中它**（*prefix-cache-aware* 而非 *prefix-cache-controlling*），这是 API 场景下少数还能拿到的引擎级收益。

---

## D-025：为什么需要自研 Runtime？

**问题**：通用沙箱（docker / gVisor / QEMU）不够吗？

**选择**：自研 Runtime 的存在理由不是"又一个后端"，而是**证明 Isolation Boundary 的接口设计对了** —— 通用后端能接上，专用后端能把接口用满。

**通用沙箱为投机不需要的东西付费**：双向对抗性隔离（投机实例是你自己的 agent，只需防意外干扰而非攻击）、持久化/崩溃一致性（实例短命且多数被丢弃）、完整 syscall 面、**merge/冲突消解**（投机单赢者永不 merge，无需三方 diff/CRDT）、长生命周期资源核算。

**投机需要、通用沙箱不给的**：**廉价 COW fork**（docker 不 fork，只能起新容器再重放）；**批量快速销毁**（`k` 个里拆掉 `k−1` 个，**teardown 延迟在关键路径上** —— 容器场景从来不在乎，却是投机的主要成本项）；**假设感知的效果暂存**（EffectClass × StateDomain 矩阵，通用沙箱不知道 commit barrier 为何物）；**确认时的增量状态提升**（赢家状态廉价地变成权威）。

**嵌套的要求**：「实例间无协作」在嵌套下不完全成立 —— depth-2 建在 depth-1 状态之上（父子状态依赖），所以必须支持**链式 fork（fork 的 fork）**，并在 depth-1 假设确认时把 depth-2 **重新挂载到权威之上**。

---

## D-026：EffectClass 一维够吗？

**问题**：EffectClass 分类够不够支撑隔离机制的设计？

**选择**：不够。**EffectClass × StateDomain 是二维的** —— EffectClass 描述纪律，StateDomain 描述状态域，二者正交，交叉出的每个格子对应一种具体隔离机制：

| StateDomain | 典型可用纪律 | 隔离机制 |
|---|---|---|
| filesystem | STAGEABLE | overlayfs / union mount |
| process, memory | STAGEABLE | namespace / COW fork / VM 快照 |
| GUI | TRANSFORMABLE | 影子渲染 / 离屏合成 |
| network (read) | PURE | 直通 |
| network (write) | TRANSFORMABLE / STAGEABLE | 录制回放 / 代理拦截 |
| database | STAGEABLE | 未提交事务 / savepoint |
| email, payment | COMPENSABLE / IRREVERSIBLE | outbox 模式延迟投递 / 硬闸 |

**这个矩阵定义 Effect Coordinator 的控制面协调空间，以及 Runtime Mechanism Effect backend 必须实现的机制空间。**

---

## D-027：Fidelity 是单向声明还是双向协商？

**问题**：算法声明契约、底座校验 —— 契约的方向是什么？

**选择**：**Speculation Contract，双向协商**：上层 → 下层（预测置信度、验证条件、失败代价）；下层 → 上层（隔离能力、隔离代价、可用纪律集合）。

**后果**：单向声明下底座只能**拒绝**不可行组合；双向协商下能回答"给定这个隔离后端，该算法最多能激进到什么程度"、"要支持该算法，隔离层至少要提供什么"。静态约束求解是它的下层形态。

**原因**：现有工作缺口就在这 —— 一类只管预测与提前执行（靠只读/幂等/沙箱绕开副作用），另一类只管系统层状态隔离、分支、事务与回滚，不建模上层的置信度、验证条件与失败代价。**两条路径之间没有契约**，9/11 篇用只读白名单绕开是系统性缺口的直接证据。

---

## D-028：无侵入底座要在哪些接缝做 shim？

**问题**：接入面最少要几个接缝？

**选择**：**四个**（把 11 篇的接入需求归纳后只剩这些）：

| # | 接缝 | 需要的能力 |
|---|---|---|
| 1 | **LLM 调用边界** | 流式、按 token 中断、logprobs、并行 dispatch、按 index 取消；prefix cache 共享属 engine plane（D-013 修订后不在范围内），只能通过塑造请求前缀间接利用 |
| 2 | **工具分发边界** | 统一 dispatch、优先级类、预算、抢占、promote 到 non-preemptible、沙箱、dry-run/staging 变体 |
| 3 | **轨迹/上下文写入边界** | truncate-and-rewrite（`S[:j] + new`）、prompt 变更、commit boundary 强制 |
| 4 | **调度/准入边界** | 引擎压力可见、准入重排、优先级覆盖 FCFS |

**实测的侵入量**（支持"无侵入可行"命题）：PASTE —— agent 侧 ~5k 行 TS + LLM 侧 ~2k 行 Python，挂在**未修改的 vLLM** 上，工具 API 完全不改；SPORK —— 标准 completion API 上的 thin controller，无重训练、无辅助模型；Skim —— 对底层 agent 只有一个硬性要求（能从任意 URL warm-start），不需要注入推理历史；Speculative Tool Calls —— 提议新端点 `POST /cache-tool-output/{response_id}`。
