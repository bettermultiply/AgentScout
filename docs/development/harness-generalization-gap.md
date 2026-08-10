# Harness 通用底座差异审查

本文审查当前 `pbw-harness` 与目标“Agent 分支执行通用底座”之间的差距。这里的“通用”特指：不同上层 Agent 分支执行策略可以建立在同一套 ASCout 控制语义上；它不表示 ASCout 要扩展成 Ray Core 那种覆盖任意分布式任务的通用计算平台。

本文属于开发分析，不是 `docs/design/` 的目标语义。它区分三类内容：当前代码已经证明的事实、为了目标需要补齐的能力、以及可能限制未来扩展的设计风险。第一版仍以 Agent Speculative Execution 为主要目标，不能因为目标长期通用化就否定当前投机执行的实现重点。

## 结论先行

当前系统还不能称为类似 Ray 的 Agent 分支执行通用底座。更准确的描述是：它已经形成了 ASCout 分支生命周期控制器的骨架，具备 `ASCoutAgent`、Harness-side `BranchTree`、Branch-local context binding、policy 注册和 Runtime Boundary 的名义边界，但真实执行、策略编排、事件管理、服务化和上下文解释都还没有形成可复用的底座能力。

主要差距不在于暂时没有 Runtime backend。Runtime backend 本来就可以后置。更根本的差距是：当前接口仍围绕有限的 `branch/commit/accept/discard` metadata 操作，而目标底座需要能够承载不同 Agent 分支执行方式的共同控制协议，包括可组合的操作、异步结果、事件、上下文、资源约束和策略计划。

这并不意味着 ASCout 必须复制 Ray 的所有抽象。Task、Actor、对象存储和通用分布式计算调度不是 ASCout 第一版的目标。ASCout 可以保留 BranchTree、single-winner、Outcome、Capability 和 loser cleanup 这些领域语义；需要泛化的是它们上面的执行控制、策略接入和服务边界。

## 当前系统实际是什么

当前 `HarnessCore` 的代码注释明确说明它只修改 Harness-side ASCoutAgent metadata，Runtime Boundary 是后续阶段的 outbound port（`crates/pbw-harness/src/core.rs:9-18`）。`handle_resolved_control` 先选择 policy，再把请求转换为 `CorePlan`，最后只执行 `ValidateCurrentBranch`、读取 context/children、`BranchMetadata`、`CommitMetadata`、`AcceptCommittedChild` 和 `DiscardSubtree` 等 metadata step（`crates/pbw-harness/src/core.rs:58-141`）。这条链路能证明 Harness 入口、BranchTree 和 policy plan 的 ownership，但不能证明真实 Agent execution 或 Runtime lifecycle 已经被编排。

当前 `Policy` 收到的 `PlanningContext` 只有当前 Branch 和 `AgentControlIntent`，返回的是同步的 `CorePlan`；默认 `SequentialPolicy` 只覆盖六类控制请求（`crates/pbw-harness/src/policy.rs:30-115`）。它还不能读取 Runtime event、outcome、Proposal 验证状态、资源预算、候选证据或服务能力，因此目前更接近 metadata command planner，而不是可承载任意分支执行算法的 policy 接口。

当前 `BranchTree` 的节点只保存 BranchId、直接 parent/children、三种 disposition 和 `committed: bool`（`crates/pbw-harness/src/branch.rs:78-187`、`crates/pbw-harness/src/branch.rs:310-375`）。代码中的 `branch` 接收调用方提供的一组 `BranchId`，并没有创建 candidate spec、Proposal、OutcomeSlot、capability、commit evidence 或 Runtime execution handle。

当前 Agent-facing 输出也仍然是 metadata 文本：`AgentControlOutput` 只有 acknowledged、context 和 children，并直接格式化 BranchId 和 disposition（`crates/pbw-harness/src/control.rs:5-29`）。Local Controller 的 intent 同样直接携带 `BranchId`，而不是设计文档所说的 session-scoped opaque candidate handle（`crates/pbw-local-control/src/lib.rs:38-109`）。

Runtime 侧有一套 `Runtime` trait，Harness 侧又有一套 `RuntimeBoundary` trait。两者的方法大致重合，但已经出现漂移：`Runtime` 包含 `publish_outcome`，`RuntimeBoundary` 没有（`crates/pbw-runtime-api/src/lib.rs:19-40`、`crates/pbw-harness/src/runtime_boundary.rs:8-31`）。当前 `Harness` 是同步的进程内 Rust 对象，通过 `with_runtime_boundary` 注入一个 trait object；它还不是 central Runtime Service 的 client（`crates/pbw-harness/src/entry.rs:9-57`）。

## 与目标通用底座的主要差距

| 方面 | 当前状态 | 对长期扩展的影响 |
|---|---|---|
| 控制对象 | 主要是 Branch metadata lifecycle | 上层算法只能围绕固定命令组合，无法表达更丰富的 Agent operation |
| Policy | 当前只读当前 Branch 和 intent，并生成同步 metadata steps | 无法根据 outcome、事件、资源、预算和候选状态制定真正的分支执行计划 |
| 执行模型 | 没有统一的异步 operation、handle、结果引用或等待模型 | 深层 Branch、并行 candidate、暂停/恢复和服务化执行没有稳定承载物 |
| 调度 | 只有文档意图，没有调度状态、资源约束或队列模型 | 未来虽然可以让 policy 决定调度，但 Harness 还没有执行调度决策所需的控制能力 |
| 事件 | `RuntimeEvent` 只有可选 disposition 和字符串 message，Harness 没有事件解释链路 | 无法可靠表达 correlation、顺序、重放、失败恢复和用户可理解的上下文 |
| Outcome | Contract 中有 `PublishOutcome`，但 Harness Boundary 和 Entry API 没有对应路径 | 真实 operation once-only、pending-result 和 candidate 验证无法闭环 |
| 状态 | 节点只有 committed 布尔值和粗粒度 disposition | 无法表达 unresolved obligation、Proposal mismatch、cancelled、failed、rejected、evidence 等状态 |
| 身份 | routine intent 和 accept/discard 直接使用 BranchId | 与隐藏 Runtime identity、opaque candidate handle 和受控权限目标冲突 |
| Runtime 边界 | 两套重复 trait，且没有 service connection、能力协商或错误分类 | 具体 Runtime 选择和连接职责没有稳定实现位置，接口容易继续分叉 |
| 用户体验 | 当前 CLI 是 `context/children/branch/commit/accept/discard` 命令 | 尚未形成“用户控制一个完整 Agent，Harness 隐藏分支”的 Root Agent 体验 |
| Operation Integration | `pbw-integration-command` 只有 profile skeleton | 尚未识别真实 operation、保证 once-only 执行或自动发布 outcome |
| 服务形态 | Registry、AgentRef 和 Runtime boundary 都是进程内内存对象 | central Harness service 所需的 session、持久化、重连和并发语义还不存在 |

## 需要重点警惕的设计风险

### 1. 把 ASCout 生命周期接口误当成通用底座接口

`branch/commit/accept/discard` 是 ASCout 的重要语义，但它们不是完整的上层执行模型。若未来所有策略都只能被翻译成这几种命令，Harness 会变成“投机分支生命周期控制器”，而不是能承载不同 Agent 分支执行方式的底座。

正确的方向不是删除这些语义，而是把它们放在更稳定的共同控制协议之上：操作需要有来源上下文、目标 Branch、策略计划、运行状态、结果或事件句柄，以及明确的成功、等待、失败和取消语义。第一版可以只实现投机执行所需的子集，但接口不能把子集误写成全部底座。

### 2. Policy 名义上可插拔，实际上没有足够输入和执行能力

现在 policy 可以注册和覆盖，但它只能看到 `AgentControlIntent` 和当前 BranchId。它既不能观察完整 BranchView，也不能消费 Runtime event 或 outcome，更不能表达资源预算、并发限制、延迟执行、恢复或条件分支。`CorePlan` 也只包含 metadata step，policy 的可替换性因此主要是形式上的。

如果未来由 policy 规定调度，Harness 仍需要负责验证和执行 policy 产出的计划。为此，policy 与 Harness 之间需要稳定地区分：policy 选择什么，Harness 固定什么，Runtime 强制什么。否则 policy 不是可扩展策略，而是绕过控制不变量的任意脚本；反过来，如果 Harness 把所有选择写死，又会失去通用性。

### 3. Metadata 与 Runtime 语义有两套同名生命周期

当前 Harness `branch`、`commit`、`accept`、`discard` 可以只改 metadata；Runtime API 中也有同名 operation，而且设计文档要求真实 Branch 创建、Resolution barrier、状态转换和 loser cleanup 经过 Runtime。两个层面使用同样的名字，却没有定义一次请求的先后、确认、失败回滚和状态对齐规则。

这不是简单的实现尚未完成，而是 ownership 需要明确：Harness 可以提出和编排语义动作，Runtime 必须强制完成机制状态转换，Harness 只能在收到已确认的 Runtime event 后 finalize 自己的视图。否则 metadata-only 成功会被误认为 ASCout lifecycle 成功。

### 4. Runtime Boundary 与 Runtime API 正在发生边界漂移

`pbw-runtime-api::Runtime` 看起来像 Runtime Service 实现接口，`pbw-harness::RuntimeBoundary` 看起来像 Harness 使用的 outbound port，但二者没有明确“服务端接口、客户端端口、适配器”三者的关系。`publish_outcome` 的缺失说明语义已经不一致；同步 `&mut self` 方法也不适合未来的远程服务、并发 operation 和事件流。

在不讨论具体 transport 的前提下，至少需要稳定三个概念：Harness 依赖的语义端口、Runtime Service 对外实现的服务契约、面向具体部署的 adapter/client。它们可以共享 contract，但不能靠两个相似 trait 长期并存。

### 5. Branch identity、candidate handle 和用户视图混在一起

设计文档要求 Agent 不依赖裸 BranchId，使用受控的 candidate handle；当前 intent、输出和 BranchTree API 却直接使用 BranchId。代码中的 `BranchRole::Root/Parent/Candidate/Winner` 也是上下文角色，而 `BranchNode` 并不保存这些角色。Accept 后 root 位置可以变化，winner 也可能只是某次 resolution 中的临时角色，因此这些词不能被当成稳定身份。

建议在后续讨论中明确三件不同的东西：`ASCoutAgent` 是一棵树的控制域，Root Agent 是用户面对的主线视图，Branch 是内部执行延续；candidate、winner、parent 是某个 operation 或视图中的角色，不应自动成为长期实体名称。

### 6. 状态模型过早简化会阻碍连续体验

`committed: bool` 和 `Live/Superseded/Discarded` 足以覆盖当前 metadata 测试，但不足以支撑用户频繁介入、candidate 修复、条件提交和连续 Agent 体验。设计文档已经区分 invalidated、rejected、cancelled、failed、superseded，并要求 unresolved obligation、Proposal compatibility 和 commit evidence；这些还没有进入 Harness 状态模型。

如果先把布尔值和三种 disposition 固化为公共接口，后续会被迫通过字符串或额外旁路补充语义。第一版可以继续使用简化内部结构，但应明确它不是长期的公共状态模型。

### 7. “服务”定位与当前同步对象模型不一致

第一版部署形态已经选择 central Harness service，但当前 `Harness` 是同步进程内对象，`ASCoutAgentRegistry` 使用递增内存编号，Local Controller server 以固定请求数循环处理连接，Runtime Boundary 通过 `&mut self` 调用。这个模型适合骨架和单元测试，不足以定义服务的并发、session、请求关联、幂等、超时、断线、重连、恢复和持久化。

这些不一定要在第一版全部实现，但必须在目标文档中标为服务化差距，不能继续把进程内对象称为已经存在的 central service。

### 8. 上下文管理目前只有名称，没有责任闭环

长期目标要求 Harness 把 Runtime response/event 组合成 Agent 或用户容易理解的内容，并剔除敏感信息。当前输出仍是 BranchId、parent、children 和 Rust debug disposition 的文本；没有 response shaping、敏感字段分类、事件关联、用户主线视图或 Root Agent 连续状态。

因此“上下文管理”不能只理解为隐藏 ASCoutAgent/Branch identity。它还包括把底层 Runtime 事件解释成 Agent 可用的控制结果，并在不同用户、policy、Branch scope 下返回不同可见内容。

## 命名和语义冲突清单

| 名称 | 当前含义 | 可能的冲突 |
|---|---|---|
| `ASCoutAgent` | Harness 拥有的一整棵 BranchTree | 文档一处把它称为内部 owner，一处又称为对外顶层 Agent；需要区分控制域对象和用户视图 |
| `Root Agent` | 用户看到的主线视图 | 当前代码没有对应对象，且容易与 `BranchTree::root` 混同；应明确它是视图，不是固定 Branch 身份 |
| `Branch` | 一棵树中的节点，也表示活的计算延续 | 代码里的 `Branch` 操作只是创建 metadata child；同名会掩盖真实执行语义 |
| `candidate` | 投机场景下的 child、候选句柄或角色 | 不应把 candidate 当成第二种核心实体；应是 Branch 在某次分支操作中的角色或受控句柄 |
| `winner` | resolution 中被接受的候选 | 是一次 resolution 的结果角色，不是稳定 Branch 类型；Accept 后 Branch 仍应保留自身身份 |
| `Root/Parent/Candidate/Winner` | `BranchRole` 中的角色 | 这些角色随当前视图和 operation 变化，不应与 Branch identity 或 disposition 混合 |
| `BranchView` / `BranchContextView` / `BranchContext` | 不同层的 Branch 视图或 Runtime context | 名称相近但责任未完全分开；需要区分内部树视图、Agent-facing context 和 Runtime 返回 context |
| `Runtime` / `RuntimeBoundary` | Runtime 服务接口和 Harness outbound port | 两个 trait 重叠且方法漂移；需要明确服务契约、客户端端口和 adapter 的命名 |
| `CoreStep` | policy 计划中的执行步骤 | 当前只有 metadata step，名称却容易让人以为包含 Runtime 操作和完整生命周期 |
| `BranchMetadata` | 当前 skeleton 的树更新动作 | 与目标语义的真实 Branch operation 同名空间，建议在开发文档中持续标明 metadata-only 限制 |
| `RuntimeEvent` / response | Runtime 的底层事件和上层可见响应 | 当前用一个 `message: String` 承担多个层次，后续需要区分机制事件、Harness 状态事件和用户响应 |
| `context` | 当前 Branch 的结构查询 | 长期还包含 Proposal、capability、outcome、可见性和用户上下文；不能让结构查询和用户上下文长期共用一个未定义概念 |

## 目前有意不算作问题的内容

以下内容是目标范围或阶段选择，不应被误判为当前设计失败：

- 第一版还没有真实 Runtime、隔离 backend、Runtime service transport 和受管启动 handshake。它们是后续机制阶段，但 Runtime Boundary 的语义必须先保持一致。
- 第一版不考虑多个 ASCoutAgent 之间的协调。每个 ASCoutAgent 独立拥有一棵 BranchTree，多 Agent 协作暂时属于 ASCout 之外的上层问题。
- 第一版继续使用 candidate、Proposal、commit、accept、resolver 等偏投机执行术语是可以接受的，前提是它们不被固化成不可替换的唯一实体模型。
- ASCout 不需要成为任意分布式任务平台。没有 Task、Actor、通用对象存储并不自动构成缺陷；只有当上层 Agent 分支执行确实需要对应能力时，才需要引入相应抽象。
- Single-winner、不做通用 merge、BranchTree 树内身份和 Runtime 强制 loser cleanup 是 ASCout 的领域约束，不应为了追求“通用”而删除。
- 调度第一版可以暂不实现。当前必须先保留 policy 未来决定调度的接口空间，并区分 policy 的选择权、Harness 的编排权和 Runtime 的资源强制权。

## 后续讨论顺序

建议按以下顺序继续收敛，而不是马上补充更多 Runtime backend：

1. 先确定 Agent 分支执行底座的最小共同抽象：一次操作、Branch 上下文、计划、结果/事件、受控句柄和策略边界分别是什么。
2. 再统一 Harness 与 Runtime 的 command/event 边界，消除 `Runtime` 与 `RuntimeBoundary` 的重复和 `publish_outcome` 漂移。
3. 再决定长期状态模型，特别是 candidate handle、Proposal、OutcomeSlot、obligation、commit evidence 和 disposition。
4. 然后定义 central Harness service 的 session、关联、并发和恢复语义，并保持进程内实现只是其中一种部署形态。
5. 最后再讨论 Root Agent 连续体验、上下文裁剪、调度和多种 policy profile 如何建立在上述共同抽象上。

这些差距说明当前实现适合作为 ASCout lifecycle 的验证骨架，但还不能把现有 API 当成未来通用底座的稳定公共接口。后续扩展的重点应是抽象和边界收敛，而不是单纯增加更多 Branch 命令。
