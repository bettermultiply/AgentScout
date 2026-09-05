# Branch 语义设计

本文定义 AScout 的 Branch 与 Speculative Tree 目标语义：Branch 结构、隔离与通信、可见执行环境、输入传播，以及 Agent 感知与 Workspace/Command 粒度。

本文只记录已确定的目标语义。仓库当前没有实现；开发决策、路线图与验收状态记录在开发文档中。

## 1. Branch 与 Speculative Tree

Branch 是 Speculative Tree 中的子树，也是可继续运行和继续分叉的计算延续。每个 Branch 的根节点称为 Branch Node；Branch Node 直接拥有自己的 local state，并可包含零个或多个 child Branch Node。Speculative Tree 描述 Branch Node 之间的派生关系，并从 root Authority Instance 对应的 root Branch Node 开始；状态本身不是另一棵独立的树。

一个 AScoutAgent 恰好拥有一棵控制面 Speculative Tree。Branch Node identity、candidate handle、capability、commit/discard 和 resolution 都只在这棵 tree 内有意义；AScout 不定义或允许跨 tree 引用、授权和 lifecycle operation。同一部署可以承载多个 AScoutAgent，但它们在 AScout 语义上彼此无关，部署复用不会形成更高层的 Branch 关系。

Branch Node 在 Speculative Tree 中存在，与对应 Instance 当前是否 running、waiting 或 suspended 是两套正交状态。一个 live Branch Node 可以暂时没有正在占用 CPU 的 Agent execution，但仍保留 parent/child relationship、状态所有权和 capabilities，并可在事件到达后恢复。AScout 不为此引入 `lineage` 实体或专门术语。

`local state of A` 只表示 Branch Node A 直接拥有的状态层，不包含 child Branch Node。`Branch A` 则表示以 A 为根的整个子树，包括 A 及其全部 descendants。Child 从 branch barrier 上的 parent execution base 开始继续运行；Branch 后的 ancestor 可见性按状态类别分别定义，siblings 互不可见。

当 Branch Node 需要建立 Instance 时，Agent 接入侧负责管理完整的 Agent 进程树，包括 Agent 主进程及其 sub-agent；Runtime 负责提供承载它的隔离执行环境。接入侧必须保证分支操作覆盖继续执行所需的 Agent 执行实体，而不是只处理调用 branch 接口的单个线程或进程。尚未建立独立环境的逻辑 Branch 可以由受控接入侧继续推进，不要求立即拥有独立进程树。

## 2. Branch 绑定、隔离与通信

每个已建立的 Agent 执行实体由一个 Instance 承载，并绑定到对应的 Branch Node。同一 Branch Node 对应 Instance 内的进程共享该 Node 的可见执行环境，并能观察这个环境中由同一执行实体产生的修改。逻辑 Branch 在建立独立环境前可以使用受控的共享执行视图；该视图不能允许它产生未登记的共享或外部效果。

除文件系统、输入、outcome event 等各自规则明确纳入的 ancestor state 外，一个 Branch 不能直接读取另一个 Branch 的 local state、进程内存或内部 Agent 上下文。Descendant 对某类 ancestor state 的可见性不代表 parent 可以反向观察 descendant local state；siblings 也不能观察彼此的 local state。

Branch 之间传递信息必须使用显式的受控通道。通过受控通道传递的信息不改变 Branch 隔离边界，也不会使接收方获得对发送方 local state 的直接访问能力。

## 3. Agent 最小感知模型

AScout 向 Agent 提供三类生命周期能力：创建 candidate Branch、提交一个 candidate subtree，以及裁决收到的 subtree。Candidate Agent 决定何时提交；direct children 的 resolver capability 默认由 parent Agent 持有，parent 可以将其显式委托给控制面。持有 resolver capability 的授权主体决定是否接受、等待或丢弃 candidate subtree；runtime 不替 Agent 或其 delegate 固化这些运行策略。

这里的 Agent lifecycle capability 不向端用户暴露。端用户始终通过 AScoutAgent 视图直接操作当前有效的权威执行主线，只能看到该路径产生的内容、状态与交互。投机 Branch 不建立独立用户会话，也不能在携带未决 Proposal 时直接向用户输出或提问；Branch 接受、丢弃、重新挂载和清理由 Policy、授权 resolver 与控制面机制在内部完成，不要求端用户参与。Accept 改变权威路径后，用户继续操作更新后的有效主线，不需要选择或重绑定 Branch。

Routine Agent-facing control surface 暂时只保留两类查询和五类 lifecycle/activity operation placeholders：查询自身 `context`、查询 direct `children`、`branch`、`commit`、`accept`、`discard` 和等待 event。这里的 placeholders 表示目标语义中的 operation slots，不表示当前 Agent product 已经实际暴露完整的 Agent-facing 接口。`branch` 表示从当前 Branch 创建一个或多个 direct child；在投机场景中这些 child 才按角色称为 candidate。CLI 与 embedded function 调用先进入当前 Branch 隔离环境内的 ASC-Daemon；ASC-Daemon 隐藏并绑定当前 AScoutAgent/Branch context，再把 resolved typed request 交给 Agent Control Gateway。Agent Control Gateway 不从 caller context 重新解析，也不直接接触 Runtime，只把请求交给控制面；控制面再派发到对应 AScoutAgent 的 Branch-level operation。完整 Runtime 语义中 `branch` 可携带 candidate 数量、Proposal、outcome slot 和可选 resolver override，未指定 resolver 时使用 AScoutAgent 配置中的默认 resolver。当前不定义这些 control-surface operation 的完整参数、返回值、阻塞方式、event model 或具体命令拼写；它们仍受已经确定的 Branch lifecycle 语义约束，不能被解释为运行时 no-op。

`PublishOutcome` 是 Agent 接入侧必须完成的内部 semantic operation，但不作为独立 Agent-facing command，也不要求通用 Runtime 理解。Interposer 可以通过 operation wrapper、Hook、effect broker 或其他产品适配机制，在真实 operation 完成后自动把结果发布到 OutcomeSlot。Resolver capability 默认由 Parent Agent 持有；需要控制面代为裁决时，delegation 通过 AScoutAgent 创建或 Branch 创建配置表达，不作为日常 Agent command。

AScout 作为 Agent 外部的控制面管理完整 Agent execution。当前支持具备足够观察和拦截能力的特定 Agent product，并以 `Command + Skill` 作为基础接入 profile；Hook、embedded in-Agent adapter、launcher 和 library 可以作为不同接入形式，但具体产品能力和实现方式另行定义。当前不承诺对任意 Agent 的完全无修改接入。接入方式必须提供足够的 safe point、context/outcome 注入和 operation lifecycle 捕获能力，具体产品适配可以依赖其内部执行结构。

控制面、Agent 接入侧与外部 Runtime 之间必须具有明确的状态边界，并通过 Runtime Adapter 交换 typed API 数据。控制面拥有 speculation planning、Proposal 业务含义、调度、预算、evaluator policy、AScoutAgent 和 Speculative Tree 视图；Agent 接入侧拥有 Agent 执行实体的本地管理上下文、operation 归属、Branch context 和本地 Decision；Runtime 只拥有隔离执行环境及其环境级执行、资源和边界状态。AScout 不定义 Runtime 如何组织环境内部状态；控制面和接入侧只能根据 Adapter 返回的结果和事件更新各自视图，也不向后端传递 Agent prompt、模型格式或 candidate 质量等控制面含义。

Runtime Adapter 对控制面提供稳定、typed、可版本化的粗粒度 command/event contract，并负责把控制面选定的目标 Runtime 解析为相应连接、能力和协议映射。具体 Runtime 可以作为独立 Runtime Service 运行。各侧只能通过显式 contract 交互，不共享跨边界的可变内部状态。AScout 不规定 Runtime Adapter 到 Runtime 的统一 control transport 或 wire format；本地 daemon、独立进程、VM channel 或远程协议不能改变 typed operation/event semantics。CLI 不参与 Runtime 选择或连接。

每个完整 AScout execution 必须由 AScout 建立受管边界后创建，从 root Agent 启动时即纳入进程树、I/O 和资源归属管理。目标语义不限定 launcher frontend；显式 CLI、IDE、daemon 或 Agent platform 可以提供不同入口，但都必须先请求 AScout 创建受管 execution。事后 attach 不能宣称具有同等的完整 Branch 语义。

控制面是 Speculative Tree 视图与 Branch 关系的权威来源；Agent 接入侧是 Agent 执行实体及其 Branch context 的本地管理方；Runtime 是隔离环境及其环境级执行状态的机制来源。Agent 或其短命命令子进程只请求 branch；逻辑 Branch 的创建先由控制面登记，只有当接入侧识别出需要独立 Branch-local 状态的操作时，才经 Runtime Adapter 请求建立独立环境。建立后，接入侧必须在该环境中承载相应 Agent 执行活动，不能把环境建立误解为由 Runtime 接管 Agent 的投机管理。

AScout 不定义创建 children 后长期冻结 parent 的 Parent barrier。它定义两个短暂且原子的执行边界：`Branch barrier` 在真实 operation 开始前建立共同 execution base、OutcomeSlot、candidate handles 和 capabilities，并让 parent/candidates 从同一个控制操作返回点按不同 role 恢复；该边界只登记逻辑 Branch，不要求立即建立独立 Runtime 环境。接入侧识别出需要独立 Branch-local 状态的操作时，控制面再请求建立对应的环境边界，并在操作开始前让接入侧在 candidate 环境中承载相应执行；`Resolution barrier` 在 Accept 或 parent fallback 时暂停受影响 executions，原子验证 obligations、转换 Speculative Tree/状态所有权、清理 losers，再恢复 winner 或 parent。

Branch barrier 建立期间，控制面和接入侧必须阻止未登记的 Branch 活动越过边界；Runtime 必须强制环境、进程和资源边界，并返回环境操作的实际结果。逻辑 Branch 登记失败不得留下半创建 Branch；需要建立独立环境时，环境建立失败也不得把未完成的环境状态当作已对齐。若资源策略允许少于请求数量的 candidates，实际登记集合和环境建立集合都必须显式返回。Command 与 Hook 只是不同触发器，必须调用同一 barrier/lifecycle 语义。

AScout 使用 outcome-guided continuation branching 支持跨步骤投机。在 pending-result boundary，控制面先从共同 execution base 登记多个逻辑 Branch；candidate 可以在受控共享执行视图中接收不同 Proposal 并提前执行后续步骤，直到某项操作要求独立状态时，Runtime 才从相应 base 建立该 Branch 的独立环境。parent 取得真实 operation outcome，candidates 按各自路径继续。Branch 是活的计算延续，不称为 Snapshot，也不在目标模型中引入 Snapshot 实体。

一旦某个 Branch 建立了独立环境，接入侧负责的 Agent 执行实体以及环境中被要求保护的本地执行状态从建立环境时的 execution base 开始独立演化，Runtime 负责强制环境边界。Branch 创建后，parent 的受保护本地状态修改不会透明传播给 children；需要同步的真实步骤结果通过显式 outcome event 交付。未被 Policy 或 Contract 要求保护、且不会影响保证范围的完全可逆变化可以继续共享或忽略。文件系统和输入继续遵循各自的动态 ancestor visibility 与 fan-out 规则。

在 speculation window 内，parent 只负责执行或等待真实 operation、发布 outcome、查看受控 candidate metadata 并裁决 candidates，不与 candidates 同时推进相同的后续业务。若没有可接受 candidate，parent 从真实 outcome 继续；若接受 candidate，则不执行 parent/candidate state merge。

Proposal 必须声明与真实 outcome 的 compatibility 条件，可以是 exact payload、只允许依赖受保证字段的 predicate，或受控 virtual-value mapping。Candidate 可以在 outcome 尚未到达时 conditional commit，但未验证的 Proposal 作为 unresolved obligation 随 subtree 保留；在 obligation 验证前不能 Accept。实际 payload 无法按声明条件兼容时必须 discard 或重新执行，AScout 不修补已经基于错误具体值完成的任意计算。

Agent-facing context 不要求暴露 runtime 内部的裸 Branch ID。Agent 可以查询自己的当前 depth、lifecycle state、Proposal 和 capabilities；depth 是 Accept 树位置重定向后会变化的派生信息，不能代替稳定 identity 或用于推导权限。一个 Branch 可以获得其 direct children 的受控 metadata view，但不能由此读取 sibling、transitive descendant 或 child local state。对 direct child 的操作使用 session-scoped opaque candidate handle。

Child 不通过 ancestor PID、identity 或 execution liveness 获取业务语义，也不依赖 parent Agent 持续运行。Outcome 和 lifecycle event 由接入侧按 Branch context、capability 与 OutcomeSlot 路由，并经控制面受管通道传递。Parent Agent 可以在等待 outcome 或 commit 时被 suspend，并在事件到达后恢复；具体调度由 Agent/控制面 Policy 决定。

默认 context 查询应保持紧凑，只返回 direct-child 统计；Agent 可以按需查询 direct children 的 handle、state、Proposal 和显式 commit summary。详细 child 查询不改变 Branch 隔离边界，返回的信息必须来自显式受控通道。

Candidate Branch 即使仍有活跃 descendants，也可以把完整 Branch 向 parent 提交。接收方愿意接受时，该 Branch 整体成为 winner 并保留内部结构；candidate 的 sibling Branches 成为 losers 并失效。

Direct children 的 evaluator/resolver capability 默认由 parent Branch 中的 Agent 持有；parent 可以把该 capability 显式委托给控制面。Evaluator 是角色，不是另一种世界实体，AScout 不引入 Evaluator World。控制面和接入侧负责校验该 capability 的投机语义与路由；Runtime 只执行 Adapter 请求中属于环境边界的授权操作，不判断 candidate 质量。

Parent execution 意外丢失时，children 不能自行提升为 winner。只有此前获得显式委托的 resolver 可以继续裁决；没有可恢复 parent 或 authorized resolver 时，控制面必须 fail closed，接入侧不得继续发起该 speculation group 的受管活动，Runtime 继续执行环境级边界控制。具体恢复宽限期与最终清理策略属于故障恢复设计。

Parent 可以在 live direct child 尚未 commit 时主动 discard 整个 child subtree。Accept 只针对已经显式 commit 的 candidate；discard 不要求 candidate ready，也不自动把 child partial state 或结果反馈给 parent。Commit 与 discard 的控制语义由控制面线性化；涉及环境的停止和清理则经 Runtime Adapter 交给 Runtime 执行并返回结果。

Candidate 未被接受不表示其不可行。受控 metadata 应区分 Proposal invalidated、evaluated but rejected、policy/resource cancelled、execution failed 和 superseded 等 disposition。Child 的 commit summary/evidence 是显式受控通信，控制面可以在 Proposal 验证前封存；读取后的信息不能通过 discard 撤销。

Accept 后 candidate 保留自己的 Branch identity，并替换 parent 在 Speculative Tree 中的位置。若 parent 是 root，candidate 成为新的 root；否则 candidate 连接到原 parent 的 parent。旧 parent 和 candidate 的 sibling subtrees 失效，candidate 原有 descendants 及其内部关系保持不变。Candidate 按各状态类别规则已经可见的 parent state、已提交生效的 external effect 和已验证 outcome 必须随 winner 延续；parent 在 Branch 创建后的任意进程内存、Agent context 或评测 scratch 不要求传播，也不执行 parent/candidate state merge。

当父 Proposal 被确认而后代 Branch 仍携带其他未决 Proposal 时，父 Branch 结束其投机身份，其执行延续成为权威路径；后代 Branch 保留原有 Branch identity、隔离环境和执行进展，仅更新 Speculative Tree 中的父级引用并重新挂到该权威执行延续之下。此过程不创建新的 Branch，也不迁移 Runtime 状态。后代 Branch 在自身未决依赖清空前，不能把结果或效果纳入权威路径；若父 Proposal 被否定，后代 Branch 不能重新挂载，依赖它们的 Branch 分别进入取消与清理流程。

世界状态操作使用原生 API、透明 proxy，还是 AScout-aware 的高性能接口，不属于目标语义的一部分。无论采用哪种接口，都必须保持相同的 Branch、状态可见性与 commit 语义。

## 4. Workspace 与 Command 语义粒度

AScout 不要求逐 syscall 或任意字节流理解 Agent 行为。文件操作直接作用于当前 Branch 的 Workspace View；Agent 内建文件工具、普通文件 API 和 shell command 必须看到同一个 workspace。Workspace View 提供当前可见 revision、branch-local modifications、按文件状态规则可见的 ancestor updates、sibling isolation、winner continuation 和 loser discard。

Shell/tool command invocation 是 AScout 理解高层 operation、outcome 和 sibling reuse 的主要粒度。命令级负责语义、结果归属与优化；命令内部的文件修改、进程和资源仍由 Branch execution/workspace boundary 隔离。普通未适配命令可以作为 branch-private opaque execution 运行，不要求 Agent 把每个操作改写成 AScout API，但 opaque command 不获得高层去重保证。

Command reuse 必须基于显式 contract 和等价输入 context，至少考虑 executable/toolchain、argv、cwd、relevant environment、stdin、可见 workspace revision 以及 external-service context。可复用结果不只包含 stdout/exit code；根据 contract 还可以包含 workspace delta、artifact 或 speculation-group shared outcome。带 workspace mutation 的命令若不向目标 Branch 应用兼容 delta，不能仅返回"已经执行"。

Command contract 至少区分 result-only query、产生 delta/artifact 的 deterministic transform、由 broker/effect runner 执行一次并 fan-out 的 shared operation、创建持久资源的 session，以及不参与 reuse 的 opaque command。Persistent process 不能用静态返回值代替；具有未知 external semantic effect 的 opaque command 仍受 service adapter observation-surface 限制。

Sibling command reuse 是显式受控通信。接收方可以得知结果被复用，但不因此获得执行该命令的 sibling identity、context 或 local state。若执行期间 workspace 发生无法归属的并发修改，结果必须标记为不可复用，或在明确的 workspace transaction 中重新执行。

## 5. Branch 的可见执行环境

对任意 Branch B，其可见执行环境由以下部分共同组成：

- B 已建立独立环境后的 local state（若已建立）；
- B 在 branch barrier 上获得的 execution base，或在 lazy 创建独立环境时实际使用的 execution base；
- 按文件系统、输入、outcome event 等类别规则对 B 可见的 ancestor state；
- root Branch 所在 Speculative Tree 之外的全局计算环境。

不同状态类别不共享一条笼统的 ancestor visibility 规则。已被要求保护的完整 Agent 进程树本地执行状态在建立独立环境后独立演化；在建立环境前，Branch 只能使用 Contract 允许共享的执行视图。文件系统可以动态解析 ancestor writes，输入可以继续 fan-out，真实步骤结果通过 outcome event 交付。Sibling local state 始终不可见。

对于具有覆盖语义的 branch-local state，同一对象在可见范围内按最新修改解析：

- ancestor 在 child 写入之后产生的更新可再次覆盖 child；
- child 的再一次更新也可以覆盖 ancestor 的旧更新；
- sibling 的更新不进入彼此可见范围。

进程内存、Agent context、输入和全局服务不使用这一覆盖规则；输入遵循独立读取位置的传播语义，outcome 使用显式验证与交付，全局服务的外部效果遵循 obligation 语义。

## 6. 输入传播

进入某个 Branch 的新输入直接对该 Branch 及其全部 descendants 可见。每个 Branch 独立维护自己的输入读取位置，因此一个 Branch 读取输入不会使 sibling 或 descendant 失去同一输入。

创建 child Branch 时，child 从 parent 当前的输入状态开始，此后拥有独立读取位置。Root Branch 运行期间收到的新输入会直接传播给当时仍在运行的全部 descendants；某个 child 的私有输入只传播给自己的 descendants，不进入 sibling subtree。

输入传播不表示旧前提继续有效。用户阻止当前权威路径上的操作时，依赖该操作继续发生或依赖其原结果的 Proposal 前提已经改变。当前阶段不尝试兼容、修复或重新对齐这类 Branch；控制面将所有依赖已变更前提的 Branch 及其 subtree 视为不兼容并剪枝，经 Runtime Adapter 取消和清理。一般地，任何已识别的 Proposal 前提变更都使用同一保守规则；未受该前提影响的 Branch 不因无关输入而丢弃。

输入是否已经读取或处理不限制 subtree 提交。Winner subtree 保留自己的输入状态继续运行，loser subtrees 的输入状态随其一起丢弃。

## 相关文档

- [架构](../architecture.md)：系统结构（四侧、所有权、调用方向）
- [设计模型](../design-model.md)：抽象分层与跨领域约束
- [设计决策](decisions.md)：Branch 生命周期相关决策速查
