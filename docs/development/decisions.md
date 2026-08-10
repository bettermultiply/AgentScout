# 开发决策

本文记录已接受的实现架构与验证决策。目标语义归 `docs/design/` 所有。

## 已退役的旧实现决策

开发决策 D-002 至 D-014 描述已删除的 ptrace/C ABI 原型，现已退役，只能从 Git history 查阅，不能约束新实现。已经独立接受的目标语义仍保留在设计决策中。

## D-015-I：首个真实 Workload 使用 Delayed-Choice Coding

首个端到端 workload 使用 `ascout codex`、Command + Skill 和真实仓库。Deterministic Agent 只用于 conformance tests。Sequential 与 speculative run 使用相同 Agent、任务、outcome 和预算；Hook automation 推迟到显式路径证明正确性与收益之后。

Workload contract 与指标见 [reference-workload.md](reference-workload.md)。

## D-016-I：默认 Runtime 优先高效隔离

默认 Runtime 面向可信 Child Agent 与工具，优先低开销 sibling isolation，不提供 adversarial containment。它仍必须强制维护 identity/capability validation、Branch isolation、资源归属、loser cleanup 和受管 external-effect paths。

需要 adversarial containment 时，用户选择另行适配的 sandbox/hardened Runtime。ASCout 不协商 security guarantee，默认 backend 技术尚未选择。

## D-017-I：新 Rust 实现不继承旧代码

新实现继续使用 Rust。旧 `src/**`、Cargo manifests、tests、examples、C header、构建产物和实验文件已经删除，未读取、迁移或复用。新实现只能依据已接受的设计与开发决策建立。

## D-018-I：Cargo Workspace 与扩展点范围（已废弃）

本条目与 D-019-I、D-020-I、D-021-I 描述已删除的 skeleton 阶段 crate 拓扑与占位验收标准，现已整体废弃：不再保留任何 `ascout-contract` / `ascout-runtime-*` / `ascout-local-control` 等 crate 组织、依赖方向或编译期约束决策。未来实现的 crate 与模块组织直接围绕目标组件模型（Control Plane：Policy Engine、Assumption Manager、Scheduler、Effect Coordinator、Ledger、Runtime Adapter；Instance 侧：Interposer、Agent、ASC-Daemon、Isolation Env）重新决定，详见 D-023-I。

必须预留六类扩展点这一目标语义仍然有效：Execution、Workspace、Input、Effect 和 Outcome 属于 Runtime 侧，Operation Integration 属于 Harness/Integration 侧。六类扩展点的职责定义见 `docs/design/overview.md`、`docs/design/glossary.md` 与投机执行底座文档。

## D-019-I：Runtime Core 可复用但非强制（已废弃）

见 D-018-I。

## D-020-I：初始 Cargo Workspace 拓扑（已废弃）

见 D-018-I。

## D-021-I：一次性 Skeleton 完成标准（已废弃）

见 D-018-I。

## D-022-I：Active Documentation 不保留旧实现手册

旧原型的 lifecycle、proxy、Runtime、control protocol、service、module 和 test-matrix 文档已经随实现删除。Active documentation 只描述目标语义、已接受的实现架构和未来验收 gates；Git history 是唯一 legacy archive。

## D-023-I：废弃 skeleton crate 拓扑，围绕目标组件模型重新决定

2026-08 决定：不再保留任何 pbw/ascout-* crate 命名、初始 Cargo workspace 拓扑、依赖方向或编译期约束决策（D-018-I 至 D-021-I 废弃）。未来实现的 crate、模块与依赖组织直接围绕目标组件模型重新决定：Control Plane（Policy Engine、Assumption Manager、Scheduler、Effect Coordinator、Ledger、Runtime Adapter）与 Instance 侧（Interposer、Agent、ASC-Daemon、Isolation Env）见 `docs/design/speculation-decisions.md` 与 `docs/development/spec-exec-abstraction.md`。

六类扩展点（Execution、Workspace、Input、Effect、Outcome、Operation Integration）的职责语义不变，只是不再绑定任何 crate 归属。
