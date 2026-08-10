# 待讨论事项

本文件是设计讨论队列，不代表其中内容已经成为设计决策。讨论完成后的结论应写入 `docs/design/`，实现细节再写入 `docs/development/`。

## 当前讨论

### Harness Core 执行模型与 Policy

- 当前构想倾向把 Harness Core 视为围绕 `ASCoutAgent` 运行的编排器：Entry API 只交付已解析的 Agent-facing typed request，Harness Core 将其翻译成一组 `ASCoutAgent` 逻辑操作和对应的 Runtime control operation。
- `ASCoutAgent`/`BranchTree` 仍是 Harness 侧 Branch 拓扑、metadata、capability view 和 BranchView 派生的权威来源；如果使用“抽象 ASCoutAgent 视图图”说法，它应指 Harness Core 维护的 `ASCoutAgentRegistry + BranchTree + 按需 BranchView/operation plan`，不引入第二个长期 source of truth。
- 每个 Agent-facing request 可以展开为一个或多个 Core step；每个 step 至少包含一个 Harness 侧逻辑意图，并可要求一次 Runtime Boundary command/event 交互。具体顺序必须按 operation 类型定义，不能简单假设所有操作都先改 Harness metadata 再调 Runtime。
- Branch/Resolution 这类需要机制强制的 operation 应通过 Runtime Boundary 完成 barrier、identity/capability/obligation validation、state transition 和 cleanup 后，再让 Harness 侧 `ASCoutAgent` metadata 与已确认的 Runtime event 对齐；metadata-only placeholder 不代表目标语义已经成功执行。
- Harness Core 的翻译过程由 ASCout 固定语义、ASCoutAgent/BranchTree 当前状态、Harness 配置、Agent/product integration 输入、resolver/evaluator delegation、预算/调度/恢复规则和外部传入策略共同决定。
- 已确认 Harness Core 应根据 `Policy` 将 Agent-facing request 翻译为 `Plan`；但 ASCout 固定语义与 Core invariant 先限定合法计划空间，Policy 只在合法空间内选择具体 plan，而不是直接生成可绕过校验的任意执行脚本。
- 已确认 `CorePlan`/`CoreStep` 应是纯数据结构，只描述待执行的逻辑/Runtime step，不携带 executor 行为、不直接持有 mutable Harness/Runtime 引用；执行、校验、错误映射和 metadata finalize 留在 Harness Core。
- 已倾向把此前的 `Speculative Policy` 提升为更一般的 `Policy`：speculation 只是其中一个 policy domain；Policy 也可覆盖 no-speculation/sequential baseline、candidate creation、resolver delegation、wait/resume、budget/resource handling、failure/fallback 和 Runtime Service selection 的 Harness 侧决策。
- 已确认 Policy ownership 采用 Harness Core 全局默认 Policy + ASCoutAgent 可覆盖：Harness Core 持有 default policy，ASCoutAgent 创建配置可以绑定自身 policy；未绑定时使用 default policy。
- `PolicyRef`/named policy handle 的意思不是把 trait object 直接塞进 `ASCoutAgent`，而是在 Harness Core 维护的 policy registry/config 中给 policy 一个稳定名字或 handle；ASCoutAgent 只保存可选 policy id，执行时由 Harness Core 解析为具体 policy。具体是否需要 registry 可等第一版实现时再定。
- Runtime-bound `branch` plan 与 Runtime response mapping 暂缓到 Runtime phase；Harness Core 通过 Policy plan 执行 Harness-side BranchTree metadata，不直接调用 Runtime Boundary。
- Policy 不能拥有 Runtime mutable state，不能绕过 Runtime capability/obligation validation，也不能改变 Branch lifecycle、single-winner、commit/discard/accept 或跨 tree 禁止等 ASCout 目标语义；它只能在这些语义约束内替 Harness Core 做选择。

### Local Controller 与 Entry API 边界

- 已确认 Agent-facing control path 为 `CLI/library inside Branch -> Branch-local Local Controller -> Harness Entry API -> Harness Core`。
- Local Controller 位于 Branch 隔离环境内，负责接收 CLI/library 的请求并隐藏、绑定当前 ASCoutAgent/Branch 上下文；Agent-facing 请求本身不携带这些身份信息。
- 第一版 Local Controller 采用 HTTP Server 形式，并在每个 Branch 隔离环境内使用约定端口；端口约定属于当前实现，不把 HTTP wire format 提升为 ASCout 目标语义。
- Entry API 不再自行解析 caller context，也不直接接触 Runtime；它接收 Local Controller 已解析的 typed request，只负责把请求交给 Harness Core。Harness Core 才拥有 ASCoutAgent/Branch metadata 和后续与 Runtime 的连接职责。
- 当前阶段暂不展开 Harness Core 到 Runtime 的连接断点、Runtime client、Runtime Service discovery 和 Runtime-side endpoint 创建。
- `ASCoutAgent` 当前只表示 Harness-side metadata BranchTree；Runtime execution state 不在该抽象内。
- 测试覆盖 Controller determines branch 与 Harness-side metadata lifecycle；HTTP malformed/unknown/error mapping 和 Runtime lifecycle 不在当前范围。
- Controller determines branch：同一个 CLI 请求分别打到 root Local Controller 和 child Local Controller，返回各自绑定 Branch 的 context，证明 Branch context 来自 Local Controller 绑定而不是 CLI 自报。
- Metadata lifecycle path：通过 CLI 命令串联 `ascout branch`、`ascout children`、`ascout commit`、`ascout discard`、`ascout accept`、`ascout context`，验证 Harness-side ASCoutAgent/BranchTree metadata 的 parent/children/root/disposition/committed 变化。
- CLI 命令暂用直白形式：`ascout context`、`ascout children`、`ascout branch`、`ascout commit`、`ascout accept`、`ascout discard`；正式 UX 后续再收敛。

### Harness-side Routine Operation Naming

- Agent-facing 的分支创建操作统一命名为 `branch`。它表示从当前 Branch 建立一个或多个 direct child；在投机场景中这些 child 才按角色称为 candidate。
- `create-child` 不是独立的 ASCout 目标操作，只是已移除的 skeleton metadata shortcut；低层 BranchTree 可以保留 child insertion primitive，但不再作为 CLI 或 Routine Agent intent 暴露。
- 计划中 `branch` 先完成 Harness-side BranchTree metadata；Proposal、outcome slot、opaque handle 和 Runtime barrier 等行为留待 Runtime phase。
- `start_ascout_agent` 的计划边界是通过 Harness Entry API/Core 创建 ASCoutAgent、注册 root metadata 并返回 ASCoutAgentRef；不调用 Runtime。
- `wait event` 仍是 operation slot，event ordering、blocking 和 Runtime event source 尚未定义，因此当前不伪造成功行为。

### ascout-cli 薄 CLI adapter 与 Runtime plugin 边界

- 已确认 `ascout-cli` 是薄 CLI adapter，只负责参数解析、命令分派、输入输出和退出码；不拥有 ASCout lifecycle、speculation/evaluator policy 或 Runtime backend 逻辑。
- 已确认具体 Runtime 实现应作为运行时插件使用，而不是作为 CLI 的编译期依赖。Harness 与 Runtime 之间保持稳定的语义 contract；具体 adapter/service 的发现、选择、加载或连接由 Harness 负责。
- 当前 CLI 不暴露 Runtime 名称或 backend 选项。未来如果允许用户通过 CLI 指定 Runtime，CLI 只传递一个中立的选择输入，不能自行解析、创建或连接 Runtime。
- 已确认 CLI 不得编译期依赖、构造或感知任何具体 Runtime。
- Runtime plugin 是独立 service/process、动态库或其他装载形式尚未决定；该问题单独讨论，不能由 CLI 先行固化。

### 逐 crate 实现顺序（crate 组织已废弃，决策语义保留）

- 已确认 Harness 与 in-process Runtime 不应绑定：Harness 通过 Harness-side Runtime Boundary client 构造，不携带具体 Runtime 类型。
- 用户要求 Harness 只依赖稳定的 Runtime API，并由 Harness 决定选择和连接哪一个独立 Runtime Service；替换底层 Runtime 不应要求修改 Harness 的 semantic API。
- 用户要求 Harness 同时支持外部传入 speculation plan、Proposal、resolver intent 等策略输入，并提供一个默认的 no-speculation/sequential policy，为不支持 speculation 的 Agent 提供服务，也用于测试与 baseline。
- 用户澄清 Agent 接入 ASCout 有两种入口：内嵌时调用 ASCout 提供的函数/库接口；不内嵌时通过 ASCout CLI 以命令行形式操控。两者应进入同一套 Harness semantic API，不形成两套 lifecycle。
- facade 只是对稳定统一入口的解释性称呼，不要求额外建立名为 facade 的组件；正式讨论优先使用 Agent-facing semantic API、library API 和 CLI integration 这些更具体的术语。
- 用户否定“CLI 作为 composition root 组装并选择 Runtime adapter”的方案：不得通过依赖注入组合 Harness 与 Runtime，CLI 也不负责选择、创建或连接 Runtime。
- 当前只确认 CLI 是 CLI integration 入口，Harness 拥有 Runtime Service 的选择与连接职责；具体 connection protocol、client/adapter 形态和 service discovery 仍未决定；此前 skeleton 的组合 wiring 不构成目标实现。
- 已确认 Harness Entry API、CLI/library integration 与 Harness Core 的连接边界：Local Controller 负责上下文绑定，Entry API 不再做 context-bound resolution，也不直接接触 Runtime；Runtime Service client/adapter 及 `publish_outcome` 的 Runtime 侧连接暂不展开。
- 已确认 Harness Core 应持有 `ASCoutAgentRegistry`；Entry/API 层应保持 stateless，不持有长期 Harness 数据结构。
- 已确认外部 Agent 不应拿到 `ASCoutAgent` session object/handle，也不应感知当前属于哪个 ASCoutAgent 或 Branch；当前由 Branch-local Local Controller 绑定并隐藏 ASCoutAgent/Branch context。
- 已确认内部 record 直接称为 `ASCoutAgent`，代表从一个 root Branch 衍生出来的一整棵 ASCout Agent tree；代码与文档术语应保持一致，继续使用 `BranchTree`，不引入 `BranchTopology` 等分裂命名导致后续遗忘和混乱。
- 已确认如果 `root` 只是单独保存一个 `BranchId`，则没有必要作为 `ASCoutAgent` 的独立字段存在；root 应作为 `BranchTree` 自身的不变量或入口查询，而不是在 ASCoutAgent 上重复保存。
- 已确认 `BranchNode` 本身必须维护直接 parent 与 direct children；parent 不一定是 root，grandchild 的 parent 应是直接 child。`BranchTree` 负责保存 root 与 node lookup，用于快速查找某个 node。
- 已确认 routine control request 的上下文边界在 Local Controller：Local Controller 将自身绑定的 ASCoutAgent/Branch context 附加到 typed request 后交给 Entry API；Entry API 保持 stateless，只把已解析请求交给 Harness Core，再由 Harness Core 派发到对应 ASCoutAgent 的 Branch-level operation。
- 已确认 `commit` 必须存在；accept 以 candidate 已提交为前置条件，具体 obligation validation 后续再收敛。
- 已确认 create child 不要求 parent 从未 discard 过 child，也不因当前 child 都失败或 parent 选择继续而禁止继续创建；只要当前 Branch 本身仍是可继续执行的 live Branch，它在后续状态/阶段仍可再次 create child。
- 已确认 parent continue 不需要单独 operation；Parent 想继续时主动 discard 所有 direct child candidate subtrees，之后 parent 继续向下执行并可进入新的 `branch` 阶段。
- 已确认普通 discard 清理整个 `subtree(child)` 并从 active parent children 中移除；accept 内部的 self-discard 不是独立终止操作，而是旧 parent 被 accepted child 替换后的 superseded/self-invalidation。
- 已确认 accept 是复杂 Resolution operation：它需要 discard 未接受 children，把旧 parent 的 parent 边重连到 accepted child；如果旧 parent 是 root，则 BranchTree root 改为 accepted child。Runtime/状态层仍必须完成必要的状态传播、记录或 materialization 后再清理旧 parent/losers；Harness BranchTree 侧只能表达拓扑与 metadata 转换，不能假装完成 Runtime state propagation。
- 已确认不引入单独的 root self-discard/terminate operation；root 被 accepted child 替换只是普通 accept 的 root case。
- 已确认 policy/helper 可以是 ASCoutAgent-specific：Harness 可以有默认 policy，但每个 ASCoutAgent 可以持有自己的 policy/helper 配置或绑定。
- 暂不引入 `RuntimeExecutionToken` 之类细粒度概念；资源 token、quota 或 runtime-specific handle 等细节不影响当前 Harness Core 可扩展性，后续需要时再设计。
- 当前倾向采用 central Harness：Harness Core/ASCoutAgentRegistry/BranchTree 作为隔离环境外的中心 owner，而不是把 Harness Core 分布到每个 Branch 隔离环境内。
- 因 Agent 运行在 Runtime 提供的 Branch 隔离环境内，Agent-facing CLI/library 调用必然从该隔离环境跨越 Runtime 边界到达 central Harness；上下文解析不依赖 Agent 自报的 ASCoutAgent/Branch 标识。
- central Harness 模型下，Branch 隔离环境内的 Local Controller 持有该 Branch 专属的本地控制入口；Local Controller 根据自身绑定的 Branch context 形成已解析请求，再交给 Harness Entry API，Harness Core 负责派发到 ASCoutAgent Branch-level operation。
- 已确认 Agent-facing control path 采用 `CLI/library inside Branch -> Branch-local Local Controller -> Harness Entry API -> central Harness Core`。Local Controller 放在 Branch 隔离环境内，和 Agent/CLI 同侧；第一版使用约定端口的 HTTP Server。对 Local Controller 而言，入口自身代表当前 Branch，不需要让 CLI 自报 ASCoutAgent/Branch identity。
- 已确认 Local Controller、Harness Entry API、Harness Core 和 Runtime Adapter 是不同概念：Local Controller 是 Branch 隔离环境内供 Agent-facing CLI/library 调用的 HTTP 控制入口；Entry API 是无 Runtime 访问的统一语义入口；Harness Core 拥有 ASCoutAgent metadata 并负责后续编排；Runtime Adapter 的连接断点暂不展开。
- Local Controller 的实现边界必须能在 Branch 隔离环境内独立运行，不能与 central Harness Core 的长期 metadata owner 混在同一个部署假设中；第一版 HTTP Server 的具体模块/二进制布局由实现阶段确定。

### 投机执行底座与 Branch tree 语义

- 已确认：投机执行底座（`docs/development/spec-exec-abstraction.md` 与 `docs/design/speculation-decisions.md`）就是当前系统（ASCout）本身的设计目标，不是另一套相邻系统的设计。
- 待收敛：`docs/design/decisions.md`（Branch tree 视角，D-001..D-038）与 `docs/design/speculation-decisions.md`（投机执行底座视角，D-001..D-028）是同一系统的两套设计记录；两套术语的对应关系（如 Branch / SpecTask、commit+accept / Reconciliation+Disposition、candidate / Proposal 等）与不一致处的收敛方式尚未确定。
- 待定：两套决策记录最终是保持分离（附术语映射表）还是合并为单一决策记录（统一编号），以及新决策推翻/修改既有决策时的更新流程。

## 已解决

### 术语收敛（第一轮）

- 已确认：投机执行底座就是当前系统（ASCout）本身的设计目标，不是另一套相邻系统的设计；`docs/design/decisions.md`（Branch tree 视角）与 `docs/design/speculation-decisions.md`（投机执行底座视角）是同一系统的两套设计记录，保持分离并建立术语映射表。
- 已确认：系统名称已改为 **ASCout（Agent Scout）**，旧名不再使用。改名范围：仅文档（本仓库文档；crate/代码标识符/CLI 命令/仓库目录名暂不联动）。**2026-08 更新（D-023-I）**：所有 pbw/ascout-* crate 名与 CLI 命令标识一并废弃，不再存在任何以 pbw 或 ascout-* 命名的实现标识；实现组织直接围绕目标组件模型重新决定。
- 已确认：BranchTree 不是与底座分离的东西，它是底座的数据结构，重命名为 **Speculative Tree**。
- 已确认：同时保留 **Branch** 与 **Instance** 两个名称；Branch 扩展为整个子树（不再区分 `subtree(A)`），Instance 为运行时执行单元。二者对应关系的精确表述待收敛。
- 已确认（待最终定名）：Local Controller 与 Local Daemon 合并为一个驻 Branch 隔离环境的本地守护进程，暂名 **ASC-Daemon**；Interposer 保持分离（请求路径 vs 旁路）。
- 已确认：③ 层间映射中的底层抽象概念（Reconciliation、Disposition、Equivalence、Authority Launch 等）全部保留，作为目标语义语言；Control Plane 各组件取舍已收敛：**API Entry 不画在架构图上（职责分散：任务提交/查询 → Entry API、policy 绑定 → Harness Core、任务表 → Harness Core、policy 声明 → Policy Engine），不单独设包**；Assumption Manager / Effect Coordinator / Scheduler / Ledger / Runtime Adapter 全部保留（含第一版实现）。
- 已确认：**Policy Compiler 与 Policy Set 合并为 Policy Engine**（六类 policy 插件的宿主 + 加载期编译校验 + 运行期被 mechanism 反向调用分发到插件）。不再使用 Compiler/Set 两个名字。
- 已确认：**Frontier Scheduler 改名 Scheduler**（组件名）。“frontier”是搜索算法行话且不能覆盖准入/预算/抢占等职责；§9.3 的 frontier budget / best-first frontier expansion 机制名暂时保留（若再嫌拗口可一并改）。
- 已确认：**Engine Plane 删除，整个不做**（D-013/D-011/D-024/D-028 已同步修订）：KV cache 层 commit、partial-token accept、引擎压力调度、sequence residency 等全部排除；跨层可见性降级为 API 可观测信号（延迟/限流/配额）；接缝 1 去掉 prefix cache 共享。札记中的双层分析保留为论文背景与未来自托管候选。
- 已确认：**术语统一用 Proposal（猜测值）**，废弃 hypothesis。Branch 视角文档已全部同步：预测值含义 → Proposal；compatibility 条件保留为 Proposal 的属性（对应底座 Assumption = proposal 成立这一假设）；“hypothesis validation” → “Proposal 验证”。**撞名处理**：原 “commit proposal”（提交提议）改称 commit（operation）/ commit request（中立值），把 proposal 让给猜测值。
- 已确认：**Site 与 pending-result boundary 是两种东西，并存**：pending-result boundary 是“真实 operation 结果未到”的执行边界（时机语义）；Site 是该边界上被标注可投机、实际启动投机的依赖边/节点（结构语义）。
- 已确认：**task 与 ASCoutAgent 是一对一** —— 一个 task（端用户提交的准入单元，API Entry 任务表条目）对应一棵 Speculative Tree（一个 ASCoutAgent 控制域），准入时绑定 policy profile 并建立 root Authority 实例。task 是准入/用户视角的名字，ASCoutAgent 是树控制域视角的名字。
- 已确认：**Branch ↔ Instance 1:1** —— Speculative Tree 的每个节点是一个 Branch（结构实体），每个 Branch 节点在运行时由一个 Instance 承载；“Branch 扩展成整个子树”指 Branch 指称包含其全部 descendants，提交/丢弃/接受以整个子树为单位，不再区分 `subtree(A)`。
- 已确认：**Interposer ↔ Hook / effect runner 映射为同一概念** —— 数据面、请求路径上拦截四接缝；与 ASC-Daemon 分离（延迟敏感 vs 旁路）。
- **术语收敛完成**：本小节全部条目均已收敛，术语映射表已落盘为 `docs/design/terminology.md`；两套决策记录的分离关系与权威声明见该文档。

- 用户希望把此前零散讨论形成的特性与约束放进一个全局视角，共同绘制整个项目的架构图。
- 第一版草图记录于 `docs/architecture.md`，只综合已经接受的设计与开发决策，不新增 backend、协议或 lifecycle 语义。
- 草图分为四个互补视角：系统上下文与 ownership、Runtime 机制与世界状态、一次投机执行的主时序、目标组件模型。这样避免把目标语义、运行时数据流和实现组织压进一张不可读的图。
- 已确认以 Harness policy plane / Runtime mechanism plane 的边界，以及“由 Harness 编排、Runtime 强制管理”的 Agent execution 为整套架构图的主轴。
- 已记录文档收口规则：实现重新建立后，README、架构图、开发文档和使用文档中用于说明“当前从零开始/尚未实现”的过渡期状态表述必须移除或改写为普通实现状态；active documentation 不应长期保留过渡期 wording。
- 用户确认当前架构设计足以进入实现；更细节的设计等源码建立后再讨论，不在当前轮继续展开。

### 空实现目标架构（crate 拓扑部分已废弃，见 D-023-I）

- 已确认新实现继续使用 Rust。旧 `src/**`、manifests、tests、examples、headers、构建产物和实验文件已经按授权删除，未读取、迁移或复用其中的过时实现。
- 已确认必须为 Execution、Workspace、Input、Effect、Outcome 和 Operation Integration 预留明确扩展点。
- 用户只参与关系到可维护性、性能与功能的重要高层权衡；命名、文件布局、普通胶水、测试组织和不改变高层边界的实现细节由 Codex 决定。

> crate 拓扑、依赖方向、占位验收与 D-021-I skeleton 等条目已废弃删除，废弃记录见 `docs/development/decisions.md` D-023-I；本文不再保留相关 stub。

#### 六类扩展点的含义

- `Execution`：Runtime 侧的 Branch execution backend boundary，负责完整 Agent 进程树及 sub-agent、内存/线程/进程关系、环境变量、FD、signal 与 branch-private kernel resources，并参与 barrier capture、实例化、suspend/resume、winner continuation 和 loser cleanup。它替换执行环境实现，不改变 Runtime 强制的身份、capability、资源归属和 lifecycle 语义。
- `Workspace`：Runtime 侧的 Branch Workspace View backend boundary，负责为 root/child 建立可见 workspace、隔离 branch-local modifications、表达 ancestor visibility，并参与 Accept/Discard 后的 winner continuation 与 loser cleanup。它替换文件状态实现，不改变 D-006/D-009/D-031 语义。
- `Input`：Runtime 侧的输入承载与交付 backend boundary，负责 ancestor-to-descendant fan-out、每 Branch 独立读取位置，以及 Branch 创建、Accept、Discard 时的输入连续性。TTY、pipe、stream 或 message backend 可以不同，不改变 D-016。
- `Effect`：Runtime 侧面向全局服务的 semantic effect adapter boundary，依据 observation surface/guarantee horizon 管理 effect obligation、once-only execution、defer/deduplicate/compensate/materialize/cleanup。它不是通用网络 transport，也不把 adapter contract 外的 operational footprint 纳入保证。
- `Outcome`：OutcomeSlot、routing、blocking obligation 和 Accept gate 属于 Runtime core；可替换部分是把工具、CI、用户输入或远程服务等真实结果规范化并发布到 OutcomeSlot，以及接入相应 payload/validation data。它不允许 adapter 绕过 core 直接改变 obligation 状态。
- `Operation Integration`：Harness/integration 侧对 shell/tool operation boundary 的接入，负责识别或包装 operation、关联 command contract、建立 pending-result integration、确保真实 operation 只执行一次，并在结束后自动触发 `PublishOutcome`。Command wrapper、Hook 或未来 embedded adapter 都实现同一语义入口，不能承担 Runtime 隔离与生命周期强制。
- 已确认 OutcomeSlot/routing/obligation/Accept gate 属于 Runtime core，只有真实 outcome source/normalization 可以替换；Outcome adapter 不能直接改变 core obligation state。

#### Workspace 之外的本地执行环境

- 现有 `Workspace` 术语只表示文件/目录及 command-visible workspace state；若把环境变量、进程树、端口和其他内核资源全部纳入 Workspace，会混合 D-031 文件视图与完整 Agent execution 的不同状态语义。
- 已确认增加第六类 `Execution` 扩展点，负责完整 Agent 进程树、内存/线程/进程关系、环境变量、FD 与 branch-private kernel resources、suspend/resume、barrier capture、winner resume 和 loser cleanup；`Workspace` 继续只表示文件/目录视图。
- 端口不是单一类别：Branch-private socket/port namespace 属于 Execution resource；对宿主或其他 Branch 可见的共享端口、配额或 reservation 属于 Effect observation surface；持久或不支持多实例的后台进程可能是 shared service/session，需要由 Effect/Operation contract broker，而不是强行复制。
- `Branch execution environment` 可以作为 Workspace、Execution、Input、Effect interaction 和 Outcome event 的聚合概念，但不需要成为另一个拥有状态与 lifecycle 的核心实体。

### 旧实现删除与文档切换

- 用户已明确授权直接删除全部旧实现；`src/**`、Cargo manifests、tests、examples、header、`target/`、实验产物和样例数据已经删除。
- 只描述旧实现的 lifecycle、proxy、runtime、control protocol、service、module 和旧测试矩阵文档已经删除，不建立 active legacy archive；Git history 是唯一历史来源。
- README、使用文档、开发架构、开发决策、路线图和验收文档曾统一切换为“当时无实现”的过渡期状态；目标设计文档继续作为语义权威。
- 本次授权不包含创建新 Cargo workspace 或实现 skeleton。正式开发选择见 `docs/development/decisions.md` 的 D-017-I 与 D-022-I。
- 该状态已被 2026-07-31 的 skeleton 实现授权与文档收口覆盖；2026-08 又按 D-023-I 废弃全部 skeleton 阶段 crate 拓扑与实现记录，当前 active documentation 只描述目标语义与开发决策，不描述任何实现。

### 安全与部署边界

- ASCout 的核心保证是高效 sibling non-interference，不要求 Harness 成为对抗恶意 Branch payload 的安全沙箱；Child Agent 是可信且知道自身 Branch role/context 的参与者。
- Runtime 强制维护 identity/capability、sibling isolation、进程与资源归属、loser cleanup 和 observation surface 内的 external effect mediation。默认 Runtime 优先效率、不提供 adversarial security；用户可以显式选择项目适配的 sandbox/hardened Runtime，不引入安全 guarantee negotiation。
- 一个 ASCoutAgent 恰好拥有一棵 Harness-side BranchTree，ASCout 禁止跨 tree identity、capability、引用和 lifecycle operation。Runtime execution state 不在当前 ASCoutAgent 抽象中展开。
- 所需 OS privilege 只由 Runtime 或其内部 helper/daemon 持有；Harness、Agent 和 integration adapter 不持有特权。
- ASCout 只固定 typed operation-event contract；control transport 与 wire format 完全由 Runtime adapter 决定。
- ASCout 正确性所需的权威 lifecycle/obligation 状态与 typed events 必须存在，但持久、tamper-evident 的 security audit 不属于 core requirement，由 Runtime adapter 按需提供。
- 不可逆外部操作仍归 `Effect adapter materialization` 议题，不在本议题中展开。
- 正式结论见 `docs/design/overview.md` 与 `docs/design/decisions.md` 的 D-032 至 D-036；默认 Runtime 开发选择见 `docs/development/decisions.md` 的 D-016-I。

### Workspace 与 Command-level semantics

- 本议题按新目标设计收敛；已删除旧原型的 ptrace、shadow state、模块划分和 syscall event path 都不构成约束。
- 文件操作直接进入 Branch Workspace View；shell/tool command 是高层 operation、outcome 和 sibling reuse 的主要粒度，命令内部仍由 Branch execution/workspace boundary 隔离。
- Command contract 区分 query、transform、shared operation、session 和 opaque；reuse 必须验证输入 context，并按需返回 result、workspace delta、artifact 或 shared outcome。
- Opaque command 可以正常隔离运行但不盲目去重；sibling reuse 是受控通信，不暴露 source sibling state。
- 正式结论见 `docs/design/overview.md` 与 `docs/design/decisions.md` 的 D-031；新实现任务见开发架构与路线图。

### Branch barrier 与 Resolution barrier

- 不定义长期冻结 parent 的 Parent barrier；Branch/Resolution barrier 都是短暂、完整 execution 范围的原子边界。
- Branch barrier 在真实 operation 前建立共同 execution base，并从同一控制返回点给 parent/candidates 不同 role；speculation window 内 parent 运行 producer/evaluator lane。
- Resolution barrier 原子验证、转换状态、清理 losers，再恢复 winner 或 fallback parent。
- Command 与 Hook 复用同一 lifecycle；失败不留下半创建 Branch，资源降级必须显式返回实际创建集合。
- 正式结论见 `docs/design/overview.md` 与 `docs/design/decisions.md` 的 D-030；新实现任务见开发路线图。

### 外部效果的 Observation surface

- ASCout 对 service adapter 声明的 observation surface 和 guarantee horizon 提供 sibling non-interference，不采用无限递归的理论因果闭包。
- 当前 execution 可直接观察的响应、受管状态、服务触发动作和直接改变请求结果的共享约束属于 semantic footprint。
- Token 费用、provider audit、telemetry、CPU 等默认只归属和计量；只有实际反馈进 execution policy 或目标业务流程时才升级进入 observation surface。
- 最终 semantic state 等价于 committed parent operations 与 winner continuation；contract/horizon 外不声称隔离保证。
- 正式结论见 `docs/design/overview.md` 与 `docs/design/decisions.md` 的 D-029；service adapter 实现任务见开发路线图。

### Branch tree 与 Agent execution activity

- Branch 在 Branch tree 中存在，与其 Agent execution running/waiting/suspended 正交；不引入 `lineage` 实体或专门术语。
- Child 不感知 ancestor PID、identity 或 liveness；outcome/lifecycle event 由 runtime 按 OutcomeSlot 和 capability 路由。
- Parent loss 后 child 不能自行成为 winner；只有 delegated resolver 可以继续，没有授权恢复路径时 fail closed。
- 正式结论见 `docs/design/overview.md` 与 `docs/design/decisions.md` 的 D-028；实现差距见 FAQ 和路线图。

### 首要验证 Workload

- 第一阶段使用 `ascout codex` + Command/Skill 在真实仓库执行 delayed-choice coding benchmark，并与 sequential baseline 对照。
- 确定性测试 Agent 只做 conformance；暂不深改 OpenCode，Command 路径证明收益后再增加 Hook，并以后用第二种 Agent 验证可移植性。
- 首先使用有限 exact outcome 和可控延迟，测量正确性、wall-clock speedup、ASCout overhead、hit rate、有效提前工作及 loser 资源成本。
- 正式开发策略与实验契约见 `docs/development/reference-workload.md`、开发决策 D-015-I 和路线图。

### Harness 与 runtime 解耦

- Harness 独占 Agent/product integration 与 workload policy；runtime 独占受管 execution、Branch/resource 状态和原子机制转换。
- 两层从第一版通过 typed、粗粒度、可版本化的 command/event contract 解耦，不共享跨边界可变状态。
- 第一版允许共处 `ascout codex` supervisor，不强制拆成两个进程；具体 transport 始终由所选 Runtime adapter 决定。
- 正式结论见 `docs/design/overview.md` 与 `docs/design/decisions.md` 的 D-027；实现边界见开发架构和路线图。

### Hook 与 embedded adapter

- Command + Skill 是基础接入。
- Hook 与 embedded in-Agent adapter 是可替代的可选增强，不要求同时存在；优先 Hook，只有明确能力缺口才引入 embedded adapter。
- Runtime 正确性不依赖任何 integration profile。
- 正式结论见 `docs/design/overview.md` 与 `docs/design/decisions.md` 的 D-026；第一阶段范围见开发架构和路线图。

### Outcome-guided continuation branching

- Branch 是活的计算延续，不称为 Snapshot，也不引入 Snapshot 核心实体；backend 内部仍可采用 checkpoint 或 VM snapshot 技术。
- Runtime 在 pending-result boundary 从共同 execution base 实例化完整 Agent 进程树；parent 取得真实 outcome，candidates 按 Proposal 提前执行后续步骤。
- Branch 后的任意 parent 进程内存与 Agent context 不透明传播；文件、输入、outcome 和外部效果分别使用明确的状态语义。
- Speculation window 内 parent 只承担真实 operation、outcome 发布、受控 evaluation 和裁决，不与 candidates 同时推进相同后续业务；无 candidate 被接受时从真实 outcome 继续。
- Proposal 使用 exact/predicate/virtual-value compatibility；candidate 可以 conditional commit，但 unresolved premise 验证前不能 Accept。真实 operation 只执行一次。
- 正式结论见 `docs/design/overview.md` 与 `docs/design/decisions.md` 的 D-024、D-025；实现差距见开发路线图。

### Parent Agent 裁决与提前 Discard

- Parent Branch 中的 Agent 默认持有 direct-child resolver capability，并可显式委托给 Outer Harness；不引入 Evaluator World。
- Parent 可以在 live child commit 前主动 discard；Accept 仍要求 candidate 已显式 commit，runtime 线性化并发 commit/discard。
- Discard 不自动泄露 child partial state 或结果；commit summary/evidence 是可封存的显式受控通信。
- Not accepted 不等于 infeasible；metadata 区分 invalidated、rejected、cancelled、failed 和 superseded。
- 正式结论见 `docs/design/overview.md` 与 `docs/design/decisions.md` 的 D-022、D-023；新实现任务见开发路线图。

### Outer Harness placement

- ASCout 是管理完整 Agent execution 的 Outer Harness；Skill 或普通命令是通用接入方式，thin in-Agent adapter 是可选优化。
- Runtime 是 Branch 状态的权威来源，并在受控 barrier 上分支请求者所属的完整 Agent 进程树。
- Agent-facing identity 使用 `ascout context` 和 `ascout children`，不使用 `ascout status` 或裸 Branch ID。
- 正式结论见 `docs/design/overview.md` 与 `docs/design/decisions.md` 的 D-018、D-019 和 D-021。

### Agent-facing Branch context

- Runtime 内部保留稳定 Branch identity；Agent context 不要求暴露裸 Branch ID。
- Agent 看到动态 depth、lifecycle state、Proposal 和 capabilities，不能依靠 depth 推导权限。
- Branch context 在语义上包含 direct-child control view；默认只返回统计，详细 child metadata 通过 `ascout children` 按需查询。
- Child 操作使用 session-scoped opaque candidate handle；不暴露 siblings、transitive descendants 或 child local state。
- 正式结论见 `docs/design/overview.md` 与 `docs/design/decisions.md` 的 D-021；新实现任务见 `docs/development/roadmap.md`。

### 受管创建与 Launcher frontend

- 完整 ASCout execution 必须由 ASCout 建立受管边界后创建；事后 attach 不提供同等级的完整 Branch 语义。
- `ascout codex` 是第一阶段 launch adapter，不是目标设计限定的唯一入口；第一版可以只是通用 managed-launch 接口的薄入口。
- 第一阶段不实现其他 launch adapter，但 Codex 特定参数、TTY、Skill/Hook 准备必须与 execution/runtime core 解耦。
- 正式结论见 `docs/design/overview.md` 与 `docs/design/decisions.md` 的 D-020；未实现能力见 `docs/development/roadmap.md`。

### Branch 作为唯一核心实体

- 删除 Domain 状态实体，只保留递归的 Branch。
- `local state of A` 表示 A 直接拥有且不包含 children 的状态层。
- `subtree(A)` 表示 A 及其全部 descendants。
- 正式结论见 `docs/design/overview.md` 与 `docs/design/decisions.md` 的 D-015。

### 世界状态的初始分类

- 文件系统。
- 计算机本地的其他环境，包括完整 Agent 进程树、sub-agent、内存、进程关系和环境变量。
- 全局服务，包括数据库、远程服务，以及不属于 Agent 进程树的复杂本地后台服务；其有状态、无状态和不可逆请求统一属于这个服务边界。
- 正式结论见 `docs/design/overview.md`。

### Subtree commit 的策略归属

- Candidate 有活跃 descendants 时仍可提交完整 subtree。
- 接收方 Agent 决定是否接受，不由底层机制强制 bottom-up。
- Winner subtree 保留内部 descendants；只丢弃 winner 的 sibling subtrees。
- 正式结论见 `docs/design/overview.md` 与 `docs/design/decisions.md` 的 D-001、D-007、D-008。

### 输入直接传播

- 进入某个 Branch 的输入直接对其全部 descendants 可见。
- 每个 Branch 独立读取，互不抢占同一输入。
- 输入读取或处理进度不限制 subtree commit。
- 正式结论见 `docs/design/overview.md` 与 `docs/design/decisions.md` 的 D-016。

### Branch tree 术语

- 使用 `Branch tree` 表示 Branch 的派生关系；状态由各 Branch 的 local state 及 ancestor path 组合，不另称为 `state tree`。
- 正式结论见 `docs/design/overview.md` 与 `docs/design/decisions.md` 的 D-015。

### Latest-visible-write 的适用范围

- Latest-visible-write 只适用于具有覆盖语义的 branch-local state。
- 输入使用 fan-out 与独立读取位置；全局服务的外部效果另行定义。
- 正式结论见 `docs/design/overview.md` 与 `docs/design/decisions.md` 的 D-006。

### 目标设计与实现状态的文档分层

- README 与使用文档必须明确说明当前没有实现，不把目标语义表述为可用功能。
- 旧实现手册不留在 active documentation；开发文档只记录新实现架构、开发决策、路线图和当前验证证据。
- 已删除原型中的 parent 立即阻塞、bottom-up、输入 barrier、winner 压平、单线程 fork 或 fake network 测试模式均不构成 ASCout 设计或实现路线。

### 输入 commit gate 的目标语义

- Greenfield 实现不得要求 `read >= delivered` 才能 commit。
- 每 Branch 保留独立 delivered/read 位置；commit 不表示输入已经被语义处理完成。该能力当前尚未实现。

### Accept 后的 Branch identity 与树重定向

- Candidate 保留自己的 Branch identity，并替换 parent 在 Branch tree 中的位置。
- Parent 是 root 时 root 改指 candidate；否则 grandparent 改指 candidate。
- 旧 parent 与 candidate siblings 失效，winner descendants 保持原结构。
- Candidate 按各状态类别规则已经可见的 parent state、已验证 outcome 和 materialized effect 必须随 winner 延续；parent post-branch memory/context 不传播，也不 merge。

## 已记录，暂不展开

### Agent-facing control surface

- Routine Agent surface 暂时只保留 `context`、`children`、创建、`commit`、`accept`、`discard` 和 event wait 这些 operation placeholders；当前不定义参数、返回值、阻塞方式、event model 或具体命令拼写。
- 这里的“空操作”表示 control-surface contract 尚未展开，不把已经确定的 Branch lifecycle 语义改成运行时 no-op。
- 不提供独立 `publish-outcome`：`PublishOutcome` 仍是 Runtime lifecycle semantic operation，由 operation integration 自动调用。
- 不提供日常 `delegate-resolver`：需要 Harness 代为裁决时，delegation 作为 managed launch 或 Branch 创建配置表达。
- Command 是基础 integration profile；未来 Hook 必须调用同一 semantic API，不能形成第二套 lifecycle。
- 用户要求当前只记录，不继续讨论详细方案。

### 通用 Harness 框架

- 用户希望未来 Harness 成为更通用的框架：上层 Agent 可以应用任意 speculative 算法/方案，Harness 都能承接，而不要求修改算法、方案或 Agent。
- 该目标当前只记录，不展开细节；可能方向包括继续拆分策略层与中间层，或采用其他架构，但要等现有目标和源码基础达成后再讨论。
- 后续讨论必须继续遵守 Harness policy plane / Runtime mechanism plane 解耦，不能把可交给 Agent 或策略层的选择固化为底层机制限制。

### Harness 术语解释

- 后续逐一解释并收敛 policy context、obligation，以及真实进程、Workspace、Effect、lifecycle state machine 各自的实现范围。
- 用户要求先理解这些术语，再决定具体实现边界。

#### 第 1 项：target initialization（已收敛）

- 原“managed launch orchestration”名称废弃；该流程的实质是初始化单一 Agent 实例，定名 **target initialization**（与 scout 对应）。
- 范围：只覆盖 root Authority 实例（task 准入到 root Instance 落地）的建立；后续每次 fork SpecTask 不属于 target initialization（那是 Scheduler + Runtime Adapter 的 fork 路径）。
- 与 D-020 的关系：target initialization 就是“受管创建”边界（完整 ASCout execution 必须先建立受管边界再创建 root Agent；事后 attach 不具备完整 Branch 语义）的流程视角。
- 职责拆分：API Entry 只做薄入口（接收提交、返回 task_id、状态查询），不建 task；建 task 与准入归 Scheduler（准入四道闸所在）；policy profile 表与绑定判定归 Policy Engine；起 Instance 0 作为 root Authority 归 Runtime Adapter。
- 预算暂不考虑：从初始化流程中剥离，Scheduler 的预算职责整个挂起，需要时再单独讨论。
- 已收敛：暂不实现“按任务特征自动绑定 profile”机制。policy 绑定方式：用户指定 policy；不指定时使用默认 policy。

### ascout-harness 实现分层与讨论约束

- 用户明确要求 ascout-harness 分为三层：第一层对接 CLI，接收 CLI 调用并解析各项参数；第二层是 ascout-harness 的实现本身；第三层对接 Runtime。
- 用户希望详细讨论 Harness 与 Runtime 究竟以什么形式连接和交互。
- 当前讨论不允许编写临时代码；除非是为尚未开始实现的代码提供空函数，否则目标是直接形成可用代码。
- 当前讨论中由用户作判断，助手不得替用户作设计取舍结论。
- 用户进一步提出：第三层应是恒定层，负责两件事：规定 Runtime API，即提供一组语义 API，任何实现该 API 的 runtime 都可以被当作 ASCout runtime 使用；以及负责对接并维护与 Runtime 的连接。
- 用户指出该职责与此前预期中的 Runtime-Adapter 功能相似，当前需要继续澄清 Harness 第三层与 Runtime-Adapter 的关系，避免概念混淆。
- 用户说明其最初对 Runtime Adapter 的定位是一个中介层，由两部分组成：一部分是直接和 Harness 连接的不变层，为 Harness 保持一致、恒定的语义与接口，不因下层 Runtime 改变；另一部分是和 Runtime 对接的可变层，需要根据 Runtime 提供的能力组合实现 Harness 需要的约定 API，也是适配某个 Runtime 必须遵守的内容。
- 当前理解中，不变层接近用户刚提出的 ascout-harness 第三层；可变层接近 Runtime-side adapter / Runtime backend adapter，但具体命名与归属尚未最终收敛。
- 用户确认采用拆分命名：不变层称为 Harness Runtime Boundary，属于 ascout-harness 第三层；Runtime Adapter 仅指面向具体 Runtime 的可变适配层。
- 用户提出继续讨论原先所谓 CLI-facing Harness layer 的命名与归属：Agent 必须有低耦合 CLI 调用方式和高耦合函数/内嵌 built-in harness 调用方式；因此 harness-core 可能同时接收 CLI 与 function/system 两类调用，且 function 方式可能是 harness-core 提供的原生方式，CLI 通过调用 function 实现。
- 当前待澄清问题：是否仍需要一个名为 CLI-facing Harness layer 的层；或者 CLI 实现本身就是 CLI-facing adapter，而 ascout-harness 内部只暴露非 CLI 专属的 Agent-facing/function semantic API。
- 用户确认采用统一入口层方案：ascout-harness 内部保留 Harness Entry Layer / Agent-facing Harness API，CLI adapter 与内嵌 function/system 调用都先经 Local Controller 或对应 library adapter 进入该入口层，再进入 Harness Core；Entry API 本身不直接对接 Runtime。
- 该入口层不应被命名为 CLI-facing，也不应限制用户只能采用 ASCout 预设的原语形态；用户可以较自由地实现自己想要的 function call，只要最终进入同一套 Harness semantic lifecycle。
- 用户倾向于让自定义 function call 在 ascout-harness 外部封装，以保持更好解耦；同时提出希望用户能够自定义 speculative policy，而不是只能使用 ASCout 规定的一种策略。
- 当前待澄清问题：自定义 speculative policy 应作为外部 wrapper/policy 调用稳定 Harness Entry API，还是由 ascout-harness Entry Layer 提供 profile/extension 注册机制；用户倾向前者，但尚未最终定案。
- 用户倾向于将最小默认 no-speculation/sequential policy 放在轻量 profile/helper crate，而不是放进 ascout-harness core。
- 用户提出希望提供 helper，方便用户低编码或少量编码地定制自己的 speculative policy，而不必直接编写复杂策略代码。
- 用户询问为何最小默认 policy 候选是 no-speculation/sequential，以及这是否与 ASCout 的 speculative 场景相背离；用户同时认为该 baseline 可能有利于 ASCout 未来扩展新场景。
- 用户确认将该 helper 设计为 policy builder crate，用于帮助用户以低编码方式定制 speculative policy；该 crate 位于 ascout-harness core 外部，通过稳定 Harness Entry API 组合 lifecycle primitives。
- 用户认可 ascout-harness 可理解为 Agent-facing control plane / lifecycle orchestration layer：记录并组织 request/Agent 相关信息，附加 policy/correlation 等 Harness-owned metadata 后交给 Runtime；也记录并解释 Runtime response/event，剥离或附加信息后返回给 Agent/policy/CLI。
- 用户指出此前列出的 harness-core 内部部件仍需商榷；当前方向是先挑出最必要部件实现一个可用原型，其它部件后续添加。
- 用户认可 ascout-harness 原型的最小部件集合：Harness Entry API、执行/树级上下文、Operation Orchestrator、Harness Runtime Boundary、Minimal Event Handling。
- 用户确认 ASCoutAgent 只表示 Harness-side BranchTree。受管启动是否需要 Runtime-side execution handle，留待 Runtime 议题单独处理。
- 用户确认首个可用场景为 start + branch + wait event；当前先完成 start 与 Harness-side branch metadata，wait event 等 Runtime/event model 收敛后实现。
- 用户允许先定测试场景；按照仓库约定，测试场景需先定稿并得到用户同意后，才能编写测试与业务逻辑。
- 用户进一步要求：类似命名必须保持类似语义，避免开发者和用户混淆；如果 AgentTree 与 BranchTree 表达两个不同概念，则不采用这种相似命名组合。
- BranchView 是一个可能较好的名称，但每个名称都必须有精确定义后再确认。
- 用户指出 ASCout Run 与 RunContext 的概念仍有混淆；可能更应将顶层对外抽象称为 ASCout Agent，因为整个 tree 对外应该抽象为一个唯一 Agent。
- 用户修正 BranchTree 归属：BranchTree 应是 Harness 侧概念；Runtime 不应拥有整体 Tree 视图，只维护某个 Agent/节点的状态。某些 Runtime 可负责不同节点之间的状态传播，但这不代表 Runtime 知道或拥有整个 tree 视图。
- 用户确认顶层对象命名为 ASCoutAgent；它是对外唯一 Agent 抽象，Harness 维护其内部 BranchTree / lifecycle 视图。
- 用户进一步澄清：ASCoutAgent 仅是 Harness Core 提供的抽象、对象或视图，不是单独组件。
- 用户要求后续代码直接围绕 ASCoutAgent、Local Controller 和 Harness Entry API 推进。
- 用户确认 Agent-facing routine API 的上下文隐藏由 Local Controller 完成：CLI/library 不显式传 ASCoutAgent id 或 Branch id，Local Controller 根据自身绑定形成 resolved request；Entry API 不再从 caller context 重新解析。
- Trusted policy/admin API 可在后续作为 scoped handle-based 边界讨论；它不属于第一版 Routine Agent API，也不进入当前首个实现范围。
- `branch` 仍是当前 Branch 的 routine Agent API：每个结果总是 Local Controller 绑定 Branch 的 direct child；在完整 Runtime 语义中 Agent 可指定 candidate 数量和 Proposal，resolver delegation 可在 ASCoutAgent 创建或 Branch 创建配置中表达；返回给 Agent 的是 children summary，而不是裸 ASCoutAgent/BranchTree 选择句柄。
- child summary 暂不展开字段；当前只实现空的 ChildrenSummary stub，后续再讨论 summary 内容。

### Runtime atomic transition contract

- Runtime 内部仍需要明确 cross-backend atomic transition contract：Branch/Resolution barrier 已被命名为原子边界，Execution、Workspace、Input、Effect、Outcome 也被列为参与者，但尚未定义由谁协调 prepare/apply/abort、各阶段何时可见、失败如何收敛，以及 cleanup obligation 何时允许恢复 winner/parent。
- 当前建议倾向由 Runtime Core 拥有 transition coordination，backend 只实现 participant contract；这只是待讨论建议，不是已接受决策。

### Control session 与 Workspace identity 细节

- 后续需要讨论 typed control session 的 request identity、幂等、event ordering、取消/超时与断线语义。
- 后续需要讨论 Workspace revision/object identity 及并发 ancestor/descendant write 的线性化规则。
- 这两个问题分别与现有“身份与控制协议”“持久化与故障恢复”和 Workspace 目标语义衔接，等源码基础建立后再展开。

### Continuation branching backend 选型

- 后续比较逻辑状态重建、完整进程树 checkpoint/restore、microVM 和定制内核机制。
- Backend 可以内部使用 checkpoint 或 VM snapshot 技术，但不得把 Snapshot 提升为 ASCout 目标实体或改变 D-024/D-025 的 continuation/outcome 语义。
- 用户要求当前只记录，不在当前对话展开。

### Effect adapter materialization

- 后续需要定义 deferred、virtualized、deduplicated、compensatable、irreversible 和 speculative 阶段禁止执行的 semantic effect 如何记录、验证、materialize 或清理。
- 未实际执行的 effect obligation 必须随 subtree 保留，不能因 commit 遗失；实际响应中的 ID、时间戳等可能要求 virtual-value mapping 或重新验证。

### 身份与控制协议

- 后续考虑 logical branch identity、PID 生命周期、generation/lease、pidfd、错误响应、取消和超时。
- Parent loss 后的恢复宽限期、session lease 和 orphaned speculation group 最终清理策略尚待定义。

### 输入边界细节

- 后续需要定义输入的流/消息边界、partial read、EOF、阻塞行为和真实 FD 所有权。
- 已确定的 ancestor fan-out、每 Branch 独立读取位置和输入进度不限制 commit 继续成立；具体输入 backend 不得反向改变这些语义。

### 持久化与故障恢复

- 后续需要定义单个 ASCoutAgent control domain 内的 runtime/control 状态持久化与 WAL、Local Controller 异常退出恢复和 Branch 隔离环境内后台进程归属；ASCout 不引入跨 Branch tree 身份或恢复关系。
- Daemon 部署还需要定义 thread/connection registry、grace period 和确定性连接排空；这些机制需与 parent loss、session lease 和 orphan cleanup 规则一致。

### Harness Core 架构层定位

- 已确认 `ascout-harness` / `Harness Core` 在软件架构中属于 Runtime 之上的 Agent-facing control plane / lifecycle orchestration layer；按分层架构可类比为 application service / use-case orchestration 层。
- `Harness Core` 维护 `ASCoutAgentRegistry`、Harness-side `BranchTree`、策略选择、request correlation 和计划执行；它拥有 ASCout lifecycle 的控制面语义，但不拥有 Runtime mutable state、OS privilege、具体 backend 或 CLI/product 入口。
- `Harness Entry API` 是进入该 application/control plane 的稳定 semantic facade；`Harness Runtime Boundary` 是它调用 Runtime mechanism plane 的 outbound port；具体 Runtime adapter/service 位于该边界之下。

### ASCout Agent 与 Harness 目标定位

- 已新增 `docs/development/ascout-agent-positioning.md` 作为 ASCout Agent、ASCout Harness 与 Runtime 定位的专门设计记录。
- 已确认 ASCout Harness 可定义为 Agent 分支执行的通用控制底座；第一版实现仍以 Agent Speculative Execution 为主要目标，后续再扩展到更多 Agent 分支执行模式。
- 已确认 ASCout Agent 的心智模型是一整棵分支执行树；对用户呈现为一个完整 Agent，是为了隐藏 BranchTree 和分支执行语义，不表示它是单个 Agent 进程或模型实例。
- 已确认 Harness 成为更通用的底座并不改变其控制器本质；需要修正的是抽象边界、策略开放性、上层模式可承载性和 Runtime 工具化关系，而不是把 Harness 弱化为中立转发层。
- 已确认 Harness 拥有控制语义和编排权；Runtime 是 Harness 之下的机制工具，负责落实执行、隔离、资源、状态转换和清理，不决定 Agent 策略或整体 BranchTree 语义。
- 已确认 ASCout Agent 对外应暴露为像一个完整 Agent 一样可使用的 Root Agent 视图；Root Agent 是用户面对的主线视图，不限制内部 BranchTree 深度，也不禁止 Branch-local Agent 或 policy 在深层继续创建 child Branch。
- 已记录用户忧虑：频繁用户操作和调整 Root Agent 可能导致大量提前执行 Branch 被过早剪掉；后续设计不能把用户每次操作都默认解释为全局 Branch 失效事件。Root Agent 变化、Branch 兼容性、修复和剪枝机制暂不展开。
- 已确认 Harness 同时服务用户入口和 Agent/policy 入口；两者最终应进入同一套 Harness 控制语义，只是暴露细节、权限和返回内容不同。
- 已确认 Harness 后续可以有调度权，但第一版暂不展开；调度选择应由 policy 在 Harness 固定不变量允许的空间内规定。
- 已确认 policy 可替换但不能绕过 Harness 核心规则；Harness 固定树内身份、分支关系、single-winner、capability、受控 effect 和 loser cleanup 等基础不变量。
- 已确认 Harness 负责解释 Runtime response/event，把底层机制结果组合成 Agent/policy/用户可理解内容，并剔除敏感信息或隐藏实现细节；该职责属于上下文管理的一部分。
- 已记录 ASCout 需要提供连续 Agent 体验，但具体如何提供尚未确定，先作为待考虑内容。
- 已确认 Harness 与 Runtime 的闭环边界：Harness 决定语义动作，Runtime 强制机制完成，Harness 根据 Runtime 事件更新控制状态；不能只更新 metadata 后假装 Runtime 侧完成。
- 已确认第一版暂不考虑多 ASCout Agent 协调；一个 ASCout Agent 是独立分支执行树。第一版部署形态先按 central Harness service 理解。

### ASCout Agent 分支执行底座待补齐内容

- 已确认当前目标范围不是任意分支执行，而是树形、单一 winner、无通用 merge、同级分支默认不协作的 Agent 分支执行。未来若支持 merge、多个 winner、DAG 或同级协作，应作为新的执行模型单独扩展，不能反向破坏当前 ASCout 基础语义。
- 已确认 `ASCoutAgent + BranchTree` 是当前 Harness 的核心组织抽象，但 `BranchTree` 只负责树结构和所有权，不能单独承担完整的分支执行语义。

按重要程度，后续需要逐项收敛以下内容：

1. **树的底层原语与高层组合操作**：明确创建节点、连接 parent-child、解除子树、替换树边、设置节点状态、登记约束、发布信息和产生事件等底层原语；再说明 `branch`、`commit`、`accept`、`discard` 如何由这些原语组合而成。用户或 policy 可以组合高层操作，但不能绕过树内身份、single-winner 和 cleanup 不变量。
2. **Branch 执行状态与控制上下文**：明确 Branch 除了树关系之外还需要保存哪些执行状态、假设、能力、提交证据、未决约束、等待/暂停/失败状态和 Runtime 对齐信息。当前 `committed: bool` 与三种 disposition 只作为 skeleton，不能直接视为长期公共状态模型。
3. **信息与约束传播**：明确用户或 parent 向 descendants 的传播、child 向 parent 的 proposal/outcome summary、同级 Branch 不直接协作的边界，以及新约束导致保留、修复、挂起或 discard 的基本语义。用户新需求应作为树上的受控约束传播，不应默认变成全局剪枝操作。
4. **Resolution 与 canonical continuation**：明确 `accept` 是选择一个 candidate subtree 成为后续 canonical continuation，而不是简单宣称该 Branch 是客观 ground truth；明确 winner 保留自身 subtree、siblings 失效、旧 parent 如何被替换，以及哪些条件必须先满足。
5. **Harness 操作与 Runtime 机制对齐**：明确 Harness 决定语义动作、Runtime 强制执行机制、Harness 根据已确认事件更新 BranchTree 的闭环；解决 metadata-only lifecycle 与真实 Runtime lifecycle 的同名操作、原子性、失败和状态对齐问题。
6. **Policy 与用户自定义操作的组合边界**：明确 policy 可以选择什么、Harness 固定什么、Runtime 强制什么，并提供足够的 Branch context、事件、outcome 和约束输入，使上层能够组合出不同的 Agent 分支执行策略，而不是只能选择固定的 Branch 命令。

- 当前只记录优先级，不在同一轮同时展开多个问题。下一项从第 1 项“树的底层原语与高层组合操作”开始。

### 树原语与用户自定义操作

- 已确认底层树原语可以向用户/policy 开放组合能力，但开放的是受控 ASCout 原语，不是裸 `BranchTree` 或 `BranchNode` 的任意修改权限。
- 用户或 policy 可以组合 ASCout-aware operation program，例如创建节点、挂接 direct child、记录约束、替换边、标记状态或发布信息；Harness 必须在执行前和执行中校验同一 ASCoutAgent tree、capability、single-winner、loser cleanup、Runtime barrier 和 sibling isolation 等不变量。
- `branch`、`commit`、`accept`、`discard` 应视为默认组合操作，而不是唯一内建操作，也不是最底层原语。用户可以在相同受控原语之上定义自己的分支执行操作。
- Runtime 相关动作不能由用户原语伪造；用户提交的是 operation program，Harness executor 负责将其拆成 precheck、Runtime barrier/command、tree mutation、event publication、cleanup 和 finalize 等阶段。
- 未来需要设计 `OperationProgram` / `PrimitiveOp` / executor 之类的抽象；当前 `CoreStep` 只是 skeleton 内部 metadata plan，不应直接视为长期公开原语模型。

### Harness 通用底座差异审查

- 已确认“Ray-like 通用底座”的范围是支撑不同 Agent 分支执行策略的通用底座，不扩展为任意分布式计算平台；原详细审查记录 `docs/development/harness-generalization-gap.md` 基于已删除的 skeleton 实现，已随实现一并删除，审查结论并入 D-023-I 与 `docs/architecture.md` §4。
- 审查记录必须区分当前尚未实现的能力、可能限制扩展的设计风险、命名/语义冲突和有意暂不纳入第一版的范围，不能把所有差距都写成当前实现错误。
