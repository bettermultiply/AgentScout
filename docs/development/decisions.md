# 开发决策

本文记录已接受的实现架构与验证决策。目标语义归 `docs/design/` 所有。

## D-015-I：首个真实 Workload 使用 Delayed-Choice Coding

首个端到端 workload 使用 `ascout codex`、Command + Skill 和真实仓库。Deterministic Agent 只用于 conformance tests。Sequential 与 speculative run 使用相同 Agent、任务、outcome 和预算；Hook automation 推迟到显式路径证明正确性与收益之后。

Workload contract 与指标见 [reference-workload.md](reference-workload.md)。

## D-016-I：默认 Runtime 优先高效隔离

默认 Runtime 面向可信 Child Agent 与工具，优先低开销 sibling isolation，不提供 adversarial containment。它仍必须强制维护 identity/capability validation、Branch isolation、资源归属、loser cleanup 和受管 external-effect paths。

需要 adversarial containment 时，用户选择另行适配的 sandbox/hardened Runtime。AScout 不协商 security guarantee，默认 backend 技术尚未选择。

## D-017-I：新 Rust 实现不继承旧代码

新实现继续使用 Rust。旧 `src/**`、Cargo manifests、tests、examples、C header、构建产物和实验文件已经删除，未读取、迁移或复用。新实现只能依据已接受的设计与开发决策建立。

## D-024-I：实现组织服从四侧一级架构

实现组织服从四侧一级架构：

1. Agent 接入侧；
2. 控制面；
3. Runtime Adapter；
4. Runtime。

Entry API、Policy Engine、Assumption Manager、Scheduler、Effect Coordinator 与 Ledger 位于控制面内部；Agent/product、ASC-Daemon、Interposer、Operation Integration、CLI/library/launcher 位于 Agent 接入职责。Runtime Adapter 自身提供稳定 typed contract，并负责目标 Runtime 的能力映射、transport 与事件规范化。Execution、Workspace、Input、Effect 与 Outcome backend 位于 Runtime；Agent execution、Instance 和 Branch 隔离环境是 Runtime 管理的运行对象，不是独立一级层。

Execution、Workspace、Input、Effect、Outcome 与 Operation Integration 六类扩展点的职责语义不绑定具体 crate、模块、进程或服务边界。

后续 crate、模块、进程和服务边界必须依据 ownership、依赖方向、权限、替换频率与部署需求在四侧内决定，不能由“一组件一 crate”直接推出。具体 Adapter transport、discovery、连接生命周期和 Runtime backend 仍待后续设计。

## D-025-I：项目名称统一为 AScout

2026-08 决定：项目现行名称统一为 **AScout**（Agent Scout），相关现行复合名称使用 `AScoutAgent`。正式设计、架构、使用和开发文档不再使用 `ASCout` 作为现行名称；本条仅保留旧拼写用于历史追溯。该名称变更不改变既有生命周期、Policy、Runtime 或接口语义。
