# 控制面设计

本文说明 AScout 控制面如何作为 Mechanism 的实现主体，把 [Speculation Contract](speculation-contract.md) 落实到投机执行的控制路径上。控制面是 AScout 自己实现并掌控的部分，不是 Contract 约束的对象；本文描述它如何组织校验与编排，不重复定义 Branch 生命周期、效果管理或 Policy 决策本身的语义——这些分别由 [Branch 语义](branch-semantics.md)、[效果管理](effect-management.md) 与 [Policy Decisions](policy-decisions.md) 承载。

控制面承接两端的差异：一侧是表达投机策略的 Policy，另一侧是承载实际执行的后端 Runtime（经 Runtime Adapter 接入）。控制面把 Policy 的高层决策转成按固定时序的受管操作，经 Adapter 落到 Runtime，再根据 Runtime 返回的结果和事件更新自己的控制状态。

## 1. 控制面在四侧中的位置

一级架构为 Agent 接入侧 → 控制面 → Runtime Adapter → Runtime。控制面持有 AScoutAgent 与 Speculative Tree 的控制状态、Policy 绑定、请求校验、操作编排和 Runtime event 解释；Runtime 持有节点执行状态、资源归属、隔离、staged effect、obligation 与受管状态转换。控制面不直接触碰 Runtime 内部状态，只经 Runtime Adapter 交换 typed operation 与 event。各侧所有权与调用方向的权威定义见 [架构](../architecture.md) 与决策 D-039。

Instance 是 Runtime 管理的运行对象，不是控制面的一部分。控制面看到的是它自己维护的 Speculative Tree 与 Branch 关系；Instance 的真实进程、workspace 与效果状态归 Runtime 所有。二者通过 Runtime Adapter 的稳定接口对齐。

## 2. 控制面要实现的语义

### 2.1 从 Policy 决策到受管操作

Policy 返回的是策略层面意图：是否对该机会投机、预测哪些 Proposal、何时取得权威值、如何处理效果、如何判定命中、如何处置候选。这些意图不能直接对应一次后端调用。控制面先把它们编排成有明确语义的受管操作——创建候选、登记未决依赖、验证、接受、丢弃、清理等——每个操作都有前置条件、作用对象和后续状态。

例如 Policy 的 `dispose` 返回"丢弃某候选"，控制面形成 `discard` 操作，它包含候选身份、当前控制状态、取消范围与清理要求，不是一个可由任意调用方直接写入的布尔字段。控制面只在收到 Runtime 对应结果或事件后，才把候选的控制状态推进到相应终态。

### 2.2 保持的固定语义

控制面在编排过程中维持以下固定语义，它们是 Policy 不能通过选择不同决策来绕过的：

- Branch 生命周期与候选状态转换；
- Proposal 依赖与 `AssumptionSet`；
- single-winner 与候选之间的隔离和受控信息流；
- 效果阻塞、commit barrier 与 obligation 边界；
- 资源准入、预算、优先级与取消；
- 非空 `AssumptionSet` 的数据不得进入权威上下文。

这些语义的细节分别由 Branch、Effect 与 Policy Decisions 文档定义；控制面保证它们在实际执行中被一致地落实。

## 3. 组件与状态

控制面由若干固定组件构成，各自拥有并修改自己的一份状态，通过固定入口协作。组件划分沿用 [投机执行底座抽象札记](../development/spec-exec-abstraction.md) §10.4 的研究结论；本文只保留作为目标语义的部分，实现细节与论文依据不在此重复。

### 3.1 Policy Engine

Policy Engine 是可替换 Policy 进入控制面的固定入口。它把 Policy 返回的内容解释为对应的 Policy Decision，检查这些决策与决策之间的组合在当前 Contract 下是否合法，再结合 Runtime Adapter 已报告的能力判断策略能否在当前接入的后端上执行。Policy 声明超出 AScout 固定语义，或当前后端无法提供所需能力时，控制面拒绝该决策或按既定语义报告降级。检查通过后，Policy 的结果仍只是策略意图，由控制面其他组件继续处理。

### 3.2 Assumption Manager

Assumption Manager 是 Proposal 依赖与确认/否定状态的 owner。它维护谁依赖谁的真相，跟踪每个 Branch 的 `AssumptionSet` 与未决 Proposal。权威值到达时，它触发匹配与处置决策，确认沿依赖闭包生效；被否定的 Proposal 使依赖它的后续工作失效，控制面向受影响的 Branch 发送取消。它同时强制信息流约束：非空 `AssumptionSet` 产生的数据不得进入权威上下文。

### 3.3 Scheduler

Scheduler 负责准入、预算、优先级、取消和资源协调。它决定是否在给定预算与当前后端能力下开启一个候选、为候选分配运行资源、在资源压力下暂停或取消低优先级投机。预算计数在这里而不是在 Ledger：调度决策需要同步读取预算余量。

### 3.4 Effect Coordinator

Effect Coordinator 负责控制面内效果处理的协调。它根据效果分类与状态域，结合 Policy、Contract 与 Runtime 能力形成允许、暂存、转换、延迟、阻塞或补偿计划，并经 Runtime Adapter 请求 Runtime 执行。它不持有真实的暂存效果、obligation 或隔离状态——这些归 Runtime 的 Effect backend。效果阻塞发生在效果边界：投机执行遇到不能安全暂存或释放的效果时停在门口，保留此前的前置进展，等待裁决。

### 3.5 Ledger

Ledger 以异步旁路记录控制面需要保留的运行事实，包括 Policy 决策、Branch 与 Proposal 状态变化、验证依据、效果与 obligation 处理结果、资源使用、取消与清理结果、Adapter 映射结果与 Runtime 失败。它用于状态对齐、问题定位、shadow/replay 和契约校验；它不从记录中重新决定生命周期，也不代替 Runtime 持有执行状态。

## 4. 失败处理：回到"Branch 从未创建"

对尚未被接受的投机 Branch，控制面在失败时把它视作在 Speculative Tree 中从未被创建：不重放该操作，不保留它的部分结果作为成功。被接受的 Branch 已完成使命，不适用这一语义。

落到机制上，控制面沿固定状态入口撤销对应 Branch 的控制状态，并经 Runtime Adapter 请求 Runtime 停止对应执行、处理资源与受管效果。清理具体如何编排由 Runtime 与 Effect backend 落实。Salvage 是可选的额外能力：若某段投机工作产生了可移交权威路径的残值，可以在失败默认之上回收；它不是失败处理的必要条件。

## 5. 与 Runtime Adapter 的衔接

控制面只能经 Runtime Adapter 访问 Runtime。Adapter 把控制面的语义操作翻译成后端可执行的组合调用，把后端能力与结果翻译回控制面可解释的能力事实和规范化事件。Adapter 的实现不受 AScout 掌控，因此控制面不假定 Adapter 自觉正确；它通过接入校验确认 Adapter 能表达基本语义，并通过运行期结果和事件确认操作是否真实完成。接入与运行期的约束见 [Speculation Contract](speculation-contract.md)。
