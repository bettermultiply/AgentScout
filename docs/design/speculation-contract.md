# Speculation Contract 设计

本文定义 AScout 的 Speculation Contract。Contract 是 AScout 在 Policy、控制面、Runtime Adapter 与 Runtime 之间建立的统一语义约束。它把投机策略对执行的要求，连接到控制面能够编排的操作，以及 Runtime 能够实际承载的执行状态。

Contract 不是一份要求各方自行遵守的说明。它要通过固定的 Policy 接口、控制面操作入口、Runtime Adapter 接入方式、状态所有权和运行期结果，限制一条投机操作能够经过的路径。Contract 规定必须成立的语义，Mechanism 则是这些语义在 AScout 中的固定实现。

## 1. Contract 的定位

`problems-and-goals.md` 将策略与运行时缺少协调列为 AScout 要解决的问题。现有投机系统通常在策略内部规定“哪些操作可以投机、需要什么隔离、失败后如何处理”，运行系统则提供自己的隔离或执行接口。两边之间缺少共同的语义，策略只能依赖某一个特定运行环境，运行环境也无法被其他策略复用。

AScout 需要回答的不是“Policy 是否愿意遵守 Runtime 的接口”，而是下面这个更具体的问题：

> 一个 Policy 想要进行投机时，控制面允许它表达什么，Runtime 必须能够执行什么，以及从策略意图到实际执行之间哪些语义不能改变？

Contract 给出这组问题的固定答案。它同时规定四件事：Policy 如何表达意图，控制面如何把意图变成操作，Runtime Adapter 如何连接操作与后端，以及 Runtime 必须提供哪些基本运行语义。它们不是四份互相独立的说明，而是同一条执行路径上的不同落点。

```text
Policy 决策
    ↓ 只能通过 Contract 规定的决策接口表达
控制面机制
    ↓ 只能形成 Contract 规定的控制操作
Runtime Adapter
    ↓ 只能映射为 Contract 规定的 Runtime 操作和事件
Runtime
    ↓ 承载并返回实际执行状态
控制面状态更新
```

Contract 与 Mechanism 是一体两面。Contract 规定这条路径必须保持的接口、状态、不变量和失败语义；Mechanism 通过固定的接口、状态 owner、操作顺序和运行期校验使它们成立。Mechanism 是抽象责任词，不是一个需要额外创建的组件。控制面中的固定组件、Runtime Adapter 和 Runtime 共同承担这部分实现。

## 2. Contract 的统一语义

Contract 的中心不是某个 Policy 或某个 Runtime 的配置，而是从策略意图到实际操作之间必须保持的语义关系。以一个 candidate Branch 为例，Policy 可以判断“应该创建一个候选”，但它不能自行创建进程、分配 workspace 或登记一个控制状态。控制面必须先把这个判断转换成受管的 Branch 操作，Runtime Adapter 再把这个操作转换成具体 Runtime 能够执行的请求，Runtime 返回的结果最终决定控制面是否可以更新自己的状态。

同样地，Policy 可以判断某个 candidate 应被丢弃，但“丢弃”不能只是一条控制面 metadata。对应的执行、资源和受管效果也必须按照 discard 语义处理，Runtime 的真实结果返回后，控制面才能把 candidate 视为已经完成丢弃。

因此，Contract 的固定语义包括以下相互联系的内容：

- Policy 只能通过规定的 Policy Decision 表达预测、准入、验证、效果和处置意图；
- 控制面必须维护 Branch、Proposal、`AssumptionSet`、single-winner、隔离、obligation 和 cleanup 等固定语义；
- 跨控制面和 Runtime 的操作必须使用有明确含义的 command 和 event；
- Runtime 必须提供 Branch 执行所需的基本操作和状态语义；
- 任何一层不能表示、执行或完成某项操作时，必须返回明确结果，不能静默改变原意。

这些语义共同构成 Contract。它们的实现位置可以不同，但同一操作在不同位置不能拥有互相矛盾的含义。

### 2.1 Policy 表达契约

Policy 是可替换的部分，但它只能通过 AScout 的决策面表达策略。当前决策面为：

```text
admit、extent、predict、launch、effect、match、dispose
```

例如，`predict` 返回 Proposal，`admit` 返回是否开启投机的判断，`dispose` 返回对候选工作的处置意图。这些结果是控制面可以检查和解释的数据，不是对 Runtime 的直接命令。Policy 不会因为返回了“创建候选”或“丢弃候选”就获得创建进程、修改 Branch 状态或提交外部效果的权限。

Policy Engine 需要在决策进入执行路径前检查它是否仍然是一个合法的 Policy 决策。检查的对象包括决策的输入输出、决策之间的组合、所依赖的验证依据，以及 Policy 对效果、资源和 Fidelity 作出的声明。若 Policy 的目标超出了 AScout 固定语义，或者当前 Runtime 无法提供实现该目标所需的能力，控制面必须拒绝该决策或按照已经定义的语义报告降级。

Policy 可以根据资源和运行条件选择不投机，也可以选择不同的候选数量、推进边界和处置方式。它不能借此改变 Branch 生命周期、single-winner、未验证信息的可见性、效果提交条件或资源所有权。这些是 Policy 决策运行的固定上下文。

### 2.2 控制面语义契约

控制面负责把 Policy 的数据决策变成受管的控制操作。它必须能够在一个统一的状态模型中表达候选的创建、执行、验证、接受、取消、清理和结果交付，并且让每次状态变化都能找到唯一的 owner。

例如，Policy 返回一个 Proposal 后，控制面需要同时建立它与 candidate Branch 的关系，登记该 Branch 尚未确认的 Proposal 依赖，并向 Runtime 请求受管的执行单元。之后 Authority 或 validator 提供验证依据，控制面依据匹配结果决定是否接受 candidate；它不能跳过依赖状态，直接把 candidate 当成 Authority。

控制面还要保持控制状态和实际执行状态之间的关系。它发出 discard 请求并不等于 discard 已经完成；只有收到 Runtime 对应的结果或事件，控制面才可以结束相应的控制状态。相反，如果 Runtime 报告执行仍在进行、只完成了一部分或清理失败，控制面必须保留这些事实，并进入相应的失败处理路径。

这部分固定语义由控制面已有的机制责任共同完成：Assumption Manager 维护 Proposal 依赖和确认/否定，Scheduler 处理准入、预算和运行资源协调，Effect Coordinator 处理效果计划和效果边界，Ledger 保存运行事实和契约证据。它们通过各自的固定入口协作，不通过互相修改内部状态来完成操作。

### 2.3 Runtime 操作契约

投机执行最终要改变真实的执行环境：它需要创建和停止进程，管理 workspace、内存和其他资源，隔离不同 Branch 的状态，并处理可能影响 Branch 外部的效果。这些状态不由 Policy 或控制面 metadata 承载，而由 Runtime 承载。

因此，Runtime 接入 AScout 必须提供一组基本操作语义。至少包括受管执行的创建、Branch identity 和操作授权的识别、执行控制、取消、清理、资源归属，以及结果和生命周期事件的返回。对于效果和 obligation，Runtime 也必须能够按照 AScout 的效果处理语义提供对应的状态转换。

Runtime 的后端实现可以不同。例如，一个 Runtime 可以使用进程组和 overlay workspace，另一个 Runtime 可以使用容器或虚拟机。它们可以用不同的原语完成同一个操作，但返回给控制面的语义必须一致：控制面请求的是“创建一个受管 candidate Branch”，而不是某个后端私有的容器或进程编号。

Runtime Adapter 是这项接入的具体实现。它将后端原语映射为 AScout 的操作和事件，并报告当前 Runtime 能够提供的附加能力及其限制，例如隔离粒度、暂停和恢复、抢占、效果暂存、资源上限和压力信号。能力报告是事实输入；它不能改变 AScout 对 Branch、生命周期、single-winner、obligation 和 cleanup 的基本要求。

### 2.4 失败语义

跨层操作可能在不同位置失败。Policy 可能无法产生有效 Proposal，控制面可能发现当前状态不允许某项转换，Adapter 可能无法把语义操作映射到后端，Runtime 可能在执行中遇到资源或清理错误。Contract 要求这些情况沿同一条语义路径返回，而不是由某一层用成功值掩盖。

失败结果需要说明操作处于什么状态以及接下来可以采取什么处理。例如，候选可以被拒绝、取消、重新执行、清理或补偿；正在执行的操作可以报告部分完成；能力不足可以导致当前投机被拒绝或明确降级。具体选择取决于对应的生命周期、效果和 Policy 语义，但“没有完成却报告成功”不属于合法结果。

## 3. Mechanism 如何落实 Contract

Contract 只有落实到固定执行点才具有约束力。AScout 不要求 Policy、Adapter 或 Runtime 通过自觉遵守一份文档来获得正确性，而是让它们只能通过受限制的接口进入执行路径。Mechanism 就是这套接口、状态 owner、操作编排和运行期检查的组合。

### 3.1 Policy Engine 的校验路径

Policy Engine 是可替换 Policy 进入控制面的固定入口。Policy Engine 不把 Policy 返回的内容直接交给 Runtime，而是先把它解释为对应的 Policy Decision，并检查它能否在当前 Contract 下继续执行。

例如，Policy 返回“对一个需要用户输入的 Edge 使用 `Absent` launch，并以 validator 结果作为依据”。Policy Engine 需要检查这个选择对应的 Fidelity 和验证语义，而不是把它当成普通的 Authority 结果。又例如，Policy 要求一个需要隔离和清理能力的候选执行，Policy Engine 需要结合 Runtime Adapter 已报告的能力判断该执行是否有合法路径。

检查通过后，Policy 的结果仍然只是策略意图。Policy Engine 将它交给控制面固定机制，由后者检查 Branch 身份、当前生命周期、未决依赖、预算和效果条件，再形成下一步操作。这样，Policy 无法通过返回一个特殊值绕过控制面状态转换。

### 3.2 控制面固定入口

控制面机制的约束来自“谁拥有状态，谁提供修改入口”。Assumption Manager 是 Proposal 依赖和确认/否定状态的修改入口；Scheduler 是准入、预算和调度状态的修改入口；Effect Coordinator 是效果计划和效果协调视图的修改入口。其他组件拿到的是数据或操作结果，而不是这些状态的任意写权限。

当控制面需要执行一个动作时，它必须先形成有明确语义的操作。例如，`discard` 包含候选身份、当前控制状态、取消范围以及后续清理要求；它不是一个可以由任意组件直接写入的布尔字段。控制面固定入口检查操作前置条件，并将操作交给 Runtime Adapter。并发的 `accept` 与 `discard`、父候选否定后的后代取消等情况，也必须在这些入口中按照既定状态语义处理。

这种结构使 Mechanism 不需要另一个 Manager 来监督自己。可替换的 Policy 被固定入口限制，控制状态由唯一 owner 修改，跨侧动作由固定 operation/event 连接。Mechanism 自身的实现正确性仍需要通过代码审查、静态不变量和 conformance 验证确认；但日常运行不依赖 Policy 或 Adapter 自己声称“我已经遵守了 Contract”。

### 3.3 Runtime Adapter 的接入路径

Runtime Adapter 不是一个可以重新解释 AScout 语义的中间层。它需要完成两项具体工作：一是把 Runtime 的实际后端能力翻译成控制面可使用的能力事实，二是把控制面操作翻译成 Runtime 能执行的调用，并把调用结果翻译回规范化事件。

接入检查首先确认 Runtime 能否提供 AScout 的基本操作语义。无法创建受管执行、无法区分 Branch 身份、无法停止和清理 loser，或者无法返回控制面需要的结果和事件时，该 Adapter 就没有完整接入路径。对于基本语义之上的能力，Adapter 需要报告支持范围和限制，使 Policy Engine 能够判断某个具体策略是否有可行的执行路径。

运行期间，Adapter 还要保持操作的身份、前置条件、原子性和失败结果。例如，控制面请求 discard 时，Adapter 不能把它降成“删除一条记录”；控制面请求 accept 时，Adapter 不能只返回“请求已发送”而不报告 Runtime 是否完成所需状态转换。后端无法完成对应语义时，Adapter 应返回明确失败或部分完成结果。

### 3.4 Runtime 的承载路径

Runtime 接收的是经过控制面和 Adapter 传递的 AScout 操作，并在自己的执行环境中承载对应状态。它管理执行实体、Branch-local 状态、资源归属、效果 obligation 和清理过程，然后通过事件告诉控制面实际发生了什么。

以 discard 为例，控制面决定候选不再保留，形成 discard 操作；Adapter 把候选身份和操作要求映射给 Runtime；Runtime 找到该身份对应的执行实体，停止仍在运行的工作，处理资源和受管效果，完成清理或报告清理失败。控制面收到结果后，才将控制状态推进到相应终态。

Runtime 的作用由此直接来自 AScout 的目标：如果投机分支只存在于控制面的一棵树中，AScout 就无法保证分支的真实进程、资源、workspace 和受管效果会随树的裁决而变化。Runtime 让这些变化有实际承载者，也让 Contract 中关于执行、隔离、取消和清理的语义能够落到真实环境。

## 4. 完整控制闭环

一次需要 Runtime 参与的投机操作经过以下闭环：

```text
Policy 产生决策
    → Policy Engine 校验决策
    → 控制面机制检查状态并形成语义操作
    → Runtime Adapter 检查并映射操作
    → Runtime 执行操作并返回结果或事件
    → Runtime Adapter 规范化结果
    → 控制面根据结果更新控制状态
```

以创建 candidate 为例，Policy 的 `predict` 产生 Proposal，`admit` 判断当前机会适合投机。Policy Engine 检查这些结果是否与当前 Edge、验证方式、效果要求和 Runtime 能力相容。检查通过后，控制面建立 Proposal 依赖和 candidate Branch 的控制状态，并向 Runtime Adapter 请求创建受管执行。Adapter 将创建请求映射到后端，Runtime 创建执行实体、隔离环境并返回 Branch identity 和初始事件。控制面只有在收到成功结果后，才把 candidate 记录为已创建；若 Runtime 只能部分建立边界或创建失败，控制面必须保留失败事实并执行对应清理，而不能继续把它当作一个正常候选。

以接受和丢弃为例，Authority 或 validator 提供依据后，控制面通过 `match` 和 `dispose` 得到处置意图。处置意图经过固定状态入口检查后，转换成 `accept` 或 `discard` 操作。Runtime 处理相应的执行、资源、效果和清理，Adapter 返回结果，控制面再更新 Branch 和 Proposal 的状态。这样，“Policy 选择了一个候选”和“候选已经成为可继续使用的执行”之间有清楚的操作和事件边界。

## 5. 运行期校验与证据

接入时的检查只能确认 Policy 的要求与 Runtime Adapter 报告的能力在形式上相容。运行期间还需要根据实际操作和事件确认 Contract 是否真的被兑现。控制面与 Runtime 至少需要产生以下事实：Policy 决策及其上下文、Branch 和 Proposal 的状态变化、验证依据、效果和 obligation 的处理结果、资源使用、取消与清理结果，以及 Adapter 映射或 Runtime 执行失败。

Ledger 记录这些事实，并把它们用于状态对齐、问题定位、shadow/replay 和 Fidelity 审计。Ledger 不从记录中重新决定 Branch 的生命周期，也不代替 Runtime 持有执行状态；它提供的是判断“规定的操作是否发生、结果是否返回、声明的目标是否达到”所需要的证据。

例如，控制面记录了 discard 请求，只能证明控制面形成了该意图；只有对应的 Runtime 结果和清理事件，才能证明受管执行已经按照 discard 语义结束。类似地，Policy 声明某种 Fidelity，只能通过 Authority、validator、效果处理和实际结果的运行事实进行审计。若这些事实显示声明无法持续达到，控制面应按照 Contract 的失败语义停止相关投机、清理受影响执行，或明确报告降级。
