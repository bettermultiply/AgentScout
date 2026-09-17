# 投机执行论文证据札记

本文只保存投机执行相关论文的事实、实验结果和由这些事实直接支持的研究依据。已经收敛的目标语义以 `docs/design/` 为准；系统结构以 [`architecture.md`](../architecture.md) 为准。本文不再重复定义 Edge、Branch、AssumptionSet、Policy Decision、Effect、控制面或 Runtime Adapter。

## 1. 论文范围

当前调研覆盖 11 篇工作：Speculative Actions、PASTE、Skim、ISP、DSP、SPAgent、Speculative Interaction Agents、SPORK、Speculative Tool Calls、Speculate with Memory 和 Cost-aware Speculative Execution。

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
