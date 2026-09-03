# Runtime Adapter 接入指南

本文面向 Runtime Adapter 作者，说明如何把一个外部 Runtime 接到 AScout 控制面。它是开发与接入指南，不定义 Runtime 内部架构，也不把建议实现提升为 AScout 目标语义。控制面接入边界的规范来源是 [Speculation Contract](../design/speculation-contract.md)；完整 operation/event schema 尚未收敛。

## 1. 接入责任

Runtime Adapter 位于控制面与外部 Runtime 之间，主要完成四件事：

1. 声明目标 Runtime 能兑现的执行、隔离、资源、效果与清理能力；
2. 把控制面的 semantic operation 映射为目标 Runtime 的一个或多个原生调用；
3. 把后端结果、事件、能力变化与错误规范化为控制面可解释的数据；
4. 在无法完整兑现语义时返回明确失败或部分完成状态，而不是把较弱行为伪装成成功。

AScout 能规定的是控制面接受什么输入输出和证据，不能规定 Adapter 内部怎样组织代码、怎样调用 Runtime 或使用什么 transport。

## 2. 状态边界

Adapter 可以维护连接、协议协商结果、后端 handle 和能力映射缓存，但这些只是适配状态。它不能向控制面呈现第二份长期权威执行状态，也不能从缓存推断一次后端转换已经完成。

Speculative Tree、Branch 关系、Policy 绑定和控制状态属于控制面；实际进程、workspace、资源、隔离和效果状态位于外部 Runtime。Adapter 负责在两者之间传递足以对齐状态的结果与事件，而不取得任一侧的状态所有权。

## 3. 建议接入过程

### 3.1 识别后端原生能力

先列出目标 Runtime 实际提供的创建、停止、隔离、资源管理、效果处理、状态查询和事件能力，不假设不同 Runtime 具有统一原语。容器可能通过新建环境与状态重建表达分支，具有 COW 能力的后端可能提供更低成本路径；这些差异应成为能力与成本事实，而不是泄漏成控制面私有 backend handle。

### 3.2 建立语义映射

对控制面要求的每项 semantic operation，说明它由哪些后端调用组成、何时算完成、失败后可观察到什么，以及是否存在部分完成。映射可以是一次原生调用，也可以是有明确失败收敛的调用序列。

不要仅凭“请求已发送”返回成功。控制面需要依据 Adapter 的结果或事件判断创建、接受、丢弃、清理或效果转换是否真正达到对应语义状态。

### 3.3 报告能力与限制

Adapter 应报告它借助目标 Runtime 能兑现的能力、保证范围、成本和已知限制。Policy Engine 使用这些事实判断 Policy 是否存在合法执行路径。无法承载的能力应标为不可用；较弱保证只能作为显式降级报告，不能静默替代。

### 3.4 规范化结果、事件与错误

后端专有的状态码、连接错误、资源压力和异步事件需要转换成控制面可以稳定解释的结果。规范化至少要保留操作身份、完成程度、失败位置和后续是否仍需清理等信息；具体字段将在 operation/event contract 中另行确定。

### 3.5 通过接入与运行期校验

接入校验用于检查 Adapter 的能力声明与基本语义是否相符；运行期校验根据真实结果与事件发现声明和实际行为之间的偏差。Adapter 通过校验只表示它在声明范围内可以接入，不表示 AScout 认证了 Runtime 的内部设计或所有安全属性。

## 4. 实现建议而非规范

以下内容由 Adapter 作者根据目标 Runtime 决定：

- 进程内调用、Unix socket、daemon IPC、VM channel 或远程 RPC 等 transport；
- discovery、连接、重连、关闭和协议协商方式；
- 后端 handle、缓存和通信组件的内部组织；
- 一个 semantic operation 如何拆成后端调用；
- 针对性能、批处理、重试与资源压力的优化。

这些选择不能改变控制面看到的 operation 语义、身份与能力边界、失败可见性或 cleanup 结果。

## 5. 尚待收敛

当前尚未定义完整的 operation/event 集合、版本协商、幂等键、事件顺序、取消、超时、断线恢复和部分完成表示。它们需要从控制面实际所需语义出发逐项讨论，不能从某个候选 Runtime 的接口反推。

## 相关文档

- [AScout 架构](../architecture.md)：Runtime Adapter 在四侧调用路径中的位置
- [Speculation Contract](../design/speculation-contract.md)：控制面接入边界与校验
- [控制面设计](../design/control-plane.md)：Adapter 的直接调用方
- [效果管理设计](../design/effect-management.md)：效果能力与结果要求
