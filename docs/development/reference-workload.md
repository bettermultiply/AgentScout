# 首要验证 Workload

本文记录 AScout 第一阶段的开发与验证 workload，不定义 AScout 的全部目标语义，也不预先选择 continuation branching backend。

### 复杂失败与恢复验证（2026-10-23）

在同一稳定入口上增加了持续交互和失败注入场景：

- `queue-many`：Promote 发生时连续提交 `ALPHA`、`BETA`、`GAMMA`，入口在外部串行化轮次，并在实际送达时读取当前权威执行；三轮答案顺序保持为 `ALPHA`、`BETA`、`GAMMA`，没有进入旧 parent 队列。候选首轮请求仍为 1，后续总请求数为 5。
- `candidate-error`：候选开始真实流式请求后主动取消，权威结果延迟返回；候选收到取消信号并走 `Discard`，没有 Promote 或候选文本发布，parent 继续从权威结果完成 100 行输出。
- `authority-error`：候选开始产生文本后，权威回调抛出错误；候选被清理，未接受的候选增量没有发布，parent 收到工具错误并正常生成可见的错误说明。这里的 parent 回退文本不是候选泄漏。
- `authority-hang`：权威回调永不主动完成，但入口在 300ms 后取消当前轮；AScout 的等待层响应取消并清理候选，进程不再永久挂起。这个保证只结束 AScout 的等待，不虚假声称非合作的外部权威操作已经停止。
- `abort-recover`：Promote 后取消在飞 winner，旧轮 Promise 明确失败；随后轮换入口的 AbortController 并清除已完成的 winner completion，新的请求返回 `RECOVERED`。这固定了一个可复现的取消后无法继续对话问题，驱动场景本身作为回归覆盖。

以上场景均使用真实 `deepseek/deepseek-flash` 调用；候选仍无业务工具，权威操作仍只调用一次。Pi `npm run check`、Agent／Agent-loop 80 项测试和 Rust Policy 测试均通过。

## Delayed-choice coding

第一阶段通过具备足够观察和拦截接缝的 Product Integration，在真实代码仓库执行 delayed-choice coding benchmark。不要求 Agent 主动调用 AScout，也不开放显式生命周期协作；候选创建、Proposal 注入、outcome 发布、提交条件与裁决由接入侧和控制面完成。Parent 等待一个延迟、有限且可验证的真实 outcome；多个 candidates 分别接收互斥 Proposal，并提前完成对应的后续代码修改、工具执行和测试。具体 Agent product 与接入方式尚未选择。

最小场景使用二选一 outcome，例如等待用户、配置服务或可控 oracle 在固定延迟后选择 `sqlite` 或 `postgres`：

```text
Parent: 等待真实 choice
├── Candidate A: proposal=sqlite，提前实现 sqlite 路径
└── Candidate B: proposal=postgres，提前实现 postgres 路径
```

真实 outcome 到达后，控制面依据 Policy 丢弃不匹配 candidate，检查匹配 candidate 的提交证据，并编排 Accept 或安排 parent 从真实 outcome 继续。Outcome source 可以先使用确定性延迟 oracle；candidate 执行必须作用于真实仓库、进程和测试，不能用纯 metadata 分支冒充端到端 workload。

## 验证路径

1. 首先实现控制面最小闭环；设计初期只编写少量端到端测试，具体测试方案须在业务逻辑实现前经用户确认。
2. 通过产品观察和拦截接缝接入首个真实 Agent，自动识别 operation、pending-result boundary 与 outcome，不依赖 Agent 主动调用控制接口。
3. 验证候选执行、winner continuation、fallback 和实际 loser cleanup，再进行真实仓库上的收益对照。
4. 暂不深度修改 Agent loop；以后用第二种 Agent 验证 Product Integration 的可移植性。

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
