# 待讨论事项

本文件是设计讨论队列，不代表其中内容已经成为设计决策。讨论完成后的目标语义应写入 `docs/design/`，实现细节应写入 `docs/development/`。已经下沉到权威文档的历史讨论不在此重复保留。

## 当前讨论

### Speculation Contract 的定位与实现

已收敛：`Contract` 直接约束 Policy 的表达，以及 Runtime Adapter 在控制面接入边界上声明和返回的可观察行为，目的是保证策略与后端接入语义正确且对齐，而不是约束控制面或定义 Runtime 内部实现。控制面是 AScout 自身实现的 Mechanism 主体，是 Contract 的实现方，不属于被约束对象。Contract 不是部署配置或独立 Manager，不能由单个任务或 Policy 修改。

已收敛：`Mechanism` 只作为抽象责任词，是 Contract 的固定实现，不是插件或独立组件；它由 AScout 自身能实现、能强制的固定机制共同构成，由控制面组件协同完成检查与编排，不归属任何单一组件。

已收敛：Runtime 是通用执行后端（例如 docker、虚拟机或进程管理能力），不要求提供 AScout 专用接口。Branch 隔离、效果、cleanup 等 AScout 语义由 Runtime Adapter 用后端原生能力组合表达；控制面只能通过 Adapter 访问后端，因此接入校验的对象是 Adapter，Adapter 通过即代表其背后的 Runtime 能承载相应语义。Adapter 不能静默放宽语义，后端确实表达不出某语义时应拒绝接入或明确降级。

已收敛：Contract 的执行路径统一描述为 `Policy 决策 → Policy Engine 校验决策取值 → 控制面机制编排为受管操作 → Runtime Adapter 映射 → Runtime 执行并返回结果/事件 → 控制面更新状态`。Policy Decision 描述高层语义，取值合法性随后端能力变化，需结合 Adapter 报告能力校验；高层决策展开为受管操作序列，Adapter 按控制面接入契约映射为后端行为。通过 Adapter 的能力声明、接入时基本语义测试与运行期校验来判断其是否兑现后端能力；不直接校验或约束 Runtime 内部实现。

已收敛：失败语义。对尚未被接受的投机 Branch，失败收敛到"该 Branch 在 Speculative Tree 中从未被创建"，不重放该操作；被接受的 Branch 已完成使命，不适用。Contract 文档写共同承诺一句，撤销与清理机制归 control-plane。

已收敛：文档拆分。`docs/design/speculation-contract.md` 收窄为"定位 / 约束什么（单节）/ 如何约束 / 完整控制闭环"；控制面内部机制（组件分工、状态 owner、操作编排、失败清理）下沉到 `docs/design/control-plane.md`。同步更新 glossary、design-model、mental-model、index、effect-management、decisions 与 architecture 的相关引用。

已收敛：`spec-exec-abstraction.md` 不再保留独立的“控制面组件职责”章节。Entry API 的目标职责补入 `control-plane.md`；Policy Engine、Assumption Manager、Scheduler、Effect Coordinator 与 Ledger 的现行职责以该文档为准。论文相关表格和数据继续作为事实依据，分别留在札记的 Effect、资源与准入、Shadow/Replay 等研究主题中；旧六插件、`site`、旧状态模型和重复贯穿示例不迁移。

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
- 已确认：**投机注册动作 = 在 pending-result boundary 上创建携带 Proposal 的 candidate Branch**（即 branch-semantics 的 branch 调用），不引入 site / "Edge 注册"作为额外注册实体；Assumption Manager 的 `(site, proposal)` 状态改为 `(edge_id, proposal_id)`，site 并入 edge 术语。
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
- 已确认：`effect` 对每个 step 的已识别效果返回 `EffectPlan`，模式包括 `default`、`simulate`、`transform`、`stage`、`mediate`、`block`、`allow` 和 `deny`。`simulate` 是伪装执行：服务 Adapter 拦截请求，不访问真实服务，在 Branch 内返回标记为 synthetic 的模拟响应；该响应不是真实 Authority，必须带来源和未验证假设，并由后续 Authority、validator 或明确规则验证。`default` 使用 Adapter/Contract 声明的默认方案，未声明时不得静默放行。`StateDomain` 与 `EffectClass` 保持正交，处理模式需经 Effect Management、Contract 和 Runtime capability 校验。
- 已确认：组合与嵌套当前只定义用户输入这一类跨 Edge 信息传播。用户中断也是用户输入的一种，和普通用户输入一样传播给当时仍在运行的全部 descendants；普通输入作为输入事实进入 Branch，中断触发各 Branch 独立停止/取消和清理。中断不通过 `AssumptionSet` 传播，也不引入集中式子树回收机制。
- 已确认：组合与嵌套的完整边界已写入 `policy-decisions.md` §4：同一 Edge 上的 Proposal/Branch 只是并行候选，由 `predict` 的 breadth、隔离和 single-winner 约束决定，不另设组合语义；嵌套 Edge 继续使用当前 task 已绑定的同一 Policy 及其 task-level 配置。`AssumptionSet` 只表达未验证依赖，跨 Edge 还通过树结构归属、用户输入传播和生命周期控制连接。祖先假设被否定时，控制面向受影响 Branch 直接发送取消命令，各 Branch 独立停止和清理，不设置集中式整棵子树回收机制。
- 已确认：AScout 暂存通过 `admit` 但尚未启动的 Edge；粗判识别可能的投机入口，精判只决定是否登记和保留，不决定立即生效。Policy 的独立 `select` 在同一 task / AScoutAgent 内的全部有效待选机会间直接选择当前启动的有序子集，可选一个、多个或暂时不选；未选机会继续等待，直到后来被选中或客观前提失效，不要求额外的主动放弃接口。Scheduler 拥有待选集合、维护有效性并在资源、Contract 与正确性硬约束内执行选择，启动前不重复 Policy 精判，也不使用固定价值公式替代 Policy。独立的 `rebalance` 面向已运行或暂停的投机 Branch，决定暂停、恢复或抢占；`preempt` 以 `resolution = preempted` 进入 `dispose`。best-first、跨 Edge 排序公式、浅层分叉/深层单链与 breadth/depth 分配均为可选 Policy。
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

## 暂停讨论

以下事项已经记录，但要等当前分支执行模型或源码基础进一步稳定后再展开：

### 控制面执行模型与 Policy

- 控制面拥有 AScoutAgent、Speculative Tree 控制状态、Policy、请求校验、操作编排与 Runtime event 解释。
- 控制面内部包括 Entry API、Policy Engine、Effect Coordinator、Assumption Manager、Scheduler 与 Ledger，不设置额外的总控 Core。
- Policy 只能在 AScout 固定生命周期、single-winner、capability、obligation、隔离和 cleanup 语义内作选择，不能拥有或修改 Runtime mutable state。

### Agent-facing control surface

- Routine Agent surface 暂时保留 `context`、`children`、创建、`commit`、`accept`、`discard` 和 event wait 这些 operation placeholders。
- 当前不定义参数、返回值、阻塞方式、event model 或具体命令拼写。
- 不提供独立 `publish-outcome`；它仍是由 Operation Integration 自动触发的 Runtime lifecycle semantic operation。
- 不提供日常 `delegate-resolver`；需要控制面代为裁决时，delegation 作为 managed launch 或 Branch 创建配置表达。

### 通用控制面框架

- 目标是让上层 Agent 使用不同 speculative 算法或分支执行方案，而不要求修改算法、方案或 Agent。
- 该目标仍不扩展为任意分布式计算平台，后续必须继续遵守控制面、Runtime Adapter 与 Runtime 的 ownership 分离。

### Adapter 可观察的原子转换语义

- 需要明确控制面要求 Adapter 对 Execution、Workspace、Input、Effect、Outcome 转换呈现怎样的原子性、可见性、失败收敛和 cleanup result。
- Adapter 与目标 Runtime 内部如何协调这些转换不由 AScout 设计；实现建议留在开发/接入文档。

### Control session 与 Workspace identity

- 需要定义 typed control session 的 request identity、幂等、event ordering、取消/超时与断线语义。
- 需要定义 Workspace revision/object identity 及并发 ancestor/descendant write 的线性化规则。

### Runtime 与状态细节

- Continuation branching backend：比较逻辑状态重建、完整进程树 checkpoint/restore、microVM 和定制内核机制。
- Effect adapter materialization：定义 deferred、virtualized、deduplicated、compensatable、irreversible effect 的记录、验证、materialize 与清理。
- 身份与控制协议：定义 logical Branch identity、PID 生命周期、generation/lease、pidfd、错误、取消、超时和 orphan cleanup。
- 输入边界：定义流/消息边界、partial read、EOF、阻塞行为和真实 FD 所有权。
- 持久化与故障恢复：定义 control/runtime 状态持久化、WAL、ASC-Daemon 恢复、连接排空和隔离环境内后台进程归属。

## 已下沉的讨论

下列主题已经在权威文档或开发决策中记录，不再在本文件保留逐条历史过程：

- **外部效果与资源影响**：额度、限流、并发槽等资源使用默认不纳入 Branch 世界状态，只归属和计量；Policy 可以将其中指定的资源或额度声明为准入约束、预算约束或取消条件。只有它改变保证范围内的可观察业务行为时，才进入 Effect 管理。正式语义见 `docs/design/effect-management.md`，决策记录见 `docs/design/decisions.md` D-029。
- **Effect Management 的范围**：已确认 Effect Management 管理投机分支对分支外部、共享或最终执行状态产生的影响，以及这些影响在投机期间的暂存、限制、提交、取消和清理。Branch-local state 也属于管理范围，但主要由 Runtime 自动完成隔离、清理和 winner 延续；描述中心放在共享、可观察的业务变化。Outcome 和 operational footprint 的管理方式与共享业务变化相似，但是否纳入额外的 Effect 管理由 Policy 决定。单个动作或工具调用只是投机执行过程中的一个环节，不称为一次投机执行。正式语义见 `docs/design/effect-management.md`。
- **Effect Management 文档组织**：已确认按“目的与描述中心 -> 管理范围与保证边界 -> 效果管理过程 -> 效果处理纪律与状态域 -> 责任边界 -> 固定保证与明确边界”的逻辑组织。管理范围与保证边界合并为第 1 节，并保留 `1.1` 到 `1.5`；不设置独立的 Branch 生命周期章节，Branch 的 lifecycle 由 Branch 语义文档负责。`EffectClass` 与 `StateDomain` 在同一节中以完整连续内容说明，不拆成多个概念小节。正式文本见 `docs/design/effect-management.md`。
- **伪装执行命名**：已确认“伪装执行”为正式中文名，对应 `effect` 决策中的 `simulate` 模式标识符，二者并存并建立术语映射。伪装执行是对已接入专门 Adapter 的服务拦截请求、不转发给真实服务、在 Branch 内生成 synthetic 模拟响应以避免不可逆外部影响的处理方式；模拟响应必须带 synthetic provenance、作为未验证假设参与 AssumptionSet 确认/否定，仍需 Authority、validator 或其他规则验证后才能被 winner 依赖。正式语义见 `docs/design/effect-management.md`，决策模式见 `docs/design/policy-decisions.md` §2.5。

- 项目定位、名称、四侧架构和控制面/Runtime ownership：见 `docs/mental-model.md`、`docs/architecture.md`、`docs/design-model.md`。
- Policy Decisions、Proposal、Authority、Equivalence 和 Disposition：见 `docs/design/policy-decisions.md`。
- 状态、效果、obligation 和 materialization：见 `docs/design/effect-management.md`。
- Speculation Contract：见 `docs/design/speculation-contract.md`。
- Branch、Accept、Discard、Commit、single-winner、输入传播和生命周期：见 `docs/design/branch-semantics.md`、`docs/design/decisions.md` 与 `docs/design/speculation-decisions.md`。
- Branch barrier、Resolution barrier、外部效果 observation surface 和 Agent execution activity：见 `docs/design/branch-semantics.md` 及对应设计决策。
- 首要验证 workload、默认 Runtime 和新 Rust 实现边界：见 `docs/development/reference-workload.md`、`docs/development/decisions.md` 与 `docs/development/roadmap.md`。
