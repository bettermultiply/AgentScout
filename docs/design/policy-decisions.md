# Policy Decisions

Policy Decision 是 Policy 根据任务、执行上下文、可用信息、资源条件和 Runtime 能力作出的可替换决策。Policy Decisions 定义策略选择空间；它们不是 Runtime 状态，也不是可以绕过 AScout 固定语义的自由执行脚本。

## 2. 正交轴与取值域

这是文档的核心。每一轴独立，论文的差异全部落在轴上的取值。

### 单次投机执行的决策面

正交轴是从论文中归纳出的分析维度，不直接规定 Policy 的调用结构。对一条投机边 `E`，Policy 实际面对的是一次投机执行的决策面：

```
S(E) = ( admit, extent, predict, launch,
         authority, effect, match, dispose )
```

这些 decision 不是一次必须按固定顺序执行的流程，而是作用于同一个 Edge 的可组合选择。它们的粗粒度含义如下：

| decision | 语义 | 主要作用阶段 |
|---|---|---|
| `admit` | 决定是否开启一次投机执行；输入包括 Edge、当前执行上下文、资源/预算条件、预期收益和实例存活概率 | 准入判断 |
| `extent` | 决定一个已经开启的投机执行在其内部继续推进的边界，以及何时主动停止并等待裁决 | 过程执行 |
| `predict` | 决定预测对象、Proposal 的生成方式与数量；Proposal 数量就是 breadth | 预测生成 |
| `launch` | 决定权威路径是否启动以及启动时机 | 准入与权威路径启动 |
| `authority` | 决定验证依据的来源及其获取方式；来源可以是 `A_u`、用户/环境或 validator | 权威获取与验证 |
| `effect` | 决定投机执行过程中各个 step 产生的外部效果如何纳入管理，以及采用暂存、代理、阻塞、补偿或其他合法纪律 | 过程执行及处置 |
| `match` | 决定 Proposal 与验证依据之间的兼容或可接受关系 | 验证 |
| `dispose` | 决定验证、取消、抢占或停止后的投机工作如何复用、提升、回收、重执行或清理 | 处置与提交 |

其中 `admit` 与 `extent` 必须区分：`admit` 决定是否创建一个投机执行过程；`extent` 只约束已经创建的过程在内部向前推进多远。`extent` 对所有投机执行都适用，不只适用于 PURE 效果。它可以由 step 数、收益阈值、停止条件或即将到达的处理边界表达；到达边界后的正常结果是暂停并等待裁决，不是自动 discard。

`extent` 也不同于嵌套深度。嵌套深度表示投机实例在执行中是否再次遇到新的 Edge 并递归调用 `admit`；`extent` 表示当前这一个投机执行过程内部已经推进了多少。前者改变投机过程的层级，后者限制单个过程的前进范围。

`effect` 包含此前由 admissibility 表达的含义，不再另设 Edge 级 admissibility 元数据。一次投机执行可以包含多个 step，并在不同 step 产生不同 StateDomain 和 EffectClass 的效果；`effect` 决定哪些变化必须纳入管理、允许采用哪些处理纪律以及相应 fallback。Effect Management 和 Runtime 再检查该选择是否有能力落实，因此可投机性是整个效果计划及其能力校验的结果，不是投机启动前的单独 enable bit。效果处理能力不足时，可以在执行到无法提前处理的效果处阻塞当前过程，不必预先把整个投机执行判死。

`admit` 是经济性和运行条件的 Policy 判断，不负责替代 `effect` 的安全管理；`effect` 也不替代 `admit`。一个过程即使已通过 `admit`，仍可能在后续 step 因效果阻塞或达到 `extent` 而暂停。

### 轴 1：权威启动策略 (Authority Launch)

**Shape 不是原语**：Overlap / Bypass / Elide / Overwrite 是"权威启动策略 × 效果释放策略"乘积上一块区域的名字。拆开后 Bypass 与 Elide 自动被边模型覆盖，且几条性质变成可推导的。

`A_u` 是否启动、何时启动：

| 取值 | 语义 | 接受判据只能用 | 论文 |
|---|---|---|---|
| **Eager 急切** | `A_u` 与 SpecTask 同时启动 | L0 / L1 / L2（有 `i` 可比） | 其余全部 9 篇 |
| **Lazy 惰性** | `A_u` 仅在 validator 拒绝后补启 | **L3**（无 `i` 可比） | **Skim** |
| **Absent 缺席** | `A_u` 永不启动 | ⊤ | **SPAgent Aggressive 阶段** |

**两条被推导出来、而非碰巧成立的性质：**

1. 不启动 `A_u` ⟹ 根本没有 `i` ⟹ 接受判据只能是一元谓词 `validate(result)` ⟹ **必须是 L3 等价**。这解释了为什么 Skim 是唯一用 validator 的 —— 不是设计偏好，是被迫。
2. Lazy 下 validator 拒绝时才补启 `A_u` ⟹ 此刻是**串行追加**而非并行 ⟹ 必然要问"投机的部分进展能否给 `A_u` 打底" ⟹ **Salvage / warm-start 是 Lazy 的必需品，不是可选优化**。

**`A_u` 可省性判据**（有预测力，可用来判断一个新场景能否做 Bypass）：

> `A_u` 可省 ⟺ (`u` 是**计算**而非**世界**) ∧ (保真度契约**不以** `A_u` 的输出定义正确性)

- **`u` 是世界 → Eager 被物理强制**：Speculate with Memory Type 2 的 `u` 是环境、Speculative Interaction Agents 的 `u` 是人。你不可能让人说得更快，最终仍必须拿到真实 observation。
- **契约以 `A_u` 定义正确 → Eager 被契约强制**：SPORK / PASTE / DSP 声明 trajectory-equivalence，而 trajectory 的定义就是"主模型会做什么"。省掉主模型等于取消正确性的定义。
- **两者都不成立**：Skim（`u` = agent 自己的导航轨迹，契约是 outcome-equivalent）、SPAgent Aggressive（`u` = 模型自己的 thought，契约是 bounded-degradation）。**这恰好是全部 11 篇里仅有的两篇非 Eager。判据自洽。**

**天花板的归因：天花板是 `depth = 1` 的推论。** Eager + depth=1 下权威工作一点没少、且权威链仍然串行，端到端延迟下界就是权威链自身的串行时间。Speculative Actions 的 breadth 上限 **50%**（`p/(1+p)·α/(α+β) ≤ 1/2`）、Speculative Tool Calls 的 client-side 上限 **2×** 都是这个下界的具体化。
- 但 Speculative Actions 的 depth 公式是 `((T−1)/T)·p·(1−b/a)`，**把上限从 1/2 抬到 1，且全程保持 trajectory-equivalent**。机制上的原因是：depth 模式下第 `t+1` 步的权威调用在第 `t` 步权威返回**之前**就发出（因为它的输入是投机的），于是**权威链本身从串行变成流水线**。

> **突破天花板有两条独立的路：Bypass（改契约，轴 7/§2.7）与 Nesting（不改契约，轴 2/§9）。后者应当优先**——它不需要牺牲保真度。详见第 9 节。

**该轴取值可以是状态依赖的函数，而非常量**：SPAgent 采样 k 个候选后用 `π_θ^e` 打 1–5 分，若全部低于阈值 β 则从 Absent 切到 Eager（调优显示 β>3 后准确率持平而延迟继续升，推荐 β=3 或 4）。这正是它属于策略轴、而非"形状"的证据。

**Overwrite 不在这条轴上**：Speculative Actions 的 OS 变体权威是 Eager 的（actor 照跑），它的特殊之处在于**投机效果在验证前就释放到了外部世界**（每秒把参数写进活内核，actor 到达时 last-write-wins 覆盖）。那是 **Effect 轴上的 release-early 取值**（第 4 节），与权威启动无关。

### 轴 2：投机拓扑 (Topology) —— breadth × depth

以 Edge 为机会单元展开一棵**投机树**。这条轴描述不同取值导致的图/树形态，而不是新增一个独立的 Policy Decision。

本轴中的 `breadth` 与 `depth` 是论文分析拓扑形态时使用的术语，不等同于决策面中的单一 decision：`breadth` 由 `predict` 决定 Proposal 数量；单次投机过程内部的前瞻范围由 `extent` 决定；嵌套时是否在新 Edge 上继续开启投机，由递归的 `admit` 决定。论文将单次过程内的链式前瞻和多个投机过程的嵌套都称为 `depth`，AScout 在决策面中将二者分开；这里保留 `depth` 是为了描述论文中的链式拓扑和全局形态。

| 维度 | 取值与论文 |
|---|---|
| **breadth 固定** | Speculative Actions Alg.1（命中率 `p(k) = 1−(1−p)^k`，因 `p(k)` 对 `k` 凹，成本增长非线性）；SPAgent 默认 `k=3`（命中率增益变平处）；Speculate with Memory 的 best-of-k |
| **breadth 动态** | Speculative Actions Thm.4：按置信度降序取前 `m`，`m* = argmax_m {q(m;p)·Δ − c·m}`，贪心加分支直到 `Δ*·δq(m;p) < c`，运行期 `O(k)`（排序+扫描），`Δ*` 离线估。实测在 chess 上用"只保留预测准确率 >50%"的常数阈值近似即可拿到最低额外 token 成本 |
| **depth = 1** | 绝大多数 |
| **depth > 1** | Speculative Actions depth-focused：分支只在投机或真实调用返回时才延伸、不一致子树立即剪枝，因此**系统最多领先 `a/b` 步，活跃分支数与视界 `T` 无关**（不会指数爆炸）；ISP/DSP 的 k 步前瞻（ISP 最大并发 `k+1`，实验 `k=4`，实测并发 4.5–5） |
| **depth 在线学习** | DSP：DistilBERT 回归器在线预测 `k`，TD(λ) 框架下 `V(s)` ≈ 该状态的期望投机步数；`λ=γ=1` 时退化为普通监督回归。用 expectile 参数 `τ` 或推理期偏移 `k = max(1, k̂+β)` 暴露延迟/成本前沿 |

**breadth 与 depth 的成本结构不同，不能混为一谈**：breadth 的收益 `p(k)` 对 `k` 凹而成本近似线性 ⟹ 存在明确最优点；depth 的收益是把 50% 天花板抬到 100%，成本项是 `(1−p)(a/2b − 1/2) + p`，刻画的是"真值返回前堆积了多少投机步"。

**与轴 1 的关系**：两条轴都作用于最终拓扑，但正交 —— **轴 1 管投机子树与权威路径的串并关系**（Eager = 并联；Lazy 被拒后 = 串联追加），**轴 2 管投机子树内部的分叉与前瞻**。

### 预测对象 (Prediction Target)：为什么不是一条轴

**归约论证。** 所谓的七种"预测对象"全部可归约：

- plan step (DSP/ISP) = action + 其执行产生的 observation
- 子轨迹 (Skim) = 若干 (action, observation) 对的复合，不是新品类
- 链式 (Speculate with Memory Type 3) = 二者的显式组合，论文自己就是这么定义的
- **未来的用户输入 (Speculative Interaction Agents) = observation，其对应 action 是"请求输入"** —— 这一条有论文的直接背书：该系统的三个动作里就有 `<pause>`，字面意思就是"停下来等用户/环境"。pause 就是那个 action，用户说的话就是 observation

再往前推一步，连 action/observation 这个二分本身也不是关于**值**的：一般 DAG 上游算子的输出（cost-aware）既不是对世界的动作、也不是来自世界的观测，但**同一个值从生产侧看是 action，从消费侧看是 observation**。这个区分只取决于**你站在边的哪一侧**，所以它是边模型的视角选择，不是一条轴。

**唯一的操作性残留**：生产者是**世界**还是**计算**。而这正是轴 1 可省性判据的第一个合取项。所以它不是独立轴，只是**轴 1 的投影**。

**内容归位，不丢东西**：Speculate with Memory 的三类型延迟节省公式仍然有用 —— Type 1 省 `min(ℓ_env, ℓ_LLM − ℓ_spec)`、Type 2 省 `min(ℓ_LLM, ℓ_env − ℓ_spec)`、Type 3 省 `ℓ_LLM + ℓ_env` —— 但它们可由"边的位置 + 两侧延迟"直接推出，应落进**成本模型**而非占据一条轴。

### 轴 3：预测器 (Predictor) —— 纯 policy，底座只定接口

| 取值 | 论文 |
|---|---|
| 小模型 / 弱 backbone | Speculative Actions、DSP、ISP、Speculate with Memory (GPT-4.1-mini/nano)、Speculative Tool Calls (xLAM 1B/3B/8B) |
| **同一模型自投机** | **SPORK**（forced tool-call prefix + prefix-cache fork，探针成本从 1.6s 降到 0.35s）、SPAgent（`π_θ^s` 无 thought 采样） |
| 离线挖掘的 pattern | **PASTE**（两遍挖掘：事件签名的控制流 + argument mapper 的隐式数据流；**晚绑定**——pattern 定"下一步是什么"，活会话供具体值） |
| 站点结构剖析 | **Skim**（offline profiler，每站一次，<16s） |
| **在线累积的记忆** | **Speculate with Memory**（contrastive transition table + episodic memory + confusion tracker；accuracy 随经验从 ~28% 涨到 >50%） |

**给底座的启示**：predictor 是最不需要底座操心的部分（纯插件），但**底座必须提供 predictor 的数据供给**。PASTE 需要历史 trace 做 pattern mining，DSP 需要 `(state, k)` 对做在线 RL，Speculate with Memory 需要 miss episodes 和完成轨迹。这三家各自造了一套数据管道。**底座提供统一的 trace / shadow / replay，就等于免费给所有 predictor 供血** —— 这是极强的无侵入卖点，见第 7 节。

### 轴 4：等价关系 (Equivalence `≈`) —— 被严重低估的一轴

判定"猜中了"有**四个本质不同的层次**，它们的可用性和保真度完全不同：

| 层次 | 判据 | 是否需要权威值 | 论文 |
|---|---|---|---|
| **L0 语法相等** | canonical form 全等 | 需要 | SPORK（name **和** serialized args 全等）；PASTE（canonicalized name+args）；Speculate with Memory（"full exact match，action type 和所有参数都必须一致"）；DSP/ISP-OpenAGI |
| **L1 语义等价** | embedding cosine ≥ 0.95 / AST 模格式相等 / semantic_json_equal / Levenshtein < 0.3 | 需要 | Cost-aware tier2；**ISP-TravelPlanner** |
| **L2 效果同余** | 对**后续**的影响相同 | 需要 | **Speculative Actions**：cache key 是 `(h, q)` call specifier，比对的**不是猜的 action 本身，而是它蕴含的下一次调用** |
| **L3 结果可接受** | validator 判定输出对不对 | **不需要** | **Skim**：schema check（零成本，无 LLM）+ semantic judge（小模型，输入是压缩后的 state summary） |

三个必须写进设计文档的结论：

1. **L2 是被低估的设计。** Speculative Actions 的洞察是：等价不必定义在"值"上，可以定义在"值对下游的影响"上。猜错了 action 但推出的下一次调用一样 —— 仍然算命中。这天然更宽松，且**仍然 lossless**（因为真正被复用的是那次调用的结果）。底座应该把 `≈` 定义为可插拔的**同余关系**，而不是相等判定。

2. **L3 是唯一能支撑 Bypass 的等价关系。** 因为它不等权威值，所以能真正短路掉权威路径。代价是它必然 lossy。Skim 明确把 verifier 调成"倾向拒绝"，让不确定性转化为 fallback 而非静默错误 —— 这个偏置方向应该是底座的默认值。

3. **放宽 `≈` 会静默打破 lossless，论文自己也承认。** ISP 明确写道：用了模糊匹配后 "it is not guaranteed that the result from speculative planning remains the same as the result from normal agent planning"，所以他们**分开报告两种模式的任务性能**。SPORK 则明确拒绝 name-only 匹配，理由是"对副作用工具会静默改变行为"。**底座必须把 `≈` 的层次和 Fidelity 契约（轴 7）绑定校验**：声明 trajectory-equivalent 的算法不允许用 L1/L3。

### 轴 5：权威 (Authority) —— 权威路径不一定跑

| 取值 | 论文 |
|---|---|
| 更强的 target agent | DSP、ISP |
| 主生成流自身 | SPORK、PASTE、SPAgent、Speculative Tool Calls |
| 真实环境返回 | Speculative Actions、Speculate with Memory |
| **人** | **ISP**：用户在 `a_i` 显示后、`t_i` 到达前的间隔里中断，注入的值被当作 oracle，"effectively a third agent"。系统只 kill 第 i 个 target process，其他并发进程照跑 |
| validator（Authority 为空时） | Skim |

**底座含义**：`resolve()` 接口必须能接受**外部注入的权威值**，不能假设权威值一定来自那条已经在跑的权威路径。ISP 的 oracle interrupt 本质上是**用人的答案替换掉了 target agent 的那一步**。这同时意味着底座要支持"取消特定 index 的权威任务"这种细粒度操作（ISP 用 `signal.SIGTSTP` + 绑定 target_task_id 实现）。

**与轴 1 的耦合关系（是约束，不是恒等）**：轴 5 管**谁**产出 `i`，轴 1 管 `A_u` **跑不跑**。耦合点只有一处 —— 可省性判据的第一个合取项：生产者是**世界**（环境 / 人）则 Eager 被物理强制。所以**轴 1 的可行域是轴 5 的函数**。

把二者当独立轴处理，会生成一个**论文里没有的设计点**，这正好是抽象质量的检验：

> `Authority = 用户输入` × `launch = Lazy` —— **先按对用户意图的猜测往下干，只在 validator 拒绝时才打扰用户。**
>
> ISP 是它的反向（用户主动中断、系统被动接受）。lazy-human 完全可实现，且在"打扰成本很高"的场景（长时间任务、异步协作、语音）价值明显。**一个好抽象应该能生成没人做过的组合，这条通过了测试。**

### 轴 6：处置 (Disposition)

初始枚举：复用、提升、提交、重执行、丢弃、补偿、合并。论文证据基本吻合，但缺了一类，而且"提升"值得单独强调：

| Disposition | 语义 | 论文 |
|---|---|---|
| **Reuse 复用** | 投机 task 已完成，结果直接转正 | PASTE、SPORK、Speculative Tool Calls |
| **Promote 提升** | 投机 task **在飞**，命中后转为权威身份、变 non-preemptible，只等剩余时间 | **PASTE 明确列为独立终态**；SPAgent 的 Action Buffer "attach to in-flight execution" |
| **Discard 丢弃** | 无匹配，过期，静默销毁 | 所有 |
| **Preempt 抢占** | 权威工作需要资源，投机 task 被回收 | PASTE（按 utility 从低到高回收）；SPAgent（等待超过 `t_w` 直接丢弃而非抢占，因为重叠价值已过期） |
| **Re-execute 重执行** | 用真值重跑 | ISP（`o'_j = execution(t_j)`，被拒 action 的 observation 不继承）；cost-aware（rollback-by-re-execution） |
| **Compensate 补偿** | 反向操作抵消已发生的副作用 | Speculative Actions 提及（refund/replace 的 roll-forward repair），**无人实现** |
| **Overwrite 覆盖** | 后写压前写，无需补偿 | Speculative Actions OS 变体 |
| **★ Salvage 残值回收** | **失败后回收部分进展** | 见下 |

**Salvage 是缺的一项，而且它可能是最有系统味的一项。**

三个独立的实例，在三个不同的层次：

- **Skim 的 warm-start**：fast path 被拒后，**它最后到达的 URL 交给完整 agent**。"escalation never begins worse off than where the cheap attempt landed"。被拒的答案和推理链全部丢弃，**只有 URL 过界**。
- **SPORK 的 partial-token accept**：被拒的 probe token 注入成 per-request draft，在同一个 decode step 里被验证 —— "no separate verification pass, no second prompt"。被拒的投机变成了 speculative decoding 的草稿。
- **PASTE 的 promote**：严格说是命中路径，但机制是同一个 —— 复用在飞任务的部分进度。

三者的共性：**投机工作往往有"公共前缀"结构**（导航前缀 / token 前缀 / 执行进度）。**底座应该把"部分进展的表示与移交"做成一等抽象**，而不是让每个算法自己发明。这可能是整个设计里最值得写论文的点：

```
interface Salvageable<P> {
    progress(): P                      // 提取当前部分进展
    handoff(p: P, to: AuthorityPath)   // 移交给权威路径
}
```

### 轴 7：保真度契约 (Fidelity) —— lossless 不是布尔值；且它不是轴，是导出不变量

**为什么不与 Equivalence 合并（二者可独立变化）**：

- 固定 L0 变 Fidelity：SPORK 是 L0 + trajectory-equivalent；Speculative Interaction Agents 也用精确的 ID 键匹配，却只有 bounded-degradation —— 它掉保真度的通道**根本不是匹配谓词**，而是"模型在不完整用户输入上已经动过手" + SFT 模型的退化行为。
- 固定 trajectory-equivalent 变 Equivalence：SPORK 是 L0，Speculative Actions 是 L2。

**但为什么它也不是一条轴**：Equivalence 是**机制层旋钮**，Fidelity 是**系统层承诺**。保真度可以从至少四条互不相同的通道丢失 —— 放松匹配（ISP 软匹配）、提前释放效果（OS 变体**根本没有匹配谓词**却破坏了轨迹等价）、不启动权威（SPAgent Aggressive）、模型协议退化（Spec Interaction）。**Fidelity 是这些轴上的一个合取，不是与它们并列的一维。**

**正确处理（这是 mechanism）**：

1. **静态**：由各轴设置**推出保真度上界**（不可能优于最弱的一轴所允许的）。
2. **加载期**：把开发者的**声明**与推导出的上界对撞，不一致则拒绝加载。声明仍然需要 —— 因为存在底座看不见的通道（如模型退化）。
3. **运行期**：由 shadow mode **实测验证**声明是否真的达成。静态只能证上界，证不了达成。

---

所有论文都在说 lossless，但说的**不是一件事**：

| 契约等级 | 保证什么 | 论文 |
|---|---|---|
| **Trajectory-equivalent** | 最终轨迹逐步等同于串行执行 | Speculative Actions（明确指出**只对 trajectory 成立，cost 不成立**）；DSP（"final execution path always conforms to T's policy"）；Speculate with Memory（"actor sees identical context regardless of prediction outcomes"）；PASTE（"a wrong prediction can waste bounded work, but it cannot expose an unconfirmed result to the agent"） |
| **Outcome-equivalent（统计）** | 端到端指标不劣化，单次可能不同 | Skim（三个 backbone 上 accuracy 持平甚至略升，但 verifier precision 只有 82%，约 1/5 被接受的 fast path 可能是错的；论文**没有把静默错误率和 cascade 率分开报**） |
| **Bounded-degradation** | 明确接受有损，但有界 | SPAgent Aggressive 阶段（论据：早期 action 是 evidence gathering，猜错"often still task-relevant"）；Speculative Interaction Agents（"minor accuracy loss"，HotpotQA 71.6→71.0） |
| **Best-effort** | 不保证 | Speculative Actions OS 变体（中间态被 workload 真实经历，trajectory 等价被真实打破）；ISP 的软匹配模式 |

**这是底座最大的差异化机会。** 现状是：每个算法在自己论文里论证自己 lossless，读者无法验证，也无法在生产里持续验证。底座应该让 **Fidelity 成为可声明、可强制、可审计的属性**：

- **可声明**：算法在注册时声明契约等级。
- **可强制**：底座校验轴间约束（声明 trajectory-equivalent 却用 L1 等价 → 拒绝加载；使用 Runtime 无法落实的效果处理纪律 → 拒绝加载或明确降级）。`IRREVERSIBLE` 效果是否可以在效果边界阻塞、等待 Branch 裁决，不由 Fidelity 声明单独决定，而由 Effect Management 与 Runtime capability 共同校验。
- **可审计**：shadow mode（投机与串行同跑，只提交串行结果）是**验证 losslessness 的唯一手段**，也顺带是 predictor 的数据管道。cost-aware 论文把它列进五阶段校准的第 2 阶段，但那是它的方法的一部分；**在底座里它应该是基础设施**。

---

## 阶段映射

一次投机执行可以按以下阶段理解。表中的决策是当前最重要的代表性内容，不是完整清单。

| 阶段 | 代表性 Policy Decisions |
|---|---|
| 机会发现 | 识别并规范化潜在的投机 Edge |
| 准入判断 | `admit`：是否开启一次投机执行 |
| 预测生成 | `predict`：预测对象、Proposal 生成方式与数量 |
| 过程执行 | `extent`：单次过程内部的推进边界；`effect`：过程内效果处理纪律 |
| 权威获取与验证 | `launch`、`authority`、`match`：权威路径、验证来源和兼容性判定 |
| 处置与提交 | `dispose`：验证、停止、取消或抢占后的处理 |

机会发现是 Policy 与 Mechanism 的接缝：接入侧识别并规范化 operation 及其 pending-result boundary，Policy 再对潜在 Edge 作出决策。Edge 不构成独立的注册实体；真正的投机注册是创建携带 Proposal 的 candidate Branch。效果纪律、资源条件和 Speculation Contract 限制各项决策的可行范围。

## 语义对象

Policy Decisions 作用于一组核心语义对象：

- **Proposal**：投机路径使用的预测值。
- **Authority**：提供真实输入、结果或外部权威判断的可选来源；Policy 可以选择或等待其来源，但实际执行可能由 Runtime 提供。Authority 为空时，Proposal 仍可由独立 validator 按可接受性条件验证。
- **Equivalence**：判断 Proposal 与验证依据是否兼容或可接受的关系。
- **Disposition**：验证后对投机工作采取的处置意图；实际清理和状态转换由 Mechanism 强制完成。

这些对象属于 AScout 的执行语义，不等同于 Policy 组件，也不表示其状态全部由 Policy 持有。

## Policy 与 Mechanism 的边界

Policy 可以组合和实现八类 decision，但不能拥有 Runtime mutable state，也不能把策略意图伪造成已经完成的 Runtime 操作。以下内容由 Mechanism 或 Runtime 强制：

- candidate Branch 的创建、隔离、暂停、恢复、接受、丢弃和清理；
- `AssumptionSet` 的传播、验证后的确认/否定和非空假设数据的信息流约束；
- single-winner、共享验证依据、commit barrier 与效果阻塞；
- 资源所有权、预算执行、抢占、取消和运行期成本记账；
- Policy 声明与 Runtime 能力之间的 Contract 校验，以及 Fidelity 的加载期检查和运行期审计。

Policy 的 `effect` decision 可以选择效果处理纪律，但不能排除 AScout 为正确性规定的最低纳管范围；Effect Management 和 Runtime 负责将该选择落实为暂存、转换、阻塞、提交、取消、清理或补偿。

## 可扩展性

Policy Decision 集合不固定为八项。后续可以加入 resolver delegation、等待与恢复、失败 fallback、Runtime Service 选择或其他决策。新增决策必须说明所属阶段、输入输出和与既有决策的依赖，并遵守 AScout 的 Edge、Branch、single-winner、隔离、capability、obligation、commit/discard/accept 和 cleanup 语义。

投机的收益取决于等待时长、预测命中率与提前工作量的相对大小，这决定了准入与调度必须是机制的一部分，而不是各算法各自处理。