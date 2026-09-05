# AScout FAQ

本文解释已经确认的 AScout 语义和当前仓库状态，不描述某个具体 Runtime backend。

## Branch 是 Snapshot 吗？

不是。Branch 是活的计算延续。Backend 内部可以使用 checkpoint 或 VM snapshot，但 Snapshot 不是 AScout semantic entity。

## 创建 candidates 后 parent 还能运行吗？

Speculative Tree 中的关系与 Agent execution activity 相互独立。Speculation window 内，parent 可以按 Agent/控制面 Policy 执行真实 operation、等待、suspend 或 resume；AScout 不强制 parent 长期阻塞。

## Parent 的内存修改会自动进入 child 吗？

不会。Child 先从 Branch barrier 登记为逻辑 Branch；只有在需要独立执行状态时，才从相应 execution base 建立独立环境。独立环境建立后的 parent 受保护进程内存、Agent context 和环境修改不会隐式传播；Outcome、Workspace、Input 和 Effect 各自遵循明确的可见性规则。

## Parent 和 child 的后续写入如何排序？

具有覆盖语义的状态在当前 Branch 与 ancestors 之间使用 latest-visible-write。Input 与全局服务 Effect 使用不同语义，不能强行套入该模型。

## AScout 必须理解每个 syscall 吗？

不需要。文件操作进入 Branch Workspace View；shell/tool invocation 是 operation、outcome 和受控 reuse 的主要语义粒度。Opaque command 可以在隔离 Branch 中运行，但不能盲目去重。

## 可以复用 sibling 的 command 结果吗？

只有显式 command contract 和输入 context 等价时才可以。Query 可以复用 result；transform 还需要兼容的 Workspace delta 或 artifact；shared operation 必须通过 broker/effect boundary 只执行一次。Persistent session 与 opaque command 不能只用缓存 stdout 表示。

## Commit 会 merge Branch state 吗？

不会。Commit 提交 candidate subtree proposal；Accept 选择一个 winner 并重定向 Speculative Tree，同时丢弃不兼容 siblings。AScout 不定义通用 sibling merge。

## Child 可以自行成为 winner 吗？

不能。默认由 direct parent Agent 持有 resolver capability，并可以显式委托给控制面。Runtime 的 validation 和 Accept gate 始终是机制强制边界。

## 计划提供哪些 Agent-facing operations？

Context、direct children、candidate 创建、commit、accept、discard 和 event wait 目前只是 placeholders，没有最终命令语法或 payload。Outcome 发布和 resolver delegation 不是独立 routine commands。

## Candidate 可以带着 live descendants commit 吗？

可以。Commit 可以提交完整 live subtree；Accept 保留 winner identity 和 descendants，并让 winner 替换旧 parent 的树位置。

## Input 必须全部读取后才能 commit 吗？

不需要。每个 Branch 有独立 delivered/read 位置，但未读 Input 不阻塞 subtree commit，commit 也不表示输入已经被语义处理。

## Loser 的进程和资源会怎样？

AScout 只接受能够通过 Adapter 证明其 Runtime 能提供隔离执行环境、环境级资源边界、停止和 cleanup 的接入；Agent 接入侧负责管理完整 Agent process tree 及其 Branch context。Accept 或 discard 的整体结果仍必须表明相关 Agent 执行活动、环境资源和受管 effect obligations 已按各自责任边界收敛，不能只依赖 Agent 自愿合作。

## External effects 会自动回滚吗？

只有落入 adapter 声明的 observation surface 和 guarantee horizon 的 effects 才获得 sibling non-interference 与 winner 状态延续保证。Contract 外的 operational footprint 可能只能归属和计量。

## 默认 Runtime 会 sandbox 恶意 Child 吗？

不会。默认 Runtime 面向可信 Child Agent 和高效环境隔离。环境边界、资源归属、停止和 cleanup 仍是强制环境要求；Branch identity、capability validation 和投机 lifecycle 由 Agent 接入侧与控制面负责。对抗恶意 workload 的宿主 containment 需要另行选择 sandbox/hardened Runtime。

## 当前有可运行实现吗？

没有可用 AScout lifecycle 实现。当前仓库只有设计文档，不提供真实 Agent execution、Branch lifecycle、Runtime backend 或 sandbox。
