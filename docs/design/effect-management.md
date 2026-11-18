# 效果管理设计

本文承载 AScout 对 Branch 运行中状态变化和外部影响的管理语义。Effect Management 管理投机执行过程中需要保护的状态变化；其中以共享、可观察的业务变化为描述中心，Branch-local state、Outcome 和 operational footprint 按各自的规则处理。

## 当前 Runtime 边界

Runtime 负责动作承载、环境边界和动作产生的效果处理；control plane 负责 Speculative Tree 的全局状态、Policy 和最终接受或丢弃决定。Instrumentation 负责 Agent Session 的内部管理。Overlay、容器和虚拟机是 Runtime 可组合的能力实现。具体实现阶段见 [Runtime 开发说明](../development/runtime-adapter.md)。

## 1. 管理范围与保证边界

投机分支对分支外部、共享或最终执行状态产生的影响，以及这些影响在投机期间的暂存、限制、提交、取消和清理，均属于 Effect Management 的管理范围。单个动作、命令或工具调用只是投机执行过程中的一个环节，不把它称为一次投机执行。

### 1.1 Branch-local state

Branch-local state 包括 Agent 进程内存、进程关系、环境变量、Branch-local workspace、临时文件和 Branch 私有 artifact 等。它们不属于共享业务变化，但仍然是投机执行过程中需要保护的状态变化。

这类状态由 Instrumentation 在可信宿主负责本地管理；Runtime 管理宿主生命周期，并为 Branch 工作区访问、受管进程与效果提供隔离执行环境及环境级资源边界。Effect Management 关注的是这些状态在投机执行过程结束时如何处理：被淘汰的 Branch 不能留下未归属的状态；被接受的 winner 需要继续使用的状态不能因 Branch 切换而丢失；未满足必要条件的状态不能进入最终执行状态。具体的进程、Workspace 和环境状态语义由相应设计文档定义。

### 1.2 Shared observable business change

Shared observable business change 是本文的描述中心。它包括会被其他 Branch、winner continuation 或目标业务观察，并可能改变后续业务行为的变化，例如数据库写入、远程资源创建、共享服务记录修改、发送邮件，以及直接改变后续请求结果的共享约束。

Effect Management 必须为这类变化确定：变化在哪里发生、哪些 Branch 或外部业务可以观察、是否需要暂存、转换、延迟或阻塞、何时可以提交，以及投机执行过程失败、取消或 Branch 被淘汰后如何清理或补偿。是否开启整个投机执行由 `admit` 决定，不由某个 operation 环节的效果管理替代。只要变化会影响保证范围内的可观察业务行为，就属于 AScout 为正确性必须管理的最低范围，Policy 不能将其排除。

### 1.3 Outcome

Outcome 是投机执行过程中的结果或后续输入，例如工具返回值、API 响应、命令结果、用户输入或 validator 判断。Outcome 首先由 Outcome 机制负责产生、规范化、验证和交付；它不因为是执行结果就自动等同于外部效果。

Policy 可以要求对特定 Outcome 增加受控保存、验证、复用、失效或交付约束。当 Outcome 本身包含共享业务变化，或会改变保证范围内的可观察业务行为时，它同时进入 Effect Management 的最低纳管范围。否则，它按照 Branch、Outcome 和受控通信语义处理。

### 1.4 Operational footprint

Operational footprint 包括 token 费用、API 请求次数、provider 内部 audit、telemetry、CPU，以及额度、限流和并发槽的使用等运行影响。它们默认只归属和计量，不自动成为 Branch 世界状态，也不自动进入 Effect Management。

Policy 可以将其中指定的资源或额度声明为准入约束、预算约束或取消条件。只有某项变化实际改变保证范围内的可观察业务行为时，才进入 Effect Management；此时必须通过相应的 EffectClass、StateDomain 和 Runtime 能力处理。不能把资源消耗是否纳入约束，与某个业务效果本身采用何种处理纪律混为一谈。

### 1.5 Observation surface 与 guarantee horizon

AScout 不追踪无限递归的理论因果链。Service Adapter 声明 `observation surface` 和 `guarantee horizon`，确定哪些外部响应、受管状态、服务契约定义的触发动作，以及直接改变后续请求结果的共享约束属于当前保证范围，以及保证持续到什么范围。

在声明的范围内，sibling 的私有变化不能改变其他 Branch 的观察或约束；被淘汰 Branch 的共享语义变化不能进入最终执行状态；winner 需要继续使用的已提交效果不能因为 Branch 切换而丢失。Contract 和 guarantee horizon 之外的 provider 内部状态或 operational footprint 只按已声明能力记录和计量，AScout 不对其声称隔离保证。

在上述声明边界内，最终 semantic state 等价于 committed parent operations 与 winner continuation。

因此，Policy 可以要求对 Outcome 或 operational footprint 进行额外纳管，但不能移除 AScout 为保证正确性而规定的最低纳管范围。只有超出默认资源计量、并且实际改变保证范围内可观察业务行为的资源或结果变化，才需要作为效果进入下面的处理流程。

## 2. 效果管理过程

Effect Management 处理的是投机执行过程中的某个 operation 环节所产生的状态变化，而不是替代投机执行过程本身。它不负责生成 Proposal、验证 Proposal 的兼容性、选择 candidate 或决定投机执行过程的拓扑；这些决策由相应的 Policy、Speculation Coordinator 和其他 control plane 机制完成。它负责把这些决策涉及的状态变化落实为可检查、可执行和可清理的处理计划。

一次效果管理过程通常沿着以下顺序展开：

```text
识别状态变化
    -> 确定状态域与保证范围
    -> 检查 Policy、Contract 与 Runtime 能力
    -> 形成处理计划
    -> 执行、暂存或限制状态变化
    -> 记录 obligation 和执行归属
    -> 根据验证或裁决结果提交、继续、取消、清理或补偿
```

首先，系统识别该 operation 环节可能产生的 Branch-local state、shared observable business change、Outcome 和 operational footprint，并确定它们是否落入 Runtime 的保证范围。纯 Branch-local 且未被 Policy 或 Contract 要求保护的可逆变化，可以继续使用共享执行视图，不要求立即建立独立 Runtime 环境；Instrumentation 识别到需要独立 Branch-local 状态的变化时，必须在变化发生前请求 lazy 创建满足要求的承载。随后，Instrumentation 本地求值 Policy 的 `effect`，结合 Speculation Contract、EffectClass、StateDomain、资源条件和可用执行能力落实处理方式。处理计划除了允许、暂存、转换和补偿，也可以规定在某个效果边界阻塞当前投机执行：分支保留阻塞前的进展，等待后续裁决。只有当 Runtime 无法安全表示该阻塞、无法在接受后继续，或无法在淘汰后清理相关责任时，才必须拒绝相关环节的投机或明确降低整个投机执行过程的保证等级。

伪装执行（`simulate`）建立在暂存之上。Instrumentation 先将真实外部请求暂存在本机，不让请求离开本机；模拟在此基础上返回明确标记来源的临时结果，让 Branch 继续。Branch 被接受后才处理被暂存的真实请求；Branch 被丢弃或前提失效则取消真实请求。模拟结果不会自动变成真实调用。Policy 的 `effect` 决定处理方式，由 Instrumentation 本地求值并落实；被定为模拟的操作交给 Runtime 内的 External Effect Coordinator。不能模拟时，真实请求继续暂存在本机，Branch 不获得模拟结果，而是在效果边界阻塞并保留已完成进展；该 Branch 成为权威后才发出真实请求并继续，丢弃或前提失效则取消请求。不能模拟本身不构成拒绝整个投机的理由。模拟响应不是真实 Authority，也不是真实的观测结果，而是一个未经确认的假设；它必须作为该环节的未验证值参与 AssumptionSet 的确认与否定流程，仍需通过 Authority、validator 或其他明确规则验证后才能被 winner 依赖。若把模拟值静默当作真值使用，后续 step 就会建立在编造的 observation 上，即使 L0 全等匹配也无法掩盖，破坏点发生在验证机制不可见的位置。因此伪装执行只推进 Branch-local 的继续执行；凡由模拟值推导出的共享业务变化仍受 obligation 与 commit barrier 约束，确认前不得释放。Policy 通过 `effect` 决策的 `simulate` 模式选择该方式，选择与合法性检查见 [Policy Decisions](policy-decisions.md) §2.5。

在执行期间，Branch-local state 按 Policy、Contract 和实际 Runtime 能力进入满足要求的承载环境，并由 Instrumentation 维持 Session 等本地管理上下文；状态可以隔离，也可以在保证允许时共享或采用其他低成本方式。本地文件与进程的隔离由相应 backend 承担；网络外部 API 的模拟由 External Effect Coordinator 与 network backend 协同处理。

当 Authority 或 validator 提供验证依据后，control plane 根据既定决策形成后续处理。满足条件的状态和效果可以随 winner 继续，满足提交条件的暂存效果可以提交；不满足条件、被取消或属于淘汰 Branch 的状态和效果必须取消、清理或按声明的补偿方式处理。结果复用和交付必须通过受控通信完成，不能借此暴露 sibling 的私有状态。Branch 的创建、commit、accept 和 discard 语义由 Branch 语义文档定义；本节只说明这些操作触发的效果处理责任。

## 3. 效果处理纪律与状态域

对于一个投机执行过程中的 operation 环节，Effect Management 需要同时描述它作用于哪一类状态，以及该状态变化可以采用什么处理纪律。`StateDomain` 描述作用范围，`EffectClass` 描述处理方式；二者共同约束处理计划，但不要求一个 operation 环节只能有一个类别，也不把某个 Runtime 实现写死为唯一方案。

`PURE` 表示在声明的保证范围内没有需要提交、取消、补偿或隔离的语义效果；`IDEMPOTENT` 表示按声明条件重放可以收敛到相同状态；`STAGEABLE` 表示效果可以先暂存，在提交条件满足后再使其生效；`TRANSFORMABLE` 表示存在不产生原始变异的等价执行，例如 dry-run 或 staging 变体；`COMPENSABLE` 表示效果发生后可以执行声明的补偿；`OVERWRITABLE` 表示效果按约定的顺序或覆盖规则收敛；`IRREVERSIBLE` 表示真实效果一旦释放，没有可接受的撤销或补偿方式；真实请求仍可在发送前暂存。遇到这类效果时，Effect Management 可以让当前投机执行阻塞在效果边界，等待该 Branch 被接受或淘汰；对于已接入专门 Adapter、能够生成并明确标记模拟结果的服务，也可以采用伪装执行，用合成的 Branch 内响应代替真实请求，从而根本不触碰外部世界（见第 2 节）；如果既无法阻塞又无法伪装，才必须拒绝相关环节的投机或明确降级。一个环节可以同时具备多种处理纪律，Policy 可以指定优先级或 fallback，Runtime 则声明具体能力、限制和成本。Overlay、容器、虚拟机等只是这些能力的不同实现，可以组合、替换或省略。

常见的状态域包括 filesystem、process、memory、GUI、network、database、email 和 payment。不同状态域可以采用不同的处理纪律和 Runtime 机制：filesystem 通常使用 Branch-local workspace 或 overlay 进行暂存；process 和 memory 可以使用 namespace、COW fork 或 VM 快照；GUI 可以使用影子渲染或离屏合成；network read 可以在保证范围内直通；network write 可以通过代理、录制回放或延迟发送处理；database 可以使用未提交事务、savepoint 或确定性 key；email 和 payment 通常需要 outbox、补偿或硬闸。

这些组合是语义约束和能力协商的输入，不是静态白名单的替代品，也不代表每个组合只有一种实现。Policy 对效果处理的选择必须经过 Contract、资源条件和 Runtime 内部效果协调机制及服务插件能力校验；Runtime 报告环境隔离、资源、暂停、恢复、终止、清理和效果处理等能力。能力不足时，系统优先在实际到达相关效果边界时阻塞当前投机执行并保留前置进展；如果 Runtime 连阻塞、接受后的继续或淘汰后的清理都无法保证，才拒绝为相关环节启动投机执行，或明确降低整个投机执行过程的保证等级。资源或额度被 Policy 纳入准入、预算或取消约束，也不因此自动改变基础 EffectClass。

## 4. 责任边界

Policy 决定是否对特定 Outcome 或 operational footprint 增加约束，选择效果处理纪律、优先级、预算、取消条件和 fallback。Policy 可以扩大纳管要求，但不能排除 AScout 为保证正确性规定的最低范围，也不能把控制计划当成 Runtime 已完成的操作。

External Effect Coordinator 属于 Runtime，只处理本机隔离覆盖不到、需通过网络访问环境之外的 API 操作，与 network backend 协同，不负责本地文件和进程隔离。Policy 的 `effect` 决定处理方式，Instrumentation 本地落实并将模拟操作交给它；它判断能否模拟。不能模拟时，真实请求继续暂存在本机，当前 Branch 在效果边界阻塞并保留已完成进展，不获得模拟结果，也不因此直接拒绝整个投机；该 Branch 成为权威后才发出真实请求并继续，丢弃或前提失效则取消请求。可模拟时才返回明确标记来源的临时结果。它无需为跨 Branch 命令去重获知整棵树中的相同命令，也不承担 Tree 的全局裁决。control plane 在 Branch 被接受、丢弃或前提失效时，只通知相应暂存请求释放或取消；接受仍须满足 Authority 与既有提交条件，不能因嵌套接受提前释放请求。

Runtime 负责受管效果的实际执行和强制处理，并持有效果 obligation 与执行归属；Instrumentation 负责 Agent 执行实体和 Session 的本地管理。Effect Management 不替代 Execution、Workspace 或其他状态域的状态责任。

Outcome 负责结果的产生、规范化和交付，Resource 负责资源压力、预算和准入，Ledger 提供回放与审计的记录来源，不驱动回放执行。系统外替换模型输出不等于录制或回放工具效果；真实工具仍受 Runtime、效果纪律和候选作用前门槛约束。Policy 要求这些对象受到额外效果约束时，它们通过 Contract 与相应效果处理机制协作，但不因此互相取代；本地隔离与外部 API 模拟仍遵守各自责任边界。

## 5. 固定保证与明确边界

AScout 固定提供以下保证：

- 未满足条件的共享业务变化不会进入保证范围内的最终执行状态。
- 暂存效果在条件满足前不会被不应看到它的 Branch 观察。
- 伪装执行产生的模拟响应不会以真实 Authority 或真实观测的身份进入后续语义；它必须明确标记为模拟结果并记录生成来源，作为未验证假设在确认前不驱动 winner 或共享状态。
- 在效果边界阻塞的投机执行会保留阻塞前的进展；该效果在 Branch 被接受前不会发生，Branch 被淘汰时也不会残留。
- 被淘汰 Branch 的受管状态和效果会被清理、取消或按声明的方式补偿。
- winner 需要延续的状态和已提交生效的效果不会因 Branch 切换而丢失。
- control plane 形成的效果计划必须由 Runtime 实际执行和强制，不能只有 metadata 对齐；涉及环境、进程或资源的动作由 Runtime 按其能力契约执行并返回结果。

AScout 不作以下保证：

- 不保证 Service Adapter contract 和 guarantee horizon 外的 provider 内部状态。
- 不自动把所有 operational footprint 纳入 Branch 世界状态或效果语义。
- 不自动推断任意 operation 环节的幂等性、可补偿性、可暂存性或可转换性。
- 不负责 Proposal 生成、兼容性验证、candidate 选择和投机执行过程的拓扑。

## 相关文档

- [心智模型](../mental-model.md) §状态与效果：高层说明
- [设计模型](../design-model.md)：抽象概念层与跨领域约束
- [Branch 语义设计](branch-semantics.md)：Branch 语义与隔离通信
- [control plane 设计](control-plane.md)：全局裁决与暂存请求的释放或取消通知
- [Speculation Contract 设计](speculation-contract.md)：Policy 表达与 runtime proxy 接入行为的语义对齐契约
