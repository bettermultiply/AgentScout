# Runtime Adapter 契约设计

本文定义 Runtime Adapter 面向 AScout 控制面的正式接入契约，说明 Adapter 提供的环境语义、操作身份与顺序、能力声明、Registry 校验以及环境操作的结果和事件。

## 1. 契约边界

Runtime Adapter 连接控制面与外部 Runtime。控制面向 Adapter 发出环境操作，Adapter 将其映射到目标 Runtime，并把环境结果和事件规范化返回。契约的核心是 Runtime 提供的环境能力：环境建立、环境内操作、静止、恢复、终止和清理，以及这些操作对不同环境内容形成的隔离保证；环境派生是可选能力，不是所有 Runtime 的前提。

Adapter 可以使用 Runtime 原生隔离、权限控制、代理、拦截或拒绝来兑现声明。Registry 检查的是 Adapter 在边界上呈现的最终环境保证，不检查某一种内部实现。

## 2. 环境操作

### 2.1 固定生命周期操作

所有合格 Adapter 都必须支持以下固定操作语义，不把它们作为可选能力声明：

- **`establish_environment`（建立环境）**：从控制面指定的环境基础建立可用的隔离环境；
- **`operate_environment`（环境内操作）**：在环境内建立 Agent 接入侧，并执行接入侧无权完成的环境级操作；用哪个 Agent 产品与版本由控制面决定并作为启动参数给出，实际启动、监视和回收由 Runtime 负责；
- **`promote_environment`（提升环境）**：把 Candidate 环境提升为权威环境。共享底层环境的 Runtime 可以将其实现为 no-op，但必须接受该操作；控制面在 Agent 切换权威前调用，失败则不得切换；
- **`suspend_environment`（静止环境）**：阻止新的环境操作，并让正在进行的环境操作完成或返回明确的未完成状态；
- **`resume_environment`（恢复环境）**：让仍然有效的静止环境继续接受操作，且已有状态和隔离边界保持不变；
- **`terminate_environment`（终止环境）**：不可恢复地停止环境内由 Runtime 管理的执行；
- **`cleanup_environment`（清理环境）**：清理 Runtime 管理的进程、资源、临时状态、句柄和租约。

环境的基本状态闭环是：建立完成后可用；可用环境可以进入静止状态并再次恢复；可用或静止环境可以终止；终止后完成清理。操作失败或部分完成时，Adapter 必须返回环境实际停留的状态，控制面不能根据预期状态继续推进。

### 2.2 环境派生操作

`branch_environment` 表示从父环境当前状态派生一个独立的子环境；父环境保持运行，子环境与父环境、与其他 sibling 互不影响。它是可选能力，不是所有 Runtime 的前提。

Adapter 声明支持 `branch_environment` 时，必须同时声明派生覆盖的环境内容范围；派生范围不要求覆盖全部内容。不能派生独立环境的后端必须返回 `Unsupported`，不得用共享别名伪装成功。分支不改变父环境的状态，派生出的环境从可用状态开始，与父环境独立演化。

### 2.3 非干扰与共享环境

Branch 的固定要求是**不影响权威路径和其他 Branch**。独立环境只是实现该要求的一种方式；Adapter 也可以让多个 Branch 共享同一个环境，但必须保证一个 Branch 的操作在影响环境之前被阻止、暂存或以其他方式受控。

Adapter 必须按环境内容声明其非干扰机制、覆盖的入口、scope 和 limitations。声明可以描述独立环境、作用前阻止、写入暂存、写时复制或其他机制；机制名称不是固定语义，控制面只依赖声明的保证范围和验证结果。

共享环境下的非干扰必须覆盖所有可能影响权威路径或其他 Branch 的入口。无法可靠识别或阻止的入口不得纳入已验证范围；控制面不得把未覆盖的内容当作已隔离，必须改用独立环境或拒绝相应投机。

隔离贯穿以上所有操作，是接入资格的固定语义。这里要求的是非干扰保证，而不是某种特定的环境形态。

### 2.4 扩展环境操作

固定生命周期操作和环境派生操作之外的环境操作属于扩展操作。扩展操作只有在 AScout 定义统一语义、结果和验证要求后，才能进入契约。例如，环境迁移可以定义为 `migrate_environment`。在该语义形成前，Adapter 可以在内部使用迁移机制，但控制面不直接调用它。

扩展操作必须说明适用条件、对环境状态的影响、失败收敛方式和验证方法。Adapter 不能通过能力声明自行增加控制面可调用的私有操作。

### 2.5 环境工具操作

Runtime 可以在能力声明中登记环境工具及其参数结构。环境工具是需要由 Runtime 在真实环境中执行、而不是由 Agent 进程自行完成的工具。登记项至少包含工具名、说明和参数 schema；控制面不得识别未登记的工具。

环境工具使用两个统一扩展操作：

- `tool_context`：读取工具当前可用的公开上下文，供 Policy 预测；不得读取隐藏状态；
- `invoke_tool`：在指定环境执行一次工具调用。结果包含 `result` 和 `evidence`：`result` 进入 Agent 工具通道，`evidence` 交给 Policy 匹配。两者都只能包含 Runtime 愿意公开的事实。

环境工具不创建 Branch。多个 Branch 共享同一环境时，Candidate 的工具调用必须在影响环境前被暂存或阻止；只有 Promote 后的权威 Branch 可以调用 `invoke_tool`。控制面不能因为工具名而推断 workload 语义或环境形态。

## 3. 操作身份与顺序

控制面与 Adapter 之间的每次交互都围绕两个由控制面分配的逻辑标识：环境标识标识一个环境，请求标识标识一次操作。Adapter 内部把逻辑标识映射到后端句柄，接口中不出现这些句柄。

### 3.1 身份与关联

控制面在请求建立环境时分配环境标识，之后一直用该标识引用这个环境。控制面为每次操作分配请求标识；结果和与某次操作相关的事件都必须携带对应的环境标识和请求标识。

重发同一请求标识表示重试同一次操作。Adapter 必须识别重试，返回同一逻辑结果，不得因此制造第二个逻辑环境，也不得掩盖第一次操作留下的部分状态。

### 3.2 顺序与中断

同一环境同一时刻只执行一个操作；控制面可以连续发送请求，Adapter 按到达顺序排队执行。`suspend_environment` 和 `terminate_environment` 不受队列限制：它们可以打断正在进行的操作，被打断的操作按失败或部分完成返回，并报告环境实际停留的状态和未完成责任。

固定生命周期操作集合不包含单独的取消操作。需要中途停止某个环境的工作时使用 `terminate_environment`；需要让环境停止接受新操作并保留既有状态时使用 `suspend_environment`。

### 3.3 超时

等待操作结果的超时由控制面掌握。超时后，控制面可以用同一请求标识重发，由重试语义保证安全；也可以转入终止或清理流程。Adapter 不自行判断调用方超时，只需保证重试安全并如实报告环境状态。

## 4. 隔离声明

Adapter 声明当前 Runtime 实例在环境操作下，对不同环境内容提供的隔离保证。

本节声明的是环境之间的隔离；共享环境中 Branch 与权威路径、其他 Branch 之间的非干扰由 §2.3 声明，二者不互相替代。

声明覆盖以下内容。每类内容可以整体标记为 `not_applicable`，表示当前实例不承担这类保证；这不表示该类内容已经通过验证，也不允许控制面把它当作可用隔离能力。

| 内容 | 关注对象 |
|---|---|
| `process` | 环境内进程、进程关系和环境级执行 |
| `workspace` | 环境可见的文件、目录和临时工作区 |
| `memory` | 环境内存、共享内存和可访问的执行状态 |
| `I/O` | 文件描述符、标准输入输出、本地 IPC 等通道 |
| `network` | 网络命名空间、端口、socket 和环境间网络访问 |
| `resource` | CPU、内存额度、进程数、文件描述符和临时存储等环境资源 |
| `environment-local control surface` | 环境内接入侧使用的本地控制入口及其环境级权限 |

每项声明使用同一组隔离属性，属性本身可随契约版本扩展：

- `visibility`、`modification`、`interference`：其他环境能否观察、直接修改，或通过资源竞争与间接路径影响该内容；
- `lifetime`：静止与恢复是否保持该内容；
- `cleanup`：环境清理后该内容是否仍有残留；
- `scope`：保证覆盖的对象、路径、通道或配置范围；
- `limitations`：已知例外、依赖条件和未覆盖路径。

`visibility`、`modification` 和 `interference` 取值为 `none`、`scoped` 或 `full`：`none` 表示其他环境完全不能，`scoped` 表示只在 `scope` 与 `limitations` 限定的范围内可以，`full` 表示可以。`lifetime` 取值为 `preserved` 或 `reset`；`cleanup` 取值为 `none` 或 `residual`。

声明必须为每个属性给出取值，不设置“未知”取值；无法担保时，最弱的诚实声明分别是 `full` 和 `residual`。`scope` 和 `limitations` 使用结构化条目（类别加值），类别来自按内容类型维护的受控词表，随契约版本扩展；自由文本只能作为补充说明，不参与比较。Registry 必须能够解析这些条目，并据此完成范围包含、相交和冲突检查。

## 5. 能力声明与解析

Adapter 声明描述当前 Adapter 实例和当前 Runtime 配置下实际声称的保证，不描述理论最大能力。未启用的功能和依赖其他部署配置的功能不得混入当前声明。当前实例声称但 Registry 无法充分证明的保证仍可保留在声明中，由 Registry 标记为未验证。

### 5.1 AdapterDeclaration

Adapter 提交一个 `AdapterDeclaration`。它描述一个确定的 Adapter 实例在一个确定的 Runtime 配置下提供的环境保证：

```text
AdapterDeclaration {
    contract_version
    adapter_instance
    runtime_configuration
    environment_contents: [ContentIsolation, ...]
    non_interference: [NonInterferenceDeclaration, ...]
    environment_tools: [RuntimeTool, ...]
    deployment_conditions
}

`environment_tools` 的每项至少包含 `name`、`description` 和 `parameters`；未登记的工具不得由控制面调用。环境工具不改变环境之间的隔离语义，也不表示 `branch_environment` 能力。
```

`environment_contents` 为每类环境内容提供一项隔离声明。当前实例不承担某类内容时，仍需提交 `not_applicable` 声明；同一类环境内容不得出现相互竞争的多项有效声明。

`non_interference` 按环境内容描述 Branch 之间以及 Branch 与权威路径之间的非干扰保证。每项至少包含 `content`、`mechanism`、`scope` 和 `limitations`；`mechanism` 是描述性的实现说明，不构成固定枚举。共享环境、独立环境或其他机制都在这里声明，Registry 使用与 `environment_contents` 相同的验证流程检查其是否兑现。

`deployment_conditions` 描述声明成立所依赖的部署条件，例如宿主内核、CPU 特征、网络配置和所需权限。这些条件变化时，原声明不再成立。

### 5.2 ContentIsolation

每项 `ContentIsolation` 描述一类环境内容在环境操作下的隔离保证：

```text
ContentIsolation {
    content
    visibility
    modification
    interference
    lifetime
    cleanup
    scope
    limitations
}
```

`visibility`、`modification` 和 `interference` 分别描述观察、直接变更和间接影响；`lifetime` 和 `cleanup` 描述环境生命周期对该内容的处理；`scope` 和 `limitations` 限定保证适用的对象、路径、通道和条件。

### 5.3 CapabilityView

Registry 将声明解析为规范化的 `CapabilityView`，并附加验证结果：

```text
CapabilityView {
    contract_version
    adapter_instance
    runtime_configuration
    environment_contents: [VerifiedContentIsolation, ...]
    non_interference: [VerifiedNonInterference, ...]
    deployment_conditions
}

VerifiedContentIsolation {
    declaration: ContentIsolation
    validation_status
    validation_evidence
}

VerifiedNonInterference {
    declaration: NonInterferenceDeclaration
    validation_status
    validation_evidence
}
```

`CapabilityView` 是控制面使用的当前能力视图。验证状态由 Registry 根据解析和主动验证结果生成，Adapter 不能自行填写验证状态。`validation_evidence` 由 Registry 生成，必须说明验证环境、使用的场景和观察到的结果，使验证结论可以被复核；Adapter 不能自行填写验证证据。

### 5.4 Registry 处理流程

Registry 按以下顺序处理 Adapter 声明：

```text
AdapterDeclaration
    -> 结构解析
    -> 规范化与一致性检查
    -> Registry 建立验证环境
    -> Registry 执行验证场景并观察结果
    -> 对照声明生成验证状态
    -> CapabilityView
```

结构解析确认契约版本、内容类型、隔离属性和部署条件可以被识别。规范化与一致性检查确认声明覆盖完整、同类内容没有冲突、范围和限制能够被解释。Registry 自主建立验证环境，使用环境操作执行验证场景，并根据实际观察结果检查声明中的保证。Adapter 提供环境建立和观察验证所需的接入信息，但不提交自测结果作为验证依据。

Registry 将最终 `CapabilityView` 提供给 Policy Engine。Policy Engine 依据它检查 Policy 的要求是否存在合法执行路径，并决定接受、拒绝或降级 Policy。配置 identity 是能力视图的一部分；配置变化必须产生新的声明并重新经过 Registry 处理。

原始声明中 Registry 无法充分证明的保证进入 `unverified` 范围，主动验证证明不成立的保证进入 `verification_failed` 范围。

### 5.5 比较规则

隔离保证按强弱排序，供 Registry 规范化、能力降级和 Policy 匹配使用：

- `visibility`、`modification`、`interference`：`none` 强于 `scoped`，`scoped` 强于 `full`；同为 `scoped` 时，`scope` 越窄越强；
- `lifetime`：`preserved` 强于 `reset`；
- `cleanup`：`none` 强于 `residual`。

能力降级只能沿更强到更弱的方向收缩，不能声称比已经验证并仍然成立的范围更强的保证。Policy 匹配按“后端能力至少等于 Policy 要求”的方向进行；Policy 如何表达要求由 Policy DSL 定义。验证状态对可用性的限制见「验证状态」一节。

`non_interference` 不按机制名称排序。独立环境、作用前阻止、写入暂存或其他机制之间没有天然的强弱关系；比较按声明覆盖的内容、入口、scope 和 limitations 的包含关系进行。机制不同但覆盖范围相同，不自动视为一方更强。

## 6. 验证状态

每个声明项及其可验证范围都有 Registry 产生的状态：

- **已验证**：主动验证证明当前实例在声明范围内兑现保证；
- **未验证**：无法完成充分证明，但没有证据表明保证失败；默认不能按已验证能力使用，部署方主动确认后才可限制使用；
- **验证失败**：主动验证证明声明不成立，不能通过用户确认绕过。

用户确认承担未验证能力的部署风险，不会把它变成已验证能力。能力视图必须保留状态、范围和限制，不能把未验证或验证失败的范围混入普通可用范围。

接入时由 Registry 主动验证是主要证明手段，运行期验证用于发现接入验证遗漏和实际偏差。运行期验证不能替代接入验证。

## 7. 结果、事件与失败

结果和事件是控制面了解环境实际状态的两种信息来源：结果回答一次请求，事件报告没有被操作直接覆盖的变化。两者都携带第 3 节规定的关联标识，并遵守同一组失败语义：任何一方都不能把未完成或部分完成的状态报告成成功。

### 7.1 操作结果

每个操作结果都必须能关联到对应的环境操作，并至少包含：

- 状态：请求已接受、操作进行中、实际完成、失败、部分完成、不适用或不支持；
- 环境实际停留的状态；
- 已处理的环境资源和仍未完成的责任，按环境内容归类；
- 是否仍需静止、恢复、终止或清理；
- 当前环境是否还能接受后续操作；
- 失败或部分完成的原因。

不能用一个 `success` 覆盖这些状态。字段名和枚举细节属于实现选择，但语义以上述内容为准。

### 7.2 事件

事件是 Adapter 主动上报的异步通知，使用与结果相同的关联标识。事件类型可以扩展，契约规定的最小集合包括：

- 异步操作完成：操作在请求返回之后才完成时，以事件送回与该操作结果相同的信息；
- 环境状态变化：不是由某个已完成操作直接报告的变化，例如环境意外停止或失败。

由已完成操作直接报告的状态变化不重复发事件。事件是通知，不是权威状态的唯一来源；控制面需要确认时通过后续操作取得环境的实际状态。契约不要求事件确认和重传，也不定义连接断开后的事件补发。偏差与运行期验证类事件由 Registry 验证流程定义。

### 7.3 失败收敛

以下情况不能被报告为成功：环境尚未建立完成、静止或终止请求尚未收敛、cleanup 只删除了 metadata、恢复请求尚未让环境重新可用，或只确认了请求已经发送。环境层结果也不能冒充 Branch accept、discard、commit 或 winner 转换结果。

如果隔离依赖拦截，拦截必须发生在受保护操作产生影响之前，并覆盖声明范围内的入口。无法可靠识别或拦截的路径不能被纳入已验证保证；拦截失败时必须拒绝、阻断或返回明确的部分完成/失败结果。

## 8. 配置变化与能力降级

Adapter 配置在运行期不允许变化。若 Registry 或运行期证据发现当前 Runtime 行为与 Adapter 声明不符，不能继续沿用旧能力视图。

控制面将 Adapter 降级到已经验证且仍然成立的范围，并让全部任务使用这个降级后的能力范围。依赖失效能力的投机活动停止并清理；不依赖失效能力的活动可以继续。Policy Engine 和后续新任务只能使用降级后的能力视图。

重新验证通过后，Adapter 才能恢复使用新的能力范围。重新验证不会自动恢复已经停止的投机活动，也不会把历史运行期偏差当作已不存在。

## 相关文档

- [Speculation Contract](speculation-contract.md)：Policy 与 Adapter 之间的总体语义关系
- [控制面设计](control-plane.md)：Adapter Registry 与控制面其他组件的协作
- [Runtime Adapter 接入指南](../development/runtime-adapter.md)：Adapter 作者的实现与接入方法
