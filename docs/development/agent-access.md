# Agent 接入侧通用层

本文记录 Agent 接入侧通用层的当前实现、接口与限制；目标语义见 [Agent 接入侧设计](../design/agent-integration.md)。

## 1. 分层

- `agent-access/core/`：与具体 Agent product 无关的通用接入逻辑，包括本地 Policy 求值调用、`extent` 的 step 计数与求值、`effect` 闸门、Blocked/Resumed 上报，以及等待控制面处置的 hold。
- `agent-access/pi/main.ts`：Pi 的 Product Integration，只负责把 Pi 的工具执行前、模型推理前等执行点转成通用层的执行点事件，执行返回的处置，并上报 Pi 专有的消息、checkpoint、session 与用量事实。

通用层不读取 Pi 的消息结构、checkpoint、session、SDK、模型对象，也不读取 NLE 专有字段。Pi 把 `checkpoint` 作为不透明的 admission fact 交给通用层，通用层只把它原样放进 `speculative_admission_request`，不解释其内容。

## 2. 接口

`agent-access/core/index.ts` 导出 `createAgentAccess(options)`。调用方提供：

| 选项 | 含义 |
| --- | --- |
| `taskId` | 上报事实中的 task 身份 |
| `report(message)` | 向控制面发送一条已构造好的消息 |
| `evaluate(artifact, decision, data, signal)` | 执行一次编译 Policy 的 Decision 求值；由接入进程注入，通用层不绑定具体 runner 位置 |

返回的接口：

| 方法 | 用途 |
| --- | --- |
| `beforeAction(point)` | 产品在真实执行前调用。执行 effect 闸门、rebalance pause、`admit`、`extent` 与相应上报，最后返回 `allow` 或 `admit` 处置 |
| `control(command)` | 按控制面命令维护 hold、本地 Policy 和 extent 计数；产品适配在命令处理入口调用 |
| `releaseAll()` | 进程退出时释放全部 hold |

`ExecutionPoint` 只承载通用事实：`branch`、`actionId`、`actionKind`、`name`、`arguments`、Policy 的 `op` 与 `contractContext`、是否允许准入、附加的 admission facts、取消信号。产品用函数形式的 `admit` / `admissionFacts` 表达“等到闸门放行后再判断是否是权威路径”和“准入成立时才生成产品事实”。

## 3. 当前覆盖

- `admit.opportunity_pass` 与 `admit.feasibility_pass` 在通用层求值；准入成立时上报 `speculative_admission_request`。
- `extent` 按 Branch 和 action identity 计数，只在工具执行点求值；结果为 `stop` 时上报 `branch_suspended(extent_reached)` 并等待 `resolve_pending` 或取消释放。
- `effect` 只对 `tool_execution` 执行点求值；`deny` 直接抛错，`block` 上报 `branch_suspended(effect_gate)` 并等待 Promote、Discard 或清理释放。模型推理是 PURE 步骤，不经过 effect 闸门。
- `rebalance` 的 pause/resume 与 extent/effect hold 都由通用层保存和释放；`branch_suspended` / `branch_resumed` 由通用层上报。
- 控制面命令的 hold 生命周：`start_candidate` 建立 effect hold，`promote` 释放 effect/extent，`discard`、`cleanup` 释放全部 hold 并清理本地状态，`resolve_pending` 释放 extent。

## 4. 限制

- 只有 Pi 一个 Product Integration；sub-agent 启动前仍无统一 Hook。
- effect 只实现 `block` / `deny` 两种最小计划，没有 Reuse、Salvage 或更多效果模式。
- extent 计数的是工具执行点的 action 次数，不是模型语义上已完成步骤数；子 Chance 的计数独立于父 Chance。
- hold 只存在于接入进程内，不跨进程恢复；控制面仍是 Tree、Chance 和提交资格的唯一权威。
- 通用层不等待“控制面返回的任意处置”；它只把控制面命令落实为 hold 的释放，真正的继续/阻止/使用结果仍由 Product Integration 在本次调用返回路径上完成。
- `node --test tests/pi-access.mjs` 覆盖 Pi 接入行为，使用伪造 SDK 与手工事件，不调用真实模型；本轮没有新增测试。
