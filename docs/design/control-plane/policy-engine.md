# Policy Engine 设计

本文展开 [control plane 设计](../control-plane.md) 中 Policy Engine 的目标语义。Policy Engine 只有两项职责：编译 Policy，以及根据规定输入作出本次 Policy Decision。具体编译产物和运行方式属于开发选择，不在本文固定。

## 1. 总体设计

Policy Engine 是 Policy 进入 AScout 的固定入口。Policy 作者使用 Policy DSL 编写 `PolicyDefinition`；Policy Engine 将其编译成可执行的 Policy Decision，并根据规定输入作出本次 Policy Decision、返回规定输出。

完整路径如下：

```text
PolicyDefinition
    -> Policy Engine 编译
    -> 可执行的 Policy Decision
        -> control plane 内由 Policy Engine 求值
        -> control plane 查询后交付给 Instrumentation，由 Instrumentation 求值
```

Policy Engine 不决定 Decision 在哪里运行，也不负责向 Instrumentation 交付编译结果。它不拥有 Speculative Tree、Branch 生命周期、效果状态或 Runtime 状态，也不把 Policy Decision 应用到这些对象。

## 2. 编译 Policy

长期设计允许修改 Policy，变更后的 Policy 同样须通过编译与契约校验；这不改变 Policy Engine 只负责编译和求值的职责。该能力不属于当前首版；同一 session 加载的 Policy 固定、不实现修改或切换的开发范围见 [Policy 开发说明](../../development/policy-engine.md)。后续支持修改时再确定切换规则，不从编译产物内容固定推导出长期禁止修改。

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

编译结果由九类 Decision 的决策逻辑组成。control plane 可以按 Decision 查询对应结果；查询只取得已经编译的逻辑，不重新解释或修改 `PolicyDefinition`。

编译结果必须可序列化和反序列化，使 control plane 能够跨进程或部署边界交付单个 Decision。它不能包含 control plane 内存对象、进程内函数引用或其他只在编译进程中有效的状态；相同结果在 control plane 和 Instrumentation 求值时必须具有相同语义。

正常运行须在运行目录原生保存实际加载的完整 PolicyDefinition、所引用且受支持的策略源码、编译结果及原来源。保存内容与执行内容必须来自同次加载，不能在加载后重新复制可能已经改变的外部文件。配置保存失败应明确拒绝启动，已有运行资料不得覆盖。恢复优先使用运行目录中的 Policy；缺失时允许显式提供并核对原绑定，不能改用当前默认 Policy。Policy 对 Runtime 配置的引用只表示配置来源关系，不代表已保存完整 Runtime。

具体编译产物属于实现选择，当前选择见 [开发决策](../../development/decisions.md)。

## 3. 作出本次 Policy Decision

第二项职责是根据调用方提供的规定输入对已编译的 Policy Decision 求值，作出本次 Policy Decision 并返回规定输出。它只作出 Policy Decision，不把决定应用到任何对象上。

### 3.1 作出决定

在 control plane 内，持有状态的组件形成 Decision 输入，Policy Engine 对对应的编译结果求值：

```text
control plane 组件提供 Decision 输入
    -> Policy Engine 对已编译 Decision 求值
    -> Policy Engine 返回本次 Policy Decision
```

求值按照 Policy 定义的算法执行，不附加另一套策略，也不自行读取调用方拥有的状态。

系统按每类 Decision 的输入约定提供允许读取的数据值或快照，而不是 control plane 内部对象、可变系统引用或任意查询入口。算法可以修改自己的输入副本和中间结果，但不能因此修改系统真实状态；如需模型等外部能力，也只能在明确授权范围内使用。数据可见范围和执行权限必须由系统实际落实，不能仅依赖函数参数、声明格式或静态检查。

外部模型能力通过 `capabilities.model_complete(request)` 请求；宿主依据自己持有的当前求值和 Decision、显式 grant、模型 allowlist、额度与 deadline 逐次授权。Policy 提供的 Decision 或授权声明不构成权限依据。capability 消息与求值结果须明确区分并绑定宿主生成的当前请求 ID；旧请求、跨请求及未授权调用拒绝，求值结束、取消或超时后授权撤销，取消须传播到宿主 provider。凭证只由宿主持有。实现进度、每次求值的状态隔离、进程 provider 入口及产品配置缺口见 [开发说明](../../development/policy-engine.md)。

求值结果须按该 Decision 的输出约定进行检查。相关状态的拥有组件仍须根据当前事实复核并落实决定，不能把算法返回的选择当成已经发生的状态变化。

隔离执行进程可以复用，但必须有明确所有者和有界数量；不同 Policy 不能因解释器复用而共享可变模块状态。每次求值的输入、输出和运行时间必须有界，超时或取消后须终止并回收进程，再接受后续求值。重新创建函数命名空间只能减少意外状态残留，不能充当 Python 安全边界。只读标准库依赖属于部署环境，不意味着允许读取宿主工作区或持有宿主凭证。

### 3.2 决定的落实

Policy Engine 返回的只是本次 Policy Decision，不是已经完成的状态变化。把 Policy Decision 应用到 Branch、Speculative Tree、外部效果或 Runtime 不属于 Policy Engine，而由相应状态的拥有组件完成。

例如，Policy Engine 为 `select` 作出决定后只返回 Policy 选择的 Chance；这些 Chance 是否仍然有效、能否启动以及启动后的状态变化，均由 Speculation Coordinator 处理。

## 4. 跨侧运行与组件边界

Policy Engine 编译全部 Decision。会改写全局控制状态或必须在多个 Chance、Branch、Proposal 之间协调的 Decision（包括 `predict`），由 control plane 调用 Policy Engine 求值并落实，所缺的本地事实由 Instrumentation 计算后提交；control plane 仅将 Instrumentation 实际求值的 `admit.opportunity_pass`、`admit.feasibility_pass`、`extent` 和 `effect` 编译结果经 Gateway 交付，Instrumentation 按相同语义求值。Instrumentation 不能自行加载、编译或修改 Policy。

各组件围绕这两项职责与 Policy Engine 协作：

- **Speculation Coordinator** 为相关 Decision 提供输入，并落实本次 Policy Decision。
- **Runtime** 通过现有 hello 报告 backend 身份。Policy Engine 在 control plane 启动握手后只确认 Policy 要求的 backend 存在；没有要求则不检查，缺少则拒绝启动。接口存在性及可解析性由 Runtime 自行检查，不属于 Policy 编译，也不代表环境保证已验证。
- **Instrumentation** 本地求值并落实 `effect`，将模拟操作交给 External Effect Coordinator，后者判断能否模拟并明确反馈。
- **Gateway** 交付编译结果、端侧 Decision 结果和执行记录；Policy Engine 不拥有通信会话或传输状态。
- **Instrumentation** 对 control plane 交付的 Decision 以相同语义求值，并返回结果和执行记录。

## 相关文档

- [control plane 设计](../control-plane.md)：control plane 组件与状态边界
- [Policy Decisions](../policy-decisions.md)：九类 Decision 的目标语义
- [Speculation Contract](../speculation-contract.md)：Policy 与 Runtime Adapter 能力的语义对齐
- [Instrumentation 设计](../agent-integration.md)：本地 Decision 的求值与记录
- [开发决策](../../development/decisions.md)：当前编译产物与实现限制

## Policy 与 session 的目录

Policy Engine 管理固定根目录中的多份 Policy，每份是含 `policy.yaml` 与 `decisions.py` 的独立子目录。配置文件为 `~/.config/ascout/config.toml`，支持 `policy_dir`、`runs_dir`、`default_policy` 与 `log_path` 四个键，缺省分别为 `~/.local/share/ascout/policies`、`~/.local/share/ascout/runs`、`policy` 与 `~/.local/share/ascout/ascout.log`；`log_path` 只配置系统运行日志的路径，格式及进程归属固定，启动时打不开即拒绝启动，不轮转。

交互终端启动前列出 Policy 并询问，允许改选；空输入使用当前默认选择。非交互启动使用显式 Policy 名字，否则使用默认。一次 session 内 Policy 固定。系统在运行根目录下创建 session 子目录，以 session id 标识运行，结束时返回该 id；会话列表列出已有 session id，恢复按 id 找到目录，不以路径作为 execution id。
