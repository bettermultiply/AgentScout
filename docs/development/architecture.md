# 开发架构

ASCout 当前已有第一版 Rust workspace skeleton。本文记录新 Rust 实现已经接受的 ownership 与依赖架构；目标语义仍以 `docs/design/` 为准。

## Harness 与 Runtime 边界

Harness 是 Agent-facing control plane 与 lifecycle orchestration layer。它对外提供统一 Harness Entry API，让 Branch-local Local Controller 和 embedded function/system integration 进入同一套语义入口；Entry API 不直接接触 Runtime，只把已解析的 typed request 交给 Harness Core。Harness Core 维护 ASCoutAgent registry、Harness 侧 BranchTree、按需派生的 BranchView、request correlation 和最小 event handling；后续再通过 Harness Runtime Boundary 连接 Runtime。

ASCoutAgent 是 Harness Core 内部的唯一 Agent tree owner，代表从一个 root Branch 衍生出来的一整棵 ASCout Agent tree；外部 Agent 不持有 ASCoutAgent session，也不需要感知当前 ASCoutAgent 或 Branch identity。BranchTree 是 ASCoutAgent 内部维护的完整 Branch 拓扑与快速查找结构：它保存 root Branch，并用 BranchId 映射到 BranchNode。每个 BranchNode 维护自己的直接 parent、direct children、metadata disposition 和 commit 标记，因此 parent 不一定是 root。BranchView 是某次调用在特定 Branch 身份下按需派生的局部可见视图，不表示整个 subtree 状态，也不作为每个 Branch 的长期独立对象保存。

当前 Harness-side BranchTree 已实现最小 Branch-level metadata operations：branch、commit、discard subtree、accept、context 和 children。`branch` 当前一次建立一个或多个 direct child；普通 discard 清理目标 subtree 并从 active parent children 中移除；parent continue 通过 discard 当前 child candidates 后让 parent 保持 live 来表达，不引入单独 operation。Accept 要求 accepted child 已 committed，在 Harness metadata 层丢弃 siblings、将 grandparent/root 重连到 accepted child，并把旧 parent 标记为 superseded；真实 execution/workspace/effect 状态传播、记录、materialization 和 cleanup 仍属于 Runtime/Resolution barrier 后续实现。

Runtime 是 mechanism provider，维护单个 Agent/节点的执行状态，并可按具体实现负责节点之间的状态传播；这不表示 Runtime 拥有或暴露整体 BranchTree 视图。Harness 不得借用 Runtime mutable state，也不得依赖具体 Runtime。

Agent-facing control path 使用 central Harness 模型：Agent 与 `pbw` CLI/library 运行在 Branch 隔离环境内，并访问同环境内的 Branch-local Local Controller。第一版 Local Controller 实现为 HTTP Server，使用 Branch 隔离环境内约定端口；HTTP、端口和 wire format 是实现选择，不进入 ASCout 目标语义。Local Controller 绑定当前 ASCoutAgent/Branch，CLI/library 只发送不含身份的 AgentControlIntent。Local Controller 将自身绑定附加成 ResolvedBranchControlRequest 后转发到 central Harness Entry API；Harness Core 再用 ASCoutAgentRegistry 验证 Branch 属于该 ASCoutAgent，并派发到 ASCoutAgent Branch-level operation。

Harness Runtime Boundary 是 Harness Core 侧恒定层：它规定稳定 Runtime API，并负责连接、session、event 和 error mapping。Entry API 不越过 Harness Core 直接访问 Runtime。Runtime Adapter 只指面向具体 Runtime 的可变适配层，位于该边界之下；当前暂不展开 Harness Core 到 Runtime 的连接断点。

CLI integration 不选择、创建或连接 Runtime。Routine CLI commands 不接受 ASCoutAgent id 或 Branch id；它们只向 Branch-local Local Controller 的约定 HTTP endpoint 发送 AgentControlIntent。Harness Core 后续通过 Harness Runtime Boundary 连接由自身选择的 Runtime Service；具体 Runtime transport、client 和 adapter 形态另行决定。

跨边界使用 typed、粗粒度 contract，只传递 launch specification、Branch context、opaque candidate handle、Proposal、outcome、commit request、disposition 和 Runtime event 等中立值；不暴露 PID、namespace path、transport frame 或 backend capture handle。

ASCout 不选择统一 Runtime control transport。进程内调用、IPC、VM channel、kernel interface 或 remote protocol 完全由 Runtime Adapter 决定。Branch-local CLI 到 Local Controller 的第一版通道可以使用约定端口 HTTP。

## 扩展边界

| 扩展点 | Owner | 职责 |
|---|---|---|
| Execution | Runtime | 完整 Agent process tree、内存/线程/进程关系、环境、FD、signal、Branch-private kernel resources、barrier、resume 和 cleanup |
| Workspace | Runtime | 文件/目录视图、Branch-local modifications、ancestor visibility、winner continuation 和 loser discard |
| Input | Runtime | Ancestor fan-out、每 Branch 独立读取位置和 lifecycle continuity |
| Effect | Runtime | Adapter observation surface 与 guarantee horizon 内的 shared/global service interaction |
| Outcome | Runtime boundary | Runtime Core 拥有 slot、routing、obligation 与 Accept gate；adapter 规范化并发布真实 outcome |
| Operation Integration | Harness | Command/tool operation capture、command contract、真实 operation once-only integration 和自动 outcome publication |

六类扩展点不各建空 crate。Runtime 侧 contract 放在 `pbw-runtime-api`，默认 placeholders 放在 `pbw-runtime-default`；Operation Integration 由 Harness 拥有，首个实现放在 `pbw-integration-command`。

Execution 与 Workspace 严格分开：Workspace 只表示文件/目录状态。Branch-private socket/port namespace 属于 Execution；宿主可见 port reservation、shared daemon 和 quota 属于 Effect 或 global-service contract。Persistent command session 先由 Operation contract 描述，再按资源归属进入 Execution 或 Effect。无法复制、隔离或 broker 的 singleton 不支持 speculative execution。

## Cargo Workspace 拓扑

Rust workspace 初始 skeleton crate 拓扑已经建立，并会随明确的 ownership、deployment 或 dependency boundary 继续演进：

| Crate | Ownership |
|---|---|
| `pbw-contract` | 最小中立 semantic values、opaque handles 和 operation/event envelopes |
| `pbw-local-control` | Branch-local Local Controller 的 request/binding values；第一版实现使用约定端口 HTTP |
| `pbw-runtime-api` | Runtime typed API 与五类 Runtime-side extension contracts |
| `pbw-runtime-core` | 可复用但非强制的 Runtime node-state 与 transition helper mechanism |
| `pbw-runtime-default` | 默认高效 Runtime adapter 与 placeholder backends |
| `pbw-harness` | Harness Entry API、Harness Core、ASCoutAgent/BranchTree 视图、Runtime Boundary 和 orchestration |
| `pbw-integration-command` | Command + Skill profile 与 Operation Integration |
| `pbw-launch-codex` | Codex-specific launch preparation |
| `pbw-cli` | `pbw` binary 与 CLI integration entry |

依赖主干是：

```text
pbw-cli -> {pbw-launch-codex, pbw-integration-command,
            pbw-harness}
pbw-launch-codex -> {pbw-harness, pbw-local-control}
pbw-harness -> {pbw-local-control, pbw-runtime-api}
pbw-local-control -> pbw-contract
pbw-runtime-api -> pbw-contract
pbw-runtime-default -> pbw-runtime-core -> pbw-runtime-api
```

Contract、Local Control、API 与 Core 禁止反向依赖 Harness、具体 Runtime、integration、launcher 或 CLI。业务 crate 不得选择具体实现。`pbw-cli` 不负责选择、创建或连接 Runtime；Harness 负责选择并连接独立 Runtime Service，具体 transport、client 和 adapter 机制另行收敛。

## Runtime 可替换性与特权

`pbw-runtime-core` 集中可复用的 Runtime node-state、capability、obligation、barrier 与 transition helper，但 Runtime conformance 不强制复用该 crate。Sandbox、VM、remote 或其他 Runtime 可以直接实现 `pbw-runtime-api`。全局 BranchTree 视图属于 Harness 侧 ASCoutAgent，不属于 Runtime Core 的所有权。

默认 Runtime 面向可信 Child Agent，优先高效隔离，不承诺 adversarial containment。OS privilege 只能由所选 Runtime 或其内部 helper/daemon 持有；Harness、Agent、launcher 和 integration 不能持有特权或绕过 Runtime API。

一个 ASCoutAgent 只拥有一棵 Harness-side BranchTree。Identity、capability、reference 和 lifecycle operation 不允许跨 tree。

Routine Agent API 的上下文隐藏由 Local Controller 完成：CLI 与 embedded function 调用不显式传 ASCoutAgent id 或 parent BranchId。Local Controller 自身代表当前 Branch，并把 endpoint-bound ASCoutAgent/Branch context 附加到发往 central Harness 的 ResolvedBranchControlRequest。Entry API 不重新解析 context，也不直接接触 Runtime；Harness Core 必须验证该 Branch 属于对应 ASCoutAgent，然后只能在当前 Branch 下执行 context、children、branch、commit、accept 或 discard 等 Harness-side Branch-level operation。Runtime-bound branch request、Proposal/outcome payload 和 ChildrenSummary 的完整 contract 留待 Runtime phase；当前 `branch` 只更新 Harness-side BranchTree metadata。

## Launcher 与 Integration

首个产品入口计划为 `pbw codex`。`pbw-launch-codex` 当前只保留 launch entry skeleton；Codex argv、cwd、environment policy、TTY 和 Skill/Hook 准备属于后续 adapter 工作。与产品无关的 ASCoutAgent 创建留在 Harness Core；后续 Runtime-side execution creation 留在 Harness/Runtime boundary。当前 start path 只完成 Harness-side root metadata 注册。

Command + Skill 是首个必须支持的 integration profile。Hook 在同步能力得到验证后再增加；embedded Agent adapter 是可选项，必须以明确能力缺口为前提。

## 一次性 Skeleton 边界

第一版 skeleton 只验证初始 ownership graph。每个 workspace crate 都必须构建，六类扩展点必须有最小 contract/placeholder，CLI integration、Local Controller request/binding 和 Harness Entry/Core 必须保持边界清晰；Runtime Service 的选择与连接属于 Harness Core 后续工作，具体连接机制不在 skeleton 中定型。

Composition construction 可以成功；当前 Harness-side `branch`、`commit`、`accept`、`discard`、`context` 和 `children` 只操作 Harness-owned BranchTree metadata。任何 Runtime-backed operation 仍必须在创建 Runtime state、process、managed resource、Workspace change 或 external effect 前返回内部 unsupported/not-implemented。当前实现不声称 Runtime lifecycle、isolation 或 security 已经实现，临时 API 不承担兼容性承诺。

Skeleton 当前已创建。它只验证初始 ownership graph、最小 contract/placeholder、CLI/Local Controller/Harness boundary wiring，不实现 lifecycle、isolation、security 或真实 backend。
