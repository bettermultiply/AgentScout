# Agent 接入状态

AScout 当前没有任何实现，当前仓库只有设计文档。Agent-facing 命令、Rust API、control protocol 和 Runtime lifecycle 均未实现。

## 当前可用内容

- `docs/design/` 中已经确认的目标语义；
- 已确认的四侧一级架构与 `docs/development/` 中的开发决策；
- 计划使用 Command + Skill 和 `ascout codex` launch adapter 的首个 integration profile。

当前没有可构建代码。临时命令、HTTP control protocol、稳定 Rust API、C API、Runtime lifecycle 和正式 Agent-facing 命令语法均不存在。

## 计划中的 Agent Surface

Routine Agent-facing control 计划通过 Branch-local ASC-Daemon 进入 Entry API。ASC-Daemon 隐藏当前 AScoutAgent/Branch context；第一版实现计划使用隔离环境内约定端口的 HTTP Server。计划覆盖以下操作：

- 查询当前 Branch context；
- 查询 direct children；
- 创建 candidates；
- commit、accept 和 discard；
- 等待 events。

Outcome 由 Operation Integration 自动发布，不作为独立 Agent command。Resolver delegation 属于 launch 或创建配置，不作为 routine command。参数、返回值、阻塞行为、event 表示和具体命令拼写均未定义。

## 计划中的 Launch 与 Integration

首个产品入口计划为 `ascout codex`。产品相关的 argv、环境、TTY、Skill 和未来 Hook 准备归 Agent 接入侧；routine CLI/control 计划先进入 ASC-Daemon，再由 Entry API 转交控制面。控制面只依赖 Runtime Adapter 的稳定 typed contract；Adapter discovery、连接和 Runtime handshake 后续单独定义。

Command + Skill 是第一个必须支持的 integration profile。Hook 只有在同步能力得到验证后才增加；embedded Agent adapter 不是正确性前提。

## 可用性规则

不得从目标语义或 roadmap item 推断当前支持。只有当前实现及其验证证据存在后，使用文档才会加入对应构建、运行和 API 说明。
