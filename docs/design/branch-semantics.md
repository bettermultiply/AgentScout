# Branch 语义设计

本文定义 AScout 的 Branch 与 Speculative Tree 目标语义：Branch 结构、生命周期、依赖、状态可见性、信息传播与 resolution。

本文只记录已确定的目标语义。仓库当前没有实现；开发决策、路线图与验收状态记录在开发文档中。

默认 Agent 完全不感知 Branch，不需要读取 Branch context、Proposal、children、capability 或 lifecycle state，也不需要主动参与 Branch 生命周期。当前不支持显式 Agent 感知，也不需要支持；Branch 的创建、推进、验证、接受、丢弃、重新挂载和清理不依赖 Agent 的主动配合。本文第 6 节单独保留显式 Agent 感知的未来设计，不改变当前边界。

## 1. Branch 与 Speculative Tree

Branch 是 Speculative Tree 中的子树，也是可继续运行和继续分叉的计算延续。每个 Branch 的根节点称为 Branch Root；Branch Root 直接拥有自己的 local state，并可包含零个或多个 child Branch Root。

一棵 Speculative Tree 可以直观地表示为：

```text
Speculative Tree T
└── Branch Root r
    ├── local state of r
    ├── Branch Root a
    │   ├── local state of a
    │   └── Branch Root c
    │       └── local state of c
    └── Branch Root b
        └── local state of b
```

形式化地，令 `T = (R, parent)` 为一棵以 `r` 为根的有根树，其中 `R` 是 Branch Root 集合，`parent` 描述根之间的派生关系。对任意根 `r_n ∈ R`：

```text
BranchRoot(r_n) = r_n 及其直接拥有的 local state
Branch(r_n)     = r_n 及其全部 descendants 构成的子树
children(r_n)   = r_n 的直接 child Branch Root 集合
```

因此，`local state of r_n` 不包含 child Branch Root；`Branch r_n` 则包含 `r_n` 以及它的全部 descendants。Speculative Tree 从 root Authority Instance 对应的 root Branch Root 开始；状态本身不是另一棵独立的树。

一个 AScoutAgent 恰好拥有一棵控制面 Speculative Tree。Branch Root identity、candidate handle、capability、commit/discard 和 resolution 都只在这棵 tree 内有意义；同一部署承载多个 AScoutAgent 不形成更高层的 Branch 关系。

## 2. Branch 状态、身份与依赖

Branch Root 在 Speculative Tree 中存在，与对应执行活动当前是否 running、waiting 或 suspended 是两套正交状态。一个 live Branch Root 可以暂时没有正在占用 CPU 的执行活动，但仍保留 parent/child relationship、Branch 状态和 capabilities，并可在事件到达后恢复。AScout 不为此引入 `lineage` 实体或专门术语。

Branch Root 可以关联一个或多个承载其执行活动的 Instance。Instance 绑定到一个 Branch Root，并承载该 Branch Root 所属 Branch 的执行活动；逻辑 Branch 可以在 Instance 建立前先存在并推进。

每个 candidate Branch 携带其路径上的未决 Proposal ID 集合：

```text
AssumptionSet = frozenset[ProposalId]
```

`AssumptionSet == ∅` 表示当前工作单元是权威工作；非空表示它仍携带未验证的 Proposal 依赖。Proposal 被确认时，从依赖它的工作单元中移除其 ID；集合变空的工作单元才具备提交条件。非空 `AssumptionSet` 产生的数据不能直接进入权威上下文。

Branch 的 capabilities 规定控制面可以代表哪些主体执行相关生命周期操作，以及哪些受控主体可以获得未来的协作权限。当前生命周期 authority 由控制面依据 Policy、task 配置和 Branch 状态维护；Evaluator/resolver 是控制面编排中的角色，不是另一种世界实体。未来显式 Agent 感知时，Agent-facing capability 也只能表达受控意图，不能替代控制面固定的生命周期、Policy 校验和执行承载约束。

## 3. 执行基础与承载

Child 从 Branch barrier 上的 parent execution base 开始继续运行。Branch barrier 首先登记逻辑 Branch，不要求立即建立独立执行承载。当当前 Policy、Contract 或状态处理要求独立承载时，控制面通过可用的执行承载请求相应环境；承载结果决定该 Branch 能否沿该路径继续推进。环境建立和执行活动的具体承载方式见 [Runtime 语义整理](../development/default-runtime-semantics.md)。

当 Branch 的执行承载从 execution base 分离后，Branch 按自身的状态和依赖继续演化；parent 后续状态变化是否对 child 可见，遵循相应状态类别的规则。需要同步的真实步骤结果通过显式 outcome event 交付。

Branch barrier 建立期间，控制面必须阻止未登记的 Branch 活动越过边界；需要执行承载时，控制面只能依据承载机制返回的实际建立或失败结果更新 Branch 视图，不能把预期状态当作已对齐。逻辑 Branch 登记失败不得留下半创建 Branch。若 Policy 或资源条件允许少于请求数量的 candidates，控制面必须显式记录实际登记集合。不同接入形式必须进入同一 barrier/lifecycle 语义。

## 4. Branch 生命周期

### 4.1 创建与 Branch barrier

AScout 使用 outcome-guided continuation branching 支持跨步骤投机。在 pending-result boundary，控制面从共同 execution base 登记多个逻辑 Branch；candidate 接收不同 Proposal 并提前执行后续步骤。Branch 是活的计算延续，不称为 Snapshot，也不在目标模型中引入 Snapshot 实体。

AScout 不定义创建 children 后长期冻结 parent 的 Parent barrier。Branch barrier 是真实 operation 开始前的短暂且原子的执行边界，用于建立共同 execution base、OutcomeSlot、candidate handles 和 capabilities，并让 parent/candidates 从同一个控制操作返回点按不同 role 恢复。

### 4.2 运行、等待与暂停

在 speculation window 内，控制面根据 Policy 和当前执行事实协调 parent 的权威 operation、outcome 发布、candidate 推进与 resolution，不让 parent 与 candidates 同时推进同一份后续业务。若没有可接受 candidate，控制面安排 parent 从真实 outcome 继续；若接受 candidate，则执行 single-winner replacement，不执行 parent/candidate state merge。

Parent execution 可以在等待 outcome、提交条件或 resolution 事实时暂停，并在事件到达后恢复；具体调度由控制面根据 Policy 和执行承载能力编排。Child 的生命周期不由 parent execution 是否持续占用 CPU 决定。

Parent execution 意外丢失时，children 不会因此自动成为 winner。控制面根据可用的执行事实、Policy 和恢复状态决定等待、恢复、取消或清理；在无法证明生命周期状态仍然可控时，控制面必须 fail closed，不能继续推进该 speculation group 的受管活动。具体恢复宽限期与最终清理策略属于故障恢复设计。

### 4.3 Commit

控制面可以将 candidate 的完整 Branch 标记为达到提交条件，即使其中仍有活跃 descendants。Branch 可以在 outcome 尚未到达时进入 conditional commit 状态；未验证的 Proposal 作为 unresolved obligation 随 subtree 保留，在 obligation 验证前不能 Accept。这里的 commit 是控制面维护的生命周期状态和固定状态入口，不要求默认 Agent 主动发起请求。

Accept 只针对已经达到控制面提交条件的 candidate。Candidate 的 commit summary/evidence 由控制面和接入侧根据执行事实形成并通过受控通道封存；已经披露的信息不能通过 discard 撤销。

### 4.4 Resolution barrier

`Resolution barrier` 是控制面依据 Policy 的 `match`/`dispose` 意图执行 Accept、fallback、discard 或其他 resolution 时的短暂且原子的执行边界。它暂停受影响 executions，验证 obligations、Proposal 和提交条件，转换 Speculative Tree 与状态所有权，失效并清理 losers，随后恢复 winner subtree 或 parent。

Proposal 必须声明与真实 outcome 的 compatibility 条件，可以是 exact payload、只依赖受保证字段的 predicate，或受控 virtual-value mapping。实际 payload 无法按声明条件兼容时，控制面必须将 candidate 导向 discard 或重新执行；AScout 不修补已经基于错误具体值完成的任意计算。

### 4.5 Accept 与 winner continuation

Accept 后 candidate 保留自己的 Branch identity，并替换 parent 在 Speculative Tree 中的位置。若 parent 是 root，candidate 成为新的 root；否则 candidate 连接到原 parent 的 parent。旧 parent 和 candidate 的 sibling subtrees 失效，candidate 原有 descendants 及其内部关系保持不变。

Candidate 按各状态类别规则已经可见的 parent state、已提交生效的 external effect 和已验证 outcome 必须随 winner 延续。parent 在 Branch 创建后的任意进程内存、Agent context 或评测 scratch 不要求传播，也不执行 parent/candidate state merge。

Promote 不以 candidate 会话或当前模型请求结束为前提；已满足提交、依赖和效果条件的在飞 candidate 可以在保留执行进展的情况下转为权威延续。用户端维持同一 AScoutAgent 视图，只更新该视图对应的权威执行及受控输入输出路由；不能因用户会话切换而取消或重启 winner。此保证不取消 resolution barrier，也不豁免旧 parent 和其他 loser 的停止、效果收敛与清理责任。

### 4.6 Discard、失败与清理

控制面可以依据 `dispose`、资源压力、Proposal 前提失效、任务结束或其他固定取消条件，在 live direct child 尚未达到提交条件时 discard 整个 child subtree。Discard 不要求 candidate ready，也不自动把 child partial state 或结果反馈给 parent。Commit 与 discard 的控制语义由控制面线性化；涉及执行承载的停止和清理经相应承载机制落实。

对尚未被接受的投机 Branch，失败收敛为“该 Branch 在 Speculative Tree 中从未被创建”：不重放该操作，不保留它的部分结果作为成功。被接受的 Branch 已完成使命，不适用这一语义。

Candidate 未被接受不表示其不可行。受控 metadata 应区分 Proposal invalidated、evaluated but rejected、policy/resource cancelled、execution failed 和 superseded 等 disposition。

### 4.7 后代 Branch 重新挂载

当控制面确认父 Proposal，而后代 Branch 仍携带其他未决 Proposal 时，父 Branch 结束其投机身份，其执行延续成为权威路径；后代 Branch 保留原有 Branch identity、执行承载和执行进展，仅更新 Speculative Tree 中的父级引用并重新挂到该权威执行延续之下。此过程不创建新的 Branch，也不改变后代已有的执行进展。

后代 Branch 在自身未决依赖清空前，不能把结果或效果纳入权威路径。若父 Proposal 被否定，后代 Branch 不能重新挂载，依赖它们的 Branch 分别进入取消与清理流程。后代的部分进展只有在父 Branch 进入 Salvage 时，才能作为父 Branch salvage 的输入，不能脱离父 Branch 独立 salvage。

## 5. 状态可见性与信息传播

### 5.1 可见执行环境

对任意 Branch B，其可见执行环境由以下部分共同组成：

- B 当前执行承载中的 local state（若已建立）；
- B 在 branch barrier 上获得的 execution base，或在按需建立执行承载时实际使用的 execution base；
- 按文件系统、输入、outcome event 等类别规则对 B 可见的 ancestor state；
- root Branch 所在 Speculative Tree 之外的全局计算环境。

不同状态类别不共享一条笼统的 ancestor visibility 规则。Branch 从 execution base 继续演化；在承载尚未分离前，Branch 可以使用受控的共享执行视图。文件系统可以动态解析 ancestor writes，输入可以继续 fan-out，真实步骤结果通过 outcome event 交付。Branch 状态的可见范围由相应状态类别规则决定。

对于具有覆盖语义的 branch-local state，同一对象在可见范围内按最新修改解析：

- ancestor 在 child 写入之后产生的更新可再次覆盖 child；
- child 的再一次更新也可以覆盖 ancestor 的旧更新；
- 不属于当前 Branch 可见范围的更新不进入该范围。

进程内存、Agent context、输入和全局服务不使用这一覆盖规则；输入遵循独立读取位置的传播语义，outcome 使用显式验证与交付，全局服务的外部效果遵循 obligation 语义。相关执行承载和环境保证见 [Runtime 语义整理](../development/default-runtime-semantics.md)。

### 5.2 Ancestor 与 Branch 间通信

Branch 对 ancestor state、输入和 outcome 的可见性分别遵循各自规则。Descendant 对某类 ancestor state 的可见性不代表 parent 可以反向获得 descendant 的未提交状态。Branch 之间传递信息必须经过显式的受控通道；通过受控通道传递的信息不改变 Branch 的结构关系，也不会使接收方获得对发送方未提交 Branch 状态的直接访问能力。

### 5.3 Outcome 与效果边界

Agent 接入侧负责从产品执行接缝中识别并规范化 operation 及其 `pending-result boundary`。涉及 shared 或 external operation 的处理由 [效果管理设计](effect-management.md) 协调；相关结果和事件通过受控的 Outcome 通道传播。`PublishOutcome` 是真实 operation 完成后把结果交付到 OutcomeSlot 的内部语义操作，不作为独立 Agent-facing command。

### 5.4 输入传播

进入某个 Branch 的新输入直接对该 Branch 及其全部 descendants 可见。每个 Branch 独立维护自己的输入读取位置，因此一个 Branch 读取输入不会使 sibling 或 descendant 失去同一输入。

创建 child Branch 时，child 从 parent 当前的输入状态开始，此后拥有独立读取位置。Root Branch 运行期间收到的新输入会直接传播给当时仍在运行的全部 descendants；某个 child 的私有输入只传播给自己的 descendants，不进入 sibling subtree。

输入是否已经读取或处理不限制 subtree 提交。Winner subtree 保留自己的输入状态继续运行，loser subtrees 的输入状态随其一起丢弃。

输入传播不表示旧前提继续有效。用户阻止当前权威路径上的操作时，依赖该操作继续发生或依赖其原结果的 Proposal 前提已经改变。当前阶段不尝试兼容、修复或重新对齐这类 Branch；控制面将所有依赖已变更前提的 Branch 及其 subtree 视为不兼容并剪枝，经相应机制取消和清理。一般地，任何已识别的 Proposal 前提变更都使用同一保守规则；未受该前提影响的 Branch 不因无关输入而丢弃。

## 6. 显式 Agent 感知设计（当前不支持）

本节保留显式 Agent 感知的设计位置。它描述未来若允许协作型 Agent 读取或参与 Branch 生命周期时需要满足的语义，不是当前 Agent 接入能力，也不是 Branch 正确性的前提。

若未来开放显式感知，Agent 可以获得三类生命周期能力：创建 candidate Branch、提交一个 candidate subtree，以及裁决收到的 subtree。Candidate Agent 可以决定何时提交；持有 resolver capability 的授权主体可以决定是否接受、等待或丢弃 candidate subtree。Runtime 不替 Agent 或其 delegate 固化这些运行策略。

未来的 routine Agent-facing control surface 可以保留两类查询和五类 lifecycle/activity operation slots：查询自身 `context`、查询 direct `children`、`branch`、`commit`、`accept`、`discard` 和等待 event。这里的 operation slots 不表示当前 Agent product 已经实际暴露接口。`branch` 表示从当前 Branch 创建一个或多个 direct child；在投机场景中这些 child 才按角色称为 candidate。完整语义中 `branch` 可以携带 candidate 数量、Proposal、outcome slot 和可选 resolver override，未指定 resolver 时使用 AScoutAgent 配置中的默认 resolver。具体参数、返回值、阻塞方式、event model、命令拼写以及 Agent-facing 组件和接入形式，见 [Agent 接入侧设计](agent-integration.md)，并须继续受本文件规定的 Branch lifecycle 语义约束。

显式 Agent 感知不向端用户扩展独立用户会话。端用户始终通过 AScoutAgent 视图直接操作当前有效的权威执行主线，只能看到该路径产生的内容、状态与交互；携带未决 Proposal 的投机 Branch 不能直接向端用户输出或提问。Branch 接受、丢弃、重新挂载和清理仍由 Policy、授权 resolver 与控制面机制在内部完成，Accept 改变权威路径后，用户继续操作更新后的有效主线。

未来的 Agent-facing context 不要求暴露 Runtime 内部的裸 Branch ID。若获得相应 capability，Agent 可以查询当前 depth、lifecycle state、Proposal 和 capabilities，也可以获取 direct children 的受控 metadata view；depth 是 Accept 后树位置的派生信息，不能代替稳定 identity 或用于推导权限。Direct-child 操作使用 session-scoped opaque candidate handle，不能借此读取不属于授权视图范围的 Branch 状态。

当前不支持上述显式 Agent 感知能力，默认 Agent 也不需要支持它们。Branch 的创建、推进、验证、接受、丢弃、重新挂载和清理继续由 AScout 的控制面与接入机制在 Agent 不知情的情况下完成。

## 相关文档

- [架构](../architecture.md)：系统结构（四侧、所有权、调用方向）
- [设计模型](../design-model.md)：抽象分层与跨领域约束
- [效果管理设计](effect-management.md)：shared/external operation 与 obligation
- [Agent 接入侧设计](agent-integration.md)：operation 识别、规范化和 Agent 接入组件
- [Runtime 语义整理](../development/default-runtime-semantics.md)：执行承载与环境能力整理
- [设计决策](decisions.md)：Branch 生命周期相关决策速查
