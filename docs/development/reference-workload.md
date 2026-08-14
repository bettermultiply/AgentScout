# 首要验证 Workload

本文记录 AScout 第一阶段的开发与验证 workload，不定义 AScout 的全部目标语义，也不预先选择 continuation branching backend。

## Delayed-choice coding

第一阶段使用 `ascout codex`、Command + Skill 和真实代码仓库执行 delayed-choice coding benchmark。Parent 等待一个延迟、有限且可验证的真实 outcome；多个 candidates 分别接收互斥 Proposal，并提前完成对应的后续代码修改、工具执行和测试。

最小场景使用二选一 outcome，例如等待用户、配置服务或可控 oracle 在固定延迟后选择 `sqlite` 或 `postgres`：

```text
Parent: 等待真实 choice
├── Candidate A: proposal=sqlite，提前实现 sqlite 路径
└── Candidate B: proposal=postgres，提前实现 postgres 路径
```

真实 outcome 到达后，parent discard 不匹配 candidate，验证匹配 candidate 的 commit，并选择 Accept 或从真实 outcome 继续。Outcome source 可以先使用确定性延迟 oracle；candidate 执行必须作用于真实仓库、进程和测试，不能用纯 metadata 分支冒充端到端 workload。

## 验证路径

1. 确定性测试 Agent 只用于 lifecycle、隔离和失败路径的 conformance tests。
2. 现有 Codex 通过 Skill 主动调用 AScout 命令，作为第一个真实 Agent workload。
3. 在 Command + Skill 路径证明收益后，再验证 Hook 自动捕获 safe point 和 outcome boundary。
4. 暂不深度修改 OpenCode 或其他 Agent loop；以后用第二种 Agent 验证 launch adapter 和 integration profile 的可移植性。

这种顺序把 runtime 机制、speculation policy 和 Agent integration 分开验证，避免同时修改 Agent 与 AScout 后无法判断失败来源。

## 对照与指标

同一 Agent、模型设置、代码任务、outcome 和资源预算至少运行两种模式：

- Sequential baseline：等待真实 outcome 后再执行后续工作。
- AScout speculation：等待期间并行执行 hypotheses，真实 outcome 到达后只保留一个 winner 或回退 parent。

正确性指标：

- 真实 operation 只执行一次；
- siblings 的文件、进程和内部状态互不可见；
- loser 进程、修改和 effect obligations 被清理；
- Proposal 验证和 Accept gate 不可绕过；
- 无匹配 candidate 时 parent 可以从真实 outcome 正常继续；
- 最终结果满足与 sequential execution 相同的任务验收条件。

有效性指标：

- end-to-end wall-clock latency 与 speedup；
- barrier、Branch 创建、outcome injection、Accept 和 discard latency；
- Proposal hit rate 与提前完成的有效工作比例；
- loser 消耗的 token、CPU、内存和工具调用；
- 每单位额外资源换取的 latency reduction。

理想延迟关系为：

```text
Sequential ~= outcome latency + downstream work
AScout        ~= max(outcome latency, downstream work) + AScout overhead
```

只有正确性成立且测得的 latency reduction 能覆盖额外成本，才能说明该 workload 上的 AScout 机制有效。

## 后续扩展

二选一 exact outcome 闭环后，再依次加入多类别 predicate、实际 payload 差异、virtual-value mapping、真实 CI/test result、用户审批和远程服务 outcome。外部不可逆 effect 不作为首个 workload 的前置条件。
