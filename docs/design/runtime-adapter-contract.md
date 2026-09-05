# Runtime Adapter 契约设计

本文定义 Runtime Adapter 面向 AScout 控制面的具体接入契约。它是 [Speculation Contract](speculation-contract.md) 的直接下一级文档，负责把“Adapter 必须提供可检查、可执行的语义”落实为控制面接入边界；不定义目标 Runtime 的内部架构、原生接口或 transport。

## 1. 文档边界

本文承载 Runtime Adapter 的强制接入语义，包括：

- Adapter 的能力声明与限制；
- 控制面接受的 typed operation/event 边界；
- Adapter 接入时的基本语义校验；
- 运行期结果、事件与能力偏差的校验；
- 失败、部分完成、取消和 cleanup 的可见性要求。

Adapter 作者如何准备接入、选择后端原语和组织内部实现，见 [Runtime Adapter 接入指南](../development/runtime-adapter.md)。

## 2. 接入责任

Adapter 必须把控制面要求的 semantic operation 映射为目标 Runtime 能实际执行的行为，并把结果、事件、能力变化和错误规范化为控制面可解释的数据。Adapter 不得把较弱行为伪装成成功；无法完整兑现语义时，必须返回明确失败或部分完成状态。

接入校验的对象是 Adapter 在控制面边界上呈现的环境管理行为，而不是 Runtime 的内部实现，也不是 Agent 接入侧的投机管理语义。Adapter 通过接入校验，只表示其能够提供符合要求的环境边界和环境操作，不表示 AScout 认证了 Runtime 的内部设计或全部安全属性。

## 3. 基础环境要求

隔离是 Runtime Adapter 必须满足的基础语义，不是可以关闭的附加能力。Adapter 必须证明 Runtime 能建立可用的隔离环境，在其中建立 Agent 接入侧并执行接入侧无权完成的环境级操作，停止环境，清理环境及其资源，并让仍然有效的环境继续使用。Runtime 只管理环境及其环境级状态，不管理 Agent 执行实体、Branch identity、capability 或投机状态。

校验必须分别覆盖进程、workspace、I/O 与通信、资源、环境内操作权限、停止后的残留、cleanup 结果，以及环境继续使用时的状态和隔离，不能用单一的“支持隔离”声明代替。每项操作必须明确区分请求已接受、实际完成、失败和部分完成；环境操作成功不自动代表上层 Branch 操作已经完成。

## 4. Adapter Registry

控制面通过 `Adapter Registry` 管理 Runtime Adapter 的接入边界。Adapter Registry 负责接入登记、契约兼容性检查、能力声明校验和接入时主动基本语义测试，并把已通过校验的能力信息发布给 Policy Engine 和控制面运行路径。它维护 Adapter 的接入状态与校验结果，但不拥有 Speculative Tree、Branch 或 Runtime 的执行状态，也不替代 Speculation Coordinator 进行运行期生命周期协调。接入验证应尽可能通过真实环境操作完成；运行期验证用于发现主动验证遗漏或实际偏差。

## 5. 验证结果

Adapter 能力按验证状态区分：

- **已验证**：已通过接入时主动验证，可作为普通可用能力提供；
- **未验证**：接入时无法完成充分证明，默认不作为已验证能力使用；部署方可以主动确认，并将其限制在指定部署范围内使用；
- **验证失败**：主动验证证明不满足要求，不能通过用户确认绕过。

用户确认只承担未验证能力的使用风险，不能把未验证能力伪装为已验证，也不能把验证失败改成通过。具体保证范围必须随能力状态明确呈现。

## 6. 尚待收敛

- 能力声明的完整字段和版本格式；
- 基本语义测试的测试项、隔离环境和通过标准；
- operation/event 的完整集合、幂等键、事件顺序、取消、超时和断线恢复；
- 运行期发现 Adapter 偏差后的 Task 处理、存量 Branch 清理和重新接入规则；
- Adapter 能力变化是否允许热更新。

## 相关文档

- [Speculation Contract](speculation-contract.md)：Policy 与 Adapter 之间的总体语义关系
- [控制面设计](control-plane.md)：Adapter Registry 与控制面其他组件的协作
- [Runtime Adapter 接入指南](../development/runtime-adapter.md)：Adapter 作者的实现与接入指南
