# 待讨论事项

本文件是设计讨论队列，不代表其中内容已经成为设计决策。讨论完成后的目标语义应写入 `docs/design/`，实现细节应写入 `docs/development/`。已经下沉到权威文档的历史讨论不在此重复保留。

## 当前讨论

### SOW 1.2.2 投机决策模型

已收敛：SOW 1.2.2 采用“投机决策模型”作为技术方法名称。本节按“投机语义对象的统一建模、策略决策面的组合化表达、策略意图的受管闭环落实”组织，形成“对象—决策—落实”的连续结构。

已收敛：对象建模以待决结果边界、Edge、Proposal、Branch、Authority/validator、Speculative Tree 和未决 Proposal 依赖统一表达单候选、多候选、连续及嵌套投机；典型等待真实结果流程不再表述为所有投机都必须遵循的固定线性骨架。

已收敛：SOW 1.2.2 中的技术名词必须在第一次出现时完成必要定义，不能先使用、再用后文补充例外或修正含义。对象本身作为句子主语，避免以“项目如何做”为重复句式；只保留理解对象关系所必需的解释，不再额外复述已经由定义表达清楚的区别。

已收敛：策略表达覆盖九类 Policy Decision 及其阶段映射，明确阶段是职责视图而非固定调用顺序；breadth、嵌套 depth、树形展开方式和 Fidelity 均由决策组合产生，不另设为孤立的固定算法开关。

已收敛：九类 Policy Decision 在 SOW 中按“机会与运行时机、投机展开方式、验证与处置”三组解释，每组单独成段；先说明阶段视图和非固定顺序，再说明各组职责，最后说明 breadth、depth、拓扑和 Fidelity 如何由决策组合产生。

已收敛：受管落实说明 Agent 接入、本地与全局 Decision 分工、事件驱动控制闭环及 Policy/Mechanism 边界。1.2.2 只说明固定约束由底座强制，不展开具体隔离、效果或后端机制；候选内部状态和外部效果的治理留在 1.2.3，策略要求与 Runtime 能力的声明、验证和降级留在 1.2.4。

已收敛：受管落实按“Policy 不拥有运行状态、Agent 执行接入、本地与全局 Decision 分工、事件驱动闭环、固定约束与章节边界”分段，避免在同一段混写责任边界、接入方式和控制流程。

已收敛：SOW 1.2.2 的最终正文采用 `1.2.2.1` 投机语义对象的统一建模、`1.2.2.2` 策略决策面的组合化表达、`1.2.2.3` 策略意图的受管闭环落实三级标题；正文以用户确认的版本为准。

### SOW 1.2.3 行为状态治理

已收敛：SOW 1.2.3 以“确定治理边界、形成处理方案、完成状态收敛”为分类依据，不再把本地状态、外部效果、资源和失败处理作为四个并列类别。对应三级标题为 `1.2.3.1` 治理边界的确定、`1.2.3.2` 状态变化的差异化处置和 `1.2.3.3` 状态生效与失败收敛。

已收敛：治理边界依据变化的归属和可见范围确定，区分 Branch 本地状态与能够影响 Branch 外部可观察行为的共享或外部影响。Outcome 和运行消耗不作为与二者并列的主要类别，只在需要受控交付、被 Policy 明确约束或改变保证范围内业务行为时进入相应治理路径。

已收敛：状态处置保留 `StateDomain × EffectClass` 二维模型，只说明 StateDomain 表示变化发生在哪里、EffectClass 表示变化能够如何收敛，不在 SOW 中枚举完整状态域、资源项目或实现清单。差异化处置直接承接本地状态隔离、外部影响控制和伪装执行。

已收敛：收敛部分只说明受管变化的 Branch 归属、可见范围、未决责任以及接受、淘汰和失败后的去向；控制状态必须依据实际完成结果更新，不能以删除候选记录代替真实清理。

已收敛：SOW 1.2.3 必须按“Branch 行为产生变化—识别纳管范围—选择差异化处置方式—记录未决责任—随接受、淘汰或失败完成收敛”的因果链展开，使治理边界、差异化处置和失败收敛三个部分构成连续过程，而不是三个并列的分类说明。

已收敛：SOW 属于任务书，只说明行为状态治理的方法、作用、主要处理方式及其前后关系，不展开服务插件接口、内部组件调用或状态域实现清单。伪装执行需要在这一尺度上说明适用问题、基本过程、适用限制和验证约束：它拦截受支持的真实服务请求并生成有来源标记的模拟响应，使 Branch 在不产生真实外部影响时继续推进；模拟响应是未验证假设，不是真实 Authority 或自动成立的 Proposal，不能未经验证被 winner 或共享业务状态依赖。

已收敛：SOW 1.2.3 使用“处置方式”串联状态识别与最终收敛，不把“处理计划”扩展为覆盖 Branch 本地状态和外部效果的统一正式对象。`EffectPlan` 仅保留其在正式设计中作为 `effect` 决策返回对象的既有含义。

### SOW 1.2.4 投机执行契约

已收敛：SOW 1.2.4 按“后端能力验证、策略要求对齐、运行履行”组织，不再把运行确认和能力偏差拆成两个相互重叠的环节。对应三级标题为 `1.2.4.1` 后端能力的声明与接入验证、`1.2.4.2` 策略要求与已验证能力的统一表达和 `1.2.4.3` 契约履行与失配收敛。

已收敛：契约直接约束 Policy 的决策表达和 Runtime Adapter 在控制面边界上的可观察行为，不规定 Runtime 内部实现，也不承接 1.2.3 的外部影响治理。SOW 只说明策略要求、后端保证及其共同语义，不枚举完整环境内容、隔离属性和结果状态。

已收敛：Adapter Registry 通过实际环境操作验证声明并形成 Capability View，Policy Engine 据此检查策略是否存在合法执行路径；无法满足时拒绝或明确降级。运行期由 Adapter 的实际结果和事件确认契约履行，能力偏差、配置变化和部分失败均在同一环节显式收敛。

已收敛：异步运行记录、影子运行和轨迹回放只作为结果保真度、资源成本和契约履行的验证证据，不参与当次实时决策，也不单列为与契约表达、校验或履行平级的技术环节。

已收敛：SOW 1.2.4 按“Runtime Adapter 声明后端保证—接入校验形成可依赖的能力视图—Policy 产生显式能力要求并与能力视图统一对齐—运行结果持续确认履行—能力失配时收缩并收敛”的因果链展开，使后端能力验证、策略要求对齐和运行履行构成连续过程。

已收敛：SOW 1.2.4 属于任务书，重点说明投机执行契约解决的跨层错位问题、三阶段方法及其作用，不展开具体接口字段、校验用例或 Runtime 原生调用。能力声明只表示后端声称能够提供的保证，接入校验只确认存在合法执行路径，实际履行仍以运行期返回的结果和事件为准。

### SOW 1.2.5 技术路线

已收敛：SOW 1.2.5 按“系统设计完善—原型系统实现—实际负载接入与系统完善—效果评估、优化与学术成果形成”四个阶段展开，从技术方案、可运行系统逐步推进到实证结果和学术成果。各阶段必须说明主要工作、形成的阶段成果及其对下一阶段的支撑关系。

已收敛：实际负载阶段与效果评估阶段承担不同任务。实际负载阶段用于确认系统在真实 Agent、代码仓库和代表性任务中能够正确、完整且稳定地运行，并据此完善系统；效果评估阶段在系统版本稳定后，通过可比实验判断投机执行的收益、适用条件和资源代价，进行针对性优化和重新评估，最终形成可复现的实验结论与学术论文。

已收敛：技术路线不能只用整段文字概括阶段目标。四个阶段分别拆成多个按依赖顺序推进的小点，每一点说明具体工作及其为下一点形成的基础，使路线呈现“逐步实现、逐步接入、逐步验证和逐步形成研究结论”的实施过程，并与1.2.2至1.2.4侧重技术原理和方法的写法区分开。

### Speculation Contract 的定位与实现

已收敛：`Contract` 直接约束 Policy 的表达，以及 Runtime Adapter 在控制面接入边界上声明和返回的可观察行为，目的是保证策略与后端接入语义正确且对齐，而不是约束控制面或定义 Runtime 内部实现。控制面是 AScout 自身实现的 Mechanism 主体，是 Contract 的实现方，不属于被约束对象。Contract 不是部署配置或独立 Manager，不能由单个任务或 Policy 修改。

已收敛：`Mechanism` 只作为抽象责任词，是 Contract 的固定实现，不是插件或独立组件；它由 AScout 自身能实现、能强制的固定机制共同构成，由控制面组件协同完成检查与编排，不归属任何单一组件。

已收敛：Runtime 是通用执行后端（例如 docker、虚拟机或进程管理能力），不要求提供 AScout 专用接口。Branch 隔离、效果、cleanup 等 AScout 语义由 Runtime Adapter 用后端原生能力组合表达；控制面只能通过 Adapter 访问后端，因此接入校验的对象是 Adapter，Adapter 通过即代表其背后的 Runtime 能承载相应语义。Adapter 不能静默放宽语义，后端确实表达不出某语义时应拒绝接入或明确降级。

已收敛：Contract 的执行路径统一描述为 `Policy 决策 → Policy Engine 校验决策取值 → 控制面机制编排为受管操作 → Runtime Adapter 映射 → Runtime 执行并返回结果/事件 → 控制面更新状态`。Policy Decision 描述高层语义，取值合法性随后端能力变化，需结合 Adapter 报告能力校验；高层决策展开为受管操作序列，Adapter 按控制面接入契约映射为后端行为。通过 Adapter 的能力声明、接入时基本语义测试与运行期校验来判断其是否兑现后端能力；不直接校验或约束 Runtime 内部实现。

已收敛：失败语义。对尚未被接受的投机 Branch，失败收敛到"该 Branch 在 Speculative Tree 中从未被创建"，不重放该操作；被接受的 Branch 已完成使命，不适用。Contract 文档写共同承诺一句，撤销与清理机制归 control-plane。

已收敛：文档拆分。`docs/design/speculation-contract.md` 收窄为"定位 / 约束什么（单节）/ 如何约束 / 完整控制闭环"；控制面内部机制（组件分工、状态 owner、操作编排、失败清理）下沉到 `docs/design/control-plane.md`。同步更新 glossary、design-model、mental-model、index、effect-management、decisions 与 architecture 的相关引用。

已收敛：`spec-exec-abstraction.md` 不再保留独立的“控制面组件职责”章节。Agent Control Gateway 的目标职责补入 `control-plane.md`；Policy Engine、Speculation Coordinator、Effect Coordinator 与 Ledger 的现行职责以该文档为准。论文相关表格和数据继续作为事实依据，分别留在札记的 Effect、资源与准入、Shadow/Replay 等研究主题中；旧六插件、`site`、旧状态模型和重复贯穿示例不迁移。

### ASCout Agent 分支执行底座

当前目标范围是树形、单一 winner、无通用 merge、同级分支默认不协作的 Agent 分支执行。后续若支持 merge、多个 winner、DAG 或同级协作，应作为新的执行模型扩展，不能反向破坏当前基础语义。

已收敛：端用户只通过 AScoutAgent 视图直接操作当前有效的权威执行主线，只能看到该路径产生的内容、状态与交互。投机 Branch 不建立另一条用户对话，也不能携带未决 Proposal 直接向用户输出或提问。用户输入或中断由当前有效主线接收后，仍可按输入传播规则交付给运行中的 descendants。

已收敛：Branch 接受、丢弃、重新挂载与清理不需要端用户参与。Policy 与授权 resolver 形成决策或处置意图，控制面编排，Runtime 经 Adapter 落实状态转换。权威路径变化后，用户继续操作更新后的有效主线，不选择或重绑定 Branch。

已收敛：用户阻止当前权威路径上的操作时，依赖该操作继续发生或依赖其原结果的 Proposal 前提已经变化。现阶段不做兼容判断、保留、挂起、修复或重新对齐；凡识别出 Proposal 前提变更，所有依赖它的 Branch 及其 subtree 一律剪枝、取消并清理。未依赖该前提的 Branch 不因无关输入而全局失效。

下一项只讨论以下第 1 项，其他项目暂不展开：

1. **Branch 执行状态与控制上下文**：明确 Branch 除树关系外需要保存的执行状态、假设、能力、提交证据、未决约束、等待/暂停/失败状态和 Runtime 对齐信息。当前 `committed: bool` 与 disposition 只能视为临时 skeleton，不能直接视为长期公共状态模型。
2. **信息与约束传播**：继续明确用户或 parent 向 descendants 的传播、child 向 parent 的 proposal/outcome summary，以及同级 Branch 不直接协作的边界；Proposal 前提变更导致受影响 subtree 一律剪枝的当前语义已经确定。
3. **Resolution 与 canonical continuation**：明确 `accept` 选择的是后续 canonical continuation，而不是简单宣称 candidate 是客观 ground truth；继续明确 winner、旧 parent 和必要前置条件的关系。
4. **控制面操作与后端结果对齐**：明确语义动作、Adapter 可观察结果、规范化事件和控制状态更新之间的闭环，特别是原子性、失败和状态对齐；不定义 Runtime 内部机制。
5. **Policy 与用户自定义操作的组合边界**：明确 policy 可以选择什么、控制面固定什么、Adapter 必须证明后端落实什么，以及上层可使用的 Branch context、事件、outcome 和约束输入。

### Mental model 的后续维护

- `docs/mental-model.md` 是项目最高层的全局认知框架，默认稳定但不是不可修改的真理。
- 局部设计和实现不能静默反向改变 mental model；若目标、证据或错误假设要求改变全局模型，必须显式回到这里讨论，并检查直接子文档影响。
- 当前已收敛的 Agent 边界是：Agent 默认无感知，但可通过受控入口协作；AScout 面向规范化 operation 和 `pending-result boundary`，具体 Hook、CLI、命令拦截和进程观察方式不属于目标语义。
- 已确认：`Policy / Mechanism / Runtime` 是抽象责任模型，描述策略、机制与运行能力之间的抽象关系；`Agent 接入侧 / 控制面 / Runtime Adapter / Runtime` 是系统结构模型，描述责任、所有权和跨侧协作方式。两者是不同视角，不互相替代；Mechanism 横跨多个架构侧面，Runtime Adapter 是四侧结构中的连接边界。
- 已确认：不保留独立的 `design-principles.md`。原有四项内容不作为独立设计原则文档保留；跨领域设计取舍准则随 `design-model.md` 迁移，见下节「文档职责与 overview.md 解散」。
- 待同步（design-model 已重组，本节未改）：design-model §2 工作模型现只保留"投机执行过程"（§2.1）；原 §2.2 投机边 Edge（含决策面）与原 §2.3 组合与嵌套已并入支撑模型 §3.1，成为新的"Policy Decisions 与投机边"节。`mental-model.md` §2 中"工作模型包含投机边 Edge、组合嵌套形式化"及 §3 中"Policy Decisions"的描述需相应更新。

### 文档职责与 overview.md 解散

- 已确认：`docs/development/ascout-agent-positioning.md` 中的全局定位、四侧架构、对象 ownership、控制面职责、Policy 边界、第一版接口和长期方向已有正式文档承接；连续 Agent 体验和用户操作导致的 Branch 前提变更也已经收敛并写入正式设计。该开发文档已无独有内容，因此删除，不保留空壳或索引引用。
- 已确认：`docs/development/spec-exec-abstraction.md` 中“正交轴与取值域”整体迁入 `docs/design/policy-decisions.md`。本次先原样迁移，不同时修正其中的轴分类、论文证据、Fidelity 定位和 Mechanism/Policy 划线；这些内容后续在 Policy Decisions 文档中逐项整理。
- 已确认：`docs/design/overview.md` 是职责混杂的"抓包"文档，不保留为合并决策的总文档；其内容按主题归位，详见下方清单。
- 已确认：`docs/design/decisions.md` 与 `docs/design/speculation-decisions.md` 只是**速查手册与原始讨论记录**，记录决策当时的选择与理由，不替代正式主题文档，也不充当语义权威。每条决策的语义内容应分散到各自合适的主题文档。
- 已确认：新建 `docs/design-model.md`（设计模型），与 `docs/architecture.md` 平级、均为 mental-model 的直接下一级。`architecture.md` 指导**实际系统结构**（mental-model §4），`design-model.md` 指导**抽象概念**（mental-model §2 工作模型 + §3 支撑模型）。其下一级主题文档包括 `policy-decisions.md`、`effect-management.md`、`speculation-contract.md` 及待定的 Branch 语义文档。
- 已确认：5 条跨领域设计约束随 `design-model.md` 一起迁移，不再作为独立 `design-principles.md`。
- 已确认：`overview.md` 中"概述"段（"AScout 是什么"）删除，mental-model 已承担该职责。已执行的内容归位：Branch/Tree/可见环境/输入传播/隔离通信/Agent 感知/Workspace 归入新建的 `docs/design/branch-semantics.md`；隔离与 Runtime 安全边界归入 `architecture.md`；世界状态分类归入 `effect-management.md`；跨领域约束随 `design-model.md`。

### Edge 的定位与粒度（design-model §3.1）

- 已确认：**Edge 是一次投机机会**（`u` 的输出 pending、`v` 依赖该输出的可投机依赖点）；一个 Edge 上可有多个 Proposal / SpecTask / candidate Branch。它是**抽象模型**，不是注册实体。可投机性由效果纪律与决策点共同承载，不再使用独立的 admissibility 元数据。
- 已确认：**投机注册动作 = 在 pending-result boundary 上创建携带 Proposal 的 candidate Branch**（即 branch-semantics 的 branch 调用），不引入 site / "Edge 注册"作为额外注册实体；Speculation Coordinator 的 `(edge_id, proposal_id)` 依赖状态以 Edge 和 Proposal 标识关联，site 并入 edge 术语。
- 已确认：Edge 的职责定位 = ① 统一不同投机对象（LLM 调用、工具、命令、远程请求、人类回复）的抽象；② 作为设计 policy decision 与 contract 的作用对象；③ 局部视角（单次投机机会如何投机的决策点）。嵌套与组合是补充的全局视角（多次投机机会如何串联、共享前提与信息流）。
- 已确认：policy decision 多数挂机会粒度：权威启动（N 个候选共享同一 `A_u` 与真值 `i`）、BestOfN(breadth)、single-winner 处置都在 round 层面；逐 Proposal 的是 Predictor 与 Equivalence。机制侧证据：同一 Edge 下的 Proposal 共享权威依据并统一进入裁决，`edge_id` 与全局唯一的 `proposal_id` 各自承担机会归组与候选引用，§2.1 允许“一个或多个 Proposal”。
- 已确认：**设计模型决策面 = 组合视图，不与 §2.1 阶段流程逐段对应**。§2.1 阶段是流程视图（机会发现→…→处置），§3.1 决策面是组合视图（policy 真正被问的作用点）；决策面是 Policy 的落地接口。
- 已确认：**决策面为九槽元组** `S = (admit, select, rebalance, extent, predict, launch, effect, match, dispose)`。`admit` 以粗判和精判决定机会是否进入待选集合，不负责立即启动；`select` 面向同一 task / AScoutAgent 内全部已准入但尚未启动的 Edge，直接选择当前启动的有序子集；`rebalance` 面向同一 task 内已运行或暂停的投机 Branch，决定暂停、恢复或抢占。其余槽分别负责单次过程推进、Proposal、权威值取得、效果、匹配与处置。具体 depth、breadth 与排序方式由 Policy 表达，不是机制固定参数。
- 已确认：**authority 不再独立成槽**（D-041）。权威值来自谁是 Edge 的固有事实，不是可替换决策；`launch`（Eager / Lazy / Absent）决定是否取得、何时取得；Absent 允许放弃天然存在的权威值（含用户输入），是否允许由 Fidelity 契约校验；validator 归属 `match`。原"轴 1 / 轴 5（authority）拆开"的旧表述废弃。
- 已确认：**admissibility 并入 effect，不再作为独立概念**。原"Edge 注册与 admissibility"机制项、Edge 携带 admissibility 元数据、六实体表均已从 design-model 移除或改写；可投机性由效果纪律与 effect/准入决策承载。阻塞语义：投机执行遇到不可安全暂存/释放的外部效果时停在门口保留前置，accept 才释放、discard 即清理，而非事前整条拒绝。
- 已确认：**固定约束不进槽**（Mechanism 强制）：效果纪律与阻塞、隔离与信息流（非空 AssumptionSet 不进权威路径）、single-winner 与共享验证依据、注册必经 candidate Branch。**Fidelity 是导出契约不是决策槽**：由槽位取值推导上界、加载期对撞、运行期 shadow 审计。
- 已确认：原 §2.2 投机边与原 §2.3 组合/嵌套已并入支撑模型 §3.1；工作模型只保留 §2.1 投机执行过程。
- 连带待改（本轮已处理 design-model、policy-decisions、effect-management、glossary 与 spec-exec-abstraction 的相关正文）：speculation-decisions D-008 的"Edge 注册与 admissibility"机制项措辞；其他历史决策记录中的旧术语暂不改，待后续统一清理。
- 已知但本轮不展开的三条缝隙：流式/交互依赖（`u` 未产完 `i` 时 `v` 部分消费）、L3/Bypass 无真实 `i` 的措辞、sibling command reuse 与投机边的关系。
- 已确认文档归属：`design-model.md` 只保留九类 decision 的上层定义与组合/嵌套抽象；各 decision 的完整语义下沉到 `docs/design/policy-decisions.md`；效果到达处理边界时的阻塞、接受后继续和淘汰后清理由 `docs/design/effect-management.md` 定义；Contract 对 Branch 执行要求与 Runtime 能力的匹配由 `docs/design/speculation-contract.md` 承载。
- 已确认：Effect 不是 Edge 的一次静态可投机标记。一次投机执行可包含多个 step 与多种效果；Policy 选择处理纪律，Effect Management/Runtime 校验并落实。遇到不能提前释放或暂存的效果时，可以阻塞当前投机执行并保留前置进展；只有无法安全阻塞、接受后继续或淘汰后清理时，才拒绝相关环节的投机或明确降级。
- 已确认：`extent` 适用于所有投机执行，限制单次过程内部的推进边界；到达边界后暂停等待裁决，不自动 discard。它不同于嵌套深度：嵌套是在新的 Edge 上递归调用 `admit`，`extent` 不创建新的投机层级。
- 已确认：`extent` 是 Policy 根据过程事实动态计算的停止条件集合，不要求 AScout 通用估算未来接受概率、收益或恢复成本。`step` 是可被控制面观察、具有明确前后状态的 Agent 继续单元；Runtime 内部 token、系统调用和线程调度默认不是 step，等待 pending-result 不算持续推进，嵌套 Edge 拥有独立 step 计数。`extent` 指标收敛为三类谓词：`progress_stop(process_state)`（已观察进度，如 step 数）、`utility_stop(process_state)`（已实现且可观察的目标/效用或明确预算，不是未来收益预测）和 `semantic_stop(process_state)`（任务语义完成）；任一成立即表示 Policy 期望边界到达。裁决边界、特殊效果边界、Runtime 硬上限、取消和失败不属于 extent，分别由验证/生命周期、effect、Contract/Runtime 和运行控制处理；extent 不能绕过这些边界。
- 已确认：当前层只规定 `process_state` 的输入事实类别，不枚举无限增长的具体指标字段。必须可见的类别是过程进度事实（至少 step 数和阶段/位置）、已实现结果事实（输出、完成项、明确预算或资源计数）和过程语义事实（任务/Policy 定义的完成状态）。pending-result、effect 阻塞、Runtime/Contract 限制、取消和失败等控制事实可用于说明停止原因，但不是 extent 指标；具体字段、事件格式和领域扩展指标下沉到过程状态与接口文档。
- 已确认：去掉通用 `position` 字段。`process_state` 当前层确定的规范字段为 `steps_completed`、`phase`、`active_time`、`resource_counters`、`result_facts` 和 `semantic_facts`；后四项可按领域扩展，但必须保持为可观察事实，不能变成预测分数或机制推断值。
- 已确认：去掉通用 `position` 字段。`process_state` 当前层确定的规范字段为 `steps_completed`、`phase`、`active_time`、`resource_counters`、`result_facts` 和 `semantic_facts`；其中除前两项外可按领域扩展，但必须保持为可观察事实，不能变成预测分数或机制推断值。字段语义已在 `policy-decisions.md` 以表格列出。
- 已确认：`predict` 的输入契约包括 `edge_context`、`process_context`、`assumption_set`、可选 `history`、`policy_config` 和 `capability_context`。输出统一为 `ProposalSet`，支持单候选、有限候选集、带 Policy 元数据的候选集和增量生成；每个 Proposal 至少有 `payload`，可带 `provenance`。Proposal 是 Edge 边界的候选输入，可为原子值或结构化值，不是 Branch 或已执行轨迹；`rank`/`score` 只供 Policy 排序调度，不能被 Mechanism 解释为正确性或 `match` 结果。Proposal 数量是 breadth 的取值结果，无法生成有效候选时不创建空候选 Branch。
- 已确认：`match` 使用 `match(proposal, evidence, match_context) -> MatchResult`；`evidence` 可以来自 Authority 的真实结果，也可以来自 validator 的可接受性判断。`MatchResult` 至少区分 `compatible`、`incompatible`、`indeterminate`、`error`，并可带 `score`。分数由用户/Policy 规定函数计算并定义含义、范围、方向和阈值，AScout 只提供承载、记录和在同一 Edge 的相同匹配规则上下文内比较的机制；不把分数解释为正确概率，也不跨 matcher、版本或 scale 比较。确定性 predicate 可无分数，模糊或带容差关系可用分数；不确定或出错不得静默接受。
- 已确认：`dispose` 根据 `resolution` 和上下文返回可组合的 `DispositionPlan`，而不是单一终态。`resolution` 至少区分 `accepted`、`rejected`、`indeterminate`、`error`、`cancelled`、`preempted`、`extent_reached` 和 `failed`；处置意图包括 `Reuse`、`Promote`、`Salvage`、`Re-execute`、`Discard`、`Compensate` 和 `Overwrite`，可组合为例如 `Salvage + Discard` 或 `Compensate + Discard`。Policy 只表达处置意图，不重新判断 `match`、不决定继续推进；Branch 转换、清理、资源释放和 commit/discard 由 Mechanism 编排，并由 Adapter 返回足以确认后端已落实的结果。
- 已确认：`effect` 对每个 step 的已识别效果返回 `EffectPlan`，模式包括 `default`、`simulate`、`transform`、`stage`、`mediate`、`block`、`allow` 和 `deny`。`simulate` 是伪装执行：服务 Adapter 拦截请求，不访问真实服务，在 Branch 内返回明确标记为模拟结果的响应；该响应不是真实 Authority，必须记录生成来源和未验证假设，并由后续 Authority、validator 或明确规则验证。`default` 使用 Adapter/Contract 声明的默认方案，未声明时不得静默放行。`StateDomain` 与 `EffectClass` 保持正交，处理模式需经 Effect Management、Contract 和 Runtime capability 校验。
- 已确认：组合与嵌套当前只定义用户输入这一类跨 Edge 信息传播。用户中断也是用户输入的一种，和普通用户输入一样传播给当时仍在运行的全部 descendants；普通输入作为输入事实进入 Branch，中断触发各 Branch 独立停止/取消和清理。中断不通过 `AssumptionSet` 传播，也不引入集中式子树回收机制。
- 已确认：组合与嵌套的完整边界已写入 `policy-decisions.md` §4：同一 Edge 上的 Proposal/Branch 只是并行候选，由 `predict` 的 breadth、隔离和 single-winner 约束决定，不另设组合语义；嵌套 Edge 继续使用当前 task 已绑定的同一 Policy 及其 task-level 配置。`AssumptionSet` 只表达未验证依赖，跨 Edge 还通过树结构归属、用户输入传播和生命周期控制连接。祖先假设被否定时，控制面向受影响 Branch 直接发送取消命令，各 Branch 独立停止和清理，不设置集中式整棵子树回收机制。
- 已确认：AScout 暂存通过 `admit` 但尚未启动的 Edge；粗判识别可能的投机入口，精判只决定是否登记和保留，不决定立即生效。Policy 的独立 `select` 在同一 task / AScoutAgent 内的全部有效待选机会间直接选择当前启动的有序子集，可选一个、多个或暂时不选；未选机会继续等待，直到后来被选中或客观前提失效，不要求额外的主动放弃接口。Speculation Coordinator 维护待选集合及其有效性，并在资源、Contract 与正确性硬约束内执行选择，启动前不重复 Policy 精判，也不使用固定价值公式替代 Policy。独立的 `rebalance` 面向已运行或暂停的投机 Branch，决定暂停、恢复或抢占；`preempt` 以 `resolution = preempted` 进入 `dispose`。best-first、跨 Edge 排序公式、浅层分叉/深层单链与 breadth/depth 分配均为可选 Policy。
- 已确认：`spec-exec-abstraction.md` §9 压缩为“论文覆盖”“成本证据与候选 Policy”“正式边界、能力依据与研究问题”三节。Edge/Branch 生命周期、待选集合、`select`、`rebalance`、依赖传播和重新挂载不再重复定义；论文覆盖、命中率与能力表格继续作为事实依据，survival、深层误接受风险和不同决策组合的成本/保证继续作为待验证假设。
- 已确认：删除单独的 `Assumption` 概念，保留 `AssumptionSet` 作为 Branch 的未决 Proposal 依赖集合；Proposal ID 全局唯一，`AssumptionSet = frozenset[ProposalId]`。集合元素只是对尚未确认 Proposal 的引用，不是另一种实体；Proposal 确认时移除其 ID，否定时向受影响 Branch 直接发送取消命令。
- 已确认：`design-model.md` 的组合与嵌套只保留宽泛叙述，细节统一下沉到 `policy-decisions.md` §4。原有未决依赖形式化、反射 gating、`P(实例存活)`、frontier budget 和浅层/深层成本分析不保留在上层设计模型，也不未经重新讨论直接迁入 Policy Decisions；其中“候选 Branch 像普通执行一样继续运行并自然遇到新 Edge”补入 `policy-decisions.md` §4.2。
- 暂不验证“不同场景是否共享同一机制”；该项保留到 Edge、Proposal、Branch 关系收敛之后。
- 下一项只讨论 **Edge、Proposal 与 Branch 的关系**；`mental-model.md` 最后统一修改。

### 架构四侧与对称框架

- 已确认：四侧是完整调用路径中的逻辑位置，不表示四侧都由 AScout 实现或拥有。Agent 接入侧与控制面是 AScout 可以定义和实现的系统部分；Runtime 是 AScout 依赖但不能定义其内部架构的外部能力来源。执行、隔离、资源和效果最终由后端原生机制承载，但 AScout 只通过 Adapter 的可观察行为判断这些能力是否兑现。
- 已确认：AScout 能定义的是控制面接受的 Runtime Adapter operation/event 边界、能力声明和接入条件；不能控制 Adapter 内部如何调用 Runtime、组织模块或选择 transport。Adapter 的强制内容来自控制面接入契约，内部实现只适合给出开发建议与接入指南。
- 已确认：`architecture.md` 的直接子文档不按四侧机械地一侧一份。直接子文档包括 Agent 接入侧与控制面设计；Adapter 的具体实现与接入方式进入开发/接入指南；Runtime 不建立通用设计子文档。
- 已执行：新建 `docs/design/agent-integration.md` 作为架构的直接子文档，新建 `docs/development/runtime-adapter.md` 作为开发/接入指南；`spec-exec-abstraction.md` 中相应的现行职责已迁移，论文证据与 backend 候选分析继续留在开发札记。
- 已确认：Interposer / ASC-Daemon 的语义由控制面定义（决定它们做什么）；它们部署在 Runtime 创建的 Branch 隔离环境内，运行生命周期由 Runtime 管理（决定它们在哪里跑、何时启停）。该决定不推翻"ASC-Daemon 能在 Branch 隔离环境内独立运行、不与控制面长期 metadata owner 混同部署"的既有约束。
- 已确认：保留 `Agent 接入侧 -> 控制面 -> Runtime Adapter -> Runtime` 四侧调用路径，但不再将其解释为对称的四个 AScout 子系统或四份实现拓扑。控制面居中，Agent 接入侧是 AScout 自有接入边界，另一端经 Runtime Adapter 使用外部 Runtime。
- 已收敛：Runtime Adapter 的具体接入契约不并入 `speculation-contract.md`，单独建立 Adapter 契约设计文档；`speculation-contract.md` 只保留 Contract 的总体定位、Policy 与 Adapter 之间的关系，以及到具体契约文档的引用。`runtime-adapter.md` 继续作为面向 Adapter 作者的开发与接入指南。
- 已收敛：控制面新增 `Adapter Registry`，负责 Runtime Adapter 的接入登记、接入校验和已通过校验的能力信息发布。它是控制面内的固定职责，不是 Runtime Adapter 本身；Policy Engine 使用其发布的能力信息进行 Policy 匹配，Speculation Coordinator 不承担 Adapter 接入登记职责。
- 已收敛：通用 Runtime 只负责隔离执行环境及其环境级执行、资源和边界状态；完整 Agent 执行实体、Branch context、operation 归属和 capability 路由由 Agent 接入侧管理，Speculative Tree 与 Branch 投机语义由控制面管理。Runtime Adapter 接入检查不能把这些上层职责转嫁给 Runtime。
- 已收敛：Runtime Adapter 的基础门槛包括建立隔离环境、在环境内建立 Agent 接入侧并执行接入侧无权完成的环境级操作、停止和清理环境，以及继续使用仍有效的环境。隔离是贯穿所有环境操作的固定语义，不是可关闭的 capability；校验按进程、workspace、I/O 与通信、资源、权限、停止残留、cleanup 和继续状态分别进行。
- 已收敛：Adapter Registry 以接入时主动验证为主要证明手段，运行期持续验证只负责发现主动验证遗漏和实际偏差。能力状态分为已验证、未验证和验证失败；未验证能力默认不可用，但部署方可以主动确认并限制在指定部署范围内使用；验证失败不能通过确认绕过。
- 已收敛：`runtime-adapter-contract.md` 定义正式的能力声明、Registry 解析与验证、固定环境操作、结果/事件语义和能力降级；`runtime-adapter.md` 只负责依据该契约指导 Adapter 的实现、声明编写、主动验证、注册、排查和实现选择。两篇文档不再拆出第三篇 operation/event 文档。
- 已收敛：Adapter 声明描述当前实例、当前 Runtime 配置下的实际隔离保证，不描述理论最大能力。隔离内容按 `process`、`workspace`、`memory`、`I/O`、`network`、`resource` 和 `environment-local control surface` 分别声明，并使用 `visibility`、`modification`、`interference`、`lifetime`、`cleanup`、`scope`、`limitations` 等维度；`input`、`outcome`、Branch context 和外部业务效果分别属于受控信息流或 Effect/Service Adapter。
- 已收敛：Adapter 配置注册后不得运行期变化。若实际能力与声明不符，全部任务使用降级后的已验证范围；依赖失效能力的投机活动停止并清理，不依赖的活动可以继续；重新验证通过后才恢复新的能力范围。
- 已收敛：Adapter 契约使用形式化的 `AdapterDeclaration -> CapabilityView` 模型。声明描述当前 Adapter 实例和当前 Runtime 配置下的实际隔离保证；`ContentIsolation` 按 `content`、`visibility`、`modification`、`interference`、`lifetime`、`cleanup`、`scope`、`limitations` 描述；Registry 解析、规范化、主动验证并附加验证状态，Policy Engine 再将 Policy 要求与能力视图匹配。最终 wire schema 和属性枚举留待实现阶段，但必须保持该对象关系与验证语义。
- 已确认：`runtime-adapter.md` 中的工作表用于把 Adapter 的声明落实为 Runtime 机制、可观察证据和验证方案，为 Registry 的主动验证准备依据；它不是新的对外契约。没有设计依据或实际证据的“常见错误”清单不进入接入指南；实现部分统一说明 Adapter 作者可自行选择的实现方式。
- 已收敛：接入验证由 Adapter Registry 自主完成。Registry 建立验证环境、执行验证场景并观察结果；Adapter 只提交声明并提供建立验证环境、执行环境操作和观察结果所需的接入信息，不提交自测结果作为接入证明。`runtime-adapter.md` 不包含 Adapter 内部工程实现方式或 Adapter 作者自建验证方案。

### 控制面 §2 组件与状态的引入与整体职责

- 已收敛：本节（`control-plane.md` §2）的目标是阐明 §1 所述控制面职责在控制面内部如何落实，读者应能从本节回答：每一类责任与固定语义由哪个组件承担，以及每个组件自身的职责（即使不属于控制面的主要责任也要讲清）；各组件拥有/不拥有哪些控制状态（状态归属）；组件之间通过什么入口、按什么顺序协作，哪些修改必须走固定状态入口。
- 已收敛：本节是组件职责与状态归属的权威描述，但只定义目标职责、状态与协作边界；实现细节与论文依据留在 `docs/development/spec-exec-abstraction.md` 对应主题。
- 已收敛：不保留抽象的 owner 总则（如“有状态组件只能修改自己拥有的状态，无状态组件不能代替状态 owner 作决定”）。该约定是软件设计的基本纪律，不承载设计信息；信息应落在每个组件“负责什么 / 维护哪些状态、不持有哪些状态 / 与谁如何协作”的具体描述上。删除对象为 `control-plane.md` §2 引入中的该类总则句。
- 已确认：项目内其余 “owner/拥有” 表述若为具体边界（控制面与 Runtime 的状态归属、Policy 不拥有队列或运行状态、Effect Coordinator 不持有真实效果状态等），是有效信息，不属于要删除的抽象总则。
- 已收敛：Policy 由控制面统一加载、解析、校验和版本管理。所有不依赖或不改变全局状态的 Decision，可以通过 Policy Engine 生成的执行表示注入 Agent 接入侧执行，以降低高频路径上的通信延迟；接入侧必须记录本地 decision log，并把结果与日志交给控制面。控制面不重复执行已经满足本地范围条件的 Decision，但仍确认执行表示、Branch 上下文和结果格式有效。依赖或改变全局状态的 Decision，以及所有 Speculative Tree、Branch 生命周期、资源、效果和 Runtime 状态转换，仍由控制面负责。
- 待展开：Agent Control Gateway 的接收与传输方式属开发未定事项；可考虑进程内调用、本地 IPC 或网络，但同进程部署不是强隔离 Runtime 下的架构要求。既有原则是 typed operation/event 语义不随 transport 改变。
- 已收敛：以 Agent Control Gateway 替代 Entry API，作为 Agent 接入侧进入控制面的统一受管通信边界。Runtime 创建 Branch 执行环境后，Agent Control Gateway 在该环境内创建并启动 Agent 接入侧，二者共同形成完整的 Branch Node。Gateway 负责 Task 启动、Agent 接入侧会话，以及 Policy dialect、本地 Decision 结果、decision log、运行事件和受管控制请求的双向传递，并进行入口级上下文、权限和格式检查；除通信会话所需的暂时状态外，不持有控制面或 Runtime 的权威状态，不直接操控 Runtime，也不独立决定 Policy Decision 或 Branch 生命周期转换。
- 已收敛：Agent Control Gateway 不绑定网络接口。进程内调用、本地 IPC 或网络属于部署与开发选择；同进程部署不是强隔离 Runtime 下的架构要求。无论采用何种传输，Gateway 接受的 typed request/event 及其控制语义保持一致。
- 已收敛：以 Speculation Coordinator 替代 Assumption Manager，并吸收原 Scheduler 的控制面职责。Speculation Coordinator 作为全局投机视图与控制循环的协调核心，根据 Agent 接入侧和 Runtime 经 Runtime Adapter 规范化后的事件，以及当前依赖和 Branch 状态，激活受影响 Edge 的 Policy Decision；各专业组件形成相应的操作意图，所有 semantic operation 经统一的控制面操作出口交给 Runtime Adapter 执行，并由返回结果或事件更新全局视图。它不直接触发固定 Runtime 操作，也不重复执行已经满足本地范围条件的 Decision。
- 已确认：投机 Branch 的逻辑创建与独立 Runtime 环境的创建可以分离。采用 lazy 创建时，控制面先在 Speculative Tree 中登记和推进逻辑 Branch；只有在即将发生需要独立 Branch-local 状态的操作前，才请求 Runtime 建立独立环境。实际环境完成相应操作后，控制面再把逻辑视图标记为已与实际状态对应，并区分逻辑视图已推进但实际环境尚未对齐的阶段。
- 已确认：Branch-local 状态不因属于内存、进程上下文或其他完全可逆状态就一律要求强隔离。默认只保证 Policy 或 Contract 要求保护的状态域；未被要求保护且不会影响保证范围的可逆变化可以共享或忽略。若后续操作需要该状态按 Branch 独立延续，仍必须在操作前建立独立环境，不能把已经共享的状态事后假定为可分支状态。
- 已确认：Agent 接入侧负责在高频路径上识别即将发生的 Branch-local 变化，并在变化发生前请求 lazy 创建；未知或无法确认的操作按需要建立独立环境处理。Effect Coordinator 不负责普通 Branch-local 操作，而专门协调共享、外部及 Policy 要求纳管的效果，尤其通过可扩展的服务效果插件支持 `simulate`。插件只在控制面启动或 Policy 加载阶段注册；Task 使用的 Policy 生效后不热替换插件目录，具体注册接口和插件实现留在开发文档。
- 已确认：效果插件实际运行在控制面内，由 Effect Coordinator 统一注册、选择和调用。Agent 接入侧只在效果真正发生前识别命令或 URL，并将受管请求转交控制面；插件可以基于全局视图执行去重、统一审计及跨 Branch 的效果协调。`simulate` 等可在控制面完成的结果由插件生成后经 Agent Control Gateway 返回；只有需要 Runtime 提供暂存、代理、转换或其他执行机制时，才经 Runtime Adapter 请求 Runtime。
- 已确认：Ledger 在当前目标语义中只负责异步旁路记录控制面决策、运行事实及逻辑视图与实际执行状态的对应变化，用于状态对齐、问题定位、审计校验、shadow/replay 和分析；它不参与实时决策，不从记录中重新决定生命周期，也不承担控制面重启后的状态恢复。WAL、检查点、事件重放顺序和从 Ledger 重建 Speculative Tree 等恢复协议留在开发文档。

## 暂停讨论

以下事项已经记录，但要等当前分支执行模型或源码基础进一步稳定后再展开：

### 控制面执行模型与 Policy

- 已确认：Policy 是由 Policy Decision 及相关决策逻辑组成的静态定义，不是用户编写并直接参与运行控制的程序。控制面通过 Policy Engine 使用 Policy 作出运行时高层决策，再将决策编排为 semantic operation；机制没有固化 Policy 决策，一切策略选择仍依照 Policy。
- 已确认：控制面维护每个 Task 的 Speculative Tree 控制面视图。AScoutAgent 是这棵 Tree 向用户呈现的统一抽象，Branch 是 Tree 中的子树，三者不是并列且彼此独立的状态对象。
- 已确认：semantic operation 经 Runtime Adapter 请求 Runtime 执行；控制面收到结果或事件后校验操作是否按语义完成并更新 Tree 的控制面视图，同时记录运行事实用于状态对齐、问题定位和审计校验。
- 控制面内部包括 Agent Control Gateway、Policy Engine、Speculation Coordinator、Effect Coordinator 与 Ledger，不设置额外的总控 Core。
- Policy 只能在 AScout 固定生命周期、single-winner、capability、obligation、隔离和 cleanup 语义内作选择，不能拥有或修改 Runtime mutable state。

### Agent-facing control surface

- 已确认：当前不向 Agent 暴露主动使用 AScout 控制能力的控制面接口；Agent-facing control surface 不作为当前 Agent 接入路径。
- 不提供独立 `publish-outcome`；它仍是由 Interposer 自动触发的 Runtime lifecycle semantic operation。

### Interposer 与 Operation Integration 合并

- 已收敛：`Interposer` 与 `Operation Integration` 合并为一个统一责任实体，不再将二者描述为两个独立的 Agent 接入组件。
- 已收敛：Interposer 统一负责 Agent 请求拦截、规范化 operation 识别、`pending-result boundary`、本地 Decision 执行、真实 operation 的 once-only 纪律以及 outcome 自动发布。它可以通过拦截器、Hook、wrapper、effect broker 或其他产品适配机制实现。
- 已确认但未展开：端侧管理节点是否需要另一个统一的本地控制循环，以及 Interposer、ASC-Daemon 和产品 Integration 在实现层的具体组织方式，继续作为后续议题讨论。

### Interposer 的事件驱动处理

- 已收敛：Interposer 按拦截事件与控制面命令逐次处理 Agent 执行，并在需要时通过 ASC-Daemon 与控制面进行异步协作。
- 已收敛：不引入“本地控制循环”作为 Interposer 或端侧管理节点的设计概念。单次处理等待控制面结果属于该次 Agent 执行的处理过程，不表示存在持续运行的本地调度循环。
- 不提供日常 `delegate-resolver`；需要控制面代为裁决时，delegation 作为 managed launch 或 Branch 创建配置表达。

### ASC-Daemon 职责

- 已收敛：ASC-Daemon 是端侧管理节点与控制面之间的本地通信入口。它建立并维护双方的通信会话，同时证明当前通信来自对应的端侧管理节点，避免信息伪造。
- 已收敛：ASC-Daemon 负责传递端侧提交的事实、请求、Decision 结果和 outcome，以及控制面返回的命令、结果和事件。
- 已收敛：ASC-Daemon 不负责解析或路由 Branch 上下文，不执行 Agent 步骤或本地 Decision；这些职责分别由 Interposer 和端侧管理节点承担。
- 已收敛：ASC-Daemon 启动后与控制面建立通信会话，在会话建立过程中确认通信双方的身份，并在后续通信中通过通信凭证证明消息来自对应的端侧管理节点。一个 ASC-Daemon 对应一个端侧管理节点及其 Interposer；具体凭证、会话建立方式、消息格式和传输协议留给实现决定。


### 端侧管理节点与 Branch、Instance 的关系

- 已收敛：端侧管理节点围绕一组 Agent 进程及其执行活动提供本地管理。Agent 主进程及其 sub-agent 进程共同构成一个 Agent 执行实体，由同一个端侧管理范围统一观察和处理。
- 已收敛：Branch 为执行活动提供逻辑归属，端侧管理节点为每个 Branch 承载独立的本地管理上下文，并依据 Branch identity、operation 归属和 capability 路由事件与控制命令。
- 已收敛：多个 Branch 在尚未需要独立 Branch-local 状态时，可以由同一个端侧管理节点在共享执行环境中承载，各 Branch 仍保持独立的管理上下文。Branch 需要独立环境时，其管理上下文随执行活动进入相应环境；具体是复用、迁移还是重新建立实际承载方式，留给实现决定。
- 已收敛：sub-agent 的推理、工具和命令活动，只要处于 integration profile 可观察范围内，就由同一个 Interposer 按所属 Branch 上下文处理，并归属于该 Branch。sub-agent 与主 Agent 的普通协作属于同一 Agent 执行实体；只有显式创建新的 AScout Branch 时，才形成新的 Branch 上下文。
- 已收敛：一个执行环境建立一个端侧管理节点，环境内的每个 Agent 在启动时通过 Integration 接入该节点，并持有独立的 Agent 执行上下文；同一节点可以承载环境内多个 Agent 及其 Branch 上下文。

### Agent product 接入能力

- 已收敛：Agent product 需要提供关键执行活动的观察点，以及在真实 operation 开始前允许 Interposer 暂停或阻止执行的拦截点。
- 已收敛：接入 profile 需要支持 operation 开始、`pending-result boundary`、完成和 outcome 的识别；需要管理的 Agent 主进程及其 sub-agent 执行活动也应处于可观察范围内。
- 已收敛：暂停或阻止真实执行后，Interposer 可以等待控制面结果或执行本地 Decision，再让真实 operation 继续执行，或让当前调用沿 Agent 原有的完成路径使用已经获得语义授权的结果完成。Agent product 提供的是观察和拦截接缝；Branch 上下文、Decision、控制面通信、日志和结果授权属于端侧管理节点与控制面职责。

### Interposer 职责补充

- 已收敛：Interposer 负责处理 Agent 的拦截步骤，从拦截点提取步骤事实，识别 operation、`pending-result boundary`、Branch-local 状态和 Effect，并根据本地 Decision 或控制面处置结果控制步骤继续、暂停、阻止或跳过。
- 已收敛：Interposer 可以暂停一次 Agent 步骤，等待本地 Decision 或控制面结果返回后继续处理。继续时沿产品原有路径执行；跳过时不执行真实 operation，并在需要时使用已经获得语义授权的结果沿原有完成路径结束。
- 已收敛：Interposer 在 Branch-local 状态或受纳管 Effect 发生前提交相应请求，保证真实 operation once-only，从原调用返回路径取得 outcome，将其归属对应 Branch 并通过 ASC-Daemon 提交控制面，同时记录本地 Decision 的输入事实、执行内容、Policy 版本、结果和执行上下文。

### Integration 的步骤接入约定

- 已收敛：Integration 是面向具体 Agent product 的产品适配组件，依据接入约定以最小改造将产品执行接缝连接到端侧管理节点。
- 已收敛：Agent 即将执行可管理步骤时，Integration 将步骤交给端侧管理节点并等待返回处置结果。返回继续时按产品原有路径执行；返回跳过时不执行原步骤，并在需要时让调用沿产品原有完成路径使用端侧管理节点提供的结果结束。暂停通过等待处置结果体现。
- 已收敛：Integration 从被拦截步骤的原调用返回路径自动取得成功、失败或取消结果，并将其交给端侧管理节点形成 outcome；产品不需要提供额外的完成回调。
- 已收敛：不同 integration profile 可以使用不同的产品接缝和步骤表示，但遵循相同的步骤接入约定。Integration 不解释步骤的投机语义，不判断 Policy Decision、Branch 或生命周期，也不决定跳过步骤时使用的结果。

### Agent 接入侧职责与当前范围

- 已收敛：Agent 接入侧不是单纯的请求适配层，而是对应 Agent 的**端侧管理节点**。它负责管理 Agent 的本地执行、观察 Agent 行为，并承担授权范围内本地 Decision 的独立判断和执行责任。
- 已收敛：Agent 接入侧可以执行由 Policy Engine 注入的、不依赖或不改变全局状态的本地 Decision，并向控制面报告 Decision 内容、结果、输入事实、Policy 版本和执行上下文。涉及全局控制状态或生命周期权威的决策必须交还控制面。
- 已收敛：当前不承诺“完全无修改、无需了解 Agent 内部实现”的通用接入，只支持具备足够拦截点或 Hook 的特定 Agent product。接入适配可以依赖产品内部执行结构，以观察和控制所需的推理、工具及 operation 生命周期；后续再评估更通用的接入方式。
- 已确认但未展开：是否允许 Agent 发起显式协作请求，以及 Agent-facing 请求的上下文绑定与转发边界，继续作为后续议题讨论。

### Agent 接入侧文档结构

- 已收敛：删除 `agent-integration.md` 的“固定约束与未定实现”整节。固定约束已在前文和相关设计文档中说明，具体实现问题由开发或使用文档承载，不在本文重复总结。

### 通用控制面框架

- 长期目标是让上层 Agent 使用不同 speculative 算法或分支执行方案，而不要求修改其算法或业务逻辑；但接入实现可以依赖具体 Agent product 的内部结构和拦截点。
- 该目标仍不扩展为任意分布式计算平台，后续必须继续遵守控制面、Runtime Adapter 与 Runtime 的 ownership 分离。

### Adapter 可观察的原子转换语义

- 需要明确控制面要求 Adapter 对 Execution、Workspace、Input、Effect、Outcome 转换呈现怎样的原子性、可见性、失败收敛和 cleanup result。
- Adapter 与目标 Runtime 内部如何协调这些转换不由 AScout 设计；实现建议留在开发/接入文档。

### Control session 与 Workspace identity

- 需要定义 typed control session 的 request identity、幂等、event ordering、取消/超时与断线语义。
- 需要定义 Workspace revision/object identity 及并发 ancestor/descendant write 的线性化规则。

### Runtime 与状态细节

- Continuation branching backend：比较逻辑状态重建、完整进程树 checkpoint/restore、microVM 和定制内核机制。
- Effect adapter 的实际建立：定义 deferred、virtualized、deduplicated、compensatable、irreversible effect 的记录、验证、建立与清理。
- 身份与控制协议：定义 logical Branch identity、PID 生命周期、generation/lease、pidfd、错误、取消、超时和 orphan cleanup。
- 输入边界：定义流/消息边界、partial read、EOF、阻塞行为和真实 FD 所有权。
- 持久化与故障恢复：定义 control/runtime 状态持久化、WAL、ASC-Daemon 恢复、连接排空和隔离环境内后台进程归属。

## 已下沉的讨论

下列主题已经在权威文档或开发决策中记录，不再在本文件保留逐条历史过程：

- **外部效果与资源影响**：额度、限流、并发槽等资源使用默认不纳入 Branch 世界状态，只归属和计量；Policy 可以将其中指定的资源或额度声明为准入约束、预算约束或取消条件。只有它改变保证范围内的可观察业务行为时，才进入 Effect 管理。正式语义见 `docs/design/effect-management.md`，决策记录见 `docs/design/decisions.md` D-029。
- **Effect Management 的范围**：已确认 Effect Management 管理投机分支对分支外部、共享或最终执行状态产生的影响，以及这些影响在投机期间的暂存、限制、提交、取消和清理。Branch-local state 也属于管理范围，但主要由 Runtime 自动完成隔离、清理和 winner 延续；描述中心放在共享、可观察的业务变化。Outcome 和 operational footprint 的管理方式与共享业务变化相似，但是否纳入额外的 Effect 管理由 Policy 决定。单个动作或工具调用只是投机执行过程中的一个环节，不称为一次投机执行。正式语义见 `docs/design/effect-management.md`。
- **Effect Management 文档组织**：已确认按“目的与描述中心 -> 管理范围与保证边界 -> 效果管理过程 -> 效果处理纪律与状态域 -> 责任边界 -> 固定保证与明确边界”的逻辑组织。管理范围与保证边界合并为第 1 节，并保留 `1.1` 到 `1.5`；不设置独立的 Branch 生命周期章节，Branch 的 lifecycle 由 Branch 语义文档负责。`EffectClass` 与 `StateDomain` 在同一节中以完整连续内容说明，不拆成多个概念小节。正式文本见 `docs/design/effect-management.md`。
- **伪装执行命名**：已确认“伪装执行”为正式中文名，对应 `effect` 决策中的 `simulate` 模式标识符，二者并存并建立术语映射。伪装执行是对已接入专门 Adapter 的服务拦截请求、不转发给真实服务、在 Branch 内生成明确标记并记录来源的模拟响应，以避免不可逆外部影响的处理方式；模拟响应作为未验证假设参与 AssumptionSet 确认/否定，仍需 Authority、validator 或其他明确规则验证后才能被 winner 依赖。正式语义见 `docs/design/effect-management.md`，决策模式见 `docs/design/policy-decisions.md` §2.5。
- **Agent 接入表述**：Agent 可以在不修改自身执行逻辑的情况下接入 AScout，但具体产品可能需要 Product Integration、Hook 或其他拦截适配，接入实现应尽可能保持修改面小。该表述不承诺完全不修改产品，也不要求 Agent 主动参与 Branch 生命周期。
- **Mechanism 与四侧架构的关系**：Mechanism 不是第五侧，而是横跨 Agent 接入侧与控制面的抽象实现责任。Agent 接入侧负责拦截和本地落实，控制面负责全局编排、校验和状态协调。
- **Effect Coordinator 的责任边界**：不引入独立的 `Effect backend`。Effect Coordinator 是外部效果的统一实际执行者，负责受管效果的实际处理及其执行状态；服务级 `simulate` 通过可注册的效果插件扩展。Runtime 负责隔离执行环境及环境级执行状态，不承担外部效果的实际执行责任。
- **抽象设计札记边界**：`docs/development/spec-exec-abstraction.md` 只保留论文事实和论文调研观察，不定义 AScout 设计；所有当前设计以正式设计文档为准。
- **过期 Agent 生命周期决策**：原 `docs/design/decisions.md` D-001 已过期并删除。当前默认 Agent 不感知 Branch，显式 Agent 感知只保留为未来设计位置。

- 项目定位、名称、四侧架构和控制面/Runtime ownership：见 `docs/mental-model.md`、`docs/architecture.md`、`docs/design-model.md`。
- Policy Decisions、Proposal、Authority、Equivalence 和 Disposition：见 `docs/design/policy-decisions.md`。
- 状态、效果、obligation 和独立执行环境的建立：见 `docs/design/effect-management.md`。
- Speculation Contract：见 `docs/design/speculation-contract.md`。
- Branch、Accept、Discard、Commit、single-winner、输入传播和生命周期：见 `docs/design/branch-semantics.md`、`docs/design/decisions.md` 与 `docs/design/speculation-decisions.md`。
- Branch barrier、Resolution barrier、外部效果 observation surface 和 Agent execution activity：见 `docs/design/branch-semantics.md` 及对应设计决策。
- 首要验证 workload、默认 Runtime 和新 Rust 实现边界：见 `docs/development/reference-workload.md` 与 `docs/development/decisions.md`。
