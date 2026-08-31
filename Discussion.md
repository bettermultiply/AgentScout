# 待讨论事项

本文件是设计讨论队列，不代表其中内容已经成为设计决策。讨论完成后的结论应写入 `docs/design/`，实现细节再写入 `docs/development/`。

## 当前讨论

### Policy Decisions 命名与范围

- 已确认：`mental-model.md` §3.1 正式命名为 **Policy Decisions**，不再使用“轴体系”作为心智模型中的现行名称。
- 已确认：Policy Decisions 表示 Policy 可根据任务、上下文、资源和 Runtime 能力作出的可替换决策；当前列出的六项只是代表性内容，不是穷尽的固定接口。
- 已确认：六项按投机阶段映射：准入与权威启动、预测生成、拓扑展开与投机执行、权威获取与验证、处置与提交；机会发现与 Site 注册是前置接缝，不单独构成一项。
- 已确认：使用 `docs/design/policy-decisions.md` 作为专门设计文档，后续在其中扩展完整 Policy 决策面、阶段归属、依赖和与 Mechanism 的边界。
- 已确认：`mental-model.md` 不展开 Proposal、Authority、Equivalence、Disposition 等对象级细节；这些内容移入 `docs/design/policy-decisions.md` 的“语义对象”小节。心智模型中的原 §3.2 移除，后续章节顺延。
- 已确认：心智模型中的状态与效果章节改为高层的“状态与效果”，不展开 `EffectClass × StateDomain` 交叉表和后端机制；详细内容移入 `docs/design/effect-management.md`。
- 已确认：资源与准入不作为心智模型支撑模型中的独立章节。准入、调度和资源选择属于 Policy Decisions；资源隔离、配额、抢占、取消与清理属于 Mechanism；规范化资源记账属于 Ledger。`mental-model.md` 删除原 §3.3，Fidelity 章节顺延为 §3.3；资源机制和记账的既有语义仍由 `speculation-decisions.md` D-011/D-012 等设计决策承载。
- 已确认：Fidelity 与运行验证也不作为心智模型支撑模型中的独立章节。Fidelity 是由 Policy Decisions、效果管理、隔离能力和 Speculation Contract 共同约束的系统级导出不变量；shadow/replay 是 Mechanism/Ledger 使用的验证与校准基础设施。支撑模型仅保留 Policy Decisions 与状态和效果，Fidelity 的契约语义仍在前文 §2.3、设计原则和 `speculation-decisions.md` D-004 中说明。
- 已确认：支撑模型按 `problems-and-goals.md` 的三个问题组织：Policy Decisions 对应策略孤岛，状态与效果对应隔离碎片化，Speculation Contract 对应上下层脱节。`mental-model.md` 的 Policy Decisions 内容保持不变；状态与效果和 Contract 按对应问题组织。Speculation Contract 同时属于工作模型中的抽象分层和支撑模型中的跨层对齐支撑，不新增独立的第四类模型。

### Agent 感知与执行观察边界

- 已确认：AScout 的投机决策主体是控制面；控制面根据所绑定的 `Policy` 寻找、评估并选择投机机会。Agent 本体通常不负责投机编排。
- 已确认：AScout 以 best effort 为目标，默认和自定义 Policy 都应在正确性、隔离、资源和 Runtime 能力约束内尽可能改善整体执行表现；策略可以选择顺序执行，不承诺每个步骤都产生分支或必然加速。
- 当前待确认：Agent 默认可以完全无感知，但是否提供可选的感知、查询、提示或协作接口；这些能力不能成为生命周期正确性的前提。
- 当前待确认：通过检测 Agent 运行位置和拦截 Agent 命令是候选实现路径，不先固化为目标语义。需要先确定 AScout 需要观察和拦截的稳定语义接缝，以及命令未被适配或无法安全解释时的行为。

## 暂停讨论

本轮从项目定位重新梳理；以下此前讨论暂不展开，待定位与边界收敛后再回到队列。

### 控制面执行模型与 Policy

- 控制面拥有 AScoutAgent、Speculative Tree 控制状态、Policy、请求校验、操作编排与 Runtime event 解释。
- 控制面内部组件为 Entry API、Policy Engine、Effect Coordinator、Assumption Manager、Scheduler 与 Ledger；不再设置一个额外的总控 Core 组件。
- Policy 只能在 AScout 固定生命周期、single-winner、capability、obligation、隔离和 cleanup 语义内作选择，不能拥有或修改 Runtime mutable state。
- 需要 Runtime 强制的操作经 Runtime Adapter 的稳定 typed contract 到达 Runtime；控制面只能根据 Runtime 已确认的 event/result 更新 Speculative Tree 控制状态。

## 已解决

### 全局文档与讨论方法

- 已确认：`docs/mental-model.md` 是项目唯一的全局根模型，用于帮助用户建立并持续维持项目的全局视角；其他文档从 mental model 的节点按树形结构向下展开。
- 已确认：每次讨论最多从当前节点深入下一级，不在同一轮继续深入更下一级；更深的问题先记入 `Discussion.md`，避免因会随编码变化的细节无限推迟项目启动。
- 已确认：当前问题收敛后，继续讨论当前文档、转到其他文档节点还是进行其他工作，由用户决定，不自动进入下一文档节点。
- 已确认：讨论可按需启用项目发起人、导师、系统架构师和技术审查委员会四种视角，不要求同时启用；技术审查委员会在上层讨论中不得提前追问超出下一层的实现细节。
- 已确认：保留 `mental-model.md` 文件名，并把它的内容职责从“概念总览”调整为“全局认知框架”。
- 已确认：读完 mental model 后，用户应真正知道 AScout 是什么，理解它要解决的问题、可用场景、逻辑上的构成与设计思路、实际系统构成，并能维持全局视野。

### 项目定位与 Policy

- 已确认项目正式名称为 **AScout**，不再使用 `ASCout` 作为现行名称；后续正式文档、术语和新实现标识需同步采用 AScout。
- AScout 面向两类用户：普通用户显式启用后按原方式使用 Agent，由 AScout 在后台自动执行投机；Agent 开发者可以为自己的 Agent 编写或替换 `Policy`，有能力的普通用户也可以自定义 Policy。
- AScout 是可复用的投机执行底座，不提供某一种固定算法；它屏蔽底层编排、控制和 Runtime 适配，使不同策略建立在共同机制上。
- 现有问题是 Agent 投机算法零散、正交能力难以集成，且现有运行底座与投机策略脱节、缺少面向投机执行的通用优化。
- 已确认普通用户未指定 Policy 时，AScout 绑定内置默认 Policy；默认和自定义 Policy 都由控制面执行，并在 AScout 固定语义约束内选择是否、何时以及如何投机。

### 控制面与 Agent 接入边界

- 已确认控制面与 in-process Runtime 不绑定：控制面只依赖 Runtime Adapter 提供的稳定 typed contract，不携带具体 Runtime 类型。
- 控制面决定目标 Runtime；Runtime Adapter 负责目标解析、能力映射、连接和协议适配。替换 Adapter/Runtime 实现不应要求修改控制面的 semantic API。
- 用户要求 控制面 同时支持外部传入 speculation plan、Proposal、resolver intent 等策略输入，并提供一个默认的 no-speculation/sequential policy，为不支持 speculation 的 Agent 提供服务，也用于测试与 baseline。
- 用户澄清 Agent 接入 ASCout 有两种入口：内嵌时调用 ASCout 提供的函数/库接口；不内嵌时通过 ASCout CLI 以命令行形式操控。两者应进入同一套 控制面 semantic API，不形成两套 lifecycle。
- facade 只是对稳定统一入口的解释性称呼，不要求额外建立名为 facade 的组件；正式讨论优先使用 Agent-facing semantic API、library API 和 CLI integration 这些更具体的术语。
- 用户否定“CLI 作为 composition root 组装并选择 Runtime Adapter”的方案：不得通过依赖注入组合 控制面 与 Runtime，CLI 也不负责选择、创建或连接 Runtime。
- 当前只确认 CLI 是 Agent 接入入口；具体 connection protocol、Adapter discovery 和连接生命周期仍未决定。
- 已确认 Entry API、CLI/library integration 与控制面的连接边界：ASC-Daemon 负责上下文绑定，Entry API 不再做 context-bound resolution，也不直接接触 Runtime；Runtime Adapter 连接及 `publish_outcome` 路径暂不展开。
- 已确认控制面拥有 AScoutAgent 与 Speculative Tree 控制状态；Entry API 保持 stateless，不持有长期控制状态。
- 已确认外部 Agent 不应拿到 `ASCoutAgent` session object/handle，也不应感知当前属于哪个 ASCoutAgent 或 Branch；当前由 Branch-local ASC-Daemon 绑定并隐藏 ASCoutAgent/Branch context。
- 已确认内部 record 直接称为 `ASCoutAgent`，代表从一个 root Branch 衍生出来的一整棵 ASCout Agent tree；代码与文档术语应保持一致，继续使用 `Speculative Tree`，不引入 `BranchTopology` 等分裂命名导致后续遗忘和混乱。
- 已确认如果 `root` 只是单独保存一个 `BranchId`，则没有必要作为 `ASCoutAgent` 的独立字段存在；root 应作为 `Speculative Tree` 自身的不变量或入口查询，而不是在 ASCoutAgent 上重复保存。
- 已确认 `BranchNode` 本身必须维护直接 parent 与 direct children；parent 不一定是 root，grandchild 的 parent 应是直接 child。`Speculative Tree` 负责保存 root 与 node lookup，用于快速查找某个 node。
- 已确认 routine control request 的上下文边界在 ASC-Daemon：ASC-Daemon 将自身绑定的 ASCoutAgent/Branch context 附加到 typed request 后交给 Entry API；Entry API 保持 stateless，只把已解析请求交给 控制面，再由 控制面 派发到对应 ASCoutAgent 的 Branch-level operation。
- 已确认 `commit` 必须存在；accept 以 candidate 已提交为前置条件，具体 obligation validation 后续再收敛。
- 已确认 create child 不要求 parent 从未 discard 过 child，也不因当前 child 都失败或 parent 选择继续而禁止继续创建；只要当前 Branch 本身仍是可继续执行的 live Branch，它在后续状态/阶段仍可再次 create child。
- 已确认 parent continue 不需要单独 operation；Parent 想继续时主动 discard 所有 direct child candidate subtrees，之后 parent 继续向下执行并可进入新的 `branch` 阶段。
- 已确认普通 discard 清理整个 `subtree(child)` 并从 active parent children 中移除；accept 内部的 self-discard 不是独立终止操作，而是旧 parent 被 accepted child 替换后的 superseded/self-invalidation。
- 已确认 accept 是复杂 Resolution operation：它需要 discard 未接受 children，把旧 parent 的 parent 边重连到 accepted child；如果旧 parent 是 root，则 Speculative Tree root 改为 accepted child。Runtime 必须完成必要的状态传播、记录或 materialization 后再清理旧 parent/losers；控制面 Speculative Tree 只能表达拓扑与 metadata 转换，不能假装完成 Runtime state propagation。
- 已确认不引入单独的 root self-discard/terminate operation；root 被 accepted child 替换只是普通 accept 的 root case。
- 已确认 policy/helper 可以是 ASCoutAgent-specific：控制面 可以有默认 policy，但每个 ASCoutAgent 可以持有自己的 policy/helper 配置或绑定。
- 暂不引入 `RuntimeExecutionToken` 之类细粒度概念；资源 token、quota 或 runtime-specific handle 等细节不影响当前 控制面 可扩展性，后续需要时再设计。
- 因 Agent 运行在 Runtime 提供的 Branch 隔离环境内，Agent-facing CLI/library 调用需要经 Agent 接入侧通道到达控制面；上下文解析不依赖 Agent 自报的 AScoutAgent/Branch 标识。
- Branch 隔离环境内的 ASC-Daemon 持有该 Branch 专属的本地控制入口；ASC-Daemon 根据自身绑定的 Branch context 形成已解析请求，再交给 Entry API，由控制面派发到 AScoutAgent Branch-level operation。
- 已确认 Agent-facing control path 采用 `CLI/library inside Branch -> Branch-local ASC-Daemon -> Entry API -> 控制面`。ASC-Daemon 位于 Agent 接入侧；第一版使用约定端口的 HTTP Server。入口自身代表当前 Branch，不需要让 CLI 自报 AScoutAgent/Branch identity。
- 已确认 ASC-Daemon、Entry API、控制面和 Runtime Adapter 是不同概念：ASC-Daemon 是 Branch 隔离环境内供 Agent-facing CLI/library 调用的本地控制入口；Entry API 是不直接访问 Runtime 的统一语义入口；控制面拥有 AScoutAgent metadata 并负责后续编排；Runtime Adapter 的连接细节暂不展开。
- ASC-Daemon 的实现边界必须能在 Branch 隔离环境内独立运行，不能与控制面的长期 metadata owner 混在同一个部署假设中；第一版 HTTP Server 的具体模块/二进制布局由实现阶段确定。

### 一级架构分层（四侧）

- 已确认 ASCout 的一级架构按以下方向组织：`Agent 接入侧 -> 控制面 -> Runtime Adapter -> Runtime`。
- **Agent 接入侧**包含 Agent/product 以及 CLI、library、launcher、Agent integration、ASC-Daemon 和 Interposer 等接入职责；它把 Agent/product intent 送入控制面，不拥有 ASCout 全局控制状态或 Runtime 机制状态。
- **控制面**包含 Entry API、Policy Engine、Effect Coordinator、Assumption Manager、Scheduler 与 Ledger，拥有 AScoutAgent、Speculative Tree 控制状态、policy、校验、编排和 Runtime event 解释；它不拥有 Runtime mutable state、OS privilege 或具体 Runtime backend。
- **Runtime Adapter**是单一第三侧，对控制面提供稳定 typed operation/event contract，并负责目标 Runtime 的能力映射、transport 和事件规范化；不再拆成两个同层组件。
- **Runtime** 拥有节点执行状态、进程与资源归属、隔离、capability/obligation validation、barrier、原子状态转换和 cleanup；它不决定 Agent 策略、candidate 质量或整体 Speculative Tree 语义。
- 四侧是逻辑 ownership/API 边界，不直接决定 crate、进程、服务或部署拓扑。ASCout control 请求通常向下流动，结果和事件向上返回；跨侧不共享可变内部状态。
- Entry API、Policy Engine、Assumption Manager、Scheduler、Effect Coordinator 和 Ledger 是控制面内部组件；Execution、Workspace、Input、Effect 和 Outcome backend 是 Runtime 内部组件。Agent execution、Branch 隔离环境和 Instance 是 Runtime 管理的运行对象，不是额外一级架构侧面。
- 四侧分层是唯一完整一级架构；控制面编排与 Runtime 强制执行只说明两者的职责关系，不能省略 Runtime Adapter。Runtime transport、Adapter discovery、连接生命周期和具体 backend 仍不在本决策中展开。

### 术语收敛（第一轮）

- 已确认：投机执行底座就是当前系统（ASCout）本身的设计目标，不是另一套相邻系统的设计；`docs/design/decisions.md`（Speculative Tree 视角）与 `docs/design/speculation-decisions.md`（投机执行底座视角）是同一系统的两套设计记录，保持分离并建立术语映射表。
- 已确认：系统现行名称为 **AScout（Agent Scout）**；当前实现组织服从 D-024-I 的四侧一级架构。
- 已确认：Speculative Tree 是 ASCout 的核心控制结构，也是投机执行底座的数据结构。
- 已确认：同时保留 **Branch** 与 **Instance** 两个名称；Branch 扩展为整个子树（不再区分 `subtree(A)`），Instance 为运行时执行单元。二者对应关系的精确表述待收敛。
- 已确认：ASC-Daemon 承担驻 Branch 隔离环境的本地控制入口与消息旁路；Interposer 保持分离（请求路径 vs 本地控制入口/消息旁路）。
- 已确认：Reconciliation、Disposition、Equivalence、Authority Launch 等底层抽象继续作为目标语义语言。Entry API、Policy Engine、Assumption Manager、Effect Coordinator、Scheduler 与 Ledger 属于控制面内部；Runtime Adapter 是第三侧；Entry API 不构成独立一级侧面。
- 已确认：Policy Engine 是六类 policy 插件的宿主，负责加载期校验和运行期分发。
- 已确认：Scheduler 负责准入、预算、优先级、抢占与过期丢弃；§9.3 的 frontier budget / best-first frontier expansion 仍是算法机制名。
- 已确认：**Engine Plane 不在范围内**（D-013/D-011/D-024/D-028 已同步修订）：KV cache 层 commit、partial-token accept、引擎压力调度、sequence residency 等全部排除；跨层可见性降级为 API 可观测信号（延迟/限流/配额）；接缝 1 去掉 prefix cache 共享。札记中的双层分析保留为论文背景与未来自托管候选。
- 已确认：**Proposal** 表示投机预测值，**Assumption** 表示 Proposal 成立这一未验证前提；提交操作称为 commit / commit request。
- 已确认：**Site 与 pending-result boundary 是两种东西，并存**：pending-result boundary 是“真实 operation 结果未到”的执行边界（时机语义）；Site 是该边界上被标注可投机、实际启动投机的依赖边/节点（结构语义）。
- 已确认：**task 与 AScoutAgent 是一对一** —— task 是 session 中一次独立的 AScout 管理单元。AScout 为每个 task 建立一个 AScoutAgent 控制域，并在其中维护一棵 Speculative Tree；AScoutAgent 是该 task 对外呈现的单一 Agent 视图，也是控制面中的控制域。
- 已确认：**Branch Node ↔ Instance 1:1** —— Branch 表示 Speculative Tree 中的一个子树，是计算延续和投机世界的结构实体；Branch Node 是该子树的根节点；Runtime 中一个 Instance 承载一个 Branch Node，代表实际运行的单一 Agent。不再使用 `subtree(A)` 作为现行术语。
- 已确认：**Interposer ↔ Hook / effect runner 映射为同一概念** —— 数据面、请求路径上拦截四接缝；与 ASC-Daemon 分离（延迟敏感 vs 旁路）。
- **术语收敛完成**：本小节全部条目均已收敛，术语映射表已落盘为 `docs/design/terminology.md`；两套决策记录的分离关系与权威声明见该文档。

- 用户希望把此前零散讨论形成的特性与约束放进一个全局视角，共同绘制整个项目的架构图。
- 第一版草图记录于 `docs/architecture.md`，只综合已经接受的设计与开发决策，不新增 backend、协议或 lifecycle 语义。
- `docs/architecture.md` 已收敛为四侧一级架构、各侧 ownership 和一次控制闭环；生命周期、世界状态和研究组件推导留在各自权威文档，不再重复压进架构总览。
- 已确认以 `Agent 接入侧 -> 控制面 -> Runtime Adapter -> Runtime` 为架构主轴，同时保持“控制面编排、Runtime 强制执行”的闭环。
- 已记录文档收口规则：实现重新建立后，README、架构图、开发文档和使用文档中用于说明“当前从零开始/尚未实现”的过渡期状态表述必须移除或改写为普通实现状态；active documentation 不应长期保留过渡期 wording。
- 用户确认当前架构设计足以进入实现；更细节的设计等源码建立后再讨论，不在当前轮继续展开。

### 空实现目标架构

- 已确认新实现继续使用 Rust。旧 `src/**`、manifests、tests、examples、headers、构建产物和实验文件已经按授权删除，未读取、迁移或复用其中的过时实现。
- 已确认必须为 Execution、Workspace、Input、Effect、Outcome 和 Operation Integration 预留明确扩展点。
- 用户只参与关系到可维护性、性能与功能的重要高层权衡；命名、文件布局、普通胶水、测试组织和不改变高层边界的实现细节由 Codex 决定。

#### 六类扩展点的含义

- `Execution`：Runtime 内的 Branch execution backend boundary，负责完整 Agent 进程树及 sub-agent、内存/线程/进程关系、环境变量、FD、signal 与 branch-private kernel resources，并参与 barrier capture、实例化、suspend/resume、winner continuation 和 loser cleanup。它替换执行环境实现，不改变 Runtime 强制的身份、capability、资源归属和 lifecycle 语义。
- `Workspace`：Runtime 内的 Branch Workspace View backend boundary，负责为 root/child 建立可见 workspace、隔离 branch-local modifications、表达 ancestor visibility，并参与 Accept/Discard 后的 winner continuation 与 loser cleanup。它替换文件状态实现，不改变 D-006/D-009/D-031 语义。
- `Input`：Runtime 内的输入承载与交付 backend boundary，负责 ancestor-to-descendant fan-out、每 Branch 独立读取位置，以及 Branch 创建、Accept、Discard 时的输入连续性。TTY、pipe、stream 或 message backend 可以不同，不改变 D-016。
- `Effect`：Runtime 内面向全局服务的 semantic effect backend boundary，依据 observation surface/guarantee horizon 管理 effect obligation、once-only execution、defer/deduplicate/compensate/materialize/cleanup。它不是通用网络 transport，也不把 contract 外的 operational footprint 纳入保证。
- `Outcome`：OutcomeSlot、routing、blocking obligation 和 Accept gate 属于 Runtime core；可替换部分是把工具、CI、用户输入或远程服务等真实结果规范化并发布到 OutcomeSlot，以及接入相应 payload/validation data。它不允许 backend 绕过 core 直接改变 obligation 状态。
- `Operation Integration`：Agent 接入侧对 shell/tool operation boundary 的接入，负责识别或包装 operation、关联 command contract、建立 pending-result integration、确保真实 operation 只执行一次，并在结束后自动触发 `PublishOutcome`。Command wrapper、Hook 或未来 embedded adapter 都实现同一语义入口，不能承担 Runtime 隔离与生命周期强制。
- 已确认 OutcomeSlot/routing/obligation/Accept gate 属于 Runtime core，只有真实 outcome source/normalization 可以替换；Outcome backend 不能直接改变 core obligation state。

#### Workspace 之外的本地执行环境

- 现有 `Workspace` 术语只表示文件/目录及 command-visible workspace state；若把环境变量、进程树、端口和其他内核资源全部纳入 Workspace，会混合 D-031 文件视图与完整 Agent execution 的不同状态语义。
- 已确认增加第六类 `Execution` 扩展点，负责完整 Agent 进程树、内存/线程/进程关系、环境变量、FD 与 branch-private kernel resources、suspend/resume、barrier capture、winner resume 和 loser cleanup；`Workspace` 继续只表示文件/目录视图。
- 端口不是单一类别：Branch-private socket/port namespace 属于 Execution resource；对宿主或其他 Branch 可见的共享端口、配额或 reservation 属于 Effect observation surface；持久或不支持多实例的后台进程可能是 shared service/session，需要由 Effect/Operation contract broker，而不是强行复制。
- `Branch execution environment` 可以作为 Workspace、Execution、Input、Effect interaction 和 Outcome event 的聚合概念，但不需要成为另一个拥有状态与 lifecycle 的核心实体。

### 旧实现删除与文档切换

- 用户已明确授权直接删除全部旧实现；`src/**`、Cargo manifests、tests、examples、header、`target/`、实验产物和样例数据已经删除。
- 只描述旧实现的 lifecycle、proxy、runtime、control protocol、service、module 和旧测试矩阵文档已经删除，不建立 active legacy archive；Git history 是唯一历史来源。
- README、使用文档、开发架构、开发决策、路线图和验收文档曾统一切换为“当时无实现”的过渡期状态；目标设计文档继续作为语义权威。
- 本次授权不包含创建新 Cargo workspace 或实现 skeleton。正式开发选择见 `docs/development/decisions.md` 的 D-017-I。

### 安全与部署边界

- ASCout 的核心保证是高效 sibling non-interference，不要求 控制面 成为对抗恶意 Branch payload 的安全沙箱；Child Agent 是可信且知道自身 Branch role/context 的参与者。
- Runtime 强制维护 identity/capability、sibling isolation、进程与资源归属、loser cleanup 和 observation surface 内的 external effect mediation。默认 Runtime 优先效率、不提供 adversarial security；用户可以显式选择项目适配的 sandbox/hardened Runtime，不引入安全 guarantee negotiation。
- 一个 ASCoutAgent 恰好拥有一棵 控制面 Speculative Tree，ASCout 禁止跨 tree identity、capability、引用和 lifecycle operation。Runtime execution state 不在当前 ASCoutAgent 抽象中展开。
- 所需 OS privilege 只由 Runtime 或其内部 helper/daemon 持有；控制面、Agent 和 integration adapter 不持有特权。
- ASCout 只固定 typed operation-event contract；control transport 与 wire format 由面向具体 Runtime 的 Adapter 决定。
- ASCout 正确性所需的权威 lifecycle/obligation 状态与 typed events 必须存在，但持久、tamper-evident 的 security audit 不属于 core requirement；具体 Runtime 可按需提供，Runtime Adapter 只负责能力与事件映射。
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

### Speculative Tree 与 Agent execution activity

- Branch 在 Speculative Tree 中存在，与其 Agent execution running/waiting/suspended 正交；不引入 `lineage` 实体或专门术语。
- Child 不感知 ancestor PID、identity 或 liveness；outcome/lifecycle event 由 runtime 按 OutcomeSlot 和 capability 路由。
- Parent loss 后 child 不能自行成为 winner；只有 delegated resolver 可以继续，没有授权恢复路径时 fail closed。
- 正式结论见 `docs/design/overview.md` 与 `docs/design/decisions.md` 的 D-028；实现差距见 FAQ 和路线图。

### 首要验证 Workload

- 第一阶段使用 `ascout codex` + Command/Skill 在真实仓库执行 delayed-choice coding benchmark，并与 sequential baseline 对照。
- 确定性测试 Agent 只做 conformance；暂不深改 OpenCode，Command 路径证明收益后再增加 Hook，并以后用第二种 Agent 验证可移植性。
- 首先使用有限 exact outcome 和可控延迟，测量正确性、wall-clock speedup、ASCout overhead、hit rate、有效提前工作及 loser 资源成本。
- 正式开发策略与实验契约见 `docs/development/reference-workload.md`、开发决策 D-015-I 和路线图。

### 控制面、Runtime Adapter 与 Runtime 解耦

- Agent/product integration 属于 Agent 接入侧；控制面拥有 workload policy 与 Speculative Tree 控制状态；Runtime 独占受管 execution、Branch/resource 状态和原子机制转换。
- 控制面只依赖 Runtime Adapter的稳定 typed、粗粒度、可版本化 operation/event contract；各侧不共享跨边界可变状态。
- 第一版部署可以共处 `ascout codex` supervisor，不强制拆成多个进程；具体 transport 由面向目标 Runtime 的 Adapter 决定。
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

- Parent Branch 中的 Agent 默认持有 direct-child resolver capability，并可显式委托给控制面；不引入 Evaluator World。
- Parent 可以在 live child commit 前主动 discard；Accept 仍要求 candidate 已显式 commit，runtime 线性化并发 commit/discard。
- Discard 不自动泄露 child partial state 或结果；commit summary/evidence 是可封存的显式受控通信。
- Not accepted 不等于 infeasible；metadata 区分 invalidated、rejected、cancelled、failed 和 superseded。
- 正式结论见 `docs/design/overview.md` 与 `docs/design/decisions.md` 的 D-022、D-023；新实现任务见开发路线图。

### Agent 外部管理位置

- ASCout 从 Agent 外部管理完整 Agent execution；Skill 或普通命令是通用接入方式，thin in-Agent adapter 是可选优化。
- 控制面拥有 Speculative Tree 控制状态；Runtime 是 Branch 执行状态的权威来源，并在受控 barrier 上分支请求者所属的完整 Agent 进程树。
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
- 第一阶段不实现其他 launch adapter，但 Codex 特定参数、TTY、Skill/Hook 准备必须与 execution / Runtime core 解耦。
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

### Speculative Tree 术语

- 使用 `Speculative Tree` 表示 Branch 的派生关系；状态由各 Branch 的 local state 及 ancestor path 组合，不另称为 `state tree`。
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

- Candidate 保留自己的 Branch identity，并替换 parent 在 Speculative Tree 中的位置。
- Parent 是 root 时 root 改指 candidate；否则 grandparent 改指 candidate。
- 旧 parent 与 candidate siblings 失效，winner descendants 保持原结构。
- Candidate 按各状态类别规则已经可见的 parent state、已验证 outcome 和 materialized effect 必须随 winner 延续；parent post-branch memory/context 不传播，也不 merge。

## 已记录，暂不展开

### Agent-facing control surface

- Routine Agent surface 暂时只保留 `context`、`children`、创建、`commit`、`accept`、`discard` 和 event wait 这些 operation placeholders；当前不定义参数、返回值、阻塞方式、event model 或具体命令拼写。
- 这里的“空操作”表示 control-surface contract 尚未展开，不把已经确定的 Branch lifecycle 语义改成运行时 no-op。
- 不提供独立 `publish-outcome`：`PublishOutcome` 仍是 Runtime lifecycle semantic operation，由 operation integration 自动调用。
- 不提供日常 `delegate-resolver`：需要控制面代为裁决时，delegation 作为 managed launch 或 Branch 创建配置表达。
- Command 是基础 integration profile；未来 Hook 必须调用同一 semantic API，不能形成第二套 lifecycle。
- 用户要求当前只记录，不继续讨论详细方案。

### 通用 控制面 框架

- 用户希望未来 控制面 成为更通用的框架：上层 Agent 可以应用任意 speculative 算法/方案，控制面 都能承接，而不要求修改算法、方案或 Agent。
- 该目标当前只记录，不展开细节；可能方向包括继续拆分策略层与中间层，或采用其他架构，但要等现有目标和源码基础达成后再讨论。
- 后续讨论必须继续遵守控制面、Runtime Adapter 与 Runtime 的 ownership 分离，不能把可交给 Agent 或 policy 的选择固化为底层机制限制。

### 控制面 术语解释

- 后续逐一解释并收敛 policy context、obligation，以及真实进程、Workspace、Effect、lifecycle state machine 各自的实现范围。
- 用户要求先理解这些术语，再决定具体实现边界。

#### 第 1 项：target initialization（已收敛）

- **target initialization** 表示初始化单一 Agent 实例的流程。
- 范围：只覆盖 root Authority 实例（task 准入到 root Instance 落地）的建立；后续每次 fork SpecTask 不属于 target initialization（那是 Scheduler + Runtime Adapter 的 fork 路径）。
- 与 D-020 的关系：target initialization 就是“受管创建”边界（完整 ASCout execution 必须先建立受管边界再创建 root Agent；事后 attach 不具备完整 Branch 语义）的流程视角。
- 职责拆分：Entry API 只做薄入口（接收提交、返回 task_id、状态查询），不持有长期控制状态；task/准入与 policy 绑定由控制面内部 owner 负责；初始化 Instance 0 的语义请求经 Runtime Adapter 到达 Runtime。
- 预算暂不考虑：从初始化流程中剥离，Scheduler 的预算职责整个挂起，需要时再单独讨论。
- 已收敛：暂不实现“按任务特征自动绑定 profile”机制。policy 绑定方式：用户指定 policy；不指定时使用默认 policy。

### Runtime atomic transition contract

- Runtime 内部仍需要明确 cross-backend atomic transition contract：Branch/Resolution barrier 已被命名为原子边界，Execution、Workspace、Input、Effect、Outcome 也被列为参与者，但尚未定义由谁协调 prepare/apply/abort、各阶段何时可见、失败如何收敛，以及 cleanup obligation 何时允许恢复 winner/parent。
- 当前建议倾向由 Runtime core 拥有 transition coordination，backend 只实现 participant contract；这只是待讨论建议，不是已接受决策。

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

- 后续需要定义单个 ASCoutAgent control domain 内的 runtime/control 状态持久化与 WAL、ASC-Daemon 异常退出恢复和 Branch 隔离环境内后台进程归属；ASCout 不引入跨 Speculative Tree 身份或恢复关系。
- Daemon 部署还需要定义 thread/connection registry、grace period 和确定性连接排空；这些机制需与 parent loss、session lease 和 orphan cleanup 规则一致。

### 控制面 架构层定位

- 已确认 `ascout-harness` / `控制面` 位于控制面，承担 lifecycle orchestration；按分层架构可类比为 application service / use-case orchestration 层。
- 控制面维护 AScoutAgent、Speculative Tree、策略选择、request correlation 和计划执行；它拥有 AScout lifecycle 的控制语义，但不拥有 Runtime mutable state、OS privilege、具体 backend 或 CLI/product 入口。
- Entry API 是控制面的稳定 semantic facade；控制面通过 Runtime Adapter 发出请求。

### ASCout Agent 与 控制面 目标定位

- 已新增 `docs/development/ascout-agent-positioning.md` 作为 ASCout Agent、ASCout 控制面 与 Runtime 定位的专门设计记录。
- 已确认 ASCout 控制面 可定义为 Agent 分支执行的通用控制底座；第一版实现仍以 Agent Speculative Execution 为主要目标，后续再扩展到更多 Agent 分支执行模式。
- 已确认 ASCout Agent 的心智模型是一整棵分支执行树；对用户呈现为一个完整 Agent，是为了隐藏 Speculative Tree 和分支执行语义，不表示它是单个 Agent 进程或模型实例。
- 已确认 控制面 成为更通用的底座并不改变其控制器本质；需要修正的是抽象边界、策略开放性、上层模式可承载性和 Runtime 工具化关系，而不是把 控制面 弱化为中立转发层。
- 已确认控制面拥有控制语义和编排权；Runtime 负责落实执行、隔离、资源、状态转换和清理，不决定 Agent 策略或整体 Speculative Tree 语义；二者之间必须经过 Runtime Adapter。
- 已确认 ASCout Agent 对外应暴露为像一个完整 Agent 一样可使用的 Root Agent 视图；Root Agent 是用户面对的主线视图，不限制内部 Speculative Tree 深度，也不禁止 Branch-local Agent 或 policy 在深层继续创建 child Branch。
- 已记录用户忧虑：频繁用户操作和调整 Root Agent 可能导致大量提前执行 Branch 被过早剪掉；后续设计不能把用户每次操作都默认解释为全局 Branch 失效事件。Root Agent 变化、Branch 兼容性、修复和剪枝机制暂不展开。
- 已确认 控制面 同时服务用户入口和 Agent/policy 入口；两者最终应进入同一套 控制面 控制语义，只是暴露细节、权限和返回内容不同。
- 已确认 控制面 后续可以有调度权，但第一版暂不展开；调度选择应由 policy 在 控制面 固定不变量允许的空间内规定。
- 已确认 policy 可替换但不能绕过 控制面 核心规则；控制面 固定树内身份、分支关系、single-winner、capability、受控 effect 和 loser cleanup 等基础不变量。
- 已确认 控制面 负责解释 Runtime response/event，把底层机制结果组合成 Agent/policy/用户可理解内容，并剔除敏感信息或隐藏实现细节；该职责属于上下文管理的一部分。
- 已记录 ASCout 需要提供连续 Agent 体验，但具体如何提供尚未确定，先作为待考虑内容。
- 已确认四侧闭环：控制面决定语义动作，经 Runtime Adapter交给 Runtime 强制完成，再根据规范化事件更新控制状态；不能只更新 metadata 后假装机制操作已经完成。
- 已确认第一版暂不考虑多 AScout Agent 协调；一个 AScout Agent 是独立分支执行树。

### ASCout Agent 分支执行底座待补齐内容

- 已确认当前目标范围不是任意分支执行，而是树形、单一 winner、无通用 merge、同级分支默认不协作的 Agent 分支执行。未来若支持 merge、多个 winner、DAG 或同级协作，应作为新的执行模型单独扩展，不能反向破坏当前 ASCout 基础语义。
- 已确认 `ASCoutAgent + Speculative Tree` 是当前 控制面 的核心组织抽象，但 `Speculative Tree` 只负责树结构和所有权，不能单独承担完整的分支执行语义。

按重要程度，后续需要逐项收敛以下内容：

1. **树的底层原语与高层组合操作**：明确创建节点、连接 parent-child、解除子树、替换树边、设置节点状态、登记约束、发布信息和产生事件等底层原语；再说明 `branch`、`commit`、`accept`、`discard` 如何由这些原语组合而成。用户或 policy 可以组合高层操作，但不能绕过树内身份、single-winner 和 cleanup 不变量。
2. **Branch 执行状态与控制上下文**：明确 Branch 除了树关系之外还需要保存哪些执行状态、假设、能力、提交证据、未决约束、等待/暂停/失败状态和 Runtime 对齐信息。当前 `committed: bool` 与三种 disposition 只作为 skeleton，不能直接视为长期公共状态模型。
3. **信息与约束传播**：明确用户或 parent 向 descendants 的传播、child 向 parent 的 proposal/outcome summary、同级 Branch 不直接协作的边界，以及新约束导致保留、修复、挂起或 discard 的基本语义。用户新需求应作为树上的受控约束传播，不应默认变成全局剪枝操作。
4. **Resolution 与 canonical continuation**：明确 `accept` 是选择一个 candidate subtree 成为后续 canonical continuation，而不是简单宣称该 Branch 是客观 ground truth；明确 winner 保留自身 subtree、siblings 失效、旧 parent 如何被替换，以及哪些条件必须先满足。
5. **控制面操作与 Runtime 机制对齐**：明确控制面决定语义动作、Runtime 强制执行机制、控制面根据已确认事件更新 Speculative Tree 的闭环；解决 metadata-only lifecycle 与真实 Runtime lifecycle 的同名操作、原子性、失败和状态对齐问题。
6. **Policy 与用户自定义操作的组合边界**：明确 policy 可以选择什么、控制面 固定什么、Runtime 强制什么，并提供足够的 Branch context、事件、outcome 和约束输入，使上层能够组合出不同的 Agent 分支执行策略，而不是只能选择固定的 Branch 命令。

- 当前只记录优先级，不在同一轮同时展开多个问题。下一项从第 1 项“树的底层原语与高层组合操作”开始。

### 树原语与用户自定义操作

- 已确认底层树原语可以向用户/policy 开放组合能力，但开放的是受控 ASCout 原语，不是裸 `Speculative Tree` 或 `BranchNode` 的任意修改权限。
- 用户或 policy 可以组合 ASCout-aware operation program，例如创建节点、挂接 direct child、记录约束、替换边、标记状态或发布信息；控制面 必须在执行前和执行中校验同一 ASCoutAgent tree、capability、single-winner、loser cleanup、Runtime barrier 和 sibling isolation 等不变量。
- `branch`、`commit`、`accept`、`discard` 应视为默认组合操作，而不是唯一内建操作，也不是最底层原语。用户可以在相同受控原语之上定义自己的分支执行操作。
- Runtime 相关动作不能由用户原语伪造；用户提交的是 operation program，控制面 executor 负责将其拆成 precheck、Runtime barrier/command、tree mutation、event publication、cleanup 和 finalize 等阶段。

### 控制面 通用底座差异审查

- 已确认“Ray-like 通用底座”的范围是支撑不同 Agent 分支执行策略的通用底座，不扩展为任意分布式计算平台。
- 审查记录必须区分当前尚未实现的能力、可能限制扩展的设计风险、命名/语义冲突和有意暂不纳入第一版的范围，不能把所有差距都写成当前实现错误。

### 心智模型的层次结构

- 已确认：心智模型不是重要概念的平铺目录，而是帮助读者逐步建立系统理解的认知框架。
- 已确认：心智模型在定位之后，先介绍“投机执行过程与生命周期”，再介绍 AScout 的工作模型，最后介绍支撑工作模型的细节。
- 已确认：`mental-model.md` 中不再把投机边、轴体系、效果管理、资源调度、Fidelity 和 Speculation Contract 并列称为"核心模型"。
- 已确认：工作模型包括投机执行过程与生命周期、Policy/Mechanism、Speculation Contract 和投机边；支撑模型包括 Policy Decisions、状态与效果、Speculation Contract。三类支撑模型分别对应策略孤岛、隔离碎片化和上下层脱节；Fidelity 与 shadow/replay 不单独成模，分别作为契约导出属性和验证基础设施。Policy 语义对象与决策细节不在心智模型中展开，见专门设计文档。
- 已确认：`mental-model.md` 采用五层结构：1. 定位；2. 工作模型（投机执行过程、投机边、组合与嵌套、抽象分层）；3. 支撑模型（Policy Decisions、状态与效果、Speculation Contract）；4. 系统结构（四侧架构、核心对象与所有权）；5. 文档地图。
- 已确认：`mental-model.md` §3.3 先说明 Policy 与 Runtime Adapter 的跨层能力协商，再说明三类固定语义底线：控制语义、世界状态与效果语义、执行边界与资源所有权。§3.2 与 §3.3 分别直接回应隔离碎片化和上下层脱节。
- 已确认：`mental-model.md` §4.1 解释四侧架构并保留最简控制闭环；§4.2 解释 `task -> AScoutAgent -> Speculative Tree -> Branch -> Branch Node -> Instance` 及其所有权，不再设置独立的 Agent 接入与 Runtime 闭环小节，详细内容由 `architecture.md` 承载。
- 已确认：AScout 是整个底座的名字，不是控制面的名字。四侧统一为 `Agent 接入侧 -> 控制面 -> Runtime Adapter -> Runtime`；Mechanism 只用于 Policy/Mechanism/Runtime 抽象分层。
- 已确认：不再以 "Assumption DAG" 作为心智模型中的独立模型；多投机之间的组合语义（嵌套、未验证前提并集、级联失效、信息流约束）折叠进投机边与执行图描述，控制面组件 Assumption Manager（D-019）作为实现决策保留。
- 已确认：投机执行不是固定的线性流程，而是一组可组合的独立阶段。Policy 可以根据任务、执行对象、可用信息和 Runtime 能力选择、替换和组合阶段；被启用的阶段仍须遵守必要的数据依赖以及 AScout 固定的正确性、隔离、资源和副作用约束。
- 已确认：当前阶段集合为机会发现、准入判断、预测生成、过程执行、验证、处置与提交。其中机会发现包含按 Policy 观察 Agent 或其他执行对象并识别投机机会；投机工作（过程执行）可以是模型推理、命令执行、工具调用、远程请求或其他计算；验证包含获取权威依据并按 Policy 判定 Proposal 兼容性（无权威依据时改用 validator 等可接受性判断）；处置与提交按验证结果处理投机工作并把合格结果纳入后续执行。
- 已确认：权威依据采用一般表述，具体来源可以是真实执行、用户输入、环境反馈、远程服务或 validator。
- 已确认：不建立统一的 `formal-model.md`；每个模型的形式化定义、约束和示例直接放在对应模型文档中。
