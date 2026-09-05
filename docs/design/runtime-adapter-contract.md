# Runtime Adapter 契约设计

本文定义 Runtime Adapter 面向 AScout 控制面的正式接入契约，说明 Adapter 提供的环境语义、能力声明、Registry 校验以及环境操作的结果和事件。

## 1. 契约边界

Runtime Adapter 连接控制面与外部 Runtime。控制面向 Adapter 发出固定的环境操作，Adapter 将其映射到目标 Runtime，并把环境结果和事件规范化返回。契约的核心是 Runtime 提供的环境能力：环境建立、环境内操作、静止、恢复、终止和清理，以及这些操作对不同环境内容形成的隔离保证。

Adapter 可以使用 Runtime 原生隔离、权限控制、代理、拦截或拒绝来兑现声明。Registry 检查的是 Adapter 在边界上呈现的最终环境保证，不检查某一种内部实现。

## 2. 固定环境操作

所有合格 Adapter 都必须支持以下固定操作语义，不把它们作为可选能力声明：

- **`establish_environment`（建立环境）**：从控制面指定的环境基础建立可用的隔离环境；
- **`operate_environment`（环境内操作）**：在环境内建立 Agent 接入侧，并执行接入侧无权完成的环境级操作；
- **`suspend_environment`（静止环境）**：阻止新的环境操作，并让正在进行的环境操作完成或返回明确的未完成状态；
- **`resume_environment`（恢复环境）**：让仍然有效的静止环境继续接受操作，且已有状态和隔离边界保持不变；
- **`terminate_environment`（终止环境）**：不可恢复地停止环境内由 Runtime 管理的执行；
- **`cleanup_environment`（清理环境）**：清理 Runtime 管理的进程、资源、临时状态、句柄和租约。

环境的基本状态闭环是：建立完成后可用；可用环境可以进入静止状态并再次恢复；可用或静止环境可以终止；终止后完成清理。操作失败或部分完成时，Adapter 必须返回环境实际停留的状态，控制面不能根据预期状态继续推进。

隔离贯穿以上所有操作，是接入资格的固定语义。

### 2.1 扩展环境操作

固定操作之外的环境操作属于扩展操作。扩展操作只有在 AScout 定义统一语义、结果和验证要求后，才能进入契约。例如，环境迁移可以定义为 `migrate_environment`。在该语义形成前，Adapter 可以在内部使用迁移机制，但控制面不直接调用它。

扩展操作必须说明适用条件、对环境状态的影响、失败收敛方式和验证方法。Adapter 不能通过能力声明自行增加控制面可调用的私有操作。

## 3. 隔离声明

Adapter 声明当前 Runtime 实例在固定环境操作下，对不同环境内容提供的隔离保证。

声明覆盖以下内容。某一项可以标记为 `not_applicable`，表示当前实例不承担这类保证；这不表示该项已经通过验证，也不允许控制面把它当作可用隔离能力。

| 内容 | 关注对象 |
|---|---|
| `process` | 环境内进程、进程关系和环境级执行 |
| `workspace` | 环境可见的文件、目录和临时工作区 |
| `memory` | 环境内存、共享内存和可访问的执行状态 |
| `I/O` | 文件描述符、标准输入输出、本地 IPC 等通道 |
| `network` | 网络命名空间、端口、socket 和环境间网络访问 |
| `resource` | CPU、内存额度、进程数、文件描述符和临时存储等环境资源 |
| `environment-local control surface` | 环境内接入侧使用的本地控制入口及其环境级权限 |

每项声明使用同一组可扩展的隔离属性：

- `visibility`：其他环境能否观察该内容；
- `modification`：其他环境能否直接修改该内容；
- `interference`：其他环境能否通过资源竞争、间接依赖或其他路径影响该内容；
- `lifetime`：静止、恢复、终止和环境清理对该内容的影响；
- `cleanup`：环境清理后该内容是否仍有残留；
- `scope`：保证覆盖的对象、路径、通道或配置范围；
- `limitations`：已知例外、依赖条件和未覆盖路径。

`visibility`、`modification` 和 `interference` 分别描述观察、直接变更和间接影响三个隔离属性。`scope` 和 `limitations` 用于限定保证边界，必须能够被 Registry 解析和比较。

## 4. 能力声明与解析

Adapter 声明描述当前 Adapter 实例和当前 Runtime 配置下实际声称的保证，不描述理论最大能力。未启用的功能和依赖其他部署配置的功能不得混入当前声明。当前实例声称但 Registry 无法充分证明的保证仍可保留在声明中，由 Registry 标记为未验证。

### 4.1 AdapterDeclaration

Adapter 提交一个 `AdapterDeclaration`。它描述一个确定的 Adapter 实例在一个确定的 Runtime 配置下提供的环境保证：

```text
AdapterDeclaration {
    contract_version
    adapter_instance
    runtime_configuration
    environment_contents: [ContentIsolation, ...]
    deployment_conditions
}
```

`environment_contents` 为每类环境内容提供一项隔离声明。当前实例不承担某类内容时，仍需提交 `not_applicable` 声明；同一类环境内容不得出现相互竞争的多项有效声明。

### 4.2 ContentIsolation

每项 `ContentIsolation` 描述一类环境内容在固定环境操作下的隔离保证：

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

### 4.3 CapabilityView

Registry 将声明解析为规范化的 `CapabilityView`，并附加验证结果：

```text
CapabilityView {
    contract_version
    adapter_instance
    runtime_configuration
    environment_contents: [VerifiedContentIsolation, ...]
    deployment_conditions
}

VerifiedContentIsolation {
    declaration: ContentIsolation
    validation_status
    validation_evidence
}
```

`CapabilityView` 是控制面使用的当前能力视图。验证状态由 Registry 根据解析和主动验证结果生成，Adapter 不能自行填写验证状态。

### 4.4 Registry 处理流程

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

结构解析确认契约版本、内容类型、隔离属性和部署条件可以被识别。规范化与一致性检查确认声明覆盖完整、同类内容没有冲突、范围和限制能够被解释。Registry 自主建立验证环境，使用固定环境操作执行验证场景，并根据实际观察结果检查声明中的保证。Adapter 提供环境建立和观察验证所需的接入信息，但不提交自测结果作为验证依据。

Registry 将最终 `CapabilityView` 提供给 Policy Engine。Policy Engine 依据它检查 Policy 的要求是否存在合法执行路径，并决定接受、拒绝或降级 Policy。配置 identity 是能力视图的一部分；配置变化必须产生新的声明并重新经过 Registry 处理。

原始声明中 Registry 无法充分证明的保证进入 `unverified` 范围，主动验证证明不成立的保证进入 `verification_failed` 范围。

## 5. 验证状态

每个声明项及其可验证范围都有 Registry 产生的状态：

- **已验证**：主动验证证明当前实例在声明范围内兑现保证；
- **未验证**：无法完成充分证明，但没有证据表明保证失败；默认不能按已验证能力使用，部署方主动确认后才可限制使用；
- **验证失败**：主动验证证明声明不成立，不能通过用户确认绕过。

用户确认承担未验证能力的部署风险，不会把它变成已验证能力。能力视图必须保留状态、范围和限制，不能把未验证或验证失败的范围混入普通可用范围。

接入时由 Registry 主动验证是主要证明手段，运行期验证用于发现接入验证遗漏和实际偏差。运行期验证不能替代接入验证。

## 6. 结果、事件与失败

Adapter 返回的环境结果至少要能区分：请求已接受、操作进行中、实际完成、失败、部分完成、不适用和不支持。具体字段尚未定稿，但不能用一个 `success` 覆盖这些状态。重复或重试同一个环境操作时，结果不能制造第二个逻辑环境，也不能掩盖第一次操作留下的部分状态。

每个结果或事件都必须能关联到对应的环境操作，并说明：

- 环境操作是否真正完成；
- 已处理哪些环境资源；
- 哪些责任仍未完成；
- 是否仍需静止、恢复、终止或清理；
- 当前环境是否还能接受后续操作。

以下情况不能被报告为成功：环境尚未建立完成、静止或终止请求尚未收敛、cleanup 只删除了 metadata、恢复请求尚未让环境重新可用，或只确认了请求已经发送。环境层结果也不能冒充 Branch accept、discard、commit 或 winner 转换结果。

如果隔离依赖拦截，拦截必须发生在受保护操作产生影响之前，并覆盖声明范围内的入口。无法可靠识别或拦截的路径不能被纳入已验证保证；拦截失败时必须拒绝、阻断或返回明确的部分完成/失败结果。

## 7. 配置变化与能力降级

Adapter 配置在运行期不允许变化。若 Registry 或运行期证据发现当前 Runtime 行为与 Adapter 声明不符，不能继续沿用旧能力视图。

控制面将 Adapter 降级到已经验证且仍然成立的范围，并让全部任务使用这个降级后的能力范围。依赖失效能力的投机活动停止并清理；不依赖失效能力的活动可以继续。Policy Engine 和后续新任务只能使用降级后的能力视图。

重新验证通过后，Adapter 才能恢复使用新的能力范围。重新验证不会自动恢复已经停止的投机活动，也不会把历史运行期偏差当作已不存在。

## 相关文档

- [Speculation Contract](speculation-contract.md)：Policy 与 Adapter 之间的总体语义关系
- [控制面设计](control-plane.md)：Adapter Registry 与控制面其他组件的协作
- [Runtime Adapter 接入指南](../development/runtime-adapter.md)：Adapter 作者的实现与接入方法
