# 设计模型

本文指导 AScout 的**抽象概念层**：它描述策略如何被表达、机制如何落实、运行能力如何支撑，不描述具体系统如何部署与连线。本文与架构是 mental-model 的两个平级展开，分别对应心智模型中的工作模型与支撑模型（§2、§3）。

本文只记录已确定的抽象语义。仓库当前没有实现；开发决策、路线图与验收状态记录在开发文档中。

## 1. 抽象分层：Policy / Mechanism / Runtime

Policy、Mechanism 与 Runtime 构成抽象责任模型，描述从策略意图到机制落实、再到具体运行能力的关系。

```
Policy（抽象接口：作者只需面对决策点）
   ↓
Mechanism（实现层：拦截、执行、隔离、资源、效果、调度）
   ↓
Runtime（具体后端原语）
```

图中的箭头表示抽象关系从策略意图向运行能力落实，不表示系统调用方向或能力提供方向。

**Policy：作者只需面对抽象。** Policy 作者面对的是一组抽象的决策点——在哪些环节投机（推理、工具执行等）、预测什么、如何判定命中、用什么公式判断是否值得投机、如何展开与处置投机工作。作者只需要给出这些决策的实现或选择，不需要知道系统是怎么落实的：命令如何被拦截、隔离环境如何创建、资源如何管理、失败分支如何清理，都由 Mechanism 负责。分层把"作者可以灵活定制"的部分与"必须可靠执行"的部分分开：作者在算法层面自由选择，正确性、隔离与资源约束由 Mechanism 兜底。

**Mechanism：把抽象落实为执行。** Mechanism 是抽象的实现责任：在 Agent 接入侧通过 Interposer 拦截命令与推理调用等环节，在控制面组织执行、验证与调度，并通过 Runtime Adapter 使用外部后端的隔离、资源与效果能力。它不一定只是组合 Runtime 原语——拦截、效果管理、调度等能力本身就是 Mechanism 的一部分；Adapter 无法证明后端具备所需能力时，降级或拒绝，不让策略悄悄跑偏。

**Runtime：外部能力来源。** Runtime 是具体执行后端，例如容器、虚拟机或进程管理能力。AScout 不定义其内部结构，也不要求不同 Runtime 原生提供同一组 AScout 原语。Runtime Adapter 负责判断并说明目标 Runtime 的原生能力能否组合出控制面所需语义。

**Speculation Contract：策略表达与后端接入的语义对齐约束。** Contract 约束 Policy 的表达以及 Runtime Adapter 在控制面边界上的可观察行为：Policy 如何表达与校验决策取值、Adapter 需要声明并兑现哪些基本运行语义。控制面是这份约束的实现方（Mechanism）：它把 Policy 决策编排为受管操作、经 Adapter 映射到后端，并在运行期以结果和事件确认约束是否被兑现。Contract 不直接规定 Runtime 内部实现。

## 2. 工作模型

### 2.1 投机执行过程

投机执行的最小条件，是后续工作暂时依赖一个尚未确认的 Proposal，并且之后存在能够验证或接受这个 Proposal 的依据。这个依据可以来自 Authority 提供的真实结果，也可以来自独立 validator 给出的可接受性判断。

等待真实结果是其中最常见的一类。此时，当 Agent 正在进行某一特定行为时，这一行为会被捕获并触发投机执行系统。系统首先根据 Agent 当前状态、系统资源条件和投机执行深度等因素，判断当前是否适合进行投机执行。

当判定适合投机时，系统预测该行为的 N 个可能结果，并将这些 Proposal 暂时视作后续行为的输入。据此创建 N 个投机执行过程并运行——这些过程彼此隔离，互不影响，也不影响 Agent 已有行为的执行。

Agent 默认无需感知背后的投机分支，仍可按原方式完成任务。对于需要显式协作的 Agent，AScout 可以提供受控的感知、查询、提示和协作入口，使 Agent 参与候选创建或裁决等工作；这些入口最终仍受控制面与 Runtime 的身份、能力、single-winner、隔离和清理约束，不能成为生命周期正确性的绕行路径。

当真实结果到达时，系统将它与各 Proposal 对照，从中选择至多一个兼容的过程。不兼容的投机执行过程被终止并回收；兼容的过程则复用其结果。如果不存在需要并行运行的权威行为，系统也可以依据独立 validator 判断 Proposal 是否达到可接受标准。至此，一次投机执行结束。

需要注意：上述"等待真实结果"的流程是典型流程，不是投机执行的普遍前提；当 Authority 为空、阶段被替换或省略，或者嵌套投机等复杂行为共同作用时，系统流程并不限于这一线性描述。

投机执行由一组可组合的阶段构成，而不是固定的线性流程。Policy 可以根据任务、执行对象、可用信息和 Runtime 能力选择其中的阶段，并决定各阶段的具体实现与组合方式。某些阶段可以缺失，但被启用的阶段仍需遵守必要的数据依赖和 AScout 固定的正确性、隔离、资源与副作用约束。

一个投机执行过程可以总结为以下阶段：机会发现 -> 待选准入 -> 启动选择 -> 预测生成 -> 过程执行 -> 验证 -> 处置与提交

1. **机会发现**：在 Agent 接入侧识别并规范化 operation 及其 `pending-result boundary`，并把该边界上潜在的投机机会表示为一条投机 Edge。具体的命令拦截、Hook、CLI、embedded adapter 或进程观察方式属于接入实现，不是投机执行的固定前提。
2. **待选准入**：由 Policy 判断当前机会是否值得登记并保留；通过后由 AScout 暂存，尚不表示立即启动。
3. **启动选择**：一个 task / AScoutAgent 绑定的 Policy 在该 task 内所有仍然有效的待选机会之间进行全局权衡，直接选择现在启动哪些；控制面只在固定正确性、Contract 和资源硬约束内落实。
4. **预测生成**：产生一个或多个 Proposal，为后续投机工作提供暂时依据。Proposal 可以来自模型、规则、历史轨迹、外部系统或其他来源。
5. **过程执行**：基于 Proposal 提前执行投机工作。投机工作可以是模型推理、工具调用、命令执行、远程请求或其他计算；Policy 可以在已启动工作之间决定暂停、恢复或抢占。
6. **验证**：获取用于验证 Proposal 的权威依据，并按 Policy 规定的条件判断 Proposal 是否兼容或达到可接受标准。权威依据可以来自 Authority 提供的真实执行、用户输入、环境反馈或远程服务；Authority 可以为空，此时由独立的 validator 或其他可接受性判断完成验证。
7. **处置与提交**：根据验证结果复用、继续、取消、重新执行、清理、残值回收、补偿或以其他方式处理投机工作，并将符合条件的结果、状态变化和外部效果纳入后续执行；没有需要提交的结果或副作用时，可以不设置独立的提交阶段。

这些阶段在职责和实现上相互独立，每个阶段中都有多个决策点(Policy decision)，Policy 可以选择、替换和组合它们，但并不意味着阶段之间完全分离。例如，基于 Proposal 的投机工作需要先获得某种暂时依据；没有权威依据时，验证必须改用 validator 或其他可接受性判断；结果和副作用在满足相应条件前不能进入最终执行状态。Policy 决定执行策略，Mechanism 保证策略在既定约束内执行。这些决策点的组合视图（决策面）与其作用对象投机边，见 §3.1。

## 3. 支撑模型

### 3.1 投机边与 Policy Decisions

我们对一次投机执行进行进一步抽象。令一个会产生结果的行为 `u`，`u` 的真实结果为 `i`，依赖该结果才能继续的后续行为 `v`。将 `i` 视作 `u` 和 `v` 之间的连线，我们就可以得到 `u --i--> v`。假设我们对这一过程进行投机执行，也就是使用虚拟的 `i^n`（或者称作 Proposal） 替代 `i` 来提前启动 `v^n` ，并最终将 `v` 和 `v^n` '合并' 成一个结果。我们将这一过程建模成 "投机边 Edge"。

Edge 是一个全局概念，从一个投机机会开始，到这个机会失效或对应的投机执行过程结束为止。Edge 表示在哪个未决依赖上可能进行投机；通过 `admit` 后，它可以先处于待选状态而尚无 Proposal 或 Branch。被 `select` 选中并开始预测后，同一个 Edge 上可以并行存在多个 Proposal `i^n`，每个 Proposal 各自派生出候选 Branch。Proposal 表示对该依赖结果的一个预测，Branch 表示基于该预测实际展开的候选执行。

Edge 还是 Policy 作用的基本对象。Policy Decision 是 Policy 可以根据任务、上下文、资源和 Runtime 能力作出的可替换决策，通过对不同 Policy Decision 的组合可以生成不同的执行策略。单条 Edge 上的局部决策、跨 Edge 的全局选择和已启动工作的调整由九类 policy decision 刻画：`admit`、`select`、`rebalance`、`extent`、`predict`、`launch`、`effect`、`match`、`dispose`。对每个 decision 的详细描述见 [Policy Decisions 设计](design/policy-decisions.md)。

组合与嵌套：同一 Edge 可以生成多个并行候选；候选 Branch 在继续执行时也可能遇到新的 Edge，由此形成嵌套。嵌套工作依赖自身以及祖先路径上尚未确认的 Proposal，任一前提被否定或发生变化都会使依赖它的后续工作失效；当前阶段不修复或重新对齐这类 Branch。父 Proposal 被确认时，原父 Branch 的执行延续成为权威路径，仍有未决依赖的后代 Branch 保留原有隔离环境和执行进展并重新挂到该权威执行延续之下；后代 Branch 只有在自身依赖清空后才具备提交资格。AScout 以树形、single-winner 和受控信息流约束这些关系；当前 task 内的嵌套 Edge 使用同一个 Policy。Proposal 依赖、确认与否定、Branch 取消、父 Branch salvage 以及用户输入传播的完整语义见 [Policy Decisions 设计](design/policy-decisions.md) §4。

### 3.2 状态与效果

不同隔离方案往往隔离粒度不同，且较为全面的方案未对投机执行进行专门的适配，投机执行算法提出的方案都较为简单。并且都没有提供统一的接入方式，适配工作量大，这正是隔离碎片化的问题。状态与效果模型把投机执行产生的文件、进程、输入、结果和外部服务交互放入统一的管理框架，提供统一的接入方式，并根据状态域和效果纪律选择相应的隔离、暂存、延迟释放、补偿或清理方式。

投机产物在验证、提交和必要的状态转换完成前不能进入最终语义状态。效果处理能力不足时，Effect Management 可以让投机执行阻塞在相关效果边界并保留前置进展；只有无法安全阻塞、接受后继续或淘汰后清理时，才拒绝相关环节的投机或明确降级。具体的 `EffectClass × StateDomain` 分类、交叉规则和后端能力要求见 [效果管理设计](design/effect-management.md)。

控制面中的 Effect Coordinator 负责效果分类、计划与协调；相应的 Effect backend 承载 staged effect、obligation、效果提交生效和清理等实际效果状态，Runtime 负责隔离执行环境及其环境级状态，Agent 接入侧负责 Agent 执行实体和本地投机上下文。Runtime Adapter 负责向控制面呈现各自负责的操作是否完成。前者形成控制计划，后端负责实际执行；AScout 不通过修改控制状态代替后端执行。

### 3.3 Speculation Contract

Speculation Contract 约束 Policy 的表达以及 Runtime Adapter 在控制面边界上的可观察行为，把策略表达、后端接入与实际运行之间必须保持的语义对齐固定为可检查、可执行的契约。它使 Policy 能在统一决策面内表达需求，也使不同 Adapter 以统一操作和事件接入，而不让策略语义隐藏在特定后端或算法实现中。控制面是这份契约的实现方，属于 Mechanism；它把 Policy 决策编排为受管操作、经 Runtime Adapter 落向后端并解释事件。

Policy Engine 在加载或任务准入时检查 Policy 的决策取值能否由当前接入的后端能力承载；无法满足时必须拒绝，或明确报告降级。运行期间，控制面通过 Authority、validator、效果处理和 Runtime 返回的实际结果检查声明是否被兑现。

Contract 不拥有 Branch 或 Runtime 的运行状态，也不直接约束 Runtime 内部实现。总体关系见 [Speculation Contract 设计](design/speculation-contract.md)，Runtime Adapter 的具体契约、校验路径、降级结果和运行期证据见 [Runtime Adapter 契约设计](design/runtime-adapter-contract.md)，控制面如何落实见 [控制面设计](design/control-plane.md)。

## 4. 跨领域设计约束

以下约束适用于 Policy、Mechanism、Runtime 及其扩展的设计取舍：

1. **策略选择不应被机制固化**：能够交给 Policy 选择的内容，不应由核心机制或 Runtime 固定。
2. **正确性与资源所有权不能依赖策略自律**：必须由 AScout Mechanism 或经 Adapter 验证的后端机制落实的约束，不能只依赖 Policy 遵守。
3. **跨层语义必须显式表达**：Policy 的要求、Runtime 的能力和运行结果不能依赖未声明的隐含假设。
4. **能力不足不得静默改变语义**：只能拒绝、明确降级或选择其他合法路径。
5. **扩展不能破坏既有固定语义**：新 Policy、Runtime、状态域或其他扩展不能绕过 Branch、single-winner、隔离、obligation 和 cleanup 等固定约束。

## 相关文档

- [架构](architecture.md)：系统结构（四侧、所有权、调用方向）
- [Policy Decisions 设计](design/policy-decisions.md)：Policy 决策面
- [效果管理设计](design/effect-management.md)：状态与效果
- [Speculation Contract 设计](design/speculation-contract.md)：策略与运行时的语义对齐契约
- [控制面设计](design/control-plane.md)：控制面作为 Mechanism 的实现
- [Branch 语义设计](design/branch-semantics.md)：Branch 生命周期、状态可见性和 single-winner 语义
