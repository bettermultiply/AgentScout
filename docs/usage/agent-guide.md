# Agent 接入状态

ASCout 当前提供可构建 Rust workspace 和 Harness-side metadata control skeleton。旧原型 binary、Rust/C facade、control protocol、examples 和 headers 已经删除；当前 CLI 命令仍是临时语法，不代表完整 Runtime lifecycle 已可用。

## 当前可用内容

- `docs/design/` 中已经确认的目标语义；
- `docs/development/` 中已经确认的 crate 与 ownership boundaries；
- 计划使用 Command + Skill 和 `pbw codex` launch adapter 的首个 integration profile。

当前可使用 Cargo 构建和测试 skeleton。已有临时 `pbw context`、`children`、`branch`、`commit`、`accept`、`discard` 命令可连接 Branch-local Local Controller，并驱动 Harness-side BranchTree metadata；还没有稳定 HTTP control protocol、稳定 Rust API、C API、Runtime lifecycle 或正式 Agent-facing 命令语法。

## 计划中的 Agent Surface

Routine Agent-facing control 暂时通过 Branch-local Local Controller 进入 Harness Entry API。Local Controller 隐藏当前 ASCoutAgent/Branch context；第一版实现使用隔离环境内约定端口的 HTTP Server。当前临时 CLI 命令覆盖以下 placeholders：

- 查询当前 Branch context；
- 查询 direct children；
- 创建 candidates；
- commit、accept 和 discard；
- 等待 events。

Outcome 由 Operation Integration 自动发布，不作为独立 Agent command。Resolver delegation 属于 launch 或创建配置，不作为 routine command。参数、返回值、阻塞行为、event 表示和具体命令拼写均未定义。

## 计划中的 Launch 与 Integration

首个产品入口是 `pbw codex`。产品相关的 argv、环境、TTY、Skill 和未来 Hook 准备归 `pbw-launch-codex`；routine CLI/control 先进入 Local Controller，再由 Harness Entry API 转交 Harness Core。Harness Core 到 Runtime 的连接断点后续单独定义。

Command + Skill 是第一个必须支持的 integration profile。Hook 只有在同步能力得到验证后才增加；embedded Agent adapter 不是正确性前提。

## 可用性规则

不得从目标语义或 roadmap item 推断当前支持。只有当前实现及其验证证据存在后，使用文档才会加入对应构建、运行和 API 说明。
