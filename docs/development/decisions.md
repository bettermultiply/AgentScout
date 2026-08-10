# 开发决策

本文记录已接受的实现架构与验证决策。目标语义归 `docs/design/` 所有。

## 已退役的旧实现决策

开发决策 D-002 至 D-014 描述已删除的 ptrace/C ABI 原型，现已退役，只能从 Git history 查阅，不能约束新实现。已经独立接受的目标语义仍保留在设计决策中。

## D-015-I：首个真实 Workload 使用 Delayed-Choice Coding

首个端到端 workload 使用 `pbw codex`、Command + Skill 和真实仓库。Deterministic Agent 只用于 conformance tests。Sequential 与 speculative run 使用相同 Agent、任务、outcome 和预算；Hook automation 推迟到显式路径证明正确性与收益之后。

Workload contract 与指标见 [reference-workload.md](reference-workload.md)。

## D-016-I：默认 Runtime 优先高效隔离

默认 Runtime 面向可信 Child Agent 与工具，优先低开销 sibling isolation，不提供 adversarial containment。它仍必须强制维护 identity/capability validation、Branch isolation、资源归属、loser cleanup 和受管 external-effect paths。

需要 adversarial containment 时，用户选择另行适配的 sandbox/hardened Runtime。ASCout 不协商 security guarantee，默认 backend 技术尚未选择。

## D-017-I：新 Rust 实现不继承旧代码

新实现继续使用 Rust。旧 `src/**`、Cargo manifests、tests、examples、C header、构建产物和实验文件已经删除，未读取、迁移或复用。新 workspace 只能依据已接受的设计与开发决策建立。

该删除授权本身不等于授权创建新 workspace 或实现 skeleton；后续已在 2026-07-31 授权并完成 D-021-I skeleton。

## D-018-I：Cargo Workspace 与扩展点范围

新实现使用 Cargo workspace。Crate boundary 按逻辑 ownership 与 replaceability 划分。独立实现、heavy/platform dependency、反向依赖防护、独立测试/发布节奏和 privilege/process deployment 是提取 crate 的理由；扩展点本身不是。

必须预留六类扩展点。Execution、Workspace、Input、Effect 和 Outcome contract 位于 `pbw-runtime-api`，默认 placeholders 位于 `pbw-runtime-default`；Operation Integration 归 Harness 所有，首个实现位于 `pbw-integration-command`。它们不各建独立空 crate。

Shared semantic types 是 `pbw-contract` 中的最小 owner-neutral values，不包含 Harness policy、Runtime mutable state、transport frame、PID/namespace 或 backend-specific handle。

## D-019-I：Runtime Core 可复用但非强制

`pbw-runtime-core` 集中 backend-neutral node-state、capability、obligation、Branch/Resolution barrier 和 transition helper；默认 Runtime 复用它。Harness-side BranchTree 视图不属于 Runtime Core 所有权。

Runtime conformance 不强制使用该 crate。Sandbox、VM、remote 或其他 adapter 可以直接实现 `pbw-runtime-api`，同时保持相同不变量。Core 不拥有 Harness policy、Harness-side BranchTree、control transport、OS privilege、platform handle 或具体 Execution/Workspace/Input/Effect backend。

## D-020-I：初始 Cargo Workspace 拓扑

Cargo workspace 初始 skeleton 拓扑为：

- `pbw-contract`；
- `pbw-runtime-api`；
- `pbw-runtime-core`；
- `pbw-runtime-default`；
- `pbw-harness`；
- `pbw-integration-command`；
- `pbw-launch-codex`；
- `pbw-cli`。

依赖主干初始为 `pbw-cli -> {pbw-launch-codex, pbw-integration-command, pbw-harness}`、`pbw-harness -> pbw-runtime-api -> pbw-contract` 和 `pbw-runtime-default -> pbw-runtime-core -> pbw-runtime-api`。Branch-local Local Controller 边界确认后，当前主干增加 `pbw-local-control -> pbw-contract`，并由 CLI/library 侧通过 Local Controller 进入 Harness Entry API。Contract、Local Control、API 与 Core 禁止反向依赖产品、Harness、具体 Runtime、integration、launcher 或 CLI。`pbw-cli` 只负责 CLI integration，不选择、创建或连接 Runtime；Harness Core 后续负责选择并连接独立 Runtime Service，具体 transport、client 和 adapter 机制尚待收敛。

项目接受独立 `pbw-contract` 与初始多 crate workspace 管理成本，以换取 ownership clarity、replaceability 和反向依赖防护。后续当新的部署、权限或依赖边界得到确认时，可以新增 crate；Branch-local control endpoint 边界确认后已新增 `pbw-local-control`。

## D-021-I：一次性 Skeleton 完成标准

第一版 skeleton 只验证 D-020-I 初始 ownership、依赖方向和 composition。临时 trait method、payload、error、文件布局和测试组织可以直接抛弃，不承担兼容性承诺。

每个 crate 都必须构建；六类扩展点必须有最小 contract/placeholder；ASCoutAgent start path 必须经过 Harness Entry API 和 Harness Core，并在当前阶段只注册 Harness-side root metadata。该 path 不定义目标 Runtime connection。Construction 可以成功；当前 Harness-side BranchTree metadata control 可以执行已确定的 `branch`、`commit`、`accept`、`discard`、`context` 和 `children`。Runtime-backed 行为型调用必须在分配 managed resources、创建 Runtime state 或产生 external effects 前返回内部 unsupported/not-implemented。

验收包括 formatting、workspace build/tests、依赖方向、CLI/Local Controller/Harness boundary wiring、显式零副作用失败和无 legacy dependency；不要求真实 Agent process、backend、transport、Workspace operation 或 service connection。

## D-022-I：Active Documentation 不保留旧实现手册

旧原型的 lifecycle、proxy、Runtime、control protocol、service、module 和 test-matrix 文档已经随实现删除。Active documentation 只描述目标语义、已接受的实现架构、当前 skeleton 状态和未来验收 gates；Git history 是唯一 legacy archive。
