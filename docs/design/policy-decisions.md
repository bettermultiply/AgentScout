# Policy Decisions

Policy Decision 是 Policy 针对一个投机机会作出的可替换选择。对一条投机 Edge `E`，一次投机执行可以由以下七类 decision 的组合刻画：

```text
S(E) = (admit, extent, predict, launch, effect, match, dispose)
```

这七类 decision 属于同一个决策面，但不要求按固定顺序调用。一次执行仍可按“机会发现、准入判断、预测生成、过程执行、验证、处置与提交”理解；决策面表达的是 Policy 在这些阶段中可以选择什么，而不是另一条固定流程。

| Decision | Policy 决定什么 | 主要阶段 |
|---|---|---|
| `admit` | 该操作是否值得作为投机入口，以及当前约束下是否开启投机（同一决策内的两级判断） | 准入判断 |
| `extent` | 已开启的投机执行在内部推进到哪里，以及暂停等待裁决的时机 | 过程执行 |
| `predict` | 预测对象与 Proposal 生成数量与方式 | 预测生成 |
| `launch` | 是否取得这条 Edge 的权威值以及何时取得 | 准入判断、权威路径启动 |
| `effect` | 投机过程中产生的外部效果采用的处理纪律 | 过程执行、处置与提交 |
| `match` | Proposal 与验证依据被视为兼容或可接受的时机 | 验证 |
| `dispose` | 命中、未命中、取消或停止后投机工作的处置 | 处置与提交 |

Policy 允许这些 decision 依赖任务、执行上下文、当前状态、资源条件、预期收益和 Runtime 能力。依赖关系应显式表达，不能通过隐含的 Runtime 行为改变 decision 的含义。

## 2. 七类 Decision

### 2.1 `admit` - 是否开启投机

```
opportunity_pass(op) -> true | false              # 粗筛：该操作是否可能成为投机入口
feasibility_pass(state, profile) -> true | false  # 精判：当前约束下是否值得开启
admit(op, state, contract_context) = opportunity_pass AND feasibility_pass
```

`admit` 决定一次投机是否开启，其内部按判断成本拆成两级，先廉价后昂贵：先用操作的机会事实（`step`）判断该操作有没有可能成为投机入口，再结合当前约束与实例事实（`state`）以及 Contract 校验产生的能力与限制事实（`contract_context`），判断这个已识别出的机会当前是否值得开启。

一个 `step` 是一个可被控制面观察到、具有明确前后状态的 Agent 继续单元。它可以是一次推理转移、一次用户输入请求、一次工具调用或远程请求，也可以是一次根据已有结果更新计划或生成输出的过程阶段。等待用户输入或工具结果本身不算持续推进；通常由一个 step 产生 pending-result boundary，过程随后暂停。

粗筛判断机会是否可能存在，只读操作的可观察事实，不涉及任何预测：

| 事实 | 含义 |
|---|---|
| 正在执行的是哪个操作 | 操作的结果被后续步骤阻塞，投机才有可隐藏的等待 |
| 操作是否产生一个结果 | 该操作会产出需要稍后才能确定的结果，投机才有预测对象 |
| 是否已有对该结果可用的预测手段 | 没有预测手段则投机无从谈起 |

粗筛通常近静态，可以按操作类别在声明期确定，不必到运行时逐次估算。粗筛不过的操作按普通执行继续，不进入投机决策面，也不投入任何投机机制。

精判判断机会当前是否值得，输入分为两类：

- AScout 提供的约束与实例事实：预算与资源计数、并发压力、当前实例已携带的 `AssumptionSet` 及其成本；
- Policy 自行建模的经济评估：预期命中与收益对失败成本的权衡、实例存活概率等。AScout 只提供评估依据，不代 Policy 下主观结论。


### 2.2 `extent` - 单个过程推进幅度

```
extent(process_state) -> stop | continue
```

`extent` 由一组停止条件集合和根据过程事实(`process_state`)动态计算的函数组成。用来表达一个已经开启的投机执行过程希望在内部推进到哪个前沿。`process_state` 由 AScout 提供，不包含任何主观条件。

- 过程进度：至少包括已完成的 step 数和过程当前所处的语义阶段；
- 已实现目标：已经产生的输出、已完成的目标项、已消耗的明确预算或资源计数；
- 过程语义：由任务或 Policy 定义的状态，例如必要信息是否齐全、当前阶段是否完成。

当前层确定的规范字段如下。它们是 `process_state` 的语义字段，不等同于某种具体 API 格式：

| 字段 | 是否必需 | 含义 |
|---|---:|---|
| `steps_completed` | 是 | 当前投机过程已经完成的 step 数，从 0 开始计数 |
| `phase` | 是 | 当前过程所处的稳定语义阶段，例如 reasoning、input-wait、tool-call、result-processing |
| `active_time` | 否 | 实际执行时间，不包含等待用户输入或外部结果的暂停时间 |
| `resource_counters` | 否 | 已实际消耗的可计量资源，例如 token 数、工具调用次数、字节数；只记录事实，不计算成本 |
| `result_facts` | 否 | 已产生的输出、观测结果和已完成工作项 |
| `semantic_facts` | 否 | 由任务或 Policy 定义的语义状态，例如必要信息是否齐全、当前阶段是否完成 |

除 `steps_completed` 和 `phase` 外，字段可以按领域扩展，但必须保持为可观察事实；具体事件格式由下层接口文档定义。

`extent` 与嵌套深度不同，`extent` 仅约束当前 Edge 对应的单个投机过程已经推进多少，每个嵌套 Edge 的子过程拥有独立的 step 计数，子过程的 step 默认不计入父过程。

`extent` 的停止条件收敛为三类 Policy 可动态评估的谓词：

```text
progress_stop(process_state) -> true | false  # 根据已观察的推进事实判断
utility_stop(process_state) -> true | false  # 根据已实现且可观察的目标或效用事实判断
semantic_stop(process_state) -> true | false  # 根据任务或过程语义判断
```

三类条件可以组合；任一已声明条件成立，即表示 Policy 期望当前 `extent` 已到达：
`extent_reached = progress_stop OR utility_stop OR semantic_stop`

`extent` 属于次级指标，当运行被 effect、Runtime、控制面等阻塞时则不再检查，即使停止条件均未到底。而达到 `extent` 边界通常意味着暂停并等待验证或其他裁决，不意味着自动 discard。过程是否继续、转为权威、清理或采用其他路径，由后续验证和 `dispose` 决定。

### 2.3 `predict` - 生成哪些 Proposal

```text
predict(predict_input) -> ProposalSet
```

`predict` 将当前 Edge 和执行上下文中的事实映射为一个或多个 Proposal。`predict_input` 当前层确定包括以下输入类别：

| 输入 | 是否必需 | 含义 |
|---|---:|---|
| `edge_context` | 是 | 当前 Edge 的 pending-result boundary、被预测结果的类型/结构，以及后续依赖点的基本描述 |
| `process_context` | 是 | 当前 Agent 过程可见的状态，包括上下文、当前阶段、已有输入和已完成结果 |
| `assumption_set` | 是 | 当前 Branch 已携带的未决 Proposal ID 集合；预测不能假装这些 Proposal 已经被确认 |
| `history` | 否 | 当前 Agent 或同类过程已经产生的历史轨迹、观测和结果 |
| `policy_config` | 是 | Policy 自己的模型、规则、阈值、候选数量限制等配置 |
| `capability_context` | 是 | Contract/Runtime 已声明的可用能力和约束，例如相关效果是否可处理、可用预算或最大候选数 |

`edge_context` 确定要预测哪个边界结果；`process_context` 和 `history` 提供生成候选所需的事实；`assumption_set` 保持未验证前提的可见性；`capability_context` 说明候选是否具备安全执行和验证的条件。来源可以是模型、规则、历史轨迹、外部系统或其他 Policy 组件，但来源不改变 Proposal 的语义。

`predict` 可以使用这些输入作出推断或规则判断，但不负责验证 Proposal 是否正确，也不负责决定 Proposal 最终能否提交。

`ProposalSet` 统一表示输出：

```text
ProposalSet = {
    proposals: [Proposal, ...],
    generation: batch | incremental # 表示 Proposal 的生成是一次性的还是递增的
}
```

每个 `Proposal` 至少包含一个当前 Edge 边界的候选输入：

```text
Proposal = {
    payload,
    provenance?
}
```

`payload` 可以是原子值或结构化值；`provenance` 可选，用于记录候选来自模型、规则、历史轨迹或外部系统等来源。结构化计划仍只是边界上的候选输入，实际展开由 candidate Branch 完成。输出可以是单候选、有限候选集、带 Policy 元数据的候选集，或增量生成的候选序列。

同一个 Edge 可以有多个 Proposal，每个 Proposal 可以派生出一个候选 Branch。Policy 可以根据自己的配置和已观察事实选择固定或动态的数量，但候选 Branch 仍必须遵守隔离、AssumptionSet 和 single-winner 约束。候选数量决定了投机的宽度 breadth；breadth 是 `predict` 的取值结果。每个已发出的 Proposal 必须不可变；如果 Policy 无法生成有效候选，应回退到普通/权威路径或结束本次投机入口。

预测对象不局限于某种 Agent 行为。它可以是 action、observation、工具调用、命令、远程请求、用户输入或它们组成的计划，只要后续工作可以暂时依赖该 Proposal，并且存在验证或接受它的依据。

### 2.4 `launch` - 是否取得权威值

```text
launch: eager | lazy | absent
```

`launch` 决定这条 Edge 的权威值如何取得、是否取得、何时取得。权威值是该 Edge 真实结果 `i` 的可靠来源——真实执行、主生成流、用户输入、环境反馈或远程服务——只有拿到它，`match` 才能与 Proposal 对照；拿不到时只能依赖 validator 或其他可接受性判断。

“权威值来自谁”不是 Policy 可以替换的选项，而是这条 Edge 的固有事实：`u` 是谁，真实结果就由谁产生。例如 `u` 是一条命令时真值来自真实执行，`u` 在等待用户输入时真值来自用户。Policy 能决定的只有“是否取得、何时取得”，即 `launch` 的取值：

| 取值 | 语义 |
|---|---|
| **Eager** | 尽早取得权威值：权威路径可并行启动就与投机工作并行启动；权威来自外部（用户、环境）则照常等待其真实到达 |
| **Lazy** | 先只靠 validator 或其他可接受性判断裁决；投机结果不可接受时才补取权威值，包括在此刻询问用户 |
| **Absent** | 从不取得权威值，即使它天然存在（例如来自用户输入）也可以放弃，只靠 validator 或其他可接受性判断裁决 |

Absent 允许策略放弃本应到来的用户输入，只按可接受性判断放行；这通常降低 Fidelity，一般不可取，但属于 Policy 的合法选择，能否采用由 Fidelity 契约校验。

权威路径能否被“启动”取决于其来源形态。来源是系统内的可发起路径（更强的 target、主生成流、可重跑的真实执行）时，Eager 意味着并行启动、Lazy 意味着稍后补启；来源是系统外的天然输入（用户、环境）时没有可“启动”的路径，Eager/Lazy 只表达“照常等待”还是“必要时才等待”。

### 2.5 `effect` - 管理投机效果

`effect` 决定投机过程中每个 step 产生的外部效果如何纳入管理，以及允许采用哪些处理纪律。一次投机执行可能包含多个 step，并涉及不同的 `StateDomain` 与 `EffectClass`。

`effect` 对每个已识别的效果返回处理计划：

```text
effect(effect_descriptor, context) -> EffectPlan
```

处理计划的模式包括：

| 模式 | 含义 |
|---|---|
| `default` | 采用服务 Adapter/Contract 声明的默认方案；未声明默认方案时不能静默放行 |
| `simulate` | 拦截真实请求，不访问真实服务，在 Branch 内生成标记为 synthetic 的模拟响应 |
| `transform` | 将原操作转换为 dry-run、staging 或其他等价的非变异形式 |
| `stage` | 暂存真实效果，满足提交条件后再释放 |
| `mediate` | 通过受管代理或 broker 执行 |
| `block` | 在效果边界暂停当前 Branch，等待后续裁决 |
| `allow` | 在声明范围内直接执行真实请求 |
| `deny` | 直接拒绝该效果或拒绝相关投机环节 |

`simulate` 要求存在针对该服务的 Adapter，并由 Adapter 声明哪些请求可以模拟以及模拟响应如何生成。模拟响应不是真实 Authority，必须带有 synthetic 来源和未验证假设；后续仍需通过 Authority、validator 或其他明确规则验证。这即正式语义所称的**伪装执行**，其约束与保证见 [效果管理设计](effect-management.md)。`EffectClass` 描述效果可采用的处理纪律，`StateDomain` 描述效果作用的状态域，二者保持正交；Policy 选择的模式必须经过 Effect Management、Contract 和 Runtime capability 校验。

Policy 可以选择暂存、代理、延迟释放、阻塞、补偿或其他合法处理方式，但不能把效果排除在 AScout 的最低纳管范围之外。Effect Management 和 Runtime 必须检查所选纪律是否有能力落实。

如果某个效果暂时无法安全提前处理，系统可以在该效果边界阻塞当前 Branch，同时保留已经安全完成的前置进展；验证接受后继续，验证拒绝或过程取消后清理、补偿或回收。只有无法安全阻塞、无法在接受后继续、也无法在淘汰后清理时，才应拒绝相关投机环节或明确降级。

### 2.6 `match` - 判断兼容或可接受

`match` 将 Proposal、验证依据和比较上下文交给 Policy 规定的匹配规则：

```text
match(proposal, evidence, match_context) -> MatchResult
```

`evidence` 可以是 `authority` 提供的真实结果，也可以是 validator 提供的可接受性判断。没有 Authority 时，`match` 判断的是 Proposal 是否满足明确的验收条件，而不是假设存在一个可比较的真实值。

`MatchResult` 至少区分以下结果：

```text
MatchResult = {
    verdict: compatible | incompatible | indeterminate | error,
    score?: MatchScore
}
```

`score` 是可选的。严格相等或确定性 predicate 可以直接产生 `verdict`，不需要分数；模糊匹配、相似度匹配或带容差的语义判断可以返回分数，再依据 Policy 声明的阈值得出 `verdict`。分数的含义、计算函数、范围、方向和阈值全部由用户/Policy 定义，AScout 只提供承载、记录和在同一 Edge 的相同匹配规则上下文内进行比较的机制。分数不是 AScout 推断的正确概率，也不在不同 matcher、版本或 scale 之间比较。

匹配关系至少应明确以下内容：

- 比较的是 Proposal 本身、其输出，还是对后续行为的影响；
- 该关系需要权威值，还是只要 validator 的可接受性判断（权威值不可得或 launch=Absent 时）；
- 判断失败、不确定或 validator 出错时如何处理；
- 该关系允许的 Fidelity 上界。

放宽匹配关系可能使结果不再与串行执行完全一致，不能对声明为严格 trajectory-equivalent 的策略静默采用。`indeterminate` 和 `error` 不能被静默当成 `compatible`；后续如何暂停、重试、降级、重新执行或丢弃，由 `dispose` 和运行控制处理。分数只能支持兼容判断或同一 Edge 内的候选排序，不能绕过 Fidelity、效果边界或 single-winner 约束。

### 2.7 `dispose` - 处置投机工作

`dispose` 根据投机过程进入处置阶段的原因和当前上下文，返回一个可组合的处置计划：

```text
dispose(branch, resolution, context) -> DispositionPlan
```

`resolution` 表示为什么进入处置，而不是具体怎么处置：
`resolution =[accepted, rejected, indeterminate, error, cancelled, preempted, extent_reached, failed]`

`DispositionPlan` 可以包含一个或多个处置意图：

- **Reuse**：已完成的候选结果通过验证后直接复用；
- **Promote**：将仍在运行的候选 Branch 转为权威身份并继续执行；
- **Salvage**：提取安全的部分进展，交给权威路径或后续过程；
- **Re-execute**：使用权威结果重新执行必要工作；
- **Discard**：丢弃候选工作及其未确认产物；
- **Compensate**：对已发生且可补偿的效果执行反向处理；
- **Overwrite**：在明确允许的状态语义下由后写覆盖先写。

这些意图可以组合。例如，`Salvage + Discard` 表示先移交可保留的部分进展，再丢弃剩余候选工作；`Compensate + Discard` 表示先处理已发生效果，再清理分支。`Promote` 表示身份转换后继续执行，`Reuse` 表示采用候选结果并清理其他候选。`dispose` 不重新判断 `match`，也不决定过程是否还能继续推进；它只根据已有裁决选择后续处置。

这些是 Policy 的处置意图，不是对 Runtime 生命周期的直接调用。实际的 Branch 转换、状态回收、效果清理、资源释放和 commit/discard 由 Mechanism 与 Runtime 强制执行。未命中的候选路径不能通过逐个修补绕过其携带的错误前提；依赖被否定时，相关子图应整体进入回收或丢弃流程。

## 3. 阶段映射与决策依赖

下表将决策面映射到典型投机执行阶段。它不是固定调用顺序，也不是完整的决策清单。

| 阶段 | 代表性 Decision | 说明 |
|---|---|---|
| 机会发现 | `admit` | admit 判断该操作是否可能成为投机入口（粗筛），Mechanism 识别并规范化 operation 及其结果未到达的等待边界。 |
| 准入判断 | `admit`、`launch` | admit 判断当前约束下是否开启投机执行（精判）；launch 决定权威值是否取得、何时取得 |
| 预测生成 | `predict` | 生成一个或多个 Proposal |
| 过程执行 | `extent`、`effect` | 限制过程推进，并管理每个 step 的状态与外部效果 |
| 验证 | `match` | 获得验证依据并判断 Proposal 是否兼容或可接受；权威值是否可取由 launch 决定 |
| 处置与提交 | `dispose` | 复用、继续、转正、重执行、清理、回收或补偿 |

一个 decision 可以跨越多个阶段。例如，`effect` 的处理会影响过程执行和最终处置，`launch` 同时影响准入和验证依据的可用时间。阶段映射用于说明责任，不应被解释为把七类 decision 串成不可替换的固定流程。

## 4. 组合与嵌套

### 4.1 组合

同一 Edge 上的多个 Proposal/Branch 是并行候选关系，主要由 `predict` 的候选数量、Branch 隔离和 single-winner 约束决定，不引入额外的组合语义：

```text
Edge E
├── Proposal p1 -> Branch B1
├── Proposal p2 -> Branch B2
└── Proposal p3 -> Branch B3
```

组合候选共享同一个 pending-result boundary 和 execution base，但彼此默认不直接协作。候选数量是 breadth 的取值结果，最终至多一个 Branch 成为 winner。

### 4.2 嵌套

候选 Branch 被创建后会像普通执行一样继续运行，因此也可能遇到新的 pending-result boundary。嵌套是某个 Branch 在继续执行时遇到新的 Edge，并再次调用 `admit`：

```text
Edge E1
└── Proposal p1 -> Branch B1
    └── Edge E2
        └── Proposal p2 -> Branch B2
```

```text
AssumptionSet(B1) = {p1}
AssumptionSet(B2) = {p1, p2}
```

新 Edge 仍按照当前 task 已绑定的同一个 Policy profile 执行既有 decision；当前不因嵌套深度切换到另一套 Policy。新 Edge 具有自己的 Proposal、Branch、未决 Proposal 依赖和过程状态；子 Edge 的 `extent` 和 step 计数独立于父 Edge。未来按嵌套深度或其他上下文切换 Policy，属于独立扩展。

每个候选 Branch 携带其路径上的未决 Proposal ID 集合：`AssumptionSet = frozenset[ProposalId]`

`AssumptionSet == ∅` 表示工作单元是权威工作。Proposal 被确认时，从依赖它的工作单元中移除其 ID；集合变空的工作单元才具备提交条件。若父 Proposal 被确认，原父 Branch 结束其投机身份，其执行延续成为权威路径；仍携带其他未决 Proposal 的后代 Branch 保留原有身份、隔离环境和执行进展，仅将结构上的父级改挂到该权威执行延续之下，不创建新的 Branch，也不迁移 Runtime 状态。后代 Branch 仍须等自身 `AssumptionSet` 清空后，才具备将结果和效果纳入权威状态的资格。Proposal 被否定时，依赖它的后续子图失效；控制面向受影响的 Branch 直接发送取消命令，各 Branch 作为独立执行个体停止自身工作并完成本地及受管效果清理，不要求一个集中式机制统一遍历和回收整棵子树。只有父 Branch 进入 `Salvage` 时，后代 Branch 的部分进展才可作为该父 Branch salvage 的输入，不能脱离父 Branch 独立 salvage。非空 `AssumptionSet` 产生的数据只能流向同样携带未决 Proposal ID 的容器，不能直接进入权威上下文。

组合与嵌套通过四类连接共同实现：树结构归属、假设依赖、用户输入传播和生命周期控制。

用户输入包括普通输入和用户中断；二者都传播给当时仍在运行的全部 descendants。普通输入作为新的输入事实进入各 Branch；中断虽然同样来自用户输入，但会触发各 Branch 自己的停止/取消处理。输入和中断不通过 `AssumptionSet` 传播。

## 5. Fidelity 是导出契约

Fidelity 不是独立的 Policy Decision，而是由决策组合、Runtime 能力和 Policy 声明共同形成的契约。它至少受到以下因素影响：

- `launch` 是否取得权威值（Absent 意味着放弃真实依据，只能走可接受性裁决）；
- Edge 的权威来源本身是否定义正确性（固有事实，Policy 不能改变）；
- `match` 采用的等价或可接受关系；
- `effect` 是否在验证前释放外部效果；
- `dispose` 是否允许重执行、补偿或其他有损处置；
- predictor 或 Agent 本身是否存在声明之外的退化行为。

控制面应先根据这些选择推导可达到的 Fidelity 上界，再将上界与开发者声明进行比较。声明超过上界时，必须拒绝加载或明确降级。运行期间还应通过 shadow 或其他审计手段检查声明是否实际达成；静态检查只能证明上界，不能证明运行结果。

常用的契约等级包括：

| 等级 | 含义 |
|---|---|
| **Trajectory-equivalent** | 最终执行轨迹与对应的权威执行保持规定的逐步等价 |
| **Outcome-equivalent** | 端到端结果在规定范围内等价，但单次轨迹可能不同 |
| **Bounded-degradation** | 明确允许有损，但退化受到声明的边界约束 |
| **Best-effort** | 不提供严格的等价或退化保证 |

具体的 Fidelity 字段、Contract 校验和运行期校验见 [Speculation Contract 设计](speculation-contract.md)。

## 6. Policy 与 Mechanism 的边界

Policy 可以实现或组合七类 decision，但不能拥有 Runtime mutable state，也不能把策略意图伪造成已经完成的 Runtime 操作。以下内容必须由 Mechanism 或 Runtime 强制：

- candidate Branch 的创建、隔离、暂停、恢复、接受、丢弃和清理；
- `AssumptionSet` 的传播、确认/否定和非空假设数据的信息流约束；
- single-winner、共享验证依据、commit barrier 和效果阻塞；
- 资源所有权、预算执行、抢占、取消和运行期成本记账；
- Policy 声明与 Adapter/Runtime 能力之间的 Speculation Contract 校验，以及 Fidelity 的加载期检查和运行期校验。

Policy 决定“希望采用哪种策略”，Mechanism 保证该策略在正确性、隔离、资源、副作用和清理约束内执行。能力不足不能静默改变语义，只能拒绝、明确降级或选择其他合法路径。

## 7. 扩展规则

七类 decision 是当前完整的基础决策面。后续如需增加决策，必须说明：

- 它解决的独立策略问题；
- 所属阶段、输入和输出；
- 与现有七类 decision 的依赖或区别；
- 对 Edge、Proposal、Branch、未决 Proposal 依赖、single-winner、commit/discard/accept、obligation 和 cleanup 语义的影响。

如果新增内容只是某一 decision 的取值、实现方式或运行时优化，不应新增为独立 decision。`breadth`、嵌套 `depth` 和 Fidelity 已属于这种情况。
