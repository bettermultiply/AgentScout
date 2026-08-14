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

本条目与 D-019-I、D-020-I、D-021-I 描述已删除的 skeleton 阶段 crate 拓扑与占位验收标准，现已整体废弃：不再保留任何 `ascout-contract` / `ascout-runtime-*` / `ascout-local-control` 等 crate 组织、依赖方向或编译期约束决策。废弃后的内部组件整理见 D-023-I；当前实现组织以 D-024-I 的四侧一级架构为准。

必须预留六类扩展点这一目标语义仍然有效：Execution、Workspace、Input、Effect 和 Outcome 属于 Runtime Mechanism，Operation Integration 属于 Agent 接入侧。六类扩展点的职责定义见 `docs/design/overview.md`、`docs/design/glossary.md` 与投机执行底座文档。

## D-019-I：Runtime Core 可复用但非强制（已废弃）

见 D-018-I。

## D-020-I：初始 Cargo Workspace 拓扑（已废弃）

见 D-018-I。

## D-021-I：一次性 Skeleton 完成标准（已废弃）

见 D-018-I。

## D-022-I：Active Documentation 不保留旧实现手册

旧原型的 lifecycle、proxy、Runtime、control protocol、service、module 和 test-matrix 文档已经随实现删除。Active documentation 只描述目标语义、已接受的实现架构和未来验收 gates；Git history 是唯一 legacy archive。

## D-023-I：废弃 skeleton crate 拓扑，保留必要内部组件

2026-08 决定：不再保留任何 pbw/ascout-* crate 命名、初始 Cargo workspace 拓扑、依赖方向或编译期约束决策（D-018-I 至 D-021-I 废弃）。Policy Engine、Assumption Manager、Scheduler、Effect Coordinator、Ledger、ASC-Daemon 与 Interposer 等必要职责继续作为内部组件候选保留，但旧 Control Plane / Instance 图不再直接决定 crate、模块或部署组织；当前一级架构和组件归属由 D-024-I 覆盖。

废弃名称只在本条留档：系统旧名 Parallel Branch World、PbwAgent、BranchTree、Policy Compiler、Policy Set、Frontier Scheduler、Local Controller、Local Daemon、API Entry、hypothesis 和 commit proposal。现行名称依次使用 ASCout、ASCoutAgent、Speculative Tree、Policy Engine、Scheduler、ASC-Daemon、Entry API、Proposal / Assumption 和 commit request；正式设计与使用文档不再引用旧名。

六类扩展点（Execution、Workspace、Input、Effect、Outcome、Operation Integration）的职责语义不变，只是不再绑定任何 crate 归属。

## D-024-I：实现组织服从四侧一级架构

D-023-I 确认保留的内部组件继续有效，但其旧 Control Plane / Instance 组件图不再直接充当 crate、进程或部署拓扑。实现组织首先服从四侧一级架构：

1. Agent 接入侧；
2. 控制面侧；
3. Runtime Adapter 侧，其中分为稳定 Harness Runtime Boundary 与面向具体 Runtime 的实际 Adapter；
4. Runtime Mechanism。

Policy Engine、Assumption Manager、Scheduler、Effect Coordinator 与 Ledger 位于控制面内部；Agent/product、ASC-Daemon、Interposer、Operation Integration、CLI/library/launcher 位于 Agent 接入职责；Execution、Workspace、Input、Effect 与 Outcome backend 位于 Runtime Mechanism。Agent execution、Instance 和 Branch 隔离环境是 Runtime Mechanism 管理的运行对象，不是独立一级层。

后续 crate、模块、进程和服务边界必须依据 ownership、依赖方向、权限、替换频率与部署需求在四侧内决定，不能由“一组件一 crate”或旧部署图直接推出。具体 Adapter transport、discovery、连接生命周期和 Runtime backend 仍待后续设计。
