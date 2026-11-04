# Runtime 设计

## 目标

Runtime 是 AScout 用来承载 Agent 一步动作的受约束执行系统。一步动作可以是一次模型推理或一次工具调用。它不是替 Agent 决定如何推理，也不只是把命令运行起来，而是提供动作正常、正确运行所需的条件，并把动作对真实世界或 Authority 的影响限制在允许范围内。投机动作通常不应直接影响真实世界；具体允许哪些影响，必须遵守任务要求和既定的效果约束。

Runtime 面向一般投机执行，不专门为 NLE 或某个 Agent 产品设计，也不等同于虚拟机。AScout 自身支持一套 Runtime；Runtime Adapter、Effect Coordinator、环境、进程与资源管理都属于这个整体中的接入或实现职责，而不是几套平级系统。适配外部执行机制是为了便于接入，不意味着 AScout 只有适配器而没有自己的 Runtime。

## 状态与接入侧

Session、Workspace、进程和其他状态可以按需采用不同的承载方式，包括复制、共享、写时复制、重建或恢复。目标是在满足正确性和效果边界的前提下尽可能降低成本，而不是每次投机都完整复制 Agent 或创建独立进程、容器、虚拟机。一步动作是最小讨论粒度，不意味着环境只能存在一步，也不要求每步重新创建环境。

Agent Session 的内部管理、fork、恢复和继续运行由接入侧负责，因为它理解具体 Agent 的会话结构和执行切点。Runtime 为相应执行活动提供环境、状态和效果约束；接入侧与 Runtime 必须关联到同一 Branch 和动作上下文，不能只恢复 Session 而忽略它所依赖的文件或进程状态。控制面仍管理全局 Branch 关系与裁决；Session 派生、环境派生和 Authority 切换不能相互冒充完成。

## 可插拔能力

Runtime 的具体能力都可以插拔和组合，包括文件系统隔离、进程隔离、网络隔离、资源限制、状态保存与恢复、效果处理等。Overlay、Container 和 Firecracker 是不同的能力实现，可以替换或组合。Overlay 是文件系统隔离的一种实现，不是所有 Runtime 的必需结构；如果 Firecracker 等机制已提供符合要求的文件系统隔离，就没有额外使用 Overlay 的要求。

也可以不使用上述隔离机制而运行，只是可提供的保证和可承载的任务范围会缩小。能力退化不意味着可以放宽必要的正确性和效果边界：仍满足要求的动作可以继续；不满足要求的动作应等待、采用其他合法路径、明确降级或拒绝，不能把未隔离状态声称为已隔离。效果处理需要服从既定的 commit、accept、discard 和 single-winner 语义，接受 Candidate 不等于无条件复制文件回 Parent，也不增加通用状态合并能力。

任务需要哪些能力、当前能力是否足够，不能由 Runtime 单方面判断；它需要与 Policy 协调、协商，并结合运行时检测确认实际保证。具体协商与检测机制、当前开发范围见 [Runtime 开发说明](../development/runtime-adapter.md)。本设计不预先固定它们的接口、部署或实现。

## 相关文档

- [架构](../architecture.md)：接入侧、控制面和 Runtime 的职责。
- [Runtime Adapter 契约](runtime-adapter-contract.md)：Runtime 的可观察接入边界。
- [Agent 接入侧](agent-integration.md)：Session 和执行切点。
- [效果管理](effect-management.md)：状态变化与效果处理纪律。
- [Branch 语义](branch-semantics.md)：接受、丢弃和状态可见性。
