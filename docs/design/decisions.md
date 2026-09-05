# 设计决策记录

本文是 AScout 设计决策的**速查手册与原始讨论记录**，记录逐条决策当时的选择与理由，不替代正式主题文档，也不充当语义权威。

本文只记录已经确定的 AScout 语义选择。具体 backend、原型折衷和实现重评条件记录在开发决策中。

## D-001：Agent 控制生命周期策略

**选择**：AScout 提供创建 Branch、提交 candidate subtree 和裁决 candidate subtree 的底层能力。Candidate Agent 决定提交时机；direct children 的 resolver capability 默认由 parent Agent 持有，parent 可以将其显式委托给控制面。持有 resolver capability 的授权主体决定接受、等待或丢弃策略。

**原因**：AScout 负责可靠地表达和执行 Speculative Tree 转换，不规定 Agent 应采用何种搜索、调度或评估策略。显式委托只改变谁代表 Agent 执行策略，不把评价策略转移给 runtime。

**接口边界**：世界状态操作可以使用原生 API、透明 proxy 或 AScout-aware 接口；接口形式不能改变 Branch 语义。

## D-004：Branch 覆盖完整 Agent 进程树

**选择**：Branch 的可分支执行实体包括 Agent 主进程及其全部 sub-agent，而不是单独的调用线程或主进程。

**原因**：Sub-agent 是 Agent 计算延续的一部分。只复制主进程会丢失进程关系、协作状态以及仍在进行的子任务，无法表示完整候选世界。

## D-006：覆盖型 branch-local state 采用 latest-visible-write

**选择**：对于具有覆盖语义的 branch-local state，同一对象在目标 Branch 和全部 ancestors 中的最新修改可见。

**原因**：Ancestor state 是 child 执行环境的一部分，因此 ancestor 后产生的修改应自然进入 descendants，同时维持 sibling 隔离。

**范围**：该规则适用于明确具有覆盖语义的状态类别，例如目标文件系统语义；不适用于进程内存、Agent context、输入、outcome 和全局服务。被 Policy 或 Contract 要求保护的进程内执行状态在建立独立环境后从相应 execution base 开始独立演化；未被要求保护且不影响保证范围的完全可逆变化可以共享或忽略。输入使用独立读取位置；outcome 使用显式验证与交付；全局服务使用 effect obligation。

## D-007：Single winner，不做 merge

**选择**：同一 parent 的一次裁决只接受一个 direct child subtree。Winner subtree 的内部结构和 descendants 保留；winner 的 sibling subtrees 丢弃。

**原因**：AScout 选择完整的计算延续，不定义跨文件、计算状态和全局服务的通用 merge 协议。

## D-008：允许提交包含活跃 descendants 的 subtree

**选择**：Candidate Branch 即使仍有活跃 descendants，也可以向 parent 提交完整 Branch。接收方 Agent 决定是否接受。

**原因**：是否等待 descendants 完成属于 Agent 的运行策略，不应成为底层机制的强制限制。接受后 winner subtree 整体保留，不要求先执行 bottom-up 压平。

## D-009：Accept 将树位置重定向到 winner

**选择**：Accept 后 candidate 保留 Branch identity，并替换 direct parent 在 Speculative Tree 中的位置。Parent 是 root 时 root 指向 candidate；否则 grandparent 的 child 边由 parent 改指 candidate。旧 parent 与 candidate siblings 失效，candidate descendants 保持原有结构。

**状态连续性**：Candidate 按各状态类别规则已经可见的 parent state、已验证 outcome 和已经实际生效的 external effect 必须成为 winner 延续状态的一部分。Backend 可以通过重定向、层复用或等价机制保持该状态，但不能因旧 parent 节点失效而丢失。Parent 在 Branch 创建后的任意进程内存、Agent context 和评测 scratch 不要求传播，不执行 parent/candidate state merge。

**原因**：接受的是完整 candidate subtree，而不是把 candidate 压平回 parent identity。保留 winner identity 和 descendants 可以让 Accept 成为树指向变换，并避免强制 bottom-up。

## D-015：Branch 与 Branch Node 如何区分？

**选择**：Branch 表示 Speculative Tree 中的一个子树，是计算延续和投机世界的结构实体；Branch Node 是该子树的根节点。每棵 Speculative Tree 从 root Authority Instance 对应的 root Branch Node 开始；Branch Node 直接拥有 local state，并包含零个或多个 child Branch Node。逻辑 Branch 可以先存在并推进；需要独立执行状态时才由 Runtime 建立 Instance。

**术语**：`local state of A` 只表示 Branch Node A 直接拥有的状态层；`Branch A` 表示 A 以及全部 descendants 构成的子树。不再使用 `subtree(A)` 作为现行术语。

**原因**：Branch 操作以整个子树为范围，而控制面和 Runtime 仍需要指称其根节点及其对应执行单元。显式区分 Branch 与 Branch Node 可以避免把子树范围和单节点状态混为一谈。

## D-016：Ancestor 输入直接 fan-out 到 descendants

**选择**：进入某个 Branch 的输入对该 Branch 及其全部 descendants 可见。每个 Branch 使用独立读取位置，读取行为不会从其他 Branch 的视图中消费同一输入。

**原因**：输入是 Branch 可见执行环境的一部分。Ancestor 在 descendants 运行期间收到的新输入应自然进入这些 descendants，而不要求额外 acknowledge 或 commit barrier。

**Commit 语义**：输入读取或处理进度不构成提交条件。Winner subtree 保留自身输入状态，loser subtrees 的输入状态被丢弃。

## D-017：Branch-local state 隔离，跨 Branch 显式通信

**选择**：同一 Branch 内的进程共享可见执行环境；除环境规则纳入的 ancestor state 外，一个 Branch 不能直接读取另一个 Branch 的 local state、进程内存或内部 Agent 上下文。跨 Branch 信息传递必须通过显式受控通道完成。

**原因**：Ancestor 可见性和全局环境共享不应破坏 branch-local state 的隔离，也不应隐式赋予 parent 或 sibling 对其他 Branch 内部状态的读取能力。

## D-018：Agent 外部管理与可选 Agent 集成

**选择**：AScout 从 Agent 外部管理完整 Agent execution。Agent 可以通过普通命令使用 AScout，Skill 可以作为命令使用说明和策略载体；Hook 与 thin in-Agent adapter 属于可选集成，不是基本接入的正确性前提。

**原因**：进程树、世界状态和 loser 清理必须由 Agent 外部的机制强制管理，同时不能要求每一种 Agent 都修改内部主循环后才能接入。

## D-019：Runtime 节点状态与外部 Branch 执行

**选择**：控制面是 AScoutAgent、Speculative Tree 视图和 Branch 关系的权威来源；Runtime 是节点执行状态和执行实体归属的权威来源。Agent 或 CLI 只发出 branch 请求；控制面经 Runtime Adapter 要求 Runtime 在受控 barrier 上分支请求者所属的完整 Agent 进程树，并分别向产生的世界交付结果。

**原因**：Agent 调用的 CLI 通常只是短命子进程。让它在自身内部执行 fork 只会复制调用进程，不能满足 D-004 的完整 Agent 进程树语义。

## D-020：受管创建与可替换 Launcher frontend

**选择**：完整 AScout execution 必须由 AScout 先建立受管边界再创建 root Agent，从进程创建开始捕获完整进程树、I/O 和资源。目标设计只规定受管创建，不固定为某一种 launcher frontend；`ascout codex`、通用 CLI、IDE、daemon 或 Agent platform 可以提供等价入口。

**能力边界**：事后 attach 可以作为受限模式研究，但不能宣称具有与受管创建相同的完整 Branch 语义。

**原因**：受管创建保证 runtime 不遗漏 Agent 启动早期产生的线程、FD、socket、sub-agent 和资源归属，同时避免把核心机制绑定到某一种 Agent 产品或交互入口。

## D-021：Agent-facing context 与 opaque candidate handle

**选择**：Runtime 内部保留稳定 Branch identity，但 Agent 查询自身 context 时不要求暴露裸 Branch ID。Context 提供动态 depth、lifecycle state、Proposal 和当前 capabilities；depth 是树位置的派生信息，不能替代 identity 或用于推导权限。

**Child view**：Branch 可以查询 direct children 的受控 metadata。默认 context 只返回紧凑统计；详细查询返回 session-scoped opaque candidate handle、state、Proposal 和显式 commit summary。Handle 只授权 direct-child 操作，不提供对 sibling、transitive descendant 或 child local state 的读取能力。

**原因**：Agent 需要理解当前推演层级并恢复控制上下文，但不需要依赖 runtime 的内部标识格式。Direct-child view 支持 review，同时维持 D-017 的隔离和受控通信边界。

## D-022：Parent-owned resolver capability

**选择**：Direct children 的 evaluator/resolver capability 默认由 parent Branch 中的 Agent 持有。Parent 可以把 capability 显式委托给控制面；runtime 不内建 Evaluator World，也不固定评价策略，只验证授权并执行决定。

**原因**：Parent Agent 拥有任务与实际 ancestor 结果，适合判断 speculative candidate 是否仍匹配；显式委托保留自动评分和资源调度能力，同时不增加第二种世界实体。

## D-023：Live-child discard 与受控结果披露

**选择**：Parent 可以在 live direct child 尚未 commit 时主动 discard 整个 child subtree。Accept 只允许作用于已经显式 commit 的 candidate；discard 不要求 candidate ready，也不自动把 partial state 或结果反馈给 parent。控制面只接受能够表明并发 commit 与 discard 已被后端线性化的 Adapter 结果。

**Disposition**：Not accepted 不等于 infeasible。受控 metadata 区分 invalidated、rejected、cancelled、failed 和 superseded 等原因。Commit summary/evidence 属于显式受控通信，控制面 可以在 Proposal 验证前封存；已经披露给 parent 的知识不能由 discard 回滚。

**原因**：跨步骤投机中，parent 常在 child 完成前获得真实 ancestor 结果，并应立即取消不匹配或不再需要的 candidate。结果披露必须维持 D-017 的隔离边界。

## D-024：Outcome-guided continuation branching

**术语**：Branch 是活的计算延续，不称为 Snapshot，目标模型不引入 Snapshot 实体。具体 backend 可以内部使用 checkpoint、VM snapshot 或其他 capture 技术，但不能把实现术语提升为 Branch 语义。

**选择**：在 pending-result boundary，控制面在受控 barrier 上从共同 execution base 登记多个逻辑 Branch。Candidate 可以在 Contract 允许的共享执行视图中接收不同 Proposal 并提前执行后续步骤；当接入侧识别到需要独立状态的操作时，Runtime 再建立对应的完整 Agent 进程树。被要求保护的本地状态在独立环境建立后独立演化，parent later writes 不透明传播；真实步骤结果通过显式 outcome event 交付，文件系统和输入使用各自的状态规则。

**Parent lane**：Speculation window 内 parent 只负责真实 operation、outcome 发布、受控 evaluation 和 candidate 裁决，不与 candidates 同时推进相同的后续业务。没有 candidate 被接受时 parent 从真实 outcome 继续；接受 candidate 时执行 single-winner replacement，不做 merge。

**原因**：该模型保留完整 Agent execution continuation，同时把真正需要同步的步骤结果建模为可验证事件，避免要求任意多进程内存 later-write propagation 或 parent/candidate merge。

## D-025：Proposal 验证与 conditional commit

**选择**：Proposal 必须声明与真实 outcome 的 compatibility 条件：exact payload、只依赖受保证字段的 predicate，或受控 virtual-value mapping。Candidate 可以在 outcome 未到达时 conditional commit；未验证 premise 作为 unresolved obligation 随 subtree 保留，在验证前不能 Accept。

**不匹配**：实际 payload 无法按声明条件兼容时，candidate 必须 discard 或重新执行。AScout 不自动修补已经基于错误具体值完成的任意计算。没有 Proposal 匹配时，parent 从真实 outcome 继续正常执行。

**真实效果**：被预测 operation 的真实效果只执行一次。Backend 可以在 branch barrier 前区分 parent producer，或使用 AScout 管理的 effect runner/proxy；不能让每个 speculative candidate 未经控制地重复真实操作。

## D-026：Agent integration profiles

**选择**：Command + Skill 是 AScout 必须支持的基础 Agent 接入。Hook 与 embedded in-Agent adapter 是可替代的可选增强，不要求同时存在；优先使用 Hook，只有同步阻塞、tool result replacement、sub-agent coverage、内部 identity 或其他必要能力缺口得到证明后，才引入 embedded adapter。

**正确性边界**：无论采用 command、Hook 还是 embedded profile，完整进程树归属、execution barrier、Branch identity、状态隔离、outcome obligation、winner replacement 和 loser cleanup 都由 runtime 保证，不能依赖 Agent integration 正确执行。

**原因**：Command 保持通用接入；Hook 能以较低侵入性提供多数语义时机；无条件同时维护 Hook 和 embedded adapter 会重复产品适配逻辑并扩大接口面。

## D-027：控制面、Runtime Adapter 与 Runtime 解耦

**状态边界**：控制面拥有 Agent/product integration 的控制语义、speculation planning、Proposal 业务含义、调度、预算、evaluator policy、AScoutAgent 和 Speculative Tree 视图。节点级执行状态、进程与资源归属、barrier、状态隔离、capability、obligation、受控状态转换和清理实际存在于外部 Runtime。AScout 不定义 Runtime 内部如何组织或传播这些状态；Runtime Adapter 只向控制面呈现所需能力、结果与事件，不能把后端状态冒充为整体 Speculative Tree 视图。

**接口**：控制面与 Runtime 经 Runtime Adapter 交换 typed、可版本化的粗粒度 command/event。Runtime Adapter 定义稳定 contract，并把它映射到目标 Runtime 的能力、原语、transport 与事件。控制面不得直接修改 Runtime mutable state；Runtime 不解释 Agent prompt、模型格式、candidate 质量或整体 Speculative Tree 视图。复杂 Proposal 验证可以由持有 validator capability 的控制面计算；obligation 与 Accept 相关的状态转换必须经 Runtime Adapter 交给 Runtime 执行。

**部署**：具体 Runtime 可以作为独立 Runtime Service 运行。控制面根据自身 policy/config 决定目标 Runtime；Runtime Adapter 负责解析目标、连接和协议适配。CLI、Agent integration 和 launcher 不负责选择、创建或连接 Runtime。各侧不能共享跨边界可变内部状态。AScout 不选择统一 transport；具体通信方式由 Runtime Adapter 按本地 daemon、独立进程、故障隔离或远程部署需求决定。

**原因**：控制面 policy 会随 Agent 和 workload 快速变化，Runtime 则承担正确性与资源强制边界。独立的 Adapter 侧使稳定语义 contract 与具体 Runtime 变化分离；粗粒度原子接口避免半完成的 barrier、Branch、Accept 或 cleanup，连接协议的变化不能泄漏为控制面 policy 或改变 lifecycle 语义。

## D-028：Speculative Tree 关系与 Agent execution activity 正交

**选择**：Branch 在 Speculative Tree 中存在，与其 Agent execution 当前 running、waiting 或 suspended 是正交状态。Live Branch 可以暂不占用 CPU，同时保留 parent/child relationship、状态所有权和 capabilities，并在 outcome、commit 或 control event 到达后恢复。不引入 `lineage` 实体或专门术语。

**可见性**：Child 不感知 ancestor PID、identity 或 execution liveness，也不依赖 parent Agent 持续运行；outcome 与 lifecycle event 由 runtime 按 OutcomeSlot 和 capability 路由。具体 suspend/resume 调度属于 Agent/控制面 Policy。

**失败边界**：Parent execution 丢失时 child 不能自行成为 winner。只有已经显式委托的 resolver 可以继续裁决；没有可恢复 parent 或 authorized resolver 时 runtime fail closed，并阻止该 speculation group 继续产生外部效果。恢复宽限期和最终清理策略另行定义。

**原因**：Speculative Tree 是计算世界的所有权与派生结构，不应泄漏或固化 backend 的进程调度状态。分离两类状态允许等待时节省资源，并维持 single-winner 与 resolver authorization。

## D-029：外部效果使用声明式 Observation surface

**选择**：Effect Management 管理投机分支对分支外部、共享或最终执行状态产生的影响，以及这些影响在投机期间的暂存、限制、提交、取消和清理。它管理 Branch 运行中需要保护的状态变化，但以共享、可观察的业务变化为描述中心；Branch-local state 主要由 Runtime 自动完成隔离、清理和 winner 延续，Outcome 与 operational footprint 是否纳入额外效果管理由 Policy 决定。AScout 不对所有理论上可能的间接因果影响做无限闭包，而是在 service adapter 声明的 observation surface 和 guarantee horizon 内提供 sibling non-interference。

Semantic effect footprint 包括当前窗口内可直接观察的响应、受管状态、服务契约定义的触发动作，以及直接改变后续请求结果的共享约束。

**Operational footprint**：Token 费用、provider 内部 audit、telemetry、CPU，以及额度、限流和并发槽的使用等默认归属并计量，但不自动成为 Branch 世界状态。Policy 可以将其中指定的资源或额度声明为准入约束、预算约束或取消条件。只有某项变化实际改变保证范围内的可观察业务行为时，才进入 Effect 管理；这时必须通过相应的 EffectClass、StateDomain 和 Runtime 能力处理。

**保证**：在声明边界内，sibling 私有效果不能改变其他 Branch 的观察或约束，loser semantic effects 不能进入最终状态；最终 semantic state 等价于 committed parent operations 与 winner continuation。AScout 不对 adapter contract 与 guarantee horizon 之外的 operational footprint 声称隔离保证。

**原因**：这覆盖返回值、trigger、webhook、消息和 shared quota 等真实的间接业务效果，同时避免把任意物理资源消耗和假设性的长期组织决策纳入不可验证的无限因果模型。

## D-030：Branch barrier 与 Resolution barrier

**术语**：不定义创建 children 后长期冻结 parent 的 Parent barrier。`Branch barrier` 与 `Resolution barrier` 都是控制 Branch 关系和状态转换的短暂原子边界；需要建立独立环境时，环境边界才作用于完整 Agent execution。speculation window 中 parent 进入 producer/evaluator lane，而不是被 barrier 永久阻塞。

**Branch barrier**：必须在真实 operation 开始前建立共同 execution base，登记或排空 in-flight operations，创建 OutcomeSlot、candidate handles 和 capabilities，并让 parent/candidates 从同一控制操作返回点按不同 role 恢复。Branch barrier 首先登记逻辑 Branch，不要求立即建立独立 Runtime 环境；接入侧识别到需要独立 Branch-local 状态的操作时，再建立作用于完整 Agent execution 的环境边界。失败不能留下半创建 Branch；少于请求数量的显式降级必须返回实际登记和环境建立集合。

**Resolution barrier**：Accept 或 parent fallback 时暂停受影响 executions，原子验证 commit、Proposal 和 blocking obligations，更新 Speculative Tree 与状态所有权，失效并清理 losers，随后恢复 winner subtree 或 parent。

**Integration boundary**：Command 与 Hook 只是 Branch barrier 的不同触发器，必须复用同一个 runtime lifecycle；integration profile 不得改变 Branch 语义。

**原因**：短暂原子边界提供一致 execution base 和 single-winner 转换，同时允许 parent 与 candidates 在 speculation window 内并行承担不同角色，避免把已删除旧原型的立即阻塞固化成目标语义。

## D-031：Workspace 直接访问与 Command-level semantics

**选择**：AScout 不要求逐 syscall 或任意字节流理解 Agent 行为。Agent 内建文件工具、普通文件 API 和 shell command 直接操作同一个 Branch Workspace View；shell/tool command invocation 是高层 operation、outcome 和 sibling reuse 的主要语义粒度。命令内部仍由 Branch execution/workspace boundary 隔离。

**Opaque fallback**：未适配命令可以作为 branch-private opaque execution 正常运行，不要求 Agent 将每个操作改写为 AScout API；opaque command 不参与高层 reuse，且未知 external semantic effect 仍受 D-029 的 adapter contract 与保证边界限制。

**Reuse contract**：Command reuse 必须验证 executable/toolchain、argv、cwd、relevant environment、stdin、visible workspace revision 和 external context 等等价条件。根据 contract，复用内容可以是 result、workspace delta、artifact 或由 broker/effect runner 执行一次的 shared outcome。带状态修改的命令不能只复用 stdout/exit code；persistent process 不能用静态结果代替。

**分类**：Command contract 至少区分 result-only query、deterministic transform、speculation-group shared operation、persistent session 和 opaque execution。Sibling reuse 属于 D-017 的显式受控通信，不暴露 source sibling identity/context/local state。并发 workspace 修改无法安全归属时，结果不可复用或必须重新执行。

**原因**：Coding Agent 的高层工作主要通过 workspace 与 tool/command boundary 表达。Command 粒度有利于理解、缓存和 outcome 建模，同时避免把 AScout 变成 syscall 或协议字节级的用户态 OS。

## D-032：语义隔离与对抗性安全分离

**核心保证**：AScout 以高效的 sibling non-interference 为核心，不要求控制面自身提供抵御恶意 Branch payload 的安全沙箱。Child Agent 是可信参与者，并知道自己的 Branch role/context；Agent-facing identity awareness 与 runtime 内部裸 identity、跨 Branch authority 是不同概念。

**强制边界**：Agent integration 属于 Agent 接入侧；policy 和 orchestration 属于控制面。AScout 只接受能够通过 Adapter 证明后端已落实 Branch identity/capability validation、sibling isolation、进程与资源归属、loser cleanup 以及 observation surface 内 external effect mediation 的接入。这些属于 AScout 正确性，不能只依赖 Agent 自愿合作执行 lifecycle 或资源清理。

**后端安全能力**：不同 Runtime 可以提供不同的附加安全保证。默认接入可以面向可信 workload、优先效率而不提供有效的 adversarial security guarantee；需要抵御不可信 workload 时，部署方选择相应的 hardened Runtime 与 Adapter。AScout 不定义 Runtime 的安全实现，也不设计 security guarantee negotiation；Adapter 报告的安全能力不得改变 Branch lifecycle、commit/discard、single-winner 或状态可见性语义。

**原因**：工具错误在 parent 与 child execution 中同样可能发生，不足以要求 控制面 默认承担通用恶意代码防护。把 adversarial containment 放在可替换 Runtime 中，既保留隔离正确性，也避免所有使用场景为非必需的安全边界支付固定成本。

## D-033：禁止跨 Speculative Tree 操作

**选择**：一个 AScoutAgent 恰好拥有一棵 控制面 Speculative Tree。Branch identity、candidate handle、capability 和所有 lifecycle operation 都限定在该 tree 内；AScout 不定义或允许跨 tree 查询、授权、commit/discard、Accept 或其他 Branch control。

**部署边界**：同一主机、daemon 或 Runtime process 可以承载多个 AScoutAgent 的节点状态，但每个 AScoutAgent 仍拥有独立的 root、Speculative Tree 视图与控制域。共用底层进程或服务不产生跨 tree AScout 语义，也不能让一个 AScoutAgent 获得另一个 AScoutAgent 的 Branch handle。

**原因**：跨 tree 控制不属于 speculative subtree selection，会引入额外的 identity、授权、故障恢复与状态组合语义。需要在多个 execution 之间协调的上层系统应使用 AScout 之外的接口，而不是扩展 Branch lifecycle。

## D-034：OS privilege 留在外部执行后端一侧

**选择**：建立节点执行、进程与资源所有权、隔离边界或外部效果执行边界所需的 OS privilege 留在外部执行后端一侧。Agent 接入侧、控制面与 Runtime Adapter 不持有这些特权，也不能用控制状态更新代替后端的特权操作。

**外部实现**：具体 Runtime 可以使用 privileged helper、daemon、sandbox monitor、hypervisor 或其他方式管理特权，但其内部拆分不由 AScout 定义。Runtime Adapter 可以连接相应接口，但不因负责协议适配而获得 OS privilege 或后端 mutable state 所有权。

**原因**：把特权交给控制面或 Agent 会混合 policy 与执行机制，并允许调用方绕过后端实际状态转换。AScout 只需规定 Adapter 必须返回足以确认操作结果的事件，不需要规定 Runtime 内部如何持有特权。

## D-035：Control transport 由 Runtime Adapter 决定

**选择**：Runtime Adapter 对控制面提供 typed、粗粒度、可版本化的 operation/event contract，并自行决定与 Runtime 之间的 transport、wire format 或 IPC 机制。进程内调用、Unix socket、daemon IPC、VM channel、ioctl、syscall 或 remote RPC 均可由对应 Adapter 实现选择。

**约束**：Transport 不得改变 operation 的原子性、identity/capability validation、role-specific response、failure semantics 或 Runtime 状态所有权。控制面和 Agent-facing API 不依赖 Adapter 私有的 frame、socket path、operation number 或 backend handle。

**原因**：高效默认 Runtime、独立 sandbox daemon、VM Runtime 与 remote Runtime 的通信需求不同。统一 transport 不增加 Branch 语义，却会限制 Runtime 的部署和优化空间。

## D-036：Security audit 不是 AScout core requirement

**正确性状态**：控制面只接受能够通过 Runtime Adapter 交付充分 typed events、证明节点 lifecycle、capability、obligation 和受控状态转换已经由后端落实的接入。整体 Speculative Tree 视图属于控制面 AScoutAgent；具体 Runtime 如何保存节点状态和机制状态不由 AScout 规定。

**审计边界**：AScout core 不要求持久、tamper-evident、面向合规或安全取证的 audit log。具体 Runtime 可以按自身安全保证与部署需求提供附加 audit，Runtime Adapter 只负责映射相关能力和事件；控制面不依赖该 audit 才能维持 Branch 正确性，audit 也不自动成为 Branch 世界状态。

**原因**：正确执行 lifecycle 需要权威当前状态，但不必承担安全日志的持久性、完整性证明和保留策略。把后者交给具体 Runtime，可以让默认高效 Runtime 避免固定成本，同时允许 hardened Runtime 提供更强能力。

## D-037：最小 Agent-facing control surface

**Routine surface placeholders**：Agent-facing surface 暂时只保留查询自身 context、查询 direct children、`branch`、commit、accept、discard 和等待 lifecycle/outcome event 这些 operation slots。`branch` 表示从当前 Branch 创建一个或多个 direct child；投机场景中这些 child 才按角色称为 candidate。当前不决定参数 schema、返回值、阻塞方式、event model 或具体命令拼写；“空操作”表示 control-surface contract 尚未展开，不表示已经确定的 lifecycle operation 在运行时不产生效果。

**Outcome publication**：`PublishOutcome` 保留为经 Runtime Adapter 传递的 lifecycle operation，但不作为独立 Agent-facing command。Interposer 可以通过 operation wrapper、Hook、effect broker 或其他产品适配机制，在真实 operation 完成后自动把 outcome 发布到对应 OutcomeSlot。

**Resolver delegation**：Direct-child resolver capability 默认由 Parent Agent 持有。需要控制面代为裁决时，delegation 作为 AScoutAgent 创建或 Branch 创建配置表达，不提供日常 `delegate-resolver` 命令。

**Integration parity**：Command 是基础 profile，未来 Hook 必须调用同一 semantic API；不同 integration profile 可以改变调用形式，不能增加第二套 lifecycle 或改变 operation authority。

**原因**：Agent 只需要表达自身意图，不应承担 Runtime 内部 outcome routing、capability transfer 或状态机复合操作。缩小 routine surface 可以避免裸 identity、隐式权限和 profile-specific lifecycle。

## D-038：ASC-Daemon 作为 Agent-facing context boundary

**选择**：CLI/library 不直接向 Agent Control Gateway 发送请求，而是先调用当前 Branch 隔离环境内的 ASC-Daemon。ASC-Daemon 通过自身绑定的 AScoutAgent/Branch context 隐藏当前身份，并将 resolved typed request 转交给 Agent Control Gateway。

**Agent Control Gateway 边界**：Agent Control Gateway 不从 caller context 重新解析 AScoutAgent、Branch 或 capability，也不直接接触 Runtime。它接收 ASC-Daemon 已解析的请求并交给控制面中的相应组件；控制面拥有 AScoutAgent/Branch metadata，并负责后续 lifecycle orchestration 与 Runtime 交互。Runtime 创建 Branch 执行环境后，Agent Control Gateway 在该环境内创建并启动 Agent 接入侧，形成完整的 Branch Node。

**运输约束**：ASC-Daemon 的具体 transport 不属于 AScout 语义。当前实现计划使用 Branch 隔离环境内约定端口的 HTTP Server；HTTP wire format、端口数值和 server/client 部署细节属于实现决策，不能改变 Agent-facing operation semantics。

**范围**：当前 AScoutAgent 只表示控制面 Speculative Tree metadata；Runtime connection 与受管启动 handshake 留待后续议题。

**原因**：把上下文隐藏和本地调用入口收敛到 ASC-Daemon，可以让 CLI/library 使用稳定的无身份请求，同时保持 Agent Control Gateway 的统一通信边界，并将控制面与 Runtime 的状态责任分开。

## D-039：四侧一级架构

**选择**：AScout 的完整调用路径按 `Agent 接入侧 -> 控制面 -> Runtime Adapter -> Runtime` 组织。AScout control 请求通常沿该方向下行，结果与事件沿相反方向返回。四侧是逻辑责任与 API 边界，不直接等同于四个由 AScout 拥有的子系统，也不直接等同于 crate、进程、服务或部署单元。

**Agent 接入侧**：包含 Agent/product，以及 Interposer、ASC-Daemon 和其他产品接入职责。它绑定或承载调用上下文并形成 Agent/product intent；Agent 或 resolver 仍可在授权范围内作 lifecycle 决定。该侧不拥有 AScout 全局控制状态或 Runtime 机制状态。

**控制面**：包含 Agent Control Gateway、Policy Engine、Speculation Coordinator、Effect Coordinator 与 Ledger，拥有 AScoutAgent、Speculative Tree 控制状态、policy、请求校验、操作编排和 Runtime event 解释。这些内部组件不构成额外一级侧面。Effect Coordinator 只拥有效果分类、计划与协调视图，不拥有 Runtime staged effect、obligation 或独立环境建立状态。

**Runtime Adapter**：位于控制面与 Runtime 之间，按控制面定义的稳定、typed、可版本化 operation/event 接入边界声明并呈现能力、结果和事件。其作者自行决定如何映射目标 Runtime 的能力、原语与 transport；AScout 不控制其内部实现，只通过接入校验与运行期结果判断它是否兑现声明。Runtime Adapter 不向控制面呈现第二份长期权威状态。

**Runtime**：AScout 依赖但不拥有的外部执行后端。实际节点执行、进程、资源、隔离和效果状态位于这一侧；其内部架构、组件和原生接口由具体 Runtime 自行决定。AScout 不直接约束或读取这些内部状态，只判断 Adapter 在控制面边界上呈现的能力、结果和事件。

**约束**：Agent 接入侧不能绕过控制面发起 AScout lifecycle/control operation。控制面不能绕过 Runtime Adapter 依赖具体 Runtime，也不接受会放宽 lifecycle、single-winner、capability、obligation 或 cleanup 语义的 Adapter 声明或结果。跨侧只交换显式 contract data，不共享可变内部状态。

**文档展开**：四侧路径不推出一侧一份架构子文档。控制面可以有内部设计文档；Agent 接入侧可在目标语义收敛后展开；Adapter 的内部实现与 Runtime 接入方法属于开发/接入指南；Runtime 不建立由 AScout 定义的通用设计文档。

**原因**：此前的 控制面 / Instance 部署图、控制面 / Runtime 状态位置和 Runtime Adapter 组件图描述的是不同观察角度，却被当成同一层级使用。四侧模型保留完整调用路径，同时避免把外部 Runtime 和第三方 Adapter 误写成 AScout 可以控制的内部实现。

## D-040：Policy 控制的 best-effort 投机

**选择**：AScout 提供内置默认 `Policy`。普通用户显式启用 AScout 且未指定自定义 Policy 时，控制面绑定该默认 Policy；Agent 开发者或具备相应能力的用户可以提供其他 Policy。无论采用哪种 Policy，投机决策主体都是控制面，Policy 负责在 AScout 固定语义、正确性、隔离、资源和 Runtime 能力约束内寻找、评估和选择投机机会。

**表现目标**：默认和自定义 Policy 都以 best effort 改善整体 Agent 执行表现为目标。Policy 可以判定当前机会不适合投机并选择顺序执行；启用 AScout 不表示每个步骤都必须创建 Branch，也不构成对每个 workload 的必然加速承诺。

**Agent 边界**：Agent 通常不需要感知或参与投机编排，底座的 lifecycle 正确性不能依赖 Agent 的配合。未来可以提供可选的 Agent 感知或协作能力，但其具体接口与语义另行收敛。

**原因**：把策略选择集中在控制面，可以让不同 Agent 复用同一套机制并替换策略；把 best-effort 与固定正确性约束分开，既保留优化空间，也避免把"启用"误解为无条件分支或性能保证。

## D-041：authority 决策槽并入 launch

**选择**：撤销独立的 `authority` 决策槽。当前决策面为 `S = (admit, select, rebalance, extent, predict, launch, effect, match, dispose)`；后来增加的 `select` 与 `rebalance` 均与 authority 无关。权威值来自谁是这条 Edge 的固有事实（`u` 是谁、真实结果就由谁产生），不是 Policy 可替换的决策；Policy 通过 `launch` 决定该权威值是否取得、何时取得（Eager / Lazy / Absent）。Absent 允许策略放弃天然存在的权威值（如用户输入）而只靠 validator 或可接受性判断，是否允许由 Fidelity 契约校验。validator 不是权威值来源，归属 `match` 的可接受性判断。

**废弃的旧表述**：`authority` 作为独立决策槽的语义（“验证依据的真实来源选择”）、以及将 validator 列为 authority 来源之一的做法。语义权威见 `design-model.md` §3.1 与 `policy-decisions.md` §2.4。

**原因**：launch 的取值已经完整决定权威值的取得方式（是否取、何时取）；来源本身由 Edge 决定，不构成独立策略选择，拆槽造成职责重叠与表述混淆。

## D-042：父 Proposal 确认后的后代 Branch 重新挂载

**选择**：父 Proposal 被确认后，原父 Branch 结束其投机身份，其执行延续成为权威路径。仍携带其他未决 Proposal 的后代 Branch 不被取消、不被重新创建，也不迁移 Runtime 状态；它们保留原有 Branch 身份、隔离环境和执行进展，仅把结构上的父级改挂到该权威执行延续之下。后代 Branch 在自身 `AssumptionSet` 清空前不能把结果或效果纳入权威状态。

**否定路径**：父 Proposal 被否定时，依赖它的后代 Branch 失效并分别取消、清理。后代的部分进展只有在父 Branch 进入 `Salvage` 时，才能作为父 Branch salvage 的输入，不能脱离父 Branch 独立 salvage。

**原因**：嵌套 Branch 运行在自身隔离环境中，父 Branch 确认不需要搬迁或重放子 Branch 状态；只需更新控制面的树结构与未决依赖。该规则保留嵌套执行进展，同时维持 `AssumptionSet` 和权威信息流约束。

## D-043：待选机会由 AScout 暂存，Policy 直接选择启动项

**选择**：`admit` 只决定投机机会是否进入待选集合，不决定立即启动。其粗判识别可能的投机入口，精判决定机会是否值得登记和保留；通过精判后不在 `select` 前重复精判。控制面 Speculation Coordinator 拥有待选集合并维护机会有效性。

新增独立的 `select(frontier_view, scheduling_context)` Policy Decision。一个 task / AScoutAgent 绑定的 Policy 在该 task 内所有仍有效的待选 Edge 之间进行全局权衡，直接选择当前希望启动的有序子集，可以选择一个、多个或暂时不选。未被选择的机会继续保留，直到后来被选择或因 pending result 到达、任务结束、祖先依赖否定等客观原因失效；不需要额外的主动放弃接口。

**机制边界**：Speculation Coordinator 向 Policy 提供集合快照和可观察调度事实，并在资源、Contract 与正确性硬约束内落实选择。启动前只做有效性与硬约束复核，不进行第二次 Policy 精判，也不使用固定价值公式替代 Policy 的跨 Edge 排序。best-first、具体排序公式、浅层分叉、深层单链和 breadth/depth 分配均为可选 Policy，不能成为 AScout 固定规则。`select` 只处理尚未启动的机会，不负责已经运行工作的状态调整。

**原因**：单 Edge 的独立“现在是否启动”无法表达多个机会竞争同一资源时的整体取舍，也会永久丢弃暂时不能启动但稍后仍有价值的机会。把登记、全局选择与机制复核分开，可以保留机会并让策略控制调度，同时不把队列状态或资源所有权交给 Policy。

## D-044：已启动投机工作由 rebalance 决定暂停、恢复和抢占

**选择**：新增 `rebalance(active_view, paused_view, scheduling_context)` Policy Decision。它面向同一 task / AScoutAgent 中已经运行或暂停的投机 Branch，由 Policy 决定哪些工作暂停、恢复或抢占；尚未启动的机会仍只由 `select` 处理。

**与其他决策的关系**：`extent` 根据单个过程自身进展决定推进边界，`rebalance` 根据多个已启动工作的相对价值与资源条件调整运行分配。`rebalance` 的 `preempt` 只触发 `resolution = preempted`，后续丢弃、Salvage、补偿和清理由 `dispose` 决定。

**机制边界**：Speculation Coordinator 提供运行与暂停集合的事实快照，并在 Runtime Adapter 声明的安全暂停/恢复能力、资源所有权、Branch 生命周期和效果纪律约束内执行 Policy 意图。Policy 不拥有运行队列，也不能直接修改 Runtime 状态。

**原因**：已经启动工作的价值会随权威进展、资源压力和其他机会变化。若暂停或抢占由 Speculation Coordinator 的固定排序规则决定，就会把运行策略固化进 Mechanism；将其作为独立 Policy Decision，既保留策略控制，也避免扩大 `select` 的职责。

## D-045：端用户只操作当前权威执行主线

**选择**：AScoutAgent 向端用户只呈现当前有效的权威执行主线。用户的输入与操作直接进入该主线；用户只能看到该路径产生的内容、状态和交互。投机 Branch 不建立独立用户会话，也不能在携带未决 Proposal 时直接向用户输出或提问。

**生命周期边界**：Branch 的接受、丢弃、重新挂载和清理不要求端用户参与。Policy 与授权 resolver 形成相应决策或处置意图，控制面在固定语义内编排，Runtime 经 Adapter 落实实际转换。Accept 改变权威路径后，用户继续通过同一 AScoutAgent 视图操作更新后的有效主线，不需要选择或重绑定 Branch。

**输入传播**：人可以作为 Authority 提供输入或中断。输入由当前有效主线接收后，可以按既有输入规则传播给仍在运行的 descendants；这种内部传播不使投机 Branch 获得用户可见的输出或交互通道。

**原因**：端用户面对的是一个连续 Agent，而不是多个候选世界。将用户可见性限定在权威路径，可以避免内部 Branch 生命周期泄漏成多条对话，同时保留投机 Branch 使用最新用户输入继续计算的能力。

## D-046：Proposal 前提变更时剪枝

**选择**：用户阻止当前权威路径上的操作时，所有依赖该操作继续发生或依赖其原结果的 Proposal 前提均视为已经变化。一般地，只要控制面识别出 Proposal 前提发生变化，所有依赖该前提的 Branch 及其 subtree 都视为不兼容并剪枝。

**当前范围**：现阶段不尝试判断变更前后的 Branch 是否仍可兼容，也不提供保留、挂起、修复或重新对齐路径。相关 Proposal 按否定处理，控制面编排 discard，Runtime 经 Adapter 落实取消与清理。未依赖该前提的 Branch 不因无关用户输入而被全局丢弃。

**原因**：前提变化后，既有投机进展不再对应当前 Proposal。现阶段缺少足以证明安全复用的兼容与修复语义，保守剪枝可以保持信息流和提交正确性。

## 相关文档

- [设计模型](../design-model.md)：抽象概念层
- [Branch 语义设计](branch-semantics.md)：Branch 与生命周期语义
- [效果管理设计](effect-management.md)：效果与状态
- [Policy Decisions 设计](policy-decisions.md)：Policy 决策面
- [Speculation Contract 设计](speculation-contract.md)：契约语义
- [Agent 接入侧设计](agent-integration.md)：Agent 接入组件与受控路径
- [控制面设计](control-plane.md)：控制面组件与实现
