# AScout Agent 定位与设计思考

本文记录 AScout Agent、控制面与 Runtime 的定位。它不是接口规格，也不替代 `docs/design/` 中已经确认的目标语义；它用于保存当前对核心抽象的理解、阶段目标和后续演进方向。

## 核心定位

AScout 是 Agent 分支执行的通用底座；控制面是其中负责控制语义与编排的一侧。

这里的“通用”不是说第一版就要支持所有 Agent 执行模式，而是说控制面的核心抽象不应只服务某一种固定流程。第一版实现仍以 Agent Speculative Execution 为主要目标；后续可以在同一组分支、控制、选择和清理语义之上扩展到更多 Agent 分支执行模式。

第一版重点可以继续偏向投机执行，包括 candidate、Proposal、commit、accept、resolver 等术语。只要这些术语不把核心语义锁死在单一流程里，它们可以先保留。后续如果需要更通用的表达，应优先在文档和外部 API 层逐步收敛，而不是过早重命名造成实现和讨论混乱。

## AScout Agent 的心智模型

AScout Agent 不是某一个具体 Agent 进程，也不是某个模型实例。

AScout Agent 表示一整棵分支执行树。它是控制面看到的完整 Agent 控制域：从 root Branch 开始，所有 child Branch、candidate、commit、accept、discard、resolver、policy 和 capability 都限定在这棵树内。

从用户视角看，AScout Agent 可以呈现为“一个完整的 Agent”。这是为了隐藏分支执行树的复杂性，让用户不必理解背后的多个可能世界、分支选择和失败清理。也就是说：

- 对外呈现：一个完整 Agent。
- 控制面内部：一棵 Speculative Tree。
- Runtime：多个受管执行节点和隔离状态。

因此，AScout Agent 的名字可以继续保留，但文档中必须明确：它不是 Agent process abstraction，而是 branch execution tree abstraction。

## 用户视图与 Root Agent

AScout Agent 对外应暴露为“像一个完整 Agent 一样可使用”的能力。用户主要看到和操控的是 Root Agent 视角，而不是内部任意 Branch 上的原始 Agent 进程。

这里的 Root Agent 是用户面对的主线视图，不是内部树结构的深度限制。也就是说：

- 普通用户默认控制 AScout Agent / Root Agent 视图。
- 控制面内部仍然维护完整 Speculative Tree。
- Speculative Tree 可以有任意深度，不限于 root 的 direct children。
- Branch-local Agent 或 policy 仍可以在当前 Branch 下继续创建 child Branch。
- Root Agent 视图的目的，是隐藏分支执行的复杂性，而不是限制控制面的分支能力。

因此，不能把 Root Agent 视图解释为“所有操作都必须以 root branch 为 parent”。更准确的理解是：用户默认通过 Root Agent 视图操控一个连续 Agent；控制面根据内部 Speculative Tree、policy 和当前控制语义决定如何路由、保留、接受或丢弃分支。

## 频繁用户操作与分支保留风险

当前需要记录一个重要忧虑：如果用户频繁操作和调整 Root Agent，而 控制面 把每次用户操作都当成全局失效事件，就可能导致大量提前执行的 Branch 被过早剪掉。

这会削弱 AScout 想实现的体验：系统虽然做了提前分支执行，但用户只要持续介入，已有 speculation 就不断失效，最终退化成普通顺序执行或只剩很浅的投机窗口。

因此，后续设计需要避免把 Root Agent 视图实现成简单的“用户一改，全部剪枝”。用户操作应被视为权威约束，但是否导致某个 Branch 丢弃、保留、挂起或修复，应该由后续的兼容性判断和 policy 决定。

本问题暂不展开具体机制。Root Agent 的变化包括什么、如何判断 Branch 是否仍兼容、如何修复或重新对齐 Branch、何时必须剪枝，均留待后续专题讨论。

## 控制面的控制器本质

AScout 成为更通用的底座，并不意味着控制面不再是控制器。

控制面的本质仍然是控制器：它负责解释 Agent 侧请求，维护 AScout Agent / Speculative Tree 的控制状态，选择策略，组织生命周期步骤，并决定何时调用 Runtime。更通用的目标不是削弱这个控制器角色，而是避免控制器只服务一种固定的投机执行流程。

要让控制面从“投机执行控制器”扩展为通用底座的控制侧，关键不只是把名词改得更中立，还包括：

- 核心语义只固定必要的不变量，例如树内身份、分支关系、单一 winner、capability、受控 effect 和 loser cleanup。
- policy 可以覆盖不同执行方式，例如无投机顺序执行、投机继续、多候选探索、验证器引导搜索、人工确认前预执行等。
- Agent-facing API 不暴露内部 AScoutAgent/Branch 身份，让上层可以把整棵树当成一个 Agent 使用。
- Runtime 只承担具体机制，不把某一种执行策略反向固化进控制面。
- 控制面内部的计划、调度、预算和事件处理应逐步从投机执行特例中抽出来，成为可服务多种分支执行模式的控制能力。

## 控制面的权力边界

控制面同时服务两个入口层次：

- 用户入口：用户像控制一个完整 Agent 一样控制 AScout Agent / Root Agent 视图。
- Agent / policy 入口：Agent 或 policy 可以在授权范围内控制当前 Branch 的生命周期。

这两个入口不应变成两套 AScout 语义。它们最终都应进入控制面维护的同一套控制语义，只是暴露的细节、权限和返回内容不同。

控制面后续可能需要拥有调度相关能力，例如何时启动 candidate、保留多少 Branch、哪些 Branch 暂停或优先执行、预算不足时如何降级等。但第一版暂不深入展开调度。即使未来加入调度权，具体调度选择也应由 policy 在控制面固定不变量允许的空间内规定，而不是由 Runtime 或产品入口私自决定。

Policy 应该可替换、可扩展，但不能绕过控制面的核心规则。控制面落实 AScout 的基础不变量，例如树内身份、分支关系、single-winner、capability、受控 effect 和 loser cleanup；policy 只能在这些规则内选择不同执行策略。

控制面应负责解释 Runtime 返回的 response 和 event。Runtime 返回的是底层机制结果；控制面需要把它们组合成 Agent、policy 或用户更容易理解的内容，并在必要时剔除敏感信息或隐藏底层实现细节。这一职责也属于上下文管理的一部分。

AScout 需要提供连续体验：用户看到的是一个持续工作的 Agent，而不是多个 Branch 的内部竞争状态。当前还没有确定具体提供方式，先作为待考虑内容记录。

控制面与外部 Runtime 的状态边界应形成完整闭环：控制面决定语义动作，经 Runtime Adapter 交给后端执行，再根据 Adapter 返回的结果和事件更新控制状态。控制面不能只更新 metadata 后假装后端已经完成隔离、状态转换或 cleanup；AScout 也不规定后端内部如何完成这些操作。

当前阶段不考虑多 AScout Agent 之间的协调。一个 AScout Agent 仍是一棵独立的分支执行树；多 Agent 协作即使未来需要，也不进入第一版核心设计。

第一版部署形态先按服务来理解：central 控制面作为 AScout Agent / Speculative Tree 的控制 owner，Branch-local ASC-Daemon 和上层入口通过稳定 API 进入它。未来是否提供 library / embedded 形态，可在不改变核心语义的前提下另行讨论。

## 第一版目标

第一版实现以 Agent Speculative Execution 为主要目标。

第一版可以优先解决这些问题：

- 一个 AScout Agent 如何拥有一棵 控制面 Speculative Tree。
- Agent 如何在当前 Branch 下创建 candidate。
- candidate 如何提交自己的结果或 continuation。
- resolver 如何在直接 child 中选择一个 winner。
- 控制面如何表达 commit、accept、discard 和 children/context 查询。
- 控制面如何把需要机制强制的动作交给 Runtime。

第一版不需要立刻支持所有未来模式。它只需要保证当前抽象不会阻止未来扩展。

## 长期目标

长期目标是让 AScout 支撑任意上层 Agent 分支执行策略，控制面负责承载相应控制语义。

这些策略可以包括但不限于：

- 普通顺序执行。
- Agent Speculative Execution。
- 多候选探索。
- verifier-guided search。
- debate / critique / repair。
- human approval 前的预执行。
- tool outcome pending 时的 continuation。
- 多 sub-agent 分支协作。

这些模式可以共享同一组底层控制语义：Branch、Speculative Tree、Outcome、Commit、Accept、Discard、Capability、Policy、Event 和 Runtime Adapter。

## 四侧关系

AScout 的完整一级关系不是 控制面 与 Runtime 两段直连，而是：

```text
Agent 接入侧
CLI / library / launcher / ASC-Daemon / Interposer
        |
        v
控制面
Entry API / Policy Engine / Assumption Manager / Scheduler /
Effect Coordinator / Ledger / Speculative Tree
        |
        v
Runtime Adapter
稳定 typed contract / 能力映射 / transport / 事件规范化
        |
        v
Runtime
执行 / 隔离 / 资源 / 原子状态转换 / cleanup
```

控制面拥有 AScout 的控制语义和编排权：它决定 Agent-facing 请求是什么意思、当前状态下是否合法、要展开成哪些步骤、何时创建分支、何时接受 winner、何时丢弃 loser，以及如何解释结果。

Runtime Adapter 承接控制面与具体 Runtime 之间的变化，对控制面保持 typed contract 稳定，并负责能力映射、transport、连接和事件规范化。Adapter 不决定 Agent 策略，也不拥有 Speculative Tree 或 Runtime mutable state。

Runtime 把控制意图落实到进程、工作区、输入、外部效果、隔离、状态传播和清理。它验证必要的身份、capability 和机制前置条件，但不决定 Agent 策略，也不拥有整体 Speculative Tree 语义。

## 当前设计约束

当前四侧 ownership 已确定，但 Runtime Adapter 与 Runtime 的完整 contract 还没有展开。因此不应过早固化 Runtime Adapter payload、transport、service discovery、session、event ordering、failure recovery 或 backend 细节。

在继续实现控制面时，应保持以下约束：

- 不把 Runtime backend 细节放进控制面语义。
- 不让 CLI 或 product integration 选择、创建或连接具体 Runtime。
- 不让 Agent-facing API 要求用户理解完整 Speculative Tree。
- 不把 Root Agent 视图解释为 Speculative Tree 的深度限制。
- 不把用户每次操作都默认解释为全局 Branch 失效事件。
- 不把 speculation 写死成唯一运行方式。
- 不为了追求通用化而过早改掉当前已经工作的投机执行术语。
- 不让 policy 绕过控制面固定不变量。
- 不让 Runtime response / event 直接泄漏为用户或 Agent 必须理解的底层细节。
- 不在第一版引入多 AScout Agent 协调语义。

## 待继续澄清

后续需要继续澄清：

- AScout Agent 的外部 API 应该暴露哪些能力，哪些只能留在控制面内部。
- policy 应该如何表达无投机、投机、多候选搜索和人工介入等不同模式。
- Root Agent 视图下，用户操作如何影响已有 Branch，以及如何避免频繁用户操作导致大量 Branch 过早剪枝。
- AScout 如何提供连续 Agent 体验，以及哪些输出、事件和状态应该对用户可见。
- 控制面的计划和事件模型如何从第一版投机执行扩展到更通用的分支执行。
- Runtime Adapter 的请求与事件、连接生命周期，以及 Runtime 的失败恢复如何定义。
- 调度权如何由 policy 表达，并如何受控制面固定不变量约束。
- 当前 candidate / resolver / commit / accept 等术语在长期通用化后是否需要保留、弱化或补充中立术语。
