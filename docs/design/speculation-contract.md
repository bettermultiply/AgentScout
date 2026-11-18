# Speculation Contract 设计

Speculation Contract 是 AScout 用于规范用户自定义 Policy，并规定 Policy、control plane 与 Runtime 之间关系的总体契约。runtime proxy 的具体接入契约、能力声明、operation/event 和校验要求见 [runtime proxy 契约设计](runtime-adapter-contract.md)。AScout 不通过这份 Contract 固定 Runtime 内部实现。

## 1. Contract 的定位

现有的投机执行工作大多把策略与运行时、甚至与 Agent 实现强耦合，一侧的修改常常迫使另一侧联动修改，运行环境无法被不同策略复用，策略之间也未考虑兼容性。这种耦合同时带来很高的切换心智负担：修改者必须同时了解投机策略、Runtime 如何运行、Agent 本身的执行阶段，才能判断一处改动是否会影响另一端完成正确的修改。

Contract 的首要目的，是对齐投机策略与 Runtime 能力，避免一侧变化迫使另一侧重写。它规定 Policy 如何表达意图、control plane 如何编排操作、Runtime 如何通过接入边界呈现能力与结果。环境承载与效果处理都是 Runtime 的整体责任，具体能力可由不同实现组合提供。

```text
Policy 决策
    ↓ 只能通过 Contract 规定的决策接口表达
control plane 机制
    ↓ runtime proxy 接入边界
Runtime：承载动作、管理状态与效果边界
    ↓
control plane 状态更新
```

Contract 是上述约束的总称，实现这些约束的抽象责任称为 Mechanism。Mechanism 由 AScout 自身能实现、能强制的固定机制共同构成，不归属 control plane 内的任何具体组件，而是组件协同完成检查和翻译的过程。

Mechanism 的责任包括校验 Policy 与 Runtime 能力的匹配，将策略意图落实为受管操作，并维护操作之间的状态与不变量。control plane 维护全局控制状态，Runtime 落实承载和效果约束；runtime proxy 规范化接入数据，不能改变这些责任。

Contract 约束 Policy 的表达，以及 Runtime 通过接入边界声明和返回的可观察行为，不固定 Runtime 内部使用 Overlay、容器或虚拟机。control plane 与 Runtime 共同落实约束；control plane 如何组织校验与编排，见 [control plane 设计](control-plane.md)。

## 2. Contract 约束什么

Contract 本质上规定 Policy 能表达什么，以及 control plane 接受怎样的 runtime proxy 能力、结果和事件。它通过受限入口和校验使策略表达与后端接入对齐。Contract 的固定语义包括以下相互联系的内容：

- Policy 已经通过 Policy Decision 约束其设计空间，但 Decision 取值的合法性仍需校验。同时随着 Runtime 的变化，其能提供的隔离能力也会有所不同，这就造成了可能出现语义不匹配，同样表现为 Decision 取值的合法性。
- runtime proxy 由固定的接口约束其输入输出，但相同接口约束下能提供的隔离度也并不相同，真实的隔离度需要进行校验。
- 失败时如何收敛：尚未被接受的投机 Branch 对有效执行、成功产物与副作用的影响等效于没有创建过，不重放该操作；审计记录必须保留，但不得重新执行或对权威生效。被接受的 Branch 已完成使命，不适用这一失败语义。

## 3. 如何约束

Contract 的约束只有落实到固定执行点才具有约束力。AScout 不要求 Policy 或 runtime proxy 通过自觉遵守一份文档来获得正确性，而是让它们只能通过受限制的接口进入执行路径。

### 3.1 对 Policy：决策取值合法性的校验

Policy 通过 Policy Decision 表达策略，其设计空间已经被约束，但具体取值是否合法仍要由 control plane 在决策进入执行路径前主动校验。Policy Engine 检查的对象包括决策的输入输出、决策之间的组合、所依赖的验证依据，以及 Policy 对效果、资源和 Fidelity 作出的声明。

校验的合法性是相对当前 Runtime 的。同一个 Decision 取值在某个后端上可行，在另一个只能提供更弱隔离能力的后端上就不可行，这种语义不匹配同样表现为取值不合法。例如，Policy 要求一个需要隔离和清理能力的候选执行，而当前 runtime proxy 报告的后端能力不足以承载时，该取值没有合法路径。若 Policy 的目标超出 AScout 固定语义，或当前后端无法提供所需能力，control plane 必须拒绝该决策或按既定语义报告降级。

检查通过后，Policy 的结果仍只是策略意图。实际 Branch、生命周期、效果处理与 Runtime 环境操作由 control plane 固定机制形成，Policy 不能通过返回一个特殊值绕过 control plane 状态转换。

### 3.2 对 runtime proxy：具体接入契约

runtime proxy 的具体接入契约、能力声明、接入校验、运行期校验和 operation/event 边界，见 [runtime proxy 契约设计](runtime-adapter-contract.md)。本 Contract 只规定这些具体约束必须服务于 Policy 与后端能力的语义对齐，不在此重复定义 runtime proxy 接口。

### 3.3 失败处理

跨层操作可能在不同位置失败：Policy 可能无法产生有效 Proposal，control plane 可能发现当前状态不允许某项转换，Runtime 可能无法完成效果处理或环境操作，runtime proxy 可能无法完成接入映射。失败结果需要说明操作处于什么状态、接下来如何收敛，不能让某一层用成功值掩盖。

对尚未被接受的投机 Branch，失败后对有效执行、成功产物与副作用的影响等效于没有创建过：不保留部分结果作为成功，也不重放该投机操作；审计和消息旁支必须保留，但不赋予执行权或权威效力。control plane 沿固定状态入口撤销对应 Branch 的控制状态，并请求 Runtime 停止对应执行、处理资源与受管效果。Salvage 是可选的残值回收，可在这一默认之上把可移交权威路径的部分进展保留下来，不是失败处理的必要条件。

系统外正确性回放是在独立验证运行中重新提供录制的模型输出，不是在原 task 自动重试、恢复或继续被丢弃 Branch；不绕过 Runtime、效果边界或 single-winner。

## 4. 完整控制闭环

一次投机执行分成两段：**创建投机 task 时的静态校验**只进行一次；之后投机内部每次具体操作运行遵循同一个闭环，运行中不再重复前置静态校验，而是以执行后的结果确认。

创建投机 task 时，做一次静态校验：

```text
Policy 产生投机决策（predict / admit / ...）
    → Policy Engine 校验决策合法性（表达、组合、声明相容）
    → 通过则建立 candidate Branch 的控制状态并创建受管执行
    → 不通过则拒绝或明确降级，不创建
```

这次校验确认 Policy 的决策在表达上合法、与 runtime proxy 声明的能力在声明层面相容（见 §3.1、§3.2）。它只证明"这个投机值得尝试、按声明应当可行"，不证明运行中会真的按语义完成。

投机 task 建立后，内部的具体操作（一次读取、一次工具调用、一次效果处理等）遵循运行闭环：

```text
Policy 产生推进或处置决策
    → control plane 机制检查状态并形成语义操作
    → Runtime 通过接入边界接收操作并落实承载及效果约束
    → Runtime 返回实际结果和事件
    → control plane 规范化并记录结果
    → control plane 根据结果更新控制状态
```

这里的“control plane 机制检查状态”判断操作在当前控制状态下是否允许。环境和效果操作都要依据 Runtime 返回的实际结果确认，control plane 据实更新控制状态，不能把计划当成完成。

以接受和丢弃为例，Authority 或 validator 提供依据后，control plane 通过 `match` 和 `dispose` 得到处置意图，并经过固定状态入口检查。Runtime 处理相关环境、进程、资源和效果；control plane 依据实际结果更新 Branch 与 Proposal。Policy 选择候选，不代表状态转换和效果处理已经完成。

## 相关文档

- [control plane 设计](control-plane.md)：control plane 如何作为 Mechanism 落实 Contract
- [runtime proxy 契约设计](runtime-adapter-contract.md)：runtime proxy 的具体接入契约与校验要求
- [Policy Decisions 设计](policy-decisions.md)：Policy 决策面
- [Branch 语义设计](branch-semantics.md)：Branch 生命周期
- [效果管理设计](effect-management.md)：状态与效果
- [架构](../architecture.md)：Instrumentation、control plane 与 Runtime 的所有权
