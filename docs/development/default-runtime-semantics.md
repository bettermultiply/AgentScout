# Runtime 语义整理

本文整理 Runtime 执行承载可能需要提供的语义，作为开发阶段的方向性文档。它不替代 `docs/design/runtime-adapter-contract.md`，也不改变 `docs/design/branch-semantics.md` 中已经确定的 Branch 结构、生命周期和信息流语义。

## 1. 执行承载

Runtime 为 Branch 的 Instance 提供执行承载。承载可以从 Branch barrier 建立的 execution base 开始，也可以在逻辑 Branch 已登记后按需建立。Runtime 只报告实际建立、运行、暂停、恢复、终止和清理结果；控制面不能把预期状态当作实际状态。

完整 Agent execution 的进程树、I/O、资源归属和本地管理上下文需要由接入侧与 Runtime 的组合共同覆盖。具体进程组织、环境布局和承载方式不在本文固定。

## 2. 环境边界

当某类 Branch-local state 要求独立承载时，Runtime 或其他执行承载机制需要从相应 execution base 建立独立环境，使该状态能够按 Branch 继续演化。环境边界的具体内容、保证范围和验证状态由 Runtime Adapter 契约描述。

环境边界可以涉及：

- 进程和进程关系；
- workspace 和临时文件；
- memory 与其他执行状态；
- I/O、网络和本地通信；
- 资源、权限与环境内控制入口；
- 停止、残留和 cleanup。

这些内容是 Runtime/Adapter 的能力与验证对象，不是 Branch Root 的结构定义。

## 3. 可选的强化语义

以下语义可以由特定 Runtime 或 Adapter 提供，但不作为所有 Runtime 必须具备的 Branch 前提：

- sibling environment 的不可见与不可修改；
- 完整进程树的 checkpoint、restore 或廉价 fork；
- memory、workspace 和 I/O 的强隔离；
- 环境间资源干扰限制；
- loser environment 的批量终止与无残留清理；
- winner continuation 所需的环境复用或状态提升。

如果某项 Policy 或 Contract 需要这些能力，控制面必须根据 Adapter 的能力声明和验证结果选择、拒绝或明确降级，不能把未验证能力当作已满足。

## 4. 与 Branch 语义的连接

Runtime 提供执行承载和环境级结果；Branch 语义决定这些结果如何参与 Branch barrier、resolution、winner continuation、discard 和 cleanup。Runtime 环境结果不能直接变成 Branch accept、discard、commit 或 winner 转换结果。

Branch 之间的业务信息通过 Branch 语义规定的 outcome、输入和其他受控通道传播。Runtime 是否提供更强的环境隔离，不改变 Branch 的树结构、single-winner、obligation 或 resolution 语义。

## 相关文档

- [Branch 语义设计](../design/branch-semantics.md)：Branch 结构、生命周期、依赖、状态可见性和 resolution
- [Runtime Adapter 契约设计](../design/runtime-adapter-contract.md)：Adapter 的正式能力声明、验证和环境操作契约
- [Runtime Adapter 接入指南](runtime-adapter.md)：Adapter 实现与接入方法
