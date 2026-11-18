# AScout 心智模型

AScout 管理的运行身份称为 run id，对应的保存位置称为“运行记录目录”。Agent 原生会话仍使用 session/session id，工作区仍为 workspace；run id 与 task id 分别独立，新用户 prompt 仍开始新 task。

AScout（Agent Scout）是面向 Agent 投机执行的通用底座。Agent 可以在不修改自身执行逻辑的情况下接入，但接入实现应尽可能保持修改面小；具体产品仍可能需要 Product Integration、Hook 或其他拦截适配。AScout 提供一套自己的 Runtime：它承载 Agent 的一步动作，使动作能够正常、正确地运行，并把动作对真实世界或 authority 的影响限制在允许范围内。Runtime 的具体隔离和执行能力可以通过不同方式实现，并不绑定某一种后端。

## 1. 定位 - 为什么需要 AScout

投机执行想要解决的问题是，Agent 在执行过程中经常需要等待一个慢结果——例如，远程 API、用户决策、CI 测试——它会阻塞后续推理执行，甚至主导整个执行流程。投机执行允许在结果尚未到达时按预测提前推进后续步骤，从而加速整个执行过程。而不同投机执行策略所依赖的不同的准入、预测、验证、隔离、取消与复用等能力，正是 AScout 抽象为统一机制的那组能力。

现有投机执行工作大多针对特定执行对象或特定状态域设计，能力与任务绑定，方案之间难以组合，也无法让不同任务选择不同策略。这些[问题](problems-and-goals.md)集中表现为策略孤岛、隔离碎片化、上下层脱节三个缺口。

AScout 通过统一的投机执行机制连接策略与运行时，承接不同策略在预测、验证、拓扑和处置等方面的差异，使策略能够独立选择和组合，而不绑定具体的执行实现。投机执行涉及的不同状态与效果也通过同一框架纳入管理，并由明确的契约约束策略要求与运行时能力之间的匹配；当运行时无法满足策略要求时，AScout 不会静默改变策略语义，而是拒绝执行或进行明确降级。

落在使用上：
- 端用户提交任务并得到结果，通常无需感知背后的投机分支。
- Agent 可以在不修改自身执行逻辑的情况下接入 AScout，并获得由 AScout 和 Runtime 共同提供的投机执行能力；具体接入方式应尽可能保持修改面小。
- 部署方负责选择和配置可用的 Runtime 能力及其接入方式，并根据部署条件选择可运行的 Policy。
- Policy 作者只需专注于算法设计，例如如何预测、如何判断命中、如何选择拓扑、成本和处置方式，不必负责底层机制的具体实现。

AScout 不是某个固定算法，也不要求每个动作都建立完整、昂贵的独立环境。它也不承诺每个任务必然加速；是否投机、何时投机以及如何投机，由 control plane 根据所绑定的 Policy 决定。Runtime 可以按任务需要共享、复制、写时复制、重建或恢复状态，并优先使用满足要求的低成本方式。投机策略能否执行以及能够达到什么保证，则取决于 Runtime 能力、运行时条件和 Speculation Contract。

当前发布路线聚焦 coding Agent：v1.0.0 交付开发者可以实际接入、编写策略并验证收益的通用底座；CUA 等更广泛 Agent 可作为 v2 方向。阶段目标见 [版本规划](development/roadmap.md)。

记录、保存和恢复属于正常 AScout；模型输出回放是系统外的验证工具，通过正常模型接口及用户输入接口驱动普通运行，不进入运行机制。具体边界见[记录与回放](development/replay.md)。

系统最终必须把 Authority 的结果物化到现实。Agent 完成本轮工作、开始等待用户输入时，task 完成；正常完成须在无存活候选且任务相关写入完成后物化，不为物化暂停执行。Runtime 向 control plane 报告物化结果；物化失败按操作失败处理，control plane 必须等物化成功后才能宣告任务成功。Agent 常用的 Ctrl-C 或退出均终止任务，取消、失败和终止须结束候选并等待相关写入完成后物化。Ctrl-C 保留程序和 session，留在对话中，待上述处理完成后可接下一轮；物化失败须显示失败，不能宣告成功。暂停任务由 AScout 提供独立于原 Agent 的 `/pause` 入口，明确继续命令为 `/continue`，恢复原任务执行，保留原树、候选和环境关系，暂停不意味着退出程序。暂停期间的普通消息先留着，不解除暂停，也不在暂停期间交给模型；必须先 `/continue`，之后再按原顺序交给模型。目标边界见 [Runtime 设计](design/runtime.md)，物化及终止／暂停入口对齐仍有实现缺口，见[恢复](development/recovery.md)。

文件系统物化按 Authority 增量最终视图直接覆盖受影响路径，保留未涉及路径，不检测或报告物化冲突；禁止复制整个挂载。任务完成须等待物化确认，完成后以新的受管增量基准继续执行。具体写回范围和属性保留要求见[物化提案](fs-backend/materialize-proposal.md)；组合 Runtime、工具接入和存储后端的验收状态见[当前实现](fs-backend/current-state.md)。

## 2. AScout 的工作模型

Agent 以 action 及其结果报告执行事实，action 不限于工具调用。Agent 与 Branch 的生命周期、Agent 对控制命令的回执、Runtime 的操作与环境事实分别归属各自产生方；control plane 据此更新 Tree，不以“发出命令”冒充“已完成”。

投机执行由一组可组合的阶段构成，而不是固定的线性流程：机会发现 -> 待选准入 -> 启动选择 -> 预测生成 -> 过程执行 -> 验证 -> 处置与提交。AScout 可以暂存已经准入但尚未启动的机会；Policy 在同一 AScoutAgent 内全部有效待选机会之间选择现在启动哪些，并可以调整已经运行或暂停的投机工作。Policy 可以选择、替换和组合这些阶段，但被启用的阶段仍需遵守必要的数据依赖和 AScout 固定的正确性、隔离、资源与副作用约束。投机执行可以基于尚未确认的 Proposal 提前开展后续工作，并在获得权威依据或其他可接受性判断后继续、Promote 或结束。Parent 得到真实结果后立即继续，候选验证可并行进行；合法 Promote 才停止旧 Parent。工具准入与结果兼容性由 Policy 结合实际能力判断，不由固定工具数量或结果类型代替策略决策。

Policy、Mechanism 与 Runtime 构成抽象责任模型：Policy 作者只需面对一组抽象的决策点；Policy 使用外部模型必须经宿主逐请求授予 capability，凭证由宿主持有，详见 [Policy Engine](design/control-plane/policy-engine.md)。Instrumentation 负责 Agent 内部会话和行为接入；Agent 与随附 Instrumentation 在 Branch 隔离环境外的可信宿主运行，由 Runtime 管理宿主生命周期。工作区文件访问、shell、受管子程序和效果在对应 Branch 环境执行，模型、内存和控制通信不绕经文件系统隔离；挂载切换与物化不要求重启宿主。Instrumentation 由与产品无关的通用层和产品专用的 Product Integration 组成——`admit` 的 `opportunity_pass`、`feasibility_pass`、`extent` 和 `effect` 由通用层求值，Product Integration 只把产品执行点接入并执行处置结果；会改写全局控制状态或必须在多个 Chance、Branch、Proposal 之间协调的其余 Decision（包括 `predict`）由 control plane 求值并落实，所缺的本地事实由 Instrumentation 计算后提交；Runtime 承载动作并提供隔离、资源和效果边界；control plane 负责全局编排、校验和裁决。Runtime Adapter 是 Runtime 的接入边界，不是与 Runtime 平级的另一套 Runtime。Speculation Contract 约束 Policy、control plane 和 Runtime 之间的可观察语义；Runtime 内部可以组合不同的执行和效果管理能力。

工具与模型推理均可 eager 并行启动权威调用，或 speculative-first 将权威调用挂在执行前、让独立候选先行。接受时 Promote 候选 session 并取消旧 Parent，不向旧调用填入候选结果；全部候选不可接受时原调用放行一次。上文 Parent 得到真实结果后继续的规则适用于已被放行的调用。

每个 session 的 AScoutAgent 拥有自己的一棵 Speculative Tree，Tree 属于 control plane。树拥有该 session 的投机控制状态，其方法检查合法转换；Policy 函数计算策略选择。Speculation Coordinator 可以协调多棵树，不保存具体 session 的第二份投机状态。同一棵树由消息和定时器触发顺序处理，耗时操作异步返回；Agent 的普通执行由 Instrumentation 负责。长期设计允许修改 Policy；当前首版同一 session 加载的 Policy 固定，不实现修改或切换，范围见 [Policy 开发说明](development/policy-engine.md)。

Instrumentation 的 ASC-Proxy 管理控制连接和独立的 Runtime 执行连接。control plane 建立 Branch 对 Environment 的明确授权并转交执行访问信息；工具执行直接进入 Runtime，结果、增量输出和取消终态由 Instrumentation 接收并报告，control plane 不代执行工具。

工作模型的完整权威描述见 [设计模型](design-model.md) §2。

## 3. 支撑模型

AScout 用三个支撑模型承接不同投机执行策略在预测、验证、拓扑和处置等方面的差异：

- **投机过程与 Policy Decisions**：Edge 是描述投机如何进行的抽象模型，给出通用过程形状，九类 Policy Decision 定义在它之上；它不是运行时对象，没有 id。Edge 的具体实例 Chance 是被 `admit` 接纳的投机机会：从 `admit` 通过时获得 id，直到验证后被接受或抛弃、过程收敛为止；一个 Chance 下可有多个 Proposal 和多个 Branch，运行时 Decision 作用于 Chance。候选 Branch 上再次被接纳的机会与无嵌套时同理，仍是 Chance 模型的具体实例。Branch 承载某次具体执行，Candidate 只是 Branch 角色。开发者可以根据任务、上下文、资源和 Runtime 能力，通过组合 Policy Decision 设计可替换的决策 Policy，并处理多次投机的组合与嵌套。
- **状态与效果**：把投机执行过程中需要保护的文件、进程、输入、结果和外部服务交互放入统一的管理框架。Agent Session 内部状态由 Instrumentation 管理；Runtime 提供动作承载和效果边界，根据需要组合隔离、暂存、延迟、阻塞、补偿或清理能力。External Effect Coordinator 属于 Runtime，与 network backend 协同处理本机隔离覆盖不到的外部 API。真实请求先暂存在本机；可模拟时返回明确标记来源的临时结果，不能模拟时在效果边界阻塞 Branch 并保留进展，待其成为权威后发出真实请求并继续，丢弃或前提失效则取消。control plane 只通知相应暂存请求释放或取消，Coordinator 无需为跨 Branch 命令去重掌握整棵树。
- **Speculation Contract**：约束 Policy、control plane 与 Runtime 之间语义对齐的契约；Runtime Adapter 负责把 Runtime 的能力、结果和事件接入这条可检查、可执行的路径，并让拒绝或降级原因可见。

支撑模型的完整权威描述见 [设计模型](design-model.md) §3；各模型的细节分别见 [Policy Decisions 设计](design/policy-decisions.md)、[效果管理设计](design/effect-management.md)、[Speculation Contract 设计](design/speculation-contract.md) 与 [control plane 设计](design/control-plane.md)。

## 4. 系统结构

### 4.1 责任边界

AScout 按以下责任边界组织，控制请求向下流动，结果与事件向上返回：

```text
Instrumentation <-> Gateway <-> 控制逻辑
UI         <-> Gateway <-> UI Projection <-> Ledger
Runtime    <-> Gateway（经 Runtime Adapter 契约）
```

这里的调用路径展示逻辑责任，不等同于进程或 crate。Gateway 属于 control plane，不独立部署，由 control plane 统一启动和停止；它负责通信如何接入、消息交给谁以及如何送达，不拥有 Tree、Policy 或投机编排。Instrumentation、UI 和 Runtime 通过统一 Gateway 通信边界接入，但三侧的消息处理责任仍由各自控制逻辑决定。Gateway 与 Runtime 采用网络通信；UI 同样不独立部署，由 control plane 启动和停止。这不规定必须同进程或自动打开浏览器。Runtime 是 AScout 支持的完整执行与效果承载系统；Runtime Adapter 只是它与 control plane 之间的接入边界。Adapter 只负责接入配置，环境及其网络配置归 Runtime，实际分配结果属于运行状态。各侧责任见 [架构](architecture.md)。

Runtime 通过接入边界接收控制请求，结果和事件沿相反方向返回：

```text
Policy Action -> control plane 编排 -> Gateway 送达 -> Runtime Adapter 接入 -> Runtime 执行
  -> typed result/event -> Gateway 送达 -> control plane 更新控制状态
```

Adapter Registry 属于 Runtime，负责登记、契约与能力声明校验、主动验证及独立验证证据，向 control plane 提供带验证状态的能力事实；control plane 据此匹配 Policy 并作出投机裁决。Registry 的归属不允许 Adapter 自行证明其声明成立。Agent 与接入进程的实际启动、监视和回收由 Runtime 负责。

Runtime 创建 Environment 并返回身份，control plane 独占绑定 `Branch -> Environment`，每个 Environment 只属于一个 Branch。建立初始环境不预定身份，派生仅指定父 Environment；创建前 Branch 可以无绑定。Runtime 的执行接入边界验证 `branch_id` 授权，内部 Backend 只使用 Environment identity，不接收或推导 Branch 身份；其内部以独立 `BackendConnection.id` 标识 Backend instance，并维护 Environment 与 Backend instance 的多对多关联。因而不同 Environment 可以分别选择 `fs1/process1` 与 `fs1/process2/network1`，而不把共享的 Backend 内部世界误认为共享 Environment 生命周期。Candidate 建立、Promote、暂停与恢复使用实际返回并保存的 Environment identity，不能以 Branch 或 authority 常量替代。

### 4.2 核心对象与所有权

```text
session
  -> AScoutAgent
      -> Speculative Tree
          -> Branch
              -> Branch Root
                  -> Instance
```

`task` 是 session 中 Agent 的一轮工作；Agent 完成本轮工作、开始等待用户输入时，task 完成。一个 session 对应一个 AScoutAgent 控制域和一棵 Speculative Tree，多轮 task 在该控制域中继续。建立初始 Agent 时，其执行单元称为 root Authority Instance。在运行过程中，随着投机执行的不断发生和结束，Authority Instance 不断变化，由此形成树形结构。树中的每个子树都称为 `Branch`。`Branch` 是树中的计算延续，也是投机世界的结构实体；Branch 对应的根节点称为 `Branch Root`。`Branch Root` 具有稳定 identity 并直接拥有该根的 local state；`Instance` 是由 Runtime 承载 Branch Root 执行活动的实际执行单元。逻辑 Branch 可以在没有立即建立独立 Instance 的情况下先存在并推进，直到其状态或效果要求独立 Runtime 环境。

对外，为只呈现一个单一 Agent，这个统一视图称为 `AScoutAgent`。端用户通过主对话直接操作当前有效的权威执行主线；另可只读观察候选 Branch 的运行状态与内容；候选内容与主对话分开展示，并随 Branch 的角色与状态显示其投机身份，以了解投机是否真实发生及其处理过程。观察不形成另一条用户对话，不表示候选已被接受，也不改变 Branch 的裁决权限或向其他 Branch 传播候选内容。一个 session 对应一个 AScoutAgent，一个 AScoutAgent 拥有一棵 Speculative Tree；control plane 拥有 Tree、Branch 关系、逻辑推进和实际环境对齐状态，已建立 Instance 的实际执行、资源和隔离状态位于 Runtime，control plane 只能通过 Adapter 观察所需结果。

control plane 在运行时以三类状态描述一次投机，它们分别回答不同问题：`ChanceStage` 属于 Chance，说明该投机机会从待选到收敛走到哪一步、最终是否被接受；`AuthorityState` 也属于 Chance，说明其权威值是否已请求和到达，与机会的运行并行；`BranchStatus` 属于 Branch，合并生命周期与运行状态，说明承载具体执行的 Branch 当前处于何处。三类状态不能互相替代。

输入侧的可用命令与提示由 Agent 侧声明，AScout 只发现、排版和转发，不复制 Agent 的命令语义或原生界面。用户端保持稳定的 AScoutAgent 交互入口，不绑定某个后端 Agent session。合法 Promote 改变该视图对应的权威执行及输入输出路由，不要求用户切换对话，也不以候选会话结束为前提；候选保留自己的环境、执行状态和在飞进展承接后续执行，接受与效果约束仍然有效，嵌套接受不能越过祖先约束。

状态交接正确是机制必须保证的条件，不能仅以路由切换或历史整理代替。最终保留单一权威 session，表示经过验证、用真实结果整理后的执行历史；它不再要求准确复原候选当时收到的预测输入。因此，原始运行上下文与从整理后历史恢复的上下文可能不同。历史整理不决定权威，也不直接替换运行中的上下文；具体语义见 [Agent 接入设计](design/agent-integration.md) §3。

暂停后的保存与恢复保留整棵有效树、各 Branch 的原运行上下文与环境关联；保存成功后才能发布恢复点，发布前须校验各组件之间的身份和引用关联，恢复校验关联、配置及资源后等待用户继续。保存源与后续可写执行独立，继续运行不能改坏既有恢复点；最终历史不能替代运行恢复状态。文件系统恢复保留受管增量，但普通宿主 bottom lower 使用恢复时内容，不承诺宿主历史版本，也不复制整个挂载；代码、配置和依赖绑定仍需校验。现有保存恢复能力及入口调整缺口见 [恢复](development/recovery.md)。

AScout 管理稳定交互入口、显示区域与自身控制事实；Pi 等 Agent 的消息、思考、工具调用和结果通过公开 SDK 报告给 AScout，再由 AScout 的 UI 排版展示，不嵌入 Agent 产品的终端画面。浏览器 UI 与 TUI 共享事实和操作接口，但彼此不依赖绘制实现。用户可从当前 AScoutAgent 的整棵 Tree 查看 Branch 关系与投机状态，并选择具体 Branch 深入只读观察；选择观察对象不改变权威路径或 Branch 生命周期。

观察以受限的只读事件接口提供：首次取得状态事件集，随后接收有序增量并支持断线补发；客户端折叠事件得到展示状态，对话内容按 Branch 按需读取。接口不直接公开 control plane 的内部 Tree。详细语义见 [control plane 设计](design/control-plane.md) §5。

UI Projection 按序号读取 Ledger 已保存记录，持有可丢弃的界面缓存与连接读取位置；HTTP 和终端界面均经它读取。它不拥有 Tree、不裁决投机，也不向 Ledger 写回画面状态或报警。系统运行日志由 control plane 进程持有，独立保存警告和错误；Ledger 写入失败报告历史不完整；已退出 Tree 的 Branch 归档首次失败后安全恢复追加边界并重试一次，最终失败先记录 run id、Branch id 与原因、经 UI Projection 报警，再释放记录。Branch 控制状态不变，报警由 UI Projection 转发。日志只允许配置路径，启动时打不开即拒绝启动，不轮转；详见 [control plane](design/control-plane.md)。

## 相关文档

- 问题与目标的完整论证见 [问题与目标](problems-and-goals.md)
- 抽象概念模型见 [设计模型](design-model.md)，逐条决策的权威出处见 [设计决策](design/decisions.md)、[投机执行底座设计决策](design/speculation-decisions.md) 与 [Branch 语义设计](design/branch-semantics.md)
- 系统结构见 [架构](architecture.md)
- 术语表见 [术语](design/glossary.md)。
