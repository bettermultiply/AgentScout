# 投机执行论文证据札记

本文保存投机执行相关论文的事实、实验结果和模型适用性审查。审查中的缺口判断与调整建议不是已批准的设计决策；已经收敛的目标语义以 `docs/design/` 为准，系统结构以 [`architecture.md`](../architecture.md) 为准。本文不重新定义 Edge、Branch、AssumptionSet、Policy Decision、Effect、控制面或 Runtime Adapter。

## 1. 论文范围

第 1–6 节是早期 11 篇工作的证据整理：Speculative Actions、PASTE、Skim、ISP、DSP、SPAgent、Speculative Interaction Agents、SPORK、Speculative Tool Calls、Speculate with Memory 和 Cost-aware Speculative Execution。完整收录范围见 [survey](../survey/Speculative-sur.md)，新增工作的适用性审查见第 8 节。

论文涉及的投机对象包括工具调用、命令、计划步骤、模型输出、整条轨迹、未来用户输入以及由这些对象组成的连续过程。现有工作的主要形态如下：

| 工作 | 论文中的投机形态 |
|---|---|
| Speculative Actions | 预测下一次调用或连续多步调用 |
| PASTE | 多候选工具调用，结合 pattern mining 与晚绑定 |
| Skim | 从候选 URL warm-start 独立快路径 |
| ISP / DSP | 在 agent 轨迹中提前执行后续 plan step；ISP 使用 `k=4`，DSP 在线学习 `k` |
| SPAgent | 预测 action；Verified 版本由主路径验证，Aggressive 版本不等待权威路径 |
| Speculative Interaction Agents | 并行推进依赖未来用户输入的交互路径 |
| SPORK | self-fork 预测 tool call；D3 进一步接入 partial-token draft |
| Speculative Tool Calls | 预测 tool call，并在 KV cache 层拼接参数与结果 |
| Speculate with Memory | 基于在线记忆预测 observation、action 及链式后续 |
| Cost-aware | 在 DAG 上预测上游输出，并按成本与成功概率控制投机 |

这些工作通常把预测、验证、执行和失败处理绑定在各自算法中，尚未形成跨论文通用的组合语义。这是研究动机，不是本文要重新规定的系统语义。

## 2. 效果处理证据

11 篇中有 9 篇主要只投机只读工具或只读 action：

| 论文 | 论文中的效果处理 |
|---|---|
| SPORK | 通过 manifest 标记 read-only；写操作或非幂等工具串行执行 |
| Speculative Tool Calls | 限于 stateless、cheap tools；有状态工具需要 undo/rollback，但论文明确置于范围外 |
| Speculate with Memory | 从 API schema 派生 read-only action 白名单，命中白名单后才启动 |
| Skim | 排除购买、表单和账户修改；state-mutating task 绕过投机 |
| SPAgent | 使用只读 Wikipedia 搜索 |
| Speculative Interaction Agents | 手工标注 safe/unsafe；unsafe action 缓冲到 commit point |
| PASTE | 将 safe action 分级，并审计、阻止潜在副作用动作 |
| Speculative Actions | 把可逆性作为前置条件，提及 fork/snapshot/roll-forward repair，但未实现 |
| Cost-aware | 使用 side-effect-free、idempotent-under-natural-key、stageable-behind-commit-barrier 三类 admissibility；不满足时关闭投机 |

PASTE 报告在 workload 中标记并阻止 602 个潜在副作用动作，且最终任务结果没有偏离 baseline。总体上，现有论文的共同做法是白名单、硬闸、暂存或只读限制；对可写外部效果的统一处理仍是空缺。

论文中出现的三条实际路线是：

- `TRANSFORMABLE`：以 dry-run 或 staging 获得真实结果，但不执行原始变异（PASTE）。
- `STAGEABLE`：不提前释放效果，缓冲到 commit point（Speculative Interaction Agents、Cost-aware）。
- 不投机：通过 manifest 或白名单直接绕过高风险操作（SPORK、Skim、Speculative Tool Calls、Speculate with Memory）。

论文没有采用“对不可逆 API 直接伪造成功并把返回值当真实结果”的方案。原因是伪造 observation 会成为后续行为条件，即使工具调用的表面匹配成立，后续轨迹也可能已建立在不可验证的值上。

## 3. 资源与负结果

论文直接给出的负结果包括：

| 来源 | 结果 |
|---|---|
| SPAgent | 朴素投机在 `>2 rps` 时比不投机慢 49.3%，推理延迟最高增加 26% |
| PASTE | Tool-Only 消融为 419s，慢于 LLM-Only 的 342s 和完整 PASTE 的 270s；原因是工具加速使 session 更频繁、更集中地返回 GPU |
| PASTE | 并发从 1 增至 192 时，LLM 生成时间增加 17 倍，工具时间近似不变；单独把工具加速 2 倍的收益在高负载下被吸收 |
| SPAgent | 并发升高时每步 decode 约慢 20%；prefill 近线性增加，hybrid batch 成本高于纯 decode batch |
| Cost-aware | elastic capacity 在限流、预留机队和高利用率自建端点下不成立；`(1-P)·C_spec` 没有覆盖投机挤占在线请求的机会成本 |
| Speculative Actions | breadth 模式的 latency reduction 理论上限为 50% |
| Speculative Tool Calls | client-side speedup 严格小于 2 倍 |

这些结果只支持“收益依赖负载，且高负载下可能为负”这一研究观察，不指定某种固定调度或准入策略。

不同论文的成本口径也不一致：Speculative Actions 无条件按去重分支数 `k̃` 计费；Cost-aware 主要按失败概率计 `(1-P)·C_spec`，并在流式取消时按已生成 token 计 fractional waste；SPORK 用 per-turn 接受率 `α` 计算 break-even；DSP 区分 total cost 与 unnecessary cost；ISP 报告 approximation/target process 的重复执行次数。论文同时涉及 token、GPU、并发槽和外部 API 配额，不能只用 token 成本横向比较。

## 4. Shadow 与 replay 证据

现有论文提供了几类可复用的方法学事实：

| 用途 | 论文事实 |
|---|---|
| predictor 数据 | PASTE 从历史 trace 挖掘 pattern；DSP 使用 `(state, k)` 数据在线学习；Speculate with Memory 使用完成轨迹和 miss episodes |
| 确定性评测 | Speculate with Memory 采集一次 actor 轨迹后 replay，以消除 actor 方差并把 API 成本隔离给 speculator |
| 成本校准 | 命中率 `p`、接受率 `α`、取消前生成比例 `ρ` 需要从实际影子流量测量 |
| 上线过程 | Cost-aware 给出 offline replay、shadow、canary、online calibration 和 drift kill-switch 的阶段路径 |

这些事实说明 shadow/replay 同时可用于保真度检查、预测器数据、成本校准和上线观察。它们是验证证据与方法学依据，不是实时决策流程的另一套语义来源。

## 5. 接入与隔离证据

论文归纳出的观察接缝有四类：LLM 调用、工具分发、轨迹或上下文写入、调度或准入。它们是调研得到的候选观察点，不是每个 Agent 都必须提供的固定接口。

论文报告的接入事实包括：

- PASTE 约有 5k 行 TypeScript agent-side 代码和 2k 行 Python LLM-side 代码，使用未修改的 vLLM startup hook；三个 agent 只需小型 dispatch wrapper，工具 API 不变。
- SPORK 使用标准 completion API 之上的 thin controller，不需要重训练、辅助模型或离线 trace。
- Skim 对底层 agent 的硬性要求是能够从任意 URL warm-start；它不需要注入推理历史，且接入了三种观察模态不同的 backbone。
- Speculative Tool Calls 提议 `POST /cache-tool-output/{response_id}`，按 tool name 与 canonicalized parameters 匹配缓存结果。
- PASTE、SPORK、Speculative Tool Calls 和 SPAgent 的部分核心机制依赖自托管推理引擎；纯 API 调用无法访问其 KV cache、draft proposer 或 hybrid-batch 调度信息。
- SPORK 明确指出 closed-source API 无法承载 fork thread；Speculative Tool Calls 也把缓存端点作为尚不存在、需要 provider 采纳的接口提出。

因此，论文证据支持区分 Agent 接入边界、执行环境隔离边界和推理引擎内部边界。当前项目范围排除 engine plane；这一范围决定及其现行语义见 [`speculation-decisions.md`](../design/speculation-decisions.md) D-013，不在本文重复展开。

## 6. 嵌套投机的已知事实

已有工作证明了单链嵌套，但没有实现并验证 breadth 与 depth 同时扩展的深层树：

- ISP 和 DSP 的后续步骤可以建立在此前尚未验证的步骤上，形成单链；ISP 的最大并发为 `k+1`。
- Speculative Actions 的 depth-focused 变体支持连续 spawn，分支在投机或真实调用返回后延伸，不一致子树立即剪枝；论文把深层树形扩展列为未实现方向。
- PASTE 的 top-1 命中率为 27.8%，多候选整体命中率为 93.8%。若把 93.8% 作为独立近似，depth 3 的存活率约为 `0.938³ ≈ 0.83`；只用 top-1 时约为 `0.278³ ≈ 0.02`。这只是待验证的收益假设。
- SPORK 的工具名命中率为 74.6–99.6%；Speculate with Memory 报告的命中率为 17–40%，且随经验增长。

这些结果支持继续研究嵌套收益、错误传播和资源边界，但不能推出默认的 breadth、depth、树形或概率模型。嵌套的目标语义见 [`design-model.md`](../design-model.md)、[`policy-decisions.md`](../design/policy-decisions.md) 和 [`branch-semantics.md`](../design/branch-semantics.md)。

## 7. 待验证问题

- 不同投机对象是否能在同一套机制下保持可接受的结果保真度。
- 在不同负载、API 限流和资源预算下，投机收益何时转为负值。
- shadow/replay 测得的保真度、命中率、取消浪费和外部效果泄漏是否与论文声明一致。
- 深层嵌套中，错误传播、staged effect 的提交顺序和部分进展回收是否保持正确。
- 不同 Agent 的最小接入接缝和所需隔离保真度是否可由现有 Adapter 契约表达。

## 8. 当前模型的算法适用性审查（2026-10-22）

### 8.1 范围、方法与结论边界

本次审查 survey「投机执行算法」中的全部 20 篇论文（ISP、DSP 分别计），另检查 TomasuLLM 的算法／运行时交叉部分及 NVIDIA / LangChain 的路由投机。续查直接读取这 21 篇论文的 arXiv HTML 正文，按预测对象、提前执行的工作、验证依据、采用内容、失败后去向和所需能力核对；原文位置索引见 §8.8。NVIDIA / LangChain 仍以 survey 所链官方公告和代码说明为依据，本轮未重新审查其代码。基础设施、普通异步执行和社区原型不混入算法覆盖率。

基准是 mental-model → design-model → Policy、Branch、效果、接入侧和 Contract 的当前目标设计，而非 Rust 原型完成度。未执行外部实现、未编写或运行测试，也未复现论文的延迟或正确性结果；正文核对不能替代运行验证。

审查区分四种情况：现有语义基本能够表达；已有意图但生命周期、事实供给或落实规则未闭合；正式条款相互冲突；能力明确处于当前范围之外。“可以在 match 中写一个函数”不证明机制能够提供该函数需要的事实或安全落实其结果；“有 Reuse/Salvage 意图”也不证明任意状态能够跨 Branch 移交。

总体判断：Edge／Proposal／验证／处置框架能承载这些算法的大部分局部决策，但不能据此声称完整覆盖。最普遍的问题是结果 Reuse 与整 Branch Accept 被混为一谈，术语表的映射和接入侧的结果返回路径之间已有明确冲突；多步算法另有带未决前缀的验证、前缀接受和后缀清理缺口。DynBranch 跨 task 复用、引擎内部优化和投机 UI 是需要明确范围的能力，不能冒充现有覆盖。没有证据要求立即推翻树形结构、single-winner 或效果屏障。

### 8.2 新增七算法逐项核对

| 算法 | 论文所需执行过程 | 当前模型的对应表达 | 判断 |
|---|---|---|---|
| [Speculate with Memory](https://arxiv.org/abs/2607.12236) | action 预测提前跑只读工具；observation 预测提前启动 actor；连续预测组合两者；每个已完成任务更新记忆 | `predict` 允许历史、模型、外部来源及 action/observation；`effect` 限制只读；`match` 精确匹配；嵌套表达连续依赖 | observation 路线基本覆盖；action 路线已有 Reuse 入口，但 Reuse／accept 映射需统一（§8.9 第 1 项）。跨任务记忆可留在外部预测器，不必进底座；其保存、反馈与版本契约仍未闭合 |
| [Speculate While You Reason](https://arxiv.org/abs/2607.25816) | 同模型双模式预测下一调用，主模式独立产出真实调用，名称和参数匹配后复用；训练交替优化两模式 | 自预测可作为 `predict` 实现，真实调用来自 `launch` 路径，精确 `match` 和结果 `Reuse` 有对应意图 | 自预测与结果复用已有入口，但 Reuse／accept 映射需统一（§8.9 第 1 项）；共享 KV 的性能能力及联合 RL 不是现有底座自动提供的能力 |
| [AOSpec](https://arxiv.org/abs/2608.00881) | 隔离运行当前或远期动作；真实动作到来后同时核对动作和执行前环境版本；真实 observation 精确匹配后保留 actor 生成 | 当前动作可对应 action Edge，observation 对应下一 Edge；`match_context` 原则上可带环境事实，`effect` 可隔离写入 | 单步组合和 observation 路线可表达；动作加执行前版本需 Contract 提供事实（§8.9 第 3 项）；远期 fork 跨中间裁决存活与 §4.5 的 sibling 失效规则冲突（§8.9 第 2 项） |
| [DynBranch](https://arxiv.org/abs/2609.31047) | 路由未定时运行候选子图；选择和精确、新鲜需求共同授权消费；确认结果可跨请求复用；独立 offer 可继续保留 | `predict/select/effect/match/dispose` 可表达单 task 内的互斥分支提前执行；原调用跳过与受控结果交付有接入点 | 单 task 内互斥候选可表达；跨 task 缓存和 dormant offer 不在现有 Tree 生命周期（§8.9 第 6 项）；独立 offer 与 §4.5 的 sibling 失效冲突（§8.9 第 2 项）；读集合新鲜度缺 Contract（§8.9 第 3 项） |
| [LEAP](https://arxiv.org/abs/2610.02670) | 小模型在环境副本上生成多步；target 在各假设前缀上并发生成；按前缀顺序接受，并采用首个纠正动作或 bonus；接受环境不重跑工具 | 每步 action 可为 Proposal；嵌套 AssumptionSet 可表示前缀；每条 Edge 的 `launch` 在所属候选上下文取得 target 结果；`match` 精确比较 | 逐步 Edge 加嵌套可表达前缀；target 作为 Authority 提供每步真值；接受环境不重跑工具可由 candidate 状态采用表达。剩余是批量验证拆分（§8.9 第 4 项）、每步状态保留和并发 target 调用能力；不需要另一套轨迹树 |
| [ObsSpec](https://arxiv.org/abs/2610.15246) | 真实动作执行中预测 observation；提前生成后续动作；匹配前不执行下一真实动作；错误则丢弃相关生成 | 工具返回值为 Proposal，真实工具为权威来源，`extent/effect` 限制在生成边界，精确 `match` 后 Reuse/Promote，错误依赖级联清理 | 预测、匹配和剪枝最贴合；真实工具是 Authority，预测 observation 是 Proposal，后续生成是 candidate；采用生成进展时必须保留真实工具完成后的环境。只需明确依赖和清理；RL 训练、token mask 和模型更新由外部训练系统提供 |
| [SpecAgent](https://arxiv.org/abs/2610.21463) | weak 执行真实读、停在首个写；strong 一次审核返回最长可接受前缀及修正；纯读 K 步无需审核即提交 | 结构化 Proposal、`extent`、写前 `effect=block`、`launch=false` 与 validator 可表达大部分决策；Fidelity 允许弱于逐步轨迹等价 | 真实读可作 PURE 共享环境执行，首个写用 `effect=block` 停住；validator 可做前缀审核；纯读免审是明确的接受规则。剩余是批量证据拆分（§8.9 第 4 项）、修正动作归属和 weak／strong 执行配置（§8.9 第 5 项），不是任意写后回滚问题 |

补充论文事实，避免兼容性判断建立在过强假设上：

- Memory §2.2 的三种记忆在每个已完成任务后更新；不能把它当作可随意共享的候选 Agent context。其只读白名单和缓存复用不证明任意外部状态读取都具有串行等价性。
- Speculate While You Reason 的评估包含在 SGLang 中串行切换预测与主模型模式；训练目标和 Hit@1 改善不证明已经实现一个完整并发效果运行时。
- AOSpec §4.3 的状态验证是 `(工具和参数相等) AND (执行前环境版本相等)`；Runtime Support 将版本落实为 CoW 文件系统 root hash，并明确排除非文件系统输入和效果。不能称其验证了任意进程、网络、数据库状态。
- DynBranch §3.2–3.3 要求完整渲染输入、stage/model/execution 配置和外部读版本一致，并要求可投机 stage 满足相应确定性；采样或外部变异 stage 不按该规则提前执行。互斥候选的 loser 会清理，独立 offers 则可保留，不能统称“所有 sibling 都保留”。
- LEAP §3.1 和 Appendix M 明确区分 target authorship 与串行轨迹一致：并发 batch 会改变数值计算与后续决策，target 确认不等于 trajectory-equivalent。已确认前缀后的纠正动作不是允许原错误后缀继续运行。
- ObsSpec §2.2 明确真实动作顺序执行，逐次确认预测后才执行下一动作；其 rollback 主要指丢弃模型生成，不是撤销已经发生的工具写入。
- SpecAgent §3.2 的审核输出是 `(j, reason, replacement)`，不是单个整段布尔值；纯读 K 步直接接受属于算法本身，而非仅 README 差异。

### 8.3 优先讨论的三个模型问题

#### 1. 复用一个执行结果，是否必须替换整个 Agent 延续？

**涉及算法**：Memory action 路线、Speculate While You Reason、AOSpec、PASTE、SPORK、Speculative Tool Calls、Speculative Actions，进一步涉及 ObsSpec 的生成进展采用和 TomasuLLM 的工具结果／状态采用。

典型过程是：主 Agent 正在推理，预测路径提前执行工具；主 Agent 稍后给出相同调用，只需要接收提前执行的结果，并保留自己的推理和上下文。若提前执行产生受管状态变化，还需要确认和采用对应状态，而不是采用预测器自己的推理轨迹。

现有条款并非完全没有入口：`policy-decisions.md` §2.7 定义 Reuse，`agent-integration.md` §2.1 允许跳过原 operation 并沿原调用返回结果，`effect-management.md` §1.3 允许对 Outcome 增加复用约束。但 `glossary.md` 的层间映射表将 `Reuse / Promote` 一并映射为 `accept`，accept 定义再次将二者等同；`branch-semantics.md` §4.2、§4.5 则规定 Accept 为 single-winner replacement，并明确不传播 parent 分叉后的任意 Agent context。因此这里不只是少了一个实现：按现有 accept 解释 Reuse，会与“保留原调用方并返回结果”的路径冲突。需要显式澄清 Reuse 是否必须经过整 Branch Accept，以及它如何关闭候选的生命周期。

**反例**：主路径从 H 推理得到 `(reasoning R, call A)`；候选只从 H 提前执行 A。整 Branch 替换可能丢掉 R，或把预测路径的上下文当成主路径；只返回结果则必须解释候选的假设何时解除、结果属于谁、在飞调用如何移交，以及候选资源如何收敛。任一解释都不能只靠 Policy 自律。

**建议，尚未批准**：先补齐已有 Reuse 的正式语义，明确其是否允许主执行保留身份、经受控通道只采用已授权的结果。与 Promote/整 Branch replacement 区分，而不立即引入新实体或新 decision。纯结果复用和带状态变更的复用分开要求能力；不能从“返回值匹配”推出“副作用可以转移”。保留 single-winner 对互斥执行延续的约束，不允许多个不兼容世界同时生效。

**受影响文档**：mental-model §2–3、design-model §2.1/§3.1、Policy dispose、Branch resolution/accept、接入侧返回路径与效果约束。Runtime 环境接口是否要变，须在确定具体复用语义后再判断，不先增加私有操作。

#### 2. 如何验证并采用一段轨迹的前缀，而不误用后面的结果？

**涉及算法**：LEAP、SpecAgent、ISP、DSP、SMC；连续 observation 投机也依赖同一顺序原则。

这里有两种不同的事实：

- LEAP、ISP、DSP 在假设前缀上调用 target；该 target 对当前局部动作的结果是真实模型执行，但仍以祖先 Proposal 为前提。
- SpecAgent 一次审核返回前缀长度和修正，SMC 则以锚点确认加模式检查接受后续动作；两者不是逐步强模型精确生成。

`policy-decisions.md` §2.6 允许比较 Proposal 输出及行为影响，也允许 validator；§4.2 已有前缀假设确认后后代保留的机制，因此不能断言“树结构不支持前缀”。但 `MatchResult` 只有 verdict 和 score，没有明确的前缀裁决如何拆成多个受控决定；验证依据本身的 AssumptionSet、可见性和失效规则也未单独规定。`branch-semantics.md` §4.4 主要描述 Proposal 与实际 outcome 兼容，和无 Authority 的 validator 路线尚未完全对齐。

**反例**：p1→p2→p3，各步 target 并发给出 t1、t2、t3。t2 是在 p1 上算出的，不可在 p1 被否定后拿到真实路径上继续使用；若 p1 接受而 p2 拒绝，则应保留 p1 对应的状态，并用此时有效的 t2 纠正，不能留下包含 p2/p3 错误修改的最新环境。单个整块 compatible/incompatible 不足以交代这些转换。

**建议，尚未批准**：优先尝试用现有逐步 Edge 和嵌套依赖表达前缀，不直接增加另一套轨迹树。明确验证依据也受前缀依赖约束；只有相应前提清空后，验证结果才能用于权威路径。整段审核若要一次生成多个决定，应明确它们的范围和顺序，并由机制逐项落实。若采用 Salvage 保留前缀，必须说明依赖解除和状态移交的依据，不能以“残值回收”名义绕过错误前提清理。

接受粒度与执行状态保留粒度也必须一致：可通过分步执行延续、受控重放或 backend 支持达成；只保留整段最后一个 VM 状态并不能自动回到任意前缀。效果仍受原有提交屏障约束。先核定语义，再决定是否需要扩展 MatchResult，不能直接将 prefix length 加入 API 当成问题已解决。

局部验证不必等待全部祖先确认才开始。比如 p2 在 p1 的前缀上与 target 结果匹配，可以先记录这次局部匹配；依赖 p1 的验证结果和后续工作仍不能进入不依赖 p1 的权威路径。p1 被否定时，这些结果一起失效。这里要补的是依据本身的依赖和采用顺序，不能为了避免误接受而把并行验证重新串行化。

#### 3. task 内的候选寿命，是否等同于所有可复用结果的寿命？

**涉及算法**：DynBranch 最直接；Memory 的跨 task 经验与它有关，但不是同一种状态。

`branch-semantics.md` §1 将 identity、handle、capability 和 resolution 限定在一棵 Tree 内；`policy-decisions.md` §2.8–2.9 的调度也限定同一 task。这适合拥有独立执行世界的 Branch，但不能直接承载 DynBranch 已确认结果在后续请求中的消费。现有 Outcome 复用方向和 Ledger 记录尚未规定跨 task 的访问授权、寿命与失效；将整棵 Branch 跨 task 挂接不属于现有语义。

**建议，尚未批准**：保留 Tree 和 Branch 的 task 所有权。跨 task 复用若纳入范围，应仅通过明确受控的结果移交／保存，而不是共享活的 Branch、Agent context 或未清空假设的数据。至少核对完整输入、模型与执行版本、状态读依赖、新鲜度、访问权限、保留期限、取消与资源归属；TTL 和参数相等不足以代替这些条件。也可明确将跨 task 缓存交给外部服务，AScout 只校验当前消费的依据与保证，不必自行实现缓存平台。

独立 offer 不是互斥世界的多个 winner。是否可拆为多个独立 Edge 要看其真实需求与依赖，不能为了绕过 single-winner 机械拆边。共享容量的部署级预算也不能假装由单 task 的 `select` 已经完成。

### 8.4 其他待讨论问题（不在当前同时展开）

以下只列证据与候选方向，讨论顺序由用户决定：

- **预测器记忆与在线更新**：`predict.history` 和外部来源允许读取历史，但没有明确完成任务／命中／未命中反馈如何送达、由谁保存经验、何时切换模型或记忆版本。Memory、DSP、LEAP 的在线更新都需要此边界。候选方向是将学习系统保留在外部，提供有来源和版本的事实与更新边界，而不是让 Policy Engine 持有任意 mutable state。未决候选数据不能未经筛选写成权威经验；可保留“预测错误”作为有明确标记的审计事实，而非接受其业务结论。
- **状态验证与远期投机**：AOSpec 的文件系统版本、DynBranch 的读版本和 TomasuLLM 的依赖新鲜度不能由现有隔离强弱声明自动推出。需要区分隔离、执行前状态标识、实际读依赖和消费时验证。AOSpec §4.3 明确允许候选跨过不改变受管环境的中间调用存活；如果每次 Branch Accept 都按 §4.5 无条件清理全部 sibling，则这些候选会提前丢失。远期动作的机会归属、到期和废弃须与真实依赖一致，不能只靠树的位置判定其失效，也不把整个未来轨迹都绑定为一个错误的祖先假设。先检查已有 Outcome/Reuse 是否可承载，不直接把树改成 DAG。
- **候选执行配置与透明接入**：`predict` 可用小模型，不等于后续 candidate Agent 已能使用不同于主 Agent 的模型、prompt 或执行器。SpecAgent/LEAP 的 weak–strong 路线、Skim 的快捷执行器需要可声明、可验证的执行差异。默认 Agent 不感知 Branch 可以保留，但“透明”不能误读为所有路径必须使用同一模型和完全同一执行步骤。
- **模型服务能力及 Fidelity**：D-013 明确排除 engine plane，因而共享 KV、partial-token 接受、sequence residency、batch 调度不属于底座自动提供的能力。外部服务可提供这些能力，但其请求控制、取消、版本、上下文和可复现性要求尚不应冒充 VM 隔离声明。LEAP 证明 target authorship 不推出串行轨迹相同；精确工具参数匹配也不证明提前读取到的 observation 与稍后读取相同。建议先明确可观察能力与保证，不因论文标注 lossless 自动选择最高 Fidelity。
- **用户交互边界**：ISP 的投机结果预览及用户直接纠正候选，与当前只向用户呈现权威主线的规则不完全相容；Speculative Interaction Agents 的持续新输入也受“前提改变则剪枝、不修复”限制。可选择只覆盖不预览候选的算法核心，或另行讨论受控预览，但不能声称完整 UI 已兼容。
- **推测数据流约束的精度**：Policy §4.2 说数据流向“同样携带未决 Proposal ID”的容器，旧决策记录部分措辞仅要求接收方也非空。仅非空不足以保证安全：依赖 p1 的数据不能进入只携带 p2 的容器。需要按实际依赖覆盖关系理解并统一文档；这是多步验证与嵌套算法正确性的前提，不是允许未决数据进入权威上下文。

### 8.5 既有算法回归检查

下表检查 survey 内全部其余算法条目；基础设施、普通异步执行和社区原型不作为算法覆盖率分母。结论是设计级映射审查，不是接入成功或性能复现。

| 算法／路线 | 当前模型可表达的部分 | 未闭合之处／范围边界 |
|---|---|---|
| Speculative Actions | API 结果预测、多候选、call specifier 匹配、连续嵌套、失败剪枝 | 缓存 future 的消费与主执行保留需统一 Reuse／accept 映射（§8.9 第 1 项）；不能只把参数相等当作所有状态下结果相等 |
| PASTE | 模式生成具体调用，参数绑定完成后再形成不可变 Proposal；只读／dry-run 纪律、选择与抢占 | 提前工具结果／在飞调用复用同 §8.9 第 1 项；未绑定 preparation hint 不能直接冒充可执行 Proposal；co-scheduler 与部署级调度不完整覆盖 |
| SPAgent Verified | 推理期间预测动作、精确匹配后复用工具结果 | Reuse 入口已有，需统一 accept 映射（§8.9 第 1 项）；推理引擎两级调度不在范围内 |
| SPAgent Aggressive | `launch=false`，依据明确接受规则跳过部分主推理，按弱 Fidelity 运行 | 不能将置信度阈值当作已证明等价；策略切换仍须由一套 task Policy 显式表达 |
| Cost-Aware Speculative Execution | `admit/select/rebalance`、资源计量、`effect`、`match`、失败重执行足以承载局部成本决策 | 任意 DAG 的 fan-in/shared work 不是当前 Tree 自动保证；若拆成结构化 Proposal 仍须提供联合依赖和中间状态。概率更新反馈、跨任务容量与机会成本未闭合；不以论文公式改写机制 |
| Speculative Interaction Agents | 未来用户输入／工具结果预测、持续事件、级联取消、写前缓冲 | 仅权威主线对用户输出；新输入使前提改变时现模型保守剪枝。不能声称覆盖论文的全部交互与重对齐行为 |
| Speculative Tool Calls | client-side 提前调用、精确参数匹配、结果复用的局部流程 | 同 §8.9 第 1 项；engine-side KV 拼接、resident sequence 与提议中的 provider endpoint 明确不在现有范围内 |
| Skim accelerate | 快路径、`launch=false/lazy`、validator 接受、失败后 URL salvage 与 fallback | 不要求保留失败推理；URL handoff 必须是明确允许的输入，不把被拒答案移入权威 context。独立快路径执行配置尚未具体化 |
| Skim aggregate | 多候选试验、validator 排序、至多一个最终输出 | 不等于自动支持多 loser 信息汇总；只选择一个经审核结果可映射，预算与快捷执行器能力仍需接入 |
| SPORK D1/D2 | 流式推理中预测调用、置信度准入、只读提前执行、精确匹配 | 需要流式前缀/logprobs 等实际服务能力；“真实 operation 前 barrier”与中途观测点的关联需接入侧证明。KV fork 加速不由普通 API 保证，结果复用同 §8.9 第 1 项 |
| SPORK D3 | dispose 可表达回收部分进展的意图 | 将 token draft 送入引擎验证器是明确排除的 engine plane，不能只因存在 Salvage 就声称完整覆盖 |
| ISP | 近似步骤连续推进，各步 target 异步验证，前缀匹配后采用，错误后缀丢弃 | 同 §8.9 第 4 项；论文 UI 展示与用户裁决投机结果不符合当前权威主线唯一可见边界，算法核心与完整 UI 必须分列 |
| DSP | ISP 路线加动态 extent、预算和收益权衡 | 同 §8.9 第 4 项；在线 trainer 与 inference predictor 的权重更新不是 `extent` 自己拥有的状态，需外部学习及版本边界 |
| SpecHop | observation 预测、连续嵌套、异步真实工具、按顺序验证、失败级联清理 | 基本拓扑匹配；后续 target 调用也必须携带祖先假设。语义等价 verifier 的范围不能用“lossless”一词代替具体保证 |
| SMC | 结构化宏 Proposal、锚点真实证据、模式与安全 predicate、批量采用动作结果 | 允许独立 validator/弱 Fidelity，因此无需强制逐步 actor 确认；但宏前缀状态、整段接受范围与后续 drafter 延续同 §8.9 第 4 项，不能仅确认锚点就清空所有未检查依赖 |
| B-PASTE | 多候选子图预测、extent、资源调度、effect 边界、晚绑定后的具体调用 | 共享子图、前缀复用与未来调用消费未闭合；一条 Edge 的多个候选默认不协作，不能把 beam 子图共享当作已有能力。论文证据初步，不据此先扩全局模型 |
| TomasuLLM（算法与运行时交叉） | 动作／observation 预测、私有写入、依赖检查、按正式轨迹验证与公开 | 乱序动作执行、实际读版本和效果 delta 的按序采用超出当前 Branch replacement 的明确语义；需统一 Reuse 映射（§8.9 第 1 项）并补状态验证契约（§8.9 第 3 项）。当前模型可用更保守的串行屏障退化，但不等于保留其核心乱序能力 |
| NVIDIA / LangChain 已知图路由投机 | 路由结果为 Proposal，多互斥候选，路由确认后选择一个、取消其余 | 只读或受控效果下基本映射；若框架采用节点结果而不替换 Agent 世界，仍需 §8.9 第 1 项的 Reuse 映射；取消不提供外部回滚 |

### 8.6 调整原则与后续顺序

1. 不推翻已经有用的抽象：action/observation 不必拆成两套系统；精确与语义接受可继续由 match 和 Fidelity 区分；不可逆效果继续受 gate 约束。
2. 先统一 Reuse／accept 映射（§8.9 第 1 项），再补批量验证拆分（第 4 项）；这两项主要是文字和规则澄清，收益最大、风险最小。
3. independent offer 是唯一真正动模型的点（第 2 项），必须单独决定；选择保守方向时，把 AOSpec／DynBranch 的 independent offer 明确标为不覆盖。
4. 状态版本、读集合和候选执行配置作为能力契约补（第 3、5 项），不放进九类 Decision。
5. 跨 task 缓存、DAG fan-in、engine plane、用户预览先写清不覆盖（第 6–8 项），不以“通用”之名无限扩大系统。
6. 当前没有批准任何上述模型调整；本次只形成证据、分类和待讨论建议。

### 8.7 续查：结果 Reuse 的最小调整草案（尚未批准）

#### 原文要求与现有条款冲突

[PASTE §4.2](https://arxiv.org/html/2603.18897#S4.SS2)、[SPAgent §III-C](https://arxiv.org/html/2511.20048#S3.SS3) 和 [Speculative Tool Calls §3.1](https://arxiv.org/html/2512.15834#S3.SS1) 都要求：主模型按自己的上下文生成正式调用，匹配后使用提前完成或仍在运行的工具工作，未命中则正常执行原调用。PASTE 所称的在飞工作 promote 是工具 job 转为正式调用的执行来源，不等于采用另一份 Agent 上下文。

用一个最小过程即可区分两种行为：

```text
主 Agent：H ──真实推理 R──> 调用 A ──使用 O──> 从 H、R、A、O 继续
候选工作：H ──预测 A────> 提前执行 A ──> O
```

原算法要求保留主 Agent 的 R；`branch-semantics.md` §4.5 的替换却保留 candidate identity，并不要求传递分叉后的 parent context。把两条路线一概映射为 accept，可能丢失 R，或错误地用预测器的轨迹替代它。即使最终答案碰巧相同，也不能声称复现了原算法。

反方向的例子来自 [ObsSpec §2.2](https://arxiv.org/html/2610.15246#S2.SS2)：真实工具可能已经修改文件；候选只在预测 observation 上提前生成下一动作。预测匹配后要保留的是生成进展与真实工具完成后的环境，不能直接切换到工具执行前分叉、尚未包含真实修改的 VM。当前 Branch §4.5 虽要求 winner 继承必要的已生效效果，但环境隔离和 Proposal 匹配本身并未提供该衔接的能力证明。

#### 建议在已有语义上明确的行为

以下是对已有 Reuse、Promote、Outcome 和 Branch 的调整建议，不新增名称、Decision、实体或接口：

1. Reuse 允许当前调用方保留自己的执行身份和上下文，仅经受控路径采用已验证的结果；整 Branch Accept 继续表示用 candidate 的执行延续替换 parent。是否采用整 Branch，由实际选择的处置意图和能力决定，不能从“采用了投机产物”自动推出。
2. 必须明确采用范围：只采用返回值、还采用 artifact／受管状态变化，或直接采用完整执行延续。前两者不授权复制候选的任意内存、推理上下文或全部世界状态；“保留主 Agent”也不表示接受不同世界的任意 merge。
3. 复用前核对结果对应的真实调用、实际输入、所依赖状态和有效性；调用名、参数相等只是其中一部分。只读不意味着新鲜，TTL 也不意味着外部数据没变。无法证明时重新执行或按声明降级。
4. Reuse 的授权必须与原调用跳过相衔接，避免候选和主路径重复产生同一受管效果。未完成工作可以继续作为正式调用的结果来源，但须明确等待、失败、取消和资源归属；不能仅把已完成结果的 Reuse 定义照搬给在飞工作。
5. 有状态工具的返回值、artifact 和效果必须一致采用。不能返回“修改成功”却丢掉修改，也不能提交效果后再执行一次原调用。需要保留进程、会话或流的工作，不能只用 stdout／exit code 替代；缺乏移交能力时回退。
6. 采用成功后再关闭相应候选与效果责任、清理无用执行承载，并按真实依赖处理后代。采用失败或部分完成不能报告为成功。single-winner 继续防止同一互斥选择的多个结果或世界同时生效；它不应被解释为每次采用一个结果都必须替换主 Agent，或把所有其他 Edge 的工作一律清掉。

这些条件是机制的强制责任。Policy 决定希望复用什么，不能自行删除假设、把状态视作已提交、跳过清理或直接修改 Runtime。

[AOSpec §4.3](https://arxiv.org/html/2608.00881#S4.SS3) 用受管文件系统版本验证执行前状态；[TomasuLLM §3.6–3.7](https://arxiv.org/html/2609.38201#S3.SS6) 则分开验证工具结果的可复用性和预测 observation 对后续轨迹的有效性。这些事实说明，采用的结果必须有自己的有效性依据。它们不要求 AScout 为任意服务自动推导读集合，也不能把状态增量回放扩大成通用世界 merge。

#### 上层模型和后续文档的影响

若用户接受上述方向，根模型下一层可以先按以下内容调整；此处只是候选表述，不直接改写正式设计：

> 基于 Proposal 提前完成的工作，通过验证后，可以由当前执行受控采用其结果和必要的受管状态，也可以由候选的执行延续替换当前执行。采用范围必须明确，相关未决依赖、状态、效果与资源责任必须先满足条件或完成合法移交。

这要求检查 mental-model §2–3 与 design-model §2.1／§3.1 的一致性，再同步 Policy dispose、Branch resolution/accept、Outcome 返回路径、效果处理及 glossary 的层间映射。具体状态转换和所需后端能力留到相应下层讨论；不能只修改术语表就宣布问题解决。

当前 Runtime Adapter 固定环境操作没有定义通用状态增量移交／回放。纯结果 Reuse 不一定需要新增环境操作；涉及受管效果时，应先明确接入侧与 Effect Coordinator 的责任，再判断哪些环境能力确实要扩展。Firecracker 能分支和隔离，不自动说明它已能安全拼接任意两个环境的状态。

本轮结论仍是待讨论草案，不是新增目标语义，也不是编码授权。

### 8.8 原文核对索引与范围边界

以下列出续查实际读取的论文正文位置。表中“重点”用于复核前述矩阵，不表示论文的全部功能均已适配。外部 HTML 下载留在临时目录，未向仓库添加论文副本或研究实现。

| 论文 | 原文位置与本轮核对重点 |
|---|---|
| Speculative Actions | [§2](https://arxiv.org/html/2510.04371#S2)、§5.3：按调用描述缓存 future；连续预测和命中后的调用消费 |
| PASTE | [§4.2–4.3](https://arxiv.org/html/2603.18897#S4.SS2)：保留正式调用路径、完成结果复用／在飞 job 转用，以及跨 session 的 LLM 调度 |
| SPAgent | [§III-B–III-E](https://arxiv.org/html/2511.20048#S3.SS2)：Aggressive 与 Verified 两条路线、Action Buffer，不把两者都归为严格验证 |
| Cost-Aware | [§1.4、§3.3](https://arxiv.org/html/2606.07846)：静态 DAG 范围、预测错误重执行的效果前提；不自动采用其幂等性或无外部痕迹假设 |
| Speculative Interaction Agents | [§3.1–3.3](https://arxiv.org/html/2605.13360#S3.SS1)：增量输入、异步 observation、工具调用修订和写前确认 |
| Speculative Tool Calls | [§3.1–3.2](https://arxiv.org/html/2512.15834#S3.SS1)：client future 复用与 engine cache／draft 注入分开 |
| Skim | [§3.1–3.4](https://arxiv.org/html/2605.16565#S3.SS1)、§5.2：快路径接受、只传 URL 的失败回收、aggregate 的输出选择 |
| SPORK | [§4.1–4.3](https://arxiv.org/html/2607.03333#S4.SS1)：前缀缓存、置信度 gate 与引擎 partial-token 路线分开 |
| ISP | [§3.1–3.2](https://arxiv.org/html/2410.00079#S3.SS1)：假设前缀上的并发 target 调用、错误后缀取消及投机 UI |
| DSP | [§4.1–4.3](https://arxiv.org/html/2509.01920#S4.SS1)：动态停止、在线 predictor 更新与运行编排的边界 |
| SpecHop | [§3.3–3.4](https://arxiv.org/html/2605.21965#S3.SS3)：连续窗口、按前缀确认、失败后的下游剪枝 |
| SMC | [§3.1–3.3](https://arxiv.org/html/2609.03236#S3.SS1)：history 与 live state 一致推进；锚点确认不等于所有宏动作均由 actor 逐步确认 |
| B-PASTE | [§4、§6–§9](https://arxiv.org/html/2604.16469#S4)：候选子图、准备节点、前缀执行、安全限制与初步证据状态 |
| Speculate with Memory | [§2.1–2.2](https://arxiv.org/html/2607.12236#S2.SS1)：action／observation／链式预测、只读白名单、跨任务记忆 |
| Speculate While You Reason | [§2.1、§3、§4.1](https://arxiv.org/html/2607.25816#S2.SS1)：调用精确命中、同模型双模式和串行评估路径；训练效果不等于并发运行时覆盖 |
| AOSpec | [§4.1–4.3](https://arxiv.org/html/2608.00881#S4.SS1)：action 与 origin state 联合验证；候选跨不改变受管状态的中间调用存活 |
| DynBranch | [§3.2–3.4、Appendix A](https://arxiv.org/html/2609.31047#S3.SS2)：输入与读版本一致、先选择再授权、跨请求复用和独立 offer |
| LEAP | [§3.1](https://arxiv.org/html/2610.02670#S3.SS1)、Appendix M：前缀顺序采用、纠正／bonus、接受环境不重跑工具、target authorship 的保证范围 |
| ObsSpec | [§2.2–2.3](https://arxiv.org/html/2610.15246#S2.SS2)：生成提前、真实工具顺序执行、精确 observation 验证及外部训练反馈 |
| SpecAgent | [§3.1–3.2](https://arxiv.org/html/2610.21463#S3.SS1)：真实读、写前停止、纯读 K 步免审、最长可接受前缀与修正 |
| TomasuLLM | [§3.2–3.7](https://arxiv.org/html/2609.38201#S3.SS2)、Appendix A.3–A.5：实际依赖、私有执行、顺序公开、效果采用；结果复用与预测正确性分别检查 |

survey 中的基础设施可以提供局部能力，但不能被当成完整算法适配的证明：TClone 的分支能力、BranchFS 的文件语义和 Chronos 的受管存储分支都不自动涵盖 Agent context 和任意外部服务。Chronos 的 merge 不是当前算法覆盖必须立即引入的通用 merge。Atomix、Ghost Tool Calls 和 Kairos 分别提示效果结算、调用发出时隐私及延后授权的约束；是否提供其全部保证需单独检查服务边界和威胁模型，不能仅凭 `stage` 或 validator 的名字宣布覆盖。

W&D、AsyncFC、Execution Edits 和 Tracking State Footprints 仍按 survey 作为相邻工作；它们不要求每个并发／恢复动作都建立预测 Proposal。社区 speculative-tools 涉及同样的结果复用和缓存有效性问题，agent-lsp 的文件变更暂存也不证明全环境接受语义。以上不改变 survey 分类或 AScout 当前范围。

### 8.9 第二轮分类结论（2026-10-23）

这一轮把“对不上”分成三类：正式条款互相矛盾、能力契约或事实缺口、明确范围边界。分类标准不是“Policy 能不能写一个函数”，而是完整执行、验证、状态采用和清理能否在现有语义下说清楚。

2026-10-23 更新：第 1、3、5 项方向已由用户确认；第 2 项暂按 A（不覆盖 independent offer）；第 4 项按“逐 Edge 原语 + Policy 组合”处理，不新增批量审核原语。

| # | 具体场景 | 现在哪里对不上 | 性质 | 建议／状态 | 修正后的影响 |
|---|---|---|---|---|---|
| 1 | 主 Agent 保留自己的推理，只采用候选提前跑出的工具结果 | `dispose` 已把 Reuse／Promote 分开，接入侧已有跳过原调用并返回结果的路径，D-031 已有 command reuse 条件；但 `glossary.md` 把两者都映射为 `accept`，`branch-semantics.md` §4.5 只描述整 Branch 替换 | 正式条款互相矛盾 | 把 `accept` 定义为 resolution 结果；Reuse 只移交已验证结果和按声明需要移交的受管状态，然后清理候选；Promote 才替换主延续。带状态变更的复用继续受 D-031／Effect Management 约束 | 不新增实体、Decision 或接口；改 `branch-semantics.md`、`glossary.md`、`policy-decisions.md` §2.7、`agent-integration.md` §2.1／§2.3、`effect-management.md` §1.3 的文字和交接规则。树、single-winner、效果屏障不变 |
| 2 | `j` 时刻提前算 `j+3` 的动作；`j+1` 接受了另一个候选，而 `j+3` 的结果并不依赖被拒 Proposal | §4.5 规定 Accept 后旧 parent 和 candidate 的 sibling subtrees 失效；AOSpec／DynBranch 要让独立提前结果活到自己的 Edge | 正式条款与论文行为冲突，是真正的模型问题 | **暂按 A（不覆盖 independent offer）**：保守，只支持落在被接受路径上的前缀。备选 B：扩展，允许不依赖被拒 Proposal、执行前状态版本仍匹配的候选继续存活，并在自己的 Edge 上重新参与 `match`／`dispose` | A 只改范围和 survey 表述，不改模型。B 会动 Branch 生命周期、sibling 清理、single-winner、AssumptionSet、状态版本和资源所有权，必须单独设计、实现和测试 |
| 3 | AOSpec 的动作加执行前环境版本；DynBranch 的读集合新鲜度；TomasuLLM 的实际读依赖 | Runtime Adapter 只声明隔离属性，不提供状态版本、读集合或新鲜度；D-031 只有 command reuse 的 workspace revision，不是通用状态版本 | 能力契约缺口 | 把状态版本和读集合做成可选能力；Policy 需要时由 Contract 校验，后端没有就拒绝或明确降级；`match` 的 evidence 携带这些事实及其来源 | 改 Runtime Adapter Contract、Capability View 和 `match` 输入；Branch 语义不变。实现需要后端配合，不能只靠控制面推断 |
| 4 | LEAP 一次验证 `k+1` 个上下文；SpecAgent 返回 `(j, reason, replacement)`；SMC 一次接受宏 | `MatchResult` 是开放的，但没有规定一次批量结果如何拆成每条 Edge 的确认／拒绝，也没有规定验证证据自身携带哪些未决 Proposal 依赖 | 规则缺口，不是结构不支持 | **方向：不新增批量原语**：`match`／`dispose` 保持逐 Edge，由 Policy 在一次审核后把结果分发；机制按前缀顺序逐条确认／拒绝，再采用对应状态。需补修正动作入口和证据依赖；确有必要时再给 evidence 增加结构化前缀字段 | 主要改 `policy-decisions.md` §2.6／§4.2 的说明；可能扩展 `MatchResult`／validator 契约。不改树，不新增 Decision |
| 5 | SpecAgent／LEAP 的 weak–strong；Skim 的快路径执行器 | `predict` 可以用小模型，但候选 Branch 的执行配置没有声明位置；文档没有说明候选能否使用不同的模型、prompt、工具集或执行器 | 能力契约缺口 | 在 candidate／Instance 上声明执行配置，由 Contract 和 Fidelity 校验；`predict` 仍只负责生成 Proposal | 改 Agent 接入侧、Contract、Instance metadata 和 Fidelity 校验；Edge／Branch／九类 Decision 不变 |
| 6 | DynBranch 跨 task 缓存已确认结果 | Branch identity、handle、capability 和 resolution 都限定在一棵 Tree／一个 task 内；Outcome 可以受控保存，但没有跨 task 的授权、寿命和失效规则 | 范围边界，可外部实现 | 不把活 Branch、Agent context 或未清空假设挂到另一个 task；用外部结果缓存或明确的受控移交，AScout 只校验消费时的依据 | 不改模型；若要在 AScout 内置，才需要新的结果存储、授权和寿命契约 |
| 7 | Cost-aware／B-PASTE 的 DAG fan-in、共享子图 | 当前是树；多个消费者共享同一份中间结果需要重复计算，或需要另一套执行模型 | 范围边界 | 明确不覆盖，或以后单独扩展 DAG；不能把 beam 子图共享说成已有能力 | 不改模型；扩 DAG 会改变树、single-winner 和依赖语义，是另一套执行模型 |
| 8 | ISP 的候选预览、Speculative Interaction Agents 的持续交互；SPORK D3／KV；TomasuLLM 的完全乱序提交 | 当前只向用户呈现权威主线；engine plane 已由 D-013 排除；乱序提交／rebase 超出 Branch replacement 的明确语义 | 范围边界 | 分别保持不覆盖，或用更保守的串行屏障退化；需要时单独设计 | 不改现有模型；不能因论文依赖这些能力就默认扩大 AScout |

#### 对首轮判断的更正

- 不能说“Reuse 没有入口”：`policy-decisions.md` §2.7、`agent-integration.md` §2.1／§2.3、D-031 都已经给了入口；问题是 `accept` 的映射没有统一。
- 不能说“树不支持前缀”：逐步 Edge 加嵌套 Branch，再配合 §4.7 的后代重挂，可以表达“接受前两步、丢掉第三步以后”；问题是批量证据拆分和每步状态保留，不是树结构本身。
- 不能说“SpecAgent 需要任意写后回滚”：它的真实读是 PURE，首个写是 held；可以建模为共享环境中的 candidate 读，加 `effect=block` 的首个写。问题是 weak／strong 执行配置和 `(j, reason, replacement)` 的归属。
- 不能说“LEAP 需要另一套轨迹树”：每步一个 Edge、target 作为 Authority、按前缀 Accept 就能表达；问题是并发 target 调用、每步环境状态和批量结果拆分。
- ObsSpec 是七项中最贴合的：真实工具是 Authority，预测 observation 是 Proposal，后续生成是 candidate；`match`／discard 正好对应。
- Memory 的跨 task 记忆是预测器输入，不是结果复用；不需要因此把记忆实体放进底座。

#### 建议顺序

1. 先统一 Reuse／accept 的映射。这是文字级调整，收益最大、风险最小，能同时解锁新旧算法的结果复用。
2. 批量审核不新增原语：保持逐 Edge `match`／`dispose`，由 Policy 在一次审核后把结果分发；补前缀顺序、修正动作入口和证据依赖。
3. independent offer 暂按 A：保留 sibling 失效，AOSpec／DynBranch 的独立提前结果当前不覆盖；B 留作后续扩展。
4. 状态版本、读集合和候选执行配置作为能力契约补，不放进九类 Decision。
5. 跨 task 缓存、DAG fan-in、engine plane、用户预览先写清不覆盖，不因“通用底座”之名扩大范围。
6. 第 1、3、5 项方向已确认，第 2 项暂按 A，第 4 项按逐 Edge 组合；第 1 项正式文档同步待嵌套产物、成功采用表示和在飞调用归属收敛后执行。
