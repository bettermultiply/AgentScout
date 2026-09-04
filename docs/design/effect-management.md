# 效果管理设计

本文承载 AScout 对 Branch 运行中状态变化和外部影响的管理语义。Effect Management 管理投机执行过程中需要保护的状态变化；其中以共享、可观察的业务变化为描述中心，Branch-local state、Outcome 和 operational footprint 按各自的规则处理。

## 1. 管理范围与保证边界

投机分支对分支外部、共享或最终执行状态产生的影响，以及这些影响在投机期间的暂存、限制、提交、取消和清理，均属于 Effect Management 的管理范围。单个动作、命令或工具调用只是投机执行过程中的一个环节，不把它称为一次投机执行。

### 1.1 Branch-local state

Branch-local state 包括 Agent 进程内存、进程关系、环境变量、Branch-local workspace、临时文件和 Branch 私有 artifact 等。它们不属于共享业务变化，但仍然是投机执行过程中需要保护的状态变化。

这类状态的隔离、清理和 winner 延续主要由 Runtime 自动完成。Effect Management 关注的是这些状态在投机执行过程结束时如何处理：被淘汰的 Branch 不能留下未归属的状态；被接受的 winner 需要继续使用的状态不能因 Branch 切换而丢失；未满足必要条件的状态不能进入最终执行状态。具体的进程、Workspace 和 Runtime 状态语义由相应设计文档定义。

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

因此，Policy 可以要求对 Outcome 或 operational footprint 进行额外纳管，但不能移除 AScout 为保证正确性而规定的最低纳管范围。只有超出默认资源计量、并且实际改变保证范围内可观察业务行为的资源或结果变化，才需要作为效果进入下面的处理流程。

## 2. 效果管理过程

Effect Management 处理的是投机执行过程中的某个 operation 环节所产生的状态变化，而不是替代投机执行过程本身。它不负责生成 Proposal、验证 Proposal 的兼容性、选择 candidate 或决定投机执行过程的拓扑；这些决策由相应的 Policy、Speculation Coordinator 和其他控制面机制完成。它负责把这些决策涉及的状态变化落实为可检查、可执行和可清理的处理计划。

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

首先，系统识别该 operation 环节可能产生的 Branch-local state、shared observable business change、Outcome 和 operational footprint，并确定它们是否落入 Adapter 声明的保证范围。纯 Branch-local 且未被 Policy 或 Contract 要求保护的可逆变化，可以继续使用共享执行视图，不要求立即建立独立 Runtime 环境；接入侧识别到需要独立 Branch-local 状态的变化时，必须在变化发生前请求 lazy 创建独立环境。随后，控制面结合 Policy、Speculation Contract、EffectClass、StateDomain、资源条件和 Runtime capability 形成处理计划。处理计划除了允许、暂存、转换和补偿，也可以规定在某个效果边界阻塞当前投机执行：分支保留阻塞前的进展，等待后续裁决。只有 Runtime 无法安全表示该阻塞、无法在接受后以权威身份继续，或无法在淘汰后清理相关责任时，才必须拒绝相关环节的投机或明确降低整个投机执行过程的保证等级。

伪装执行（`simulate`）是对某个已注册服务效果插件的外部服务采用的一种处理方式。当真实请求一旦发出就会产生不可逆或难以管理的外部影响，且该服务不能安全地暂存、转换或补偿时，Agent 接入侧拦截请求而不把它转发给真实服务，将受管请求交给控制面内的插件；插件在控制面构造一个明确标记为模拟结果并记录生成来源的响应，再经 Agent Control Gateway 返回接入侧，使该环节在本地表现为已经完成。这样投机阶段完全不触碰外部世界，把不可逆影响推迟到 Branch 被接受之后。伪装执行不能泛化到任意服务：只有已注册插件明确声明哪些命令或 URL 可以模拟、模拟响应如何生成，Policy 才能选择它。模拟响应不是真实 Authority，也不是真实的观测结果，而是一个未经确认的假设；它必须作为该环节的未验证值参与 AssumptionSet 的确认与否定流程，仍需通过 Authority、validator 或其他明确规则验证后才能被 winner 依赖。若把模拟值静默当作真值使用，后续 step 就会建立在编造的 observation 上，即使 L0 全等匹配也无法掩盖，破坏点发生在验证机制不可见的位置。因此伪装执行只推进 Branch-local 的继续执行；凡由模拟值推导出的共享业务变化仍受 obligation 与 commit barrier 约束，确认前不得释放。Policy 通过 `effect` 决策的 `simulate` 模式选择该方式，选择与合法性检查见 [Policy Decisions](policy-decisions.md) §2.5。

在执行期间，Branch-local state 按 Policy、Contract 和实际环境建立状态进入 Branch 自己的隔离环境；未被要求保护且不影响保证范围的完全可逆变化可以继续使用共享执行视图。共享业务变化进入受管的暂存、代理或延迟路径；需要受控的 Outcome 进入相应的结果槽或通信路径；纳入 Policy 约束的 operational footprint 进入资源预算和取消判断。Runtime 需要记录效果的 obligation、执行归属和清理责任，不能只在控制面记录一个计划而不实际约束执行。

当 Authority 或 validator 提供验证依据后，控制面根据既定决策形成后续处理。满足条件的状态和效果可以随 winner 继续，满足提交条件的暂存效果可以提交；不满足条件、被取消或属于淘汰 Branch 的状态和效果必须取消、清理或按声明的补偿方式处理。结果复用和交付必须通过受控通信完成，不能借此暴露 sibling 的私有状态。Branch 的创建、commit、accept 和 discard 语义由 Branch 语义文档定义；本节只说明这些操作触发的效果处理责任。

## 3. 效果处理纪律与状态域

对于一个投机执行过程中的 operation 环节，Effect Management 需要同时描述它作用于哪一类状态，以及该状态变化可以采用什么处理纪律。`StateDomain` 描述作用范围，`EffectClass` 描述处理方式；二者共同约束处理计划，但不要求一个 operation 环节只能有一个类别，也不把某个 Runtime 实现写死为唯一方案。

`PURE` 表示在声明的保证范围内没有需要提交、取消、补偿或隔离的语义效果；`IDEMPOTENT` 表示按声明条件重放可以收敛到相同状态；`STAGEABLE` 表示效果可以先暂存，在提交条件满足后再使其生效；`TRANSFORMABLE` 表示存在不产生原始变异的等价执行，例如 dry-run 或 staging 变体；`COMPENSABLE` 表示效果发生后可以执行声明的补偿；`OVERWRITABLE` 表示效果按约定的顺序或覆盖规则收敛；`IRREVERSIBLE` 表示没有可接受的提前释放、暂存、转换或补偿方式。遇到这类效果时，Effect Management 可以让当前投机执行阻塞在效果边界，等待该 Branch 被接受或淘汰；对于已接入专门 Adapter、能够生成并明确标记模拟结果的服务，也可以采用伪装执行，用合成的 Branch 内响应代替真实请求，从而根本不触碰外部世界（见第 2 节）；如果既无法阻塞又无法伪装，才必须拒绝相关环节的投机或明确降级。一个环节可以同时具备多种处理纪律，Policy 可以指定优先级或 fallback，Runtime Adapter 则声明具体能力、限制和成本。

常见的状态域包括 filesystem、process、memory、GUI、network、database、email 和 payment。不同状态域可以采用不同的处理纪律和 Runtime 机制：filesystem 通常使用 Branch-local workspace 或 overlay 进行暂存；process 和 memory 可以使用 namespace、COW fork 或 VM 快照；GUI 可以使用影子渲染或离屏合成；network read 可以在保证范围内直通；network write 可以通过代理、录制回放或延迟发送处理；database 可以使用未提交事务、savepoint 或确定性 key；email 和 payment 通常需要 outbox、补偿或硬闸。

这些组合是语义约束和能力协商的输入，不是静态白名单的替代品，也不代表每个组合只有一种实现。Policy 对效果处理的选择必须经过 Contract、资源条件和 Runtime capability 校验；Runtime Adapter 报告具体后端能够提供的隔离、暂存、转换、阻塞、补偿、取消和清理能力。能力不足时，系统优先在实际到达相关效果边界时阻塞当前投机执行并保留前置进展；如果连阻塞、接受后的继续或淘汰后的清理都无法保证，才拒绝为相关环节启动投机执行，或明确降低整个投机执行过程的保证等级。资源或额度被 Policy 纳入准入、预算或取消约束，也不因此自动改变基础 EffectClass。

## 4. 责任边界

Policy 决定是否对特定 Outcome 或 operational footprint 增加约束，选择效果处理纪律、优先级、预算、取消条件和 fallback。Policy 可以扩大纳管要求，但不能排除 AScout 为保证正确性规定的最低范围，也不能把控制计划当成 Runtime 已完成的操作。

Effect Coordinator 位于控制面，持有效果分类、相关规则、服务效果插件和协调计划。Agent 接入侧在效果发生前识别受管命令或 URL，并将请求交给 Effect Coordinator；插件可以在控制面直接生成模拟响应、拒绝或阻塞请求，也可以根据 Task、Branch 和全局效果状态执行去重及跨 Branch 协调。它结合 Policy、Speculation Contract 与 Runtime capability，形成暂存、转换、延迟、阻塞、补偿、拒绝或其他处理计划；只有需要 Runtime 提供对应执行机制时，才经 Runtime Adapter 请求 Runtime 执行。插件在控制面启动或 Policy 加载阶段注册，Task 使用的 Policy 生效后不热替换。Effect Coordinator 不负责普通 Branch-local 操作，也不持有真实的暂存效果、obligation、隔离环境或效果提交和清理状态。

Runtime Effect backend 负责实际的效果隔离和强制执行，持有 obligation 与执行归属，完成暂存、提交、取消、清理和必要的补偿，并在控制面计划被绕过时 fail closed。Runtime 还负责 Branch-local state 的隔离、清理和 winner 延续；Effect Management 不替代 Execution、Workspace 或其他 Runtime backend 的状态所有权。

Outcome 负责结果的产生、规范化和交付，Resource 负责资源压力、预算和准入，Ledger 负责记录、回放和审计。Policy 要求这些对象受到额外效果约束时，它们通过 Contract 和 Effect Coordinator 协作，但不因此互相取代，也不改变 Runtime 的最终强制责任。

## 5. 固定保证与明确边界

AScout 固定提供以下保证：

- 未满足条件的共享业务变化不会进入保证范围内的最终执行状态。
- 暂存效果在条件满足前不会被不应看到它的 Branch 观察。
- 伪装执行产生的模拟响应不会以真实 Authority 或真实观测的身份进入后续语义；它必须明确标记为模拟结果并记录生成来源，作为未验证假设在确认前不驱动 winner 或共享状态。
- 在效果边界阻塞的投机执行会保留阻塞前的进展；该效果在 Branch 被接受前不会发生，Branch 被淘汰时也不会残留。
- 被淘汰 Branch 的受管状态和效果会被清理、取消或按声明的方式补偿。
- winner 需要延续的状态和已提交生效的效果不会因 Branch 切换而丢失。
- 控制面形成的效果计划必须由 Runtime 实际执行和强制，不能只有 metadata 对齐。

AScout 不作以下保证：

- 不保证 Service Adapter contract 和 guarantee horizon 外的 provider 内部状态。
- 不自动把所有 operational footprint 纳入 Branch 世界状态或效果语义。
- 不自动推断任意 operation 环节的幂等性、可补偿性、可暂存性或可转换性。
- 不负责 Proposal 生成、兼容性验证、candidate 选择和投机执行过程的拓扑。

## 相关文档

- [心智模型](../mental-model.md) §状态与效果：高层说明
- [设计模型](../design-model.md)：抽象概念层与跨领域约束
- [Branch 语义设计](branch-semantics.md)：Branch 语义与隔离通信
- [控制面设计](control-plane.md)：控制面组件与效果协调
- [Speculation Contract 设计](speculation-contract.md)：Policy 表达与 Runtime Adapter 接入行为的语义对齐契约
