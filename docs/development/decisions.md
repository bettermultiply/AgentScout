# 开发决策

本文记录已接受的实现架构与验证决策。目标语义归 `docs/design/` 所有。

## D-026-I：抽象札记不重复维护已收敛语义

2026-09-03 决定：`docs/development/spec-exec-abstraction.md` 的原 §1（Edge 生命周期与核心实体）和原 §3（Assumption DAG 组合语义）不再保留第二份规范定义。两节改为指向 `docs/design-model.md` 与 `docs/design/policy-decisions.md` 的下沉说明；该开发札记仅保留论文调研、候选方案和实现分析。同步更新 glossary 中受影响的术语引用，避免把开发札记当作语义来源。

## D-027-I：效果管理规范只保留单一来源

2026-09-03 决定：`spec-exec-abstraction.md` §4 保留论文对比和研究动机；EffectClass、StateDomain、效果处理模式、效果边界阻塞及审计等规范性内容以 `docs/design/effect-management.md` 和 `docs/design/policy-decisions.md` 为唯一来源，不在开发札记重复维护。同步更新 glossary 的 EffectClass/StateDomain 引用。

## D-028-I：嵌套投机章节只保留调研与假设

2026-09-03 决定：`spec-exec-abstraction.md` §9 不再重复定义 Edge、Branch、AssumptionSet 或父子生命周期语义；正式规则以 `docs/design-model.md`、`docs/design/policy-decisions.md` 和 `docs/design/branch-semantics.md` 为唯一来源。§9 保留论文覆盖情况、成本/收益分析、实验假设和候选实现方向；未经确认的概率公式、深度限制和接口形态不得写成通用机制约束。

## D-029-I：废弃单 Edge 即时启动与固定 frontier 策略

2026-09-04 决定：废弃由 `admit` 精判直接决定投机是否立即启动的七槽决策面，现由 `admit` 决定机会是否进入待选集合，独立的 `select` 在同一 task / AScoutAgent 的全部待选 Edge 间选择当前启动项。也废弃将 best-first、固定的 `P(存活) × 期望延迟节省 / 资源成本` 排序、浅层分叉/深层单链或强制 `survival` 字段写成 AScout 机制规则的表述；这些机制作为 Policy 可选策略继续受支持。当前语义见 `docs/design/policy-decisions.md`、`docs/design/control-plane.md` 与 `docs/design/decisions.md` D-043。

## D-030-I：废弃 Scheduler 固定暂停与抢占策略

2026-09-04 决定：废弃“所有投机工作固定低优先级”以及 Scheduler 根据内置价值排序自行暂停或抢占投机工作的表述。Scheduler 继续拥有运行队列、资源和硬约束，但暂停、恢复和价值驱动的抢占由独立 `rebalance` Policy Decision 选择；抢占后的处置由 `dispose` 决定。当前语义见 `docs/design/policy-decisions.md` §2.9 与 `docs/design/decisions.md` D-044。

## D-033-I：Speculation Coordinator 吸收 Scheduler 的控制面职责

2026-09-04 决定：在控制面组件模型中，不再将 Scheduler 作为独立组件。其待选机会、运行调整、预算和硬约束执行职责并入 Speculation Coordinator；`select` 与 `rebalance` 仍由 Policy Decision 决定，Speculation Coordinator 在统一的全局投机视图上负责激活、复核和执行这些意图。D-030-I 中关于 Scheduler 的描述保留为历史决策记录，当前组件结构以本条和 `docs/design/control-plane.md` 为准。

## D-031-I：废弃端用户直接参与投机 Branch 裁决

2026-09-04 决定：废弃通过专门投机 UI 让端用户直接观察、选择或裁决内部 Branch 的表述。端用户只通过 AScoutAgent 视图操作当前有效的权威执行主线，并只能看到该路径的内容、状态与交互；人作为 Authority 提供输入或中断时，也通过该主线进入系统。Branch 接受、丢弃、重新挂载与清理在内部完成，不要求用户选择或重绑定 Branch。当前语义见 `docs/design/agent-integration.md`、`docs/design/branch-semantics.md` 与 `docs/design/decisions.md` D-045。

## D-032-I：前提变更暂不兼容或修复 Branch

2026-09-04 决定：废弃在用户操作改变 Proposal 前提后继续保留、挂起、兼容判断、修复或重新对齐受影响 Branch 的候选方向。现阶段所有依赖已变更前提的 Branch 及其 subtree 一律剪枝；未依赖该前提的 Branch 不受无关输入影响。当前语义见 `docs/design/branch-semantics.md` §6、`docs/design/policy-decisions.md` §4 与 `docs/design/decisions.md` D-046。

## D-034-I：抽象札记收敛为论文证据

2026-09-14 决定：`spec-exec-abstraction.md` 只保留论文覆盖、实验结果、负结果、效果处理现状、shadow/replay 方法学、接入与隔离事实、嵌套投机事实及待验证问题。已经在 `docs/design/` 正式呈现的生命周期、Policy Decision、Effect、架构、组件职责、契约和指标方案从该文删除；该文不再作为目标语义或实现结构的来源。

## D-015-I：首个真实 Workload 使用 Delayed-Choice Coding

首个端到端 workload 使用 `ascout codex`、Command + Skill 和真实仓库。Deterministic Agent 只用于 conformance tests。Sequential 与 speculative run 使用相同 Agent、任务、outcome 和预算；Hook automation 推迟到显式路径证明正确性与收益之后。

Workload contract 与指标见 [reference-workload.md](reference-workload.md)。

## D-016-I：默认 Runtime 优先高效隔离

默认 Runtime 面向可信 Child Agent 与工具，优先低开销环境隔离，不提供 adversarial containment。它必须强制维护隔离环境、环境级资源边界、环境内受控操作、停止和 cleanup。Agent 执行实体、Branch context、operation 归属和 capability 路由由 Agent 接入侧管理；受管 external effect 由控制面内的 Effect Coordinator 负责实际处理。

旧的“Runtime 直接维护 Branch identity/capability、完整 Agent 执行实体和所有 effect obligation”划分已废弃；它把投机执行职责固化进通用 Runtime，违背 Runtime 只提供环境能力的边界。

需要 adversarial containment 时，用户选择另行适配的 sandbox/hardened Runtime。AScout 不协商 security guarantee，默认 backend 技术尚未选择。

## D-017-I：新 Rust 实现不继承旧代码

新实现继续使用 Rust。旧 `src/**`、Cargo manifests、tests、examples、C header、构建产物和实验文件已经删除，未读取、迁移或复用。新实现只能依据已接受的设计与开发决策建立。

## D-024-I：实现组织服从 AScout 可控边界

完整调用路径仍为：

1. Agent 接入侧；
2. 控制面；
3. Runtime Adapter；
4. Runtime。

这四侧不再直接作为 AScout 源码或子文档的一一对应拓扑。Agent Control Gateway、Policy Engine、Speculation Coordinator、Effect Coordinator 与 Ledger 位于控制面内部；Interposer、ASC-Proxy 及 Product Integration 位于 Agent 接入职责。控制面定义 Runtime Adapter 的接入边界，但 Adapter 作者自行负责目标 Runtime 的能力映射、transport 与事件规范化。Runtime 是外部后端，不属于 AScout 可定义的实现模块。

Execution、Workspace、Input、Effect、Outcome 与 Interposer 的职责语义不绑定具体 crate、模块、进程或服务边界。

后续 crate、模块、进程和服务边界必须依据责任、依赖方向、权限、替换频率与部署需求决定，不能由“一侧一模块”或“一组件一 crate”直接推出。具体 Adapter transport、discovery、连接生命周期和 Runtime backend 属于 Adapter/部署实现选择。

**废弃记录**：原决定要求“实现组织服从四侧一级架构”，并把 Runtime backend 视为 AScout 待设计的实现部分。该解释已废弃；四侧现在只表示完整调用路径中的逻辑位置。

## D-025-I：项目名称统一为 AScout

2026-08 决定：项目现行名称统一为 **AScout**（Agent Scout），相关现行复合名称使用 `AScoutAgent`。正式设计、架构、使用和开发文档不再使用 `ASCout` 作为现行名称；本条仅保留旧拼写用于历史追溯。该名称变更不改变既有生命周期、Policy、Runtime 或接口语义。
