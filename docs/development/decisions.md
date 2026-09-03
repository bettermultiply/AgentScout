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

## D-015-I：首个真实 Workload 使用 Delayed-Choice Coding

首个端到端 workload 使用 `ascout codex`、Command + Skill 和真实仓库。Deterministic Agent 只用于 conformance tests。Sequential 与 speculative run 使用相同 Agent、任务、outcome 和预算；Hook automation 推迟到显式路径证明正确性与收益之后。

Workload contract 与指标见 [reference-workload.md](reference-workload.md)。

## D-016-I：默认 Runtime 优先高效隔离

默认 Runtime 面向可信 Child Agent 与工具，优先低开销 sibling isolation，不提供 adversarial containment。它仍必须强制维护 identity/capability validation、Branch isolation、资源归属、loser cleanup 和受管 external-effect paths。

需要 adversarial containment 时，用户选择另行适配的 sandbox/hardened Runtime。AScout 不协商 security guarantee，默认 backend 技术尚未选择。

## D-017-I：新 Rust 实现不继承旧代码

新实现继续使用 Rust。旧 `src/**`、Cargo manifests、tests、examples、C header、构建产物和实验文件已经删除，未读取、迁移或复用。新实现只能依据已接受的设计与开发决策建立。

## D-024-I：实现组织服从 AScout 可控边界

完整调用路径仍为：

1. Agent 接入侧；
2. 控制面；
3. Runtime Adapter；
4. Runtime。

这四侧不再直接作为 AScout 源码或子文档的一一对应拓扑。Entry API、Policy Engine、Assumption Manager、Scheduler、Effect Coordinator 与 Ledger 位于控制面内部；ASC-Daemon、Interposer、Operation Integration、CLI/library/launcher 位于 Agent 接入职责。控制面定义 Runtime Adapter 的接入边界，但 Adapter 作者自行负责目标 Runtime 的能力映射、transport 与事件规范化。Runtime 是外部后端，不属于 AScout 可定义的实现模块。

Execution、Workspace、Input、Effect、Outcome 与 Operation Integration 六类扩展点的职责语义不绑定具体 crate、模块、进程或服务边界。

后续 crate、模块、进程和服务边界必须依据责任、依赖方向、权限、替换频率与部署需求决定，不能由“一侧一模块”或“一组件一 crate”直接推出。具体 Adapter transport、discovery、连接生命周期和 Runtime backend 属于 Adapter/部署实现选择。

**废弃记录**：原决定要求“实现组织服从四侧一级架构”，并把 Runtime backend 视为 AScout 待设计的实现部分。该解释已废弃；四侧现在只表示完整调用路径中的逻辑位置。

## D-025-I：项目名称统一为 AScout

2026-08 决定：项目现行名称统一为 **AScout**（Agent Scout），相关现行复合名称使用 `AScoutAgent`。正式设计、架构、使用和开发文档不再使用 `ASCout` 作为现行名称；本条仅保留旧拼写用于历史追溯。该名称变更不改变既有生命周期、Policy、Runtime 或接口语义。
