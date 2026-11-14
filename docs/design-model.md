# 设计模型

本文指导 AScout 的**抽象概念层**：它描述策略如何被表达、机制如何落实、运行能力如何支撑，不描述具体系统如何部署与连线。本文与架构是 mental-model 的两个平级展开，分别对应心智模型中的工作模型与支撑模型（§2、§3）。

本文只记录已确定的抽象语义。仓库包含 Pi/NLE 原型实现；实现覆盖范围、限制与验收状态记录在开发文档中，不能把原型等同于完整通用实现。

## 1. 抽象分层：Policy / Mechanism / Runtime

Policy、Mechanism 与 Runtime 构成抽象责任模型，描述从策略意图到机制落实、再到具体运行能力的关系。

```
Policy（抽象接口：作者只需面对决策点）
   ↓
Mechanism（实现层：拦截、执行、隔离、资源、效果、调度）
   ↓
Runtime（动作承载、状态和效果边界）
```

图中的箭头表示抽象关系从策略意图向运行能力落实，不表示系统调用方向或能力提供方向。

**Policy：作者只需面对抽象。** Policy 作者面对的是一组抽象的决策点——在哪些环节投机（推理、工具执行等）、预测什么、如何判定命中、用什么公式判断是否值得投机、如何展开与处置投机工作。作者只需要给出这些决策的实现或选择，不需要知道系统是怎么落实的：命令如何被拦截、隔离环境如何创建、资源如何管理、失败分支如何清理，都由 Mechanism 负责。分层把"作者可以灵活定制"的部分与"必须可靠执行"的部分分开：作者在算法层面自由选择，正确性、隔离与资源约束由 Mechanism 兜底。

**Mechanism：把抽象落实为执行。** Mechanism 不是额外的一侧，而是横跨 Agent 接入侧、控制面与 Runtime 的实现责任：Agent 接入侧通过 Interposer 拦截命令与推理调用等环节，控制面组织执行、验证与调度，Runtime 承载动作并落实环境、资源和效果边界。它不一定只是组合 Runtime 原语；拦截、编排、校验、调度和实际边界执行都是 Mechanism 的一部分。Runtime 无法提供所需能力时，Mechanism 必须拒绝或明确降级，不让策略悄悄跑偏。

**Runtime：动作的受约束承载系统。** Runtime 承载 Agent 的一步动作，使其能够正常、正确地运行，并把动作对真实世界或 authority 的影响限制在允许范围内。Runtime 可以组合进程、容器、虚拟机、Overlay 和效果协调等能力；这些是可插拔的能力，不是 Runtime 的固定形态。Runtime Adapter 是 Runtime 与控制面之间的接入边界；Registry 属于 Runtime，负责能力声明校验、主动验证和独立证据，控制面使用其能力事实匹配 Policy。Agent Session 的内部复制、共享、恢复和继续运行由 Agent 接入侧负责，Runtime 承载其所需的执行状态和外部效果边界。

Gateway 统一承接 Agent 接入侧、UI 与 Runtime 的通信，负责接入、消息分发和送达；消息对投机执行的变化由相应控制逻辑决定。它不拥有 Tree、Policy 或投机编排，通信统一也不改变 Runtime Adapter 的契约或 Registry 的证明职责。

**Speculation Contract：策略、控制面与 Runtime 的语义对齐约束。** Contract 约束 Policy 的表达以及 Runtime 通过 Adapter 呈现的可观察行为：Policy 如何表达与校验决策取值、Runtime 需要声明并兑现哪些基本运行语义。控制面与 Runtime 共同落实这份约束（Mechanism）：控制面编排受管操作，Runtime 实际承载动作并返回结果和事件。Contract 不直接规定 Runtime 内部实现。

## 2. 工作模型

一个 session 对应一个 AScoutAgent 和一棵 Speculative Tree。task 是 Agent 的一轮工作；Agent 完成本轮工作并开始等待用户输入时，task 完成，多轮 task 在同一控制域中继续。长期设计允许修改 Policy；当前首版同一 session 加载的 Policy 固定，不实现修改或切换，范围见 [Policy 开发说明](development/policy-engine.md)。Runtime 向控制面报告物化完成或失败，物化失败等同操作失败；控制面必须等物化成功后才能宣告任务成功，物化前提见 [Runtime 设计](design/runtime.md)。

候选被接受并承接后续执行时继续使用候选环境，仍受祖先依赖与效果约束。Agent 常用 Ctrl-C 或退出终止任务，结束候选及相关写入后物化；暂停由 AScout 独立接口提供，保留树、候选和环境等待继续。

### 2.1 投机执行过程

投机执行的最小条件，是后续工作暂时依赖一个尚未确认的 Proposal，并且之后存在能够验证或接受这个 Proposal 的依据。这个依据可以来自 Authority 提供的真实结果，也可以来自独立 validator 给出的可接受性判断。

等待真实结果是其中最常见的一类。此时，当 Agent 正在进行某一特定行为时，这一行为会被捕获并触发投机执行系统。系统首先根据 Agent 当前状态、系统资源条件和投机执行深度等因素，判断当前是否适合进行投机执行。

当判定适合投机时，系统预测该行为的 N 个可能结果，并将这些 Proposal 暂时视作后续行为的输入。据此创建 N 个投机执行过程并运行——这些过程彼此隔离，互不影响，也不影响 Agent 已有行为的执行。

Agent 默认无需感知背后的投机分支，仍可按原方式完成任务。对于需要显式协作的 Agent，AScout 可以提供受控的感知、查询、提示和协作入口，使 Agent 参与候选创建或裁决等工作；这些入口最终仍受控制面与 Runtime 的身份、能力、single-winner、隔离和清理约束，不能成为生命周期正确性的绕行路径。

当真实结果到达时，系统将它与各 Proposal 对照，从中选择至多一个兼容的过程。不兼容的投机执行过程被终止并回收；兼容的过程则复用其结果。如果不存在需要并行运行的权威行为，系统也可以依据独立 validator 判断 Proposal 是否达到可接受标准。至此，一次投机执行结束。

需要注意：上述"等待真实结果"的流程是典型流程，不是投机执行的普遍前提；当 Authority 为空、阶段被替换或省略，或者嵌套投机等复杂行为共同作用时，系统流程并不限于这一线性描述。

投机执行由一组可组合的阶段构成，而不是固定的线性流程。Policy 可以根据任务、执行对象、可用信息和 Runtime 能力选择其中的阶段，并决定各阶段的具体实现与组合方式。某些阶段可以缺失，但被启用的阶段仍需遵守必要的数据依赖和 AScout 固定的正确性、隔离、资源与副作用约束。

一个投机执行过程可以总结为以下阶段：机会发现 -> 待选准入 -> 启动选择 -> 预测生成 -> 过程执行 -> 验证 -> 处置与提交

1. **机会发现**：在 Agent 接入侧识别并规范化 operation 及其 `pending-result boundary`，并把该边界上潜在的投机机会按 Edge 模型描述；此阶段没有 id，机会通过 `admit` 后才成为拥有 id 的 Chance。具体的命令拦截、Hook、CLI、embedded adapter 或进程观察方式属于接入实现，不是投机执行的固定前提。
2. **待选准入**：由 Policy 判断当前机会是否值得登记并保留；通过后由 AScout 暂存，尚不表示立即启动。
3. **启动选择**：Policy 在同一 AScoutAgent 内所有仍然有效的待选机会之间进行全局权衡，直接选择现在启动哪些；控制面只在固定正确性、Contract 和资源硬约束内落实。
4. **预测生成**：产生一个或多个 Proposal，为后续投机工作提供暂时依据。Proposal 可以来自模型、规则、历史轨迹、外部系统或其他来源。
5. **过程执行**：基于 Proposal 提前执行投机工作。投机工作可以是模型推理、工具调用、命令执行、远程请求或其他计算；Policy 可以在已启动工作之间决定暂停、恢复或抢占。
6. **验证**：获取用于验证 Proposal 的权威依据，并按 Policy 规定的条件判断 Proposal 是否兼容或达到可接受标准。权威依据可以来自 Authority 提供的真实执行、用户输入、环境反馈或远程服务；Authority 可以为空，此时由独立的 validator 或其他可接受性判断完成验证。
7. **处置与提交**：根据验证结果复用、继续、取消、重新执行、清理、残值回收、补偿或以其他方式处理投机工作，并将符合条件的结果、状态变化和外部效果纳入后续执行；没有需要提交的结果或副作用时，可以不设置独立的提交阶段。

这些阶段在职责和实现上相互独立，每个阶段中都有多个决策点(Policy decision)，Policy 可以选择、替换和组合它们，但并不意味着阶段之间完全分离。例如，基于 Proposal 的投机工作需要先获得某种暂时依据；没有权威依据时，验证必须改用 validator 或其他可接受性判断；结果和副作用在满足相应条件前不能进入最终执行状态。Policy 决定执行策略，Mechanism 保证策略在既定约束内执行。这些决策点的组合视图（决策面）与其作用对象 Chance，见 §3.1。

## 3. 支撑模型

### 3.1 投机边与 Policy Decisions

我们对一次投机执行进行进一步抽象。令一个会产生结果的行为 `u`，`u` 的真实结果为 `i`，依赖该结果才能继续的后续行为 `v`。将 `i` 视作 `u` 和 `v` 之间的连线，我们就可以得到 `u --i--> v`。假设我们对这一过程进行投机执行，也就是使用虚拟的 `i^n`（或者称作 Proposal） 替代 `i` 来提前启动 `v^n` ，并最终将 `v` 和 `v^n` '合并' 成一个结果。我们将这一过程建模成 "投机边 Edge"。

Edge 是描述投机如何进行的抽象模型，给出这一通用过程的形状；它不是运行时对象，没有 id，代码中不需要 Edge 实体类型。Edge 的一个具体实例称为 Chance，即被 `admit` 接纳的投机机会：从 `admit` 通过起拥有 id，直到被验证后接受或抛弃、整个过程收敛为止。某次具体执行由 Branch 承载。Proposal 是 Chance 边界上的不可变候选输入，Branch 以 Candidate 角色承载基于 Proposal 的实际展开；一个 Chance 可以有多个 Proposal 和多个 Branch，每个 Proposal/Branch 只对应一个 Chance。

九类 Policy Decision 的语义定义在 Edge 模型上，运行时作用于具体的 Chance。Policy Decision 是 Policy 可以根据任务、上下文、资源和 Runtime 能力作出的可替换决策，通过对不同 Policy Decision 的组合可以生成不同的执行策略。单个 Chance 上的局部决策、跨 Chance 的全局选择和已启动工作的调整由九类 policy decision 刻画：`admit`、`select`、`rebalance`、`extent`、`predict`、`launch`、`effect`、`match`、`dispose`。对每个 decision 的详细描述见 [Policy Decisions 设计](design/policy-decisions.md)。

组合与嵌套：同一 Chance 可以生成多个并行候选；候选 Branch 在继续执行时也可能遇到新的投机机会（admit 后成为新的 Chance），由此形成嵌套。嵌套工作依赖自身以及祖先路径上尚未确认的 Proposal，任一前提被否定或发生变化都会使依赖它的后续工作失效；当前阶段不修复或重新对齐这类 Branch。父 Proposal 被确认时，原父 Branch 的执行延续成为权威路径，仍有未决依赖的后代 Branch 保留原有隔离环境和执行进展并重新挂到该权威执行延续之下；后代 Branch 只有在自身依赖清空后才具备提交资格。AScout 以树形、single-winner 和受控信息流约束这些关系。Proposal 依赖、确认与否定、Branch 取消、父 Branch salvage 以及用户输入传播的完整语义见 [Policy Decisions 设计](design/policy-decisions.md) §4。

### 3.2 状态与效果

不同隔离方案往往隔离粒度不同，且较为全面的方案未对投机执行进行专门的适配，投机执行算法提出的方案都较为简单。并且都没有提供统一的接入方式，适配工作量大，这正是隔离碎片化的问题。状态与效果模型把投机执行产生的文件、进程、输入、结果和外部服务交互放入统一的管理框架，提供统一的接入方式，并根据状态域和效果纪律选择相应的隔离、暂存、延迟释放、补偿或清理方式。

投机产物在验证、提交和必要的状态转换完成前不能进入最终语义状态。效果处理能力不足时，Effect Management 可以让投机执行阻塞在相关效果边界并保留前置进展；只有无法安全阻塞、接受后继续或淘汰后清理时，才拒绝相关环节的投机或明确降级。具体的 `EffectClass × StateDomain` 分类、交叉规则和后端能力要求见 [效果管理设计](design/effect-management.md)。

Runtime 整体负责动作的执行承载、环境边界以及动作产生的效果处理；内部可以包含效果协调机制和服务效果插件。它需要限制 Agent 动作对真实世界或 authority 的影响，并在接受、丢弃、阻塞、清理或降级时返回可观察结果。Agent 接入侧负责 Agent 执行实体和 Session 的本地管理。具体哪些效果需要什么处理方式，由 Policy、Runtime 能力和运行时条件共同决定。具体实现阶段见 [Runtime 开发说明](development/runtime-adapter.md)。

### 3.3 Speculation Contract

Speculation Contract 约束 Policy 的表达以及 Runtime 通过接入边界呈现的可观察行为，把策略表达、Runtime 能力与实际运行之间必须保持的语义对齐固定为可检查、可执行的契约。它使 Policy 能在统一决策面内表达需求，也使不同 Runtime 以统一操作和事件接入，而不让策略语义隐藏在特定实现中。控制面与 Runtime 共同落实这份契约：控制面编排受管操作，Runtime 实际执行并返回结果和事件。

Policy Engine 在加载或任务准入时检查 Policy 的决策取值能否由当前接入的后端能力承载；无法满足时必须拒绝，或明确报告降级。运行期间，控制面通过 Authority、validator、效果处理和 Runtime 返回的实际结果检查声明是否被兑现。

Contract 不拥有 Branch 或 Runtime 的运行状态，也不直接规定 Runtime 内部实现。总体关系见 [Speculation Contract 设计](design/speculation-contract.md)，Runtime Adapter 的具体接入契约、校验路径、降级结果和运行期证据见 [Runtime Adapter 契约设计](design/runtime-adapter-contract.md)，控制面如何落实见 [控制面设计](design/control-plane.md)。

## 4. 跨领域设计约束

以下约束适用于 Policy、Mechanism、Runtime 及其扩展的设计取舍：

1. **策略选择不应被机制固化**：能够交给 Policy 选择的内容，不应由核心机制或 Runtime 固定。
2. **正确性与资源所有权不能依赖策略自律**：必须由 AScout Mechanism 或经 Adapter 验证的后端机制落实的约束，不能只依赖 Policy 遵守。
3. **跨层语义必须显式表达**：Policy 的要求、Runtime 的能力和运行结果不能依赖未声明的隐含假设。
4. **能力不足不得静默改变语义**：只能拒绝、明确降级或选择其他合法路径。
5. **扩展不能破坏既有固定语义**：新 Policy、Runtime、状态域或其他扩展不能绕过 Branch、single-winner、隔离、obligation 和 cleanup 等固定约束。

## 相关文档

- [架构](architecture.md)：系统结构（责任边界、所有权、调用方向）
- [Policy Decisions 设计](design/policy-decisions.md)：Policy 决策面
- [效果管理设计](design/effect-management.md)：状态与效果
- [Speculation Contract 设计](design/speculation-contract.md)：策略与运行时的语义对齐契约
- [控制面设计](design/control-plane.md)：控制面作为 Mechanism 的实现
- [Branch 语义设计](design/branch-semantics.md)：Branch 生命周期、状态可见性和 single-winner 语义
