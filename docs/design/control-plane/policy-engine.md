# Policy Engine 设计

本文展开 [控制面设计](../control-plane.md) 中 Policy Engine 的目标语义。Policy Engine 只有两项职责：编译 Policy，以及根据规定输入作出本次 Policy Decision。具体编译产物和运行方式属于开发选择，不在本文固定。

## 1. 总体设计

Policy Engine 是 Policy 进入 AScout 的固定入口。Policy 作者使用 Policy DSL 编写 `PolicyDefinition`；Policy Engine 将其编译成可执行的 Policy Decision，并根据规定输入作出本次 Policy Decision、返回规定输出。

完整路径如下：

```text
PolicyDefinition
    -> Policy Engine 编译
    -> 可执行的 Policy Decision
        -> 控制面内由 Policy Engine 求值
        -> 控制面查询后交付给 Agent 接入侧，由接入侧求值
```

Policy Engine 不决定 Decision 在哪里运行，也不负责向 Agent 接入侧交付编译结果。它不拥有 Speculative Tree、Branch 生命周期、效果状态或 Runtime 状态，也不把 Policy Decision 应用到这些对象。

## 2. 编译 Policy

编译职责是把 `PolicyDefinition` 转换为 AScout 可以直接运行的 Policy Decision。它以 Policy DSL 为输入，以经过检查的可执行逻辑为输出。

### 2.1 输入

Policy DSL 是 `PolicyDefinition` 的唯一入口。它规定九类 Policy Decision 各自的输入与输出、可读取事实、可使用能力和组合约束。每类 Decision 都按规定的输入作出规定的输出，快捷写法只是其简化表达。

`PolicyDefinition` 由声明部分和其引用的独立算法源码共同组成。声明部分组织各 Decision 的行为、配置和源码引用；算法在系统规定的输入范围内计算并返回对应结果。算法可复用成熟语言，不要求重新设计通用计算语法，也不局限于选择内置算法。被引用源码属于同一 PolicyDefinition，不构成绕过 Policy DSL 的独立加载入口。

九类 Decision 的具体语义由 [Policy Decisions](../policy-decisions.md) 定义，不由 Policy Engine 另行解释。

### 2.2 编译过程

Policy Engine 解析 Policy DSL，展开快捷写法，并检查生成的九类 Decision 是否符合 Policy DSL、Speculation Contract 和当前可用能力的约束。不能形成合法执行逻辑的 `PolicyDefinition` 不得进入运行期；无法满足的能力要求必须被拒绝或按 Policy 已声明的方式明确降级。

源码引用在编译时解析并固定所用代码，受支持的依赖也须明确。编译结果必须包含执行相关 Decision 所需的逻辑，不能只保留作者机器上的源码路径并在求值时重新读取。引用解析和接口检查属于编译过程，不代替动态输入、输出和当前执行条件的运行期检查，也不能证明任意算法的正确性或 Fidelity。

这些解析、展开和检查都是编译过程的组成部分，不构成 Policy Engine 的额外职责。运行期对已经编译的 Decision 求值时，不再重新解析或静态检查 Policy DSL。

### 2.3 编译结果

编译结果由九类 Decision 的决策逻辑组成。控制面可以按 Decision 查询对应结果；查询只取得已经编译的逻辑，不重新解释或修改 `PolicyDefinition`。

编译结果必须可序列化和反序列化，使控制面能够跨进程或部署边界交付单个 Decision。它不能包含控制面内存对象、进程内函数引用或其他只在编译进程中有效的状态；相同结果在控制面和 Agent 接入侧求值时必须具有相同语义。

具体编译产物属于实现选择，当前选择见 [开发决策](../../development/decisions.md)。

## 3. 作出本次 Policy Decision

第二项职责是根据调用方提供的规定输入对已编译的 Policy Decision 求值，作出本次 Policy Decision 并返回规定输出。它只作出 Policy Decision，不把决定应用到任何对象上。

### 3.1 作出决定

在控制面内，持有状态的组件形成 Decision 输入，Policy Engine 对对应的编译结果求值：

```text
控制面组件提供 Decision 输入
    -> Policy Engine 对已编译 Decision 求值
    -> Policy Engine 返回本次 Policy Decision
```

求值按照 Policy 定义的算法执行，不附加另一套策略，也不自行读取调用方拥有的状态。

系统按每类 Decision 的输入约定提供允许读取的数据值或快照，而不是控制面内部对象、可变系统引用或任意查询入口。算法可以修改自己的输入副本和中间结果，但不能因此修改系统真实状态；如需模型等外部能力，也只能在明确授权范围内使用。数据可见范围和执行权限必须由系统实际落实，不能仅依赖函数参数、声明格式或静态检查。

求值结果须按该 Decision 的输出约定进行检查。相关状态的拥有组件仍须根据当前事实复核并落实决定，不能把算法返回的选择当成已经发生的状态变化。

### 3.2 决定的落实

Policy Engine 返回的只是本次 Policy Decision，不是已经完成的状态变化。把 Policy Decision 应用到 Branch、Speculative Tree、外部效果或 Runtime 不属于 Policy Engine，而由相应状态的拥有组件完成。

例如，Policy Engine 为 `select` 作出决定后只返回 Policy 选择的 Edge；这些 Edge 是否仍然有效、能否启动以及启动后的状态变化，均由 Speculation Coordinator 处理。

## 4. 跨侧运行与组件边界

Policy Engine 编译全部 Decision。控制面需要作出决定时，直接调用 Policy Engine；Agent 接入侧需要作出决定时，控制面查询对应的编译结果并经 Agent Control Gateway 交付，接入侧按相同语义求值。接入侧不能自行加载、编译或修改 Policy。

各组件围绕这两项职责与 Policy Engine 协作：

- **Speculation Coordinator** 为相关 Decision 提供输入，并落实本次 Policy Decision。
- **Adapter Registry** 提供 Runtime Adapter 的能力信息和验证状态，作为 Policy 编译时的约束依据。
- **Effect Coordinator** 提供 `effect` Decision 所需的事实，并落实其返回的效果处理选择。
- **Agent Control Gateway** 交付编译结果、端侧 Decision 结果和执行记录；Policy Engine 不拥有通信会话或传输状态。
- **Agent 接入侧** 对控制面交付的 Decision 以相同语义求值，并返回结果和执行记录。

## 相关文档

- [控制面设计](../control-plane.md)：控制面组件与状态边界
- [Policy Decisions](../policy-decisions.md)：九类 Decision 的目标语义
- [Speculation Contract](../speculation-contract.md)：Policy 与 Runtime Adapter 能力的语义对齐
- [Agent 接入侧设计](../agent-integration.md)：本地 Decision 的求值与记录
- [开发决策](../../development/decisions.md)：当前编译产物与实现限制
