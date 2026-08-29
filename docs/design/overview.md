# 状态与生命周期语义

本文只记录 AScout 已确定的目标语义。仓库当前没有实现；开发决策、路线图与验收状态记录在开发文档中。

目标语义与已采用方案以本文和 [decisions.md](decisions.md) 为准。已删除旧原型及其历史实现方式不能反向定义或约束目标设计。

## 概述

AScout 是一种基于 Speculative Tree 的计算世界管理机制。它允许一个正在运行的计算世界递归产生多个 candidate Branch，使它们在共享 ancestor 环境的同时隔离各自的 local state，并继续独立演化。

AScout 分支的对象是影响 Agent 计算行为的完整运行环境，而不是代码、单一文件或单个进程。它覆盖文件系统、完整 Agent 进程树、计算机本地的其他环境，以及全局服务交互状态。

Branch 的创建与丢弃应当廉价，不应要求预先复制整个计算世界，也不应通过回滚 loser 的全部操作来完成销毁。AScout 不提供多个候选世界的通用 merge；一次裁决选择一个完整 candidate subtree 成为 winner，其他 sibling subtrees 的内部状态被丢弃。

AScout 对外暴露的顶层抽象称为 AScoutAgent。一个 task 对外呈现为一个 AScoutAgent；控制面为它维护一棵 Speculative Tree，Runtime 维护各 Branch Node 对应 Instance 的执行状态，并可实现必要的节点间状态传播，但不拥有整体 Speculative Tree 视图。受管启动描述的是 Runtime 需要满足的创建过程，不是 AScoutAgent 之外的第二种对外对象。

## Branch 与 Speculative Tree

Branch 是 Speculative Tree 中的子树，也是可继续运行和继续分叉的计算延续。每个 Branch 的根节点称为 Branch Node；Branch Node 直接拥有自己的 local state，并可包含零个或多个 child Branch Node。Speculative Tree 描述 Branch Node 之间的派生关系，并从 root Authority Instance 对应的 root Branch Node 开始；状态本身不是另一棵独立的树。

一个 AScoutAgent 恰好拥有一棵控制面 Speculative Tree。Branch Node identity、candidate handle、capability、commit/discard 和 resolution 都只在这棵 tree 内有意义；AScout 不定义或允许跨 tree 引用、授权和 lifecycle operation。同一部署可以承载多个 AScoutAgent，但它们在 AScout 语义上彼此无关，部署复用不会形成更高层的 Branch 关系。

Branch Node 在 Speculative Tree 中存在，与对应 Instance 当前是否 running、waiting 或 suspended 是两套正交状态。一个 live Branch Node 可以暂时没有正在占用 CPU 的 Agent execution，但仍保留 parent/child relationship、状态所有权和 capabilities，并可在事件到达后恢复。AScout 不为此引入 `lineage` 实体或专门术语。

`local state of A` 只表示 Branch Node A 直接拥有的状态层，不包含 child Branch Node。`Branch A` 则表示以 A 为根的整个子树，包括 A 及其全部 descendants。Child 从 branch barrier 上的 parent execution base 开始继续运行；Branch 后的 ancestor 可见性按状态类别分别定义，siblings 互不可见。

一个 Branch Node 对应 Instance 的可分支执行实体是完整的 Agent 进程树，包括 Agent 主进程及其 sub-agent。Branch 操作必须覆盖这棵进程树继续执行所依赖的计算状态，而不是只处理调用 branch 接口的单个线程或进程。

## Branch 绑定、隔离与通信

每个 Agent 执行实体由一个 Instance 承载，并绑定到对应的 Branch Node。同一 Branch Node 对应 Instance 内的进程共享该 Node 的可见执行环境，并能观察这个环境中由同一执行实体产生的修改。

除文件系统、输入、outcome event 等各自规则明确纳入的 ancestor state 外，一个 Branch 不能直接读取另一个 Branch 的 local state、进程内存或内部 Agent 上下文。Descendant 对某类 ancestor state 的可见性不代表 parent 可以反向观察 descendant local state；siblings 也不能观察彼此的 local state。

Branch 之间传递信息必须使用显式的受控通道。通过受控通道传递的信息不改变 Branch 隔离边界，也不会使接收方获得对发送方 local state 的直接访问能力。

## 隔离保证与 Runtime 安全边界

AScout 的核心保证是高效的 sibling non-interference，不要求控制面本身成为抵御恶意 Branch payload 的安全沙箱。Child Agent 是可信且知道自身 Branch role/context 的参与者；这一感知不要求暴露 runtime 内部裸 identity，也不赋予超出当前 capabilities 的权限。Agent 或工具发生普通错误时，Runtime 仍必须维持 Branch 语义，但 AScout 不默认承诺阻止可信执行主动攻击宿主或底层隔离机制。

控制面负责 speculation policy、AScoutAgent/Speculative Tree 视图与 orchestration，不承担 adversarial containment；Agent/product integration 属于 Agent 接入侧。Runtime 是节点级机制强制边界：所有 Runtime 都必须对经 Runtime Adapter到达的节点操作验证必要 identity 与 capability，维持 sibling isolation、完整进程与资源归属、loser cleanup，以及 service adapter observation surface 内的 external effect mediation。这些是 AScout 正确性要求，不能退化为由 Agent 自愿遵守的调用约定。

建立 AScoutAgent 节点执行、资源所有权或隔离边界所需的 OS privilege 只属于 Runtime。Agent 接入侧、控制面和 Runtime Adapter 不持有 mount、namespace、cgroup、VM 或等价机制所需的特权。Runtime 可以在内部使用 privileged helper、daemon 或其他分权组件，但这些组件仍属于所选 Runtime 的机制边界，不成为控制面的一部分。

具体 Runtime 可以在相同 Branch 语义之上提供不同的附加安全保证，例如使用 sandbox、namespace、VM 或其他隔离设施抵御不可信 workload。是否提供这类保证属于具体 Runtime 的能力；控制面根据自身 policy/config 选择目标 Runtime，Runtime Adapter 负责连接和协议适配，但 AScout 不协商或认证其安全等级。Runtime Adapter 的稳定 contract 不改变控制面 policy、Branch lifecycle、commit/discard 或 single-winner 语义。没有明确提供安全保证的 Runtime 不得被解释为安全边界。

Runtime 必须维护节点执行 lifecycle、obligation validation 和受控状态转换所需的权威状态，并经 Runtime Adapter 通过 typed events 交付控制面必须观察的结果。整体 Speculative Tree 视图属于控制面 AScoutAgent，而不是 Runtime 的全局视图。这一正确性状态不等于 security audit。AScout core 不要求持久、tamper-evident 或面向合规取证的审计日志；具体 Runtime 可以按其安全与部署需求额外提供 audit，Runtime Adapter 只负责映射该能力，且 audit 不改变 Branch 语义。

## 世界状态分类

世界状态当前分为三类：

1. **文件系统**：文件内容、目录结构、元数据及其他文件系统对象。
2. **计算机本地的其他环境**：完整 Agent 进程树及其 sub-agent 的内存、进程关系、环境变量和其他本地执行状态。
3. **全局服务**：数据库、远程服务，以及不属于 Agent 进程树的复杂本地后台服务。它们作为一个整体具有全局且唯一的状态；其中既可以包含无状态请求，也可以包含有状态乃至不可逆的操作。

全局服务隔离使用 service adapter 声明的 observation surface 和 guarantee horizon，不追踪无限递归的理论因果链。Semantic effect footprint 包括在当前保证窗口内可由 Agent/目标系统直接观察的响应、受管状态、服务契约定义的触发动作，以及会直接改变后续请求结果的共享约束；这些效果必须满足 sibling non-interference，并只从 committed parent operation 与 winner continuation 进入最终语义状态。

Token 费用、provider 内部 audit、telemetry、CPU 等 operational footprint 默认只归属和计量，不自动成为 Branch 世界状态。只有某项 operational footprint 通过已声明的预算控制、rate limit、告警或其他自动规则实际反馈进当前 execution 时，才进入 observation surface。Adapter contract 必须明确包含与排除项；AScout 不对 contract 和 guarantee horizon 之外的影响声称隔离保证。

## Agent 最小感知模型

AScout 向 Agent 提供三类生命周期能力：创建 candidate Branch、提交一个 candidate subtree，以及裁决收到的 subtree。Candidate Agent 决定何时提交；direct children 的 resolver capability 默认由 parent Agent 持有，parent 可以将其显式委托给控制面。持有 resolver capability 的授权主体决定是否接受、等待或丢弃 candidate subtree；runtime 不替 Agent 或其 delegate 固化这些运行策略。

Routine Agent-facing control surface 暂时只保留两类查询和五类 lifecycle/activity operation placeholders：查询自身 `context`、查询 direct `children`、`branch`、`commit`、`accept`、`discard` 和等待 event。`branch` 表示从当前 Branch 创建一个或多个 direct child；在投机场景中这些 child 才按角色称为 candidate。CLI 与 embedded function 调用先进入当前 Branch 隔离环境内的 ASC-Daemon；ASC-Daemon 隐藏并绑定当前 AScoutAgent/Branch context，再把 resolved typed request 交给 Entry API。Entry API 不从 caller context 重新解析，也不直接接触 Runtime，只把请求交给 控制面；控制面 再派发到对应 AScoutAgent 的 Branch-level operation。完整 Runtime 语义中 `branch` 可携带 candidate 数量、Proposal、outcome slot 和可选 resolver override，未指定 resolver 时使用 AScoutAgent 配置中的默认 resolver。当前不定义这些 control-surface operation 的完整参数、返回值、阻塞方式、event model 或具体命令拼写；它们仍受已经确定的 Branch lifecycle 语义约束，不能被解释为运行时 no-op。Command 与 Hook 必须调用同一 semantic API。

`PublishOutcome` 是 Runtime Adapter 上必需的 semantic operation，但不作为独立 Agent-facing command。Operation wrapper、Hook、effect broker 或 Operation Integration 在真实 operation 完成后自动把结果发布到 OutcomeSlot。Resolver capability 默认由 Parent Agent 持有；需要控制面代为裁决时，delegation 通过 AScoutAgent 创建或 Branch 创建配置表达，不作为日常 Agent command。

AScout 作为 Agent 外部的 控制面 管理完整 Agent execution。Command + Skill 是必须支持的基础接入；Agent Hook 与 embedded in-Agent adapter 是可替代的可选增强，不是正确性前提。优先使用 Hook 提供 safe point、context/outcome 注入和 tool lifecycle 捕获；只有 Hook 无法同步阻塞、替换结果、覆盖 sub-agent 或提供必要内部 identity 等能力缺口得到证明后，才引入 embedded adapter。两者不要求同时存在。

控制面与 Runtime 必须具有明确的状态所有权，并通过 Runtime Adapter交换 typed API 数据。控制面拥有 speculation planning、Proposal 业务含义、调度、预算、evaluator policy、AScoutAgent 和 Speculative Tree 视图；Runtime 独占节点级执行状态、进程与资源归属、barrier、状态隔离、capability、obligation、受控状态转换和清理。控制面不能直接修改 Runtime 内部状态，Runtime 不理解 Agent prompt、模型格式或 candidate 质量。

Runtime Adapter 对控制面提供稳定、typed、可版本化的粗粒度 command/event contract，并负责把控制面选定的目标 Runtime 解析为相应连接、能力和协议映射。具体 Runtime 可以作为独立 Runtime Service 运行。各侧只能通过显式 contract 交互，不共享跨边界的可变内部状态。AScout 不规定 Runtime Adapter 到 Runtime 的统一 control transport 或 wire format；本地 daemon、独立进程、VM channel 或远程协议不能改变 typed operation/event semantics。CLI 不参与 Runtime 选择或连接。

每个完整 AScout execution 必须由 AScout 建立受管边界后创建，从 root Agent 启动时即纳入进程树、I/O 和资源归属管理。目标语义不限定 launcher frontend；显式 CLI、IDE、daemon 或 Agent platform 可以提供不同入口，但都必须先请求 AScout 创建受管 execution。事后 attach 不能宣称具有同等的完整 Branch 语义。

控制面是 Speculative Tree 视图与 Branch 关系的权威来源；Runtime 是节点执行状态和执行实体归属的机制来源。Agent 或其短命命令子进程只请求 branch；真正的 Branch 创建必须经 Runtime Adapter触发 Runtime 的节点级操作，并作用于请求者所属的完整 Agent 进程树，不能退化为只复制发出请求的进程。

AScout 不定义创建 children 后长期冻结 parent 的 Parent barrier。它定义两个短暂且原子的执行边界：`Branch barrier` 在真实 operation 开始前作用于完整 Agent execution，建立共同 execution base、OutcomeSlot、candidate handles 和 capabilities，并让 parent/candidates 从同一个控制操作返回点按不同 role 恢复；`Resolution barrier` 在 Accept 或 parent fallback 时暂停受影响 executions，原子验证 obligations、转换 Speculative Tree/状态所有权、清理 losers，再恢复 winner 或 parent。

Branch barrier 建立期间，runtime 必须阻止新的进程、资源和未登记 external operation 越过边界，并确认 in-flight operation 已完成或已成为明确 obligation。建立失败不得留下半创建 Branch；若资源策略允许少于请求数量的 candidates，实际创建集合必须显式返回。Command 与 Hook 只是不同触发器，必须调用同一 barrier/lifecycle 语义。

AScout 使用 outcome-guided continuation branching 支持跨步骤投机。在 pending-result boundary，runtime 从共同 execution base 实例化多个可继续运行的 Branch；parent 取得真实 operation outcome，candidates 接收不同 Proposal 并提前执行后续步骤。Branch 是活的计算延续，不称为 Snapshot，也不在目标模型中引入 Snapshot 实体。

完整 Agent 进程树的内存、线程、进程关系、环境变量和其他本地执行状态从 barrier 时的 execution base 开始独立演化。Branch 创建后，parent 的任意进程内存或 Agent context 修改不会透明传播给 children；需要同步的真实步骤结果通过显式 outcome event 交付。文件系统和输入继续遵循各自的动态 ancestor visibility 与 fan-out 规则。

在 speculation window 内，parent 只负责执行或等待真实 operation、发布 outcome、查看受控 candidate metadata 并裁决 candidates，不与 candidates 同时推进相同的后续业务。若没有可接受 candidate，parent 从真实 outcome 继续；若接受 candidate，则不执行 parent/candidate state merge。

Proposal 必须声明与真实 outcome 的 compatibility 条件，可以是 exact payload、只允许依赖受保证字段的 predicate，或受控 virtual-value mapping。Candidate 可以在 outcome 尚未到达时 conditional commit，但未验证的 Proposal 作为 unresolved obligation 随 subtree 保留；在 obligation 验证前不能 Accept。实际 payload 无法按声明条件兼容时必须 discard 或重新执行，AScout 不修补已经基于错误具体值完成的任意计算。

Agent-facing context 不要求暴露 runtime 内部的裸 Branch ID。Agent 可以查询自己的当前 depth、lifecycle state、Proposal 和 capabilities；depth 是 Accept 树位置重定向后会变化的派生信息，不能代替稳定 identity 或用于推导权限。一个 Branch 可以获得其 direct children 的受控 metadata view，但不能由此读取 sibling、transitive descendant 或 child local state。对 direct child 的操作使用 session-scoped opaque candidate handle。

Child 不通过 ancestor PID、identity 或 execution liveness 获取业务语义，也不依赖 parent Agent 持续运行。Outcome 和 lifecycle event 由 runtime 按 capability 与 OutcomeSlot 路由。Parent Agent 可以在等待 outcome 或 commit 时被 suspend，并在事件到达后恢复；具体调度由 Agent/控制面 Policy 决定。

默认 context 查询应保持紧凑，只返回 direct-child 统计；Agent 可以按需查询 direct children 的 handle、state、Proposal 和显式 commit summary。详细 child 查询不改变 Branch 隔离边界，返回的信息必须来自显式受控通道。

Candidate Branch 即使仍有活跃 descendants，也可以把完整 Branch 向 parent 提交。接收方愿意接受时，该 Branch 整体成为 winner 并保留内部结构；candidate 的 sibling Branches 成为 losers 并失效。

Direct children 的 evaluator/resolver capability 默认由 parent Branch 中的 Agent 持有；parent 可以把该 capability 显式委托给控制面。Evaluator 是角色，不是另一种世界实体，AScout 不引入 Evaluator World。Runtime 只验证 resolver capability 并执行决定，不判断 candidate 质量。

Parent execution 意外丢失时，children 不能自行提升为 winner。只有此前获得显式委托的 resolver 可以继续裁决；没有可恢复 parent 或 authorized resolver 时，runtime 必须 fail closed，阻止该 speculation group 继续产生外部效果。具体恢复宽限期与最终清理策略属于故障恢复设计。

Parent 可以在 live direct child 尚未 commit 时主动 discard 整个 child subtree。Accept 只针对已经显式 commit 的 candidate；discard 不要求 candidate ready，也不自动把 child partial state 或结果反馈给 parent。Commit 与 discard 的竞态必须由 runtime 线性化。

Candidate 未被接受不表示其不可行。受控 metadata 应区分 Proposal invalidated、evaluated but rejected、policy/resource cancelled、execution failed 和 superseded 等 disposition。Child 的 commit summary/evidence 是显式受控通信，控制面 可以在 Proposal 验证前封存；读取后的信息不能通过 discard 撤销。

Accept 后 candidate 保留自己的 Branch identity，并替换 parent 在 Speculative Tree 中的位置。若 parent 是 root，candidate 成为新的 root；否则 candidate 连接到原 parent 的 parent。旧 parent 和 candidate 的 sibling subtrees 失效，candidate 原有 descendants 及其内部关系保持不变。Candidate 按各状态类别规则已经可见的 parent state、已验证 outcome 和 materialized external effect 必须随 winner 延续；parent 在 Branch 创建后的任意进程内存、Agent context 或评测 scratch 不要求传播，也不执行 parent/candidate state merge。

世界状态操作使用原生 API、透明 proxy，还是 AScout-aware 的高性能接口，不属于目标语义的一部分。无论采用哪种接口，都必须保持相同的 Branch、状态可见性与 commit 语义。

## Workspace 与 Command 语义粒度

AScout 不要求逐 syscall 或任意字节流理解 Agent 行为。文件操作直接作用于当前 Branch 的 Workspace View；Agent 内建文件工具、普通文件 API 和 shell command 必须看到同一个 workspace。Workspace View 提供当前可见 revision、branch-local modifications、按文件状态规则可见的 ancestor updates、sibling isolation、winner continuation 和 loser discard。

Shell/tool command invocation 是 AScout 理解高层 operation、outcome 和 sibling reuse 的主要粒度。命令级负责语义、结果归属与优化；命令内部的文件修改、进程和资源仍由 Branch execution/workspace boundary 隔离。普通未适配命令可以作为 branch-private opaque execution 运行，不要求 Agent 把每个操作改写成 AScout API，但 opaque command 不获得高层去重保证。

Command reuse 必须基于显式 contract 和等价输入 context，至少考虑 executable/toolchain、argv、cwd、relevant environment、stdin、可见 workspace revision 以及 external-service context。可复用结果不只包含 stdout/exit code；根据 contract 还可以包含 workspace delta、artifact 或 speculation-group shared outcome。带 workspace mutation 的命令若不向目标 Branch 应用兼容 delta，不能仅返回“已经执行”。

Command contract 至少区分 result-only query、产生 delta/artifact 的 deterministic transform、由 broker/effect runner 执行一次并 fan-out 的 shared operation、创建持久资源的 session，以及不参与 reuse 的 opaque command。Persistent process 不能用静态返回值代替；具有未知 external semantic effect 的 opaque command 仍受 service adapter observation-surface 限制。

Sibling command reuse 是 D-017 下的显式受控通信。接收方可以得知结果被复用，但不因此获得执行该命令的 sibling identity、context 或 local state。若执行期间 workspace 发生无法归属的并发修改，结果必须标记为不可复用，或在明确的 workspace transaction 中重新执行。

## Branch 的可见执行环境

对任意 Branch B，其可见执行环境由三部分共同组成：

- B 的 local state；
- B 在 branch barrier 上获得的 execution base；
- 按文件系统、输入、outcome event 等类别规则对 B 可见的 ancestor state；
- root Branch 所在 Speculative Tree 之外的全局计算环境。

不同状态类别不共享一条笼统的 ancestor visibility 规则。完整 Agent 进程树的本地执行状态在 barrier 上形成 execution base，之后各 Branch 独立演化；文件系统可以动态解析 ancestor writes，输入可以继续 fan-out，真实步骤结果通过 outcome event 交付。Sibling local state 始终不可见。

对于具有覆盖语义的 branch-local state，同一对象在可见范围内按最新修改解析：

- ancestor 在 child 写入之后产生的更新可再次覆盖 child；
- child 的再一次更新也可以覆盖 ancestor 的旧更新；
- sibling 的更新不进入彼此可见范围。

进程内存、Agent context、输入和全局服务不使用这一覆盖规则；输入遵循独立读取位置的传播语义，outcome 使用显式验证与交付，全局服务的外部效果遵循 obligation 语义。

## 输入传播

进入某个 Branch 的新输入直接对该 Branch 及其全部 descendants 可见。每个 Branch 独立维护自己的输入读取位置，因此一个 Branch 读取输入不会使 sibling 或 descendant 失去同一输入。

创建 child Branch 时，child 从 parent 当前的输入状态开始，此后拥有独立读取位置。Root Branch 运行期间收到的新输入会直接传播给当时仍在运行的全部 descendants；某个 child 的私有输入只传播给自己的 descendants，不进入 sibling subtree。

输入是否已经读取或处理不限制 subtree 提交。Winner subtree 保留自己的输入状态继续运行，loser subtrees 的输入状态随其一起丢弃。
