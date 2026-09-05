# Runtime Adapter 接入指南

本文面向 Runtime Adapter 作者，说明如何准备能力声明、实现环境操作、参加 Registry 验证、完成注册并处理运行期偏差。正式语义和接入条件见 [Runtime Adapter 契约设计](../design/runtime-adapter-contract.md)。

## 1. 接入前先画清边界

Adapter 接入工作的对象是 Runtime 环境：`establish_environment`、`operate_environment`、`suspend_environment`、`resume_environment`、`terminate_environment` 和 `cleanup_environment`。能力声明描述这些固定操作对不同环境内容形成的实际隔离保证。

先确认以下事实：

- 当前 Runtime 实例使用了哪些配置；
- 哪些环境可以建立、环境内操作、静止、恢复、终止和清理；
- Agent 接入侧如何在环境内建立并获得本地控制入口；
- 接入侧无权完成的环境级操作由哪一侧执行；
- 进程、workspace、memory、I/O、network、resource 和环境内控制入口分别有什么边界；
- 哪些路径依赖拦截，拦截是否能在影响发生前覆盖全部声明范围；
- 哪些保证当前无法证明，应标记为未验证或不适用。

不要从 Runtime 的理论能力出发编写声明。声明必须反映当前 Adapter 实例和当前配置的实际保证。

## 2. 编写能力声明

固定环境操作由契约统一规定。能力声明描述这些操作作用于不同内容时形成的实际隔离保证。

Adapter 提交的是当前实例、当前 Runtime 配置下的 `AdapterDeclaration`，不是理论能力清单。声明包括契约版本、实例标识、配置标识、各类环境内容的 `ContentIsolation` 和当前部署条件。Registry 解析声明后生成 `CapabilityView`，并附加验证状态；Adapter 不填写验证结果。

每类内容按以下隔离属性整理：

| 维度 | 要回答的问题 |
|---|---|
| `visibility` | 其他环境能看到什么？ |
| `modification` | 其他环境能直接修改什么？ |
| `interference` | 其他环境还能通过什么路径产生影响？ |
| `lifetime` | 静止、恢复、终止和清理如何影响它？ |
| `cleanup` | 清理后是否还有残留？ |
| `scope` | 保证覆盖哪些对象、路径或配置？ |
| `limitations` | 哪些例外和路径不在保证范围？ |

这组属性可以随着契约演进扩展。不要用一个 `isolation_level` 掩盖不同内容之间的差异，也不要用自由文本替代可解析的范围和限制。

每项内容可以标记为 `not_applicable`，但必须仍然遵守接口契约并返回明确结果。`not_applicable`、`unsupported`、`unverified` 和 `verification_failed` 含义不同，不能混用。

同一类内容只提交一项有效声明。范围和限制要能被 Registry 解析；自由文本只能补充说明，不能承担能力匹配的唯一依据。配置变化后重新生成声明并重新接入验证。

## 3. 实现固定环境操作

对每个固定操作，明确完成条件和失败收敛：

- `establish_environment` 完成后，必须能证明环境已经可用，才能返回完成；
- `operate_environment` 需要返回实际建立、执行或拒绝的结果；
- `suspend_environment` 必须阻止新的环境操作，并说明未完成工作；
- `resume_environment` 必须让原环境重新可用，并保持已有状态和隔离边界；
- `terminate_environment` 必须不可恢复地停止 Runtime 管理的执行；
- `cleanup_environment` 必须处理 Runtime 管理的进程、资源、临时状态、句柄和租约；

多个后端调用组成一个操作时，Adapter 要定义中间失败如何暴露，以及后续是否必须 cleanup。不要把请求发送、metadata 更新或本地缓存变化当作 Runtime 已完成。

## 4. 注册和运行

注册流程如下：

```text
固定当前 Runtime 配置
    -> 提交当前实例的能力声明
    -> Registry 解析声明并检查一致性
    -> Registry 建立验证环境并执行验证
    -> Registry 发布带验证状态的能力视图
    -> Policy Engine 检查 Policy 是否匹配
```

Adapter 需要向 Registry 提供建立验证环境、执行环境内操作和观察验证结果所需的接入信息。验证场景和验证结果由 Registry 管理；Adapter 作者不提交自测结果作为接入证明。

Adapter 配置注册后不得在运行期变化。若发现 Runtime 实际行为不符合声明，Adapter 必须提供足够证据让 Registry 和控制面识别偏差；不能继续返回旧的成功能力。

未验证能力默认不进入普通可用范围。部署方可以主动确认，并把它限制在指定部署范围内使用；Adapter 作者必须明确记录这项能力的范围、限制和风险。验证失败的能力不能通过用户确认绕过。

## 5. 运行期偏差

运行期验证是接入验证的兜底，不是替代。发现声明与实际行为不符时，控制面会把 Adapter 降级到仍然成立的能力范围；全部任务使用降级后的范围，依赖失效能力的投机活动停止并清理，不依赖的活动可以继续。重新验证通过后才能恢复新的能力范围。

## 相关文档

- [Runtime Adapter 契约设计](../design/runtime-adapter-contract.md)：正式接入契约
- [控制面设计](../design/control-plane.md)：Adapter Registry 与控制面协作
- [Agent 接入侧设计](../design/agent-integration.md)：环境内 Agent 接入侧的责任
- [效果管理设计](../design/effect-management.md)：外部效果和相应 backend 的责任
